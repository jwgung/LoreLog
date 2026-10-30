//! Running the receiving server inside another program (the desktop app keeps the vault on the user's PC, see
//! docs/adr/0002). Its settings are those of `life-ingest`, in `<config>/ingest.env`; the token is made on first use.
//! The host starts it on its own tokio runtime, on the addresses it chooses (loopback for its own screen, the tailnet
//! address for phones and other PCs), and stops it when it exits.
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config::{Paths, read_env_file};
use crate::http::{App, serve};
use crate::llm::{Http, HttpSpeech};

/// What `life-ingest` reads from ingest.env.
#[derive(Clone)]
pub struct IngestSettings {
    pub token: String,
    pub bind: IpAddr,
    pub port: u16,
    pub max_bytes: usize,
    /// the class records get when the sender says nothing (`personal` or `work`)
    pub cls: String,
    pub cert: Option<String>,
    pub key: Option<String>,
}

impl std::fmt::Debug for IngestSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IngestSettings")
            .field("token", &"<hidden>")
            .field("bind", &self.bind)
            .field("port", &self.port)
            .field("max_bytes", &self.max_bytes)
            .field("cls", &self.cls)
            .finish()
    }
}

/// The shortest token the server accepts.
pub const MIN_TOKEN: usize = 16;

pub fn ingest_env(paths: &Paths) -> PathBuf {
    paths.conf.join("ingest.env")
}

impl IngestSettings {
    /// The settings from ingest.env. Err: why the server cannot start with them.
    pub fn load(paths: &Paths) -> Result<IngestSettings, String> {
        IngestSettings::from_map(&read_env_file(&ingest_env(paths)))
            .map_err(|e| format!("{}: {e}", ingest_env(paths).display()))
    }

    pub fn from_map(cfg: &BTreeMap<String, String>) -> Result<IngestSettings, String> {
        let get = |k: &str, d: &str| cfg.get(k).cloned().unwrap_or_else(|| d.to_string());
        let set = |k: &str| cfg.get(k).filter(|v| !v.is_empty()).cloned();
        let token = get("INGEST_TOKEN", "");
        if token.chars().count() < MIN_TOKEN {
            return Err(format!("needs INGEST_TOKEN (at least {MIN_TOKEN} characters)"));
        }
        let bind = get("INGEST_BIND", "127.0.0.1");
        let bind = bind.parse::<IpAddr>().map_err(|_| format!("INGEST_BIND is not an IP address: {bind}"))?;
        let port = get("INGEST_PORT", "8765").parse::<u16>().map_err(|_| "INGEST_PORT is not a port number")?;
        let max_mb: f64 = get("INGEST_MAX_MB", "20").parse().unwrap_or(20.0);
        Ok(IngestSettings {
            token,
            bind,
            port,
            max_bytes: (max_mb * 1024.0 * 1024.0) as usize,
            cls: get("INGEST_CLASS", "personal"),
            cert: set("INGEST_CERT"),
            key: set("INGEST_KEY"),
        })
    }
}

/// Where the chat and speech models are: `LIFE_LLM_URL`, `LIFE_LLM_MODEL`, `LIFE_ASR_URL`, `LIFE_ASR_MODEL`, in
/// ingest.env or the environment (the environment wins). An empty value is no setting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Models {
    pub chat_url: String,
    pub chat_model: String,
    pub asr_url: String,
    pub asr_model: String,
}

impl Models {
    pub fn load(paths: &Paths) -> Models {
        let env: BTreeMap<String, String> = std::env::vars().collect();
        Models::from_maps(&read_env_file(&ingest_env(paths)), &env)
    }

    pub fn from_maps(file: &BTreeMap<String, String>, env: &BTreeMap<String, String>) -> Models {
        let get = |k: &str| env.get(k).or_else(|| file.get(k)).filter(|v| !v.is_empty()).cloned().unwrap_or_default();
        Models {
            chat_url: get("LIFE_LLM_URL"),
            chat_model: get("LIFE_LLM_MODEL"),
            asr_url: get("LIFE_ASR_URL"),
            asr_model: get("LIFE_ASR_MODEL"),
        }
    }

