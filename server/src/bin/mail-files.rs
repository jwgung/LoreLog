//! mail-files: keep the attachments of the mails in the configured mail folders (read-only on the server).
use std::process::ExitCode;
use std::sync::LazyLock;

use life_server::collect::{imapc, mail, mailfiles, settings};
use life_server::config::Paths;
use life_server::store::RawStore;
use regex::Regex;

const HELP: &str = "\
Command line of mail-files: keep the attachments of the mails in the configured mail folders (read-only on the server).

Usage: mail-files [--dry-run] [--limit N] [--folder NAME] [-h]

Folders are opened read-only and mails are read with BODY.PEEK, so nothing on the server changes. Every configured
mail folder is used; a folder whose name looks like a sent folder (sent / 보낸 / 발신) marks its files as sent, the
others as received. Processed UIDs are remembered per folder, so a run only reads mails it has not seen. --limit is per
folder. --dry-run reads sizes and headers only and stores nothing.";

const DEFAULT_LIMIT: usize = 100;

/// -> (mails read, files seen, bytes written), or what went wrong
fn run_folder(
    s: &mut imapc::Session,
    folder: &str,
    account: &str,
    store: &RawStore,
    state: &mut imapc::State,
    limit: usize,
    dry: bool,
) -> Result<(usize, usize, usize), String> {
    static MULTIPART: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)multipart/(mixed|related)").unwrap());
    let (uidv, all) = imapc::open(s, folder, None).map_err(|_| format!("[{folder}] cannot open folder"))?;
    let mut seen = state.seen(folder, &uidv);
    let todo: Vec<u32> = all.iter().copied().filter(|u| !seen.contains(u)).collect();
    let take: Vec<u32> = todo.iter().copied().take(limit).collect();
    println!("[{folder}] server {} · not yet read {} (this run {})", seen.len() + todo.len(), todo.len(), take.len());
    if dry {
        let sizes = imapc::sizes(s, &take).map_err(|_| format!("[{folder}] cannot read sizes"))?;
        let carrying: Vec<&(u32, String)> = sizes.iter().filter(|(_, h)| MULTIPART.is_match(h)).collect();
        let bytes: u64 = carrying.iter().map(|(n, _)| *n as u64).sum();
        println!(
            "[dry-run] {} of {} mails look like they carry attachments, about {:.1} MB to read",
            carrying.len(),
            take.len(),
            bytes as f64 / 1e6
        );
        return Ok((take.len(), 0, 0));
    }
    let direction = if mailfiles::is_sent_folder(folder) { "sent" } else { "received" };
    let (mut mails, mut files, mut written) = (0, 0, 0);
    for u in take {
        let raw = imapc::body(s, u).map_err(|_| format!("[{folder}] cannot read a mail"))?;
        seen.insert(u);
        let Some(raw) = raw else { continue };
        let Some(msg) = mail::parse(&raw) else { continue };
        let (n, w) = mailfiles::keep(store, &msg, &format!("{account}/{folder}"), direction, "personal")
            .map_err(|e| format!("[{folder}] storing failed: {:?}", e.kind()))?;
        mails += 1;
        files += n;
        written += w;
    }
    state.set(folder, &uidv, &seen);
    Ok((mails, files, written))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let dry = args.iter().any(|a| a == "--dry-run");
    let limit = settings::arg(&args, "--limit").and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_LIMIT);
    let paths = Paths::from_env();
    let cfg = match settings::imap(&paths, settings::arg(&args, "--folder")) {
        Ok(c) => c,
        Err(need) => {
            eprintln!(
                "Missing {} ({}, {}).",
                need.join(", "),
                settings::env_file(&paths).display(),
                settings::sources_file(&paths).display()
            );
            return ExitCode::from(2);
        }
    };
    if cfg.folders.is_empty() {
        eprintln!("No mail folders configured.");
        return ExitCode::from(2);
    }
    let store = RawStore::new(&paths.data);
    let state_file = paths.data.join("mail-files-state.json");
    let mut state = imapc::State::load(&state_file);
    let mut s = match imapc::connect(&cfg.host, cfg.port, &cfg.user, &cfg.password) {
        Ok(s) => s,
        Err(k) => {
            eprintln!("IMAP connection failed: {k}");
            return ExitCode::FAILURE;
        }
    };
    let (mut failed, mut totals) = (false, (0, 0, 0));
    for folder in &cfg.folders {
        match run_folder(&mut s, folder, &cfg.account, &store, &mut state, limit, dry) {
            Ok((m, f, w)) => totals = (totals.0 + m, totals.1 + f, totals.2 + w),
            Err(problem) => {
                eprintln!("{problem}");
                failed = true;
            }
        }
        if !dry && let Err(e) = state.save(&state_file) {
            eprintln!("saving the state failed: {:?}", e.kind()); // after each folder: an interrupted run resumes
            failed = true;
        }
    }
    let _ = s.logout();
    if !dry {
        println!(
            "read {} mails · {} attachments seen · {:.1} MB newly written",
            totals.0,
            totals.1,
            totals.2 as f64 / 1e6
        );
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
