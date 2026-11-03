//! "전에 어떻게 했지" (#95, ADR 0005): the earlier matters like the one at hand. The same counterparty's latest ones
//! first, three at most (as many as there are); with no counterparty, or no earlier matter with it, the ones in the
//! same situation (the same stage, the same part on the other side). The matter at hand is the one the question names,
//! else the one under way, asked about once. Each earlier matter comes with how it went and its mails (a revised sum
//! shown before and after), nothing after the day asked.
mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use common::FakeModel;
use life_server::assistant::recall::{self, Named, Picked};
use life_server::config::Paths;
use life_server::librarian::Librarian;
use life_server::librarian::graph;
use life_server::llm::{Ask, Model};
use serde_json::{Value, json};

const ME: &str = "나 <me@mine.co.kr>";
const DAY: &str = "2026-10-31";
const ALPHA: &str = "알파 담당 <a@alpha.co.kr>";

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

    /// A mail; `first`: the mail of the thread it answers, if any.
    #[allow(clippy::too_many_arguments)]
    fn mail(&self, name: &str, day: &str, from: &str, to: &str, subject: &str, body: &str, first: Option<&str>) {
        let path = self.p.vault.join("Mail").join(&day[..4]).join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let sent = from == ME;
        let source = if sent { "account/Sent Messages" } else { "account/INBOX" };
        let refs = first.map(|f| format!("[<{f}@x>]")).unwrap_or_else(|| "[]".into());
        let subject = if first.is_some() { format!("RE: {subject}") } else { subject.to_string() };
        fs::write(
            path,
            format!("---\ntype: mail\nmail_id: <{name}@x>\nmail_source: {source}\ndate: {day}\ntime: 09:00\nfrom: {from}\nto: [{to}]\ncc: []\nsubject: \"{subject}\"\nreferences: {refs}\n---\n## Message\n{body}\n"),
        )
        .unwrap();
    }

    /// A quotation matter with a client: asked for, then sent by the user.
    fn quote(&self, tag: &str, client: &str, subject: &str, day: &str) {
        let start = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap();
        let next = (start + chrono::Duration::days(2)).format("%Y-%m-%d").to_string();
        self.mail(&format!("{tag}1.md"), day, client, ME, subject, "견적 요청드립니다", None);
        self.mail(
            &format!("{tag}2.md"),
            &next,
            ME,
            client,
            subject,
            "견적서 송부드립니다",
            Some(&format!("{tag}1.md")),
        );
    }

    fn built(&self) -> Librarian {
        let mut lib = Librarian::open(&self.p, None, Duration::from_secs(5)).unwrap();
        lib.update(false).unwrap();
        graph::extract(&mut lib, reader().as_ref(), &graph::Run { limit: 100, workers: 1 }).unwrap();
        lib
    }
}

/// The model's reading of each mail, by words in it: the stage, and the part of the other side.
fn reader() -> Arc<FakeModel> {
    FakeModel::new(|msgs| {
        let text = msgs.last().unwrap()["content"].as_str().unwrap_or("").to_string();
        let mut r = Vec::new();
        for (words, state) in [
            ("견적 요청드립니다", "견적 요청"),
            ("견적서 송부드립니다", "견적 발송"),
            ("개정 견적", "견적 개정"),
            ("계약 체결", "계약"),
        ] {
            if text.contains(words) {
                r.push(json!(["건:case", "state", "", state, "", words]));
            }
        }
        for (words, role) in [("공급사 담당", "공급"), ("고객사 담당", "고객")] {
            if text.contains(words) {
                r.push(json!([format!("{words}:org"), "role", "건:case", role, "", words]));
            }
        }
        Ok(json!({ "r": r }).to_string())
    })
}

