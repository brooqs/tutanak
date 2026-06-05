//! Layered, registry-based provider configuration.
//!
//! Precedence (lowest → highest):
//!   built-in defaults  →  ~/.config/tutanak/config.toml  →  env overrides
//!
//! Providers are a REGISTRY of named profiles. `stt.provider` / `summary.provider`
//! reference a profile by name. Each profile has a transport `kind`:
//!   - "openai"      → any OpenAI-compatible HTTP server (Groq cloud, FastFlowLM
//!                     on the AMD NPU @ :52625, Ollama @ :11434, llama.cpp, ...)
//!   - "whisper-cpp" → in-process whisper.cpp on CPU/GPU (engine lands in v1)
//!
//! Adding a new OpenAI-compatible backend is ZERO code: add a `[providers.x]`
//! section with a base_url. The config FILE is the source of truth a future UI
//! reads/writes. Secrets never live in the file — a profile names an env var via
//! `api_key_env` (e.g. GROQ_API_KEY); local NPU/CPU servers need no key.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

pub const DEFAULT_STT_MODEL: &str = "whisper-large-v3-turbo";
pub const DEFAULT_LLM_MODEL: &str = "llama-3.3-70b-versatile";
pub const DEFAULT_CHUNK_THRESHOLD_BYTES: u64 = 24 * 1024 * 1024;
pub const DEFAULT_CHUNK_OVERLAP_SECS: u32 = 3;
pub const DEFAULT_BACKOFF_BASE_MS: u64 = 800;
pub const DEFAULT_LOCAL_THREADS: u32 = 4;

// ---------------------------------------------------------------------------
// Transport + provider profile
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Transport {
    /// OpenAI-compatible HTTP server (Groq, FastFlowLM, Ollama, llama.cpp, ...).
    #[serde(rename = "openai")]
    OpenAi,
    /// In-process whisper.cpp (CPU/GPU). Engine lands in v1.
    #[serde(rename = "whisper-cpp")]
    WhisperCpp,
}

/// A named backend profile.
#[derive(Debug, Clone)]
pub struct Provider {
    pub kind: Transport,
    /// OpenAI transport: server base URL (e.g. https://api.groq.com/openai/v1).
    pub base_url: Option<String>,
    /// Name of the env var holding this provider's API key (None = no key, e.g. local).
    pub api_key_env: Option<String>,
    /// Default STT model for this provider (overridable by `stt.model`).
    pub stt_model: Option<String>,
    /// Default chat/LLM model for this provider (overridable by `summary.model`).
    pub llm_model: Option<String>,
    /// whisper-cpp transport: path to a ggml/gguf model file.
    pub model_path: Option<PathBuf>,
    pub threads: u32,
}

impl Provider {
    /// Resolve this provider's API key from the env var it names (if any).
    pub fn resolve_key(&self) -> Option<String> {
        self.api_key_env
            .as_ref()
            .and_then(|name| std::env::var(name).ok())
            .filter(|v| !v.is_empty())
    }

    fn openai(base_url: &str, key_env: Option<&str>) -> Provider {
        Provider {
            kind: Transport::OpenAi,
            base_url: Some(base_url.into()),
            api_key_env: key_env.map(|s| s.into()),
            stt_model: None,
            llm_model: None,
            model_path: None,
            threads: DEFAULT_LOCAL_THREADS,
        }
    }
}

