//! life-ingest: the collection endpoint and the assistant's HTTP API.
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use life_server::config::{Paths, read_env_file};
use life_server::http::{App, serve, serve_tls};

const HELP: &str = r#"life-ingest: collection endpoint that any client (phone, PC, ...) can send NDJSON to. Raw data is stored on this server.

  POST /ingest/<source>      body: NDJSON (one JSON object per line), header: Authorization: Bearer <token>
  POST /ingest/file          body: file bytes; headers X-File-Sha256, X-File-Name (percent-encoded), X-File-Mime, X-File-Ts (ms).
                             Keeps the bytes under their content hash (shared with mail-files) and records a sighting in raw/pcfile.
                             With X-File-Sighting: 1 and no body: records that an already kept file was seen again.
  GET  /ingest/file/<sha256> {present: bool}: whether that blob is already kept (the agent asks before uploading)
  GET  /diary/<date>         the day's draft (summary + question); made on demand -> 202 while it is being made
                             (?refresh=1 makes it again)
  POST /diary/<date>         body: JSON {corrections:[{wrong,right}], reflection}; stored in the day's note
  GET  /diary/senders        notification senders of the last 14 days with the user's allowed flag
  GET  /diary/allow          the allowed list as stored
  POST /diary/senders        body: {allow: ["<app>: <sender>" | "<app>: *"]}; which messengers may feed the diary
  GET  /diary/<date>/chat    the day's chat exchange (to show it again)
  POST /diary/<date>/chat    body: {message, wrong?}; stores it in the diary and returns the assistant's reply
  POST /transcribe           body: audio (wav); returns {text} from the speech model (LIFE_ASR_URL); nothing is stored
  GET  /todos                proposed (found by the model), open and recently done to-dos (+ extracting flag)
  POST /todos                body: {text}; add the user's own to-do
  POST /todos/<id>           body: {action: accept|done|dismiss|reopen}
  POST /todos/extract        extract new to-dos from the records in the background (throttled)
  GET  /healthz              only reports that it is alive (no auth, reveals nothing else)

- Raw data is only ever appended to <data dir>/raw/<source>/<YYYY-MM-DD>.ndjson; nothing is edited or deleted.
  The date comes from the record's ts (epoch milliseconds, LoreLog format) or an ISO time, else from the receive
  time (server local time).
- A record that arrives again (a resend) is stored only once.
- Records are kept as they are; only _src, _recv and _class are added. _class defaults to personal.

Settings: ~/.config/vault-sync/ingest.env  (KEY=VALUE, mode 600)
  INGEST_TOKEN=...            required. The value clients send as the Bearer token (at least 16 characters)
  INGEST_BIND=127.0.0.1       address to listen on; for Tailscale use this machine's tailnet address
  INGEST_PORT=8765
  LIFE_DATA_DIR=...           default: the OS data location (Linux ~/.local/share/life, Windows %LOCALAPPDATA%\life,
                              macOS ~/Library/Application Support/life)
  INGEST_CLASS=personal       default class (work | personal)
  INGEST_MAX_MB=20            maximum size of one request
  INGEST_CERT / INGEST_KEY    HTTPS if both are set (PEM files)

Usage: life-ingest [-h]"#;

fn main() -> std::process::ExitCode {
    if std::env::args().skip(1).any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return std::process::ExitCode::SUCCESS;
    }
    let paths = Paths::from_env();
    let env_file = paths.conf.join("ingest.env");
    let cfg = read_env_file(&env_file);
    let get = |k: &str, d: &str| cfg.get(k).cloned().unwrap_or_else(|| d.to_string());
    let token = get("INGEST_TOKEN", "");
    if token.chars().count() < 16 {
        eprintln!("{} needs INGEST_TOKEN (at least 16 characters).", env_file.display());
        return std::process::ExitCode::from(2);
    }
    let bind = get("INGEST_BIND", "127.0.0.1");
    let Ok(ip) = bind.parse::<IpAddr>() else {
        eprintln!("INGEST_BIND is not an IP address: {bind}");
        return std::process::ExitCode::from(2);
    };
    let Ok(port) = get("INGEST_PORT", "8765").parse::<u16>() else {
        eprintln!("INGEST_PORT is not a port number");
        return std::process::ExitCode::from(2);
    };
    let max_mb: f64 = get("INGEST_MAX_MB", "20").parse().unwrap_or(20.0);
    let max_bytes = (max_mb * 1024.0 * 1024.0) as usize;
    let cert = cfg.get("INGEST_CERT").filter(|v| !v.is_empty()).cloned();
    let key = cfg.get("INGEST_KEY").filter(|v| !v.is_empty()).cloned();
    let app = Arc::new(App::new(paths.clone(), &token, &get("INGEST_CLASS", "personal"), max_bytes));
    let addr = SocketAddr::new(ip, port);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = rt.block_on(async move {
        let https = cert.is_some() && key.is_some();
        eprintln!(
            "life-ingest started: {bind}:{port}  data: {}  {}",
            paths.data.display(),
            if https { "HTTPS" } else { "HTTP" }
        );
        match (cert, key) {
            (Some(c), Some(k)) => serve_tls(app, addr, c.as_ref(), k.as_ref()).await,
            _ => {
                let listener = tokio::net::TcpListener::bind(addr).await?;
                serve(app, listener, async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await
            }
        }
    });
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("life-ingest: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
