//! life-agent command line: run, once, flush, status, config, install, uninstall.
use life_agent::{platform, run};
use life_agent_core::agent::{Agent, Sampler, system_clock};
use life_agent_core::segments::Segmenter;
use life_agent_core::{agent::hostname, config, spool::Spool};
use std::process::ExitCode;

const USAGE: &str = "usage: life-agent <run|once|flush|status|config|watch>\n       life-agent watch <list|add DIR|remove DIR>";

fn watch(args: &[String]) -> Result<(), String> {
    let mut cfg = config::load()?;
    match args.first().map(String::as_str) {
        Some("list") | None => {
            if cfg.watch_dirs.is_empty() {
                println!("no watched folders (add one with: life-agent watch add <folder>)");
            } else {
                for d in &cfg.watch_dirs {
                    println!("{d}");
                }
            }
        }
        Some("add") => {
            let dir = args.get(1).ok_or("usage: life-agent watch add <folder>")?;
            let before = cfg.watch_dirs.len();
            let dirs = config::add_watch_dir(&mut cfg, dir)?;
            if dirs.len() == before {
                println!("already watched");
            } else {
                println!("watching: {}", dirs.last().unwrap());
            }
        }
        Some("remove") => {
            let dir = args.get(1).ok_or("usage: life-agent watch remove <folder>")?;
            let (_dirs, removed) = config::remove_watch_dir(&mut cfg, dir)?;
            if !removed {
                return Err(format!("not watched: {dir}"));
            }
            println!("stopped watching: {dir}");
        }
        Some(other) => return Err(format!("unknown watch command: {other}")),
    }
    Ok(())
}

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
        "config:  {}  (url {})",
        config::config_path().display(),
        set(&cfg.url),
    );
    println!("token:   {}  ({})", config::token_path().display(), set(&cfg.token));
    println!(
        "spool:   {} records waiting  ({})",
        spool.read().len(),
        spool.path().display()
    );
    println!("log:     {}", config::home().join("agent.log").display());
    if cfg.watch_dirs.is_empty() {
        println!("watch:   no watched folders");
    } else {
        println!("watch:   {} folder(s): {}", cfg.watch_dirs.len(), cfg.watch_dirs.join(", "));
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => run::run(),
        Some("once") => once(),
        Some("flush") => flush(),
        Some("status") => status(),
        Some("config") => config::write_template().map(|p| {
            println!("settings: {}", p.display());
            println!("token:    {}  (put the server's INGEST_TOKEN here, one line)", config::token_path().display());
            println!("edit the settings file to set url, then write the token file.");
        }),
        Some("watch") => watch(&args[1..]),
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
