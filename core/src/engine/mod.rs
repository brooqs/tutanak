//! Engine registry: pick a concrete STT / LLM engine from the config's selected
//! provider profile.
//!
//! Decision (revised at the user's request, supersedes plan-eng-review X2): the
//! provider abstraction is built NOW because there are genuinely multiple real
//! backends — Groq (cloud), FastFlowLM (AMD NPU), Ollama (local LLM), and later
//! in-process whisper.cpp. With several concrete shapes in hand, the trait is no
//! longer premature.
//!
//!   Config.stt_provider ─► stt_profile().kind
//!                              ├─ OpenAi      ─► OpenAiStt   (Groq / FastFlowLM / ...)
//!                              └─ WhisperCpp  ─► (v1)
//!   Config.summary_provider ─► summary_profile().kind
//!                              ├─ OpenAi      ─► OpenAiLlm   (Groq / Ollama / ...)
//!                              └─ WhisperCpp  ─► (invalid for LLM)

pub mod openai;

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::config::{Config, Transport};

/// Result of transcribing audio.
#[derive(Debug, Clone)]
pub struct TranscribeOutput {
    pub transcript: String,
    /// Normalized 2-letter source language (best-effort), e.g. "en".
    pub language: String,
}

/// A speech-to-text backend.
pub trait SttEngine {
    fn transcribe(&self, wav_path: &Path, job_dir: &Path) -> Result<TranscribeOutput>;
    /// Human-readable label, e.g. "groq (whisper-large-v3-turbo)".
    fn label(&self) -> String;
}

/// A translate + summarize backend.
pub trait LlmEngine {
    fn summarize(&self, transcript: &str, target_lang: &str) -> Result<String>;
    fn translate(&self, text: &str, target_lang: &str) -> Result<String>;
    fn label(&self) -> String;
}

/// Build the STT engine for the configured provider.
pub fn build_stt(cfg: &Config) -> Result<Box<dyn SttEngine>> {
    let p = cfg.stt_profile()?;
    match p.kind {
        Transport::OpenAi => {
            let base = p
                .base_url
                .clone()
                .with_context(|| format!("'{}' provider'ında base_url eksik", cfg.stt_provider))?;
            Ok(Box::new(openai::OpenAiStt::new(
                cfg.stt_provider.clone(),
                base,
                p.resolve_key(),
                cfg.effective_stt_model(),
                cfg.chunk_threshold_bytes,
                cfg.chunk_overlap_secs,
                cfg.backoff_base_ms,
            )?))
        }
        Transport::WhisperCpp => bail!(
            "In-process whisper.cpp STT v1'de gelecek. NPU için provider = \"fastflowlm\", bulut için \"groq\"."
        ),
    }
}

/// Build the translate/summarize engine for the configured provider.
pub fn build_llm(cfg: &Config) -> Result<Box<dyn LlmEngine>> {
    let p = cfg.summary_profile()?;
    match p.kind {
        Transport::OpenAi => {
            let base = p.base_url.clone().with_context(|| {
                format!("'{}' provider'ında base_url eksik", cfg.summary_provider)
            })?;
            Ok(Box::new(openai::OpenAiLlm::new(
                cfg.summary_provider.clone(),
                base,
                p.resolve_key(),
                cfg.effective_summary_model(),
                cfg.backoff_base_ms,
            )?))
        }
        Transport::WhisperCpp => {
            bail!("whisper-cpp bir özet/LLM transport'u değil. summary.provider = \"groq\" veya \"ollama\".")
        }
    }
}
