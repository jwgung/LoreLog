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

/// The value after `name` on a command line (`--limit 5` -> "5").
pub fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// What the mail collectors need to reach the mailbox.
#[derive(Debug, PartialEq)]
pub struct Imap {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    /// the name notes and files are filed under
    pub account: String,
    pub folders: Vec<String>,
    /// only mail from this day (YYYY-MM-DD) on
    pub since: Option<String>,
}

/// The mail settings, `folder` (from `--folder`) replacing the configured folders. Err: the required settings that are
/// missing or empty.
pub fn imap(paths: &Paths, folder: Option<String>) -> Result<Imap, Vec<&'static str>> {
    let cfg = mail(paths);
    let get = |k: &str| cfg.get(k).filter(|v| !v.is_empty()).cloned();
    let need: Vec<&'static str> =
        ["IMAP_HOST", "IMAP_USER", "IMAP_PASSWORD"].into_iter().filter(|k| get(k).is_none()).collect();
    if !need.is_empty() {
        return Err(need);
    }
    Ok(Imap {
        host: get("IMAP_HOST").unwrap_or_default(),
        port: cfg.get("IMAP_PORT").and_then(|p| p.parse().ok()).unwrap_or(993),
        user: get("IMAP_USER").unwrap_or_default(),
        password: get("IMAP_PASSWORD").unwrap_or_default(),
        account: cfg.get("MAIL_ACCOUNT").cloned().unwrap_or_else(|| "account".into()),
        folders: match folder {
            Some(f) => vec![f],
            None => list(cfg.get("MAIL_FOLDERS"), "INBOX,Sent Messages"),
        },
        since: get("MAIL_SINCE"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conf(env: &str, sources: &str) -> (PathBuf, Paths) {
        let root = std::env::temp_dir().join(format!("settings-{}-{}", std::process::id(), env.len() + sources.len()));
        let paths = Paths::under(&root);
        fs::create_dir_all(&paths.conf).unwrap();
        fs::write(env_file(&paths), env).unwrap();
        fs::write(sources_file(&paths), sources).unwrap();
        (root, paths)
    }

    #[test]
    fn imap_settings_merge_env_and_sources_with_defaults() {
        let (root, paths) = conf(
            "IMAP_USER=me\nIMAP_PASSWORD=pw\nIMAP_HOST=old.example\n",
            "[mail]\nhost = \"imap.example\"\nfolders = [\"INBOX\", \"Sent\"]\nsince = \"2026-01-01\"\n",
        );
        let got = imap(&paths, None).unwrap();
        assert_eq!(
            got,
            Imap {
                host: "imap.example".into(),
                port: 993,
                user: "me".into(),
                password: "pw".into(),
                account: "account".into(),
                folders: vec!["INBOX".into(), "Sent".into()],
                since: Some("2026-01-01".into()),
            }
        );
        assert_eq!(imap(&paths, Some("Archive".into())).unwrap().folders, vec!["Archive".to_string()]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_or_empty_imap_settings_are_named() {
        let (root, paths) = conf("IMAP_USER=me\nIMAP_PASSWORD=\n", "");
        assert_eq!(imap(&paths, None), Err(vec!["IMAP_HOST", "IMAP_PASSWORD"]));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn arg_takes_the_value_after_the_flag() {
        let args: Vec<String> = ["--dry-run", "--limit", "5", "--folder"].map(String::from).to_vec();
        assert_eq!(arg(&args, "--limit"), Some("5".into()));
        assert_eq!(arg(&args, "--folder"), None);
        assert_eq!(arg(&args, "--since"), None);
    }
}
