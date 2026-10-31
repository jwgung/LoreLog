//! What to collect, set up without a terminal: the desktop app's settings screen and `life add` both use these. The
//! mail account's secrets go to `env` (0600), what to collect to `sources.toml` (no secrets). Reading back never
//! returns a password, only whether one is stored.
use serde::Serialize;

use super::calendar::{self, Account};
use super::{imapc, settings, setup};
use crate::config::{Paths, read_env_file};
use crate::fsutil::write_private_atomic;

const SOURCES_HEADER: &str = "# What to collect. No passwords or tokens here (they live in env). Safe to commit.\n# `life add mail`, `life add calendar` and the desktop app's settings rewrite this file, so comments are not preserved.\n\n";

/// A mail account to save. `password: None` keeps the stored one (only for the same server).
pub struct MailSetup {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    /// raw IMAP names, as the server lists them
    pub folders: Vec<String>,
}

/// The mail account as configured, without its password.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MailView {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub has_password: bool,
    pub folders: Vec<String>,
    /// the folders as people read them
    pub folder_labels: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourcesView {
    pub mail: Option<MailView>,
    pub calendars: Vec<String>,
    /// CardDAV address books
    pub books: Vec<String>,
}

/// A folder the server has, for choosing.
#[derive(Clone, Debug, Serialize)]
pub struct Folder {
    pub name: String,
    pub label: String,
    /// collected now, or (nothing chosen yet) offered first
    pub chosen: bool,
}

/// A calendar the account has, for choosing.
#[derive(Clone, Debug, Serialize)]
pub struct Calendar {
    pub name: String,
    pub url: String,
    /// what it can hold, e.g. "VEVENT,VTODO"
    pub components: String,
    pub chosen: bool,
}

fn strings(t: &toml::Table, k: &str) -> Vec<String> {
    t.get(k)
        .and_then(toml::Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str()).map(str::to_string).collect())
        .unwrap_or_default()
}

fn text(t: &toml::Table, k: &str) -> String {
    t.get(k).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default()
}

fn table(src: &toml::Table, name: &str) -> toml::Table {
    src.get(name).and_then(toml::Value::as_table).cloned().unwrap_or_default()
}

pub fn view(paths: &Paths) -> SourcesView {
    let src = settings::read_sources(paths);
    let env = read_env_file(&settings::env_file(paths));
    let mail = src.get("mail").and_then(toml::Value::as_table).map(|t| {
        let folders = strings(t, "folders");
        MailView {
            host: text(t, "host"),
            port: t.get("port").and_then(toml::Value::as_integer).and_then(|p| u16::try_from(p).ok()).unwrap_or(993),
            user: env.get("IMAP_USER").cloned().unwrap_or_default(),
            has_password: env.get("IMAP_PASSWORD").is_some_and(|p| !p.is_empty()),
            folder_labels: folders.iter().map(|f| setup::imap_utf7_decode(f)).collect(),
            folders,
        }
    });
    SourcesView {
        mail,
        calendars: strings(&table(&src, "calendar"), "urls"),
        books: strings(&table(&src, "contacts"), "urls"),
    }
}

pub fn write_sources(paths: &Paths, src: &toml::Table) -> std::io::Result<()> {
    let text = format!("{SOURCES_HEADER}{}", toml::to_string(src).unwrap_or_default());
    write_private_atomic(&settings::sources_file(paths), text.as_bytes())
}

/// The password to use: the given one, else the stored one when the server is the one it was stored for (a stored
/// password never goes to another server). Err when there is none to use.
fn password(paths: &Paths, host: &str, port: u16, given: &Option<String>) -> Result<String, String> {
    if let Some(p) = given.clone().filter(|p| !p.is_empty()) {
        return Ok(p);
    }
    let same = view(paths).mail.is_some_and(|m| m.host.eq_ignore_ascii_case(host.trim()) && m.port == port);
    let stored = read_env_file(&settings::env_file(paths)).get("IMAP_PASSWORD").cloned().unwrap_or_default();
    if same && !stored.is_empty() { Ok(stored) } else { Err("비밀번호를 넣어 주세요".into()) }
}

/// Log in and list the folders that can hold mail; the configured ones (else INBOX and the sent folder) are chosen.
/// Nothing is saved. `password: None` uses the stored one.
pub fn mail_folders(
    paths: &Paths,
    host: &str,
    port: u16,
    user: &str,
    pw: &Option<String>,
) -> Result<Vec<Folder>, String> {
    let pw = password(paths, host, port, pw)?;
    let mut s = imapc::connect(host.trim(), port, user.trim(), &pw).map_err(|k| format!("연결하지 못했습니다: {k}"))?;
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
        .map_err(|_| "폴더 목록을 읽지 못했습니다".to_string())?;
    if folders.is_empty() {
        return Err("폴더 목록을 읽지 못했습니다".into());
    }
    let names: Vec<&String> = folders.iter().map(|(n, _)| n).collect();
    let mut chosen: Vec<String> = strings(&table(&settings::read_sources(paths), "mail"), "folders")
        .into_iter()
        .filter(|f| names.contains(&f))
        .collect();
    if chosen.is_empty() {
        chosen = setup::default_folders(&folders);
    }
    Ok(folders
        .iter()
        .map(|(n, _)| Folder { name: n.clone(), label: setup::imap_utf7_decode(n), chosen: chosen.contains(n) })
        .collect())
}

