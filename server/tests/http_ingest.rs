//! The collection endpoint: records and files from phones and PCs. Ported from the Python server's
//! test_ingest_http.py (the ingest part) and the life-ingest selftest.
mod common;

use common::{Server, TOKEN, ms};
use serde_json::json;

#[test]
fn healthz_needs_no_token_and_says_nothing_else() {
    let s = Server::start();
    assert_eq!(s.call("GET", "/healthz", None, &[], None), (200, json!({"ok": true})));
}

#[test]
fn everything_but_healthz_needs_the_token() {
    let s = Server::start();
    for (method, path) in [
        ("POST", "/ingest/lorelog"),
        ("POST", "/ingest/file"),
        ("GET", &format!("/ingest/file/{}", "a".repeat(64)) as &str),
    ] {
        assert_eq!(s.call(method, path, Some(b"{}"), &[], Some("wrong")).0, 401, "{path}");
        assert_eq!(s.call(method, path, Some(b"{}"), &[], None).0, 401, "{path}");
    }
}

#[test]
fn the_server_is_api_only_and_unknown_paths_are_404() {
    let s = Server::start();
    for path in ["/", "/index.html", "/app.js", "/nope", "/healthz/", "/healthz?x=1"] {
        assert_eq!(s.get(path).0, 404, "{path}");
    }
    assert_eq!(s.post("/ingest/BadSource", b"{}").0, 404);
    assert_eq!(s.post("/ingest/lorelog?x=1", b"{}").0, 404, "the query is part of what a route must match");
}

#[test]
fn records_are_stored_once_and_bad_lines_are_counted() {
    let s = Server::start();
    let ts = ms(2026, 10, 26, 13, 5);
    let body = format!(
        "{}\n{}\n이건 JSON 이 아님\n[1,2]\n",
        json!({"type": "voice", "ts": ts, "text": "회의 시작"}),
        json!({"type": "app", "ts": ts + 1000, "pkg": "com.x"})
    );
    assert_eq!(
        s.post("/ingest/lorelog", body.as_bytes()),
        (200, json!({"accepted": 2, "duplicates": 0, "rejected": 2}))
    );
    assert_eq!(
        s.post("/ingest/lorelog/", body.as_bytes()),
        (200, json!({"accepted": 0, "duplicates": 2, "rejected": 2}))
    );
    let text = std::fs::read_to_string(s.root.join("raw/lorelog/2026-10-26.ndjson")).unwrap();
    assert_eq!(text.lines().count(), 2);
}

#[test]
fn an_empty_or_too_large_body_is_refused() {
    let s = Server::start_with(100);
    assert_eq!(s.post("/ingest/lorelog", b"").0, 413);
    let big = vec![b' '; 200];
    let (status, body) = s.post("/ingest/lorelog", &big);
    assert_eq!((status, body["max_bytes"].as_u64()), (413, Some(100)));
}

// ---------- POST /ingest/file (from test_filestore.py) ----------

fn upload(
    s: &Server,
    data: &[u8],
    name: &str,
    when_ms: Option<i64>,
    sha: Option<&str>,
    token: &str,
) -> (u16, serde_json::Value) {
    let real = life_server::store::sha256_hex(data);
    let sha = sha.unwrap_or(&real).to_string();
    let quoted = percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC).to_string();
    let ts = when_ms.map(|w| w.to_string());
    let mut headers =
        vec![("X-File-Sha256", sha.as_str()), ("X-File-Name", quoted.as_str()), ("X-File-Mime", "application/pdf")];
    if let Some(t) = ts.as_deref() {
        headers.push(("X-File-Ts", t));
    }
    s.call("POST", "/ingest/file", Some(data), &headers, Some(token))
}

#[test]
fn a_file_needs_the_token() {
    let s = Server::start();
    assert_eq!(upload(&s, b"x", "a", None, None, "wrong").0, 401);
}

