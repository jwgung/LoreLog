//! life: add, run and check collection sources from one place. See `life -h`.
//!
//! The collection runs in-process (`collect::sync`, the same jobs mail-sync, mail-files and calendar-sync run);
//! afterwards the librarian indexes it. Secrets live in env (mode 600); what to collect lives in sources.toml (no secrets). Passwords
//! are read without echo.
use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;

use life_server::collect::{settings, setup, sources, sync};
use life_server::config::{Paths, read_env_file};

fn help(paths: &Paths) -> String {
    format!(
        "usage: life <command> ...

Add, run and check collection sources from one place.
The actual collection is done by mail-sync, mail-files and calendar-sync; this command ties their setup and runs together.

commands:
  add mail          mail account (IMAP): server, account, password (hidden), then folders to collect
  add calendar      calendars (CalDAV) of the account from `life add mail`
  add contacts      address books (CardDAV) of that account: everything a card holds but photos
  sync [--dry-run]  collect from every configured source; last, the librarian's index is brought up to date
                    (`life-librarian update`; not with --dry-run). Mail is capped at 300 messages per run.
  status            last run and result per source
  export [FILE]     everything the vault holds of your records in one zip, in open formats, to take elsewhere
                    (default: lorelog-export-<date>.zip here); the index, caches and secrets stay behind
  verify FILE       check an export against its list of hashes
  backup set DIR    where the daily sealed copy goes (outside the vault: an external disk, a NAS) and its password
                    (asked twice, hidden). `life sync` then makes one a day, checks it and keeps 7 days and 4 weeks.
  backup            make a copy now
  restore FILE OUT  open a copy with its password into OUT (a zip like `life export`'s) and check it
  held              what the vault holds, by source: files, size, first and last day
  forget SOURCE FROM TO [--yes]
                    forget those days (YYYY-MM-DD, both included) of a source (mail, calendar, phone, pc, diary,
                    contacts): records, notes, files nothing else names, untaken to-dos. Without --yes it only says
                    what would go. Cannot be undone.

First-time setup:
  life add mail            mail account and folders to collect
  life add calendar        calendars to collect
  life sync --dry-run      preview what would be collected, without saving
  life sync                collect for real
  life status              last run and result per source

Files (secrets live only in env):
  {}
      account and password (mode 600)
  {}
      what to collect (no secrets)
  {}
      run results",
        settings::env_file(paths).display(),
        settings::sources_file(paths).display(),
        sync::status_file(paths).display()
    )
}

// ---------- prompts ----------

fn need_tty() -> Result<(), ExitCode> {
    if std::io::stdin().is_terminal() {
        return Ok(());
    }
    eprintln!("This command is interactive. Run it from a terminal.");
    Err(ExitCode::from(2))
}

fn ask(prompt: &str, default: &str) -> String {
    if default.is_empty() {
        print!("{prompt}: ");
    } else {
        print!("{prompt} [{default}]: ");
    }
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    let v = line.trim().to_string();
    if v.is_empty() { default.to_string() } else { v }
}

/// Pick from a numbered list. Enter keeps the [x] defaults; otherwise comma-separated numbers.
fn choose(items: &[String], labels: &[String], defaults: &[String], what: &str) -> Result<Vec<String>, ExitCode> {
    for (i, (it, lab)) in items.iter().zip(labels).enumerate() {
        println!("  {:>2}. [{}] {lab}", i + 1, if defaults.contains(it) { "x" } else { " " });
    }
    let raw = ask(&format!("{what} - numbers, comma-separated (Enter = keep [x])"), "");
    if raw.is_empty() {
        return Ok(defaults.to_vec());
    }
    raw.split(',')
        .filter(|x| !x.trim().is_empty())
        .map(|x| x.trim().parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|n| items.get(n).cloned()))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            eprintln!("Invalid number.");
            ExitCode::from(2)
        })
}

// ---------- add ----------

