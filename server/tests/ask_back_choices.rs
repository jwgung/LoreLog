//! Asking back on the screen (#108, map #91): when the librarian asks which matter or person was meant, the chat's
//! answer carries the candidates as choices (one label each; the matter under way is one, answered yes or no), the
//! day's chat shows them again only on the last turn while it is unanswered, and a choice comes back by its position
//! (never by the librarian's key) or as "no". A position picks that candidate whatever the picker reads; "no" (pressed
//! or typed) drops what was offered and searches the question's topic, or asks for a word when it has none, and is
//! never asked back again.
mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use common::{FakeModel, Server};
use life_server::assistant::chat::{Answer, Choice, chat_answer, chat_history};
use life_server::assistant::recall::{self, ChoiceKind};
use life_server::config::Paths;
use life_server::librarian::Librarian;
use serde_json::{Value, json};

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

    fn mail(&self, name: &str, day: &str, from: &str, to: &str, subject: &str, refs: &str) {
        let path = self.p.vault.join("Mail").join(&day[..4]).join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = if from == ME { "account/Sent Messages" } else { "account/INBOX" };
        fs::write(
            path,
            format!(
                "---\ntype: mail\nmail_id: <{name}@x>\nmail_source: {source}\ndate: {day}\ntime: 09:00\nfrom: {from}\nto: [{to}]\ncc: []\nsubject: \"{subject}\"\nin_reply_to: \nreferences: {refs}\nattachments: []\n---\n## Message\n{subject} 이야기\n"
            ),
        )
        .unwrap();
    }

    /// A matter of `n` mails of one thread, a day apart from `day`; the user answers the second.
    fn matter(&self, tag: &str, subject: &str, day: &str, n: usize) {
        let start = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap();
        for i in 0..n {
            let d = (start + chrono::Duration::days(i as i64)).format("%Y-%m-%d").to_string();
            let (from, to) = if i == 1 { (ME, "거래처 <c@client.com>") } else { ("거래처 <c@client.com>", ME) };
            let (subject, refs) = if i == 0 {
                (subject.to_string(), "[]".to_string())
            } else {
                (format!("RE: {subject}"), format!("[<{tag}0.md@x>]"))
            };
            self.mail(&format!("{tag}{i}.md"), &d, from, to, &subject, &refs);
        }
    }

    fn built(&self) {
        let mut lib = Librarian::open(&self.p, None, Duration::from_secs(5)).unwrap();
        lib.update(false).unwrap();
    }
}

/// Four quotation matters of comparable size; the latest is 견적 사차.
fn several() -> T {
    let t = T::new();
    t.matter("a", "견적 일차", "2026-03-01", 3);
    t.matter("b", "견적 이차", "2026-05-01", 2);
    t.matter("c", "견적 삼차", "2026-07-01", 2);
    t.matter("d", "견적 사차", "2026-08-01", 2);
    t.built();
    t
}

/// Two people named 김영희 with as many records each.
fn namesakes() -> T {
    let t = T::new();
    t.mail("y1.md", "2026-05-01", "김영희 <yh@x.com>", ME, "회의록", "[]");
    t.mail("y2.md", "2026-05-02", ME, "김영희 <yh@x.com>", "RE: 회의록", "[<y1.md@x>]");
    t.mail("z1.md", "2026-06-01", "김영희 <yh2@other.com>", ME, "다른 회사 영희", "[]");
    t.mail("z2.md", "2026-06-02", "김영희 <yh2@other.com>", ME, "다른 회사 영희 둘째", "[]");
    // a quotation with someone else: neither 김영희 is on it
    t.mail("q1.md", "2026-06-10", "거래처 <c@client.com>", ME, "견적 요청", "[]");
    t.built();
    t
}

const ONE: &str = r#"{"search": true, "words": ["견적"], "one": true}"#;
const AT_HAND: &str = r#"{"search": true, "words": ["견적"], "precedent": true, "at_hand": true}"#;
const NAMES_NOTHING: &str = r#"{"search": true, "words": [], "precedent": true, "at_hand": true}"#;

/// A chat whose word picker answers by what it is shown.
fn chat(pick: impl Fn(&str) -> String + Send + Sync + 'static) -> Arc<FakeModel> {
    FakeModel::new(move |msgs| {
        if msgs[0]["content"].as_str().unwrap_or("").starts_with(recall::WORDS_SYSTEM) {
            return Ok(pick(msgs.last().unwrap()["content"].as_str().unwrap()));
        }
        Ok("어느 건인가요?".into())
    })
}

fn say(t: &T, m: &FakeModel, message: &str, choice: Option<Choice>) -> Answer {
    let recall = |msg: &str, d: &str, said: &[String], before: Option<&str>, chosen: Option<&recall::Chosen>| {
        recall::find_with_sources(&t.p, m, msg, d, said, before, chosen)
    };
    chat_answer(&t.p, m, &recall, DAY, message, None, choice).unwrap()
}

