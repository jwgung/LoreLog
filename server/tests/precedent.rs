//! "전에 어떻게 했지" (#95, #101, ADR 0006·0007): the matter the question fits best is found (by meaning when it can
//! be, else by the words); when that first one has a counterparty outside, that counterparty's other matters are gathered
//! besides it: the user's own first, then the latest, three at most, never by how near they are to the question. A first
//! one among the user's own people gathers nothing, and nothing is guessed in its place: no other found matter is taken
//! for the one at hand, no matter in the same situation is gathered. A question that points at the matter at hand
//! ("이런 경우", "이 건") asks about the one under way once and gathers by the one chosen. What was found and what was
//! gathered are said apart, with the matter they were gathered by. Each gathered matter comes with how it went and its
//! mails (a revised sum shown before and after), nothing after the day asked.
mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use common::FakeModel;
use life_server::assistant::recall::{self, Named, Picked};
use life_server::config::Paths;
use life_server::librarian::Librarian;
use life_server::librarian::graph;
use life_server::llm::{Ask, Model, WordsEmbedder};
use serde_json::{Value, json};

const ME: &str = "나 <me@mine.co.kr>";
const DAY: &str = "2026-10-31";
const ALPHA: &str = "알파 담당 <a@alpha.co.kr>";
const COLLEAGUE: &str = "동료 <kim@mine.co.kr>";

struct T {
    _dir: tempfile::TempDir,
    p: Paths,
}

