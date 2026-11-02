//! The first run (#69): the connection code a phone pastes (address + token in one line, only for addresses the token
//! may go to), checking that a model address answers a tiny request, and how far the first sync and graph got.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

use life_server::config::Paths;
use life_server::onboard::{self, check_model, connect_code, parse_connect_code, safe_for_token};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn a_token_goes_only_over_https_or_to_the_tailnet_the_same_network_and_this_device() {
    for ok in [
        "https://vault.example.com/ingest/lorelog",
        "https://pc.tail8d1ab7.ts.net:48765",
        "http://pc.tail8d1ab7.ts.net:8765",
        "http://100.101.1.2:8765",
        "http://127.0.0.1:8765",
        "http://localhost:8765",
        "http://[::1]:8765",
        "http://[fd7a:115c:a1e0::1]:8765",
        "http://pc.tail8d1ab7.ts.net.:8765",
        "http://100.64.0.1.:8765",
        // the same network (a private IPv4 address): plain http in the user's own Wi-Fi
        "http://192.168.0.10:8765",
        "http://192.168.100.3:8765/ingest/lorelog",
        "http://10.0.0.1:8765",
        "http://10.255.255.254:8765",
        "http://172.16.0.1:8765",
        "http://172.31.255.1:8765",
    ] {
        assert!(safe_for_token(ok), "{ok}");
    }
    for bad in [
        "http://172.15.0.1:8765",  // just below 172.16.0.0/12
        "http://172.32.0.1:8765",  // just above
        "http://192.169.0.1:8765", // not 192.168
        "http://11.0.0.1:8765",
        "http://169.254.1.1:8765", // link-local
        "http://0.0.0.0:8765",
        "http://8.8.8.8:8765",
        "http://012.0.0.1:8765", // octal: 10.0.0.1 to some readers, so a leading zero is refused
        "http://192.168.00.1:8765", // a leading zero
        "http://0xa.0.0.1:8765", // hex
        "http://10.1:8765",      // short form of 10.0.0.1
        "http://3232235521:8765", // 192.168.0.1 as one number
        "http://100.64.1:8765",  // short form of a tailnet address
        "http://10.0.0.1@evil.com/", // userinfo
        "http://10.0.0.1\\@evil.com/",
        "http://evil.com\\@10.0.0.1/",
        "http://%31%30.0.0.1:8765", // percent-encoded
        "http://+10.0.0.1:8765",
        "http://nas.local:8765", // names are not looked up: https only
        "http://nas:8765",
        "http://[fd00::1]:8765", // an IPv6 private address is not the same network rule
        "http://[::ffff:192.168.0.1]:8765", // nor a mapped one
        "http://evil.example.com",
        "http://100.200.1.2:8765", // outside 100.64.0.0/10
        "http://100.064.0.1:8765", // octal: 100.52.0.1
        "http://0100.64.0.1:8765",
        "http://ts.net.evil.com",
        "http://127.0.0.1@evil.com/",
        "http://pc.ts.net@evil.com/",
        "ftp://pc.tail8d1ab7.ts.net",
        "lorelog://x",
        "",
        "not a url",
    ] {
        assert!(!safe_for_token(bad), "{bad}");
    }
}

#[test]
fn the_connection_code_carries_address_and_token_and_reads_back() {
    let code = connect_code("http://pc.tail8d1ab7.ts.net:8765", TOKEN).unwrap();
    assert!(code.starts_with("lorelog://connect?"), "{code}");
    assert!(!code.contains(char::is_whitespace), "one line: {code}");
    assert_eq!(parse_connect_code(&code).unwrap(), ("http://pc.tail8d1ab7.ts.net:8765".to_string(), TOKEN.to_string()));

    // pasted with spaces and a line end around it, from a chat app or a note
    assert_eq!(parse_connect_code(&format!("  {code}\n")).unwrap().1, TOKEN);

    // characters that need escaping survive; a trailing slash on the address is dropped
    let odd = "ab+c/d=e&f?g%h_0123456789";
    let code = connect_code("https://vault.example.com/", odd).unwrap();
    assert_eq!(parse_connect_code(&code).unwrap(), ("https://vault.example.com".to_string(), odd.to_string()));

    // the exact line the phone's test (app/.../ConnectCodeTest.kt) reads: the two sides agree on the format
    assert_eq!(
        connect_code("https://vault.example.com/ingest/lorelog", odd).unwrap(),
        "lorelog://connect?u=https%3A%2F%2Fvault.example.com%2Fingest%2Florelog&t=ab%2Bc%2Fd%3De%26f%3Fg%25h_0123456789"
    );
    // a code for this PC's address in the same Wi-Fi, the line ConnectCodeTest.kt reads too
    assert_eq!(
        connect_code("http://192.168.100.3:8765", TOKEN).unwrap(),
        format!("lorelog://connect?u=http%3A%2F%2F192.168.100.3%3A8765&t={TOKEN}")
    );
    assert!(parse_connect_code(&format!("lorelog://connect?u=http%3A%2F%2F012.0.0.1%3A8765&t={TOKEN}")).is_err());
}