fn add_mail(paths: &Paths) -> Result<(), ExitCode> {
    need_tty()?;
    let cur = sources::view(paths).mail;
    let (host0, port0) =
        cur.as_ref().map(|m| (m.host.clone(), m.port.to_string())).unwrap_or_else(|| (String::new(), "993".into()));
    let env = read_env_file(&settings::env_file(paths));
    let user0 = env.get("IMAP_USER").cloned().unwrap_or_default();
    let host = ask("IMAP server", &host0);
    let Ok(port) = ask("Port", &port0).parse::<u16>() else {
        eprintln!("The port is a number.");
        return Err(ExitCode::from(2));
    };
    let user = ask("Account", &user0);
    let typed = rpassword::prompt_password("Password (hidden; Enter keeps the current one): ").unwrap_or_default();
    let password = (!typed.is_empty()).then_some(typed);
    let same_server = cur.as_ref().is_some_and(|m| m.host == host && m.port == port && m.has_password);
    if host.is_empty() || user.is_empty() || (password.is_none() && !same_server) {
        eprintln!("Server, account and password are all required.");
        return Err(ExitCode::from(2));
    }
    let folders = sources::mail_folders(paths, &host, port, &user, &password).map_err(|e| {
        eprintln!("Connection failed: {e}. Nothing was saved.");
        ExitCode::FAILURE
    })?;
    println!("Connected. {} folders:", folders.len());
    let names: Vec<String> = folders.iter().map(|f| f.name.clone()).collect();
    let labels: Vec<String> = folders.iter().map(|f| f.label.clone()).collect();
    let defaults: Vec<String> = folders.iter().filter(|f| f.chosen).map(|f| f.name.clone()).collect();
    let chosen = choose(&names, &labels, &defaults, "Folders to collect")?;
    if chosen.is_empty() {
        eprintln!("No folder selected. Nothing was saved.");
        return Err(ExitCode::from(2));
    }
    let shown: Vec<String> = chosen.iter().map(|c| setup::imap_utf7_decode(c)).collect();
    sources::save_mail(paths, &sources::MailSetup { host, port, user, password, folders: chosen }).map_err(|e| {
        eprintln!("{e}");
        ExitCode::FAILURE
    })?;
    println!(
        "Saved: {} (mode 600), {}\nFolders: {}\nNext: life sync --dry-run",
        settings::env_file(paths).display(),
        settings::sources_file(paths).display(),
        shown.join(", ")
    );
    Ok(())
}

fn add_calendar(paths: &Paths) -> Result<(), ExitCode> {
    need_tty()?;
    let env = read_env_file(&settings::env_file(paths));
    if env.get("CALDAV_USER").or_else(|| env.get("IMAP_USER")).is_none_or(|u| u.is_empty()) {
        eprintln!("Run `life add mail` first (calendars use the same account).");
        return Err(ExitCode::from(2));
    }
    let existing = sources::view(paths).calendars.first().cloned().unwrap_or_default();
    let base = url::Url::parse(&existing)
        .map(|u| format!("{}://{}/", u.scheme(), u.host_str().unwrap_or("")))
        .unwrap_or_default();
    let server = ask("CalDAV server", &base);
    if server.is_empty() {
        eprintln!("A CalDAV server URL is required.");
        return Err(ExitCode::from(2));
    }
    let cals = sources::calendars(paths, &server).map_err(|e| {
        eprintln!("{e}");
        ExitCode::FAILURE
    })?;
    let urls: Vec<String> = cals.iter().map(|c| c.url.clone()).collect();
    let labels: Vec<String> = cals.iter().map(|c| format!("{}  ({})", c.name, c.components)).collect();
    let defaults: Vec<String> = cals.iter().filter(|c| c.chosen).map(|c| c.url.clone()).collect();
    let chosen = choose(&urls, &labels, &defaults, "Calendars to collect")?;
    if chosen.is_empty() {
        eprintln!("No calendar selected. Nothing was saved.");
        return Err(ExitCode::from(2));
    }
    sources::save_calendars(paths, &chosen).map_err(|e| {
        eprintln!("{e}");
        ExitCode::FAILURE
    })?;
    println!("Saved: {}\nNext: life sync --dry-run", settings::sources_file(paths).display());
    Ok(())
}

