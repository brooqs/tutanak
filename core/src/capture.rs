//! System-audio capture via the PulseAudio API (`parecord`).
//!
//! Decision (plan-eng-review X3): target the PulseAudio API, not pw-record —
//! works on PulseAudio AND PipeWire (pipewire-pulse shim), widest coverage.
//!
//! Two capture sources:
//!  - `SystemOnly`   — the default sink's `.monitor` (what plays out of the
//!                     speakers, i.e. the OTHER meeting participants).
//!  - `MicAndSystem` — a "what-u-hear + mic" mix so the user's OWN voice is in
//!                     the recording too. Built on the fly:
//!
//!      mic source ──loopback──┐
//!                             ├──► null-sink "tutanak_mix" ──.monitor──► parecord
//!      sink.monitor ─loopback─┘
//!
//!    The modules are unloaded again on stop (clean teardown).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

/// What to capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Only system playback (other participants); excludes the local mic.
    SystemOnly,
    /// System playback mixed with the local microphone (full meeting).
    MicAndSystem,
}

/// A live recording. Holds the `parecord` child and any PulseAudio modules that
/// must be torn down on [`stop`].
pub struct Session {
    child: Child,
    /// pactl module ids to unload on stop (in reverse load order).
    modules: Vec<String>,
    pub wav: PathBuf,
}

/// Start recording `source` to `out` as 16kHz mono WAV.
pub fn start(source: Source, out: &Path) -> Result<Session> {
    match source {
        Source::SystemOnly => {
            let monitor = default_monitor_source()?;
            let child = spawn_parecord(&monitor, out)?;
            Ok(Session { child, modules: Vec::new(), wav: out.to_path_buf() })
        }
        Source::MicAndSystem => start_mixed(out),
    }
}

/// Stop a recording, finalizing the WAV and unloading any mix modules.
pub fn stop(session: Session) -> Result<()> {
    let Session { child, modules, .. } = session;
    let r = sigint_and_wait(child);
    // Always tear down modules, even if the child wait errored.
    for id in modules.iter().rev() {
        let _ = unload_module(id);
    }
    r
}

fn start_mixed(out: &Path) -> Result<Session> {
    let monitor = default_monitor_source()?;
    let mic = default_mic_source()?;
    let sink_name = format!("tutanak_mix_{}", std::process::id());
    let mut modules: Vec<String> = Vec::new();

    let built = (|| -> Result<Child> {
        modules.push(load_module(&[
            "module-null-sink".into(),
            format!("sink_name={sink_name}"),
            "sink_properties=device.description=tutanak-mix".into(),
        ])?);
        // Mic → mix
        modules.push(load_module(&[
            "module-loopback".into(),
            format!("source={mic}"),
            format!("sink={sink_name}"),
            "latency_msec=30".into(),
        ])?);
        // System (other participants) → mix
        modules.push(load_module(&[
            "module-loopback".into(),
            format!("source={monitor}"),
            format!("sink={sink_name}"),
            "latency_msec=30".into(),
        ])?);
        std::thread::sleep(Duration::from_millis(150)); // let routing settle
        spawn_parecord(&format!("{sink_name}.monitor"), out)
    })();

    match built {
        Ok(child) => Ok(Session { child, modules, wav: out.to_path_buf() }),
        Err(e) => {
            for id in modules.iter().rev() {
                let _ = unload_module(id);
            }
            Err(e).context("mic+sistem mix kurulamadı")
        }
    }
}

/// Monitor source for the current default sink (fallback: first `.monitor`).
pub fn default_monitor_source() -> Result<String> {
    if let Some(sink) = default_of("get-default-sink") {
        if !sink.is_empty() {
            return Ok(format!("{sink}.monitor"));
        }
    }
    let out = pactl(&["list", "short", "sources"])?;
    for line in out.lines() {
        if let Some(name) = line.split_whitespace().nth(1) {
            if name.ends_with(".monitor") {
                return Ok(name.to_string());
            }
        }
    }
    Err(anyhow!("Sistem sesi (monitor source) bulunamadı. Çıkış cihazını kontrol et."))
}

/// The default microphone source.
pub fn default_mic_source() -> Result<String> {
    default_of("get-default-source")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Varsayılan mikrofon bulunamadı. Giriş cihazını kontrol et."))
}

/// Normalize an arbitrary audio/video file to 16kHz mono WAV via ffmpeg.
pub fn normalize_to_wav(input: &Path, out: &Path) -> Result<()> {
    let status = Command::new("ffmpeg")
        .args(["-y", "-i"])
        .arg(input)
        .args(["-ar", "16000", "-ac", "1", "-f", "wav"])
        .arg(out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("ffmpeg çalıştırılamadı. Gerekli: `sudo pacman -S ffmpeg` (veya dağıtım eşdeğeri)")?;
    if !status.success() {
        bail!("ffmpeg dosyayı dönüştüremedi: {}", input.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// internals
// ---------------------------------------------------------------------------

fn spawn_parecord(device: &str, out: &Path) -> Result<Child> {
    Command::new("parecord")
        .arg(format!("--device={device}"))
        .arg("--format=s16le")
        .arg("--rate=16000")
        .arg("--channels=1")
        .arg("--file-format=wav")
        .arg(out)
        .stdin(Stdio::null())
        .spawn()
        .context("parecord başlatılamadı (pulseaudio-utils kurulu mu?)")
}

/// SIGINT (not SIGKILL) so parecord finalizes the WAV header, then wait.
fn sigint_and_wait(mut child: Child) -> Result<()> {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        // SAFETY: pid is a live child we own; SIGINT is a defined signal.
        let rc = unsafe { libc::kill(pid, libc::SIGINT) };
        if rc != 0 {
            let _ = child.kill();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    child.wait().context("parecord beklenirken hata")?;
    Ok(())
}

fn default_of(subcommand: &str) -> Option<String> {
    let out = Command::new("pactl").arg(subcommand).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn pactl(args: &[&str]) -> Result<String> {
    let out = Command::new("pactl")
        .args(args)
        .output()
        .context("pactl çalıştırılamadı (PulseAudio/PipeWire kurulu mu?)")?;
    if !out.status.success() {
        bail!("pactl {:?} başarısız", args);
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn load_module(args: &[String]) -> Result<String> {
    let out = Command::new("pactl")
        .arg("load-module")
        .args(args)
        .output()
        .context("pactl load-module çalıştırılamadı")?;
    if !out.status.success() {
        bail!(
            "pactl load-module {} başarısız: {}",
            args.first().map(|s| s.as_str()).unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn unload_module(id: &str) -> Result<()> {
    Command::new("pactl")
        .args(["unload-module", id])
        .status()
        .context("pactl unload-module çalıştırılamadı")?;
    Ok(())
}