    /// The same with the given defaults where nothing is set (the home server keeps its old ones).
    pub fn or(self, d: &Models) -> Models {
        let pick = |a: String, b: &str| if a.is_empty() { b.to_string() } else { a };
        Models {
            chat_url: pick(self.chat_url, &d.chat_url),
            chat_model: pick(self.chat_model, &d.chat_model),
            asr_url: pick(self.asr_url, &d.asr_url),
            asr_model: pick(self.asr_model, &d.asr_model),
        }
    }

    pub fn app(&self, paths: Paths, s: &IngestSettings) -> App {
        App::with(
            paths,
            &s.token,
            &s.cls,
            s.max_bytes,
            Arc::new(Http { url: self.chat_url.clone(), model: self.chat_model.clone() }),
            Arc::new(HttpSpeech { url: self.asr_url.clone(), model: self.asr_model.clone() }),
        )
    }
}

/// The token in ingest.env, made (random, 32 hex characters) and written when there is none or it is too short.
/// The other settings in the file are kept.
pub fn ensure_token(paths: &Paths) -> std::io::Result<String> {
    let file = ingest_env(paths);
    let mut cfg = read_env_file(&file);
    if let Some(t) = cfg.get("INGEST_TOKEN").filter(|t| t.chars().count() >= MIN_TOKEN) {
        return Ok(t.clone());
    }
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(format!("no randomness: {e}")))?;
    let token = hex::encode(bytes);
    cfg.insert("INGEST_TOKEN".into(), token.clone());
    let text: String = cfg.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    crate::fsutil::ensure_private_dir(&paths.conf)?;
    crate::fsutil::write_private_atomic(&file, text.as_bytes())?;
    Ok(token)
}

/// Is this a tailnet address? Tailscale gives IPv4 addresses in 100.64.0.0/10 and IPv6 in fd7a:115c:a1e0::/48.
pub fn is_tailnet_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            o[0] == 100 && (64..128).contains(&o[1])
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
        }
    }
}

/// The server running in this process.
pub struct Running {
    stop: tokio::sync::watch::Sender<bool>,
    listening: Vec<SocketAddr>,
    failed: Vec<(IpAddr, String)>,
    /// set when a listener stopped with an error after it started
    broken: Arc<Mutex<Vec<String>>>,
}

impl Running {
    /// The addresses it listens on (with the port the OS gave when the port was 0).
    pub fn listening(&self) -> &[SocketAddr] {
        &self.listening
    }

    /// The addresses it could not listen on, and why.
    pub fn failed(&self) -> &[(IpAddr, String)] {
        &self.failed
    }

    /// Listeners that stopped with an error after they started.
    pub fn broken(&self) -> Vec<String> {
        self.broken.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Stop every listener (requests in flight finish).
    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

/// Start serving `app` on each of `addrs` at `port` on the given runtime (call it from outside the runtime: it waits
/// for the binds). An address that cannot be bound is reported in [`Running::failed`]; the others serve. Started again
/// later with the same `app` (an address that appeared since), both serve the same state. Plain HTTP: inside the
/// tailnet the traffic is already encrypted, and HTTPS is for the user to add (`tailscale serve`).
pub fn start(rt: &tokio::runtime::Handle, app: Arc<App>, port: u16, addrs: &[IpAddr]) -> Running {
    let (stop, watch) = tokio::sync::watch::channel(false);
    let broken = Arc::new(Mutex::new(Vec::new()));
    let (mut listening, mut failed) = (Vec::new(), Vec::new());
    for &ip in addrs {
        let bound = rt.block_on(tokio::net::TcpListener::bind(SocketAddr::new(ip, port)));
        let listener = match bound.and_then(|l| l.local_addr().map(|a| (l, a))) {
            Ok((l, a)) => {
                listening.push(a);
                l
            }
            Err(e) => {
                failed.push((ip, format!("{:?}", e.kind())));
                continue;
            }
        };
        let (app, mut watch, broken) = (app.clone(), watch.clone(), broken.clone());
        rt.spawn(async move {
            let stopped = async move {
                let _ = watch.wait_for(|s| *s).await;
            };
            if let Err(e) = serve(app, listener, stopped).await {
                broken.lock().unwrap_or_else(|e| e.into_inner()).push(format!("{ip}: {:?}", e.kind()));
            }
        });
    }
    Running { stop, listening, failed, broken }
}