/// What the answer was written from: the user's turn with the librarian's section in front of it.
fn prompt(m: &FakeModel) -> String {
    m.last().last().unwrap()["content"].as_str().unwrap().to_string()
}

/// What the word picker was last shown.
fn picker_saw(m: &FakeModel) -> String {
    let asked = m.asked.lock().unwrap();
    let w = asked.iter().rev().find(|msgs| msgs[0]["content"].as_str().unwrap().starts_with(recall::WORDS_SYSTEM));
    w.unwrap().last().unwrap()["content"].as_str().unwrap().to_string()
}

// ---------- the choices on the screen ----------

#[test]
fn asking_back_about_matters_gives_one_choice_per_candidate_in_their_order() {
    let t = several();
    let m = chat(|_| ONE.to_string());
    let a = say(&t, &m, "견적 건 어떻게 됐지", None);
    let c = a.choices.expect("asked back");
    assert_eq!(c.kind, ChoiceKind::Matters);
    assert_eq!(c.options.len(), recall::MAX_CANDIDATES, "{c:?}");
    assert!(c.options[0].contains("견적 일차") && c.options[0].contains("2026-03-01"), "the biggest first: {c:?}");
    assert!(c.options.iter().all(|o| !o.contains("case:") && !o.contains('\n')), "no key, one line each: {c:?}");
}

