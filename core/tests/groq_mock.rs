//! Integration tests for the OpenAI-compatible engine against a mock HTTP server
//! (httpmock). Deterministic: happy path, per-chunk resume, auth error, 413.
//! A real-API smoke test lives behind an env flag (see `real_smoke`).
//!
//! The mock stands in for ANY OpenAI-compatible provider (Groq, FastFlowLM,
//! Ollama) — they share the same engine, differing only by base_url.

#![allow(clippy::field_reassign_with_default)]

use std::path::Path;

use httpmock::prelude::*;
use tutanak_core::config::Config;
use tutanak_core::engine;

/// A config whose `groq` provider points at the mock server (no auth).
fn test_cfg(base_url: String) -> Config {
    let mut c = Config::default();
    c.backoff_base_ms = 2;
    c.chunk_threshold_bytes = 64 * 1024 * 1024; // single chunk for small test WAVs
    c.chunk_overlap_secs = 0;
    let g = c.providers.get_mut("groq").unwrap();
    g.base_url = Some(base_url);
    g.api_key_env = None; // mock doesn't check auth
    c
}

fn write_silence(path: &Path, secs: u32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for _ in 0..(16_000 * secs) {
        w.write_sample(0i16).unwrap();
    }
    w.finalize().unwrap();
}

#[test]
fn happy_transcribe_then_summarize() {
    let server = MockServer::start();
    let stt = server.mock(|when, then| {
        when.method(POST).path("/audio/transcriptions");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({"text": "hello world", "language": "english"}));
    });
    let chat = server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "choices": [{"message": {"content": "- karar: gönder"}}]
            }));
    });

    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("in.wav");
    write_silence(&wav, 1);
    let cfg = test_cfg(server.base_url());

    let stt_engine = engine::build_stt(&cfg).unwrap();
    let out = stt_engine.transcribe(&wav, &dir.path().join("job")).unwrap();
    assert_eq!(out.transcript, "hello world");
    assert_eq!(out.language, "en");
    stt.assert();

    let llm = engine::build_llm(&cfg).unwrap();
    let summary = llm.summarize(&out.transcript, "tr").unwrap();
    assert!(summary.contains("karar"));
    chat.assert();
}

#[test]
fn resume_skips_network_on_second_run() {
    let server = MockServer::start();
    let stt = server.mock(|when, then| {
        when.method(POST).path("/audio/transcriptions");
        then.status(200)
            .json_body(serde_json::json!({"text": "resumed text", "language": "turkish"}));
    });

    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("in.wav");
    write_silence(&wav, 1);
    let job = dir.path().join("job");
    let cfg = test_cfg(server.base_url());

    let first = engine::build_stt(&cfg).unwrap().transcribe(&wav, &job).unwrap();
    assert_eq!(first.transcript, "resumed text");
    let hits_after_first = stt.hits();
    assert!(hits_after_first >= 1);

    let second = engine::build_stt(&cfg).unwrap().transcribe(&wav, &job).unwrap();
    assert_eq!(second.transcript, "resumed text");
    assert_eq!(stt.hits(), hits_after_first, "resume should not re-hit the API");
}

#[test]
fn auth_error_is_helpful() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/audio/transcriptions");
        then.status(401).body("unauthorized");
    });

    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("in.wav");
    write_silence(&wav, 1);
    let cfg = test_cfg(server.base_url());

    let err = engine::build_stt(&cfg)
        .unwrap()
        .transcribe(&wav, &dir.path().join("job"))
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("api_key_env") || msg.contains("yetkisiz"), "msg: {msg}");
}

#[test]
fn payload_too_large_at_min_chunk_errors() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/audio/transcriptions");
        then.status(413).body("payload too large");
    });

    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("in.wav");
    write_silence(&wav, 1); // == MIN_CHUNK_FRAMES, cannot subdivide further
    let cfg = test_cfg(server.base_url());

    let err = engine::build_stt(&cfg)
        .unwrap()
        .transcribe(&wav, &dir.path().join("job"))
        .unwrap_err();
    assert!(format!("{err:#}").contains("413"));
}

/// Opt-in real-API smoke. Run with:
///   GROQ_API_KEY=... TUTANAK_REAL_SMOKE=1 cargo test -- --ignored real_smoke
#[test]
#[ignore]
fn real_smoke() {
    if std::env::var("TUTANAK_REAL_SMOKE").is_err() {
        eprintln!("TUTANAK_REAL_SMOKE ayarlı değil — atlanıyor");
        return;
    }
    let cfg = Config::load().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("in.wav");
    write_silence(&wav, 2);
    let out = engine::build_stt(&cfg)
        .unwrap()
        .transcribe(&wav, &dir.path().join("job"))
        .unwrap();
    eprintln!("gerçek transkript: {:?}", out);
}
