use life_agent_core::agent::{Agent, Sampler};
use life_agent_core::config::Config;
use life_agent_core::segments::Sample;
use life_agent_core::spool::Spool;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

const TOKEN: &str = "agent-test-token-0123456789";
const T0: f64 = 1_800_000_000.0;

/// A stand-in for life-ingest: Bearer auth, NDJSON body, duplicate lines stored once.
struct FakeServer {
    url: String,
    stored: Arc<Mutex<Vec<String>>>,
}

fn fake_server() -> FakeServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let stored = Arc::new(Mutex::new(Vec::<String>::new()));
    let store = stored.clone();
    std::thread::spawn(move || {
        let mut seen = HashSet::new();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut len, mut auth, mut path) = (0usize, String::new(), String::new());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            path = line.split_whitespace().nth(1).unwrap_or(&path).to_string();
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                let lower = line.to_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                if lower.starts_with("authorization:") {
                    auth = line.split_once(':').unwrap().1.trim().to_string();
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            let (code, text) = if auth != format!("Bearer {TOKEN}") {
                (401, "unauthorized")
            } else if path != "/ingest/pc" {
                (404, "not found")
            } else {
                for l in String::from_utf8(body).unwrap().lines() {
                    if seen.insert(l.to_string()) {
                        store.lock().unwrap().push(l.to_string());
                    }
                }
                (200, "{\"ok\":true}")
            };
            write!(stream, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len()).unwrap();
        }
    });
    FakeServer { url, stored }
}

#[derive(Clone)]
struct Fake(Rc<RefCell<Result<Sample, String>>>);

impl Sampler for Fake {
    fn sample(&mut self) -> Result<Sample, String> {
        self.0.borrow().clone()
    }
}

struct Rig {
    agent: Agent<Fake>,
    now: Rc<Cell<f64>>,
    shown: Rc<RefCell<Result<Sample, String>>>,
    _dir: tempfile::TempDir,
}

fn rig(url: &str, token: &str) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let now = Rc::new(Cell::new(T0));
    let shown = Rc::new(RefCell::new(Ok(Sample {
        app: "a.exe".into(),
        title: "문서".into(),
        idle_s: 0.0,
        ..Sample::default()
    })));
    let cfg = Config {
        url: url.into(),
        token: token.into(),
        ..Config::default()
    };
    let clock = now.clone();
    let agent = Agent::new(
        cfg,
        Fake(shown.clone()),
        Spool::new(dir.path()),
        Box::new(move || clock.get()),
    );
    Rig {
        agent,
        now,
        shown,
        _dir: dir,
    }
}

#[test]
fn send_spool_refuses_to_send_the_token_to_a_public_host() {
    use life_agent_core::sender::send_spool;
    let dir = tempfile::tempdir().unwrap();
    let spool = Spool::new(dir.path());
    // put a line in the spool so there is something to send
    std::fs::write(spool.path(), "{\"type\":\"window\",\"app\":\"x\"}\n").unwrap();
    let cfg = Config {
        url: "https://evil.example.com".into(),
        token: TOKEN.into(),
        ..Config::default()
    };
    let (sent, err) = send_spool(&cfg, &spool);
    assert_eq!(sent, 0, "nothing is sent to a non-tailnet host");
    assert!(err.contains("tailnet"), "the refusal names the reason: {err}");
    // the spool is left intact for a later, correctly-configured send
    assert_eq!(spool.read().len(), 1);
}

impl Rig {
    fn tick(&mut self, secs: f64, n: usize) {
        for _ in 0..n {
            self.now.set(self.now.get() + secs);
            self.agent.step();
        }
    }
    fn show(&self, app: &str, title: &str) {
        *self.shown.borrow_mut() = Ok(Sample {
            app: app.into(),
            title: title.into(),
            idle_s: 0.0,
            ..Sample::default()
        });
    }
}

#[test]
fn records_reach_the_server_and_leave_the_spool() {
    let server = fake_server();
    let mut r = rig(&server.url, TOKEN);
    r.tick(5.0, 6);
    r.show("b.exe", "표");
    r.tick(5.0, 4);
    assert_eq!(r.agent.stop(), (2, String::new()));
    let stored = server.stored.lock().unwrap().clone();
    let recs: Vec<serde_json::Value> = stored
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(recs[0]["app"], "a.exe");
    assert_eq!(recs[0]["title"], "문서");
    assert_eq!(recs[1]["app"], "b.exe");
    assert!(
        r.agent.spool().read().is_empty(),
        "delivered lines leave the spool"
    );
}

#[test]
fn nothing_is_lost_while_the_server_refuses_and_a_resend_adds_nothing() {
    let server = fake_server();
    let mut r = rig(&server.url, "wrong-token-0123456789");
    r.tick(5.0, 4);
    let (sent, err) = r.agent.stop();
    assert_eq!((sent, err.as_str()), (0, "the server refused the token"));
    let pending = r.agent.spool().read().len();
    assert!(pending > 0, "undelivered records stay in the spool");
    r.agent.config_mut().token = TOKEN.into();
    assert_eq!(r.agent.flush(), (pending, String::new()));
    assert_eq!(r.agent.flush(), (0, String::new()));
    assert_eq!(server.stored.lock().unwrap().len(), pending);
}

#[test]
fn sleep_gap_ends_the_segment_where_it_was_last_seen() {
    let mut r = rig("", ""); // no url: everything stays in the spool, so the test reads only its own records
    r.tick(5.0, 4); // first seen at t0+5, last seen at t0+20
    r.tick(3600.0, 1); // the laptop slept for an hour
    r.agent.stop();
    let lines = r.agent.spool().read();
    let first: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(
        first["dur_s"], 15,
        "the hour of sleep is not counted as work"
    );
    assert_eq!(first["end"], ((T0 + 20.0) * 1000.0) as i64);
}

#[test]
fn missing_url_and_token_are_reported_not_raised() {
    let mut r = rig("", "");
    r.tick(5.0, 3);
    assert!(r.agent.flush().1.contains("not set"));
}

#[test]
fn a_failed_sample_does_not_stop_the_agent() {
    let mut r = rig("", "");
    *r.shown.borrow_mut() = Err("window vanished".into());
    r.tick(5.0, 2);
    r.show("a.exe", "x");
    r.tick(5.0, 3);
    r.agent.stop();
    assert_eq!(r.agent.spool().read().len(), 1);
}
