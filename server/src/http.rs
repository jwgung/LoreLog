//! HTTP surface of the server: a route table, Bearer auth and the server itself. The work lives in other modules.
//!
//! Routes match the whole request target (path and query) like the Python server did, so `/diary/<day>?refresh=1`
//! is one route and anything else after a path is not found. Every route but `/healthz` needs the Bearer token.
//! Handlers are plain functions over a [`Request`]; axum only carries the bytes, so the same routes run in a test, in
//! the server binary and inside the desktop app.
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response as AxumResponse};
use chrono::Local;
use regex::Regex;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

use crate::config::Paths;
use crate::filestore::{self, FileError, FileMeta};
use crate::store::RawStore;

/// Largest JSON body read for the small endpoints.
pub const BODY_LIMIT: usize = 1_000_000;

/// What the server needs to answer requests.
pub struct App {
    pub paths: Paths,
    pub store: RawStore,
    pub token: String,
    /// the class records get when the sender says nothing (`personal` or `work`)
    pub cls: String,
    /// largest body for uploads (records, files, audio)
    pub max_bytes: usize,
}

impl App {
    pub fn new(paths: Paths, token: &str, cls: &str, max_bytes: usize) -> App {
        let store = RawStore::new(&paths.data);
        App {
            paths,
            store,
            token: token.to_string(),
            cls: cls.to_string(),
            max_bytes,
        }
    }
}

/// One request as the handlers see it.
pub struct Request {
    pub method: Method,
    /// path and query, as sent
    pub target: String,
    pub headers: HeaderMap,
    /// the body, or empty when the declared length was larger than anything a handler reads
    pub body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> String {
        self.headers
            .get(name)
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .unwrap_or_default()
    }

    /// The declared body length. A missing header is 0; one that is not a number is a bad request.
    fn length(&self) -> Result<usize, Reply> {
        let v = self.header("content-length");
        let v = v.trim();
        if v.is_empty() {
            return Ok(0);
        }
        v.parse::<i64>()
            .map(|n| n.max(0) as usize)
            .map_err(|_| Reply::new(400, json!({"error": "bad length"})))
    }

    /// The body as JSON (at most `limit` bytes; a longer body is a bad request).
    #[allow(dead_code)] // the diary, chat and to-do routes use it (next ticket)
    fn json(&self, limit: usize) -> Result<Value, Reply> {
        let n = self.length()?;
        let bad = || Reply::new(400, json!({"error": "bad body"}));
        if n > limit || n > self.body.len() {
            return Err(bad());
        }
        let text = std::str::from_utf8(&self.body[..n]).map_err(|_| bad())?;
        serde_json::from_str(text).map_err(|_| bad())
    }

    /// The body as bytes for uploads: refused when empty or larger than `max`.
    fn blob(&self, max: usize) -> Result<&[u8], Reply> {
        let n = self.length()?;
        if n == 0 || n > max || n > self.body.len() {
            return Err(Reply::new(
                413,
                json!({"error": "too large or empty", "max_bytes": max}),
            ));
        }
        Ok(&self.body[..n])
    }
}

/// A status and a JSON body.
#[derive(Debug, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub body: Value,
}

impl Reply {
    pub fn new(status: u16, body: Value) -> Reply {
        Reply { status, body }
    }

    fn ok(body: Value) -> Reply {
        Reply::new(200, body)
    }
}

type Handler = fn(&App, &Request, &regex::Captures) -> Result<Reply, Reply>;

/// (method, target pattern, handler). Order matters where patterns overlap: `/diary/senders` before `/diary/<day>`.
static ROUTES: LazyLock<Vec<(Method, Regex, Handler)>> = LazyLock::new(|| {
    let r = |p: &str| Regex::new(&format!("^(?:{p})$")).expect("route pattern");
    vec![
        (
            Method::GET,
            r("/healthz"),
            (|_, _, _| Ok(Reply::ok(json!({"ok": true})))) as Handler,
        ),
        (Method::GET, r("/ingest/file/([0-9a-f]{64})"), file_has),
        (Method::POST, r("/ingest/file"), file_put),
        (Method::POST, r("/ingest/([a-z0-9_-]{1,32})/?"), ingest),
    ]
});

/// Answer one request.
pub fn handle(app: &App, req: &Request) -> Reply {
    for (method, pattern, handler) in ROUTES.iter() {
        if *method != req.method {
            continue;
        }
        let Some(caps) = pattern.captures(&req.target) else {
            continue;
        };
        if req.target != "/healthz" && !authorized(app, req) {
            return Reply::new(401, json!({"error": "unauthorized"}));
        }
        return match handler(app, req, &caps) {
            Ok(r) | Err(r) => r,
        };
    }
    Reply::new(404, json!({"error": "not found"}))
}

fn authorized(app: &App, req: &Request) -> bool {
    let given = req.header("authorization");
    match given.strip_prefix("Bearer ") {
        Some(t) => bool::from(t.trim().as_bytes().ct_eq(app.token.as_bytes())),
        None => false,
    }
}

/// A failure the client is not told about; the type goes to the journal.
fn internal(req: &Request, what: &dyn std::fmt::Debug) -> Reply {
    let path = req.target.split('?').next().unwrap_or("");
    eprintln!(
        "{} {} {} failed: {}",
        Local::now().format("%H:%M:%S"),
        req.method,
        path,
        kind_of(what)
    );
    Reply::new(500, json!({"error": "internal error"}))
}

