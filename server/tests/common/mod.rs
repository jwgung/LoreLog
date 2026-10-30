//! A real server on a free local port over throwaway directories, and a client that does not treat 4xx as errors.
#![allow(dead_code)]
use std::path::PathBuf;
use std::sync::Arc;

use life_server::config::Paths;
use life_server::http::{App, serve};
use serde_json::Value;

pub const TOKEN: &str = "test-token-0123456789";

pub struct Server {
    pub base: String,
    pub root: PathBuf,
    pub paths: Paths,
    _dir: tempfile::TempDir,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    pub fn start() -> Server {
        Server::start_with(1024 * 1024)
    }

    pub fn start_with(max_bytes: usize) -> Server {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let app = Arc::new(App::new(paths.clone(), TOKEN, "personal", max_bytes));
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
