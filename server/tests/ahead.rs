//! The assistant speaks first, where the next thing happens (#60): ten minutes before an event, one line about a
//! request someone of that event's people or organizations made that the user has not answered, with the open to-do
//! of the same matter. Only what is certain: an attendee address that is exactly a person or an organization of the
//! graph, a request with its record and day. Once per event, never the same (event, matter) again, nothing for
//! 22:00–07:00 or all-day events, nothing from an organization the user stopped. The buttons are corrections (#10).
mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use chrono::{NaiveDateTime, TimeDelta};
use common::{FakeModel, Server, TOKEN};
use life_server::assistant::ahead::{self, Ask, Event};
use life_server::assistant::context::subj_key;
use life_server::librarian::Librarian;
use life_server::librarian::graph;
use serde_json::{Value, json};

const ME: &str = "나 <me@mine.co.kr>";

#[allow(clippy::too_many_arguments)]
fn mail(s: &Server, name: &str, day: &str, from: &str, to: &str, subject: &str, body: &str, sent: bool, refs: &str) {
    let path = s.paths.vault.join("Mail").join(&day[..4]).join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let source = if sent { "account/Sent Messages" } else { "account/INBOX" };
    fs::write(
        path,
        format!("---\ntype: mail\nmail_id: <{name}@x>\nmail_source: {source}\ndate: {day}\ntime: 09:00\nfrom: {from}\nto: [{to}]\ncc: []\nsubject: \"{subject}\"\nreferences: {refs}\n---\n## Message\n{body}\n"),
    )
    .unwrap();
}

fn reader() -> Arc<FakeModel> {
    FakeModel::new(|msgs| {
        let text = msgs.last().unwrap()["content"].as_str().unwrap_or("").to_string();
        let r = if text.contains("측정 자료 요청드립니다") {
            json!([[
                "박고객 <park@client.co.kr>:person",
                "requested",
                "측정 자료:doc",
                "측정 자료",
                "2026-03-20",
                "측정 자료 요청드립니다"
            ]])
        } else if text.contains("설치 사진 보내 주세요") {
            json!([[
                "이담당 <lee@client.co.kr>:person",
                "requested",
                "설치 사진:doc",
                "설치 사진",
                "2026-03-25",
                "설치 사진 보내 주세요"
            ]])
        } else if text.contains("견적서 보내 주세요") {
            json!([[
                "최대리 <choi@other.co.kr>:person",
                "requested",
                "견적서:doc",
                "견적서",
                "2026-03-26",
                "견적서 보내 주세요"
            ]])
        } else {
            json!([])
        };
        Ok(json!({"r": r}).to_string())
    })
}

/// Requests of client.co.kr: one answered (측정 자료), one not (설치 사진, with its open to-do); one of other.co.kr.
fn world() -> Server {
    let s = Server::start();
    fs::create_dir_all(s.paths.vault.join("Events")).unwrap();
    let park = "박고객 <park@client.co.kr>";
    mail(&s, "a1.md", "2026-03-20", park, ME, "측정 자료", "측정 자료 요청드립니다", false, "[]");
    mail(&s, "a2.md", "2026-03-22", ME, park, "RE: 측정 자료", "보내드립니다", true, "[<a1.md@x>]");
    mail(&s, "b1.md", "2026-03-25", "이담당 <lee@client.co.kr>", ME, "설치 사진", "설치 사진 보내 주세요", false, "[]");
    mail(&s, "c1.md", "2026-03-26", "최대리 <choi@other.co.kr>", ME, "견적 문의", "견적서 보내 주세요", false, "[]");
    index(&s);
    fs::write(
        s.paths.todo_file(),
        json!({"items": [
            {"id": "abc1234567", "title": "설치 사진 보내기", "source": "메일", "due": "", "status": "open",
             "day": "2026-03-25", "thread": subj_key("설치 사진"), "created": 0},
            {"id": "def1234567", "title": "다른 일", "source": "메일", "due": "", "status": "open",
             "day": "2026-03-25", "thread": subj_key("다른 제목"), "created": 0}
        ], "last_extract": 0})
        .to_string(),
    )
    .unwrap();
    s
}

