//! The desktop app's screens are served from its own origin (`tauri://localhost`, on Windows
//! `http://tauri.localhost`) and call the server on 127.0.0.1 with the token: a cross-origin request. The webview
//! asks first (OPTIONS) and reads the answer only when the server allows that origin. Only the app's origins are
//! allowed; a web page in a browser is not.
mod common;

use common::{Server, TOKEN};

fn send(s: &Server, method: &str, path: &str, headers: &[(&str, &str)]) -> (u16, Option<String>, Option<String>) {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let mut req = ureq::http::Request::builder().method(method).uri(format!("{}{}", s.base, path));
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = agent.run(req.body(Vec::new()).unwrap()).unwrap();
    let h = |n: &str| resp.headers().get(n).map(|v| v.to_str().unwrap().to_string());
    (resp.status().as_u16(), h("access-control-allow-origin"), h("access-control-allow-headers"))
}

const PREFLIGHT: [(&str, &str); 2] =
    [("Access-Control-Request-Method", "POST"), ("Access-Control-Request-Headers", "authorization,content-type")];

#[test]
fn the_apps_own_origins_may_ask_first_without_the_token() {
    let s = Server::start();
    for origin in ["tauri://localhost", "http://tauri.localhost"] {
        let mut h = vec![("Origin", origin)];
        h.extend(PREFLIGHT);
        let (status, allow, headers) = send(&s, "OPTIONS", "/diary/2026-01-01/chat", &h);
        assert_eq!(status, 204, "{origin}");
        assert_eq!(allow.as_deref(), Some(origin));
        let headers = headers.unwrap().to_lowercase();
        assert!(headers.contains("authorization") && headers.contains("content-type"), "{headers}");
    }
}

#[test]
fn answers_to_the_app_say_it_may_read_them() {
    let s = Server::start();
    let bearer = format!("Bearer {TOKEN}");
    let (status, allow, _) =
        send(&s, "GET", "/todos", &[("Origin", "tauri://localhost"), ("Authorization", bearer.as_str())]);
    assert_eq!((status, allow.as_deref()), (200, Some("tauri://localhost")));
    // a wrong token is still refused, and the app may read that it was (to ask for the token again)
    let (status, allow, _) =
        send(&s, "GET", "/todos", &[("Origin", "tauri://localhost"), ("Authorization", "Bearer nope")]);
    assert_eq!((status, allow.as_deref()), (401, Some("tauri://localhost")));
}

#[test]
fn other_origins_are_not_allowed() {
    let s = Server::start();
    for origin in ["https://evil.example", "http://localhost:3000", "null", "tauri://localhost.evil"] {
        let mut h = vec![("Origin", origin)];
        h.extend(PREFLIGHT);
        let (status, allow, _) = send(&s, "OPTIONS", "/diary/2026-01-01/chat", &h);
        assert_eq!((status, allow), (404, None), "{origin}");
        let bearer = format!("Bearer {TOKEN}");
        let (status, allow, _) = send(&s, "GET", "/todos", &[("Origin", origin), ("Authorization", bearer.as_str())]);
        assert_eq!((status, allow), (200, None), "{origin}");
    }
}

#[test]
fn without_an_origin_nothing_changes() {
    let s = Server::start();
    let (status, allow, _) = send(&s, "OPTIONS", "/todos", &PREFLIGHT);
    assert_eq!((status, allow), (404, None));
}