// ---------------------------------------------------------------------------
// Resolved runtime config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Config {
    /// ISO-639-1 output language for summary/translation (e.g. "tr").
    pub output_lang: String,
    pub stt_provider: String,
    pub stt_model_override: Option<String>,
    pub summary_provider: String,
    pub summary_model_override: Option<String>,
    pub providers: BTreeMap<String, Provider>,
    pub chunk_threshold_bytes: u64,
    pub chunk_overlap_secs: u32,
    pub backoff_base_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        let mut providers = BTreeMap::new();
        // Groq cloud.
        let mut groq = Provider::openai("https://api.groq.com/openai/v1", Some("GROQ_API_KEY"));
        groq.stt_model = Some(DEFAULT_STT_MODEL.into());
        groq.llm_model = Some(DEFAULT_LLM_MODEL.into());
        providers.insert("groq".into(), groq);
        // FastFlowLM on the AMD Ryzen AI NPU (OpenAI-compatible, no key).
        // Server: `flm serve gemma4-it:e2b --asr 1` exposes BOTH whisper ASR
        // (/v1/audio/transcriptions) AND the LLM (/v1/chat/completions) on one
        // port — so stt + summary can both point here for a fully-local pipeline.
        // gemma4-it:e2b has strong Turkish output.
        let mut flm = Provider::openai("http://localhost:52625/v1", None);
        flm.stt_model = Some("whisper-v3:turbo".into());
        flm.llm_model = Some("gemma4-it:e2b".into());
        providers.insert("fastflowlm".into(), flm);
        // Ollama (OpenAI-compatible endpoint, no key).
        let mut ollama = Provider::openai("http://localhost:11434/v1", None);
        ollama.llm_model = Some("llama3.1".into());
        providers.insert("ollama".into(), ollama);
        // In-process whisper.cpp (v1).
        providers.insert(
            "whispercpp".into(),
            Provider {
                kind: Transport::WhisperCpp,
                base_url: None,
                api_key_env: None,
                stt_model: None,
                llm_model: None,
                model_path: None,
                threads: DEFAULT_LOCAL_THREADS,
            },
        );

        Config {
            output_lang: "tr".into(),
            stt_provider: "groq".into(),
            stt_model_override: None,
            summary_provider: "groq".into(),
            summary_model_override: None,
            providers,
            chunk_threshold_bytes: DEFAULT_CHUNK_THRESHOLD_BYTES,
            chunk_overlap_secs: DEFAULT_CHUNK_OVERLAP_SECS,
            backoff_base_ms: DEFAULT_BACKOFF_BASE_MS,
        }
    }
}

