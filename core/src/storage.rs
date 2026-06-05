//! Markdown note storage under the XDG data dir.
//!
//! v0 writes plain markdown files to `~/.local/share/tutanak/`. No SQLite
//! (that lands with search in v3). Data stays on the user's disk; no telemetry.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::Notes;

/// A saved note in the history list.
#[derive(Debug, Clone)]
pub struct NoteEntry {
    pub path: PathBuf,
    /// Display label: "YYYY-MM-DD HH:MM — <heading>".
    pub label: String,
    pub stamp: u64,
}

/// A note parsed back from disk for display.
#[derive(Debug, Clone, Default)]
pub struct LoadedNote {
    pub title: String,
    pub summary: String,
    pub translation: Option<String>,
    pub transcript: String,
}

/// List saved notes, newest first.
pub fn list_notes() -> Result<Vec<NoteEntry>> {
    let dir = notes_dir()?;
    let mut v = Vec::new();
    let Ok(rd) = fs::read_dir(&dir) else {
        return Ok(v); // dir not created yet → empty history
    };
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let fname = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let stamp = fname.split('-').next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let heading = first_heading(&path).unwrap_or_else(|| "(başlıksız)".to_string());
        let label = format!("{} — {}", fmt_stamp(stamp), heading);
        v.push(NoteEntry { path, label, stamp });
    }
    v.sort_by_key(|e| std::cmp::Reverse(e.stamp));
    Ok(v)
}

/// Parse a saved markdown note back into its sections.
pub fn load_note(path: &Path) -> Result<LoadedNote> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("not okunamadı: {}", path.display()))?;
    let mut n = LoadedNote::default();
    let mut translation = String::new();

    #[derive(PartialEq)]
    enum Sec {
        None,
        Summary,
        Translation,
        Transcript,
    }
    let mut sec = Sec::None;

    for line in content.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            sec = match h.trim() {
                "Özet" => Sec::Summary,
                "Çeviri" => Sec::Translation,
                "Transkript" => Sec::Transcript,
                _ => Sec::None,
            };
            continue;
        }
        if let Some(t) = line.strip_prefix("# ") {
            if n.title.is_empty() {
                n.title = t.trim().to_string();
            }
            continue;
        }
        match sec {
            Sec::Summary => {
                n.summary.push_str(line);
                n.summary.push('\n');
            }
            Sec::Translation => {
                translation.push_str(line);
                translation.push('\n');
            }
            Sec::Transcript => {
                n.transcript.push_str(line);
                n.transcript.push('\n');
            }
            Sec::None => {}
        }
    }
    n.summary = n.summary.trim().to_string();
    n.transcript = n.transcript.trim().to_string();
    let tr = translation.trim();
    n.translation = if tr.is_empty() { None } else { Some(tr.to_string()) };
    Ok(n)
}

/// First `# ` heading in a markdown file (the note title).
fn first_heading(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    content
        .lines()
        .find_map(|l| l.strip_prefix("# ").map(|h| h.trim().to_string()))
}

/// Format a unix timestamp (seconds) as "YYYY-MM-DD HH:MM" in UTC (no deps).
fn fmt_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    let (h, min) = (rem / 3600, (rem % 3600) / 60);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}")
}

/// Howard Hinnant's days-from-epoch → (year, month, day), UTC.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

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

    #[test]
    fn render_then_load_note_round_trips() {
        let md = render(&notes(), "Sprint");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("1700000000-sprint.md");
        std::fs::write(&path, md).unwrap();

        let loaded = load_note(&path).unwrap();
        assert_eq!(loaded.title, "Sprint");
        assert_eq!(loaded.summary, "- karar: ship");
        assert_eq!(loaded.transcript, "hello world");
        assert_eq!(loaded.translation.as_deref(), Some("merhaba dünya"));
    }

    #[test]
    fn fmt_stamp_known_value() {
        // 2021-01-01 00:00:00 UTC = 1609459200
        assert_eq!(fmt_stamp(1_609_459_200), "2021-01-01 00:00");
    }
}
