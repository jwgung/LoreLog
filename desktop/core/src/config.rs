//! Settings file of the agent. The token lives here (the user puts it in), never in the repo.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const HIDDEN_TITLE: &str = "(제목 숨김)";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// e.g. https://<machine>.<tailnet>.ts.net:48765
    pub url: String,
    /// The server's INGEST_TOKEN.
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
    home().join("config.json")
}

pub fn load() -> Result<Config, String> {
    match std::fs::read_to_string(config_path()) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", config_path().display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(format!("{}: {e}", config_path().display())),
    }
}

/// Create config.json with the defaults if it does not exist yet; return its path.
pub fn write_template() -> Result<PathBuf, String> {
    let p = config_path();
    if !p.exists() {
        std::fs::create_dir_all(home()).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(&Config::default()).map_err(|e| e.to_string())?;
        std::fs::write(&p, text).map_err(|e| e.to_string())?;
    }
    Ok(p)
}