fn add_contacts(paths: &Paths) -> Result<(), ExitCode> {
    need_tty()?;
    if sources::calendar_account(paths).is_err() {
        eprintln!("Run `life add mail` first (address books use the same account).");
        return Err(ExitCode::from(2));
    }
    let existing = sources::view(paths).books.first().cloned().unwrap_or_default();
    let base = url::Url::parse(&existing)
        .map(|u| format!("{}://{}/", u.scheme(), u.host_str().unwrap_or("")))
        .unwrap_or_default();
    let server = ask("CardDAV server", &base);
    if server.is_empty() {
        eprintln!("A CardDAV server URL is required.");
        return Err(ExitCode::from(2));
    }
    let books = sources::books(paths, &server).map_err(|e| {
        eprintln!("{e}");
        ExitCode::FAILURE
    })?;
    let urls: Vec<String> = books.iter().map(|b| b.url.clone()).collect();
    let labels: Vec<String> = books.iter().map(|b| b.name.clone()).collect();
    let defaults: Vec<String> = books.iter().filter(|b| b.chosen).map(|b| b.url.clone()).collect();
    let chosen = choose(&urls, &labels, &defaults, "Address books to collect")?;
    if chosen.is_empty() {
        eprintln!("No address book selected. Nothing was saved.");
        return Err(ExitCode::from(2));
    }
    sources::save_books(paths, &chosen).map_err(|e| {
        eprintln!("{e}");
        ExitCode::FAILURE
    })?;
    println!("Saved: {}\nNext: life sync --dry-run", settings::sources_file(paths).display());
    Ok(())
}

// ---------- sync / status ----------

fn sync(paths: &Paths, dry: bool) -> ExitCode {
    let shown = sync::sync(paths, dry, &mut |job, _, lines| {
        let text: Vec<&str> = lines
            .iter()
            .map(|l| match l {
                sync::Line::Say(s) | sync::Line::Problem(s) => s.as_str(),
            })
            .collect();
        println!("== {}\n{}\n", job.name(), text.join("\n"));
    });
    match shown {
        Err(sync::NoSources) => {
            eprintln!("No sources configured. Add one with `life add mail` or `life add calendar`.");
            ExitCode::from(2)
        }
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
    }
}

