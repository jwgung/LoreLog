//! Settings shared by the collectors, in `~/.config/vault-sync/` (or `LIFE_CONF_DIR`):
//!   env           secrets only (KEY=VALUE, mode 600): IMAP_USER, IMAP_PASSWORD, CALDAV_USER, CALDAV_PASSWORD
//!   sources.toml  what to collect (no secrets): [mail] host, port, account, folders, since; [calendar] urls, past_days,
//!                 future_days. Values in sources.toml win over the same settings in env.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{Paths, read_env_file};

pub fn env_file(paths: &Paths) -> PathBuf {
    paths.conf.join("env")
}

pub fn sources_file(paths: &Paths) -> PathBuf {
    paths.conf.join("sources.toml")
}

/// sources.toml as a table (empty when missing or not TOML).
pub fn read_sources(paths: &Paths) -> toml::Table {
    fs::read_to_string(sources_file(paths)).ok().and_then(|t| t.parse::<toml::Table>().ok()).unwrap_or_default()
}

fn toml_text(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Array(a) => a.iter().map(toml_text).collect::<Vec<_>>().join(","),
        other => other.to_string(),
    }
}

/// env, with the given sources.toml table's keys laid over it (`(toml key, env key)` pairs).
fn merged(paths: &Paths, table: &str, keys: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut cfg = read_env_file(&env_file(paths));
    if let Some(t) = read_sources(paths).get(table).and_then(toml::Value::as_table) {
        for (tk, ek) in keys {
            if let Some(v) = t.get(*tk) {
                cfg.insert(ek.to_string(), toml_text(v));
            }
        }
    }
    cfg
}

/// Mail settings: IMAP_HOST, IMAP_PORT, IMAP_USER, IMAP_PASSWORD, MAIL_ACCOUNT, MAIL_FOLDERS (comma-separated),
/// MAIL_SINCE.
pub fn mail(paths: &Paths) -> BTreeMap<String, String> {
    merged(
        paths,
        "mail",
        &[
            ("host", "IMAP_HOST"),
            ("port", "IMAP_PORT"),
            ("account", "MAIL_ACCOUNT"),
            ("since", "MAIL_SINCE"),
            ("folders", "MAIL_FOLDERS"),
        ],
    )
}

/// Calendar settings: CALDAV_URL (comma-separated), CALDAV_PAST_DAYS, CALDAV_FUTURE_DAYS, and the account.
pub fn calendar(paths: &Paths) -> BTreeMap<String, String> {
    merged(
        paths,
        "calendar",
        &[("urls", "CALDAV_URL"), ("past_days", "CALDAV_PAST_DAYS"), ("future_days", "CALDAV_FUTURE_DAYS")],
    )
}

/// The folders of a comma-separated setting.
pub fn list(v: Option<&String>, default: &str) -> Vec<String> {
    v.map(String::as_str)
        .unwrap_or(default)
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Where a collector keeps its state between runs: `~/.local/share/vault-sync/`.
pub fn state_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".local/share/vault-sync")
}

/// A non-blocking exclusive lock, so a timer run and a manual run never overlap (both would write the same notes).
/// None when another run holds it. Keep the file open for the whole run.
pub fn lock(name: &str) -> Option<fs::File> {
    let path = state_dir().join(format!("{name}.lock"));
    fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).ok()?;
    let f = fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path).ok()?;
    f.try_lock().ok().map(|_| f)
}
