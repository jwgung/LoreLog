//! Where the server reads and writes. Every place has a default and can be moved with an environment variable, so the
//! same code runs as the home server, inside the desktop app, or in a test against throwaway directories.
//!
//! The defaults sit in the OS places ([`OsDirs`]); on Linux they are the files the home server always used:
//!   vault       LIFE_VAULT, else OBSIDIAN_VAULT, else ~/Obsidian          (Mail/, Events/, Diary/ notes)
//!   data        LIFE_DATA_DIR, else LIFE_DATA_DIR in ingest.env, else the OS data dir + /life   (raw/, todos.json, index)
//!   config      LIFE_CONF_DIR, else the OS config dir + /vault-sync       (env, sources.toml, ingest.env, diary.json)
//!               (Linux ~/.config/vault-sync, Windows %APPDATA%\vault-sync,
//!               macOS ~/Library/Application Support/vault-sync)
//! On Linux the OS places follow XDG_CONFIG_HOME and XDG_DATA_HOME when they are set.
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

/// Can this be a value in a `KEY=VALUE` file? A line break (or any control character) would start a line of its own
/// and could set another key.
pub fn env_value_ok(v: &str) -> bool {
    !v.chars().any(char::is_control)
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

/// Set (`Some`) or remove (`None`) keys in a `KEY=VALUE` file, editing only their lines: comments, order and the other
/// keys stay as written. A new key goes at the end; the file is created (0600, its folder 0700) if missing. Err, with
/// the file untouched, when it exists but cannot be read as text (so it is never rewritten from nothing) or a value
/// holds a control character.
pub fn set_env_keys(path: &Path, keys: &[(&str, Option<&str>)]) -> Result<(), String> {
    if keys.iter().any(|(_, v)| v.is_some_and(|v| !env_value_ok(v))) {
        return Err("줄바꿈 같은 제어 문자는 넣을 수 없습니다".into());
    }
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}을(를) 읽지 못해 바꾸지 않았습니다: {:?}", path.display(), e.kind())),
    };
    let key_of = |line: &str| {
        let l = line.trim();
        (!l.starts_with('#')).then(|| l.split_once('=').map(|(k, _)| k.trim().to_string())).flatten()
    };
    let mut done = std::collections::HashSet::new();
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        match key_of(line).and_then(|k| keys.iter().find(|(want, _)| *want == k)) {
            Some((k, v)) => {
                if let Some(v) = v
                    && done.insert(*k)
                {
                    lines.push(format!("{k}={v}"));
                }
                done.insert(*k); // later copies of the key go
            }
            None => lines.push(line.to_string()),
        }
    }
    for (k, v) in keys {
        if let Some(v) = v
            && !done.contains(k)
        {
            lines.push(format!("{k}={v}"));
        }
    }
    let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let fail = |e: std::io::Error| format!("저장하지 못했습니다: {:?}", e.kind());
    if let Some(dir) = path.parent() {
        crate::fsutil::ensure_private_dir(dir).map_err(fail)?;
    }
    crate::fsutil::write_private_atomic(path, body.as_bytes()).map_err(fail)
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
    fn setting_keys_edits_only_those_lines_and_an_unreadable_file_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ingest.env");
        fs::write(&p, "# the server\nINGEST_TOKEN=abc\nLIBRARIAN_ASK_LOG=off\nINGEST_PORT=9000\n").unwrap();
        set_env_keys(&p, &[("LIBRARIAN_ASK_LOG", None), ("LIFE_LLM_URL", Some("http://h/v1"))]).unwrap();
        assert_eq!(
            fs::read_to_string(&p).unwrap(),
            "# the server\nINGEST_TOKEN=abc\nINGEST_PORT=9000\nLIFE_LLM_URL=http://h/v1\n",
            "comments and order kept, the key removed, the new one added"
        );
        set_env_keys(&p, &[("INGEST_PORT", Some("9001"))]).unwrap();
        assert!(fs::read_to_string(&p).unwrap().contains("INGEST_PORT=9001\nLIFE_LLM_URL"));
        assert!(set_env_keys(&p, &[("A", Some("x\nB=y"))]).is_err(), "a line break is refused");
        let fresh = dir.path().join("new/env");
        set_env_keys(&fresh, &[("A", Some("1"))]).unwrap();
        assert_eq!(fs::read_to_string(&fresh).unwrap(), "A=1\n");
        fs::write(&p, b"\xff\xfeINGEST_TOKEN=abc\n").unwrap(); // not text: must not be rewritten from nothing
        assert!(set_env_keys(&p, &[("A", Some("1"))]).is_err());
        assert_eq!(fs::read(&p).unwrap(), b"\xff\xfeINGEST_TOKEN=abc\n");
    }

    #[test]
    fn everything_can_live_under_one_directory() {
        let p = Paths::under(Path::new("/t"));
        assert_eq!(p.vault, PathBuf::from("/t/vault"));
        assert_eq!(p.raw(), PathBuf::from("/t/data/raw"));
        assert_eq!(p.allow_file, PathBuf::from("/t/conf/diary.json"));
    }
}
