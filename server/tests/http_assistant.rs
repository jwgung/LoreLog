//! The assistant's HTTP API: the day's draft, the user's words, chat, senders, to-dos, speech.
//! Ported from the Python server's test_ingest_http.py (the assistant part).
mod common;

use std::time::Duration;

use common::{FakeModel, Server};
use serde_json::json;

#[test]
fn every_assistant_route_needs_the_token() {
    let s = Server::start();
    for (method, path) in [
        ("GET", "/todos"),
        ("GET", "/diary/senders"),
        ("GET", "/diary/2026-01-01"),
        ("POST", "/todos"),
        ("POST", "/transcribe"),
        ("GET", "/diary/allow"),
        ("GET", "/diary/2026-01-01/chat"),
    ] {
        assert_eq!(
            s.call(method, path, Some(b"{}"), &[], Some("wrong")).0,
            401,
            "{path}"
        );
    }
}

#[test]
fn the_allow_list_is_readable_so_the_screen_keeps_rules_it_cannot_show() {
    let s = Server::start();
    assert_eq!(
        s.post_json(
            "/diary/senders",
            &json!({"allow": ["카카오톡: 홍길동", "앱: *"]})
        ),
        (200, json!({"saved": true}))
    );
    assert_eq!(
        s.get("/diary/allow"),
        (200, json!({"allow": ["앱: *", "카카오톡: 홍길동"]}))
    );
    assert_eq!(s.post("/diary/senders", b"not json").0, 400);
    assert_eq!(s.get("/diary/senders"), (200, json!([])));
}

#[test]
fn a_todo_is_added_finished_and_listed() {
    let s = Server::start();
    assert_eq!(s.post_json("/todos", &json!({"text": "  "})).0, 400);
    assert_eq!(
        s.post_json("/todos", &json!({"text": "A 님께 견적 보내기"})),
        (200, json!({"ok": true}))
    );
    let (_, view) = s.get("/todos");
    let item = view["open"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["title"] == "A 님께 견적 보내기")
        .unwrap()
        .clone();
    assert_eq!(view["extracting"], false);
    let id = item["id"].as_str().unwrap();
    assert_eq!(
        s.post_json(&format!("/todos/{id}"), &json!({"action": "bogus"}))
            .0,
        400
    );
    assert_eq!(
        s.post_json(&format!("/todos/{id}"), &json!({"action": "done"})),
        (200, json!({"ok": true}))
    );
    assert_eq!(
        s.post_json("/todos/0000000000", &json!({"action": "done"}))
            .0,
        404
    );
    let (_, view) = s.get("/todos");
    assert_eq!(view["done"][0]["title"], "A 님께 견적 보내기");
}

#[test]
fn the_users_words_are_saved_and_an_empty_chat_message_is_refused() {
    let s = Server::start();
    assert_eq!(
        s.post_json("/diary/2026-02-02", &json!({"reflection": "좋은 하루"})),
        (200, json!({"saved": true}))
    );
    assert_eq!(
        s.get("/diary/2026-02-02/chat"),
        (200, json!({"messages": []}))
    );
    assert_eq!(
        s.post_json("/diary/2026-02-02/chat", &json!({"message": "   "}))
            .0,
        400
    );
    assert_eq!(
        s.post_json("/diary/2026-02-02/chat", &json!({"text": "no message"}))
            .0,
        400
    );
    assert_eq!(s.post("/diary/2026-02-02", b"{broken").0, 400);
    let note = std::fs::read_to_string(s.paths.vault.join("Diary/2026-02-02.md")).unwrap();
    assert!(note.contains("## 회고\n좋은 하루"));
}

#[test]
fn a_chat_message_is_answered_and_both_turns_are_kept() {
    let model = FakeModel::plain();
    let s = Server::start_full(1 << 20, model.clone());
    assert_eq!(
        s.post_json(
            "/diary/2026-02-03/chat",
            &json!({"message": "오늘 좀 바빴어"})
        ),
        (200, json!({"reply": "네, 확인했어요."}))
    );
    let asked = model.last();
    assert!(
        asked[0]["content"]
            .as_str()
            .unwrap()
            .contains("[오늘 기록]\n## 일정")
    );
    assert_eq!(asked.last().unwrap()["content"], "오늘 좀 바빴어");
    assert_eq!(
        s.get("/diary/2026-02-03/chat").1["messages"],
        json!([{"role": "user", "text": "오늘 좀 바빴어"}, {"role": "assistant", "text": "네, 확인했어요."}])
    );
    let (_, ok) = s.post_json(
        "/diary/2026-02-03/chat",
        &json!({"message": "20분 했어", "wrong": "10초 통화"}),
    );
    assert_eq!(ok["reply"], "네, 확인했어요.");
    let history = s.get("/diary/2026-02-03/chat").1;
    assert_eq!(
        history["messages"][2],
        json!({"role": "user", "text": "20분 했어", "wrong": "10초 통화"})
    );
}

#[test]
fn a_failing_model_is_a_502_with_the_reason() {
    let s = Server::start_full(1 << 20, FakeModel::new(|_| anyhow::bail!("model down")));
    let (status, body) = s.post_json("/diary/2026-02-04/chat", &json!({"message": "안녕"}));
    assert_eq!(status, 502);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("chat failed: model down")
    );
}

#[test]
fn a_draft_failure_is_reported_once_then_a_retry_is_allowed() {
    let s = Server::start();
    let app_jobs = s.jobs();
    app_jobs.set_failed("2026-03-03", "LLM request failed: down");
    assert_eq!(
        s.get("/diary/2026-03-03"),
        (502, json!({"error": "LLM request failed: down"}))
    );
    assert!(!app_jobs.pending("2026-03-03"));
}

#[test]
fn a_missing_draft_is_made_in_the_background_then_served() {
    let s = Server::start();
    assert_eq!(
        s.get("/diary/2026-04-04"),
        (202, json!({"status": "generating"}))
    );
    let mut served = None;
    for _ in 0..100 {
        let (status, body) = s.get("/diary/2026-04-04");
        if status == 200 {
            served = Some(body);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let body = served.expect("the draft is served once made");
    assert_eq!(body["summary"], json!(["오전 9시 회의가 있었어요."]));
    assert_eq!(body["question"], "어떠셨어요?");
    assert_eq!(
        s.get("/diary/2026-04-04?refresh=1"),
        (202, json!({"status": "generating"})),
        "a refresh makes it again"
    );
    assert_eq!(s.get("/diary/2026-04-04?other=1").0, 404);
}

#[test]
fn speech_is_transcribed_and_a_failure_names_only_its_kind() {
    let s = Server::start();
    assert_eq!(
        s.post("/transcribe", b"wavdata"),
        (200, json!({"text": "7바이트 들음"}))
    );
    assert_eq!(
        s.post("/transcribe", b""),
        (
            413,
            json!({"error": "too large or empty", "max_bytes": 1 << 20})
        )
    );
    let (status, body) = s.post("/transcribe", b"broken");
    assert_eq!(status, 502);
    assert!(body["error"].as_str().unwrap().starts_with("asr failed: "));
}
