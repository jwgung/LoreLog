//! A sync: mail, its attachments, calendars, and last the librarian, run in-process. The command line tools
//! (`mail-sync`, `mail-files`, `calendar-sync`, `life-librarian`, `life sync`) and the desktop app call the same jobs;
//! when to run them is the caller's business (a systemd timer, the desktop app's hourly tick).
//!
//! A job reports in lines (what it says, what went wrong) instead of printing, and ends in an [`Outcome`].
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use chrono::{Local, TimeDelta};
use regex::Regex;
use serde_json::Value;

use super::calendar::{self, Account, Counts, DavError};
use super::{imapc, mail, mailfiles, settings, setup};
use crate::config::{OsDirs, Paths};
use crate::librarian::Librarian;
use crate::store::RawStore;

/// One line a job reports: what it did, or what went wrong (the command line prints these to stdout and stderr).
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Say(String),
    Problem(String),
}

/// How a job ended. The command line tools exit 0, 1 and 2 for these.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Ok,
    /// it ran, but something could not be read or written
    Failed,
    /// its settings are missing
    NotSetUp,
}

impl Outcome {
    pub fn exit_code(self) -> u8 {
        match self {
            Outcome::Ok => 0,
            Outcome::Failed => 1,
            Outcome::NotSetUp => 2,
        }
    }
}

/// Where a job's lines go.
pub type Out<'a> = &'a mut dyn FnMut(Line);

/// How the command line tools show a line: what a job says on stdout, its problems on stderr.
pub fn console(line: Line) {
    match line {
        Line::Say(s) => println!("{s}"),
        Line::Problem(s) => eprintln!("{s}"),
    }
}

fn say(out: Out, s: impl Into<String>) {
    out(Line::Say(s.into()))
}

fn problem(out: Out, s: impl Into<String>) {
    out(Line::Problem(s.into()))
}

fn dry_mark(dry: bool) -> &'static str {
    if dry { "[dry-run] " } else { "" }
}

// ---------- mail ----------

pub const MAIL_LIMIT: usize = 300;
pub const FILES_LIMIT: usize = 100;
/// attachments read per folder in one sync
const SYNC_FILES_LIMIT: usize = 200;

/// A run of a mail job: `limit` mails per folder at most, `folder` instead of the configured folders.
#[derive(Clone, Debug)]
pub struct MailRun {
    pub dry: bool,
    pub limit: usize,
    pub folder: Option<String>,
}

/// The mail settings, or a line saying what is missing (`with_hint`: name the files and how to set them up, as
/// mail-sync always has; mail-files names the files only).
fn imap_settings(paths: &Paths, run: &MailRun, out: Out, with_hint: bool) -> Option<settings::Imap> {
    let need = match settings::imap(paths, run.folder.clone()) {
        Ok(c) => return Some(c),
        Err(need) => need.join(", "),
    };
    let (env, src) = (settings::env_file(paths), settings::sources_file(paths));
    problem(
        out,
        if with_hint {
            format!("Missing {need} (env: {}, sources.toml: {}). Run `life add mail`.", env.display(), src.display())
        } else {
            format!("Missing {need} ({}, {}).", env.display(), src.display())
        },
    );
    None
}

fn connect(cfg: &settings::Imap, out: Out) -> Option<imapc::Session> {
    match imapc::connect(&cfg.host, cfg.port, &cfg.user, &cfg.password) {
        Ok(s) => Some(s),
        Err(k) => {
            problem(out, format!("IMAP connection failed: {k}"));
            None
        }
    }
}

/// The user's own addresses (to tell sent from received), from hub-sync's identity file if there is one.
fn own_addresses() -> HashSet<String> {
    let identity = OsDirs::here().data.join("hub-sync/identity.json");
    std::fs::read_to_string(identity)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| {
            v["emails"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_lowercase).collect())
        })
        .unwrap_or_default()
}

