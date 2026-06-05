//! tutanak desktop GUI (slint). A thin frontend over `tutanak-core`:
//! pick provider profiles, record system audio (+ mic), edit settings, and get
//! markdown notes — with the pipeline running on a worker thread so the UI never
//! blocks.
//!
//!   [Kaydı Başlat] ─► capture (mic+system mix) ─► [Durdur ve İşle]
//!                                                    │ worker thread
//!        UI status ◄── upgrade_in_event_loop ────────┤ run_pipeline_staged
//!        Özet/Transkript sekmeleri ◄─────────────────┘ + storage.save_markdown
//!
//! On Linux the GUI needs a running display; build is headless-safe.

slint::include_modules!();

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use slint::{ModelRc, SharedString, VecModel};
use tutanak_core::{
    capture, capture::Session, config, config::Config, run_pipeline_staged, storage, RunOptions,
    Stage,
};

fn main() -> Result<()> {
    let cfg = Rc::new(RefCell::new(Config::load()?));
    let ui = MainWindow::new()?;

    // Populate provider dropdowns from the registry.
    {
        let c = cfg.borrow();
        let names: Vec<SharedString> = c.providers.keys().map(|k| k.as_str().into()).collect();
        ui.set_stt_providers(ModelRc::from(Rc::new(VecModel::from(names.clone()))));
        ui.set_summary_providers(ModelRc::from(Rc::new(VecModel::from(names))));
        ui.set_stt_provider(c.stt_provider.as_str().into());
        ui.set_summary_provider(c.summary_provider.as_str().into());
        ui.set_output_lang(c.output_lang.as_str().into());
        if let Ok(p) = config::config_path() {
            ui.set_config_path(p.display().to_string().into());
        }
    }

    let state: Rc<RefCell<Option<Session>>> = Rc::new(RefCell::new(None));
    let history: Rc<RefCell<Vec<PathBuf>>> = Rc::new(RefCell::new(Vec::new()));

    // Load past notes into the history dropdown; show the newest on startup.
    refresh_history(&ui, &history);
    {
        let newest = history.borrow().first().cloned();
        if let Some(p) = newest {
            ui.set_history_index(0);
            load_into_tabs(&ui, &p);
        }
    }

    // ---- Record / Stop ----
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let cfg = cfg.clone();
        ui.on_toggle_record(move || {
            let ui = ui_weak.unwrap();
            let mut slot = state.borrow_mut();

            // not recording → start
            if slot.is_none() {
                let source = if ui.get_include_mic() {
                    capture::Source::MicAndSystem
                } else {
                    capture::Source::SystemOnly
                };
                match start_recording(source) {
                    Ok(session) => {
                        let with_mic = source == capture::Source::MicAndSystem;
                        *slot = Some(session);
                        ui.set_recording(true);
                        ui.set_saved_path("".into());
                        ui.set_status(
                            if with_mic {
                                "● Kaydediliyor (mic + sistem)… Durdur'a basınca işlenecek."
                            } else {
                                "● Kaydediliyor (sadece sistem)… Durdur'a basınca işlenecek."
                            }
                            .into(),
                        );
                    }
                    Err(e) => ui.set_status(format!("Hata: {e:#}").into()),
                }
                return;
            }

            // recording → stop + process
            let session = slot.take().unwrap();
            let wav = session.wav.clone();
            ui.set_recording(false);
            if let Err(e) = capture::stop(session) {
                ui.set_status(format!("Durdurma hatası: {e:#}").into());
                let _ = std::fs::remove_file(&wav);
                return;
            }
            ui.set_processing(true);
            ui.set_status("İşleniyor…".into());

            // Build a run config from current config + UI selections.
            let mut run_cfg = cfg.borrow().clone();
            run_cfg.stt_provider = ui.get_stt_provider().to_string();
            run_cfg.summary_provider = ui.get_summary_provider().to_string();
            run_cfg.output_lang = ui.get_output_lang().to_string();
            let translate = ui.get_translate_too();
            let title = ui.get_meeting_title().to_string();
            let opts = RunOptions { translate, job_id: format!("run-{}", millis()) };

            let ui_weak = ui.as_weak();
            std::thread::spawn(move || {
                let stage_weak = ui_weak.clone();
                let mut on_stage = move |s: Stage| {
                    let msg = match s {
                        Stage::Transcribing => "Transkript çıkarılıyor…",
                        Stage::Translating => "Çeviriliyor…",
                        Stage::Summarizing => "Özetleniyor…",
                    };
                    let _ = stage_weak.upgrade_in_event_loop(move |ui| ui.set_status(msg.into()));
                };

                let result = run_pipeline_staged(&run_cfg, &wav, &opts, &mut on_stage)
                    .and_then(|notes| {
                        let path = storage::save_markdown(&notes, &title)?;
                        Ok((notes, path))
                    });
                let _ = std::fs::remove_file(&wav);

                let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                    ui.set_processing(false);
                    match result {
                        Ok((notes, path)) => {
                            ui.set_summary_text(notes.summary.as_str().into());
                            let mut body = String::new();
                            if let Some(tr) = &notes.translation {
                                body.push_str("== Çeviri ==\n");
                                body.push_str(tr);
                                body.push_str("\n\n== Transkript ==\n");
                            }
                            body.push_str(&notes.transcript);
                            ui.set_transcript_text(body.into());
                            ui.set_saved_path(path.display().to_string().into());
                            let lang = if notes.language.is_empty() { "?".to_string() } else { notes.language };
                            ui.set_status(format!("Tamam (kaynak dil: {lang}).").into());
                            ui.invoke_refresh_history(); // add the new note to the history list
                        }
                        Err(e) => ui.set_status(format!("Hata: {e:#}").into()),
                    }
                });
            });
        });
    }

    // ---- Settings: open ----
    {
        let ui_weak = ui.as_weak();
        let cfg = cfg.clone();
        ui.on_open_settings(move || {
            let ui = ui_weak.unwrap();
            fill_settings(&ui, &cfg.borrow());
            ui.set_show_settings(true);
        });
    }

    // ---- Settings: save ----
    {
        let ui_weak = ui.as_weak();
        let cfg = cfg.clone();
        ui.on_save_settings(move || {
            let ui = ui_weak.unwrap();
            let stt = ui.get_stt_provider().to_string();
            let sum = ui.get_summary_provider().to_string();
            {
                let mut c = cfg.borrow_mut();
                if let Some(p) = c.providers.get_mut(&stt) {
                    p.base_url = nonempty(ui.get_e_stt_base());
                    p.stt_model = nonempty(ui.get_e_stt_model());
                    p.api_key_env = nonempty(ui.get_e_stt_keyenv());
                }
                if let Some(p) = c.providers.get_mut(&sum) {
                    p.base_url = nonempty(ui.get_e_sum_base());
                    p.llm_model = nonempty(ui.get_e_sum_model());
                    p.api_key_env = nonempty(ui.get_e_sum_keyenv());
                }
                if let Ok(mb) = ui.get_e_chunk_mb().trim().parse::<u64>() {
                    c.chunk_threshold_bytes = mb.max(1) * 1024 * 1024;
                }
                c.stt_provider = stt;
                c.summary_provider = sum;
                c.output_lang = ui.get_output_lang().to_string();
            }
            match cfg.borrow().save() {
                Ok(path) => {
                    ui.set_show_settings(false);
                    ui.set_status(format!("Ayarlar kaydedildi: {}", path.display()).into());
                }
                Err(e) => ui.set_status(format!("Kaydetme hatası: {e:#}").into()),
            }
        });
    }

    // ---- Settings: close ----
    {
        let ui_weak = ui.as_weak();
        ui.on_close_settings(move || {
            ui_weak.unwrap().set_show_settings(false);
        });
    }

    // ---- History: select a past note ----
    {
        let ui_weak = ui.as_weak();
        let history = history.clone();
        ui.on_select_history(move |idx| {
            let ui = ui_weak.unwrap();
            let path = history.borrow().get(idx.max(0) as usize).cloned();
            if let Some(p) = path {
                load_into_tabs(&ui, &p);
            }
        });
    }

    // ---- History: rebuild list (after a new note is saved) ----
    {
        let ui_weak = ui.as_weak();
        let history = history.clone();
        ui.on_refresh_history(move || {
            let ui = ui_weak.unwrap();
            refresh_history(&ui, &history);
            ui.set_history_index(0);
        });
    }

    // ---- Open config file externally ----
    ui.on_open_config(move || {
        if let Ok(path) = config::config_path() {
            let _ = config::init_file(false);
            let _ = std::process::Command::new("xdg-open").arg(path).spawn();
        }
    });

    ui.run()?;
    Ok(())
}