fn index(s: &Server) {
    let mut lib = Librarian::open(&s.paths, None, Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    graph::extract(&mut lib, reader().as_ref(), &graph::Run { limit: 50, workers: 1 }).unwrap();
}

fn at(day: &str, hm: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(&format!("{day} {hm}"), "%Y-%m-%d %H:%M").unwrap()
}

fn event(key: &str, start: NaiveDateTime, title: &str, attendees: &[&str]) -> Event {
    Event {
        key: key.into(),
        start,
        title: title.into(),
        attendees: attendees.iter().map(|a| a.to_string()).collect(),
        all_day: false,
    }
}

fn ask(events: Vec<Event>) -> Ask {
    Ask { events, stopped: Vec::new() }
}

const DAY: &str = "2026-11-02";

#[test]
fn an_event_with_someone_of_an_organization_with_an_unanswered_request_gets_one_line_ten_minutes_before() {
    let s = world();
    let e = event("e1", at(DAY, "10:00"), "점검 회의", &["박고객 <park@client.co.kr>"]);
    let got = ahead::take(&s.paths, &ask(vec![e]), at(DAY, "09:00"), ahead::WINDOW).unwrap();
    assert_eq!(got.len(), 1, "{got:?}");
    let a = &got[0];
    assert_eq!(a.title, "10분 뒤 「점검 회의」");
    assert_eq!(a.text, "이담당님이 3월 25일에 요청한 「설치 사진」에 아직 답하지 않았어요. 할 일: 설치 사진 보내기");
    assert_eq!(a.question, "이담당님이 요청한 「설치 사진」 건 어떻게 됐지?");
    assert_eq!(a.at, at(DAY, "09:50"));
    assert_eq!(a.start, at(DAY, "10:00"));
    assert_eq!(a.event, "e1");
    assert_eq!(a.stop, "org:client.co.kr");
    assert!(a.edge.starts_with("edge:") && a.edge.len() == 21, "{}", a.edge);
    assert!(a.wrong.contains("설치 사진"), "{}", a.wrong);
}

#[test]
fn an_event_is_spoken_of_once_and_the_same_matter_not_again() {
    let s = world();
    let e = || event("e1", at(DAY, "10:00"), "점검 회의", &["park@client.co.kr"]);
    assert_eq!(ahead::take(&s.paths, &ask(vec![e()]), at(DAY, "09:00"), ahead::WINDOW).unwrap().len(), 1);
    assert!(ahead::take(&s.paths, &ask(vec![e()]), at(DAY, "09:15"), ahead::WINDOW).unwrap().is_empty());
    // the same event known by another device (another key, the same start and title) is the same event
    let other = event("dav-uid", at(DAY, "10:00"), "점검 회의", &["park@client.co.kr"]);
    assert!(ahead::take(&s.paths, &ask(vec![other]), at(DAY, "09:20"), ahead::WINDOW).unwrap().is_empty());
    // another event with the same people may speak of it once
    let later = event("e2", at(DAY, "11:00"), "후속 회의", &["park@client.co.kr"]);
    assert_eq!(ahead::take(&s.paths, &ask(vec![later]), at(DAY, "09:30"), ahead::WINDOW).unwrap().len(), 1);
}

#[test]
fn only_what_is_certain() {
    let s = world();
    let now = at(DAY, "09:00");
    let none = |e: Event| assert!(ahead::take(&s.paths, &ask(vec![e]), now, ahead::WINDOW).unwrap().is_empty());
    none(event("x1", at(DAY, "10:00"), "모르는 사람", &["nobody@nowhere.kr"]));
    none(event("x2", at(DAY, "10:00"), "이름만", &["박고객"]));
    none(event("x3", at(DAY, "10:00"), "참석자 없음", &[]));
    none(Event { all_day: true, ..event("x4", at(DAY, "00:00"), "종일", &["park@client.co.kr"]) });
    none(event("x5", at(DAY, "11:30"), "두 시간 뒤", &["park@client.co.kr"]));
    none(event("x6", at(DAY, "08:59"), "이미 시작", &["park@client.co.kr"]));
    // the other organization's own request, not client.co.kr's
    let a = ahead::take(
        &s.paths,
        &ask(vec![event("x7", at(DAY, "10:00"), "견적", &["choi@other.co.kr"])]),
        now,
        ahead::WINDOW,
    )
    .unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].text, "최대리님이 3월 26일에 요청한 「견적서」에 아직 답하지 않았어요.");
}

