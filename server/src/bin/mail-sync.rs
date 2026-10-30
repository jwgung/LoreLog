//! mail-sync: import IMAP mail (inbox and sent folders) into the vault as Mail/ notes. Read-only mirror.
use std::collections::HashSet;
use std::process::ExitCode;

use chrono::Local;
use life_server::collect::{imapc, mail, settings};
use life_server::config::Paths;
use serde_json::Value;

const HELP: &str = "\
Import IMAP mail (inbox and sent folders) into the vault as Mail/ notes. Read-only mirror.

- Never modifies the server: folders are opened read-only and bodies are read with BODY.PEEK,
  so read flags do not change.
- Incremental: processed UIDs are recorded per folder (start over if UIDVALIDITY changes). A mail that
  already has a note is skipped, matched by mail_id (Message-ID) or by a date/time/sender/subject fingerprint.
- Notes use the same frontmatter as the existing Mail/ notes created by Mailstone, so hub-sync reads them unchanged.
- Raw .eml files and attachments are not stored (attachments are listed by name only).

Settings live in ~/.config/vault-sync/ :
  env           secrets only (KEY=VALUE, mode 600): IMAP_USER, IMAP_PASSWORD
  sources.toml  what to collect, under [mail]: host, port, account, folders, since
                (env may also carry IMAP_HOST, IMAP_PORT, MAIL_ACCOUNT, MAIL_FOLDERS, MAIL_SINCE; sources.toml wins)
Prefer `life add mail` and `life sync` over editing these by hand.

Exit status: 0 on success, 1 if the connection failed or a folder could not be read, 2 if settings are missing.

Usage: mail-sync [--dry-run] [--limit N] [--folder NAME] [-h]";

const DEFAULT_LIMIT: usize = 300;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let dry = args.iter().any(|a| a == "--dry-run");
    let _lock = if dry {
        None // a dry run writes nothing, so it needs no lock
    } else {
        match settings::lock("mail-sync") {
            Some(l) => Some(l),
            None => {
                println!("Another mail-sync run is in progress; skipping this one.");
                return ExitCode::SUCCESS;
            }
        }
    };
    let limit = arg(&args, "--limit").and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_LIMIT);
    let paths = Paths::from_env();
    let cfg = settings::mail(&paths);
    let need: Vec<&str> = ["IMAP_HOST", "IMAP_USER", "IMAP_PASSWORD"]
        .into_iter()
        .filter(|k| cfg.get(*k).map(|v| v.is_empty()).unwrap_or(true))
        .collect();
    if !need.is_empty() {
        eprintln!(
            "Missing {} (env: {}, sources.toml: {}). Run `life add mail`.",
            need.join(", "),
            settings::env_file(&paths).display(),
            settings::sources_file(&paths).display()
        );
        return ExitCode::from(2);
    }
    let folders = match arg(&args, "--folder") {
        Some(f) => vec![f],
        None => settings::list(cfg.get("MAIL_FOLDERS"), "INBOX,Sent Messages"),
    };
    let account = cfg.get("MAIL_ACCOUNT").cloned().unwrap_or_else(|| "account".into());
    let since = cfg.get("MAIL_SINCE").filter(|v| !v.is_empty()).cloned();
    let home = dirs::home_dir().unwrap_or_default();
    let own: HashSet<String> = std::fs::read_to_string(home.join(".local/share/hub-sync/identity.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| {
            v["emails"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_lowercase).collect())
        })
        .unwrap_or_default();
    let state_file = settings::state_dir().join("mail-state.json");
    let mut state = imapc::State::load(&state_file);
    let (mut known, mut known_fp) = mail::known_notes(&paths.vault);
    let synced = Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string();
    let port: u16 = cfg.get("IMAP_PORT").and_then(|p| p.parse().ok()).unwrap_or(993);
    let mut s = match imapc::connect(&cfg["IMAP_HOST"], port, &cfg["IMAP_USER"], &cfg["IMAP_PASSWORD"]) {
        Ok(s) => s,
        Err(k) => {
            eprintln!("IMAP connection failed: {k}");
            return ExitCode::FAILURE;
        }
    };
    let (mut total, mut failed) = (0usize, false);
    for folder in &folders {
        let (uidv, all) = match imapc::open(&mut s, folder, since.as_deref()) {
            Ok(x) => x,
            Err(_) => {
                eprintln!("[{folder}] cannot open folder");
                failed = true;
                continue;
            }
        };
        let mut seen = state.seen(folder, &uidv);
        let todo: Vec<u32> = all.iter().copied().filter(|u| !seen.contains(u)).collect();
        // 1) the headers first, so mails that already have a note are not downloaded
        let mut fresh = Vec::new();
        for batch in todo.chunks(200) {
            let heads = match imapc::headers(&mut s, batch) {
                Ok(h) => h,
                Err(_) => {
                    eprintln!("[{folder}] cannot read headers");
                    failed = true;
                    break;
                }
            };
            for u in batch {
                let (mid, fp) = heads
                    .get(u)
                    .and_then(|h| {
                        mail::parse(h).map(|m| (mail::norm_id(m.message_id().unwrap_or("")), mail::msg_fingerprint(&m)))
                    })
                    .unwrap_or_default();
                if (!mid.is_empty() && known.contains(&mid)) || (!fp.is_empty() && known_fp.contains(&fp)) {
                    seen.insert(*u);
                } else {
                    fresh.push(*u);
                }
            }
        }
        let take: Vec<u32> = fresh.iter().copied().take(limit).collect();
        println!(
            "[{folder}] server {} · already known/processed {} · new {} (this run {})",
            all.len(),
            all.len() - fresh.len(),
            fresh.len(),
            take.len()
        );
        let mut wrote = 0;
        for u in take {
            let raw = match imapc::body(&mut s, u) {
                Ok(Some(r)) => r,
                Ok(None) => continue,
                Err(_) => {
                    eprintln!("[{folder}] cannot read a mail");
                    failed = true;
                    break;
                }
            };
            let Some(msg) = mail::parse(&raw) else { continue };
            let (rel, note, mid) = mail::render(&msg, &format!("{account}/{folder}"), &own, &synced);
            seen.insert(u);
            let fp = mail::msg_fingerprint(&msg);
            if known.contains(&mid) || (!fp.is_empty() && known_fp.contains(&fp)) {
                continue;
            }
            if !dry && let Err(e) = mail::write_note(&paths.vault, &rel, &note) {
                eprintln!("[{folder}] writing a note failed: {:?}", e.kind());
                failed = true;
                break;
            }
            known.insert(mid);
            known_fp.insert(fp);
            wrote += 1;
        }
        total += wrote;
        if !dry {
            state.set(folder, &uidv, &seen);
        }
    }
    let _ = s.logout();
    if !dry && let Err(e) = state.save(&state_file) {
        eprintln!("saving the state failed: {:?}", e.kind());
        failed = true;
    }
    println!("{}{total} new notes", if dry { "[dry-run] " } else { "" });
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
