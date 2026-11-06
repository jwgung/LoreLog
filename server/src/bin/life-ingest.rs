//! life-ingest: the collection endpoint and the assistant's HTTP API.
use std::net::SocketAddr;
use std::sync::Arc;

use life_server::config::Paths;
use life_server::host::{IngestSettings, Models};
use life_server::http::{serve, serve_tls};

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
  GET  /diary/<date>/chat    the day's chat exchange (to show it again); the last turn carries {choices} while it
                             asked back and is not answered yet
  POST /diary/<date>/chat    body: {message, wrong?, about?: summary|answer, choice?: 1-3|"no"}; stores it in the
                             diary and returns the assistant's {reply}, with wrong {changed}: what the correction
                             changed, in one line, when the librarian found records {sources: [{day, kind, title,
                             ref}]}: the ones quoted, and when it asks back {choices: {kind: matters|people|now,
                             options: [label]}}. `choice` answers the candidates of the turn before by position, or
                             "no" for none of them (a plain typed "no" is the same); never with `wrong` (400)
  POST /corrections          body: {wrong, right, ref: summary:<date>|chat:<date>|todo:<id>}; keeps the pair in
                             raw/corrections, applies it (the day's note, the chat, the to-do's name), returns {changed}
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
  INGEST_BIND=127.0.0.1       address(es) to listen on, comma-separated, each one bound
                              (e.g. 100.101.171.95,192.168.0.20). For Tailscale use this machine's tailnet address;
                              for phones and PCs in the same Wi-Fi add its private address (10.x, 172.16-31.x,
                              192.168.x): plain HTTP there is not encrypted, so only on a network you trust
  INGEST_PORT=8765
  LIFE_DATA_DIR=...           default: the OS data location (Linux ~/.local/share/life, Windows %LOCALAPPDATA%\life,
                              macOS ~/Library/Application Support/life)
  INGEST_CLASS=personal       default class (work | personal)
  INGEST_MAX_MB=20            maximum size of one request
  INGEST_CERT / INGEST_KEY    HTTPS if both are set (PEM files)
  LIFE_LLM_URL / LIFE_LLM_MODEL, LIFE_ASR_URL / LIFE_ASR_MODEL
                              the chat and speech models (OpenAI-compatible, e.g. Ollama http://127.0.0.1:11434/v1);
                              the environment wins over this file. None is built in: unset, the chat and voice answer
                              that no address is set and the graph waits

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
    // the model addresses from ingest.env or the environment (none built in)
    let app = Arc::new(Models::load(&paths).app(paths.clone(), &s));
    let addrs: Vec<SocketAddr> = s.bind.iter().map(|ip| SocketAddr::new(*ip, s.port)).collect();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = rt.block_on(async move {
        let https = s.cert.is_some() && s.key.is_some();
        let shown: Vec<String> = addrs.iter().map(|a| a.to_string()).collect();
        eprintln!(
            "life-ingest started: {}  data: {}  {}",
            shown.join(", "),
            paths.data.display(),
            if https { "HTTPS" } else { "HTTP" }
        );
        let mut servers = tokio::task::JoinSet::new();
        match (&s.cert, &s.key) {
            (Some(c), Some(k)) => {
                // as for HTTP below: an address that cannot be bound stops the start before any serves
                for addr in &addrs {
                    drop(
                        std::net::TcpListener::bind(addr)
                            .map_err(|e| std::io::Error::new(e.kind(), format!("{addr}: {e}")))?,
                    );
                }
                for addr in addrs {
                    let (app, c, k) = (app.clone(), c.clone(), k.clone());
                    servers.spawn(async move { serve_tls(app, addr, c.as_ref(), k.as_ref()).await });
                }
            }
            _ => {
                // every address is bound before any serves: one that cannot be bound stops the start
                let mut listeners = Vec::new();
                for addr in &addrs {
                    let l = tokio::net::TcpListener::bind(addr)
                        .await
                        .map_err(|e| std::io::Error::new(e.kind(), format!("{addr}: {e}")))?;
                    listeners.push(l);
                }
                let (stop, watch) = tokio::sync::watch::channel(false);
                tokio::spawn(async move {
                    let _ = tokio::signal::ctrl_c().await;
                    let _ = stop.send(true);
                });
                for listener in listeners {
                    let (app, mut watch) = (app.clone(), watch.clone());
                    servers.spawn(async move {
                        serve(app, listener, async move {
                            let _ = watch.wait_for(|s| *s).await;
                        })
                        .await
                    });
                }
            }
        }
        // one listener failing ends the server (the service manager restarts it), as with a single address
        while let Some(done) = servers.join_next().await {
            done.map_err(std::io::Error::other)??;
        }
        Ok::<(), std::io::Error>(())
    });
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("life-ingest: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
