//! tutanak v0 CLI: capture or import audio, run the pipeline, write
//! meeting notes as markdown. Engines are chosen via config provider profiles
//! (Groq cloud, FastFlowLM NPU, Ollama, ...).

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tutanak_core::{capture, config, config::Config, run_pipeline, storage, RunOptions};

#[derive(Parser)]
#[command(name = "tutanak", version, about = "Linux toplantı-notu asistanı (v0 CLI)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Sistem sesini kaydet, durdur, transkript+özet çıkar.
    Record {
        /// Tam transkript çevirisi de üret (uzun toplantıda maliyetli).
        #[arg(long)]
        translate: bool,
        #[arg(long, default_value = "Toplantı")]
        title: String,
        /// Resume iş kimliği (aynı id ile başarısız çalışmayı sürdür).
        #[arg(long)]
        job: Option<String>,
        /// Ham WAV dosyasını sakla (silme).
        #[arg(long)]
        keep_wav: bool,
        /// Sadece sistem sesini al (kendi mikrofonunu DAHİL ETME).
        #[arg(long)]
        system_only: bool,
    },
    /// Var olan bir ses/video dosyasını işle (ffmpeg ile 16kHz mono'ya çevrilir).
    Process {
        input: PathBuf,
        #[arg(long)]
        translate: bool,
        #[arg(long, default_value = "Toplantı")]
        title: String,
        #[arg(long)]
        job: Option<String>,
    },
    /// Yapılandırma yönetimi (provider profilleri vb.).
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Config dosyasının yolunu yazdır.
    Path,
    /// Varsayılan (yorumlu) config dosyasını oluştur.
    Init {
        /// Var olan dosyanın üzerine yaz.
        #[arg(long)]
        force: bool,
    },
    /// Etkin (çözülmüş) yapılandırmayı göster (sır göstermez).
    Show,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("\nHata: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Config { action } => run_config(action),
        Command::Record { translate, title, job, keep_wav, system_only } => {
            let cfg = Config::load()?;
            let wav = temp_wav();
            let source = if system_only {
                capture::Source::SystemOnly
            } else {
                capture::Source::MicAndSystem
            };
            let session = capture::start(source, &wav)?;
            eprint!(
                "● Kayıt başladı ({}). Durdurmak için ENTER'a bas...",
                if system_only { "sadece sistem" } else { "mic + sistem" }
            );
            io::stderr().flush().ok();
            let mut line = String::new();
            io::stdin().read_line(&mut line).ok();
            capture::stop(session)?;
            eprintln!("■ Kayıt durdu: {}", wav.display());

            let opts = RunOptions { translate, job_id: job_id(job) };
            eprintln!("İş kimliği: {} (hata olursa --job {} ile sürdür)", opts.job_id, opts.job_id);
            let notes = run_pipeline(&cfg, &wav, &opts)?;
            finish(&notes, &title)?;

            if keep_wav {
                eprintln!("Ham WAV saklandı: {}", wav.display());
            } else {
                let _ = std::fs::remove_file(&wav);
            }
            Ok(())
        }
        Command::Process { input, translate, title, job } => {
            anyhow::ensure!(input.exists(), "Girdi dosyası yok: {}", input.display());
            let cfg = Config::load()?;
            let wav = temp_wav();
            eprintln!("ffmpeg ile 16kHz mono'ya çevriliyor...");
            capture::normalize_to_wav(&input, &wav)?;

            let opts = RunOptions { translate, job_id: job_id(job) };
            eprintln!("İş kimliği: {} (hata olursa --job {} ile sürdür)", opts.job_id, opts.job_id);
            let notes = run_pipeline(&cfg, &wav, &opts)?;
            finish(&notes, &title)?;
            let _ = std::fs::remove_file(&wav);
            Ok(())
        }
    }
}

fn run_config(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Path => {
            println!("{}", config::config_path()?.display());
        }
        ConfigAction::Init { force } => {
            let (path, created) = config::init_file(force)?;
            if created {
                println!("Config oluşturuldu: {}", path.display());
            } else {
                println!("Config zaten var: {} (üzerine yazmak için --force)", path.display());
            }
        }
        ConfigAction::Show => {
            let cfg = Config::load()?;
            println!("# kaynak: {}", config::config_path()?.display());
            println!("{}", cfg.describe());
        }
    }
    Ok(())
}

fn finish(notes: &tutanak_core::Notes, title: &str) -> Result<()> {
    let path = storage::save_markdown(notes, title).context("notlar kaydedilemedi")?;
    println!("\n=== ÖZET ({}) ===\n{}\n", notes.output_lang, notes.summary.trim());
    println!("Notlar kaydedildi: {}", path.display());
    Ok(())
}

fn temp_wav() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("tutanak-{}-{stamp}.wav", std::process::id()))
}

/// Resume id: explicit `--job` (intentional resume), else a unique per-run id so
/// a fresh recording never reuses a previous run's cached transcript.
fn job_id(job: Option<String>) -> String {
    job.unwrap_or_else(|| {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("run-{ms}")
    })
}