#[test]
fn an_answer_that_asks_nothing_has_no_choices() {
    let t = several();
    let m = chat(|_| r#"{"search": false}"#.to_string());
    assert_eq!(say(&t, &m, "오늘 좀 피곤하네", None).choices, None);
}

#[test]
fn the_day_s_chat_shows_the_choices_again_only_on_the_last_turn_while_unanswered() {
    let t = several();
    let m = chat(|said| if said.contains("피곤") { r#"{"search": false}"#.into() } else { ONE.into() });
    say(&t, &m, "견적 건 어떻게 됐지", None);
    let h = chat_history(&t.p, DAY);
    let last = h.last().unwrap();
    assert_eq!(last["role"], "assistant");
    assert_eq!(last["choices"]["kind"], "matters", "{h:?}");
    assert_eq!(last["choices"]["options"].as_array().unwrap().len(), 3, "{h:?}");
    assert!(last.get("keys").is_none() && !last.to_string().contains("case:"), "the keys stay on the server: {h:?}");
    say(&t, &m, "오늘 좀 피곤하네", None);
    let h = chat_history(&t.p, DAY);
    assert!(h.iter().all(|l| l.get("choices").is_none()), "answered: no buttons again: {h:?}");
}

// ---------- a choice by position ----------

#[test]
fn a_choice_by_position_answers_that_matter_whatever_the_picker_reads() {
    let t = several();
    let m = chat(|_| ONE.to_string()); // the picker names no matter: the position alone tells which
    let asked = say(&t, &m, "견적 건 어떻게 됐지", None).choices.unwrap();
    let third = asked.options[2].clone();
    let title = if third.contains("견적 이차") { "견적 이차" } else { "견적 삼차" };
    let a = say(&t, &m, &third, Some(Choice::Pick(3)));
    let p = prompt(&m);
    assert!(!p.contains(recall::MATTERS) && !p.contains(recall::ASSUMED), "{p}");
    assert!(p.contains(title) && !p.contains("견적 일차") && !p.contains("견적 사차"), "only the one chosen: {p}");
    assert_eq!(a.choices, None);
    let h = chat_history(&t.p, DAY);
    assert_eq!(h[h.len() - 2]["text"], third.as_str(), "the label pressed is the user's line");
}

#[test]
fn a_position_beyond_the_candidates_is_read_as_words() {
    let t = several();
    let m = chat(|_| ONE.to_string());
    say(&t, &m, "견적 건 어떻게 됐지", None);
    say(&t, &m, "아무거나", Some(Choice::Pick(9)));
    assert!(prompt(&m).contains(recall::ASSUMED), "as an answer that chose none: {}", prompt(&m));
}

#[test]
fn a_choice_with_nothing_asked_before_is_read_as_words() {
    let t = several();
    let m = chat(|_| r#"{"search": true, "words": ["견적 일차"]}"#.to_string());
    say(&t, &m, "견적 일차", Some(Choice::Pick(1)));
    assert!(prompt(&m).contains("견적 일차"), "{}", prompt(&m));
}

#[test]
fn people_of_one_name_are_choices_with_what_tells_them_apart_and_a_position_picks_that_person() {
    let t = namesakes();
    let m = chat(|_| r#"{"search": true, "people": ["김영희"], "words": []}"#.to_string());
    let c = say(&t, &m, "영희 님이랑 뭐 있었지", None).choices.expect("asked back");
    assert_eq!(c.kind, ChoiceKind::People);
    assert_eq!(c.options.len(), 2, "{c:?}");
    assert!(c.options.iter().all(|o| o.contains("김영희") && !o.contains('@')), "a name, not an address: {c:?}");
    let other = c.options.iter().position(|o| o.contains("other.com")).expect("the company tells them apart") + 1;
    say(&t, &m, &c.options[other - 1], Some(Choice::Pick(other)));
    let p = prompt(&m);
    assert!(!p.contains(recall::NAMESAKES) && !p.contains(recall::ASSUMED), "{p}");
    assert!(p.contains("다른 회사 영희") && !p.contains("회의록"), "{p}");
}

// ---------- the matter under way, and "no" ----------

#[test]
fn the_matter_under_way_is_one_choice_to_answer_yes_or_no() {
    let t = several();
    let m = chat(|_| AT_HAND.to_string());
    let c = say(&t, &m, "이 견적 건 전에 어떻게 했지", None).choices.expect("asked back");
    assert_eq!(c.kind, ChoiceKind::Now);
    assert_eq!(c.options.len(), 1, "{c:?}");
    assert!(c.options[0].contains("견적 사차"), "the latest: {c:?}");
    // yes: that matter
    say(&t, &m, "네, 그 건", Some(Choice::Pick(1)));
    let p = prompt(&m);
    assert!(p.contains(recall::NOW_HIT) && p.contains("견적 사차") && !p.contains(recall::NOW), "{p}");
}

#[test]
fn no_to_the_matter_under_way_searches_the_question_s_topic_without_it() {
    let t = several();
    let m = chat(|_| AT_HAND.to_string());
    say(&t, &m, "이 견적 건 전에 어떻게 했지", None);
    let a = say(&t, &m, "아니요", Some(Choice::No));
    let saw = picker_saw(&m);
    assert!(saw.contains("이 견적 건 전에 어떻게 했지"), "the topic is read from the question: {saw}");
    assert!(!saw.contains("견적 사차"), "not from the candidate turned down: {saw}");
    let p = prompt(&m);
    assert!(!p.contains(recall::NOW) && !p.contains(recall::ASSUMED), "not asked again, nothing assumed: {p}");
    assert!(p.contains(recall::FOUND_HIT) && p.contains("견적 일차"), "found by the topic: {p}");
    assert!(!p.contains("견적 사차"), "the one turned down is left out: {p}");
    assert_eq!(a.choices, None);
}

#[test]
fn no_with_nothing_to_search_asks_for_a_word_and_does_not_ask_back_again() {
    let t = several();
    let m = chat(|_| NAMES_NOTHING.to_string());
    say(&t, &m, "이런 경우 전에 어떻게 했지", None);
    let a = say(&t, &m, "아니요", Some(Choice::No));
    let p = prompt(&m);
    assert!(p.contains(recall::REFUSED), "{p}");
    assert!(!p.contains(recall::NOW) && !p.lines().any(|l| l.starts_with(recall::CANDIDATE)), "{p}");
    assert_eq!(a.choices, None);
}

#[test]
fn a_typed_no_is_the_same_as_the_button() {
    let t = several();
    let m = chat(|_| NAMES_NOTHING.to_string());
    say(&t, &m, "이런 경우 전에 어떻게 했지", None);
    say(&t, &m, "아뇨", None);
    assert!(prompt(&m).contains(recall::REFUSED), "{}", prompt(&m));
}

#[test]
fn no_to_matters_offered_searches_the_others() {
    let t = several();
    let m = chat(|_| ONE.to_string());
    let c = say(&t, &m, "견적 건 어떻게 됐지", None).choices.unwrap();
    say(&t, &m, "아니요", Some(Choice::No));
    let p = prompt(&m);
    let saw = picker_saw(&m);
    assert!(!p.contains(recall::MATTERS) && !p.contains(recall::ASSUMED), "{p}");
    for o in &c.options {
        let title = o.split(" · ").next().unwrap();
        assert!(!p.contains(title), "{title} was turned down: {p}");
        assert!(!saw.contains(title), "{title} is not shown to the picker: {saw}");
    }
    assert!(p.contains("[받은 메일]") || p.contains("[보낸 메일]"), "the other one is found: {p}");
}

#[test]
fn a_typed_no_with_nothing_asked_before_is_read_as_words() {
    let t = several();
    let m = chat(|_| r#"{"search": false}"#.to_string());
    say(&t, &m, "아니요", None);
    assert!(!prompt(&m).contains(recall::REFUSED), "{}", prompt(&m));
    assert!(picker_saw(&m).ends_with("아니요"), "the message itself is read: {}", picker_saw(&m));
}

#[test]
fn a_label_shows_no_address_even_when_a_name_or_title_is_one() {
    let people = recall::Asked::People(vec![recall::Candidate {
        addr: "kim@x.com".into(),
        name: "kim@x.com".into(),
        company: String::new(),
        last: "2026-05-01".into(),
        subject: "s".into(),
    }]);
    let matters = recall::Asked::Matters(vec![recall::Matter {
        case: "case:1".into(),
        title: "견적 a@b.com 회신".into(),
        first: "2026-03-01".into(),
        state: String::new(),
    }]);
    for c in [people.choices(), matters.choices()] {
        assert!(c.options.iter().all(|o| !o.contains('@') && !o.contains("case:")), "{c:?}");
    }
}

#[test]
fn no_to_people_of_one_name_searches_the_rest_of_the_question_without_them() {
    let t = namesakes();
    // the topic does not tell the two apart: with the name kept, they would be asked about again
    let m = chat(|_| r#"{"search": true, "people": ["김영희"], "words": ["견적"]}"#.into());
    say(&t, &m, "영희 님이랑 견적 건 어떻게 됐지", None);
    assert!(prompt(&m).contains(recall::NAMESAKES), "{}", prompt(&m));
    // no: the name is dropped and the topic is searched
    say(&t, &m, "아니요", Some(Choice::No));
    let p = prompt(&m);
    assert!(!p.contains(recall::NAMESAKES) && !p.contains(recall::REFUSED), "{p}");
    assert!(p.contains("견적 요청"), "the topic is searched without the name: {p}");
}

#[test]
fn no_to_people_of_one_name_with_nothing_else_asked_for_a_word() {
    let t = namesakes();
    let m = chat(|_| r#"{"search": true, "people": ["김영희"], "words": []}"#.into());
    say(&t, &m, "영희 님이랑 뭐 있었지", None);
    say(&t, &m, "아니요", Some(Choice::No));
    let p = prompt(&m);
    assert!(p.contains(recall::REFUSED) && !p.contains(recall::NAMESAKES), "{p}");
}

// ---------- over HTTP ----------

#[test]
fn the_chat_takes_a_position_or_no_and_refuses_anything_else() {
    let s = Server::start();
    let day = "/diary/2026-10-31/chat";
    for bad in [json!("maybe"), json!(0), json!(4), json!(-1), json!(1.5), json!([1])] {
        let (code, body) = s.post_json(day, &json!({"message": "x", "choice": bad}));
        assert_eq!(code, 400, "{bad}: {body}");
    }
    let (code, _) = s.post_json(day, &json!({"message": "x", "choice": 1, "wrong": "틀린 문장"}));
    assert_eq!(code, 400, "a choice is no correction");
    for ok in [json!(1), json!(3), json!("no")] {
        let (code, body) = s.post_json(day, &json!({"message": "x", "choice": ok}));
        assert_eq!(code, 200, "{ok}: {body}");
        assert!(body.get("choices").is_none(), "nothing asked: {body}");
    }
    let (_, body) = s.get(day);
    assert!(body["messages"].as_array().unwrap().iter().all(|l: &Value| l.get("choices").is_none()));
}

#[test]
fn over_http_the_answer_and_the_day_s_chat_carry_the_choices_and_a_position_answers_them() {
    let m = chat(|_| ONE.to_string());
    let s = Server::start_full(1024 * 1024, m.clone());
    let t = T { _dir: tempfile::tempdir().unwrap(), p: s.paths.clone() };
    t.matter("a", "견적 일차", "2026-03-01", 3);
    t.matter("b", "견적 이차", "2026-05-01", 2);
    t.matter("c", "견적 삼차", "2026-07-01", 2);
    let day = "/diary/2026-10-31/chat";
    let (code, body) = s.post_json(day, &json!({"message": "견적 건 어떻게 됐지"}));
    assert_eq!(code, 200, "{body}");
    assert_eq!(body["choices"]["kind"], "matters", "{body}");
    let options = body["choices"]["options"].as_array().unwrap().clone();
    assert_eq!(options.len(), 3, "{body}");
    let (_, h) = s.get(day);
    assert_eq!(h["messages"].as_array().unwrap().last().unwrap()["choices"], body["choices"]);
    let second = options[1].as_str().unwrap();
    let (code, body) = s.post_json(day, &json!({"message": second, "choice": 2}));
    assert_eq!(code, 200, "{body}");
    assert!(body.get("choices").is_none(), "{body}");
    let p = prompt(&m);
    let title = second.split(" · ").next().unwrap();
    assert!(p.contains(title) && !p.contains(recall::MATTERS) && !p.contains(recall::ASSUMED), "{p}");
}