/// Save the mail account and the folders to collect. Err (nothing saved) when something required is missing.
pub fn save_mail(paths: &Paths, m: &MailSetup) -> Result<(), String> {
    let (host, user) = (m.host.trim(), m.user.trim());
    if host.is_empty() || user.is_empty() {
        return Err("메일 서버와 계정을 넣어 주세요".into());
    }
    if m.folders.is_empty() {
        return Err("모을 폴더를 하나 이상 골라 주세요".into());
    }
    let pw = password(paths, host, m.port, &m.password)?;
    if ![host, user, pw.as_str()].iter().all(|v| crate::config::env_value_ok(v)) {
        return Err("줄바꿈 같은 제어 문자는 넣을 수 없습니다".into());
    }

    let mut src = settings::read_sources(paths);
    let mut t = table(&src, "mail");
    t.insert("host".into(), toml::Value::String(host.to_string()));
    t.insert("port".into(), toml::Value::Integer(m.port.into()));
    t.entry("account").or_insert_with(|| toml::Value::String("account".into()));
    t.insert("folders".into(), toml::Value::Array(m.folders.iter().cloned().map(toml::Value::String).collect()));
    src.insert("mail".into(), toml::Value::Table(t));
    let fail = |e: std::io::Error| format!("저장하지 못했습니다: {:?}", e.kind());
    crate::config::set_env_keys(
        &settings::env_file(paths),
        &[("IMAP_USER", Some(user)), ("IMAP_PASSWORD", Some(&pw))],
    )?;
    crate::fsutil::ensure_private_dir(&paths.conf).map_err(fail)?;
    write_sources(paths, &src).map_err(fail)
}

/// The account (the mail account's, unless CALDAV_* are set) for calendars and address books.
pub fn calendar_account(paths: &Paths) -> Result<Account, String> {
    let env = read_env_file(&settings::env_file(paths));
    let get =
        |a: &str, b: &str| env.get(a).filter(|v| !v.is_empty()).or_else(|| env.get(b)).cloned().unwrap_or_default();
    let acct = Account { user: get("CALDAV_USER", "IMAP_USER"), password: get("CALDAV_PASSWORD", "IMAP_PASSWORD") };
    if acct.user.is_empty() || acct.password.is_empty() {
        return Err("먼저 메일 계정을 넣어 주세요(일정은 같은 계정을 씁니다)".into());
    }
    Ok(acct)
}

/// The calendars that hold events on the CalDAV server, the configured ones (else all) chosen. Nothing is saved.
pub fn calendars(paths: &Paths, server: &str) -> Result<Vec<Calendar>, String> {
    let acct = calendar_account(paths)?;
    let cals = calendar::list_calendars(server.trim(), &acct)?;
    let configured = view(paths).calendars;
    let rows: Vec<(String, String, String)> = cals.into_iter().filter(|c| c.2.contains("VEVENT")).collect();
    if rows.is_empty() {
        return Err("일정을 담는 달력(VEVENT)을 찾지 못했습니다".into());
    }
    let any = rows.iter().any(|(_, u, _)| configured.contains(u));
    Ok(rows
        .into_iter()
        .map(|(name, url, components)| Calendar { chosen: !any || configured.contains(&url), name, url, components })
        .collect())
}

/// Save the calendars to collect (https addresses). An empty list stops collecting calendars.
pub fn save_calendars(paths: &Paths, urls: &[String]) -> Result<(), String> {
    for u in urls {
        calendar::check_url(u).map_err(|e| format!("{u}: {e}"))?;
    }
    let mut src = settings::read_sources(paths);
    if urls.is_empty() {
        src.remove("calendar");
    } else {
        let mut t = table(&src, "calendar");
        t.insert("urls".into(), toml::Value::Array(urls.iter().cloned().map(toml::Value::String).collect()));
        t.entry("past_days").or_insert(toml::Value::Integer(400));
        t.entry("future_days").or_insert(toml::Value::Integer(400));
        src.insert("calendar".into(), toml::Value::Table(t));
    }
    crate::fsutil::ensure_private_dir(&paths.conf).map_err(|e| format!("저장하지 못했습니다: {:?}", e.kind()))?;
    write_sources(paths, &src).map_err(|e| format!("저장하지 못했습니다: {:?}", e.kind()))
}

/// An address book the account has, for choosing.
#[derive(Clone, Debug, Serialize)]
pub struct Book {
    pub name: String,
    pub url: String,
    pub chosen: bool,
}

/// The address books on a CardDAV server, the configured ones (else all) chosen. Nothing is saved.
pub fn books(paths: &Paths, server: &str) -> Result<Vec<Book>, String> {
    let acct = calendar_account(paths)?;
    let found = super::carddav::list_books(server.trim(), &acct)?;
    if found.is_empty() {
        return Err("주소록을 찾지 못했습니다".into());
    }
    let configured = view(paths).books;
    let any = found.iter().any(|(_, u)| configured.contains(u));
    Ok(found.into_iter().map(|(name, url)| Book { chosen: !any || configured.contains(&url), name, url }).collect())
}

/// Save the address books to collect (https). An empty list stops collecting them.
pub fn save_books(paths: &Paths, urls: &[String]) -> Result<(), String> {
    for u in urls {
        calendar::check_url(u).map_err(|e| format!("{u}: {e}"))?;
    }
    let mut src = settings::read_sources(paths);
    if urls.is_empty() {
        src.remove("contacts");
    } else {
        let mut t = table(&src, "contacts");
        t.insert("urls".into(), toml::Value::Array(urls.iter().cloned().map(toml::Value::String).collect()));
        src.insert("contacts".into(), toml::Value::Table(t));
    }
    crate::fsutil::ensure_private_dir(&paths.conf).map_err(|e| format!("저장하지 못했습니다: {:?}", e.kind()))?;
    write_sources(paths, &src).map_err(|e| format!("저장하지 못했습니다: {:?}", e.kind()))
}
