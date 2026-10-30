//! Where the server reads and writes. Every place has a default and can be moved with an environment variable, so the
//! same code runs as the home server, inside the desktop app, or in a test against throwaway directories.
//!
//! The defaults sit in the OS places ([`OsDirs`]); on Linux they are the files the home server has always used:
//!   vault       LIFE_VAULT, else OBSIDIAN_VAULT, else ~/Obsidian          (Mail/, Events/, Diary/ notes)
//!   data        LIFE_DATA_DIR, else LIFE_DATA_DIR in ingest.env, else the OS data dir + /life   (raw/, todos.json, index)
//!   config      LIFE_CONF_DIR, else the OS config dir + /vault-sync       (env, sources.toml, ingest.env, diary.json)
//!               (Linux ~/.config/vault-sync, Windows %APPDATA%\vault-sync, macOS ~/Library/Application Support/vault-sync)
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The places the server uses. Built once and passed down; nothing reads the environment after that.
#[derive(Clone, Debug)]
pub struct Paths {
    pub vault: PathBuf,
    pub data: PathBuf,
    pub conf: PathBuf,
    /// which messenger senders may feed the diary (`{"allow": [...]}`)
    pub allow_file: PathBuf,
}

impl Paths {
    /// The places from the environment and `ingest.env`, with the defaults above.
    pub fn from_env() -> Paths {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let os = OsDirs::here();
        let conf = env_path("LIFE_CONF_DIR").unwrap_or_else(|| os.conf_dir());
        let vault =
            env_path("LIFE_VAULT").or_else(|| env_path("OBSIDIAN_VAULT")).unwrap_or_else(|| home.join("Obsidian"));
        let ingest = read_env_file(&conf.join("ingest.env"));
        let data = env_path("LIFE_DATA_DIR")
            .or_else(|| ingest.get("LIFE_DATA_DIR").filter(|v| !v.is_empty()).map(|v| expand_home(v, &home)))
            .unwrap_or_else(|| os.data_dir());
        let allow_file = env_path("LIFE_DIARY_CONFIG").unwrap_or_else(|| conf.join("diary.json"));
        Paths { vault, data, conf, allow_file }
    }

    /// Everything under one directory: `<root>/vault`, `<root>/data`, `<root>/conf`. For tests and trial runs.
    pub fn under(root: &Path) -> Paths {
        let conf = root.join("conf");
        Paths { vault: root.join("vault"), data: root.join("data"), allow_file: conf.join("diary.json"), conf }
    }

    pub fn raw(&self) -> PathBuf {
        self.data.join("raw")
    }

    pub fn todo_file(&self) -> PathBuf {
        self.data.join("todos.json")
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn expand_home(v: &str, home: &Path) -> PathBuf {
    match v.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if v == "~" => home.to_path_buf(),
        None => PathBuf::from(v),
    }
}

/// The OS base places the defaults hang off, so the library runs the same on every OS (in the home server or inside
/// the desktop app) without a path of one OS written into it.
#[derive(Clone, Debug)]
pub struct OsDirs {
    /// Linux `$XDG_CONFIG_HOME` (else ~/.config), Windows `%APPDATA%`, macOS `~/Library/Application Support`
    pub config: PathBuf,
    /// Linux `$XDG_DATA_HOME` (else ~/.local/share), Windows `%LOCALAPPDATA%`, macOS `~/Library/Application Support`
    pub data: PathBuf,
}

impl OsDirs {
    pub fn here() -> OsDirs {
        let base = || PathBuf::from(".");
        OsDirs { config: dirs::config_dir().unwrap_or_else(base), data: dirs::data_local_dir().unwrap_or_else(base) }
    }

    /// Settings: `env`, `sources.toml`, `ingest.env`, `diary.json`.
    pub fn conf_dir(&self) -> PathBuf {
        self.config.join("vault-sync")
    }

    /// Records, the vault of files and the index.
    pub fn data_dir(&self) -> PathBuf {
        self.data.join("life")
    }

    /// What the collectors remember between runs (seen UIDs, locks).
    pub fn state_dir(&self) -> PathBuf {
        self.data.join("vault-sync")
    }
}

/// A `KEY=VALUE` file (secrets live in these, mode 600). Blank lines and `#` comments are skipped; values may be quoted.
/// A missing or unreadable file is empty.
pub fn read_env_file(path: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(text) = fs::read_to_string(path) else {
        return out;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.insert(k.trim().to_string(), v.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_env_file_skips_comments_and_unquotes_values() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("env");
        fs::write(&p, "# secrets\n\nA=1\n B = \"two words\" \nC='x=y'\nnot a pair\n").unwrap();
        let m = read_env_file(&p);
        assert_eq!(m.get("A").map(String::as_str), Some("1"));
        assert_eq!(m.get("B").map(String::as_str), Some("two words"));
        assert_eq!(m.get("C").map(String::as_str), Some("x=y"));
        assert_eq!(m.len(), 3);
        assert!(read_env_file(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn default_places_follow_the_os_and_stay_put_on_linux() {
        let os = OsDirs { config: PathBuf::from("/os/config"), data: PathBuf::from("/os/data") };
        assert_eq!(os.conf_dir(), PathBuf::from("/os/config/vault-sync"));
        assert_eq!(os.state_dir(), PathBuf::from("/os/data/vault-sync"));
        assert_eq!(os.data_dir(), PathBuf::from("/os/data/life"));
        #[cfg(target_os = "linux")]
        if std::env::var_os("XDG_CONFIG_HOME").is_none() && std::env::var_os("XDG_DATA_HOME").is_none() {
            let home = dirs::home_dir().unwrap();
            let here = OsDirs::here();
            assert_eq!(here.conf_dir(), home.join(".config/vault-sync"), "where the Linux server has always read");
            assert_eq!(here.state_dir(), home.join(".local/share/vault-sync"));
        }
    }

    #[test]
    fn everything_can_live_under_one_directory() {
        let p = Paths::under(Path::new("/t"));
        assert_eq!(p.vault, PathBuf::from("/t/vault"));
        assert_eq!(p.raw(), PathBuf::from("/t/data/raw"));
        assert_eq!(p.allow_file, PathBuf::from("/t/conf/diary.json"));
    }
}