#[test]
fn a_file_is_kept_under_its_hash_with_a_sighting() {
    let s = Server::start();
    let data = "분기 보고서 본문".as_bytes();
    let sha = life_server::store::sha256_hex(data);
    let when = ms(2026, 10, 27, 9, 0);
    assert_eq!(
        upload(&s, data, "보고서.pdf", Some(when), None, TOKEN),
        (200, json!({"sha256": sha, "stored": true, "duplicate": false}))
    );
    assert_eq!(std::fs::read(s.root.join("raw/files").join(&sha[..2]).join(&sha)).unwrap(), data);
    let line = std::fs::read_to_string(s.root.join("raw/pcfile/2026-10-27.ndjson")).unwrap();
    let rec: serde_json::Value = serde_json::from_str(line.lines().next().unwrap()).unwrap();
    assert_eq!(
        (rec["type"].as_str(), rec["sha256"].as_str(), rec["name"].as_str()),
        (Some("file"), Some(sha.as_str()), Some("보고서.pdf"))
    );
    assert_eq!(rec["size"].as_u64(), Some(data.len() as u64));
}

#[test]
fn a_resend_keeps_one_blob_but_records_each_sighting() {
    let s = Server::start();
    let data = b"same file seen twice";
    assert_eq!(upload(&s, data, "a.txt", Some(ms(2026, 10, 28, 8, 0)), None, TOKEN).1["stored"], true);
    let (status, body) = upload(&s, data, "a.txt", Some(ms(2026, 10, 28, 15, 0)), None, TOKEN);
    assert_eq!((status, &body["stored"], &body["duplicate"]), (200, &json!(false), &json!(true)));
    let text = std::fs::read_to_string(s.root.join("raw/pcfile/2026-10-28.ndjson")).unwrap();
    assert_eq!(text.lines().count(), 2);
}

#[test]
fn a_hash_mismatch_is_refused_and_nothing_is_written() {
    let s = Server::start();
    let zeros = "0".repeat(64);
    assert_eq!(upload(&s, b"the body", "b.txt", None, Some(&zeros), TOKEN).0, 400);
    assert!(!s.root.join("raw/files/00").join(&zeros).exists());
    assert!(!s.root.join("raw/pcfile").exists(), "no sighting either");
}

#[test]
fn the_agent_can_ask_whether_a_blob_is_kept() {
    let s = Server::start();
    let data = b"ask before upload";
    let sha = life_server::store::sha256_hex(data);
    assert_eq!(s.get(&format!("/ingest/file/{sha}")).1, json!({"sha256": sha, "present": false}));
    upload(&s, data, "c.txt", Some(ms(2026, 1, 2, 9, 0)), None, TOKEN);
    assert_eq!(s.get(&format!("/ingest/file/{sha}")).1, json!({"sha256": sha, "present": true}));
}

#[test]
fn a_sighting_records_a_line_without_the_bytes_and_takes_the_size_from_the_blob() {
    let s = Server::start();
    let data = b"a kept deliverable seen again";
    let sha = life_server::store::sha256_hex(data);
    upload(&s, data, "d.txt", Some(ms(2026, 10, 30, 9, 0)), None, TOKEN);
    let ts = ms(2026, 10, 30, 17, 0).to_string();
    let headers = [
        ("X-File-Sha256", sha.as_str()),
        ("X-File-Name", "d.txt"),
        ("X-File-Mime", "text/plain"),
        ("X-File-Sighting", "1"),
        ("X-File-Ts", ts.as_str()),
    ];
    let (status, body) = s.call("POST", "/ingest/file", Some(b""), &headers, Some(TOKEN));
    assert_eq!((status, &body["stored"], &body["duplicate"]), (200, &json!(false), &json!(true)));
    let text = std::fs::read_to_string(s.root.join("raw/pcfile/2026-10-30.ndjson")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    let rec: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(rec["size"].as_u64(), Some(data.len() as u64));
}

#[test]
fn a_sighting_of_an_unknown_or_malformed_sha_is_refused_not_a_path() {
    let s = Server::start();
    let ones = "1".repeat(64);
    let g = "g".repeat(64);
    for bad in [ones.as_str(), "", "../../etc/passwd", "zz", g.as_str()] {
        let headers = [("X-File-Sha256", bad), ("X-File-Name", "x"), ("X-File-Sighting", "1")];
        assert_eq!(s.call("POST", "/ingest/file", Some(b""), &headers, Some(TOKEN)).0, 409, "{bad}");
    }
}