#[test]
fn nothing_between_ten_at_night_and_seven_in_the_morning() {
    assert!(ahead::quiet(at(DAY, "22:00")) && ahead::quiet(at(DAY, "03:00")) && ahead::quiet(at(DAY, "06:59")));
    assert!(!ahead::quiet(at(DAY, "07:00")) && !ahead::quiet(at(DAY, "21:59")));
    let s = world();
    let late = event("n1", at(DAY, "22:15"), "밤 회의", &["park@client.co.kr"]);
    assert!(ahead::take(&s.paths, &ask(vec![late]), at(DAY, "21:00"), ahead::WINDOW).unwrap().is_empty());
    let early = event("n2", at("2026-11-03", "07:05"), "아침 회의", &["park@client.co.kr"]);
    assert!(ahead::take(&s.paths, &ask(vec![early]), at("2026-11-03", "06:00"), ahead::WINDOW).unwrap().is_empty());
    // said at 21:55, before the night
    let before = event("n3", at(DAY, "22:05"), "늦은 회의", &["park@client.co.kr"]);
    assert_eq!(ahead::take(&s.paths, &ask(vec![before]), at(DAY, "21:00"), ahead::WINDOW).unwrap().len(), 1);
}

#[test]
fn an_organization_the_user_stopped_is_not_spoken_of_again() {
    let s = world();
    let stopped = Ask {
        events: vec![event("s1", at(DAY, "10:00"), "회의", &["park@client.co.kr"])],
        stopped: vec!["org:client.co.kr".into()],
    };
    assert!(ahead::take(&s.paths, &stopped, at(DAY, "09:00"), ahead::WINDOW).unwrap().is_empty());
    // kept: the desktop, which never sees the swipes, honours it too
    let again = event("s2", at(DAY, "11:00"), "회의", &["park@client.co.kr"]);
    assert!(ahead::take(&s.paths, &ask(vec![again]), at(DAY, "10:00"), ahead::WINDOW).unwrap().is_empty());
}

#[test]
fn the_events_of_the_vault_are_asked_about_too() {
    let s = world();
    fs::write(
        s.paths.vault.join("Events").join(format!("{DAY} 현장 점검.md")),
        format!("---\ntype: event\ndav_uid: u1\ntitle: 현장 점검\nstart: {DAY}T10:00:00\nend: {DAY}T11:00:00\nall_day: false\nattendees:\n  - 이담당 <lee@client.co.kr>\n---\n"),
    )
    .unwrap();
    let got = ahead::take(&s.paths, &ask(Vec::new()), at(DAY, "09:00"), ahead::WINDOW).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].title, "10분 뒤 「현장 점검」");
    // what the desktop asks: what starts within ten minutes, said now
    let s = world();
    fs::write(
        s.paths.vault.join("Events").join(format!("{DAY} 현장 점검.md")),
        format!("---\ntype: event\ndav_uid: u1\ntitle: 현장 점검\nstart: {DAY}T10:00:00\nall_day: false\nattendees:\n  - lee@client.co.kr\n---\n"),
    )
    .unwrap();
    assert!(ahead::take(&s.paths, &ask(Vec::new()), at(DAY, "09:45"), ahead::LEAD).unwrap().is_empty());
    let now = ahead::take(&s.paths, &ask(Vec::new()), at(DAY, "09:52"), ahead::LEAD).unwrap();
    assert_eq!(now[0].title, "8분 뒤 「현장 점검」");
}

#[test]
fn the_phones_calendar_in_the_vault_is_asked_about_too_and_once_with_the_phones_own_ask() {
    let phone_calendar = || {
        let s = world();
        let start = at(DAY, "10:00").and_local_timezone(chrono::Local).unwrap().timestamp_millis();
        let rec = json!({"type": "calendar_event", "ts": start - 86_400_000, "key": "5@1", "uid": "",
                         "title": "현장 점검", "start": start, "end": start + 3_600_000, "all_day": false,
                         "location": "", "attendees": ["이담당 <lee@client.co.kr>"], "status": "upsert"});
        let dir = s.paths.raw().join("lorelog");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("2026-11-01.ndjson"), format!("{rec}\n")).unwrap();
        index(&s);
        s
    };
    // the desktop asks with nothing of its own: the phone's calendar, as the vault has it
    let s = phone_calendar();
    let now = ahead::take(&s.paths, &ask(Vec::new()), at(DAY, "09:52"), ahead::LEAD).unwrap();
    assert_eq!(now.len(), 1);
    assert_eq!(now[0].title, "8분 뒤 「현장 점검」");
    // the phone asks with the same event of its own: one event, one line
    let s = phone_calendar();
    let mine = event("5@1", at(DAY, "10:00"), "현장 점검", &["lee@client.co.kr"]);
    let got = ahead::take(&s.paths, &ask(vec![mine]), at(DAY, "09:00"), ahead::WINDOW).unwrap();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].event, "5@1", "the asker's own key comes back");
}

