//! tutanak core: the full record→transcribe→translate→summarize→notes
//! pipeline, built on a provider registry.
//!
//!   capture (parec) ─► WAV ─► STT engine (Groq / FastFlowLM-NPU / whisper.cpp) ─► transcript
//!                                                                                  │
//!                       summary (output lang, map-reduce) ◄───── LLM engine ───────┤
//!                       translation (optional, if src≠dst) ◄──── (Groq / Ollama) ──┘
//!                                          │
//!                                       markdown notes
// ASCII pipeline diagrams in module docs are not markdown lists.
#![allow(clippy::doc_overindented_list_items)]

pub mod audio;
pub mod capture;
pub mod config;
pub mod engine;
pub mod storage;
pub mod stitch;
pub mod text;

use std::path::Path;

use anyhow::Result;

pub use config::Config;
pub use engine::TranscribeOutput;

/// The product of the pipeline: a meeting's notes.
#[derive(Debug, Clone)]
pub struct Notes {
    /// Detected source language (normalized 2-letter, best-effort).
    pub language: String,
    /// Output language for summary/translation.
    pub output_lang: String,
    pub transcript: String,
    /// Full translation of the transcript into `output_lang`, if produced.
    pub translation: Option<String>,
    /// Summary in `output_lang`.
    pub summary: String,
}

/// Options controlling the pipeline run.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Also produce a full transcript translation (cost-heavy on long meetings).
    /// Auto-skipped when the source language already equals the output language.
    pub translate: bool,
    /// Stable id for the resume directory; reuse it to resume a failed run.
    pub job_id: String,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions { translate: false, job_id: "default".into() }
    }
}

/// Coarse pipeline stage, reported to a frontend (GUI status line, CLI log).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Transcribing,
    Translating,
    Summarizing,
}

/// Run the full pipeline on a 16kHz mono WAV at `wav_path`, using the engines
/// selected by `cfg`'s provider profiles. (CLI entry point — no progress sink.)
pub fn run_pipeline(cfg: &Config, wav_path: &Path, opts: &RunOptions) -> Result<Notes> {
    run_pipeline_staged(cfg, wav_path, opts, &mut |_| {})
}

/// Like [`run_pipeline`], but reports each coarse [`Stage`] to `on_stage` so a
/// GUI can show live status. Per-chunk detail still goes to stderr.
pub fn run_pipeline_staged(
    cfg: &Config,
    wav_path: &Path,
    opts: &RunOptions,
    on_stage: &mut dyn FnMut(Stage),
) -> Result<Notes> {
    cfg.ensure_v0_supported()?;
    let stt = engine::build_stt(cfg)?;
    let llm = engine::build_llm(cfg)?;
    let job_dir = job_dir(&opts.job_id)?;

    on_stage(Stage::Transcribing);
    eprintln!("Transkripsiyon [{}]...", stt.label());
    let t = stt.transcribe(wav_path, &job_dir)?;

    let want_translation = opts.translate && t.language != cfg.output_lang;
    let translation = if want_translation {
        on_stage(Stage::Translating);
        eprintln!("Çeviri [{}] ({} → {})...", llm.label(), t.language, cfg.output_lang);
        Some(llm.translate(&t.transcript, &cfg.output_lang)?)
    } else {
        if opts.translate {
            eprintln!("Çeviri atlandı: kaynak dil zaten {}", cfg.output_lang);
        }
        None
    };

    on_stage(Stage::Summarizing);
    eprintln!("Özet [{}] ({})...", llm.label(), cfg.output_lang);
    let summary = llm.summarize(&t.transcript, &cfg.output_lang)?;

    Ok(Notes {
        language: t.language,
        output_lang: cfg.output_lang.clone(),
        transcript: t.transcript,
        translation,
        summary,
    })
}

/// Resume directory: `~/.cache/tutanak/<job_id>/`.
fn job_dir(job_id: &str) -> Result<std::path::PathBuf> {
    use anyhow::Context;
    let base = dirs::cache_dir().context("XDG cache dizini bulunamadı")?;
    Ok(base.join("tutanak").join(sanitize(job_id)))
}

fn sanitize(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if out.is_empty() { "default".into() } else { out }
}