impl Config {
    /// Load config: defaults → file (if present) → env overrides.
    pub fn load() -> Result<Config> {
        let mut cfg = Config::default();
        let path = config_path()?;
        if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("config okunamadı: {}", path.display()))?;
            let file: FileConfig = toml::from_str(&raw)
                .with_context(|| format!("config TOML hatalı: {}", path.display()))?;
            file.apply_to(&mut cfg);
        }
        cfg.apply_env();
        Ok(cfg)
    }

    pub fn stt_profile(&self) -> Result<&Provider> {
        self.providers
            .get(&self.stt_provider)
            .ok_or_else(|| anyhow!("Bilinmeyen STT provider'ı: '{}'. config'de [providers.{}] tanımla.", self.stt_provider, self.stt_provider))
    }

    pub fn summary_profile(&self) -> Result<&Provider> {
        self.providers
            .get(&self.summary_provider)
            .ok_or_else(|| anyhow!("Bilinmeyen özet provider'ı: '{}'. config'de [providers.{}] tanımla.", self.summary_provider, self.summary_provider))
    }

    pub fn effective_stt_model(&self) -> String {
        self.stt_model_override
            .clone()
            .or_else(|| self.stt_profile().ok().and_then(|p| p.stt_model.clone()))
            .unwrap_or_else(|| DEFAULT_STT_MODEL.into())
    }

    pub fn effective_summary_model(&self) -> String {
        self.summary_model_override
            .clone()
            .or_else(|| self.summary_profile().ok().and_then(|p| p.llm_model.clone()))
            .unwrap_or_else(|| DEFAULT_LLM_MODEL.into())
    }

    /// Reject transports whose engines aren't implemented yet (v0).
    /// OpenAI transport (Groq, FastFlowLM/NPU, Ollama) all work now.
    pub fn ensure_v0_supported(&self) -> Result<()> {
        if self.stt_profile()?.kind == Transport::WhisperCpp {
            bail!("In-process whisper.cpp STT v1'de gelecek. NPU için provider = \"fastflowlm\", bulut için \"groq\" kullan.");
        }
        if self.summary_profile()?.kind == Transport::WhisperCpp {
            bail!("whisper-cpp bir özet/LLM transport'u değil. summary.provider = \"groq\" veya \"ollama\" kullan.");
        }
        Ok(())
    }

    fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("TUTANAK_OUTPUT_LANG") {
            self.output_lang = v;
        }
        if let Ok(v) = std::env::var("TUTANAK_STT_PROVIDER") {
            self.stt_provider = v;
        }
        if let Ok(v) = std::env::var("TUTANAK_STT_MODEL") {
            self.stt_model_override = Some(v);
        }
        if let Ok(v) = std::env::var("TUTANAK_SUMMARY_PROVIDER") {
            self.summary_provider = v;
        }
        if let Ok(v) = std::env::var("TUTANAK_LLM_MODEL") {
            self.summary_model_override = Some(v);
        }
        // Back-compat: a bare GROQ_URL override still points the groq profile.
        if let Ok(v) = std::env::var("TUTANAK_GROQ_URL") {
            if let Some(p) = self.providers.get_mut("groq") {
                p.base_url = Some(v);
            }
        }
        if let Some(v) = env_parse("TUTANAK_CHUNK_BYTES") {
            self.chunk_threshold_bytes = v;
        }
        if let Some(v) = env_parse("TUTANAK_CHUNK_OVERLAP_SECS") {
            self.chunk_overlap_secs = v;
        }
        if let Some(v) = env_parse("TUTANAK_BACKOFF_MS") {
            self.backoff_base_ms = v;
        }
    }

    /// Serialize the current config to TOML (round-trips through `FileConfig`).
    pub fn to_toml(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("output_lang = \"{}\"\n\n", self.output_lang));

        s.push_str(&format!("[stt]\nprovider = \"{}\"\n", self.stt_provider));
        if let Some(m) = &self.stt_model_override {
            s.push_str(&format!("model = \"{m}\"\n"));
        }
        s.push('\n');

        s.push_str(&format!("[summary]\nprovider = \"{}\"\n", self.summary_provider));
        if let Some(m) = &self.summary_model_override {
            s.push_str(&format!("model = \"{m}\"\n"));
        }
        s.push('\n');

        for (name, p) in &self.providers {
            s.push_str(&format!("[providers.{name}]\n"));
            let kind = match p.kind {
                Transport::OpenAi => "openai",
                Transport::WhisperCpp => "whisper-cpp",
            };
            s.push_str(&format!("kind = \"{kind}\"\n"));
            if let Some(u) = &p.base_url {
                s.push_str(&format!("base_url = \"{u}\"\n"));
            }
            if let Some(e) = &p.api_key_env {
                s.push_str(&format!("api_key_env = \"{e}\"\n"));
            }
            if let Some(m) = &p.stt_model {
                s.push_str(&format!("stt_model = \"{m}\"\n"));
            }
            if let Some(m) = &p.llm_model {
                s.push_str(&format!("llm_model = \"{m}\"\n"));
            }
            if let Some(mp) = &p.model_path {
                s.push_str(&format!("model_path = \"{}\"\n", mp.display()));
            }
            s.push_str(&format!("threads = {}\n\n", p.threads));
        }

        s.push_str(&format!(
            "[chunking]\nthreshold_bytes = {}\noverlap_secs = {}\n\n",
            self.chunk_threshold_bytes, self.chunk_overlap_secs
        ));
        s.push_str(&format!("[network]\nbackoff_base_ms = {}\n", self.backoff_base_ms));
        s
    }

    /// Persist the current config to `config_path()`. Returns the path written.
    pub fn save(&self) -> Result<PathBuf> {
        let path = config_path()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).context("config dizini oluşturulamadı")?;
        }
        std::fs::write(&path, self.to_toml()).context("config dosyası yazılamadı")?;
        Ok(path)
    }

    /// Redacted, human-readable view of the effective config (no secret values).
    pub fn describe(&self) -> String {
        let mut s = format!(
            "output_lang = {}\n[stt]     provider = {}  model = {}\n[summary] provider = {}  model = {}\n",
            self.output_lang,
            self.stt_provider,
            self.effective_stt_model(),
            self.summary_provider,
            self.effective_summary_model(),
        );
        s.push_str("providers:\n");
        for (name, p) in &self.providers {
            let key = match &p.api_key_env {
                Some(env) => format!("key<{}>={}", env, if p.resolve_key().is_some() { "ayarlı" } else { "yok" }),
                None => "key=yok".into(),
            };
            s.push_str(&format!(
                "  {name}: kind={:?} base_url={} {key}\n",
                p.kind,
                p.base_url.as_deref().unwrap_or("-"),
            ));
        }
        s.push_str(&format!(
            "[chunking] threshold_bytes={} overlap_secs={}\n[network] backoff_base_ms={}",
            self.chunk_threshold_bytes, self.chunk_overlap_secs, self.backoff_base_ms
        ));
        s
    }
}