/// Four earlier quotations with one client and the one under way; the latest earlier one revised its sum, and goes
/// on after the day asked; another matter with the client starts after it.
fn alpha() -> T {
    let t = T::new();
    t.quote("p", ALPHA, "알파 펌프 견적", "2026-01-10");
    t.quote("v", ALPHA, "알파 밸브 견적", "2026-03-10");
    t.quote("s", ALPHA, "알파 센서 견적", "2026-05-10");
    t.mail("b1.md", "2026-07-10", ALPHA, ME, "알파 배관 견적", "견적 요청드립니다", None);
    t.mail("b2.md", "2026-07-12", ME, ALPHA, "알파 배관 견적", "견적서 송부드립니다. 금액 100만원", Some("b1.md"));
    t.mail("b3.md", "2026-07-20", ME, ALPHA, "알파 배관 견적", "개정 견적 다시 보냅니다. 금액 120만원", Some("b1.md"));
    t.mail("b4.md", "2026-11-05", ALPHA, ME, "알파 배관 견적", "최종 금액 150만원으로 하겠습니다", Some("b1.md"));
    t.mail("f1.md", "2026-11-20", ALPHA, ME, "알파 후속 발주", "발주서 보냅니다", None);
    // the matter under way
    t.mail("n1.md", "2026-10-20", ALPHA, ME, "알파 설비 견적", "설비 견적 요청드립니다", None);
    t.mail("n2.md", "2026-10-21", ME, ALPHA, "알파 설비 견적", "확인했습니다. 검토 후 회신드리겠습니다", Some("n1.md"));
    t
}

fn hits(lib: &mut Librarian, p: &Picked) -> Vec<Value> {
    match recall::lookup(lib, p, 8, DAY, &[]).unwrap() {
        Named::Hits(h) => h,
        Named::Ask(a) => panic!("asked back: {a:?}"),
    }
}

fn title(h: &Value) -> String {
    h["title"].as_str().unwrap_or("").trim_start_matches("RE: ").to_string()
}

/// The earlier matters gathered, in their order.
fn earlier(hits: &[Value]) -> Vec<String> {
    hits.iter().filter(|h| h.get("like").is_some()).map(title).collect()
}

fn named(words: &[&str]) -> Picked {
    Picked { words: words.iter().map(|w| w.to_string()).collect(), precedent: true, ..Picked::default() }
}

#[test]
fn the_latest_earlier_matters_with_the_same_counterparty_come_after_the_one_at_hand_three_at_most() {
    let t = alpha();
    let mut lib = t.built();
    let got = hits(&mut lib, &named(&["설비"]));
    assert_eq!(title(&got[0]), "알파 설비 견적", "{got:#?}");
    assert_eq!(got[0]["now"], true);
    assert_eq!(earlier(&got), ["알파 배관 견적", "알파 센서 견적", "알파 밸브 견적"], "{got:#?}");
    assert!(got[1]["like"].as_str().unwrap().contains("alpha.co.kr"), "{:?}", got[1]["like"]);
    assert!(!got.iter().any(|h| title(h).contains("후속 발주")), "a matter after the day asked is no earlier one");
}

#[test]
fn as_many_earlier_matters_as_there_are() {
    let t = T::new();
    t.quote("o", "베타 <b@beta.co.kr>", "베타 모터 견적", "2026-04-01");
    t.quote("n", "베타 <b@beta.co.kr>", "베타 설비 견적", "2026-10-20");
    let mut lib = t.built();
    let got = hits(&mut lib, &named(&["설비"]));
    assert_eq!(title(&got[0]), "베타 설비 견적");
    assert_eq!(earlier(&got), ["베타 모터 견적"], "{got:#?}");
}

#[test]
fn with_no_counterparty_the_matters_of_the_same_stage_and_the_same_part_on_the_other_side_are_gathered() {
    let t = T::new();
    t.mail("x0.md", "2026-01-02", ME, "동료 <kim@mine.co.kr>", "주간 보고", "보고드립니다", None);
    // among the user's own people: no counterparty
    t.mail(
        "i1.md",
        "2026-10-20",
        "동료 <kim@mine.co.kr>",
        ME,
        "사내 장비 구매",
        "공급사 담당 확인했고 견적 요청드립니다",
        None,
    );
    // the same stage and part / the same stage, another part / another stage, the same part
    t.mail(
        "g1.md",
        "2026-06-01",
        "감마 <g@gamma.co.kr>",
        ME,
        "감마 부품",
        "공급사 담당입니다. 견적 요청드립니다",
        None,
    );
    t.mail(
        "d1.md",
        "2026-07-01",
        "델타 <d@delta.co.kr>",
        ME,
        "델타 부품",
        "고객사 담당입니다. 견적 요청드립니다",
        None,
    );
    t.mail(
        "e1.md",
        "2026-08-01",
        "엡실론 <e@eps.co.kr>",
        ME,
        "엡실론 부품",
        "공급사 담당입니다. 계약 체결합니다",
        None,
    );
    let mut lib = t.built();
    let got = hits(&mut lib, &named(&["장비", "구매"]));
    assert_eq!(title(&got[0]), "사내 장비 구매", "{got:#?}");
    assert_eq!(earlier(&got), ["감마 부품"], "{got:#?}");
    let like = got[1]["like"].as_str().unwrap();
    assert!(like.contains("같은 상황") && like.contains("견적 요청") && like.contains("공급"), "{like}");
}

