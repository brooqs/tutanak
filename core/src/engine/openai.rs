//! OpenAI-compatible HTTP engine. One implementation serves every provider that
//! speaks the OpenAI API: Groq (cloud), FastFlowLM (AMD NPU @ :52625), Ollama
//! (@ :11434), llama.cpp server, LocalAI, ... — differentiated only by base_url
//! and (optional) API key.
//!
//!   wav ──plan_chunks──► [chunk0 chunk1 …] ──POST /audio/transcriptions──► texts
//!                              │  (resume: skip chunks already on disk)
//!                              └── 413? split chunk in half and retry
//!   texts ──stitch──► transcript ──POST /chat/completions (map-reduce)──► summary / translation

use std::fmt;
use std::fs;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::{multipart, Client, RequestBuilder, Response};
use serde::Deserialize;

use crate::audio::{self, ChunkSpec, WavInfo};
use crate::engine::{LlmEngine, SttEngine, TranscribeOutput};
use crate::stitch;
use crate::text;

const MAX_RETRIES: u32 = 5;
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const MAP_WINDOW_WORDS: usize = 1800;
const MIN_CHUNK_FRAMES: u32 = 16_000; // ~1s @ 16kHz

fn http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .context("HTTP client kurulamadı")
}

fn auth(rb: RequestBuilder, key: &Option<String>) -> RequestBuilder {
    match key {
        Some(k) if !k.is_empty() => rb.bearer_auth(k),
        _ => rb,
    }
}

// ---------------------------------------------------------------------------
// STT
// ---------------------------------------------------------------------------

pub struct OpenAiStt {
    provider: String,
    base_url: String,
    api_key: Option<String>,
    model: String,
    chunk_threshold_bytes: u64,
    chunk_overlap_secs: u32,
    backoff_base_ms: u64,
    client: Client,
}

impl OpenAiStt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: String,
        base_url: String,
        api_key: Option<String>,
        model: String,
        chunk_threshold_bytes: u64,
        chunk_overlap_secs: u32,
        backoff_base_ms: u64,
    ) -> Result<Self> {
        Ok(OpenAiStt {
            provider,
            base_url,
            api_key,
            model,
            chunk_threshold_bytes,
            chunk_overlap_secs,
            backoff_base_ms,
            client: http_client()?,
        })
    }

    fn transcribe_range(
        &self,
        wav_path: &Path,
        info: &WavInfo,
        spec: ChunkSpec,
    ) -> Result<(String, Option<String>)> {
        let wav_bytes = audio::read_chunk_wav(wav_path, info, spec)?;
        match self.transcribe_chunk(wav_bytes) {
            Ok(r) => Ok((r.text, r.language.map(|l| normalize_lang(&l)))),
            Err(e) if e.downcast_ref::<PayloadTooLarge>().is_some() => {
                if spec.frames() <= MIN_CHUNK_FRAMES {
                    return Err(e).context("chunk minimuma indi ama hâlâ 413");
                }
                let mid = spec.start_frame + spec.frames() / 2;
                let left = ChunkSpec { start_frame: spec.start_frame, end_frame: mid };
                let right = ChunkSpec { start_frame: mid, end_frame: spec.end_frame };
                let (lt, ll) = self.transcribe_range(wav_path, info, left)?;
                let (rt, _rl) = self.transcribe_range(wav_path, info, right)?;
                Ok((format!("{lt} {rt}"), ll))
            }
            Err(e) => Err(e),
        }
    }

    fn transcribe_chunk(&self, wav_bytes: Vec<u8>) -> Result<ChunkText> {
        let url = format!("{}/audio/transcriptions", self.base_url);
        let resp = execute_with_retry(self.backoff_base_ms, || {
            let part = multipart::Part::bytes(wav_bytes.clone())
                .file_name("chunk.wav")
                .mime_str("audio/wav")
                .expect("static mime");
            let form = multipart::Form::new()
                .part("file", part)
                .text("model", self.model.clone())
                .text("response_format", "verbose_json")
                .text("temperature", "0");
            auth(self.client.post(&url), &self.api_key).multipart(form)
        })?;

        if resp.status().as_u16() == 413 {
            return Err(anyhow!(PayloadTooLarge));
        }
        let resp = ensure_ok(resp, &self.provider, "transkripsiyon")?;
        let parsed: VerboseTranscription =
            resp.json().context("transkripsiyon yanıtı parse edilemedi")?;
        Ok(ChunkText { text: parsed.text.trim().to_string(), language: parsed.language })
    }
}

