//! `life-ingest` listens on every address in INGEST_BIND (e.g. the tailnet address and the same-network one).
#![cfg(target_os = "linux")] // 127.0.0.2 answers without setup on Linux only

use std::net::TcpListener;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn healthy(url: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Ok(mut r) = ureq::get(url).call() {
            return r.body_mut().read_to_string().is_ok_and(|b| b == r#"{"ok":true}"#);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn life_ingest_listens_on_each_address_in_ingest_bind() {
    let dir = tempfile::tempdir().unwrap();
    let conf = dir.path().join("conf");
    std::fs::create_dir_all(&conf).unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    std::fs::write(
        conf.join("ingest.env"),
        format!("INGEST_TOKEN=0123456789abcdef\nINGEST_BIND=127.0.0.1, 127.0.0.2\nINGEST_PORT={port}\n"),
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_life-ingest"))
        .env("LIFE_CONF_DIR", &conf)
        .env("LIFE_DATA_DIR", dir.path().join("data"))
        .env("LIFE_VAULT", dir.path().join("vault"))
        .spawn()
        .unwrap();
    let _server = Killed(child);
    assert!(healthy(&format!("http://127.0.0.1:{port}/healthz")), "the first address");
    assert!(healthy(&format!("http://127.0.0.2:{port}/healthz")), "the second address");
}

#[test]
fn life_ingest_help_explains_several_addresses() {
    let out = Command::new(env!("CARGO_BIN_EXE_life-ingest")).arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("INGEST_BIND=127.0.0.1") && help.contains("comma"), "{help}");
}
