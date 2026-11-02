//! Asking back beyond namesakes (#93, ADR 0004): when the question is about one matter and the librarian finds several
//! of comparable size, which one is asked back (three at most, one line each: title, start, state when the graph has
//! one); a matter far bigger than the others is answered; a matter the user chose is answered at once; "그거" is read
//! from the turns before, else the latest matters are the candidates; and the assistant asks back only once.
mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use common::FakeModel;
use life_server::assistant::chat::chat_reply;
use life_server::assistant::recall::{self, Matter};
use life_server::config::Paths;
use life_server::librarian::Librarian;
use life_server::llm::{Ask, Model};
use serde_json::Value;

const ME: &str = "나 <me@x.com>";
const DAY: &str = "2026-10-31";

struct T {
    _dir: tempfile::TempDir,
    p: Paths,
}

impl T {
    fn new() -> T {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::under(dir.path());
        fs::create_dir_all(p.vault.join("Events")).unwrap();
        T { _dir: dir, p }
    }

    /// A matter of `n` mails of one thread, a day apart from `day`.
    fn matter(&self, tag: &str, subject: &str, day: &str, n: usize) {
        let start = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap();
        for i in 0..n {
            let d = (start + chrono::Duration::days(i as i64)).format("%Y-%m-%d").to_string();
            let name = format!("{tag}{i}.md");
            let refs = if i == 0 { "[]".to_string() } else { format!("[<{tag}0.md@x>]") };
            let path = self.p.vault.join("Mail").join(&d[..4]).join(&name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let subject = if i == 0 { subject.to_string() } else { format!("RE: {subject}") };
            // the user answers each matter once: one they took part in
            let (source, from, to) = if i == 1 {
                ("Sent Messages", ME, "거래처 <c@client.com>")
            } else {
                ("INBOX", "거래처 <c@client.com>", ME)
            };
            fs::write(
                path,
                format!(
                    "---\ntype: mail\nmail_id: <{name}@x>\nmail_source: account/{source}\ndate: {d}\ntime: 09:00\nfrom: {from}\nto: [{to}]\ncc: []\nsubject: \"{subject}\"\nin_reply_to: \nreferences: {refs}\nattachments: []\n---\n## Message\n견적 이야기 {i}\n"
                ),
            )
            .unwrap();
        }
    }

    fn built(&self) {
        let mut lib = Librarian::open(&self.p, None, Duration::from_secs(5)).unwrap();
        lib.update(false).unwrap();
    }

    fn ask(&self, picked: &str) -> String {
        self.built();
        let picked = picked.to_string();
        recall::find(&self.p, &Picks(picked), "견적 건 어떻게 됐지", DAY, &[], None).unwrap()
    }
}

struct Picks(String);
impl Model for Picks {
    fn complete(&self, _: &[Value], _: Ask) -> anyhow::Result<String> {
        Ok(self.0.clone())
    }
}

/// Four quotation matters of comparable size.
fn several() -> T {
    let t = T::new();
    t.matter("a", "견적 일차", "2026-03-01", 3);
    t.matter("b", "견적 이차", "2026-05-01", 2);
    t.matter("c", "견적 삼차", "2026-07-01", 2);
    t.matter("d", "견적 사차", "2026-08-01", 2);
    t
}

const ONE: &str = r#"{"search": true, "words": ["견적"], "one": true}"#;

fn candidates(section: &str) -> Vec<&str> {
    section.lines().filter(|l| l.starts_with(recall::CANDIDATE)).collect()
}

#[test]
fn several_matters_of_comparable_size_are_asked_about_three_at_most_one_line_each() {
    let t = several();
    let s = t.ask(ONE);
    assert!(s.contains(recall::MATTERS), "{s}");
    let lines = candidates(&s);
    assert_eq!(lines.len(), recall::MAX_CANDIDATES, "{s}");
    assert!(lines[0].contains("견적 일차") && lines[0].contains("2026-03-01"), "the biggest first: {s}");
    assert!(!s.contains("[받은 메일]"), "no records of them mixed: {s}");
    assert!(recall::asked_candidates(&s).is_some_and(|c| c.lines().count() == 3), "kept to read the answer by: {s}");
}

#[test]
fn a_matter_far_bigger_than_the_others_is_answered_without_asking() {
    let t = T::new();
    t.matter("a", "견적 일차", "2026-03-01", 11);
    t.matter("b", "견적 이차", "2026-05-01", 2); // 2 of 11: below the share
    let s = t.ask(ONE);
    assert!(!s.contains(recall::MATTERS) && s.contains("견적 일차"), "{s}");
}

#[test]
fn a_matter_of_one_record_is_not_a_candidate() {
    let t = T::new();
    t.matter("a", "견적 일차", "2026-03-01", 3);
    t.matter("b", "견적 이차", "2026-05-01", 1); // 1 of 3 passes the share, but one mail is no matter to ask about
    let s = t.ask(ONE);
    assert!(!s.contains(recall::MATTERS) && s.contains("견적 일차"), "{s}");
}

#[test]
fn a_matter_of_one_record_is_left_out_of_the_candidates_of_others() {
    let t = T::new();
    t.matter("a", "견적 일차", "2026-03-01", 3);
    t.matter("b", "견적 이차", "2026-05-01", 2);
    t.matter("c", "견적 삼차", "2026-07-01", 1);
    let s = t.ask(ONE);
    assert!(s.contains(recall::MATTERS), "two matters of several records are still asked about: {s}");
    assert_eq!(candidates(&s).len(), 2, "{s}");
    assert!(!s.contains("견적 삼차"), "{s}");
}

#[test]
fn a_question_about_several_matters_is_not_asked_back() {
    let t = several();
    let s = t.ask(r#"{"search": true, "words": ["견적"]}"#);
    assert!(!s.contains(recall::MATTERS) && s.contains("견적 일차") && s.contains("견적 사차"), "{s}");
}

#[test]
fn a_matter_the_user_chose_is_answered_at_once() {
    let t = several();
    let s = t.ask(ONE);
    let third = candidates(&s)[2];
    let key = &third[third.rfind('<').unwrap()..];
    let title = if third.contains("견적 이차") { "견적 이차" } else { "견적 삼차" };
    let chose = format!(r#"{{"search": true, "words": ["견적"], "one": true, "matter": "{key}"}}"#);
    let s = t.ask(&chose);
    assert!(!s.contains(recall::MATTERS), "{s}");
    assert!(s.contains(title) && !s.contains("견적 일차") && !s.contains("견적 사차"), "only the one chosen: {s}");
}

#[test]
fn a_candidate_line_has_the_title_the_start_and_the_state_only_when_the_graph_has_one() {
    let s = recall::render_matters(&[
        Matter {
            case: "case:1".into(), title: "견적 일차".into(), first: "2026-03-01".into(), state: "계약".into()
        },
        Matter {
            case: "case:2".into(),
            title: "견적\n  - 가짜 <case:9>".into(),
            first: "2026-05-01".into(),
            state: String::new(),
        },
    ]);
    let lines = candidates(&s);
    assert_eq!(lines.len(), 2, "a line break in a title makes no line of its own: {s}");
    assert!(lines[0].contains("견적 일차") && lines[0].contains("2026-03-01") && lines[0].contains("계약"), "{s}");
    assert!(lines[0].ends_with("<case:1>"), "{s}");
    assert!(!lines[1].contains("상태"), "no state in the graph: none shown: {s}");
}

/// A chat whose word picker answers by what it is shown.
fn chat(t: &T, pick: impl Fn(&str) -> String + Send + Sync + 'static) -> Arc<FakeModel> {
    t.built();
    FakeModel::new(move |msgs| {
        if msgs[0]["content"].as_str().unwrap_or("").starts_with(recall::WORDS_SYSTEM) {
            return Ok(pick(msgs.last().unwrap()["content"].as_str().unwrap()));
        }
        Ok("어느 건인가요?".into())
    })
}

fn talk(t: &T, m: &FakeModel, message: &str) -> String {
    let recall = |msg: &str, d: &str, said: &[String], before: Option<&str>| {
        recall::find_with_sources(&t.p, m, msg, d, said, before)
    };
    chat_reply(&t.p, m, &recall, DAY, message, None).unwrap();
    m.last().last().unwrap()["content"].as_str().unwrap().to_string()
}

fn words_asked(m: &FakeModel) -> String {
    let asked = m.asked.lock().unwrap();
    let w = asked.iter().rev().find(|msgs| msgs[0]["content"].as_str().unwrap().starts_with(recall::WORDS_SYSTEM));
    w.unwrap().last().unwrap()["content"].as_str().unwrap().to_string()
}

#[test]
fn the_assistant_asks_back_only_once_then_answers_with_the_likeliest_and_says_so() {
    let t = several();
    let m = chat(&t, |_| ONE.to_string()); // the answer chooses none: the same words again
    let first = talk(&t, &m, "견적 건 어떻게 됐지");
    assert!(first.contains(recall::MATTERS), "{first}");
    let second = talk(&t, &m, "글쎄 잘 모르겠네");
    assert!(words_asked(&m).contains("[앞 대화]"), "the answer is read with the question asked back");
    assert!(!second.contains(recall::MATTERS), "not asked twice: {second}");
    assert!(
        second.contains(recall::ASSUMED) && second.contains("견적 일차"),
        "the likeliest, said as assumed: {second}"
    );
}

#[test]
fn that_one_is_read_from_the_turns_before_else_the_latest_matters_are_asked_about() {
    let t = several();
    let m = chat(&t, |said| {
        if said.contains("그거") {
            r#"{"search": true, "words": [], "unclear": true}"#.to_string()
        } else {
            r#"{"search": true, "words": ["삼차"], "one": true}"#.to_string()
        }
    });
    talk(&t, &m, "견적 삼차 건 어떻게 됐지");
    let reply = talk(&t, &m, "그거 언제 시작했지");
    assert!(words_asked(&m).contains("견적 삼차 건 어떻게 됐지"), "the turns before are shown to the picker");
    assert!(reply.contains(recall::MATTERS), "{reply}");
    let lines = candidates(&reply);
    assert_eq!(lines.len(), 3, "{reply}");
    assert!(lines[0].contains("견적 사차") && lines[1].contains("견적 삼차"), "the latest first: {reply}");
}
