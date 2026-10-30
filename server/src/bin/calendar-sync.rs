//! calendar-sync: import CalDAV events into the vault as Events/ notes (read-only on the server).
use std::process::ExitCode;

use life_server::collect::{calendar, sync};
use life_server::config::Paths;

const HELP: &str = "\
Import events (VEVENT) from CalDAV calendars into the vault as Events/ notes. Read-only mirror (never writes to the server).

- Fetches the events of a window (default: 400 days back to 400 days ahead) with REPORT. If the server expands
  recurring events, each occurrence becomes its own note.
- Notes use the same frontmatter as the existing Events/ notes created by the Calendar Notes plugin.
- An existing event is recognised by (dav_uid, start). Only events whose etag changed are rewritten.
  Notes of events that disappeared from the server are not deleted.

Settings live in the OS config place + vault-sync/ (Linux ~/.config/vault-sync/, or LIFE_CONF_DIR):
  env           secrets only (KEY=VALUE, mode 600): CALDAV_USER / CALDAV_PASSWORD (default: IMAP_USER / IMAP_PASSWORD)
  sources.toml  what to collect, under [calendar]: urls, past_days, future_days
                (env may also carry CALDAV_URL (comma-separated), CALDAV_PAST_DAYS, CALDAV_FUTURE_DAYS; sources.toml wins)
Prefer `life add calendar` and `life sync` over editing these by hand.

Exit status: 0 on success, 1 if any calendar could not be read, 2 if settings are missing.

Usage: calendar-sync [--dry-run] [--list [SERVER_URL]] [-h]
  --list   discover the account's calendars and print: name, URL, supported components (tab-separated).
           Without SERVER_URL it starts from the first configured URL.";

fn main() -> ExitCode {
    life_server::logging::to_stderr();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let paths = Paths::from_env();
    let Some(i) = args.iter().position(|a| a == "--list") else {
        let dry = args.iter().any(|a| a == "--dry-run");
        return ExitCode::from(sync::calendar(&paths, dry, &mut sync::console).exit_code());
    };
    let setup = match sync::calendar_setup(&paths) {
        Ok(s) => s,
        Err(missing) => {
            eprintln!("{missing}");
            return ExitCode::from(2);
        }
    };
    let base = args.get(i + 1).filter(|a| !a.starts_with('-')).cloned().unwrap_or_else(|| setup.urls[0].clone());
    match calendar::list_calendars(&base, &setup.account) {
        Ok(cals) => {
            for (name, url, comps) in &cals {
                println!("{name}\t{url}\t{comps}");
            }
            eprintln!("{} calendars (only events are collected, so use one that supports VEVENT)", cals.len());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
