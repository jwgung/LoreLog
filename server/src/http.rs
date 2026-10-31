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

use crate::assistant::{chat, diary, senders, todos};
use crate::config::Paths;
use crate::filestore::{self, FileError, FileMeta};
use crate::jobs::Jobs;
use crate::llm::{Model, Speech};
use crate::store::RawStore;

/// Largest JSON body read for the small endpoints.
pub const BODY_LIMIT: usize = 1_000_000;

/// What the librarian adds to a chat question (see [`chat::Recall`]); set by whoever builds the app.
pub type RecallFn = Arc<dyn Fn(&str, &str, &[String], Option<&str>) -> Option<String> + Send + Sync>;

/// What the server needs to answer requests.
pub struct App {
    pub paths: Paths,
    pub store: RawStore,
    pub token: String,
    /// the class records get when the sender says nothing (`personal` or `work`)
    pub cls: String,
    /// largest body for uploads (records, files, audio)
    pub max_bytes: usize,
    pub model: Arc<dyn Model>,
    pub speech: Arc<dyn Speech>,
    pub recall: RecallFn,
    pub jobs: Arc<Jobs>,
}

impl App {
    pub fn with(
        paths: Paths,
        token: &str,
        cls: &str,
        max_bytes: usize,
        model: Arc<dyn Model>,
        speech: Arc<dyn Speech>,
    ) -> App {
        let store = RawStore::new(&paths.data);
        // a chat question asks the librarian of the same places
        let (p, m) = (paths.clone(), model.clone());
        let recall: RecallFn = Arc::new(move |message, day, said, before| {
            crate::assistant::recall::find(&p, m.as_ref(), message, day, said, before)
        });
        App {
            paths,
            store,
            token: token.to_string(),
            cls: cls.to_string(),
            max_bytes,
            model,
            speech,
            recall,
            jobs: Arc::new(Jobs::default()),
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
        self.headers.get(name).map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned()).unwrap_or_default()
    }

    /// The declared body length. A missing header is 0; one that is not a number is a bad request.
    fn length(&self) -> Result<usize, Reply> {
        let v = self.header("content-length");
        let v = v.trim();
        if v.is_empty() {
            return Ok(0);
        }
        v.parse::<i64>().map(|n| n.max(0) as usize).map_err(|_| Reply::new(400, json!({"error": "bad length"})))
    }

    /// The body as JSON (at most `limit` bytes; a longer body is a bad request).
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
            return Err(Reply::new(413, json!({"error": "too large or empty", "max_bytes": max})));
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
    const DAY: &str = "([0-9]{4}-[0-9]{2}-[0-9]{2})";
    vec![
        (Method::GET, r("/healthz"), (|_, _, _| Ok(Reply::ok(json!({"ok": true})))) as Handler),
        (Method::GET, r("/todos"), |app, _, _| Ok(Reply::ok(app.jobs.todos(&app.paths)))),
        (Method::POST, r("/todos"), todos_post),
        (Method::POST, r("/todos/extract"), todos_extract),
        (Method::POST, r("/todos/([0-9a-f]{10})"), todo_act),
        (Method::GET, r("/diary/senders"), |app, _, _| {
            Ok(Reply::ok(Value::Array(senders::list_senders(&app.paths, 14))))
        }),
        (Method::GET, r("/diary/allow"), |app, _, _| Ok(Reply::ok(json!({"allow": senders::load_allow(&app.paths)})))),
        (Method::POST, r("/diary/senders"), senders_post),
        (Method::GET, r(&format!("/diary/{DAY}/chat")), |app, _, c| {
            Ok(Reply::ok(json!({"messages": chat::chat_history(&app.paths, &c[1])})))
        }),
        (Method::POST, r(&format!("/diary/{DAY}/chat")), chat_post),
        (Method::POST, r("/corrections"), correction_post),
        (Method::GET, r(&format!("/diary/{DAY}(\\?refresh=1)?")), draft_get),
        (Method::POST, r(&format!("/diary/{DAY}")), diary_post),
        (Method::POST, r("/transcribe"), transcribe),
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
    authorized_token(&app.token, &req.headers)
}

/// Does the request carry the Bearer token (compared in constant time)?
fn authorized_token(token: &str, headers: &HeaderMap) -> bool {
    let given =
        headers.get("authorization").map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned()).unwrap_or_default();
    match given.strip_prefix("Bearer ") {
        Some(t) => bool::from(t.trim().as_bytes().ct_eq(token.as_bytes())),
        None => false,
    }
}

/// A failure the client is not told about; the type goes to the journal.
fn internal(req: &Request, what: &dyn std::fmt::Debug) -> Reply {
    let path = req.target.split('?').next().unwrap_or("");
    log::warn!("{} {} {} failed: {}", Local::now().format("%H:%M:%S"), req.method, path, crate::errors::kind(what));
    Reply::new(500, json!({"error": "internal error"}))
}

