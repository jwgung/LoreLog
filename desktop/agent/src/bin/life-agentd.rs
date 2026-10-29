//! Background collector: the same loop as `life-agent run`, but with no console window (this is what autostart launches).
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    if let Err(e) = life_agent::run::run() {
        life_agent_core::agent::log(
            &life_agent_core::config::home(),
            &format!("not started: {e}"),
        );
        std::process::exit(2);
    }
}