// ---------------------------------------------------------------------------
// File schema
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    output_lang: Option<String>,
    stt: Option<FileSelect>,
    summary: Option<FileSelect>,
    providers: Option<BTreeMap<String, FileProvider>>,
    chunking: Option<FileChunking>,
    network: Option<FileNetwork>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSelect {
    provider: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileProvider {
    kind: Option<Transport>,
    base_url: Option<String>,
    api_key_env: Option<String>,
    stt_model: Option<String>,
    llm_model: Option<String>,
    model_path: Option<String>,
    threads: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileChunking {
    threshold_bytes: Option<u64>,
    overlap_secs: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileNetwork {
    backoff_base_ms: Option<u64>,
}

impl FileConfig {
    fn apply_to(self, cfg: &mut Config) {
        if let Some(v) = self.output_lang {
            cfg.output_lang = v;
        }
        if let Some(stt) = self.stt {
            if let Some(p) = stt.provider {
                cfg.stt_provider = p;
            }
            if let Some(m) = stt.model {
                cfg.stt_model_override = Some(m);
            }
        }
        if let Some(sum) = self.summary {
            if let Some(p) = sum.provider {
                cfg.summary_provider = p;
            }
            if let Some(m) = sum.model {
                cfg.summary_model_override = Some(m);
            }
        }
        if let Some(provs) = self.providers {
            for (name, fp) in provs {
                // Merge into an existing built-in profile, or create a new one.
                let entry = cfg.providers.entry(name).or_insert_with(|| Provider {
                    kind: Transport::OpenAi,
                    base_url: None,
                    api_key_env: None,
                    stt_model: None,
                    llm_model: None,
                    model_path: None,
                    threads: DEFAULT_LOCAL_THREADS,
                });
                if let Some(k) = fp.kind {
                    entry.kind = k;
                }
                if let Some(u) = fp.base_url {
                    entry.base_url = Some(u);
                }
                if let Some(e) = fp.api_key_env {
                    entry.api_key_env = Some(e);
                }
                if let Some(m) = fp.stt_model {
                    entry.stt_model = Some(m);
                }
                if let Some(m) = fp.llm_model {
                    entry.llm_model = Some(m);
                }
                if let Some(p) = fp.model_path {
                    entry.model_path = Some(expand_tilde(&p));
                }
                if let Some(t) = fp.threads {
                    entry.threads = t;
                }
            }
        }
        if let Some(c) = self.chunking {
            if let Some(v) = c.threshold_bytes {
                cfg.chunk_threshold_bytes = v;
            }
            if let Some(v) = c.overlap_secs {
                cfg.chunk_overlap_secs = v;
            }
        }
        if let Some(n) = self.network {
            if let Some(v) = n.backoff_base_ms {
                cfg.backoff_base_ms = v;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Paths + scaffolding
// ---------------------------------------------------------------------------

pub fn config_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("TUTANAK_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    let base = dirs::config_dir().context("XDG config dizini bulunamadı")?;
    Ok(base.join("tutanak").join("config.toml"))
}

pub const TEMPLATE: &str = r#"# tutanak yapılandırması
# Katmanlama: varsayılan -> bu dosya -> ortam değişkenleri (env üstün)
# Sır (API anahtarı) burada DEĞİL: provider'ın api_key_env'i ile env'den okunur.

output_lang = "tr"     # özet/çeviri çıktı dili (ISO-639-1)

# Hangi provider profili kullanılsın (aşağıdaki [providers.*] tablosundan):
[stt]
provider = "groq"      # "groq" (bulut) | "fastflowlm" (AMD NPU) | "whispercpp" (CPU, v1)
# model = "whisper-large-v3-turbo"   # opsiyonel; boşsa profilin default'u

[summary]
provider = "groq"      # "groq" | "ollama" (yerel LLM)
# model = "llama-3.3-70b-versatile"

# --- Provider registry: yeni bir OpenAI-uyumlu backend = sıfır kod, sadece profil ---

[providers.groq]
kind = "openai"
base_url = "https://api.groq.com/openai/v1"
api_key_env = "GROQ_API_KEY"
stt_model = "whisper-large-v3-turbo"
llm_model = "llama-3.3-70b-versatile"

# AMD Ryzen AI NPU — whisper V3 turbo + LLM, OpenAI-uyumlu @ :52625.
# Sunucu (ikisini birden, tek port): flm serve gemma4-it:e2b --asr 1
# Sadece ASR (standalone):           flm serve --asr 1
# Hem stt hem summary'yi buna yöneltirsen pipeline %100 yerel/NPU olur.
# gemma4-it:e2b Türkçe çıktıda güçlü.
[providers.fastflowlm]
kind = "openai"
base_url = "http://localhost:52625/v1"
stt_model = "whisper-v3:turbo"
llm_model = "gemma4-it:e2b"

[providers.ollama]          # yerel LLM (özet için)
kind = "openai"
base_url = "http://localhost:11434/v1"
llm_model = "llama3.1"

[providers.whispercpp]      # in-process whisper.cpp (v1'de gelecek)
kind = "whisper-cpp"
# model_path = "~/.local/share/tutanak/models/ggml-large-v3.bin"
threads = 4

[chunking]
threshold_bytes = 25165824   # ~24MB (Groq free=25MB, dev=100MB; yerel için sınır yok sayılır)
overlap_secs = 3

[network]
backoff_base_ms = 800
"#;

/// Write the template config if it doesn't exist (or `force`). Returns (path, created?).
pub fn init_file(force: bool) -> Result<(PathBuf, bool)> {
    let path = config_path()?;
    if path.exists() && !force {
        return Ok((path, false));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).context("config dizini oluşturulamadı")?;
    }
    std::fs::write(&path, TEMPLATE).context("config dosyası yazılamadı")?;
    Ok((path, true))
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(p)
}

#[allow(dead_code)]
fn _p(_: &Path) {}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_groq_and_fastflowlm() {
        let c = Config::default();
        assert_eq!(c.stt_provider, "groq");
        assert_eq!(c.effective_stt_model(), DEFAULT_STT_MODEL);
        let flm = c.providers.get("fastflowlm").unwrap();
        assert_eq!(flm.kind, Transport::OpenAi);
        assert_eq!(flm.base_url.as_deref(), Some("http://localhost:52625/v1"));
        assert!(flm.api_key_env.is_none()); // local NPU, no key
    }

    #[test]
    fn fastflowlm_is_v0_supported_but_whispercpp_is_not() {
        let mut c = Config::default();
        c.stt_provider = "fastflowlm".into();
        assert!(c.ensure_v0_supported().is_ok(), "FastFlowLM (openai transport) v0'da çalışmalı");

        c.stt_provider = "whispercpp".into();
        assert!(c.ensure_v0_supported().is_err());
    }

    #[test]
    fn file_selects_provider_and_overrides_model() {
        let toml = r#"
            [stt]
            provider = "fastflowlm"
            model = "whisper-v3-turbo"
            [providers.fastflowlm]
            base_url = "http://127.0.0.1:52625/v1"
        "#;
        let file: FileConfig = toml::from_str(toml).unwrap();
        let mut c = Config::default();
        file.apply_to(&mut c);
        assert_eq!(c.stt_provider, "fastflowlm");
        assert_eq!(c.effective_stt_model(), "whisper-v3-turbo");
        assert_eq!(
            c.stt_profile().unwrap().base_url.as_deref(),
            Some("http://127.0.0.1:52625/v1")
        );
    }

    #[test]
    fn file_can_add_new_provider() {
        let toml = r#"
            [providers.myserver]
            kind = "openai"
            base_url = "http://192.168.1.5:8000/v1"
            stt_model = "whisper-large-v3"
        "#;
        let file: FileConfig = toml::from_str(toml).unwrap();
        let mut c = Config::default();
        file.apply_to(&mut c);
        let p = c.providers.get("myserver").unwrap();
        assert_eq!(p.base_url.as_deref(), Some("http://192.168.1.5:8000/v1"));
    }

    #[test]
    fn template_parses() {
        let _: FileConfig = toml::from_str(TEMPLATE).unwrap();
    }

    #[test]
    fn to_toml_round_trips() {
        let mut c = Config::default();
        c.summary_provider = "ollama".into();
        c.output_lang = "en".into();
        c.providers.get_mut("groq").unwrap().base_url = Some("http://x/v1".into());

        let toml = c.to_toml();
        let file: FileConfig = toml::from_str(&toml).expect("kendi ürettiğimiz TOML parse edilmeli");
        let mut back = Config::default();
        file.apply_to(&mut back);

        assert_eq!(back.summary_provider, "ollama");
        assert_eq!(back.output_lang, "en");
        assert_eq!(back.providers.get("groq").unwrap().base_url.as_deref(), Some("http://x/v1"));
        assert_eq!(back.providers.get("fastflowlm").unwrap().base_url.as_deref(), Some("http://localhost:52625/v1"));
    }

    #[test]
    fn resolve_key_reads_named_env() {
        let p = Provider::openai("http://x/v1", Some("TUTANAK_TEST_KEY_XYZ"));
        assert!(p.resolve_key().is_none());
        std::env::set_var("TUTANAK_TEST_KEY_XYZ", "secret");
        assert_eq!(p.resolve_key().as_deref(), Some("secret"));
        std::env::remove_var("TUTANAK_TEST_KEY_XYZ");
    }
}
