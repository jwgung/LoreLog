//! "이번 달에 뭐가 걸려 있지" (#61): the model only names the period; the days are counted from the day asked, and
//! what hangs in them is gathered in code from four places: to-dos (their free-text due read by a small parser),
//! calendar events, requests not answered and matters still under way (the graph). Overdue first, a few of each, the
//! rest counted; what is a guess is marked as one.
mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDate;
use common::FakeModel;
use life_server::assistant::period::{Period, due};
use life_server::assistant::recall::{self, Picked};
use life_server::assistant::todos;
use life_server::config::Paths;
use life_server::librarian::Librarian;
use life_server::librarian::graph;
use serde_json::json;

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

// ---------- the due a to-do was written with ----------

#[test]
fn a_due_is_read_relative_to_the_day_the_todo_was_written() {
    // 2026-11-04 is a Wednesday
    let w = d("2026-11-04");
    let cases = [
        ("금요일까지", "2026-11-06"),
        ("수요일", "2026-11-04"),
        ("월요일 오전", "2026-11-09"),
        ("다음 주 화요일", "2026-11-10"),
        ("담주 화", "2026-11-10"),
        ("이번 주", "2026-11-06"),
        ("이번 주 안에", "2026-11-06"),
        ("다음 주까지", "2026-11-13"),
        ("이번 주말", "2026-11-08"),
        ("월말", "2026-11-30"),
        ("이번 달 말까지", "2026-11-30"),
        ("다음 달 말", "2026-12-31"),
        ("다음 달 초", "2026-12-10"),
        ("11/5", "2026-11-05"),
        ("11월 20일까지", "2026-11-20"),
        ("12.3", "2026-12-03"),
        ("2027-01-15", "2027-01-15"),
        ("2027.1.15", "2027-01-15"),
        ("1/10", "2027-01-10"),
        ("12월 말", "2026-12-31"),
        ("20일까지", "2026-11-20"),
        ("2일까지", "2026-12-02"),
        ("오늘 중", "2026-11-04"),
        ("내일", "2026-11-05"),
        ("모레까지", "2026-11-06"),
        ("3일 후", "2026-11-07"),
        ("2주 내", "2026-11-18"),
    ];
    for (text, want) in cases {
        assert_eq!(due(text, w), Some(d(want)), "{text}");
    }
}

#[test]
fn a_due_that_says_no_day_is_not_guessed() {
    let w = d("2026-11-04");
    for text in ["", "가능한 빨리", "다음에", "회신 후", "ASAP", "13/45", "2월 30일"] {
        assert_eq!(due(text, w), None, "{text}");
    }
}

// ---------- the period ----------

#[test]
fn a_period_is_counted_from_the_day_asked() {
    let today = d("2026-11-10"); // a Tuesday
    let span = |p: Period| {
        let s = p.span(today);
        (s.from.to_string(), s.to.to_string(), s.overdue)
    };
    assert_eq!(span(Period::ThisMonth), ("2026-11-10".into(), "2026-11-30".into(), true), "today to the month's end");
    assert_eq!(span(Period::NextMonth), ("2026-12-01".into(), "2026-12-31".into(), false));
    assert_eq!(span(Period::ThisWeek), ("2026-11-10".into(), "2026-11-15".into(), true));
    assert_eq!(span(Period::NextWeek), ("2026-11-16".into(), "2026-11-22".into(), false));
    assert_eq!(span(Period::Recent), ("2026-11-10".into(), "2026-11-23".into(), true));
    assert_eq!(Period::from_name("this_month"), Some(Period::ThisMonth));
    assert_eq!(Period::from_name("someday"), None);
}