#[test]
fn no_code_is_made_or_read_for_an_address_the_token_must_not_go_to() {
    let e = connect_code("http://8.8.8.8:8765", TOKEN).unwrap_err();
    assert!(e.contains("tailnet") && e.contains("와이파이"), "{e}");
    let e = connect_code("http://pc.tail8d1ab7.ts.net:8765", "short").unwrap_err();
    assert!(e.contains("토큰"), "{e}");

    // a code written by hand for a public plain-http host is refused like manual entry
    let e = parse_connect_code(&format!("lorelog://connect?u=http%3A%2F%2Fevil.example.com&t={TOKEN}")).unwrap_err();
    assert!(e.contains("tailnet"), "{e}");
    for bad in [
        "".to_string(),
        "hello".to_string(),
        format!("https://x.ts.net/connect?u=https%3A%2F%2Fx.ts.net&t={TOKEN}"), // not our code
        "lorelog://connect?u=https%3A%2F%2Fx.ts.net".to_string(),               // no token
        format!("lorelog://connect?t={TOKEN}"),                                 // no address
        "lorelog://connect?u=https%3A%2F%2Fx.ts.net&t=short".to_string(),       // too short a token
        format!("lorelog://connect?u=https%3A%2F%2Fx.ts.net&t={TOKEN}%0A"),     // a line break in the token
    ] {
        assert!(parse_connect_code(&bad).is_err(), "{bad}");
    }
}

/// A one-shot HTTP server on a free port: answers one request with `status` and `body`, and hands back the request.
fn fake_model(status: u16, body: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());
        let (mut head, mut len) = (String::new(), 0usize);
        loop {
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap();
            }
            if line == "\r\n" || line.is_empty() {
                break;
            }
            head.push_str(&line);
        }
        let mut body_in = vec![0; len];
        r.read_exact(&mut body_in).unwrap();
        let reply = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(reply.as_bytes()).unwrap();
        tx.send(format!("{head}\n{}", String::from_utf8_lossy(&body_in))).unwrap();
    });
    (url, rx)
}

#[test]
fn a_model_that_answers_passes_the_check() {
    let (url, asked) = fake_model(200, r#"{"choices":[{"message":{"content":"OK"}}]}"#);
    let ok = check_model(&url, "qwen3:8b").unwrap();
    assert_eq!(ok.reply, "OK");
    let req = asked.recv().unwrap();
    assert!(req.starts_with("POST /v1/chat/completions"), "{req}");
    let body: serde_json::Value = serde_json::from_str(req.split_once("\n\n").unwrap().1).unwrap();
    assert_eq!(body["model"], "qwen3:8b", "the model name is sent");
}

#[test]
fn a_failed_check_says_what_to_look_at() {
    let (url, _) = fake_model(404, r#"{"error":"model 'nope' not found"}"#);
    let e = check_model(&url, "nope").unwrap_err();
    assert!(e.contains("모델 이름"), "{e}");

    // nothing listens there
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let e = check_model(&format!("http://127.0.0.1:{port}/v1"), "qwen3:8b").unwrap_err();
    assert!(e.contains("닿지 못했습니다"), "{e}");

    let (url, _) = fake_model(200, r#"{"unexpected": true}"#);
    assert!(check_model(&url, "qwen3:8b").unwrap_err().contains("OpenAI 호환"));

    assert!(check_model("", "qwen3:8b").unwrap_err().contains("주소"));
    assert!(check_model("127.0.0.1:11434/v1", "qwen3:8b").unwrap_err().contains("http"));
    assert!(check_model("http://127.0.0.1:11434/v1", " ").unwrap_err().contains("모델 이름"));
}

#[test]
fn progress_of_a_new_vault_is_zero_and_makes_no_index() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths {
        vault: dir.path().join("notes"),
        data: dir.path().join("data"),
        conf: dir.path().join("conf"),
        allow_file: dir.path().join("conf/diary.json"),
    };
    let p = onboard::progress(&paths);
    assert_eq!((p.records, p.mail, p.graph_read, p.graph_waiting), (0, 0, 0, 0));
    assert!(!dir.path().join("data/librarian.sqlite").exists(), "looking does not create the index");
}

#[test]
fn progress_counts_the_indexed_records_and_the_mails_the_graph_has_read_and_waits_for() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    for (i, day) in ["2026-03-02", "2026-03-03"].iter().enumerate() {
        let file = paths.vault.join("Mail/2026").join(format!("m{i}.md"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            file,
            format!("---\ntype: mail\nmail_id: <m{i}@x>\nmail_source: a/INBOX\ndate: {day}\ntime: 09:00\nfrom: 김 <kim@client.co.kr>\nto: [나 <me@mine.co.kr>]\ncc: []\nsubject: \"견적 {i}\"\nreferences: []\n---\n## Message\n견적서 부탁드립니다.\n"),
        )
        .unwrap();
    }
    let mut lib = life_server::librarian::Librarian::open(&paths, None, std::time::Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    let p = onboard::progress(&paths);
    assert_eq!((p.mail, p.graph_read, p.graph_waiting), (2, 0, 2));
    assert!(p.records >= 2);

    let run = life_server::librarian::graph::Run { limit: 1, workers: 1 };
    life_server::librarian::graph::extract(&mut lib, &NoRelations, &run).unwrap();
    let p = onboard::progress(&paths);
    assert_eq!((p.graph_read, p.graph_waiting), (1, 1));
}

struct NoRelations;
impl life_server::llm::Model for NoRelations {
    fn complete(&self, _: &[serde_json::Value], _: life_server::llm::Ask) -> anyhow::Result<String> {
        Ok(r#"{"r": []}"#.into())
    }
}
