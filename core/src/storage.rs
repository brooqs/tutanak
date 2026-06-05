//! Markdown note storage under the XDG data dir.
//!
//! v0 writes plain markdown files to `~/.local/share/tutanak/`. No SQLite
//! (that lands with search in v3). Data stays on the user's disk; no telemetry.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::Notes;

/// Directory where notes are stored: `~/.local/share/tutanak/`.
pub fn notes_dir() -> Result<PathBuf> {
    let base = dirs::data_dir().context("XDG data dizini bulunamadı")?;
    Ok(base.join("tutanak"))
}

/// Render `notes` to markdown and write a timestamped file. Returns the path.
pub fn save_markdown(notes: &Notes, title: &str) -> Result<PathBuf> {
    let dir = notes_dir()?;
    fs::create_dir_all(&dir).context("notlar dizini oluşturulamadı")?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("{stamp}-{}.md", slug(title)));
    fs::write(&path, render(notes, title)).context("not dosyası yazılamadı")?;
    Ok(path)
}

/// Render notes to markdown (pure, so it's unit-testable).
pub fn render(notes: &Notes, title: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {title}\n\n"));
    s.push_str(&format!(
        "- Kaynak dil: {}\n- Çıktı dili: {}\n\n",
        if notes.language.is_empty() { "(bilinmiyor)" } else { &notes.language },
        notes.output_lang
    ));

    s.push_str("## Özet\n\n");
    s.push_str(notes.summary.trim());
    s.push_str("\n\n");

    if let Some(tr) = &notes.translation {
        s.push_str("## Çeviri\n\n");
        s.push_str(tr.trim());
        s.push_str("\n\n");
    }

    s.push_str("## Transkript\n\n");
    s.push_str(notes.transcript.trim());
    s.push('\n');
    s
}

fn slug(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let trimmed = s.trim_matches('-').to_string();
    let collapsed = trimmed.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if collapsed.is_empty() { "not".into() } else { collapsed }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes() -> Notes {
        Notes {
            language: "en".into(),
            output_lang: "tr".into(),
            transcript: "hello world".into(),
            translation: Some("merhaba dünya".into()),
            summary: "- karar: ship".into(),
        }
    }

    #[test]
    fn render_contains_all_sections() {
        let md = render(&notes(), "Toplantı");
        assert!(md.contains("# Toplantı"));
        assert!(md.contains("## Özet"));
        assert!(md.contains("## Çeviri"));
        assert!(md.contains("## Transkript"));
        assert!(md.contains("hello world"));
    }

    #[test]
    fn render_omits_translation_when_none() {
        let mut n = notes();
        n.translation = None;
        let md = render(&n, "x");
        assert!(!md.contains("## Çeviri"));
    }

    #[test]
    fn slug_is_filesystem_safe() {
        assert_eq!(slug("Hello, World!"), "hello-world");
        assert_eq!(slug("  ??? "), "not");
    }
}