// ---------- handlers ----------

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn todos_post(app: &App, req: &Request, _: &regex::Captures) -> Result<Reply, Reply> {
    let payload = req.json(100_000)?;
    let Some(o) = payload.as_object() else { return Ok(Reply::new(400, json!({"error": "bad body"}))) };
    let raw = o.get("text").map(|v| crate::assistant::util::pystr(Some(v))).unwrap_or_default();
    let text: String = raw.trim().chars().take(200).collect();
    if text.is_empty() {
        return Ok(Reply::new(400, json!({"error": "empty"})));
    }
    todos::add_user(&app.paths, &text, &today()).map_err(|e| internal(req, &e))?;
    Ok(Reply::ok(json!({"ok": true})))
}

fn todo_act(app: &App, req: &Request, caps: &regex::Captures) -> Result<Reply, Reply> {
    let payload = req.json(100_000)?;
    let action = payload.get("action").and_then(Value::as_str).unwrap_or("");
    if !matches!(action, "accept" | "done" | "dismiss" | "reopen") {
        return Ok(Reply::new(400, json!({"error": "bad action"})));
    }
    match todos::act(&app.paths, &caps[1], action).map_err(|e| internal(req, &e))? {
        todos::Acted::Changed => Ok(Reply::ok(json!({"ok": true}))),
        todos::Acted::Missing => Ok(Reply::new(404, json!({"error": "not found"}))),
        todos::Acted::NotNow => Ok(Reply::new(409, json!({"error": "not in a state for that"}))),
    }
}

fn todos_extract(app: &App, _: &Request, _: &regex::Captures) -> Result<Reply, Reply> {
    app.jobs.extract_todos(&app.paths, &app.model);
    Ok(Reply::new(202, json!({"status": "extracting"})))
}

fn senders_post(app: &App, req: &Request, _: &regex::Captures) -> Result<Reply, Reply> {
    let payload = req.json(BODY_LIMIT)?;
    let items = payload.get("allow").and_then(Value::as_array).cloned().unwrap_or_default();
    senders::save_allow(&app.paths, &items).map_err(|e| internal(req, &e))?;
    Ok(Reply::ok(json!({"saved": true})))
}

fn chat_post(app: &App, req: &Request, caps: &regex::Captures) -> Result<Reply, Reply> {
    let payload = req.json(BODY_LIMIT)?;
    let Some(message) = payload.get("message").and_then(Value::as_str).filter(|m| !m.trim().is_empty()) else {
        return Ok(Reply::new(400, json!({"error": "bad body"})));
    };
    let wrong = payload.get("wrong").and_then(Value::as_str).filter(|w| !w.is_empty());
    let fix = match (wrong, payload.get("about").and_then(Value::as_str)) {
        (None, _) => None,
        (Some(w), None | Some("summary")) => Some(chat::Fix::Summary(w)),
        (Some(w), Some("answer")) => Some(chat::Fix::Answer(w)),
        (Some(_), Some(_)) => return Ok(Reply::new(400, json!({"error": "bad about"}))),
    };
    let recall = |m: &str, d: &str, said: &[String], before: Option<&str>| (app.recall)(m, d, said, before);
    match chat::chat_answer(&app.paths, app.model.as_ref(), &recall, &caps[1], message, fix) {
        Ok(chat::Answer { reply, changed: None }) => Ok(Reply::ok(json!({"reply": reply}))),
        Ok(chat::Answer { reply, changed: Some(c) }) => Ok(Reply::ok(json!({"reply": reply, "changed": c}))),
        Err(e) => {
            Ok(Reply::new(502, json!({"error": format!("chat failed: {e}").chars().take(300).collect::<String>()})))
        }
    }
}

/// `{wrong, right, ref}` (#10): keep it, apply it where it refers to, and say what changed.
fn correction_post(app: &App, req: &Request, _: &regex::Captures) -> Result<Reply, Reply> {
    use crate::assistant::corrections::{self, Correction, Ref};
    let payload = req.json(100_000)?;
    let s = |k: &str| payload.get(k).and_then(Value::as_str).unwrap_or("");
    let Some(c) = Ref::parse(s("ref")).and_then(|r| Correction::new(s("wrong"), s("right"), r)) else {
        return Ok(Reply::new(400, json!({"error": "bad body"})));
    };
    match &c.refers {
        Ref::Todo(id) => match todos::rename(&app.paths, id, &c.right).map_err(|e| internal(req, &e))? {
            todos::Acted::Changed => {}
            todos::Acted::Missing => return Ok(Reply::new(404, json!({"error": "not found"}))),
            todos::Acted::NotNow => return Ok(Reply::new(409, json!({"error": "not in a state for that"}))),
        },
        Ref::Summary(day) => {
            let fix = json!({"append": true, "corrections": [{"wrong": c.wrong, "right": c.right}]});
            diary::save_user(&app.paths, day, &fix).map_err(|e| internal(req, &e))?;
        }
        Ref::Chat(_) => {}
    }
    corrections::keep(&app.paths, &c).map_err(|e| internal(req, &e))?;
    Ok(Reply::ok(json!({"saved": true, "changed": c.changed()})))
}