fn kind_of(what: &dyn std::fmt::Debug) -> String {
    let s = format!("{what:?}");
    s.split(['(', ' ', '{'])
        .next()
        .unwrap_or("error")
        .to_string()
}

// ---------- handlers ----------

fn ingest(app: &App, req: &Request, caps: &regex::Captures) -> Result<Reply, Reply> {
    let source = &caps[1];
    let body = String::from_utf8_lossy(req.blob(app.max_bytes)?).into_owned();
    let c = app
        .store
        .store(source, &body, &app.cls)
        .map_err(|e| internal(req, &e))?;
    eprintln!(
        "{} {source} stored {} duplicates {} rejected {}",
        Local::now().format("%H:%M:%S"),
        c.accepted,
        c.duplicates,
        c.rejected
    );
    Ok(Reply::ok(
        json!({"accepted": c.accepted, "duplicates": c.duplicates, "rejected": c.rejected}),
    ))
}

fn file_has(app: &App, _req: &Request, caps: &regex::Captures) -> Result<Reply, Reply> {
    let sha = &caps[1];
    Ok(Reply::ok(
        json!({"sha256": sha, "present": filestore::has_blob(&app.store, sha)}),
    ))
}

fn file_put(app: &App, req: &Request, _caps: &regex::Captures) -> Result<Reply, Reply> {
    let sha = req.header("x-file-sha256").trim().to_lowercase();
    let decoded = percent_encoding::percent_decode_str(&req.header("x-file-name"))
        .decode_utf8_lossy()
        .into_owned();
    let name: String = decoded.chars().take(400).collect();
    let name = if name.is_empty() {
        "(unnamed)".to_string()
    } else {
        name
    };
    let mime_header = req.header("x-file-mime");
    let mime: String = if mime_header.is_empty() {
        "application/octet-stream".into()
    } else {
        mime_header.chars().take(200).collect()
    };
    let ts = req.header("x-file-ts");
    let when_ms = match ts.parse::<i64>() {
        Ok(v) if !ts.is_empty() && ts.bytes().all(|b| b.is_ascii_digit()) => v,
        _ => Local::now().timestamp_millis(),
    };
    let meta = FileMeta {
        name: &name,
        mime: &mime,
        when_ms,
        cls: &app.cls,
    };
    if req.header("x-file-sighting").trim() == "1" {
        return match filestore::record_sighting(&app.store, &sha, &meta) {
            Ok(()) => Ok(Reply::ok(
                json!({"sha256": sha, "stored": false, "duplicate": true}),
            )),
            Err(FileError::NotKept) => Ok(Reply::new(409, json!({"error": "blob not present"}))),
            Err(e) => Err(internal(req, &e)),
        };
    }
    let data = req.blob(app.max_bytes)?;
    match filestore::keep_file(&app.store, data, &sha, &meta) {
        Ok((sha, stored)) => {
            let safe: String = name.escape_default().collect(); // no raw newlines etc. in the journal line
            let what = if stored { "stored" } else { "dup" };
            eprintln!(
                "{} file {} {what} {safe}",
                Local::now().format("%H:%M:%S"),
                &sha[..12]
            );
            Ok(Reply::ok(
                json!({"sha256": sha, "stored": stored, "duplicate": !stored}),
            ))
        }
        Err(FileError::HashMismatch) => Ok(Reply::new(400, json!({"error": "hash mismatch"}))),
        Err(e) => Err(internal(req, &e)),
    }
}

// ---------- serving ----------

/// The axum router: every request goes through [`handle`] on a blocking thread (handlers do file and network work).
pub fn router(app: Arc<App>) -> Router {
    Router::new().fallback(entry).with_state(app)
}

async fn entry(State(app): State<Arc<App>>, req: axum::extract::Request) -> AxumResponse {
    let (parts, body) = req.into_parts();
    let target = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| parts.uri.path().to_string());
    let declared = parts
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    // read only what some handler may use; a larger declared body is refused by the handler without being read
    let cap = app.max_bytes.max(BODY_LIMIT);
    let body = if declared > 0 && declared <= cap {
        read(body, declared).await
    } else {
        Vec::new()
    };
    let request = Request {
        method: parts.method,
        target,
        headers: parts.headers,
        body,
    };
    let app2 = app.clone();
    let reply = tokio::task::spawn_blocking(move || handle(&app2, &request))
        .await
        .unwrap_or_else(|_| Reply::new(500, json!({"error": "internal error"})));
    respond(reply)
}

async fn read(body: Body, limit: usize) -> Vec<u8> {
    to_bytes(body, limit)
        .await
        .map(|b| b.to_vec())
        .unwrap_or_default()
}

fn respond(reply: Reply) -> AxumResponse {
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = serde_json::to_vec(&reply.body).unwrap_or_default();
    (
        status,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        body,
    )
        .into_response()
}

/// Serve until the future `stop` completes.
pub async fn serve(
    app: Arc<App>,
    listener: tokio::net::TcpListener,
    stop: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(
        listener,
        router(app).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(stop)
    .await
}

/// Serve HTTPS with a certificate and key in PEM files, until the process ends.
pub async fn serve_tls(
    app: Arc<App>,
    addr: SocketAddr,
    cert: &std::path::Path,
    key: &std::path::Path,
) -> std::io::Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
    axum_server::bind_rustls(addr, tls)
        .serve(router(app).into_make_service_with_connect_info::<SocketAddr>())
        .await
}
