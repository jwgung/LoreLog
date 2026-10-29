//! life-agent command line: run, once, flush, status, config, install, uninstall.
use life_agent::{platform, run};
use life_agent_core::agent::{Agent, Sampler, system_clock};
use life_agent_core::segments::Segmenter;
use life_agent_core::{agent::hostname, config, spool::Spool};
use std::process::ExitCode;

const USAGE: &str = "usage: life-agent <run|once|flush|status|config|install|uninstall>";
const AUTOSTART_NAME: &str = "LifeAgent";

fn once() -> Result<(), String> {
    let cfg = config::load()?;
    let s = platform::new().sample()?;
    let shown = if s.app.is_empty() {
        String::new()
    } else {
        Segmenter::new(&cfg, &hostname()).shown_title(&s.app, &s.title)
    };
    let app = if s.app.is_empty() {
        "(none: locked or no window)"
    } else {
        &s.app
    };
    println!("app={app}  title={shown:?}  idle={:.0}s", s.idle_s);
    Ok(())
}

fn flush() -> Result<(), String> {
    let cfg = config::load()?;
    let mut agent = Agent::new(
        cfg,
        platform::new(),
        Spool::new(&config::home()),
        Box::new(system_clock),
    );
    let (sent, err) = agent.flush();
    println!(
        "sent {sent} records{}",
        if err.is_empty() {
            String::new()
        } else {
            format!(", problem: {err}")
        }
    );
    if err.is_empty() { Ok(()) } else { Err(err) }
}

fn status() -> Result<(), String> {
    let cfg = config::load()?;
    let spool = Spool::new(&config::home());
    let set = |v: &str| if v.is_empty() { "NOT set" } else { "set" };
    println!(
        "config:  {}  (url {}, token {})",
        config::config_path().display(),
        set(&cfg.url),
        set(&cfg.token)
    );
    println!(
        "spool:   {} records waiting  ({})",
        spool.read().len(),
        spool.path().display()
    );
    println!("log:     {}", config::home().join("agent.log").display());
    Ok(())
}

fn autostart(on: bool) -> Result<(), String> {
    // the background binary sits next to this one
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let daemon = me.with_file_name(if cfg!(windows) {
        "life-agentd.exe"
    } else {
        "life-agentd"
    });
    if on && !daemon.exists() {
        return Err(format!("{} not found next to life-agent", daemon.display()));
    }
    let al = auto_launch::AutoLaunchBuilder::new()
        .set_app_name(AUTOSTART_NAME)
        .set_app_path(&daemon.to_string_lossy())
        .build()
        .map_err(|e| e.to_string())?;
    if on {
        al.enable().map_err(|e| e.to_string())?;
        println!("The agent will start when you sign in.");
    } else {
        al.disable().map_err(|e| e.to_string())?;
        println!("Autostart removed.");
    }
    Ok(())
}

fn main() -> ExitCode {
    let result = match std::env::args().nth(1).as_deref() {
        Some("run") => run::run(),
        Some("once") => once(),
        Some("flush") => flush(),
        Some("status") => status(),
        Some("config") => config::write_template().map(|p| println!("{}", p.display())),
        Some("install") => autostart(true),
        Some("uninstall") => autostart(false),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}
