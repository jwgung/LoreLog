//! The collection loop shared by `life-agent run` (console) and `life-agentd` (background, no console window).
use life_agent_core::agent::{Agent, log, system_clock};
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
    let cfg = config::load()?;
    if cfg.url.is_empty() || cfg.token.is_empty() {
        return Err(format!(
            "Set url and token in {} first (life-agent config creates it).",
            config::config_path().display()
        ));
    }
    let _lock = single_instance()?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst)).map_err(|e| e.to_string())?;
    let interval = Duration::from_secs(cfg.interval_s.max(1));
    let dir = config::home();
    let mut agent = Agent::new(
        cfg,
        crate::platform::new(),
        Spool::new(&dir),
        Box::new(system_clock),
    );
    log(&dir, "started");
    while !stop.load(Ordering::SeqCst) {
        agent.step();
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
