//! Settings of the agent, split like the server does: non-secret settings in `config.toml`, the token in its own
//! file (`token`, mode 0600) so it is not mixed into the settings. The user puts the token in; it never goes in the repo.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const HIDDEN_TITLE: &str = "(제목 숨김)";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// e.g. https://<machine>.<tailnet>.ts.net:48765
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

impl Default for Config {
    fn default() -> Self {
        let own = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
        Config {
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
    Ok(cfg)
}

fn write_private(path: &PathBuf, text: &str) -> Result<(), String> {
    std::fs::create_dir_all(home()).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
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
            let mut cfg = Config::default();
            cfg.url = "https://host.ts.net:48765".into();
            cfg.token = "secret-token-value".into();
            cfg.watch_dirs = vec!["C:/Users/me/Downloads".into()];
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
}
