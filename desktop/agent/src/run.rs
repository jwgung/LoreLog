//! The collection loop. `life-agent run` runs it in the console; the desktop app runs the same loop (`run_until`)
//! in the background. The chosen folders and the server are read from the shared config.
use life_agent_core::agent::{Agent, log, system_clock};
use life_agent_core::intake::IntakeEngine;
use life_agent_core::sender::ServerUploader;
use life_agent_core::{config, spool::Spool};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Only one collector per user: two would write overlapping segments. The lock is released when the process ends.
fn single_instance() -> Result<std::fs::File, String> {
    let dir = config::home();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("agent.lock"))
        .map_err(|e| e.to_string())?;
    f.try_lock()
        .map_err(|_| "another life-agent is already running".to_string())?;
    Ok(f)
}

pub fn run() -> Result<(), String> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst)).map_err(|e| e.to_string())?;
    let _lock = single_instance()?;
    run_until(stop)
}

/// How often the watched-folder intake runs inside the collection loop.
const INTAKE_EVERY: Duration = Duration::from_secs(5);

fn open_intake(dir: &std::path::Path, settle_s: f64, max_upload_mb: u64) -> Option<IntakeEngine> {
    match IntakeEngine::open_with_settle(&dir.join("intake-state.json"), settle_s) {
        Ok(e) => Some(e.with_max_bytes(max_upload_mb.saturating_mul(1024 * 1024))),
        Err(e) => {
            log(dir, &format!("intake disabled: {e}"));
            None
        }
    }
}

/// The collection loop until `stop` is set: window segments plus the watched-folder intake. Shared by the CLI
/// (`run`, which also takes the single-instance lock and listens for Ctrl+C) and the desktop shell (which sets
/// `stop` on app exit). The server address, token and interval are read once at the start; the watched-folder
/// list is re-read each intake cycle, so folders added or removed (from the app or the CLI) take effect without a
/// restart. Returns an error if url/token are unset.
pub fn run_until(stop: Arc<AtomicBool>) -> Result<(), String> {
    let cfg = config::load()?;
    if cfg.url.is_empty() || cfg.token.is_empty() {
        return Err(format!(
            "Set url and token in {} first (life-agent config creates it).",
            config::config_path().display()
        ));
    }
    let interval = Duration::from_secs(cfg.interval_s.max(1));
    let settle_s = cfg.settle_s;
    let max_upload_mb = cfg.max_upload_mb;
    let dir = config::home();
    let uploader = ServerUploader::new(&cfg);
    let mut intake = open_intake(&dir, settle_s, max_upload_mb);
    let mut last_intake = std::time::Instant::now();
    let mut agent = Agent::new(
        cfg,
        crate::platform::new(),
        Spool::new(&dir),
        Box::new(system_clock),
    );
    log(&dir, "started");
    while !stop.load(Ordering::SeqCst) {
        agent.step();
        if last_intake.elapsed() >= INTAKE_EVERY {
            last_intake = std::time::Instant::now();
            // re-read the chosen folders each cycle so changes take effect without a restart
            let watch_dirs: Vec<std::path::PathBuf> = config::load()
                .map(|c| c.watch_dirs.iter().map(std::path::PathBuf::from).collect())
                .unwrap_or_default();
            if !watch_dirs.is_empty() {
                if intake.is_none() {
                    intake = open_intake(&dir, settle_s, max_upload_mb);
                }
                if let Some(eng) = intake.as_mut() {
                    let now = system_clock();
                    eng.scan(&watch_dirs, now);
                    eng.step(&uploader, now);
                }
            }
        }
        // sleep in short slices so Ctrl+C is answered quickly
        let until = std::time::Instant::now() + interval;
        while std::time::Instant::now() < until && !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    agent.stop();
    log(&dir, "stopped");
    Ok(())
}