impl SttEngine for OpenAiStt {
    fn transcribe(&self, wav_path: &Path, job_dir: &Path) -> Result<TranscribeOutput> {
        let info = WavInfo::read(wav_path)?;
        if info.total_frames == 0 {
            bail!("Kayıt boş (sıfır-uzunluk) — yakalama başarısız olmuş olabilir");
        }
        fs::create_dir_all(job_dir).context("iş dizini oluşturulamadı")?;

        let max_frames = audio::max_frames_for_threshold(&info, self.chunk_threshold_bytes);
        let overlap_frames = self.chunk_overlap_secs * info.sample_rate;
        let chunks = audio::plan_chunks(info.total_frames, max_frames, overlap_frames);

        let mut texts: Vec<String> = Vec::with_capacity(chunks.len());
        let mut langs: Vec<String> = Vec::new();

        for (i, spec) in chunks.iter().enumerate() {
            let txt_path = job_dir.join(format!("chunk_{i:04}.txt"));
            let lang_path = job_dir.join(format!("chunk_{i:04}.lang"));

            if txt_path.exists() {
                texts.push(fs::read_to_string(&txt_path).context("resume: chunk metni okunamadı")?);
                if let Ok(l) = fs::read_to_string(&lang_path) {
                    langs.push(l.trim().to_string());
                }
                eprintln!("  chunk {}/{}: resume (diskten)", i + 1, chunks.len());
                continue;
            }

            eprintln!("  chunk {}/{}: transcribe ({})...", i + 1, chunks.len(), self.provider);
            let (text, lang) = self.transcribe_range(wav_path, &info, *spec)?;
            fs::write(&txt_path, &text).context("chunk metni yazılamadı (resume için)")?;
            if let Some(l) = &lang {
                let _ = fs::write(&lang_path, l);
                langs.push(l.clone());
            }
            texts.push(text);
        }

        Ok(TranscribeOutput {
            transcript: stitch::stitch(&texts),
            language: majority_lang(&langs),
        })
    }

    fn label(&self) -> String {
        format!("{} ({})", self.provider, self.model)
    }
}

// ---------------------------------------------------------------------------
// LLM (translate + summarize)
// ---------------------------------------------------------------------------

pub struct OpenAiLlm {
    provider: String,
    base_url: String,
    api_key: Option<String>,
    model: String,
    backoff_base_ms: u64,
    client: Client,
}

impl OpenAiLlm {
    pub fn new(
        provider: String,
        base_url: String,
        api_key: Option<String>,
        model: String,
        backoff_base_ms: u64,
    ) -> Result<Self> {
        Ok(OpenAiLlm { provider, base_url, api_key, model, backoff_base_ms, client: http_client()? })
    }

    fn chat(&self, system: &str, user: &str) -> Result<String> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "temperature": 0.2,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
        });
        let resp = execute_with_retry(self.backoff_base_ms, || {
            auth(self.client.post(&url), &self.api_key).json(&body)
        })?;
        let resp = ensure_ok(resp, &self.provider, "sohbet (LLM)")?;
        let parsed: ChatResponse = resp.json().context("LLM yanıtı parse edilemedi")?;
        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content.trim().to_string())
            .ok_or_else(|| anyhow!("LLM yanıtında içerik yok"))
    }
}

impl LlmEngine for OpenAiLlm {
    fn translate(&self, text: &str, target_lang: &str) -> Result<String> {
        let lang_name = lang_name(target_lang);
        let windows = text::split_into_windows(text, MAP_WINDOW_WORDS);
        let mut out = Vec::with_capacity(windows.len());
        for (i, w) in windows.iter().enumerate() {
            eprintln!("  çeviri {}/{}...", i + 1, windows.len());
            let sys = format!(
                "You are a professional translator. Translate the user's text into {lang_name}. \
                 Output only the translation, preserving meaning and tone. Do not add commentary."
            );
            out.push(self.chat(&sys, w)?);
        }
        Ok(out.join("\n\n"))
    }