#[test]
fn a_matter_like_none_says_so_and_keeps_what_the_words_found() {
    let t = T::new();
    t.mail("x0.md", "2026-01-02", ME, "동료 <kim@mine.co.kr>", "주간 보고", "보고드립니다", None);
    t.mail("i1.md", "2026-10-20", "동료 <kim@mine.co.kr>", ME, "사내 장비 구매", "장비 목록입니다", None);
    let mut lib = t.built();
    let got = hits(&mut lib, &named(&["장비"]));
    assert_eq!(got[0]["no_precedent"], true, "{got:#?}");
    let section = recall::render(&got);
    assert!(section.contains(recall::NO_PRECEDENT), "{section}");
}

#[test]
fn each_earlier_matter_shows_how_it_went_and_its_mails_a_revised_sum_before_and_after_nothing_after_the_day() {
    let t = alpha();
    let mut lib = t.built();
    let section = recall::render(&hits(&mut lib, &named(&["설비"])));
    assert!(section.starts_with(recall::NOW_HIT), "{section}");
    assert!(section.contains("같은 거래처(alpha.co.kr)의 지난 건"), "{section}");
    let pipe: Vec<&str> = section.lines().skip_while(|l| !l.contains("알파 배관 견적")).take(4).collect();
    let line = pipe[0];
    // how it went, the oldest first, with the days: the revision is a step of it
    assert!(line.contains("경과: 견적 요청 (2026-07-10) → 견적 발송 (2026-07-12) → 견적 개정 (2026-07-20)"), "{line}");
    // the mails it holds, the sum before and after the revision: up to the day asked
    let all = pipe.join("\n");
    assert!(all.contains("100만원") && all.contains("120만원"), "{all}");
    assert!(all.contains("2026-07-12") && all.contains("2026-07-20"), "the days of the mails: {all}");
    assert!(!section.contains("150만원") && !section.contains("2026-11-05"), "nothing after the day asked: {section}");
    assert!(section.contains(recall::OTHER_HITS) || !section.lines().any(|l| l.contains("알파 펌프")), "{section}");
}

struct Picks(String);
impl Model for Picks {
    fn complete(&self, _: &[Value], _: Ask) -> anyhow::Result<String> {
        Ok(self.0.clone())
    }
}

const NAMES_NOTHING: &str = r#"{"search": true, "words": [], "precedent": true, "unclear": true}"#;