fn post_ahead(s: &Server, events: Value) -> (u16, Value) {
    s.post_json("/assistant/ahead", &json!({"events": events}))
}

#[test]
fn the_buttons_are_corrections_and_close_the_request() {
    let s = world();
    let e = event("h1", at(DAY, "10:00"), "회의", &["park@client.co.kr"]);
    let a = ahead::take(&s.paths, &ask(vec![e]), at(DAY, "09:00"), ahead::WINDOW).unwrap().remove(0);
    // 「이미 했어요」: the request is answered; nothing more about it
    let (code, done) = s.post_json("/corrections", &json!({"wrong": a.wrong, "right": ahead::ANSWERED, "ref": a.edge}));
    assert_eq!(code, 200, "{done}");
    assert!(done["changed"].as_str().unwrap().starts_with("요청에 답한 것으로 했어요"), "{done}");
    let later = |key: &str, hm: &str| ask(vec![event(key, at(DAY, hm), "다른 회의", &["park@client.co.kr"])]);
    assert!(ahead::take(&s.paths, &later("h2", "11:00"), at(DAY, "10:00"), ahead::WINDOW).unwrap().is_empty());
    // kept when the index is made again
    index(&s);
    assert!(ahead::take(&s.paths, &later("h3", "12:00"), at(DAY, "11:00"), ahead::WINDOW).unwrap().is_empty());
}

#[test]
fn not_this_takes_the_request_away() {
    let s = world();
    let e = event("w1", at(DAY, "10:00"), "회의", &["choi@other.co.kr"]);
    let a = ahead::take(&s.paths, &ask(vec![e]), at(DAY, "09:00"), ahead::WINDOW).unwrap().remove(0);
    let (code, r) = s.post_json("/corrections", &json!({"wrong": a.wrong, "right": "-", "ref": a.edge}));
    assert_eq!(code, 200, "{r}");
    let e = event("w2", at(DAY, "11:00"), "회의", &["choi@other.co.kr"]);
    assert!(ahead::take(&s.paths, &ask(vec![e]), at(DAY, "10:00"), ahead::WINDOW).unwrap().is_empty());
}

#[test]
fn the_route_hands_the_phone_its_lines_in_epoch_milliseconds() {
    let s = world();
    let now = chrono::Local::now().naive_local();
    let start = now + TimeDelta::minutes(60);
    let ms = start.and_local_timezone(chrono::Local).unwrap().timestamp_millis();
    let (code, got) = post_ahead(
        &s,
        json!([{"key": "p1", "start": ms, "title": "회의", "attendees": ["park@client.co.kr"], "all_day": false}]),
    );
    assert_eq!(code, 200, "{got}");
    let items = got["items"].as_array().unwrap();
    // the night is the rule's own (tested with a fixed time above): this only checks the shape
    assert_eq!(items.len(), usize::from(!ahead::quiet(start - ahead::LEAD)), "{got}");
    if let Some(it) = items.first() {
        assert_eq!((it["event"].as_str(), it["start"].as_i64()), (Some("p1"), Some(ms)));
        assert_eq!(it["at"].as_i64().unwrap(), ms - 10 * 60_000);
        for k in ["title", "text", "question", "stop", "edge", "wrong", "matter"] {
            assert!(it[k].as_str().is_some_and(|v| !v.is_empty()), "{k}: {it}");
        }
    }
}

#[test]
fn the_route_needs_the_token_and_a_body_of_events() {
    let s = world();
    assert_eq!(s.call("POST", "/assistant/ahead", Some(b"{}"), &[], Some("wrong")).0, 401);
    assert_eq!(s.call("POST", "/assistant/ahead", Some(b"[1]"), &[], Some(TOKEN)).0, 400);
    assert_eq!(post_ahead(&s, json!([{"key": "k", "start": "soon"}])).0, 400);
    let (code, got) = s.post_json("/assistant/ahead", &json!({}));
    assert_eq!((code, got), (200, json!({"items": []})), "no events here or in the vault");
}

#[test]
fn nothing_is_said_before_an_event_of_many_people() {
    // a team or company event: the people with requests are only some of many, the event is not about them
    let s = world();
    let mut many: Vec<String> = (0..11).map(|i| format!("동료{i} <c{i}@mine.co.kr>")).collect();
    many.push("이담당 <lee@client.co.kr>".into());
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    let e = event("m", at(DAY, "10:00"), "전사 행사", &refs);
    assert!(ahead::take(&s.paths, &ask(vec![e]), at(DAY, "09:55"), ahead::WINDOW).unwrap().is_empty());
}