fn draft_get(app: &App, _: &Request, caps: &regex::Captures) -> Result<Reply, Reply> {
    let (status, body) = app.jobs.draft(&app.paths, &app.model, &caps[1], caps.get(2).is_some());
    Ok(Reply::new(status, body))
}

fn diary_post(app: &App, req: &Request, caps: &regex::Captures) -> Result<Reply, Reply> {
    let payload = req.json(BODY_LIMIT)?;
    diary::save_user(&app.paths, &caps[1], &payload).map_err(|e| internal(req, &e))?;
    Ok(Reply::ok(json!({"saved": true})))
}

fn transcribe(app: &App, req: &Request, _: &regex::Captures) -> Result<Reply, Reply> {
    let audio = req.blob(app.max_bytes)?;
    match app.speech.transcribe(audio) {
        Ok(text) => Ok(Reply::ok(json!({"text": text}))),
        Err(e) => Ok(Reply::new(502, json!({"error": format!("asr failed: {}", crate::errors::kind(&e))}))),
    }
}

fn ingest(app: &App, req: &Request, caps: &regex::Captures) -> Result<Reply, Reply> {
    let source = &caps[1];
    let body = String::from_utf8_lossy(req.blob(app.max_bytes)?).into_owned();
    let c = app.store.store(source, &body, &app.cls).map_err(|e| internal(req, &e))?;
    log::info!(
        "{} {source} stored {} duplicates {} rejected {}",
        Local::now().format("%H:%M:%S"),
        c.accepted,
        c.duplicates,
        c.rejected
    );
    Ok(Reply::ok(json!({"accepted": c.accepted, "duplicates": c.duplicates, "rejected": c.rejected})))
}

fn file_has(app: &App, _req: &Request, caps: &regex::Captures) -> Result<Reply, Reply> {
    let sha = &caps[1];
    Ok(Reply::ok(json!({"sha256": sha, "present": filestore::has_blob(&app.store, sha)})))
}

fn file_put(app: &App, req: &Request, _caps: &regex::Captures) -> Result<Reply, Reply> {
    let sha = req.header("x-file-sha256").trim().to_lowercase();
    let decoded = percent_encoding::percent_decode_str(&req.header("x-file-name")).decode_utf8_lossy().into_owned();
    let name: String = decoded.chars().take(400).collect();
    let name = if name.is_empty() { "(unnamed)".to_string() } else { name };
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
    let meta = FileMeta { name: &name, mime: &mime, when_ms, cls: &app.cls };
    if req.header("x-file-sighting").trim() == "1" {
        return match filestore::record_sighting(&app.store, &sha, &meta) {
            Ok(()) => Ok(Reply::ok(json!({"sha256": sha, "stored": false, "duplicate": true}))),
            Err(FileError::NotKept) => Ok(Reply::new(409, json!({"error": "blob not present"}))),
            Err(e) => Err(internal(req, &e)),
        };
    }
    let data = req.blob(app.max_bytes)?;
    match filestore::keep_file(&app.store, data, &sha, &meta) {
        Ok((sha, stored)) => {
            let safe: String = name.escape_default().collect(); // no raw newlines etc. in the journal line
            let what = if stored { "stored" } else { "dup" };
            log::info!("{} file {} {what} {safe}", Local::now().format("%H:%M:%S"), &sha[..12]);
            Ok(Reply::ok(json!({"sha256": sha, "stored": stored, "duplicate": !stored})))
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
    let target =
        parts.uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| parts.uri.path().to_string());
    let declared = parts
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    // read only what some handler may use; a larger declared body is refused by the handler without being read
    let cap = app.max_bytes.max(BODY_LIMIT);
    // an unauthenticated request gets nothing read: it is answered 401 (or 404) without its body
    let open = target == "/healthz" || authorized_token(&app.token, &parts.headers);
    let body = if open && declared > 0 && declared <= cap { read(body, declared).await } else { Vec::new() };
    let request = Request { method: parts.method, target, headers: parts.headers, body };
    let app2 = app.clone();
    let reply = tokio::task::spawn_blocking(move || handle(&app2, &request))
        .await
        .unwrap_or_else(|_| Reply::new(500, json!({"error": "internal error"})));
    respond(reply)
}

async fn read(body: Body, limit: usize) -> Vec<u8> {
    to_bytes(body, limit).await.map(|b| b.to_vec()).unwrap_or_default()
}

fn respond(reply: Reply) -> AxumResponse {
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = serde_json::to_vec(&reply.body).unwrap_or_default();
    (status, [(header::CONTENT_TYPE, "application/json; charset=utf-8")], body).into_response()
}

/// Serve until the future `stop` completes.
pub async fn serve(
    app: Arc<App>,
    listener: tokio::net::TcpListener,
    stop: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router(app).into_make_service_with_connect_info::<SocketAddr>())
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
    axum_server::bind_rustls(addr, tls).serve(router(app).into_make_service_with_connect_info::<SocketAddr>()).await
}