fn status(paths: &Paths) -> ExitCode {
    let names = setup::configured(&settings::read_sources(paths));
    if names.is_empty() {
        println!("No sources configured.");
        return ExitCode::SUCCESS;
    }
    let st = setup::load_status(&sync::status_file(paths));
    for name in names {
        let Some(j) = st.get(name) else {
            println!("{name:<9} never run");
            continue;
        };
        let s = |k: &str, d: &str| j.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
        println!(
            "{name:<9} {}  last run {}  last success {}\n          {}",
            if j.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) { "ok" } else { "FAILED" },
            s("last_run", "?"),
            s("last_ok", "never"),
            s("detail", "")
        );
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    life_server::logging::to_stderr();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = Paths::from_env();
    let first = args.first().map(String::as_str).unwrap_or("");
    match first {
        "add" => match args.get(1).map(String::as_str) {
            Some("mail") => add_mail(&paths).err().unwrap_or(ExitCode::SUCCESS),
            Some("calendar") => add_calendar(&paths).err().unwrap_or(ExitCode::SUCCESS),
            Some("contacts") => add_contacts(&paths).err().unwrap_or(ExitCode::SUCCESS),
            _ => {
                eprintln!("usage: life add {{mail,calendar,contacts}}");
                ExitCode::from(2)
            }
        },
        "sync" => match args.get(1).map(String::as_str) {
            None => sync(&paths, false),
            Some("--dry-run") => sync(&paths, true),
            Some(other) => {
                eprintln!("usage: life sync [--dry-run]\nlife: error: unrecognized arguments: {other}");
                ExitCode::from(2)
            }
        },
        "status" => status(&paths),
        "export" => {
            let out = args
                .get(1)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| format!("lorelog-export-{}.zip", chrono::Local::now().format("%Y-%m-%d")).into());
            match life_server::export::export(&paths, &out) {
                Ok(r) => {
                    println!("exported {} files ({} MB) to {}", r.files, r.bytes / 1_000_000, out.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("life: export failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "backup" => match args.get(1).map(String::as_str) {
            None => match life_server::backup::run(&paths) {
                Ok(d) => {
                    println!("{} files ({} MB) sealed and checked: {}", d.files, d.bytes / 1_000_000, d.file.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("life: backup failed: {e}");
                    ExitCode::FAILURE
                }
            },
            Some("set") => {
                let Some(dir) = args.get(2) else {
                    eprintln!("usage: life backup set DIR");
                    return ExitCode::from(2);
                };
                if let Err(c) = need_tty() {
                    return c;
                }
                let pw = rpassword::prompt_password("password for the copies (8+ letters; keep it elsewhere too): ")
                    .unwrap_or_default();
                if rpassword::prompt_password("again: ").unwrap_or_default() != pw {
                    eprintln!("life: the two did not match; nothing changed");
                    return ExitCode::FAILURE;
                }
                match life_server::backup::set(&paths, std::path::Path::new(dir), &pw) {
                    Ok(()) => {
                        println!(
                            "copies go to {dir}; the first is made on the next `life sync` (or `life backup` now)"
                        );
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("life: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
            Some(other) => {
                eprintln!("usage: life backup [set DIR]\nlife: error: unrecognized arguments: {other}");
                ExitCode::from(2)
            }
        },
        "restore" => {
            let (Some(file), Some(out)) = (args.get(1), args.get(2)) else {
                eprintln!("usage: life restore FILE OUT");
                return ExitCode::from(2);
            };
            if let Err(c) = need_tty() {
                return c;
            }
            let pw = rpassword::prompt_password("password: ").unwrap_or_default();
            match life_server::backup::restore(std::path::Path::new(file), &pw, std::path::Path::new(out)) {
                Ok(n) => {
                    println!("ok: {n} files match their hashes; unzip {out} to see them");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("life: restore failed (wrong password or a damaged copy): {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "held" => {
            for h in life_server::vault::holdings(&paths) {
                println!(
                    "{:<9} {:<14} {:>6} files {:>8} MB  {} ~ {}",
                    h.source,
                    h.label,
                    h.files,
                    h.bytes / 1_000_000,
                    h.first,
                    h.last
                );
            }
            ExitCode::SUCCESS
        }
        "forget" => {
            let (Some(src), Some(from), Some(to)) = (args.get(1), args.get(2), args.get(3)) else {
                eprintln!("usage: life forget SOURCE FROM TO [--yes]");
                return ExitCode::from(2);
            };
            let span = life_server::vault::Span { from: from.clone(), to: to.clone() };
            let yes = args.get(4).map(String::as_str) == Some("--yes");
            let run = if yes {
                life_server::vault::forget(&paths, src, &span)
            } else {
                life_server::vault::plan(&paths, src, &span)
            };
            match run {
                Ok(p) => {
                    let verb = if yes { "forgot" } else { "would forget (run again with --yes)" };
                    println!(
                        "{verb}: {} files ({} MB), {} proposed to-dos",
                        p.files.len(),
                        p.bytes / 1_000_000,
                        p.todos
                    );
                    if p.comes_back {
                        println!("note: this source is read whole again on every sync; what is still there comes back");
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("life: forget failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "verify" => match args.get(1) {
            Some(f) => match life_server::export::verify(std::path::Path::new(f)) {
                Ok(n) => {
                    println!("ok: {n} files match their hashes");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("life: verify failed: {e}");
                    ExitCode::FAILURE
                }
            },
            None => {
                eprintln!("usage: life verify FILE");
                ExitCode::from(2)
            }
        },
        "" | "-h" | "--help" => {
            println!("{}", help(&paths));
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("usage: life <command> ...\nlife: error: argument <command>: invalid choice: '{other}'");
            ExitCode::from(2)
        }
    }
}
