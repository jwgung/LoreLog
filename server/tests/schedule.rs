//! The schedule (#62): the events the vault holds (CalDAV's `Events/` notes and the phone's calendar, #77) from a day
//! to a day, each with the assistant's notes made from templates, never by the model: a request someone of the event,
//! or their organization, made and the user has not answered (as #60 judges it), how the same matter last stood (a
//! guess marked 추정 with its words, #64), the last record with those people and the open to-dos of the matter.
//! Nothing is noted when nothing is certain. Tapping an event opens the chat with a question about it.
mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDate;
use common::{FakeModel, Server, TOKEN, ms};
use life_server::assistant::context::subj_key;
use life_server::assistant::schedule::{self, Item};
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

fn event_note(s: &Server, start: &str, end: &str, title: &str, place: &str, attendees: &[&str]) {
    let all_day = start.len() == 10;
    let mut front = format!(
        "---\ntype: event\ndav_uid: {title}-{start}\ndav_source: cal\ndav_state: live\ntitle: {title}\nstart: {start}\nend: {end}\nall_day: {all_day}\nlocation: \"{place}\"\n"
    );
    if !attendees.is_empty() {
        front.push_str("attendees:\n");
        for a in attendees {
            front.push_str(&format!("  - {a}\n"));
        }
    }
    let path = s.paths.vault.join("Events").join(format!("{} {title}.md", &start[..10]));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("{front}---\n%% dav:begin %%\n")).unwrap();
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
        } else if text.contains("설치 일정 확정") {
            json!([["설치 사진:case", "state", "2026-03-26", "일정 확정", "2026-03-26", "설치 일정 확정"]])
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

/// client.co.kr: one request answered (측정 자료), one not (설치 사진, its matter's state and its open to-do);
/// other.co.kr: one not answered. The week of 3월 27일 has events with them, with no one known and with no one.
fn world() -> Server {
    let s = Server::start();
    let park = "박고객 <park@client.co.kr>";
    let lee = "이담당 <lee@client.co.kr>";
    mail(&s, "a1.md", "2026-03-20", park, ME, "측정 자료", "측정 자료 요청드립니다", false, "[]");
    mail(&s, "a2.md", "2026-03-22", ME, park, "RE: 측정 자료", "보내드립니다", true, "[<a1.md@x>]");
    mail(&s, "b1.md", "2026-03-25", lee, ME, "설치 사진", "설치 사진 보내 주세요", false, "[]");
    mail(&s, "b2.md", "2026-03-26", lee, ME, "RE: 설치 사진", "설치 일정 확정했습니다", false, "[<b1.md@x>]");
    mail(&s, "c1.md", "2026-03-26", "최대리 <choi@other.co.kr>", ME, "견적 문의", "견적서 보내 주세요", false, "[]");
    event_note(&s, "2026-03-27T10:00:00", "2026-03-27T11:00:00", "설치 사진", "현장 2층", &[lee]);
    event_note(&s, "2026-03-28T14:00:00", "2026-03-28T15:00:00", "주간 회의", "", &["모르는 <nobody@nowhere.kr>"]);
    event_note(&s, "2026-03-29", "2026-03-29", "휴가", "", &[]);
    event_note(&s, "2026-03-28T09:00:00", "2026-03-28T09:30:00", "아침 회의", "", &[]);
    event_note(&s, "2026-04-05T10:00:00", "2026-04-05T11:00:00", "다음 달", "", &[lee]);
    // the phone's calendar
    let start = ms(2026, 3, 30, 10, 0);
    let rec = json!({"type": "calendar_event", "ts": start - 86_400_000, "key": "9@1", "uid": "", "title": "견적 협의",
                     "start": start, "end": start + 3_600_000, "all_day": false, "location": "",
                     "attendees": ["최대리 <choi@other.co.kr>"], "status": "upsert"});
    let dir = s.paths.raw().join("lorelog");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("2026-03-20.ndjson"), format!("{rec}\n")).unwrap();
    let mut lib = Librarian::open(&s.paths, None, Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    graph::extract(&mut lib, reader().as_ref(), &graph::Run { limit: 50, workers: 1 }).unwrap();
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

fn day(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn week(s: &Server) -> Vec<Item> {
    schedule::between(&s.paths, day("2026-03-27"), day("2026-04-02"), day("2026-03-27")).unwrap()
}

fn notes(it: &Item) -> Vec<(&str, &str, bool)> {
    it.notes.iter().map(|n| (n.kind, n.text.as_str(), n.guess)).collect()
}

#[test]
fn the_events_of_the_days_asked_in_order_with_their_time_place_and_people() {
    let s = world();
    let got = week(&s);
    let seen: Vec<(&str, &str, &str, bool)> =
        got.iter().map(|i| (i.day.as_str(), i.start.as_str(), i.title.as_str(), i.all_day)).collect();
    assert_eq!(
        seen,
        vec![
            ("2026-03-27", "10:00", "설치 사진", false),
            ("2026-03-28", "09:00", "아침 회의", false),
            ("2026-03-28", "14:00", "주간 회의", false),
            ("2026-03-29", "", "휴가", true),
            ("2026-03-30", "10:00", "견적 협의", false),
        ],
        "{got:?}"
    );
    let first = &got[0];
    assert_eq!((first.end.as_str(), first.place.as_str()), ("11:00", "현장 2층"));
    assert_eq!(first.attendees, vec!["이담당".to_string()]);
    assert_eq!(got[2].attendees, vec!["모르는".to_string()]);
}

#[test]
fn an_event_gets_the_notes_of_what_is_certain_about_its_people_and_matter() {
    let s = world();
    let got = week(&s);
    assert_eq!(
        notes(&got[0]),
        vec![
            ("request", "이담당님이 3월 25일에 요청한 「설치 사진」에 아직 답하지 않았어요.", false),
            ("state", "추정 마지막 상태: 일정 확정 (3월 26일, 근거 \"설치 일정 확정\")", true),
            ("last", "이담당 마지막 기록: 3월 26일 받은 메일 「RE: 설치 사진」", false),
            ("todo", "할 일: 설치 사진 보내기", false),
        ],
        "{:?}",
        got[0]
    );
    assert_eq!(got[0].question, "이담당님이 요청한 「설치 사진」 건 어떻게 됐지?");
    // the phone's event, with someone of another organization
    assert_eq!(
        notes(&got[4]),
        vec![
            ("request", "최대리님이 3월 26일에 요청한 「견적서」에 아직 답하지 않았어요.", false),
            ("last", "최대리 마지막 기록: 3월 26일 받은 메일 「견적 문의」", false),
        ],
        "{:?}",
        got[4]
    );
}

#[test]
fn nothing_is_noted_when_nothing_is_certain() {
    let s = world();
    let got = week(&s);
    for it in &got[1..4] {
        assert!(it.notes.is_empty(), "{it:?}");
    }
    // tapped, it still opens the chat about it
    assert_eq!(got[1].question, "3월 28일 「아침 회의」 일정, 관련된 지난 기록과 남은 일 알려 줘");
}

#[test]
fn the_last_record_is_what_was_before_the_event_not_a_later_one() {
    let s = world();
    // asked about from the day before the requests were made: only what was by then
    let got = schedule::between(&s.paths, day("2026-03-27"), day("2026-03-27"), day("2026-03-25")).unwrap();
    let last: Vec<&str> = got[0].notes.iter().filter(|n| n.kind == "last").map(|n| n.text.as_str()).collect();
    assert_eq!(last, vec!["이담당 마지막 기록: 3월 25일 받은 메일 「설치 사진」"]);
    assert!(!got[0].notes.iter().any(|n| n.kind == "state"), "the state came later: {:?}", got[0]);
}

#[test]
fn an_event_two_calendars_hold_is_listed_once() {
    let s = world();
    let note = s.paths.vault.join("Events").join("2026-03-28 아침 회의 (개인).md");
    fs::write(
        note,
        "---\ntype: event\ndav_uid: other-uid\ndav_source: personal\ndav_state: live\ntitle: 아침 회의\nstart: 2026-03-28T09:00:00\nend: 2026-03-28T09:30:00\nall_day: false\n---\n%% dav:begin %%\n",
    )
    .unwrap();
    let mut lib = Librarian::open(&s.paths, None, Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    let titles: Vec<String> = week(&s).into_iter().map(|i| i.title).collect();
    assert_eq!(titles.iter().filter(|t| *t == "아침 회의").count(), 1, "{titles:?}");
}

#[test]
fn an_answered_request_is_not_noted() {
    let s = world();
    // the user answered 설치 사진
    mail(&s, "b3.md", "2026-03-27", ME, "이담당 <lee@client.co.kr>", "RE: 설치 사진", "보냅니다", true, "[<b2.md@x>]");
    let mut lib = Librarian::open(&s.paths, None, Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    graph::extract(&mut lib, reader().as_ref(), &graph::Run { limit: 50, workers: 1 }).unwrap();
    let got = week(&s);
    assert!(!got[0].notes.iter().any(|n| n.kind == "request"), "{:?}", got[0]);
}

fn get(s: &Server, q: &str) -> (u16, Value) {
    s.get(&format!("/schedule{q}"))
}

#[test]
fn the_route_hands_the_days_asked_with_their_notes() {
    let s = world();
    let (code, got) = get(&s, "?from=2026-03-27&to=2026-04-02");
    assert_eq!(code, 200, "{got}");
    assert_eq!((got["from"].as_str(), got["to"].as_str()), (Some("2026-03-27"), Some("2026-04-02")));
    let items = got["items"].as_array().unwrap();
    assert_eq!(items.len(), 5, "{got}");
    let it = &items[0];
    for k in ["key", "day", "start", "end", "title", "place", "question"] {
        assert!(it[k].is_string(), "{k}: {it}");
    }
    assert_eq!(it["all_day"], false);
    assert_eq!(it["attendees"], json!(["이담당"]));
    assert_eq!(it["notes"][1]["kind"], "state");
    assert_eq!(it["notes"][1]["guess"], true);
    assert!(items[1]["notes"].as_array().unwrap().is_empty());
}

#[test]
fn the_route_asks_for_a_week_from_today_by_default_and_refuses_bad_days() {
    let s = world();
    let (code, got) = get(&s, "");
    assert_eq!(code, 200, "{got}");
    let today = chrono::Local::now().date_naive();
    assert_eq!(got["from"], today.format("%Y-%m-%d").to_string());
    assert_eq!(got["to"], (today + chrono::Days::new(6)).format("%Y-%m-%d").to_string());
    assert_eq!(get(&s, "?from=2026-04-02&to=2026-03-27").0, 400, "backwards");
    assert_eq!(get(&s, "?from=2026-02-30&to=2026-03-02").0, 400, "no such day");
    assert_eq!(get(&s, "?from=2026-01-01&to=2026-03-01").0, 400, "longer than a month");
    assert_eq!(s.call("GET", "/schedule", None, &[], Some("wrong")).0, 401);
    assert_eq!(s.call("GET", "/schedule?from=2026-03-27&to=2026-03-28", None, &[], Some(TOKEN)).0, 200);
}

#[test]
fn an_index_not_made_yet_is_said_so() {
    let s = Server::start();
    let (code, got) = get(&s, "?from=2026-03-27&to=2026-03-28");
    assert_eq!(code, 503, "{got}");
    assert_eq!(got["error"], "index not ready");
}

#[test]
fn an_all_day_event_or_one_of_many_people_gets_no_notes() {
    // a day off on a team calendar, a company event: their people's requests are not about them
    let s = world();
    let lee = "이담당 <lee@client.co.kr>";
    event_note(&s, "2026-03-31", "2026-03-31", "이담당 연차", "", &[lee]);
    let mut many: Vec<String> = (0..11).map(|i| format!("동료{i} <c{i}@mine.co.kr>")).collect();
    many.push(lee.into());
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    event_note(&s, "2026-04-01T17:00:00", "2026-04-01T19:00:00", "전사 행사", "", &refs);
    let mut lib = Librarian::open(&s.paths, None, Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    let got = week(&s);
    for title in ["이담당 연차", "전사 행사"] {
        let it = got.iter().find(|i| i.title == title).unwrap_or_else(|| panic!("{title}: {got:?}"));
        assert!(it.notes.is_empty(), "{it:?}");
    }
    assert!(!got[0].notes.is_empty(), "a meeting with the person still gets them");
}
