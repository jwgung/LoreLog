//! life-ingest: the collection endpoint and the assistant's HTTP API.
use std::net::SocketAddr;
use std::sync::Arc;

use life_server::config::Paths;
use life_server::host::{IngestSettings, Models};
use life_server::http::{serve, serve_tls};
use life_server::llm::{Http, HttpSpeech};

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
  POST /todos/<id>           body: {action: accept|done|dismiss|reopen}; 409 when the to-do is not in a state for it
  POST /todos/extract        extract new to-dos from the records in the background (throttled)
  GET  /healthz              only reports that it is alive (no auth, reveals nothing else)

- Raw data is only ever appended to <data dir>/raw/<source>/<YYYY-MM-DD>.ndjson; nothing is edited or deleted.
  The date comes from the record's ts (epoch milliseconds, LoreLog format) or an ISO time, else from the receive
  time (server local time).
- A record that arrives again (a resend) is stored only once.
- Records are kept as they are; only _src, _recv and _class are added. _class defaults to personal.

Settings: <config>/ingest.env  (KEY=VALUE, mode 600; <config> is LIFE_CONF_DIR, else Linux ~/.config/vault-sync)
  INGEST_TOKEN=...            required. The value clients send as the Bearer token (at least 16 characters)
  INGEST_BIND=127.0.0.1       address to listen on; for Tailscale use this machine's tailnet address
  INGEST_PORT=8765
  LIFE_DATA_DIR=...           default: the OS data location (Linux ~/.local/share/life, Windows %LOCALAPPDATA%\life,
                              macOS ~/Library/Application Support/life)
  INGEST_CLASS=personal       default class (work | personal)
  INGEST_MAX_MB=20            maximum size of one request
  INGEST_CERT / INGEST_KEY    HTTPS if both are set (PEM files)
  LIFE_LLM_URL / LIFE_LLM_MODEL, LIFE_ASR_URL / LIFE_ASR_MODEL
                              the chat and speech models (OpenAI-compatible); the environment wins over this file

Usage: life-ingest [-h]"#;

fn main() -> std::process::ExitCode {
    life_server::logging::to_stderr();
    if std::env::args().skip(1).any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return std::process::ExitCode::SUCCESS;
    }
    let paths = Paths::from_env();
    let s = match IngestSettings::load(&paths) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return std::process::ExitCode::from(2);
        }
    };
    // the model addresses from ingest.env or the environment, else this server's old defaults
    let (chat, asr) = (Http::from_env(), HttpSpeech::from_env());
    let defaults = Models { chat_url: chat.url, chat_model: chat.model, asr_url: asr.url, asr_model: asr.model };
    let app = Arc::new(Models::load(&paths).or(&defaults).app(paths.clone(), &s));
    let addr = SocketAddr::new(s.bind, s.port);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = rt.block_on(async move {
        let https = s.cert.is_some() && s.key.is_some();
        eprintln!(
            "life-ingest started: {}:{}  data: {}  {}",
            s.bind,
            s.port,
            paths.data.display(),
            if https { "HTTPS" } else { "HTTP" }
        );
        match (&s.cert, &s.key) {
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
