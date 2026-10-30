//! calendar-sync: import CalDAV events into the vault as Events/ notes (read-only on the server).
use std::process::ExitCode;

use chrono::{Duration, Local};
use life_server::collect::calendar::{self, Account, Counts, DavError};
use life_server::collect::settings;
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

fn short(url: &str, n: usize) -> String {
    url.trim_end_matches('/').rsplit('/').next().unwrap_or("").chars().take(n).collect()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let dry = args.iter().any(|a| a == "--dry-run");
    let list = args.iter().position(|a| a == "--list");
    let _lock = if dry || list.is_some() {
        None // dry runs and --list write nothing
    } else {
        match settings::lock("calendar-sync") {
            Some(l) => Some(l),
            None => {
                println!("Another calendar-sync run is in progress; skipping this one.");
                return ExitCode::SUCCESS;
            }
        }
    };
    let paths = Paths::from_env();
    let cfg = settings::calendar(&paths);
    let urls = settings::list(cfg.get("CALDAV_URL"), "");
    let get =
        |a: &str, b: &str| cfg.get(a).filter(|v| !v.is_empty()).or_else(|| cfg.get(b)).cloned().unwrap_or_default();
    let acct = Account { user: get("CALDAV_USER", "IMAP_USER"), password: get("CALDAV_PASSWORD", "IMAP_PASSWORD") };
    if urls.is_empty() || acct.user.is_empty() || acct.password.is_empty() {
        eprintln!(
            "Missing calendar URL or credentials (env: {}, sources.toml: {}). Run `life add calendar`.",
            settings::env_file(&paths).display(),
            settings::sources_file(&paths).display()
        );
        return ExitCode::from(2);
    }
    if let Some(i) = list {
        let base = args.get(i + 1).filter(|a| !a.starts_with('-')).cloned().unwrap_or_else(|| urls[0].clone());
        return match calendar::list_calendars(&base, &acct) {
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
        };
    }
    let today = Local::now().date_naive();
    let days = |k: &str| cfg.get(k).and_then(|v| v.parse::<i64>().ok()).unwrap_or(400);
    let start = (today - Duration::days(days("CALDAV_PAST_DAYS"))).format("%Y%m%dT000000Z").to_string();
    let end = (today + Duration::days(days("CALDAV_FUTURE_DAYS"))).format("%Y%m%dT000000Z").to_string();
    let mut known = calendar::existing_events(&paths.vault);
    let mut counts = Counts::default();
    let mut failed = false;
    for url in &urls {
        let items = calendar::report(url, &acct, &start, &end, true)
            .or_else(|e| match e {
                DavError::Status(_) => calendar::report(url, &acct, &start, &end, false), // cannot expand: originals only
                other => Err(other),
            })
            .and_then(|xml| calendar::events_from(url, &acct, &xml, 50)); // a failing multiget is a failed calendar
        let items = match items {
            Ok(i) => i,
            Err(e) => {
                let why = match &e {
                    DavError::Status(c) => format!("HTTP {c}"),
                    DavError::Other(k) => k.clone(),
                };
                eprintln!("CalDAV request failed ({why}): {}", short(url, usize::MAX));
                failed = true;
                continue;
            }
        };
        let source = format!("caldav/{}", short(url, 12));
        match calendar::apply(&paths.vault, &source, &items, &mut known, dry, &mut counts) {
            Ok(n) => println!("[{}] {n} events on server", short(url, 12)),
            Err(e) => {
                eprintln!("writing notes failed: {:?}", e.kind());
                failed = true;
            }
        }
    }
    println!(
        "{}new notes {}, updated {}, unchanged {}",
        if dry { "[dry-run] " } else { "" },
        counts.created,
        counts.updated,
        counts.same
    );
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