impl T {
    fn new() -> T {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::under(dir.path());
        fs::create_dir_all(p.vault.join("Events")).unwrap();
        let t = T { _dir: dir, p };
        // the user's own organization is the one of their mail
        t.mail("x0.md", "2026-01-02", ME, COLLEAGUE, "주간 보고", "보고드립니다", None);
        t
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

fn titles(hits: &[Value]) -> Vec<String> {
    hits.iter().map(title).collect()
}

/// The counterparty's other matters gathered, in their order.
fn earlier(hits: &[Value]) -> Vec<String> {
    hits.iter().filter(|h| h.get("like").is_some()).map(title).collect()
}

fn named(words: &[&str]) -> Picked {
    Picked { words: words.iter().map(|w| w.to_string()).collect(), precedent: true, ..Picked::default() }
}

/// Nothing is guessed: no matter among what was found is taken for the one at hand.
fn nothing_guessed(got: &[Value]) {
    assert!(got.iter().all(|h| h.get("now").is_none() && h.get("now_guessed").is_none()), "{got:#?}");
}

// ---------- the counterparty of the matter found first ----------

#[test]
fn the_matter_found_first_with_a_counterparty_gathers_its_latest_other_matters_three_at_most() {
    let t = alpha();
    let mut lib = t.built();
    let got = hits(&mut lib, &named(&["설비"]));
    assert_eq!(title(&got[0]), "알파 설비 견적", "{got:#?}");
    assert_eq!(got[0]["top"], true, "{got:#?}");
    assert_eq!(earlier(&got), ["알파 배관 견적", "알파 센서 견적", "알파 밸브 견적"], "{got:#?}");
    assert!(got[1]["like"].as_str().unwrap().contains("alpha.co.kr"), "{:?}", got[1]["like"]);
    assert!(!got.iter().any(|h| title(h).contains("후속 발주")), "a matter after the day asked is no other one");
    nothing_guessed(&got);
}

#[test]
fn as_many_other_matters_as_there_are() {
    let t = T::new();
    t.quote("o", "베타 <b@beta.co.kr>", "베타 모터 견적", "2026-04-01");
    t.quote("n", "베타 <b@beta.co.kr>", "베타 설비 견적", "2026-10-20");
    let mut lib = t.built();
    let got = hits(&mut lib, &named(&["설비"]));
    assert_eq!(title(&got[0]), "베타 설비 견적");
    assert_eq!(earlier(&got), ["베타 모터 견적"], "{got:#?}");
}

#[test]
fn the_user_s_own_matters_with_the_counterparty_come_before_later_ones_they_took_no_part_in() {
    let t = T::new();
    let beta = "베타 <b@beta.co.kr>";
    t.quote("a", beta, "베타 펌프 견적", "2026-02-01");
    t.quote("b", beta, "베타 밸브 견적", "2026-03-01");
    t.quote("c", beta, "베타 센서 견적", "2026-04-01");
    // the latest, but only received: the user took no part in it
    t.mail("u1.md", "2026-09-01", beta, ME, "베타 소식", "새 제품 안내입니다", None);
    t.quote("n", beta, "베타 설비 견적", "2026-10-20");
    let mut lib = t.built();
    let got = hits(&mut lib, &named(&["설비"]));
    assert_eq!(title(&got[0]), "베타 설비 견적", "{got:#?}");
    assert_eq!(earlier(&got), ["베타 센서 견적", "베타 밸브 견적", "베타 펌프 견적"], "{got:#?}");
}

/// A first one among the user's own people gathers nothing: no other found matter with a counterparty is taken for the
/// one at hand in its place (ADR 0006 took back the guess of ADR 0005).
fn kind_of_work() -> T {
    let t = T::new();
    // the words fit an internal matter best
    t.mail("i1.md", "2026-10-25", COLLEAGUE, ME, "포트 포워딩 포트 포워딩 정리", "포트 포워딩 포트 포워딩 목록", None);
    t.mail(
        "i2.md",
        "2026-10-26",
        ME,
        COLLEAGUE,
        "포트 포워딩 포트 포워딩 정리",
        "포트 포워딩 포트 포워딩 확인",
        Some("i1.md"),
    );
    // a client's, with other matters of the same client
    t.mail("c1.md", "2026-09-01", ALPHA, ME, "포트 포워딩 요청", "포트 포워딩 요청드립니다", None);
    t.mail("c2.md", "2026-09-02", ME, ALPHA, "포트 포워딩 요청", "적용했습니다", Some("c1.md"));
    t.quote("e", ALPHA, "알파 방화벽 견적", "2026-05-01");
    t.quote("f", ALPHA, "알파 서버 견적", "2026-03-01");
    t
}

#[test]
fn a_matter_found_first_with_no_counterparty_gathers_nothing_and_no_other_is_taken_in_its_place() {
    let t = kind_of_work();
    let mut lib = t.built();
    let got = hits(&mut lib, &named(&["포트 포워딩"]));
    assert_eq!(title(&got[0]), "포트 포워딩 포트 포워딩 정리", "{got:#?}");
    assert_eq!(got[0]["top"], true);
    assert!(earlier(&got).is_empty(), "{got:#?}");
    assert_eq!(got[0]["no_precedent"], true, "{got:#?}");
    assert!(titles(&got).contains(&"포트 포워딩 요청".to_string()), "what else was found stays: {got:#?}");
    assert!(!titles(&got).iter().any(|t| t.contains("방화벽")), "{got:#?}");
    nothing_guessed(&got);
    let section = recall::render(&got);
    assert!(section.starts_with(recall::FOUND_HIT), "{section}");
    assert!(section.contains(recall::NO_PRECEDENT), "{section}");
}

#[test]
fn matters_in_the_same_situation_are_not_gathered() {
    let t = T::new();
    // among the user's own people: no counterparty; a stage and a part on the other side the graph reads
    t.mail("i1.md", "2026-10-20", COLLEAGUE, ME, "사내 장비 구매", "공급사 담당 확인했고 견적 요청드립니다", None);
    // the same stage and the same part, with another organization
    t.mail(
        "g1.md",
        "2026-06-01",
        "감마 <g@gamma.co.kr>",
        ME,
        "감마 부품",
        "공급사 담당입니다. 견적 요청드립니다",
        None,
    );
    let mut lib = t.built();
    let got = hits(&mut lib, &named(&["장비", "구매"]));
    assert_eq!(title(&got[0]), "사내 장비 구매", "{got:#?}");
    assert!(earlier(&got).is_empty(), "{got:#?}");
    assert_eq!(got[0]["no_precedent"], true);
    assert!(!recall::render(&got).contains("같은 상황"), "{}", recall::render(&got));
}

// ---------- what was found and what was gathered, said apart ----------

#[test]
fn what_was_found_comes_first_and_the_counterparty_s_other_matters_after_under_the_matter_they_were_gathered_by() {
    let t = T::new();
    // the words fit the client's first, then an internal one
    t.mail("c1.md", "2026-10-01", ALPHA, ME, "방화벽 설정 요청", "방화벽 설정 변경 요청드립니다. 방화벽 설정", None);
    t.mail("c2.md", "2026-10-02", ME, ALPHA, "방화벽 설정 요청", "방화벽 설정 적용했습니다", Some("c1.md"));
    t.mail("i1.md", "2026-09-01", COLLEAGUE, ME, "방화벽 점검", "방화벽 점검 결과입니다", None);
    t.quote("e", ALPHA, "알파 펌프 견적", "2026-05-01");
    t.quote("f", ALPHA, "알파 밸브 견적", "2026-03-01");
    let mut lib = t.built();
    let got = hits(&mut lib, &named(&["방화벽", "설정"]));
    assert_eq!(titles(&got), ["방화벽 설정 요청", "방화벽 점검", "알파 펌프 견적", "알파 밸브 견적"], "{got:#?}");
    let section = recall::render(&got);
    let lines: Vec<&str> = section.lines().collect();
    assert_eq!(lines[0], recall::FOUND_HIT, "{section}");
    let head = lines.iter().position(|l| l.contains("같은 거래처(alpha.co.kr)의 다른 건")).expect(&section);
    assert!(
        lines[head].contains("방화벽 설정 요청\" 건의 거래처로"),
        "the matter they were gathered by is named: {section}"
    );
    let found = lines.iter().position(|l| l.contains("방화벽 점검")).unwrap();
    let pump = lines.iter().position(|l| l.contains("알파 펌프 견적")).unwrap();
    assert!(found < head && head < pump, "{section}");
}

#[test]
fn each_gathered_matter_shows_how_it_went_and_its_mails_a_revised_sum_before_and_after_nothing_after_the_day() {
    let t = alpha();
    let mut lib = t.built();
    let section = recall::render(&hits(&mut lib, &named(&["설비"])));
    assert!(section.starts_with(recall::FOUND_HIT), "{section}");
    let pipe: Vec<&str> = section.lines().skip_while(|l| !l.contains("알파 배관 견적")).take(4).collect();
    let line = pipe[0];
    // how it went, the oldest first, with the days: the revision is a step of it
    assert!(line.contains("경과: 견적 요청 (2026-07-10) → 견적 발송 (2026-07-12) → 견적 개정 (2026-07-20)"), "{line}");
    // the mails it holds, the sum before and after the revision: up to the day asked
    let all = pipe.join("\n");
    assert!(all.contains("100만원") && all.contains("120만원"), "{all}");
    assert!(all.contains("2026-07-12") && all.contains("2026-07-20"), "the days of the mails: {all}");
    assert!(!section.contains("150만원") && !section.contains("2026-11-05"), "nothing after the day asked: {section}");
    // the fourth other one is left out
    assert!(!section.contains("알파 펌프 견적"), "{section}");
}

// ---------- a question that points at the matter at hand ----------

struct Picks(String);
impl Model for Picks {
    fn complete(&self, _: &[Value], _: Ask) -> anyhow::Result<String> {
        Ok(self.0.clone())
    }
}

const NAMES_NOTHING: &str = r#"{"search": true, "words": [], "precedent": true, "unclear": true}"#;

#[test]
fn a_question_that_names_no_matter_asks_once_about_the_one_under_way_and_gathers_by_the_choice() {
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
    // chosen: its counterparty's other matters
    let key = &cands[cands.rfind('<').unwrap()..];
    let chose = format!(r#"{{"search": true, "words": [], "precedent": true, "at_hand": true, "matter": "{key}"}}"#);
    let s = recall::find(&t.p, &Picks(chose), "응 그거", DAY, &[], Some(&before)).unwrap();
    assert!(s.starts_with(recall::NOW_HIT) && s.contains("알파 배관 견적"), "{s}");
    assert!(s.contains("알파 설비 견적\" 건의 거래처로"), "the matter they were gathered by is named: {s}");
    // not chosen: not asked again, the one offered is assumed and said to be
    let s = recall::find(&t.p, &Picks(NAMES_NOTHING.into()), "글쎄", DAY, &[], Some(&before)).unwrap();
    assert!(!s.contains(recall::NOW), "asked once: {s}");
    assert!(s.starts_with(recall::ASSUMED) && s.contains("알파 설비 견적") && s.contains("알파 배관 견적"), "{s}");
}

#[test]
fn a_question_pointing_at_the_matter_at_hand_asks_about_the_one_under_way_though_it_has_words() {
    let t = alpha();
    t.built();
    let at_hand = r#"{"search": true, "words": ["견적"], "precedent": true, "at_hand": true}"#;
    let s = recall::find(&t.p, &Picks(at_hand.into()), "이 견적 건 전에 어떻게 했지", DAY, &[], None).unwrap();
    assert!(s.starts_with(recall::NOW), "{s}");
    assert!(recall::asked_candidates(&s).unwrap().contains("알파 설비 견적"), "{s}");
}

#[test]
fn a_question_about_a_kind_of_work_is_not_asked_back_and_finds_by_its_topic() {
    let t = alpha();
    t.built();
    let kind = r#"{"search": true, "words": ["설비"], "precedent": true}"#;
    let s = recall::find(&t.p, &Picks(kind.into()), "설비 견적 낼 땐 전에 어떻게 했지", DAY, &[], None).unwrap();
    assert!(!s.contains(recall::NOW), "{s}");
    assert!(s.starts_with(recall::FOUND_HIT) && s.contains("알파 배관 견적"), "{s}");
}

#[test]
fn the_picker_reads_whether_the_question_points_at_the_matter_at_hand() {
    let m = Picks(r#"{"search": true, "words": [], "precedent": true, "at_hand": true}"#.into());
    let p = recall::pick(&m, "이런 경우 전에 어떻게 했지", None).unwrap();
    assert!(p.precedent && p.at_hand && p.words.is_empty(), "nothing named stays nothing named: {p:?}");
    // when it does not say, the topic is searched
    let m = Picks(r#"{"search": true, "words": ["견적"], "precedent": true}"#.into());
    assert!(!recall::pick(&m, "견적 낼 땐 전에 어떻게 했지", None).unwrap().at_hand);
    assert!(recall::WORDS_SYSTEM.contains("\"precedent\"") && recall::WORDS_SYSTEM.contains("\"at_hand\""));
}

// ---------- by meaning, the same rules ----------

/// The client's matter the question means, an older one of the client's near it in meaning, three later ones that are
/// not, and an internal one.
fn by_meaning() -> (T, Librarian) {
    let t = T::new();
    t.mail("c1.md", "2026-10-01", ALPHA, ME, "포트 포워딩 요청", "포트 포워딩 요청드립니다", None);
    t.mail("c2.md", "2026-10-02", ME, ALPHA, "포트 포워딩 요청", "적용했습니다", Some("c1.md"));
    t.mail("o1.md", "2026-01-05", ALPHA, ME, "포트 포워딩 변경", "포트 포워딩 변경 요청드립니다", None);
    t.mail("o2.md", "2026-01-06", ME, ALPHA, "포트 포워딩 변경", "변경했습니다", Some("o1.md"));
    t.quote("e", ALPHA, "알파 펌프 견적", "2026-03-01");
    t.quote("f", ALPHA, "알파 밸브 견적", "2026-05-01");
    t.quote("g", ALPHA, "알파 센서 견적", "2026-07-01");
    t.mail("i1.md", "2026-10-10", COLLEAGUE, ME, "회의실 예약 안내", "회의실 예약 방법 안내드립니다", None);
    t.mail("i2.md", "2026-10-11", ME, COLLEAGUE, "회의실 예약 안내", "확인했습니다", Some("i1.md"));
    let mut lib = t.built();
    lib.embed_missing(&WordsEmbedder::new("fake")).unwrap();
    lib.embedder = Some(Arc::new(WordsEmbedder::new("fake")));
    (t, lib)
}

/// Words the records do not hold: what is found was found by meaning.
fn meant(question: &str) -> Picked {
    Picked { question: question.into(), ..named(&["없는낱말"]) }
}

#[test]
fn by_meaning_the_matter_found_first_gathers_its_counterparty_s_latest_others_not_the_nearest() {
    let (_t, mut lib) = by_meaning();
    let got = hits(&mut lib, &meant("포트 포워딩 요청 전에 어떻게 했지"));
    assert_eq!(title(&got[0]), "포트 포워딩 요청", "{got:#?}");
    assert_eq!(got[0]["top"], true);
    assert_eq!(earlier(&got), ["알파 센서 견적", "알파 밸브 견적", "알파 펌프 견적"], "{got:#?}");
    nothing_guessed(&got);
}

#[test]
fn by_meaning_a_matter_found_first_with_no_counterparty_gathers_nothing() {
    let (_t, mut lib) = by_meaning();
    let got = hits(&mut lib, &meant("회의실 예약 안내 전에 어떻게 했지"));
    assert_eq!(title(&got[0]), "회의실 예약 안내", "{got:#?}");
    assert!(earlier(&got).is_empty(), "{got:#?}");
    assert_eq!(got[0]["no_precedent"], true);
    nothing_guessed(&got);
}

// ---------- the eval ----------

/// The eval (#95, ADR 0006): a precedent case lists the threads of the earlier matters it should gather. Three are
/// shown at most, so what is found is the labels shown over min(3, labels); how exact is the share of the shown
/// earlier matters that are labels.
#[test]
fn the_eval_scores_the_earlier_matters_by_how_many_labels_are_found_and_how_exact_what_is_shown() {
    use life_server::librarian::eval::{self, Kind};
    let t = T::new();
    t.quote("p", ALPHA, "알파 펌프 견적", "2026-01-05");
    t.quote("v", ALPHA, "알파 밸브 견적", "2026-02-05");
    t.quote("s", ALPHA, "알파 센서 견적", "2026-03-05");
    t.quote("b", ALPHA, "알파 배관 견적", "2026-04-05");
    t.quote("n", ALPHA, "알파 설비 견적", "2026-05-05");
    t.quote("z", "다른 담당 <z@other.co.kr>", "다른 곳 견적", "2026-05-20");
    let mut lib = t.built();
    let thread = |name: &str| lib.thread_of(&t.p.vault.join("Mail/2026").join(name).to_string_lossy()).unwrap();
    let case = |at: &str, q: &str, earlier: Option<&[&str]>| {
        let list = earlier.map(|e| {
            let l: Vec<String> = e.iter().map(|e| format!("\"{}\"", thread(e))).collect();
            format!("earlier = [{}]\n", l.join(", "))
        });
        format!(
            "[[case]]\nkind = \"precedent\"\nq = \"{q}\"\nthread = \"{}\"\n{}",
            thread(at),
            list.unwrap_or_default()
        )
    };
    let f = t.p.data.join("recall.toml");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(
        &f,
        format!(
            "{}{}{}",
            // four labels; the latest three earlier ones are shown: two of them are labels
            case("n1.md", "설비 견적 전에 어떻게 했지", Some(&["b1.md", "s1.md", "p1.md", "z1.md"])),
            // no label: not in this measure
            case("n1.md", "설비 견적 전에 어떻게 했지 (2)", None),
            // labels all shown
            case("n1.md", "설비 견적 전에 어떻게 했지 (3)", Some(&["b1.md", "s1.md", "v1.md"])),
        ),
    )
    .unwrap();
    let cases = eval::load(&f).unwrap();
    let m = Picks(r#"{"search": true, "words": ["설비"], "precedent": true}"#.into());
    let out = eval::run(&mut lib, Some(&m), &t.p.vault, &cases, 8).unwrap();
    assert_eq!(out[0].model_rank, Some(1), "the matter found comes first");
    assert_eq!(out[0].gathered, Some(3), "the counterparty's other matters gathered");
    let e = out[0].earlier.expect("scored");
    assert_eq!((e.labels, e.found, e.shown, e.right), (4, 2, 3, 2), "{e:?}");
    assert!(out[1].earlier.is_none(), "no label: left out of this measure");
    let e = out[2].earlier.expect("scored");
    assert_eq!((e.labels, e.found, e.shown, e.right), (3, 3, 3, 3), "{e:?}");

    let s = &eval::summary(&out)[&Kind::Precedent].earlier;
    assert_eq!(s.cases, 2);
    assert!((s.found_avg().unwrap() - (2.0 / 3.0 + 1.0) / 2.0).abs() < 1e-9, "{s:?}");
    assert!((s.exact_avg().unwrap() - (2.0 / 3.0 + 1.0) / 2.0).abs() < 1e-9, "{s:?}");
    assert_eq!((s.any, s.none), (2, 0));
}

/// A matter with no earlier one shows none: how exact is not defined, it is counted as "none shown" apart.
#[test]
fn the_eval_counts_a_case_with_no_earlier_matter_shown_apart_and_leaves_it_out_of_how_exact() {
    use life_server::librarian::eval::{self, Kind};
    let t = T::new();
    t.quote("n", "다른 담당 <z@other.co.kr>", "다른 곳 견적", "2026-05-05");
    let mut lib = t.built();
    let thread = |name: &str| lib.thread_of(&t.p.vault.join("Mail/2026").join(name).to_string_lossy()).unwrap();
    let f = t.p.data.join("recall.toml");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(
        &f,
        format!(
            "[[case]]\nkind = \"precedent\"\nq = \"다른 곳 견적 전에 어떻게 했지\"\nthread = \"{}\"\nearlier = [\"{}\"]\n",
            thread("n1.md"),
            "mail:<label-not-shown@x>"
        ),
    )
    .unwrap();
    let cases = eval::load(&f).unwrap();
    let m = Picks(r#"{"search": true, "words": ["다른 곳"], "precedent": true}"#.into());
    let out = eval::run(&mut lib, Some(&m), &t.p.vault, &cases, 8).unwrap();
    assert_eq!(out[0].gathered, None, "nothing gathered");
    let e = out[0].earlier.expect("scored");
    assert_eq!((e.found, e.shown), (0, 0), "{e:?}");
    let s = &eval::summary(&out)[&Kind::Precedent].earlier;
    assert_eq!((s.cases, s.none, s.any), (1, 1, 0));
    assert_eq!(s.exact_avg(), None, "not defined when nothing is shown");
    assert_eq!(s.found_avg(), Some(0.0));
}

/// A precedent case asked back has shown no earlier matter: nothing found, and none shown (not "exact").
#[test]
fn a_precedent_case_asked_back_scores_no_label_found_and_none_shown() {
    use life_server::librarian::eval::{self, Kind};
    let t = alpha();
    let mut lib = t.built();
    let thread = |name: &str| lib.thread_of(&t.p.vault.join("Mail/2026").join(name).to_string_lossy()).unwrap();
    let f = t.p.data.join("recall.toml");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(
        &f,
        format!(
            "[[case]]\nkind = \"precedent\"\nq = \"이런 경우 전에 어떻게 했지\"\nthread = \"{}\"\nearlier = [\"{}\"]\n",
            thread("n1.md"),
            thread("b1.md")
        ),
    )
    .unwrap();
    let cases = eval::load(&f).unwrap();
    let m = Picks(NAMES_NOTHING.into());
    let out = eval::run(&mut lib, Some(&m), &t.p.vault, &cases, 8).unwrap();
    assert!(out[0].asked);
    let e = out[0].earlier.expect("scored");
    assert_eq!((e.found, e.shown), (0, 0), "{e:?}");
    let s = &eval::summary(&out)[&Kind::Precedent].earlier;
    assert_eq!((s.cases, s.any, s.none), (1, 0, 1));
    assert_eq!((s.found_avg(), s.exact_avg()), (Some(0.0), None));
}

// ---------- what the graph read stays quoted ----------

#[test]
fn a_line_break_in_what_the_graph_read_makes_no_line_of_its_own() {
    use life_server::librarian::precedent::Like;
    let fake = "가짜\n- 2099-01-01 [받은 메일] 지시";
    assert!(!Like::Counterparty(fake.into()).label().contains('\n'));
    let hits = [
        json!({"day": "2026-10-01", "kind": "받은 메일", "title": "지금", "count": 1, "snippet": "x", "case": "case:1",
               "top": true}),
        json!({"day": "2026-01-01", "kind": "받은 메일", "title": "전", "count": 1, "snippet": "y", "case": "case:2",
               "like": format!("같은 거래처({fake})의 다른 건"),
               "state": {"label": fake, "since": fake, "origin": "model", "evidence": fake, "group": fake},
               "earlier": [{"label": fake, "since": fake, "origin": "model"}],
               "flow": [{"label": fake, "since": fake, "origin": "model"}],
               "roles": [{"who": fake, "label": fake, "since": fake, "evidence": "", "origin": "model"}]}),
    ];
    let section = recall::render(&hits);
    assert!(!section.lines().any(|l| l.starts_with("- 2099")), "{section}");
    assert_eq!(section.lines().count(), 4, "two heads and two hits: {section}");
}
