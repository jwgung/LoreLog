//! A real server on a free local port over throwaway directories, and a client that does not treat 4xx as errors.
#![allow(dead_code)]
use std::path::PathBuf;
use std::sync::Arc;

use life_server::config::Paths;
use life_server::http::{App, serve};
use life_server::llm::{Ask, Model, Speech};
use serde_json::Value;

type Answer = Box<dyn Fn(&[Value]) -> anyhow::Result<String> + Send + Sync>;

/// A model that answers from a function of the messages, and remembers what it was asked.
pub struct FakeModel {
    pub answer: Answer,
    pub asked: std::sync::Mutex<Vec<Vec<Value>>>,
}

impl FakeModel {
    pub fn new(
        answer: impl Fn(&[Value]) -> anyhow::Result<String> + Send + Sync + 'static,
    ) -> Arc<FakeModel> {
        Arc::new(FakeModel {
            answer: Box::new(answer),
            asked: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// A model for the server tests: a fixed summary for the summary prompt, "네, 확인했어요." for a chat.
    pub fn plain() -> Arc<FakeModel> {
        FakeModel::new(|msgs| {
            let system = msgs[0]["content"].as_str().unwrap_or("");
            if system.starts_with("너는 사용자의 개인 비서다. 오늘 수집된") {
                Ok(r#"{"summary": "오전 9시 회의가 있었어요.", "question": "어떠셨어요?"}"#.into())
            } else {
                Ok("네, 확인했어요.".into())
            }
        })
    }

    pub fn last(&self) -> Vec<Value> {
        self.asked
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }
}

impl Model for FakeModel {
    fn complete(&self, messages: &[Value], _ask: Ask) -> anyhow::Result<String> {
        self.asked.lock().unwrap().push(messages.to_vec());
        (self.answer)(messages)
    }
}

pub struct FakeSpeech;

impl Speech for FakeSpeech {
    fn transcribe(&self, audio: &[u8]) -> anyhow::Result<String> {
        if audio == b"broken" {
            anyhow::bail!("down")
        } else {
            Ok(format!("{}바이트 들음", audio.len()))
        }
    }
}

pub const TOKEN: &str = "test-token-0123456789";

pub struct Server {
    pub base: String,
    pub root: PathBuf,
    pub paths: Paths,
    pub app: Arc<App>,
    _dir: tempfile::TempDir,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    pub fn start() -> Server {
        Server::start_with(1024 * 1024)
    }

    pub fn start_with(max_bytes: usize) -> Server {
        Server::start_full(max_bytes, FakeModel::plain())
    }

    pub fn start_full(max_bytes: usize, model: Arc<FakeModel>) -> Server {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let app = Arc::new(App::with(
            paths.clone(),
            TOKEN,
            "personal",
            max_bytes,
            model,
            Arc::new(FakeSpeech),
        ));
        Server::run(dir, paths, app)
    }

    pub fn run(dir: tempfile::TempDir, paths: Paths, app: Arc<App>) -> Server {
        let kept = app.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let (port_tx, port_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                port_tx.send(listener.local_addr().unwrap().port()).unwrap();
                serve(app, listener, async move {
                    let _ = rx.await;
                })
                .await
                .unwrap();
            });
        });
        let port = port_rx.recv().unwrap();
        Server {
            base: format!("http://127.0.0.1:{port}"),
            root: paths.data.clone(),
            paths,
            app: kept,
            _dir: dir,
            stop: Some(tx),
            thread: Some(thread),
        }
    }

    pub fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
        token: Option<&str>,
    ) -> (u16, Value) {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let mut req = ureq::http::Request::builder()
            .method(method)
            .uri(format!("{}{}", self.base, path));
        if let Some(t) = token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let req = req
            .body(body.map(|b| b.to_vec()).unwrap_or_default())
            .unwrap();
        let mut resp = agent.run(req).unwrap();
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        (
            status,
            if text.is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&text).unwrap()
            },
        )
    }

    pub fn jobs(&self) -> Arc<life_server::jobs::Jobs> {
        self.app.jobs.clone()
    }

    pub fn get(&self, path: &str) -> (u16, Value) {
        self.call("GET", path, None, &[], Some(TOKEN))
    }

    pub fn post_json(&self, path: &str, body: &Value) -> (u16, Value) {
        self.call(
            "POST",
            path,
            Some(body.to_string().as_bytes()),
            &[],
            Some(TOKEN),
        )
    }

    pub fn post(&self, path: &str, body: &[u8]) -> (u16, Value) {
        self.call("POST", path, Some(body), &[], Some(TOKEN))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Epoch milliseconds of a local time.
pub fn ms(y: i32, m: u32, d: u32, h: u32, mi: u32) -> i64 {
    use chrono::TimeZone;
    chrono::Local
        .with_ymd_and_hms(y, m, d, h, mi, 0)
        .unwrap()
        .timestamp_millis()
}