    fn summarize(&self, transcript: &str, target_lang: &str) -> Result<String> {
        let lang_name = lang_name(target_lang);
        let windows = text::split_into_windows(transcript, MAP_WINDOW_WORDS);
        if windows.is_empty() {
            return Ok(String::new());
        }

        let map_sys = format!(
            "You are a meeting-notes assistant. Summarize the transcript section in {lang_name}. \
             Use concise bullet points covering: key decisions, action items (with owners if \
             mentioned), and open questions. Output only the summary in {lang_name}."
        );
        let mut partials = Vec::with_capacity(windows.len());
        for (i, w) in windows.iter().enumerate() {
            eprintln!("  özet (map) {}/{}...", i + 1, windows.len());
            partials.push(self.chat(&map_sys, w)?);
        }

        if partials.len() == 1 {
            return Ok(partials.into_iter().next().unwrap());
        }

        eprintln!("  özet (reduce)...");
        let reduce_sys = format!(
            "You are a meeting-notes assistant. The user provides several partial summaries of \
             one meeting in {lang_name}. Merge them into a single coherent summary in {lang_name}, \
             deduplicating and grouping: Decisions, Action Items, Open Questions. Output only the \
             final summary in {lang_name}."
        );
        self.chat(&reduce_sys, &partials.join("\n\n---\n\n"))
    }

    fn label(&self) -> String {
        format!("{} ({})", self.provider, self.model)
    }
}

// ---------------------------------------------------------------------------
// Shared HTTP helpers + parsing
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct PayloadTooLarge;
impl fmt::Display for PayloadTooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "payload too large (413)")
    }
}
impl std::error::Error for PayloadTooLarge {}

#[derive(Debug)]
struct ChunkText {
    text: String,
    language: Option<String>,
}

#[derive(Deserialize)]
struct VerboseTranscription {
    text: String,
    #[serde(default)]
    language: Option<String>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}
#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}
#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

fn execute_with_retry(backoff_base_ms: u64, build: impl Fn() -> RequestBuilder) -> Result<Response> {
    let mut attempt = 0u32;
    loop {
        let resp = build().send().context("HTTP isteği gönderilemedi")?;
        let status = resp.status();
        let retriable = status.as_u16() == 429 || status.is_server_error();
        if retriable && attempt < MAX_RETRIES {
            let wait = retry_after(&resp).unwrap_or_else(|| backoff(backoff_base_ms, attempt));
            eprintln!(
                "  HTTP {} — {}ms sonra tekrar ({}/{})",
                status.as_u16(),
                wait.as_millis(),
                attempt + 1,
                MAX_RETRIES
            );
            sleep(wait);
            attempt += 1;
            continue;
        }
        return Ok(resp);
    }
}

fn ensure_ok(resp: Response, provider: &str, what: &str) -> Result<Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let code = status.as_u16();
    let body = resp.text().unwrap_or_default();
    let hint = match code {
        401 | 403 => " (API anahtarı geçersiz/yetkisiz — provider'ın api_key_env'ini kontrol et)",
        429 => " (kota/oran sınırı — krediler bitmiş ya da hız limiti olabilir)",
        _ => "",
    };
    bail!("{provider} {what} başarısız: HTTP {code}{hint}: {}", body.trim());
}

fn retry_after(resp: &Response) -> Option<Duration> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

fn backoff(base_ms: u64, attempt: u32) -> Duration {
    let ms = base_ms.saturating_mul(1u64 << attempt.min(6));
    Duration::from_millis(ms).min(MAX_BACKOFF)
}

pub fn normalize_lang(raw: &str) -> String {
    let l = raw.trim().to_lowercase();
    match l.as_str() {
        "en" | "english" => "en",
        "tr" | "turkish" => "tr",
        "de" | "german" => "de",
        "fr" | "french" => "fr",
        "es" | "spanish" => "es",
        "it" | "italian" => "it",
        "ru" | "russian" => "ru",
        "ar" | "arabic" => "ar",
        other => other,
    }
    .to_string()
}

fn lang_name(code: &str) -> &str {
    match normalize_lang(code).as_str() {
        "en" => "English",
        "tr" => "Turkish",
        "de" => "German",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "ru" => "Russian",
        "ar" => "Arabic",
        _ => "the target language",
    }
}

fn majority_lang(langs: &[String]) -> String {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for l in langs {
        *counts.entry(l.as_str()).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(l, _)| l.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lang_maps_names_and_codes() {
        assert_eq!(normalize_lang("English"), "en");
        assert_eq!(normalize_lang("turkish"), "tr");
        assert_eq!(normalize_lang("klingon"), "klingon");
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff(100, 0), Duration::from_millis(100));
        assert_eq!(backoff(100, 1), Duration::from_millis(200));
        assert!(backoff(100, 20) <= MAX_BACKOFF);
    }

    #[test]
    fn majority_lang_picks_most_common() {
        let v = vec!["en".to_string(), "en".to_string(), "tr".to_string()];
        assert_eq!(majority_lang(&v), "en");
        assert_eq!(majority_lang(&[]), "");
    }
}
