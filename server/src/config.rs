//! Where the server reads and writes. Every place has a default and can be moved with an environment variable, so the
//! same code runs as the home server, inside the desktop app, or in a test against throwaway directories.
//!
//! The defaults are those of the Python server this replaces, so both read and write the same files:
//!   vault       LIFE_VAULT, else OBSIDIAN_VAULT, else ~/Obsidian          (Mail/, Events/, Diary/ notes)
//!   data        LIFE_DATA_DIR, else LIFE_DATA_DIR in ingest.env, else the OS data dir + /life   (raw/, todos.json, index)
//!   config      LIFE_CONF_DIR, else ~/.config/vault-sync/                 (env, sources.toml, ingest.env, diary.json)
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
        let conf = env_path("LIFE_CONF_DIR").unwrap_or_else(|| home.join(".config/vault-sync"));
        let vault = env_path("LIFE_VAULT")
            .or_else(|| env_path("OBSIDIAN_VAULT"))
            .unwrap_or_else(|| home.join("Obsidian"));
        let ingest = read_env_file(&conf.join("ingest.env"));
        let data = env_path("LIFE_DATA_DIR")
            .or_else(|| {
                ingest
                    .get("LIFE_DATA_DIR")
                    .filter(|v| !v.is_empty())
                    .map(|v| expand_home(v, &home))
            })
            .unwrap_or_else(default_data_dir);
        let allow_file = env_path("LIFE_DIARY_CONFIG").unwrap_or_else(|| conf.join("diary.json"));
        Paths {
            vault,
            data,
            conf,
            allow_file,
        }
    }

    /// Everything under one directory: `<root>/vault`, `<root>/data`, `<root>/conf`. For tests and trial runs.
    pub fn under(root: &Path) -> Paths {
        let conf = root.join("conf");
        Paths {
            vault: root.join("vault"),
            data: root.join("data"),
            allow_file: conf.join("diary.json"),
            conf,
        }
    }

    pub fn raw(&self) -> PathBuf {
        self.data.join("raw")
    }

    pub fn todo_file(&self) -> PathBuf {
        self.data.join("todos.json")
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn expand_home(v: &str, home: &Path) -> PathBuf {
    match v.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if v == "~" => home.to_path_buf(),
        None => PathBuf::from(v),
    }
}

/// The OS place for application data: Linux `$XDG_DATA_HOME/life` (else ~/.local/share/life), Windows
/// `%LOCALAPPDATA%\life`, macOS `~/Library/Application Support/life`.
pub fn default_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("life")
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
            out.insert(
                k.trim().to_string(),
                v.trim().trim_matches('"').trim_matches('\'').to_string(),
            );
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
        fs::write(
            &p,
            "# secrets\n\nA=1\n B = \"two words\" \nC='x=y'\nnot a pair\n",
        )
        .unwrap();
        let m = read_env_file(&p);
        assert_eq!(m.get("A").map(String::as_str), Some("1"));
        assert_eq!(m.get("B").map(String::as_str), Some("two words"));
        assert_eq!(m.get("C").map(String::as_str), Some("x=y"));
        assert_eq!(m.len(), 3);
        assert!(read_env_file(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn everything_can_live_under_one_directory() {
        let p = Paths::under(Path::new("/t"));
        assert_eq!(p.vault, PathBuf::from("/t/vault"));
        assert_eq!(p.raw(), PathBuf::from("/t/data/raw"));
        assert_eq!(p.allow_file, PathBuf::from("/t/conf/diary.json"));
    }
}