/// Import mail into the vault as `Mail/` notes, read-only on the server. A mail that already has a note (by Message-ID
/// or by a date/time/sender/subject fingerprint) is skipped; processed UIDs are remembered per folder.
pub fn mail(paths: &Paths, run: &MailRun, out: Out) -> Outcome {
    let Some(cfg) = imap_settings(paths, run, out, true) else { return Outcome::NotSetUp };
    let _lock = if run.dry {
        None // a dry run writes nothing, so it needs no lock
    } else {
        match settings::lock("mail-sync") {
            Some(l) => Some(l),
            None => {
                say(out, "Another mail-sync run is in progress; skipping this one.");
                return Outcome::Ok;
            }
        }
    };
    let own = own_addresses();
    let state_file = settings::state_dir().join("mail-state.json");
    let mut state = imapc::State::load(&state_file);
    let (mut known, mut known_fp) = mail::known_notes(&paths.vault);
    let synced = Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string();
    let Some(mut s) = connect(&cfg, out) else { return Outcome::Failed };
    let (mut total, mut failed) = (0usize, false);
    for folder in &cfg.folders {
        let (uidv, all) = match imapc::open(&mut s, folder, cfg.since.as_deref()) {
            Ok(x) => x,
            Err(_) => {
                problem(out, format!("[{folder}] cannot open folder"));
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
                    problem(out, format!("[{folder}] cannot read headers"));
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
        let take: Vec<u32> = fresh.iter().copied().take(run.limit).collect();
        say(
            out,
            format!(
                "[{folder}] server {} · already known/processed {} · new {} (this run {})",
                all.len(),
                all.len() - fresh.len(),
                fresh.len(),
                take.len()
            ),
        );
        let mut wrote = 0;
        for u in take {
            let raw = match imapc::body(&mut s, u) {
                Ok(Some(r)) => r,
                Ok(None) => continue,
                Err(_) => {
                    problem(out, format!("[{folder}] cannot read a mail"));
                    failed = true;
                    break;
                }
            };
            let Some(msg) = mail::parse(&raw) else { continue };
            let (rel, note, mid) = mail::render(&msg, &format!("{}/{folder}", cfg.account), &own, &synced);
            seen.insert(u);
            let fp = mail::msg_fingerprint(&msg);
            if known.contains(&mid) || (!fp.is_empty() && known_fp.contains(&fp)) {
                continue;
            }
            if !run.dry
                && let Err(e) = mail::write_note(&paths.vault, &rel, &note)
            {
                problem(out, format!("[{folder}] writing a note failed: {:?}", e.kind()));
                failed = true;
                break;
            }
            known.insert(mid);
            known_fp.insert(fp);
            wrote += 1;
        }
        total += wrote;
        if !run.dry {
            state.set(folder, &uidv, &seen);
        }
    }
    let _ = s.logout();
    if !run.dry
        && let Err(e) = state.save(&state_file)
    {
        problem(out, format!("saving the state failed: {:?}", e.kind()));
        failed = true;
    }
    say(out, format!("{}{total} new notes", dry_mark(run.dry)));
    if failed { Outcome::Failed } else { Outcome::Ok }
}

/// One folder's attachments -> (mails read, files seen, bytes written), or what went wrong.
fn files_folder(
    s: &mut imapc::Session,
    folder: &str,
    account: &str,
    store: &RawStore,
    state: &mut imapc::State,
    run: &MailRun,
    out: Out,
) -> Result<(usize, usize, usize), String> {
    static MULTIPART: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)multipart/(mixed|related)").unwrap());
    let (uidv, all) = imapc::open(s, folder, None).map_err(|_| format!("[{folder}] cannot open folder"))?;
    let mut seen = state.seen(folder, &uidv);
    let todo: Vec<u32> = all.iter().copied().filter(|u| !seen.contains(u)).collect();
    let take: Vec<u32> = todo.iter().copied().take(run.limit).collect();
    say(
        out,
        format!(
            "[{folder}] server {} · not yet read {} (this run {})",
            seen.len() + todo.len(),
            todo.len(),
            take.len()
        ),
    );
    if run.dry {
        let sizes = imapc::sizes(s, &take).map_err(|_| format!("[{folder}] cannot read sizes"))?;
        let carrying: Vec<&(u32, String)> = sizes.iter().filter(|(_, h)| MULTIPART.is_match(h)).collect();
        let bytes: u64 = carrying.iter().map(|(n, _)| *n as u64).sum();
        say(
            out,
            format!(
                "[dry-run] {} of {} mails look like they carry attachments, about {:.1} MB to read",
                carrying.len(),
                take.len(),
                bytes as f64 / 1e6
            ),
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

/// Keep the attachments of the mails in the configured folders, as they are, in the raw store's vault of files.
pub fn files(paths: &Paths, run: &MailRun, out: Out) -> Outcome {
    let Some(cfg) = imap_settings(paths, run, out, false) else { return Outcome::NotSetUp };
    if cfg.folders.is_empty() {
        problem(out, "No mail folders configured.");
        return Outcome::NotSetUp;
    }
    let store = RawStore::new(&paths.data);
    let state_file = paths.data.join("mail-files-state.json");
    let mut state = imapc::State::load(&state_file);
    let Some(mut s) = connect(&cfg, out) else { return Outcome::Failed };
    let (mut failed, mut totals) = (false, (0, 0, 0));
    for folder in &cfg.folders {
        match files_folder(&mut s, folder, &cfg.account, &store, &mut state, run, out) {
            Ok((m, f, w)) => totals = (totals.0 + m, totals.1 + f, totals.2 + w),
            Err(p) => {
                problem(out, p);
                failed = true;
            }
        }
        if !run.dry
            && let Err(e) = state.save(&state_file)
        {
            problem(out, format!("saving the state failed: {:?}", e.kind())); // after each folder: a run resumes
            failed = true;
        }
    }
    let _ = s.logout();
    if !run.dry {
        say(
            out,
            format!(
                "read {} mails · {} attachments seen · {:.1} MB newly written",
                totals.0,
                totals.1,
                totals.2 as f64 / 1e6
            ),
        );
    }
    if failed { Outcome::Failed } else { Outcome::Ok }
}

// ---------- calendar ----------

/// What calendar-sync needs: the calendars, the account, and the window of days around today.
pub struct CalendarSetup {
    pub urls: Vec<String>,
    pub account: Account,
    pub past_days: i64,
    pub future_days: i64,
}

/// The calendar settings (credentials default to the mail account's). Err: what is missing, as a line to show.
pub fn calendar_setup(paths: &Paths) -> Result<CalendarSetup, String> {
    let cfg = settings::calendar(paths);
    let urls = settings::list(cfg.get("CALDAV_URL"), "");
    let get =
        |a: &str, b: &str| cfg.get(a).filter(|v| !v.is_empty()).or_else(|| cfg.get(b)).cloned().unwrap_or_default();
    let account = Account { user: get("CALDAV_USER", "IMAP_USER"), password: get("CALDAV_PASSWORD", "IMAP_PASSWORD") };
    if urls.is_empty() || account.user.is_empty() || account.password.is_empty() {
        return Err(format!(
            "Missing calendar URL or credentials (env: {}, sources.toml: {}). Run `life add calendar`.",
            settings::env_file(paths).display(),
            settings::sources_file(paths).display()
        ));
    }
    let days = |k: &str| cfg.get(k).and_then(|v| v.parse::<i64>().ok()).unwrap_or(400);
    Ok(CalendarSetup { urls, account, past_days: days("CALDAV_PAST_DAYS"), future_days: days("CALDAV_FUTURE_DAYS") })
}

/// The last part of a calendar URL, at most `n` characters (how calendars are named in notes and lines).
fn short(url: &str, n: usize) -> String {
    url.trim_end_matches('/').rsplit('/').next().unwrap_or("").chars().take(n).collect()
}

/// Import events from the CalDAV calendars into the vault as `Events/` notes, read-only on the server. An event is
/// known by (dav_uid, start); only events whose etag changed are rewritten.
pub fn calendar(paths: &Paths, dry: bool, out: Out) -> Outcome {
    let cal = match calendar_setup(paths) {
        Ok(s) => s,
        Err(missing) => {
            problem(out, missing);
            return Outcome::NotSetUp;
        }
    };
    let _lock = if dry {
        None // a dry run writes nothing
    } else {
        match settings::lock("calendar-sync") {
            Some(l) => Some(l),
            None => {
                say(out, "Another calendar-sync run is in progress; skipping this one.");
                return Outcome::Ok;
            }
        }
    };
    let today = Local::now().date_naive();
    let start = (today - TimeDelta::days(cal.past_days)).format("%Y%m%dT000000Z").to_string();
    let end = (today + TimeDelta::days(cal.future_days)).format("%Y%m%dT000000Z").to_string();
    let acct = &cal.account;
    let mut known = calendar::existing_events(&paths.vault);
    let mut counts = Counts::default();
    let mut failed = false;
    for url in &cal.urls {
        let items = calendar::report(url, acct, &start, &end, true)
            .or_else(|e| match e {
                DavError::Status(_) => calendar::report(url, acct, &start, &end, false), // cannot expand: originals only
                other => Err(other),
            })
            .and_then(|xml| calendar::events_from(url, acct, &xml, 50)); // a failing multiget is a failed calendar
        let items = match items {
            Ok(i) => i,
            Err(e) => {
                let why = match &e {
                    DavError::Status(c) => format!("HTTP {c}"),
                    DavError::Other(k) => k.clone(),
                };
                problem(out, format!("CalDAV request failed ({why}): {}", short(url, usize::MAX)));
                failed = true;
                continue;
            }
        };
        let source = format!("caldav/{}", short(url, 12));
        match calendar::apply(&paths.vault, &source, &items, &mut known, dry, &mut counts) {
            Ok(n) => say(out, format!("[{}] {n} events on server", short(url, 12))),
            Err(e) => {
                problem(out, format!("writing notes failed: {:?}", e.kind()));
                failed = true;
            }
        }
    }
    say(
        out,
        format!("{}new notes {}, updated {}, unchanged {}", dry_mark(dry), counts.created, counts.updated, counts.same),
    );
    if failed { Outcome::Failed } else { Outcome::Ok }
}

// ---------- address books ----------

/// Read the configured CardDAV address books and keep each in the raw store when it changed (read-only on the server).
pub fn books(paths: &Paths, dry: bool, out: Out) -> Outcome {
    let urls = super::sources::view(paths).books;
    let acct = match super::sources::calendar_account(paths) {
        Ok(a) => a,
        Err(e) => {
            problem(out, e);
            return Outcome::NotSetUp;
        }
    };
    let (mut cards, mut changed, mut failed) = (0usize, 0usize, false);
    for url in &urls {
        match super::carddav::fetch(url, &acct) {
            Ok(c) => {
                cards += c.len();
                if dry {
                    continue;
                }
                match super::carddav::keep(paths, url, &c) {
                    Ok(true) => changed += 1,
                    Ok(false) => {}
                    Err(e) => {
                        problem(out, format!("keeping the address book failed: {:?}", e.kind()));
                        failed = true;
                    }
                }
            }
            Err(e) => {
                let why = match &e {
                    DavError::Status(c) => format!("HTTP {c}"),
                    DavError::Other(k) => k.clone(),
                };
                problem(out, format!("CardDAV request failed ({why}): {}", super::calendar::shown(url)));
                failed = true;
            }
        }
    }
    say(out, format!("{}{} address books, {cards} contacts, {changed} changed", dry_mark(dry), urls.len()));
    if failed { Outcome::Failed } else { Outcome::Ok }
}

// ---------- librarian ----------

/// Bring the librarian's index up to date with the records (`rebuild`: throw it away and build it again, in one
/// transaction, so a search meanwhile sees the old index).
pub fn librarian(paths: &Paths, rebuild: bool, out: Out) -> Outcome {
    let result = (|| -> anyhow::Result<String> {
        let mut lib = Librarian::open(paths, None, Duration::from_secs(30))?;
        let t = Instant::now();
        let r = lib.update(rebuild)?;
        let today = Local::now().format("%Y-%m-%d").to_string();
        if let Err(e) = crate::librarian::asks::prune(paths, &today, crate::librarian::asks::KEEP_DAYS) {
            log::warn!("dropping old questions failed: {:?}", e.kind()); // the index is up to date either way
        }
        let skipped =
            if r.skipped > 0 { format!(", skipped {} (file names on stderr)", r.skipped) } else { String::new() };
        Ok(format!(
            "indexed {} files, removed {}{skipped}, {} documents ({:.1}s)",
            r.indexed,
            r.removed,
            r.docs,
            t.elapsed().as_secs_f64()
        ))
    })();
    match result {
        Ok(line) => {
            say(out, line);
            Outcome::Ok
        }
        Err(e) => {
            problem(out, format!("life-librarian: {e}"));
            Outcome::Failed
        }
    }
}

// ---------- a whole sync ----------

/// The jobs of a sync, by the names `life status` shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Job {
    Mail,
    Files,
    Calendar,
    Books,
    Librarian,
    /// the graph (#45): the next records read by the model, after the index is up to date
    Graph,
    /// the day's sealed copy of the vault (#16), last
    Backup,
}

impl Job {
    pub fn name(self) -> &'static str {
        match self {
            Job::Mail => "mail",
            Job::Files => "files",
            Job::Calendar => "calendar",
            Job::Books => "contacts",
            Job::Librarian => "librarian",
            Job::Graph => "graph",
            Job::Backup => "backup",
        }
    }
}

/// No source is configured, so there is nothing to sync.
#[derive(Debug, PartialEq)]
pub struct NoSources;

/// Where the last result of every job is kept.
pub fn status_file(paths: &Paths) -> PathBuf {
    paths.data.join("status.json")
}

/// The jobs of a sync, in order: mail, its attachments, calendars, the librarian (it indexes what was just collected
/// and the phone and PC records that arrived since), and once a day the backup. A dry run saves nothing, the index
/// included.
fn jobs(sources: &toml::Table, dry: bool, backup_due: bool) -> Vec<Job> {
    let has = |t: &str, k: &str| {
        sources.get(t).and_then(|v| v.get(k)).and_then(toml::Value::as_array).map(|a| !a.is_empty()).unwrap_or(false)
    };
    let mut jobs = Vec::new();
    if has("mail", "folders") {
        jobs.extend([Job::Mail, Job::Files]);
    }
    if has("calendar", "urls") {
        jobs.push(Job::Calendar);
    }
    if has("contacts", "urls") {
        jobs.push(Job::Books);
    }
    if !jobs.is_empty() && !dry {
        jobs.extend([Job::Librarian, Job::Graph]);
    }
    if backup_due && !dry {
        jobs.push(Job::Backup);
    }
    jobs
}

/// records the model reads in one sync: about ten minutes of four asks at once; a first full run spreads over a day
pub const GRAPH_LIMIT: usize = 300;

/// The chat model of these places (the home server's when none is set).
pub fn model_of(paths: &Paths) -> crate::llm::Http {
    let m = crate::host::Models::load(paths);
    let d = crate::llm::Http::from_env();
    crate::llm::Http {
        url: if m.chat_url.is_empty() { d.url } else { m.chat_url },
        model: if m.chat_model.is_empty() { d.model } else { m.chat_model },
    }
}

/// The graph: the model reads up to `limit` records not read yet (newest first).
pub fn graph(paths: &Paths, limit: usize, out: Out) -> Outcome {
    let result = (|| -> anyhow::Result<String> {
        let mut lib = Librarian::open(paths, None, Duration::from_secs(30))?;
        let t = Instant::now();
        let model = model_of(paths);
        let d =
            crate::librarian::graph::extract(&mut lib, &model, &crate::librarian::graph::Run { limit, workers: 4 })?;
        // the new role and state names, put with the ones of the same meaning (a failure waits for the next run)
        let grouped = crate::librarian::graph::group_labels(&lib, &model).unwrap_or(0);
        Ok(format!(
            "read {} records ({} failed, tried again next time), {} relations, {} names grouped, {} waiting ({:.0}s)",
            d.asked,
            d.failed,
            d.edges,
            grouped,
            d.left,
            t.elapsed().as_secs_f64()
        ))
    })();
    match result {
        Ok(line) => {
            say(out, line);
            Outcome::Ok
        }
        Err(e) => {
            problem(out, format!("graph: {e}"));
            Outcome::Failed
        }
    }
}

/// The day's copy: made, sealed and checked, or why not.
fn backup(paths: &Paths, out: Out) -> Outcome {
    match crate::backup::run(paths) {
        Ok(d) => {
            say(
                out,
                format!(
                    "{} files ({} MB) sealed and checked: {}{}",
                    d.files,
                    d.bytes / 1_000_000,
                    d.file.display(),
                    if d.removed > 0 { format!(" · {} old copies taken away", d.removed) } else { String::new() }
                ),
            );
            Outcome::Ok
        }
        Err(e) => {
            problem(out, format!("backup failed: {e}"));
            Outcome::Failed
        }
    }
}

/// What `life status` shows for a run: the last thing it said when it worked; the last problem (else the last line)
/// when it did not.
pub fn detail(outcome: Outcome, lines: &[Line]) -> String {
    let said = || lines.iter().rev().find_map(|l| if let Line::Say(s) = l { Some(s.clone()) } else { None });
    let trouble = || lines.iter().rev().find_map(|l| if let Line::Problem(s) = l { Some(s.clone()) } else { None });
    match outcome {
        Outcome::Ok => said(),
        _ => trouble().or_else(said),
    }
    .unwrap_or_default()
}

/// Run every configured job, record each result (not for a dry run) and hand each to `done` with what it said.
/// -> whether all of them worked
pub fn sync(paths: &Paths, dry: bool, done: &mut dyn FnMut(Job, Outcome, &[Line])) -> Result<bool, NoSources> {
    let jobs = jobs(&settings::read_sources(paths), dry, crate::backup::due(paths, chrono::Local::now()));
    if jobs.is_empty() {
        return Err(NoSources);
    }
    let mut all_ok = true;
    for job in jobs {
        let mut lines = Vec::new();
        let mut out = |l: Line| lines.push(l);
        let mail_run = |limit| MailRun { dry, limit, folder: None };
        // a job that panics is a failed job, and the next ones still run (as when each job was its own process)
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match job {
            Job::Mail => mail(paths, &mail_run(MAIL_LIMIT), &mut out),
            Job::Files => files(paths, &mail_run(SYNC_FILES_LIMIT), &mut out),
            Job::Calendar => calendar(paths, dry, &mut out),
            Job::Books => books(paths, dry, &mut out),
            Job::Librarian => librarian(paths, false, &mut out),
            Job::Graph => graph(paths, GRAPH_LIMIT, &mut out),
            Job::Backup => backup(paths, &mut out),
        }));
        let outcome = ran.unwrap_or_else(|_| {
            lines.push(Line::Problem(format!("{} stopped unexpectedly", job.name())));
            Outcome::Failed
        });
        if !dry
            && let Err(e) =
                setup::record(&status_file(paths), job.name(), outcome == Outcome::Ok, &detail(outcome, &lines))
        {
            lines.push(Line::Problem(format!("could not record the result: {:?}", e.kind())));
        }
        done(job, outcome, &lines);
        all_ok &= outcome == Outcome::Ok;
    }
    Ok(all_ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_librarian_indexes_after_collecting_and_a_dry_run_leaves_the_index_alone() {
        let src: toml::Table = "[mail]\nfolders = [\"INBOX\"]\n[calendar]\nurls = [\"x\"]\n".parse().unwrap();
        assert_eq!(jobs(&src, false, false), [Job::Mail, Job::Files, Job::Calendar, Job::Librarian, Job::Graph]);
        assert_eq!(jobs(&src, true, true), [Job::Mail, Job::Files, Job::Calendar]);
        let with_books: toml::Table = "[contacts]\nurls = [\"x\"]\n".parse().unwrap();
        assert_eq!(jobs(&with_books, false, false), [Job::Books, Job::Librarian, Job::Graph]);
        assert!(jobs(&toml::Table::new(), false, false).is_empty());
        assert_eq!(jobs(&src, false, true).last(), Some(&Job::Backup), "the copy holds what was just collected");
        assert_eq!(jobs(&toml::Table::new(), false, true), [Job::Backup], "a vault fed only by devices is copied too");
    }
}