#[test]
fn a_question_that_names_no_matter_asks_once_about_the_one_under_way_and_is_answered_by_the_choice() {
    let t = alpha();
    t.built();
    let q = "이런 경우 전에 어떻게 했지";
    let s = recall::find(&t.p, &Picks(NAMES_NOTHING.into()), q, DAY, &[], None).unwrap();
    assert!(s.starts_with(recall::NOW), "{s}");
    let cands = recall::asked_candidates(&s).unwrap();
    assert_eq!(cands.lines().count(), 1, "the latest under way: {s}");
    assert!(cands.contains("알파 설비 견적") && cands.contains("견적 요청"), "{s}");
    assert!(!s.contains("[받은 메일]") && !s.contains("[보낸 메일]"), "no records before it is known: {s}");
    let before = format!("사용자가 물은 말: {q}\n비서가 되물은 말: 알파 설비 견적 말씀인가요?\n후보:\n{cands}");
    // chosen: the earlier ones like it
    let key = &cands[cands.rfind('<').unwrap()..];
    let chose = format!(r#"{{"search": true, "words": [], "precedent": true, "matter": "{key}"}}"#);
    let s = recall::find(&t.p, &Picks(chose), "응 그거", DAY, &[], Some(&before)).unwrap();
    assert!(s.starts_with(recall::NOW_HIT) && s.contains("알파 배관 견적"), "{s}");
    // not chosen: not asked again, the one offered is assumed and said to be
    let s = recall::find(&t.p, &Picks(NAMES_NOTHING.into()), "글쎄", DAY, &[], Some(&before)).unwrap();
    assert!(!s.contains(recall::NOW), "asked once: {s}");
    assert!(s.starts_with(recall::ASSUMED) && s.contains("알파 설비 견적") && s.contains("알파 배관 견적"), "{s}");
}

#[test]
fn the_picker_reads_a_question_about_how_it_was_done_before() {
    let m = Picks(r#"{"search": true, "words": [], "precedent": true}"#.into());
    let p = recall::pick(&m, "이런 경우 전에 어떻게 했지", None).unwrap();
    assert!(p.precedent && p.words.is_empty(), "nothing named stays nothing named: {p:?}");
    assert!(recall::WORDS_SYSTEM.contains("\"precedent\""));
}

/// The eval (#95): a precedent case may list the threads of the earlier matters with the same counterparty it should
/// gather; it counts as gathered when every one of them is among the earlier matters shown.
#[test]
fn the_eval_counts_whether_the_earlier_matters_with_the_same_counterparty_were_gathered() {
    use life_server::librarian::eval::{self, Kind};
    let t = T::new();
    t.quote("p", ALPHA, "알파 펌프 견적", "2026-01-05");
    t.quote("v", ALPHA, "알파 밸브 견적", "2026-02-05");
    t.quote("s", ALPHA, "알파 센서 견적", "2026-03-05");
    t.quote("b", ALPHA, "알파 배관 견적", "2026-04-05");
    t.quote("n", ALPHA, "알파 설비 견적", "2026-05-05");
    let mut lib = t.built();
    let thread = |name: &str| lib.thread_of(&t.p.vault.join("Mail/2026").join(name).to_string_lossy()).unwrap();
    let case = |earlier: &[&str]| {
        let list: Vec<String> = earlier.iter().map(|e| format!("\"{}\"", thread(e))).collect();
        format!(
            "[[case]]\nkind = \"precedent\"\nq = \"설비 견적 전에 어떻게 했지\"\nthread = \"{}\"\nearlier = [{}]\n",
            thread("n1.md"),
            list.join(", ")
        )
    };
    let f = t.p.data.join("recall.toml");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    // the latest three are gathered; the oldest is not
    fs::write(&f, format!("{}{}", case(&["b1.md", "s1.md"]), case(&["b1.md", "p1.md"]))).unwrap();
    let cases = eval::load(&f).unwrap();
    let m = Picks(r#"{"search": true, "words": ["설비"], "precedent": true}"#.into());
    let out = eval::run(&mut lib, Some(&m), &t.p.vault, &cases, 8).unwrap();
    assert_eq!(out[0].model_rank, Some(1), "the matter at hand comes first");
    assert_eq!((out[0].earlier_shown, out[1].earlier_shown), (Some(true), Some(false)));
    assert_eq!(eval::summary(&out)[&Kind::Precedent].earlier, (1, 2));
}

/// A question about a kind of work (#95, the user's choice A): it names no matter, no one and nothing of its own,
/// only words. Among what the words found, the first matter with a counterparty is taken for the matter at hand (a
/// guess, said as one), and the earlier matters with that counterparty are gathered.
fn kind_of_work() -> T {
    let t = T::new();
    t.mail("x0.md", "2026-01-02", ME, "동료 <kim@mine.co.kr>", "주간 보고", "보고드립니다", None);
    // the words fit an internal matter best
    t.mail(
        "i1.md",
        "2026-10-25",
        "동료 <kim@mine.co.kr>",
        ME,
        "포트 포워딩 포트 포워딩 정리",
        "포트 포워딩 포트 포워딩 목록",
        None,
    );
    t.mail(
        "i2.md",
        "2026-10-26",
        ME,
        "동료 <kim@mine.co.kr>",
        "포트 포워딩 포트 포워딩 정리",
        "포트 포워딩 포트 포워딩 확인",
        Some("i1.md"),
    );
    // a client's: the one at hand, and its earlier ones with the same client
    t.mail("c1.md", "2026-09-01", ALPHA, ME, "포트 포워딩 요청", "포트 포워딩 요청드립니다", None);
    t.mail("c2.md", "2026-09-02", ME, ALPHA, "포트 포워딩 요청", "적용했습니다", Some("c1.md"));
    t.quote("e", ALPHA, "알파 방화벽 견적", "2026-05-01");
    t.quote("f", ALPHA, "알파 서버 견적", "2026-03-01");
    // another client's matter about it, all after the day asked: not the one at hand
    t.mail("l1.md", "2026-11-10", "베타 <b@beta.co.kr>", ME, "포트 포워딩 포트 포워딩 요청", "포트 포워딩 요청", None);
    t
}

#[test]
fn a_question_about_a_kind_of_work_takes_the_first_matter_with_a_counterparty_for_the_one_at_hand() {
    let t = kind_of_work();
    let mut lib = t.built();
    let got = hits(&mut lib, &named(&["포트 포워딩"]));
    assert_eq!(title(&got[0]), "포트 포워딩 요청", "{got:#?}");
    assert_eq!(got[0]["now_guessed"], true);
    assert_eq!(earlier(&got), ["알파 방화벽 견적", "알파 서버 견적"], "{got:#?}");
    let section = recall::render(&got);
    assert!(section.starts_with(recall::NOW_GUESSED), "said to be a guess: {section}");
    assert!(section.contains("포트 포워딩 포트 포워딩 정리"), "what else the words found stays: {section}");
}

#[test]
fn a_question_about_a_kind_of_work_with_no_matter_with_a_counterparty_falls_back_as_before() {
    let t = T::new();
    t.mail("x0.md", "2026-01-02", ME, "동료 <kim@mine.co.kr>", "주간 보고", "보고드립니다", None);
    t.mail("i1.md", "2026-10-25", "동료 <kim@mine.co.kr>", ME, "포트 포워딩 정리", "포트 포워딩 목록", None);
    t.mail("i2.md", "2026-09-25", "동료 <kim@mine.co.kr>", ME, "포트 포워딩 변경", "포트 포워딩 변경", None);
    let mut lib = t.built();
    let got = hits(&mut lib, &named(&["포트 포워딩"]));
    assert_eq!(title(&got[0]), "포트 포워딩 정리", "{got:#?}");
    assert!(got[0].get("now_guessed").is_none());
    assert_eq!(got[0]["now"], true);
}

#[test]
fn a_question_that_names_something_keeps_the_best_fitting_matter_as_the_one_at_hand() {
    let t = kind_of_work();
    let mut lib = t.built();
    let p = Picked { things: vec!["포트 포워딩".into()], ..named(&["포트 포워딩"]) };
    let got = hits(&mut lib, &p);
    assert!(got[0].get("now_guessed").is_none(), "{got:#?}");
    assert_ne!(title(&got[0]), "포트 포워딩 요청", "{got:#?}");
}

#[test]
fn a_counterparty_with_no_earlier_matter_does_not_take_the_place_of_the_best_fit() {
    let t = T::new();
    t.mail("x0.md", "2026-01-02", ME, "동료 <kim@mine.co.kr>", "주간 보고", "보고드립니다", None);
    t.mail(
        "i1.md",
        "2026-10-25",
        "동료 <kim@mine.co.kr>",
        ME,
        "포트 포워딩 포트 포워딩 정리",
        "포트 포워딩 포트 포워딩 목록",
        None,
    );
    t.mail(
        "i2.md",
        "2026-10-26",
        ME,
        "동료 <kim@mine.co.kr>",
        "포트 포워딩 포트 포워딩 정리",
        "포트 포워딩 확인",
        Some("i1.md"),
    );
    // a client's matter about it, the only one with that client
    t.mail("c1.md", "2026-09-01", ALPHA, ME, "포트 포워딩 요청", "포트 포워딩 요청드립니다", None);
    let mut lib = t.built();
    let got = hits(&mut lib, &named(&["포트 포워딩"]));
    assert_eq!(title(&got[0]), "포트 포워딩 포트 포워딩 정리", "{got:#?}");
    assert!(got[0].get("now_guessed").is_none());
}
