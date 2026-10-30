//! Settings of the agent, split like the server does: non-secret settings in `config.toml`, the token in its own
//! file (`token`, mode 0600) so it is not mixed into the settings. The user puts the token in; it never goes in the repo.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const HIDDEN_TITLE: &str = "(제목 숨김)";

/// Is this a server address the token may be sent to? The server lives on the tailnet: https to a `*.ts.net` host,
/// or http only to loopback (`localhost`/`127.0.0.1`/`::1`) or `*.ts.net`. Nothing else — a mistyped public host must
/// not receive the Bearer token. Parsed with `url` so userinfo (`user@host`), ports and IPv6 brackets are read the
/// way the real HTTP client reads them (a hand split on `:` would let `http://127.0.0.1@evil.com` through).
pub fn is_acceptable_url(u: &str) -> bool {
    let Ok(parsed) = url::Url::parse(u) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.strip_suffix('.').unwrap_or(host); // an absolute FQDN (host.ts.net.) is the same host
    let is_tailnet = host == "ts.net" || host.ends_with(".ts.net");
    let is_loopback = matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]");
    match parsed.scheme() {
        "https" => is_tailnet,
        "http" => is_loopback || is_tailnet,
        _ => false,
    }
}

/// Where the vault lives (docs/adr/0002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// on this PC: the app runs the server itself
    Host,
    /// elsewhere: the app sends to and asks a server at `url`
    Connect,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Where the vault lives; None until the user chooses on the first run.
    pub mode: Option<Mode>,
    /// Host mode: the folder mail and event notes are written to (e.g. an Obsidian vault). None = the app's own.
    pub vault_dir: Option<String>,
    /// e.g. https://<machine>.<tailnet>.ts.net:48765 (in host mode the app's own server on loopback)
    pub url: String,
    /// The server's INGEST_TOKEN. Kept in its own file, not in config.toml — never serialized here.
    #[serde(skip)]
    pub token: String,
    /// Records go to <url>/ingest/<source>.
    pub source: String,
    pub interval_s: u64,
    /// No keyboard/mouse for this long counts as away.
    pub idle_after_s: f64,
    /// A long stay is cut into pieces of this length so nothing is lost if the process is killed.
    pub max_segment_s: f64,
    /// Shorter window visits (alt-tab flicker) are not recorded.
    pub min_segment_s: f64,
    pub flush_every_s: u64,
    /// Folders the user chose to keep deliverables from. Empty = nothing is watched (the user turns it on).
    pub watch_dirs: Vec<String>,
    /// A file in a watched folder is read only after it has held still this long (still downloading/saving otherwise).
    pub settle_s: f64,
    /// Files larger than this (megabytes) are not read or uploaded. Keep it at or below the server's INGEST_MAX_MB,
    /// so a file the agent sends is one the server will accept (an oversize file is skipped, not retried forever).
    pub max_upload_mb: u64,
    /// Windows of these apps are recorded by app name only; their titles are replaced.
    pub hide_title_apps: Vec<String>,
    /// A title containing one of these (case-insensitive) is replaced too: private browsing windows.
    pub hide_title_contains: Vec<String>,
}

impl std::fmt::Debug for Config {
    /// Everything but the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("mode", &self.mode)
            .field("url", &self.url)
            .field("token", &"<hidden>")
            .field("watch_dirs", &self.watch_dirs)
            .finish_non_exhaustive()
    }
}

impl Default for Config {
    fn default() -> Self {
        let own = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
        Config {
            mode: None,
            vault_dir: None,
            url: String::new(),
            token: String::new(),
            source: "pc".into(),
            interval_s: 5,
            idle_after_s: 300.0,
            max_segment_s: 600.0,
            min_segment_s: 3.0,
            flush_every_s: 60,
            watch_dirs: Vec::new(),
            settle_s: 10.0,
            max_upload_mb: 20,
            hide_title_apps: own(&[
                "KeePass.exe",
                "KeePassXC.exe",
                "1Password.exe",
                "Bitwarden.exe",
            ]),
            hide_title_contains: own(&["InPrivate", "시크릿", "Incognito", "Private Browsing"]),
        }
    }
}