fn fill_settings(ui: &MainWindow, cfg: &Config) {
    let stt = ui.get_stt_provider().to_string();
    let sum = ui.get_summary_provider().to_string();
    if let Some(p) = cfg.providers.get(&stt) {
        ui.set_e_stt_base(p.base_url.clone().unwrap_or_default().into());
        ui.set_e_stt_model(p.stt_model.clone().unwrap_or_default().into());
        ui.set_e_stt_keyenv(p.api_key_env.clone().unwrap_or_default().into());
    }
    if let Some(p) = cfg.providers.get(&sum) {
        ui.set_e_sum_base(p.base_url.clone().unwrap_or_default().into());
        ui.set_e_sum_model(p.llm_model.clone().unwrap_or_default().into());
        ui.set_e_sum_keyenv(p.api_key_env.clone().unwrap_or_default().into());
    }
    ui.set_e_chunk_mb(format!("{}", cfg.chunk_threshold_bytes / (1024 * 1024)).into());
}

fn nonempty(s: SharedString) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

/// Rebuild the history dropdown from saved notes; store their paths (newest first).
fn refresh_history(ui: &MainWindow, paths: &Rc<RefCell<Vec<PathBuf>>>) {
    let entries = storage::list_notes().unwrap_or_default();
    let labels: Vec<SharedString> = entries.iter().map(|e| e.label.as_str().into()).collect();
    ui.set_history_titles(ModelRc::from(Rc::new(VecModel::from(labels))));
    *paths.borrow_mut() = entries.into_iter().map(|e| e.path).collect();
}

/// Load a saved note from disk into the Özet / Transkript tabs.
fn load_into_tabs(ui: &MainWindow, path: &Path) {
    if let Ok(n) = storage::load_note(path) {
        ui.set_summary_text(n.summary.into());
        let mut body = String::new();
        if let Some(tr) = &n.translation {
            body.push_str("== Çeviri ==\n");
            body.push_str(tr);
            body.push_str("\n\n== Transkript ==\n");
        }
        body.push_str(&n.transcript);
        ui.set_transcript_text(body.into());
        ui.set_saved_path(path.display().to_string().into());
    }
}

fn start_recording(source: capture::Source) -> Result<Session> {
    let wav = std::env::temp_dir()
        .join(format!("tutanak-ui-{}-{}.wav", std::process::id(), millis()));
    capture::start(source, &wav)
}

fn millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