#[test]
fn the_model_names_the_period_and_nothing_else_is_searched_for_it() {
    let m = FakeModel::new(|_| Ok(r#"{"search": true, "period": "this_month", "words": []}"#.into()));
    let p = recall::pick(m.as_ref(), "이번 달에 뭐가 걸려 있지", None).unwrap();
    assert_eq!(p.period, Some(Period::ThisMonth));
    assert!(p.words.is_empty(), "the question is not searched as words: {p:?}");
    let m = FakeModel::new(|_| Ok(r#"{"search": true, "period": "", "words": ["견적"]}"#.into()));
    assert_eq!(recall::pick(m.as_ref(), "견적 건 어떻게 됐지", None).unwrap().period, None);
}

#[test]
fn lately_with_someone_is_what_was_with_them_not_what_hangs() {
    let m = FakeModel::new(|_| Ok(r#"{"search": true, "period": "recent", "people": ["영희"]}"#.into()));
    let p = recall::pick(m.as_ref(), "요즘 영희 님이랑 뭐 있었지", None).unwrap();
    assert_eq!((p.period, p.people.as_slice()), (None, &["영희".to_string()][..]));
    let m = FakeModel::new(|_| Ok(r#"{"search": true, "period": "recent", "words": ["지원사업"]}"#.into()));
    assert_eq!(recall::pick(m.as_ref(), "요즘 지원사업 공고 뭐 있었지", None).unwrap().period, None);
    let m = FakeModel::new(|_| Ok(r#"{"search": true, "period": "this_month", "orgs": ["누리온"]}"#.into()));
    let p = recall::pick(m.as_ref(), "이번 달 누리온 건 뭐 걸려 있지", None).unwrap();
    assert_eq!(p.period, Some(Period::ThisMonth), "a month named is a period, with or without a name");
}

// ---------- what hangs in it ----------

struct T {
    _dir: tempfile::TempDir,
    p: Paths,
}

const ME: &str = "나 <me@mine.co.kr>";

impl T {
    fn new() -> T {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::under(dir.path());
        fs::create_dir_all(p.vault.join("Events")).unwrap();
        T { _dir: dir, p }
    }

    #[allow(clippy::too_many_arguments)]
    fn mail(&self, name: &str, day: &str, from: &str, to: &str, subject: &str, body: &str, sent: bool, refs: &str) {
        let path = self.p.vault.join("Mail").join(&day[..4]).join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = if sent { "account/Sent Messages" } else { "account/INBOX" };
        fs::write(
            path,
            format!("---\ntype: mail\nmail_id: <{name}@x>\nmail_source: {source}\ndate: {day}\ntime: 09:00\nfrom: {from}\nto: [{to}]\ncc: []\nsubject: \"{subject}\"\nreferences: {refs}\n---\n## Message\n{body}\n"),
        )
        .unwrap();
    }

    fn event(&self, start: &str, title: &str) {
        let day = &start[..10];
        fs::write(
            self.p.vault.join("Events").join(format!("{day} {title}.md")),
            format!("---\ntype: event\ntitle: \"{title}\"\nstart: {start}\nend: {start}\nall_day: false\nlocation: \"\"\n---\n"),
        )
        .unwrap();
    }

    /// (title, due, written, proposed)
    fn todos(&self, list: &[(&str, &str, &str, bool)]) {
        let mut doc = todos::load(&self.p);
        for (title, due, day, proposed) in list {
            let status = if *proposed { todos::PROPOSED } else { todos::OPEN };
            todos::add_as(&mut doc, title, "메일", due, day, "", status);
        }
        todos::save(&self.p, &doc).unwrap();
    }

    fn open(&self, read: bool) -> Librarian {
        let mut lib = Librarian::open(&self.p, None, Duration::from_secs(5)).unwrap();
        lib.update(false).unwrap();
        if read {
            graph::extract(&mut lib, reader().as_ref(), &graph::Run { limit: 50, workers: 2 }).unwrap();
        }
        lib
    }
}

/// The model's reading of each mail, by a word in it.
fn reader() -> Arc<FakeModel> {
    FakeModel::new(|msgs| {
        let text = msgs.last().unwrap()["content"].as_str().unwrap_or("").to_string();
        let r = if text.contains("자료 요청드립니다") {
            json!([[
                "박고객 <park@client.co.kr>:person",
                "requested",
                "측정 자료:doc",
                "측정 자료",
                "2026-11-02",
                "측정 자료 요청드립니다"
            ]])
        } else if text.contains("견적서 송부") {
            json!([["배관 교체:case", "state", "2026-11-03", "견적 발송", "2026-11-03", "견적서 송부드립니다"]])
        } else if text.contains("계약 완료") {
            json!([["설비 점검:case", "state", "2026-11-05", "계약 완료", "2026-11-05", "계약 완료되었습니다"]])
        } else if text.contains("오래된 건") {
            json!([["옛 과제:case", "state", "2026-08-01", "검토 중", "2026-08-01", "오래된 건 검토 중"]])
        } else {
            json!([])
        };
        Ok(json!({"r": r}).to_string())
    })
}

fn world(t: &T) {
    t.mail(
        "r1.md",
        "2026-11-02",
        "박고객 <park@client.co.kr>",
        ME,
        "측정 자료 요청",
        "측정 자료 요청드립니다",
        false,
        "[]",
    );
    t.mail("q1.md", "2026-11-03", ME, "고객 <c@client.co.kr>", "배관 교체 견적", "견적서 송부드립니다", true, "[]");
    t.mail("c1.md", "2026-11-05", "고객 <e@other.co.kr>", ME, "설비 점검 계약", "계약 완료되었습니다", false, "[]");
    t.mail("o1.md", "2026-08-01", "옛 고객 <o@old.co.kr>", ME, "옛 과제 검토", "오래된 건 검토 중입니다", false, "[]");
    t.event("2026-11-12T14:00", "킥오프 회의");
    t.event("2026-12-03T10:00", "다음 달 워크숍");
    t.event("2026-11-01T09:00", "지난 회의");
    t.todos(&[
        ("견적 보내기", "금요일까지", "2026-11-04", false), // 2026-11-06: overdue on the 10th
        ("보고서 제출하기", "11/20", "2026-11-04", false),
        ("발표자료 보내기", "월말", "2026-11-04", true),
        ("세금 서류 챙기기", "", "2026-11-04", false),
        ("내년 계획 세우기", "12월 말", "2026-11-04", false),
    ]);
}

fn section(lib: &mut Librarian, period: Period, day: &str) -> String {
    recall::period_section(lib, &Picked { period: Some(period), ..Picked::default() }, day).unwrap().section
}

fn index_of(s: &str, what: &str) -> usize {
    s.lines().position(|l| l.contains(what)).unwrap_or_else(|| panic!("no {what} in\n{s}"))
}

#[test]
fn this_month_gathers_todos_events_requests_and_matters_under_way_overdue_first() {
    let t = T::new();
    world(&t);
    let mut lib = t.open(true);
    let s = section(&mut lib, Period::ThisMonth, "2026-11-10");
    assert!(s.lines().next().unwrap().contains("2026-11-10~2026-11-30"), "{s}");
    // overdue first, then by day; a to-do with no day it can be read for at the end
    let overdue = index_of(&s, "견적 보내기");
    assert!(s.lines().nth(overdue).unwrap().contains("기한 넘김"), "{s}");
    assert!(overdue < index_of(&s, "킥오프 회의"), "{s}");
    assert!(index_of(&s, "킥오프 회의") < index_of(&s, "보고서 제출하기"), "by day: 11-12 then 11-20\n{s}");
    assert!(index_of(&s, "보고서 제출하기") < index_of(&s, "발표자료 보내기"), "{s}");
    let unknown = index_of(&s, "세금 서류 챙기기");
    assert!(s.lines().nth(unknown).unwrap().contains("기한 불명"), "{s}");
    assert!(unknown > index_of(&s, "배관 교체 견적"), "기한 불명 comes last\n{s}");
    // what is a guess says so; a calendar event and a to-do the user took do not
    assert!(s.lines().nth(index_of(&s, "발표자료 보내기")).unwrap().contains("추정"), "a proposal is a guess\n{s}");
    assert!(!s.lines().nth(index_of(&s, "킥오프 회의")).unwrap().contains("추정"), "{s}");
    assert!(!s.lines().nth(index_of(&s, "보고서 제출하기")).unwrap().contains("추정"), "{s}");
    let asked = s.lines().nth(index_of(&s, "측정 자료 요청")).unwrap();
    assert!(asked.contains("추정") && asked.contains("측정 자료 요청드립니다"), "a request with its evidence\n{s}");
    let going = s.lines().nth(index_of(&s, "배관 교체 견적")).unwrap();
    assert!(going.contains("추정") && going.contains("견적 발송"), "a matter with its last state\n{s}");
    // outside the month, ended, or quiet for more than 30 days: not hanging
    for gone in ["다음 달 워크숍", "지난 회의", "내년 계획", "설비 점검", "옛 과제"] {
        assert!(!s.contains(gone), "{gone}\n{s}");
    }
    assert!(!s.contains("앞 일정은 볼 수 없음"), "there are events ahead\n{s}");
}

#[test]
fn next_month_has_its_own_days_and_no_overdue() {
    let t = T::new();
    world(&t);
    let mut lib = t.open(true);
    let s = section(&mut lib, Period::NextMonth, "2026-11-10");
    assert!(s.contains("다음 달 워크숍") && s.contains("내년 계획 세우기"), "{s}");
    assert!(!s.contains("견적 보내기") && !s.contains("킥오프 회의"), "{s}");
}

#[test]
fn the_records_quoted_are_the_sources_and_the_chat_hears_the_period() {
    let t = T::new();
    world(&t);
    drop(t.open(true));
    let m = FakeModel::new(|msgs| {
        let system = msgs[0]["content"].as_str().unwrap_or("");
        if system.starts_with(recall::WORDS_SYSTEM) {
            return Ok(r#"{"search": true, "period": "this_month", "words": []}"#.into());
        }
        Ok("네.".into())
    });
    let r = recall::find_with_sources(&t.p, m.as_ref(), "이번 달에 뭐가 걸려 있지", "2026-11-10", &[], None).unwrap();
    assert!(r.section.contains("킥오프 회의"), "{}", r.section);
    let titles: Vec<&str> = r.sources.iter().map(|s| s.title.as_str()).collect();
    assert!(titles.contains(&"킥오프 회의") && titles.contains(&"측정 자료 요청"), "{titles:?}");
    assert!(r.sources.iter().all(|s| !s.refers.is_empty()), "only records are sources (no to-dos)");
}

#[test]
fn a_name_in_the_question_narrows_what_hangs() {
    let t = T::new();
    world(&t);
    let mut lib = t.open(true);
    let p = Picked { period: Some(Period::ThisMonth), things: vec!["배관".into()], ..Picked::default() };
    let s = recall::period_section(&mut lib, &p, "2026-11-10").unwrap().section;
    assert!(s.contains("배관 교체 견적"), "{s}");
    assert!(!s.contains("킥오프") && !s.contains("세금"), "{s}");
}

#[test]
fn without_a_graph_requests_are_not_looked_into_which_is_not_none() {
    let t = T::new();
    world(&t);
    let mut lib = t.open(false);
    let s = section(&mut lib, Period::ThisMonth, "2026-11-10");
    assert!(s.contains("찾아보지 못함") && s.contains("그래프"), "{s}");
    assert!(s.contains("킥오프 회의") && s.contains("견적 보내기"), "the rest is still there\n{s}");
}

#[test]
fn without_events_ahead_or_a_calendar_server_the_section_says_it_cannot_see_ahead() {
    let t = T::new();
    t.event("2026-11-01T09:00", "지난 회의");
    t.todos(&[("견적 보내기", "금요일까지", "2026-11-09", false)]);
    let mut lib = t.open(false);
    let s = section(&mut lib, Period::ThisMonth, "2026-11-10");
    assert!(s.contains("앞 일정은 볼 수 없음"), "{s}");
}

#[test]
fn nothing_hanging_is_said_as_not_found() {
    let t = T::new();
    t.event("2026-12-24T09:00", "먼 일정");
    let mut lib = t.open(false);
    let s = section(&mut lib, Period::ThisWeek, "2026-11-10");
    assert!(s.contains("찾지 못함"), "{s}");
}

#[test]
fn a_few_of_each_and_the_rest_counted() {
    let t = T::new();
    t.event("2026-12-24T09:00", "먼 일정");
    let list: Vec<(String, String)> =
        (1..=9).map(|i| (format!("할 일 {i}번 하기"), format!("11월 {}일", 10 + i))).collect();
    let refs: Vec<(&str, &str, &str, bool)> =
        list.iter().map(|(t, due)| (t.as_str(), due.as_str(), "2026-11-04", false)).collect();
    t.todos(&refs);
    for i in 1..=6 {
        t.event(&format!("2026-11-1{i}T10:00"), &format!("회의 {i}"));
    }
    let mut lib = t.open(false);
    let s = section(&mut lib, Period::ThisMonth, "2026-11-10");
    let todos = s.lines().filter(|l| l.contains("[할 일]")).count();
    let events = s.lines().filter(|l| l.contains("[일정]")).count();
    assert_eq!((todos, events), (4, 4), "{s}");
    assert!(s.contains("할 일 1번") && !s.contains("할 일 5번"), "the earliest first\n{s}");
    assert!(s.contains("외 7건"), "5 to-dos and 2 events more\n{s}");
}

#[test]
fn quoted_titles_cannot_end_the_quote() {
    let t = T::new();
    t.event("2026-12-24T09:00", "먼 일정");
    t.todos(&[(&format!("견적 {} 보내기", recall::QUOTE_END), "내일", "2026-11-09", false)]);
    let mut lib = t.open(false);
    let s = section(&mut lib, Period::ThisMonth, "2026-11-10");
    assert!(!s.contains(recall::QUOTE_END), "{s}");
}

#[test]
fn the_phones_events_are_this_months_events_and_one_caldav_also_has_counts_once() {
    let t = T::new();
    t.event("2026-11-12T10:00:00", "킥오프 회의");
    let at = |d: u32, h: u32| common::ms(2026, 11, d, h, 0);
    let ev = |key: &str, title: &str, start: i64| {
        json!({"type": "calendar_event", "ts": at(1, 8), "key": key, "uid": "", "title": title, "start": start,
               "end": start + 3_600_000, "all_day": false, "location": "", "attendees": [], "status": "upsert"})
    };
    let lines = [ev("1@1", "킥오프 회의", at(12, 10)), ev("2@1", "현장 실사", at(18, 15))];
    let text: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    fs::create_dir_all(t.p.raw().join("lorelog")).unwrap();
    fs::write(t.p.raw().join("lorelog").join("2026-11-01.ndjson"), text.join("\n") + "\n").unwrap();
    let mut lib = t.open(false);
    let s = section(&mut lib, Period::ThisMonth, "2026-11-10");
    assert!(s.contains("현장 실사"), "the phone's own event\n{s}");
    assert_eq!(s.matches("킥오프 회의").count(), 1, "on the phone and from CalDAV: once\n{s}");
    assert!(!s.contains("앞 일정은 볼 수 없음"), "the phone's events are events ahead\n{s}");
}