/// Folder of config, spool and log. LIFE_AGENT_HOME overrides it (tests).
pub fn home() -> PathBuf {
    if let Some(p) = std::env::var_os("LIFE_AGENT_HOME") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("life-agent")
}

pub fn config_path() -> PathBuf {
    home().join("config.toml")
}

pub fn token_path() -> PathBuf {
    home().join("token")
}

fn read_token() -> String {
    std::fs::read_to_string(token_path())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub fn load() -> Result<Config, String> {
    let mut cfg = match std::fs::read_to_string(config_path()) {
        Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", config_path().display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => return Err(format!("{}: {e}", config_path().display())),
    };
    cfg.token = read_token();
    if cfg.mode.is_none() && !cfg.url.is_empty() {
        cfg.mode = Some(Mode::Connect); // set up before there was a choice: it was connecting to a server
    }
    Ok(cfg)
}

/// Write through a temp file that is 0600 from the moment it exists (the token is never readable by others, not even
/// briefly), then rename it over the old one.
fn write_private(path: &PathBuf, text: &str) -> Result<(), String> {
    use std::io::Write;
    std::fs::create_dir_all(home()).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp); // a leftover from an interrupted write
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(&tmp).map_err(|e| e.to_string())?;
    f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Forget the token (switching to another server, which has its own).
pub fn clear_token() -> Result<(), String> {
    match std::fs::remove_file(token_path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Write the settings (config.toml) and, if set, the token (its own 0600 file). The token is never written into
/// config.toml.
pub fn save(cfg: &Config) -> Result<(), String> {
    let text = toml::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    write_private(&config_path(), &text)?;
    if !cfg.token.is_empty() {
        write_private(&token_path(), &cfg.token)?;
    }
    Ok(())
}

/// Add a folder to the watch list, stored as its real absolute path so the intake journal keys stay stable.
/// -> the updated list. A folder that cannot be resolved is an error; an already-watched folder is a no-op.
pub fn add_watch_dir(cfg: &mut Config, dir: &str) -> Result<Vec<String>, String> {
    let abs = std::fs::canonicalize(dir)
        .map_err(|e| format!("{dir}: {e}"))?
        .to_string_lossy()
        .to_string();
    if !cfg.watch_dirs.contains(&abs) {
        cfg.watch_dirs.push(abs);
        save(cfg)?;
    }
    Ok(cfg.watch_dirs.clone())
}

/// Remove a folder from the watch list, matching either the given string or its resolved absolute path.
/// -> (updated list, whether anything was removed).
pub fn remove_watch_dir(cfg: &mut Config, dir: &str) -> Result<(Vec<String>, bool), String> {
    let abs = std::fs::canonicalize(dir)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| dir.to_string());
    let before = cfg.watch_dirs.len();
    cfg.watch_dirs.retain(|d| d != &abs && d != dir);
    let removed = cfg.watch_dirs.len() != before;
    if removed {
        save(cfg)?;
    }
    Ok((cfg.watch_dirs.clone(), removed))
}

/// Create config.toml with the defaults if it does not exist yet; return its path.
pub fn write_template() -> Result<PathBuf, String> {
    let p = config_path();
    if !p.exists() {
        save(&Config::default())?;
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each test points the config at its own temp dir via LIFE_AGENT_HOME. They run serially (env is process-wide).
    fn with_home<F: FnOnce()>(f: F) {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _g = LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("life-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        unsafe { std::env::set_var("LIFE_AGENT_HOME", &tmp) };
        f();
        unsafe { std::env::remove_var("LIFE_AGENT_HOME") };
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn settings_are_toml_and_the_token_is_a_separate_file() {
        with_home(|| {
            let cfg = Config {
                url: "https://host.ts.net:48765".into(),
                token: "secret-token-value".into(),
                watch_dirs: vec!["C:/Users/me/Downloads".into()],
                ..Config::default()
            };
            save(&cfg).unwrap();

            // config.toml holds the settings but NOT the token
            let toml_text = std::fs::read_to_string(config_path()).unwrap();
            assert!(toml_text.contains("url = \"https://host.ts.net:48765\""));
            assert!(toml_text.contains("Downloads"));
            assert!(!toml_text.contains("secret-token-value"), "the token must not be written into config.toml");

            // the token is in its own file
            assert_eq!(std::fs::read_to_string(token_path()).unwrap().trim(), "secret-token-value");

            // load() brings them back together
            let back = load().unwrap();
            assert_eq!(back.url, "https://host.ts.net:48765");
            assert_eq!(back.token, "secret-token-value");
            assert_eq!(back.watch_dirs, vec!["C:/Users/me/Downloads".to_string()]);
        });
    }

    #[test]
    fn a_missing_config_is_the_default_with_an_empty_token() {
        with_home(|| {
            let cfg = load().unwrap();
            assert!(cfg.url.is_empty() && cfg.token.is_empty());
            assert_eq!(cfg.max_upload_mb, 20);
            assert!(!cfg.hide_title_apps.is_empty(), "title hiding is on by default");
        });
    }

    #[test]
    fn the_vault_mode_is_unset_until_chosen_and_an_old_setup_is_a_connection() {
        with_home(|| {
            assert_eq!(load().unwrap().mode, None, "a first run asks");
            let mut cfg = Config { url: "https://host.ts.net:48765".into(), ..Config::default() };
            save(&cfg).unwrap();
            assert_eq!(load().unwrap().mode, Some(Mode::Connect), "set up before modes existed: it connects");
            cfg.mode = Some(Mode::Host);
            cfg.vault_dir = Some("D:/Obsidian".into());
            save(&cfg).unwrap();
            let text = std::fs::read_to_string(config_path()).unwrap();
            assert!(text.contains("mode = \"host\""), "{text}");
            let back = load().unwrap();
            assert_eq!((back.mode, back.vault_dir.as_deref()), (Some(Mode::Host), Some("D:/Obsidian")));
        });
    }

    #[test]
    fn only_tailnet_or_loopback_addresses_are_acceptable() {
        // the server lives on the tailnet; a public host must never receive the token
        assert!(is_acceptable_url("https://gungbuntu.tail8d1ab7.ts.net:48765"));
        assert!(is_acceptable_url("http://localhost:8765"));
        assert!(is_acceptable_url("http://127.0.0.1:8765"));
        assert!(is_acceptable_url("http://[::1]:8765"));
        assert!(is_acceptable_url("http://host.tail8d1ab7.ts.net:8765"));

        assert!(!is_acceptable_url("https://evil.example.com"), "a public https host is refused");
        assert!(!is_acceptable_url("http://evil.example.com"), "http off-tailnet is refused");
        assert!(!is_acceptable_url("https://ts.net.evil.com"), "a suffix lookalike is refused");
        assert!(!is_acceptable_url(""), "empty is refused");
        assert!(!is_acceptable_url("ftp://x.ts.net"), "a non-http scheme is refused");
        assert!(!is_acceptable_url("http://8.8.8.8:8765"), "a public IP is refused");

        // userinfo must not fool the host check: the real client connects to evil.com, not the loopback/tailnet
        assert!(!is_acceptable_url("http://127.0.0.1@evil.com/x"), "userinfo bypass with loopback is refused");
        assert!(!is_acceptable_url("https://legit.ts.net@evil.com/"), "userinfo bypass with tailnet is refused");
        assert!(!is_acceptable_url("http://localhost@evil.com"), "userinfo bypass with localhost is refused");

        // an absolute FQDN (trailing dot) is the same host, accepted for tailnet
        assert!(is_acceptable_url("https://host.ts.net."), "a trailing-dot tailnet FQDN is accepted");
    }
}
