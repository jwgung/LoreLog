//! life: add, run and check collection sources from one place. See `life -h`.
//!
//! The collection is done by mail-sync, mail-files and calendar-sync (binaries next to this one); afterwards the
//! librarian indexes it. Secrets live in env (mode 600); what to collect lives in sources.toml (no secrets). Passwords
//! are read without echo.
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use life_server::collect::{calendar, imapc, settings, setup};
use life_server::config::{Paths, read_env_file};
use life_server::fsutil::write_private_atomic;

const SOURCES_HEADER: &str = "# What to collect. No passwords or tokens here (they live in env). Safe to commit.\n# `life add mail` and `life add calendar` rewrite this file, so comments are not preserved.\n\n";

fn help(paths: &Paths) -> String {
    format!(
        "usage: life <command> ...

Add, run and check collection sources from one place.
The actual collection is done by mail-sync, mail-files and calendar-sync; this command ties their setup and runs together.

commands:
  add mail          mail account (IMAP): server, account, password (hidden), then folders to collect
  add calendar      calendars (CalDAV) of the account from `life add mail`
  sync [--dry-run]  collect from every configured source; last, the librarian's index is brought up to date
                    (`life-librarian update`; not with --dry-run). Mail is capped at 300 messages per run.
  status            last run and result per source

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
        status_file(paths).display()
    )
}

fn status_file(paths: &Paths) -> PathBuf {
    paths.data.join("status.json")
}

fn bin(name: &str) -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(name))).unwrap_or_else(|| PathBuf::from(name))
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

fn write_env(paths: &Paths, env: &std::collections::BTreeMap<String, String>) -> std::io::Result<()> {
    let text: String = env.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    write_private_atomic(&settings::env_file(paths), text.as_bytes())?;
    life_server::fsutil::set_mode(&paths.conf, 0o700);
    Ok(())
}

fn write_sources(paths: &Paths, src: &toml::Table) -> std::io::Result<()> {
    let text = format!("{SOURCES_HEADER}{}", toml::to_string(src).unwrap_or_default());
    write_private_atomic(&settings::sources_file(paths), text.as_bytes())
}

// ---------- add ----------

fn add_mail(paths: &Paths) -> Result<(), ExitCode> {
    need_tty()?;
    let mut env = read_env_file(&settings::env_file(paths));
    let mut src = settings::read_sources(paths);
    let cur = src.get("mail").and_then(toml::Value::as_table).cloned().unwrap_or_default();
    let get = |k: &str| {
        cur.get(k).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default()
    };
    let host = ask("IMAP server", &get("host"));
    let port_default = if get("port").is_empty() { "993".to_string() } else { get("port") };
    let port_text = ask("Port", &port_default);
    let Ok(port) = port_text.parse::<u16>() else {
        eprintln!("The port is a number.");
        return Err(ExitCode::from(2));
    };
    let user = ask("Account", env.get("IMAP_USER").map(String::as_str).unwrap_or(""));
    let typed = rpassword::prompt_password("Password (hidden; Enter keeps the current one): ").unwrap_or_default();
    let pw = if typed.is_empty() { env.get("IMAP_PASSWORD").cloned().unwrap_or_default() } else { typed };
    if host.is_empty() || user.is_empty() || pw.is_empty() {
        eprintln!("Server, account and password are all required.");
        return Err(ExitCode::from(2));
    }
    let mut s = imapc::connect(&host, port, &user, &pw).map_err(|k| {
        eprintln!("Connection failed: {k}. Nothing was saved.");
        ExitCode::FAILURE
    })?;
    let names = s.list(None, Some("*"));
    let _ = s.logout();
    let folders: Vec<(String, Vec<String>)> = names
        .map(|ns| {
            ns.iter()
                .map(|n| {
                    // \Sent, \NoSelect, ... as the server's flags read in lower case
                    let flags: Vec<String> =
                        n.attributes().iter().map(|a| format!("\\{}", format!("{a:?}").to_lowercase())).collect();
                    (n.name().to_string(), flags)
                })
                .filter(|(_, flags)| !flags.iter().any(|f| f == "\\noselect"))
                .collect()
        })
        .unwrap_or_default();
    if folders.is_empty() {
        eprintln!("Could not read the folder list.");
        return Err(ExitCode::FAILURE);
    }
    println!("Connected. {} folders:", folders.len());
    let names: Vec<String> = folders.iter().map(|(n, _)| n.clone()).collect();
    let mut defaults: Vec<String> = cur
        .get("folders")
        .and_then(toml::Value::as_array)
        .map(|a| {
            a.iter().filter_map(|v| v.as_str()).filter(|n| names.iter().any(|x| x == n)).map(str::to_string).collect()
        })
        .unwrap_or_default();
    if defaults.is_empty() {
        defaults = setup::default_folders(&folders);
    }
    let labels: Vec<String> = names.iter().map(|n| setup::imap_utf7_decode(n)).collect();
    let chosen = choose(&names, &labels, &defaults, "Folders to collect")?;
    if chosen.is_empty() {
        eprintln!("No folder selected. Nothing was saved.");
        return Err(ExitCode::from(2));
    }
    env.insert("IMAP_USER".into(), user);
    env.insert("IMAP_PASSWORD".into(), pw);
    write_env(paths, &env).map_err(|_| ExitCode::FAILURE)?;
    let mut table = cur;
    table.insert("host".into(), toml::Value::String(host));
    table.insert("port".into(), toml::Value::Integer(port.into()));
    table.entry("account").or_insert_with(|| toml::Value::String("account".into()));
    table.insert("folders".into(), toml::Value::Array(chosen.iter().cloned().map(toml::Value::String).collect()));
    src.insert("mail".into(), toml::Value::Table(table));
    write_sources(paths, &src).map_err(|_| ExitCode::FAILURE)?;
    let shown: Vec<String> = chosen.iter().map(|c| setup::imap_utf7_decode(c)).collect();
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
    let user = env.get("CALDAV_USER").or_else(|| env.get("IMAP_USER")).cloned().unwrap_or_default();
    let password = env.get("CALDAV_PASSWORD").or_else(|| env.get("IMAP_PASSWORD")).cloned().unwrap_or_default();
    if user.is_empty() {
        eprintln!("Run `life add mail` first (calendars use the same account).");
        return Err(ExitCode::from(2));
    }
    let mut src = settings::read_sources(paths);
    let mut cur = src.get("calendar").and_then(toml::Value::as_table).cloned().unwrap_or_default();
    let existing = cur
        .get("urls")
        .and_then(toml::Value::as_array)
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let base = url::Url::parse(&existing)
        .map(|u| format!("{}://{}/", u.scheme(), u.host_str().unwrap_or("")))
        .unwrap_or_default();
    let server = ask("CalDAV server", &base);
    if server.is_empty() {
        eprintln!("A CalDAV server URL is required.");
        return Err(ExitCode::from(2));
    }
    let cals = calendar::list_calendars(&server, &calendar::Account { user, password }).map_err(|e| {
        eprintln!("{e}");
        ExitCode::FAILURE
    })?;
    let rows: Vec<&(String, String, String)> = cals.iter().filter(|c| c.2.contains("VEVENT")).collect();
    if rows.is_empty() {
        eprintln!("No calendar that supports events (VEVENT) was found.");
        return Err(ExitCode::FAILURE);
    }
    let urls: Vec<String> = rows.iter().map(|r| r.1.clone()).collect();
    let mut defaults: Vec<String> = cur
        .get("urls")
        .and_then(toml::Value::as_array)
        .map(|a| {
            a.iter().filter_map(|v| v.as_str()).filter(|u| urls.iter().any(|x| x == u)).map(str::to_string).collect()
        })
        .unwrap_or_default();
    if defaults.is_empty() {
        defaults = urls.clone();
    }
    let labels: Vec<String> = rows.iter().map(|r| format!("{}  ({})", r.0, r.2)).collect();
    let chosen = choose(&urls, &labels, &defaults, "Calendars to collect")?;
    if chosen.is_empty() {
        eprintln!("No calendar selected. Nothing was saved.");
        return Err(ExitCode::from(2));
    }
    cur.insert("urls".into(), toml::Value::Array(chosen.into_iter().map(toml::Value::String).collect()));
    cur.entry("past_days").or_insert(toml::Value::Integer(400));
    cur.entry("future_days").or_insert(toml::Value::Integer(400));
    src.insert("calendar".into(), toml::Value::Table(cur));
    write_sources(paths, &src).map_err(|_| ExitCode::FAILURE)?;
    println!("Saved: {}\nNext: life sync --dry-run", settings::sources_file(paths).display());
    Ok(())
}

// ---------- sync / status ----------

/// Run one job and record its result (not for a dry run). -> whether it worked
fn run_job(paths: &Paths, name: &str, cmd: &[String], dry: bool) -> bool {
    let out = Command::new(bin(&cmd[0])).args(&cmd[1..]).output();
    let (ok, stdout, all) = match out {
        Ok(o) => {
            let so = String::from_utf8_lossy(&o.stdout).into_owned();
            let se = String::from_utf8_lossy(&o.stderr).into_owned();
            (o.status.success(), so.clone(), format!("{so}{se}").trim().to_string())
        }
        Err(e) => (false, String::new(), format!("{}: cannot run ({:?})", cmd[0], e.kind())),
    };
    println!("== {name}\n{all}\n");
    if !dry {
        let detail = if ok {
            stdout.trim().lines().last().unwrap_or("").to_string()
        } else {
            all.lines().last().unwrap_or("").to_string()
        };
        if let Err(e) = setup::record(&status_file(paths), name, ok, &detail) {
            eprintln!("could not record the result: {:?}", e.kind());
        }
    }
    ok
}

fn sync(paths: &Paths, dry: bool) -> ExitCode {
    let jobs = setup::sync_jobs(&settings::read_sources(paths), dry);
    if jobs.is_empty() {
        eprintln!("No sources configured. Add one with `life add mail` or `life add calendar`.");
        return ExitCode::from(2);
    }
    let mut ok = true;
    for (name, cmd) in &jobs {
        ok = run_job(paths, name, cmd, dry) && ok;
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn status(paths: &Paths) -> ExitCode {
    let names = setup::configured(&settings::read_sources(paths));
    if names.is_empty() {
        println!("No sources configured.");
        return ExitCode::SUCCESS;
    }
    let st = setup::load_status(&status_file(paths));
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = Paths::from_env();
    let first = args.first().map(String::as_str).unwrap_or("");
    match first {
        "add" => match args.get(1).map(String::as_str) {
            Some("mail") => add_mail(&paths).err().unwrap_or(ExitCode::SUCCESS),
            Some("calendar") => add_calendar(&paths).err().unwrap_or(ExitCode::SUCCESS),
            _ => {
                eprintln!("usage: life add {{mail,calendar}}");
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
