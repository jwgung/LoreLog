//! The structure the user confirms (#63): the record's words for a role or a state put under the user's name for them
//! (`label:<record word>`, right a name or `-` to take it out), and the order the user's stages go in
//! (`order:<group>`, right `a→b→c`). Kept as any correction (#10), so they hold through a rebuild; a group the user did
//! not answer is only used inside, as before.
mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use common::{FakeModel, Server};
use life_server::librarian::Librarian;
use life_server::librarian::graph::{self, Ask1};
use life_server::librarian::structure;
use serde_json::{Value, json};

const ME: &str = "나 <me@mine.co.kr>";
const DAY: &str = "2026-10-31";

fn mail(s: &Server, name: &str, day: &str, subject: &str, body: &str) {
    let path = s.paths.vault.join("Mail").join(&day[..4]).join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!("---\ntype: mail\nmail_id: <{name}@x>\nmail_source: account/Sent Messages\ndate: {day}\ntime: 09:00\nfrom: {ME}\nto: [고객 <c@client.co.kr>]\ncc: []\nsubject: \"{subject}\"\nreferences: []\n---\n## Message\n{body}\n"),
    )
    .unwrap();
}

/// The model's grouping of the record's words for states.
fn model_groups() -> Value {
    json!({"견적 요청": "견적", "견적서 송부": "견적", "가격 제안": "견적", "계약 체결": "계약", "납품 완료": "납품",
           "지원자": "지원자", "응시자": "지원자", "후보자": "지원자",
           "공급사": "관계사", "납품사": "관계사", "제조사": "관계사", "협력사": "관계사", "파트너사": "관계사",
           "외주사": "관계사", "시공사": "관계사", "설계사": "관계사", "감리사": "관계사"})
}

/// The model: the states of each matter by the words of its mail, and the groups of their names.
fn reader() -> Arc<FakeModel> {
    FakeModel::new(|msgs| {
        let system = msgs[0]["content"].as_str().unwrap_or("");
        if system.starts_with("업무 기록에서 뽑은 역할·상태 이름들을") {
            return Ok(json!({"g": model_groups()}).to_string());
        }
        let text = msgs.last().unwrap()["content"].as_str().unwrap_or("");
        let st = |case: &str, day: &str, label: &str, ev: &str| {
            json!([format!("{case}:case"), "state", day, label, day, ev])
        };
        let r = if text.contains("설비 점검") {
            json!([
                st("설비 점검", "2026-03-01", "견적 요청", "견적 요청드립니다"),
                st("설비 점검", "2026-03-02", "견적서 송부", "견적서 송부합니다"),
                st("설비 점검", "2026-03-10", "계약 체결", "계약 체결했습니다"),
                st("설비 점검", "2026-03-20", "납품 완료", "납품 완료했습니다")
            ])
        } else if text.contains("배관 교체") {
            json!([
                st("배관 교체", "2026-03-05", "견적서 송부", "견적서 송부합니다"),
                st("배관 교체", "2026-03-12", "계약 체결", "계약 체결했습니다")
            ])
        } else if text.contains("센서 설치") {
            json!([
                st("센서 설치", "2026-04-01", "가격 제안", "가격 제안드립니다"),
                st("센서 설치", "2026-04-09", "납품 완료", "납품 완료했습니다")
            ])
        } else if text.contains("도장 작업") {
            json!([st("도장 작업", "2026-04-03", "견적 요청", "견적 요청드립니다")])
        } else if let Some((case, who, role)) = [
            ("연구원 채용", "정다운", "지원자"),
            ("인턴 채용", "김하나", "응시자"),
            ("엔지니어 채용", "이수진", "후보자"),
        ]
        .into_iter()
        .find(|(c, ..)| text.contains(c))
        {
            json!([[
                format!("{who}:person"),
                "role",
                format!("{case}:case"),
                role,
                "2026-05-10",
                format!("{role} {who}")
            ]])
        } else if let Some(i) = ["협력 일", "협력 이", "협력 삼"].iter().position(|c| text.contains(c)) {
            // a group too big to ask about in one line: nine words
            let roles =
                [["공급사", "납품사", "제조사"], ["협력사", "파트너사", "외주사"], ["시공사", "설계사", "감리사"]][i];
            let case = format!("협력 {}:case", ["일", "이", "삼"][i]);
            json!(
                roles
                    .iter()
                    .map(|r| json!([format!("{r} 담당:person"), "role", case, r, "2026-06-10", r]))
                    .collect::<Vec<_>>()
            )
        } else {
            json!([])
        };
        Ok(json!({"r": r}).to_string())
    })
}

fn world() -> (Server, Librarian) {
    world_with(FakeModel::plain())
}

/// The world, with a server whose model answers the chat with this.
fn world_with(chat: Arc<FakeModel>) -> (Server, Librarian) {
    let s = Server::start_full(1024 * 1024, chat);
    fs::create_dir_all(s.paths.vault.join("Events")).unwrap();
    mail(
        &s,
        "a.md",
        "2026-03-20",
        "설비 점검",
        "설비 점검: 견적 요청드립니다. 견적서 송부합니다. 계약 체결했습니다. 납품 완료했습니다.",
    );
    mail(&s, "b.md", "2026-03-12", "배관 교체", "배관 교체: 견적서 송부합니다. 계약 체결했습니다.");
    mail(&s, "c.md", "2026-04-09", "센서 설치", "센서 설치: 가격 제안드립니다. 납품 완료했습니다.");
    mail(&s, "d.md", "2026-04-03", "도장 작업", "도장 작업: 견적 요청드립니다.");
    mail(&s, "h1.md", "2026-05-10", "연구원 채용", "연구원 채용: 지원자 정다운");
    mail(&s, "h2.md", "2026-05-10", "인턴 채용", "인턴 채용: 응시자 김하나");
    mail(&s, "h3.md", "2026-05-10", "엔지니어 채용", "엔지니어 채용: 후보자 이수진");
    mail(&s, "p1.md", "2026-06-10", "협력 일", "협력 일: 공급사 납품사 제조사");
    mail(&s, "p2.md", "2026-06-10", "협력 이", "협력 이: 협력사 파트너사 외주사");
    mail(&s, "p3.md", "2026-06-10", "협력 삼", "협력 삼: 시공사 설계사 감리사");
    let mut lib = Librarian::open(&s.paths, None, Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    let model = reader();
    graph::extract(&mut lib, model.as_ref(), &graph::Run { limit: 50, workers: 1 }).unwrap();
    graph::group_labels(&lib, model.as_ref()).unwrap();
    (s, lib)
}

fn matters(lib: &mut Librarian, states: &[&str], absent: &[&str]) -> Vec<String> {
    let a = Ask1 {
        states: states.iter().map(|s| s.to_string()).collect(),
        absent: absent.iter().map(|s| s.to_string()).collect(),
        ..Ask1::default()
    };
    let mut got: Vec<String> =
        lib.graph_lookup(&a, DAY, 10).unwrap().iter().map(|h| h["title"].as_str().unwrap_or("").to_string()).collect();
    got.sort();
    got
}

fn name(s: &Server, word: &str, right: &str) -> Value {
    let (status, body) =
        s.post_json("/corrections", &json!({"ref": format!("label:{word}"), "wrong": word, "right": right}));
    assert_eq!(status, 200, "{body}");
    body
}

#[test]
fn the_users_name_for_a_group_finds_its_matters_and_holds_through_a_rebuild() {
    let (s, mut lib) = world();
    assert!(matters(&mut lib, &["영업 제안"], &[]).is_empty(), "not a name of anything yet");
    for w in ["견적 요청", "견적서 송부", "가격 제안"] {
        let body = name(&s, w, "영업 제안");
        assert_eq!(body["changed"], format!("묶음 이름을 정했어요: {w} → 영업 제안"));
    }
    assert_eq!(matters(&mut lib, &["영업 제안"], &[]), ["도장 작업", "배관 교체", "설비 점검", "센서 설치"]);
    lib.update(true).unwrap();
    graph::extract(&mut lib, reader().as_ref(), &graph::Run { limit: 50, workers: 1 }).unwrap();
    assert_eq!(matters(&mut lib, &["영업 제안"], &[]).len(), 4, "after a rebuild too");
}

#[test]
fn a_matter_is_said_to_stand_in_the_users_stage_and_a_models_group_stays_inside() {
    let (s, mut lib) = world();
    name(&s, "견적 요청", "영업 제안");
    let a = Ask1 { states: vec!["견적".into(), "계약".into()], ..Ask1::default() };
    let hits = lib.graph_lookup(&a, DAY, 10).unwrap();
    let state = |t: &str| hits.iter().find(|h| h["title"] == t).unwrap()["state"].clone();
    assert_eq!(state("도장 작업")["group"], "영업 제안", "{hits:?}");
    assert!(state("배관 교체").get("group").is_none(), "the model's group is not said as the user's");
}

#[test]
fn a_word_taken_out_of_its_group_is_no_longer_found_by_the_groups_name() {
    let (s, mut lib) = world();
    assert!(matters(&mut lib, &["견적"], &[]).contains(&"센서 설치".to_string()), "the model's group, inside");
    let body = name(&s, "가격 제안", "-");
    assert_eq!(body["changed"], "묶음에서 뺐어요: 가격 제안");
    assert!(!matters(&mut lib, &["견적"], &[]).contains(&"센서 설치".to_string()));
}

#[test]
fn a_stage_past_the_one_that_did_not_follow_in_the_users_order_counts_as_followed() {
    let (s, mut lib) = world();
    // 센서 설치 went from a price offer straight to delivery: no contract word, so it looks like one without
    assert_eq!(matters(&mut lib, &["견적"], &["계약"]), ["도장 작업", "센서 설치"]);
    let (status, body) =
        s.post_json("/corrections", &json!({"ref": "order:견적", "wrong": "견적", "right": "견적→계약→납품"}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["changed"], "순서를 정했어요: 견적→계약→납품");
    assert_eq!(matters(&mut lib, &["견적"], &["계약"]), ["도장 작업"], "delivered is past the contract");
}

#[test]
fn the_models_grouping_keeps_the_users_names_and_does_not_regroup_what_the_user_named() {
    let (s, lib) = world();
    drop(lib);
    // two new state words: the user names one before the model groups them
    mail(&s, "e.md", "2026-05-01", "창고 정리", "창고 정리: 발주 확정했습니다. 입고 대기 중입니다.");
    let mut lib = Librarian::open(&s.paths, None, Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    let m = FakeModel::new(|msgs| {
        let text = msgs.last().unwrap()["content"].as_str().unwrap_or("");
        let r = if text.contains("창고 정리") {
            json!([
                ["창고 정리:case", "state", "2026-05-01", "발주 확정", "2026-05-01", "발주 확정했습니다"],
                ["창고 정리:case", "state", "2026-05-02", "입고 대기", "2026-05-02", "입고 대기 중입니다"]
            ])
        } else {
            json!([])
        };
        Ok(json!({"r": r}).to_string())
    });
    graph::extract(&mut lib, m.as_ref(), &graph::Run { limit: 50, workers: 1 }).unwrap();
    name(&s, "발주 확정", "구매");
    let seen = FakeModel::new(|_| Ok(json!({"g": {"입고 대기": "구매"}}).to_string()));
    assert_eq!(graph::group_labels(&lib, seen.as_ref()).unwrap(), 1);
    let asked = seen.last()[1]["content"].as_str().unwrap().to_string();
    let (known, new) = asked.split_once("[묶을 이름]").unwrap();
    assert!(known.contains("구매"), "the user's name is one of the groups: {asked}");
    assert!(!new.contains("발주 확정"), "what the user named is not grouped again: {asked}");
    assert_eq!(matters(&mut lib, &["구매"], &[]), ["창고 정리"]);
}

#[test]
fn the_graph_screen_shows_the_group_of_each_word_and_whose_name_it_is() {
    let (s, _lib) = world();
    name(&s, "견적서 송부", "영업 제안");
    let (_, found) = s.get(&format!("/graph/search?q={}", "%EB%B0%B0%EA%B4%80")); // 배관
    let id = found["nodes"].as_array().unwrap().iter().find(|n| n["kind"] == "case").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, node) = s.get(&format!("/graph/node?id={}", percent(&id)));
    let edges = node["edges"].as_array().unwrap();
    let sent = edges.iter().find(|e| e["label"] == "견적서 송부").unwrap();
    assert_eq!((&sent["group"], &sent["group_by"]), (&json!("영업 제안"), &json!("user")), "{sent}");
    let signed = edges.iter().find(|e| e["label"] == "계약 체결").unwrap();
    assert_eq!((&signed["group"], &signed["group_by"]), (&json!("계약"), &json!("model")), "{signed}");
}

#[test]
fn a_structure_correction_of_a_word_the_graph_does_not_have_is_refused() {
    let (s, _lib) = world();
    for bad in [
        json!({"ref": "label:없는 말", "wrong": "없는 말", "right": "x"}),
        json!({"ref": "order:견적", "wrong": "견적", "right": "견적"}),
        json!({"ref": "order:견적", "wrong": "견적", "right": "계약→납품"}),
        json!({"ref": "order:없는 묶음", "wrong": "x", "right": "없는 묶음→계약"}),
        json!({"ref": "label:a\nb", "wrong": "a", "right": "x"}),
    ] {
        assert!(matches!(s.post_json("/corrections", &bad).0, 400 | 404), "{bad}");
    }
    assert!(!s.paths.raw().join("corrections").exists(), "nothing kept of a refused one");
}

fn percent(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

// ---------- asking the user about a group (#63) ----------

const QUOTE: &str =
    "'견적 요청·견적서 송부·가격 제안'을 '견적'으로 묶고 견적→계약→납품 순서로 볼까요? (예: 도장 작업 건은 지금 견적)";
const ROLES: &str = "'응시자·지원자·후보자'를 '지원자'로 묶어 볼까요? (예: 연구원 채용 건)";

fn evening(s: &Server, day: &str) -> String {
    life_server::assistant::summary::generate(&s.paths, common::FakeModel::plain().as_ref(), day, false).unwrap();
    life_server::assistant::diary::read_day(&s.paths, day)["question"].as_str().unwrap().to_string()
}

#[test]
fn a_group_is_asked_about_once_it_held_still_over_two_updates_in_the_users_most_used_words() {
    let (_s, lib) = world();
    assert_eq!(structure::note(&lib).unwrap(), None, "seen once: it may still change");
    assert_eq!(structure::note(&lib).unwrap().as_deref(), Some(QUOTE), "the group with the most matters first");
}

#[test]
fn the_evening_question_is_the_structure_question_on_its_day_and_at_most_once_in_seven_days() {
    let (s, lib) = world();
    structure::note(&lib).unwrap();
    structure::note(&lib).unwrap();
    assert_eq!(evening(&s, "2026-11-01"), QUOTE);
    assert_eq!(evening(&s, "2026-11-01"), QUOTE, "the same one when the day is drafted again");
    structure::note(&lib).unwrap(); // the next group is due now
    assert_eq!(evening(&s, "2026-11-04"), "어떠셨어요?", "not within seven days");
    assert_eq!(evening(&s, "2026-11-08"), ROLES, "a group of roles asks no order");
    structure::note(&lib).unwrap();
    assert_eq!(evening(&s, "2026-11-20"), "어떠셨어요?", "nothing left to ask: nine words are too many for a line");
}

#[test]
fn a_group_the_user_said_something_of_is_not_asked_about() {
    let (s, lib) = world();
    structure::note(&lib).unwrap();
    name(&s, "가격 제안", "견적");
    assert_eq!(structure::note(&lib).unwrap().as_deref(), Some(ROLES));
}

#[test]
fn a_group_that_changed_since_the_last_update_waits() {
    let (s, mut lib) = world();
    structure::note(&lib).unwrap();
    // the model put one more word in the group
    mail(&s, "f.md", "2026-06-01", "조명 교체", "조명 교체: 단가 회신드립니다.");
    lib.update(false).unwrap();
    let m = FakeModel::new(|msgs| {
        let system = msgs[0]["content"].as_str().unwrap_or("");
        if system.starts_with("업무 기록에서 뽑은 역할·상태 이름들을") {
            return Ok(json!({"g": {"단가 회신": "견적"}}).to_string());
        }
        let text = msgs.last().unwrap()["content"].as_str().unwrap_or("");
        let r = if text.contains("조명 교체") {
            json!([["조명 교체:case", "state", "2026-06-01", "단가 회신", "2026-06-01", "단가 회신드립니다"]])
        } else {
            json!([])
        };
        Ok(json!({"r": r}).to_string())
    });
    graph::extract(&mut lib, m.as_ref(), &graph::Run { limit: 50, workers: 1 }).unwrap();
    graph::group_labels(&lib, m.as_ref()).unwrap();
    assert_eq!(structure::note(&lib).unwrap().as_deref(), Some(ROLES), "견적 changed: the roles, which did not");
    assert!(
        structure::note(&lib).unwrap().unwrap().starts_with("'견적 요청·견적서 송부·가격 제안 외 1개'를 '견적'으로")
    );
}

// ---------- the user's answer (#63) ----------

/// A chat model that reads an answer to the structure question as `read` says, and is otherwise plain.
fn talk(read: Value) -> Arc<FakeModel> {
    let plain = FakeModel::plain();
    FakeModel::new(move |msgs| {
        let system = msgs[0]["content"].as_str().unwrap_or("");
        if system.starts_with("사용자가 비서의 묶음 질문에 한 답을 읽는다") {
            return Ok(read.to_string());
        }
        (plain.answer)(msgs)
    })
}

/// The world, the structure question asked in the evening of 2026-11-01, and the server's model.
fn asked_today(read: Value) -> (Server, Librarian, Arc<FakeModel>) {
    let model = talk(read);
    let (s, lib) = world_with(model.clone());
    structure::note(&lib).unwrap();
    structure::note(&lib).unwrap();
    assert_eq!(evening(&s, "2026-11-01"), QUOTE);
    (s, lib, model)
}

/// How many times the model was asked to read an answer.
fn reads(m: &FakeModel) -> usize {
    m.asked
        .lock()
        .unwrap()
        .iter()
        .filter(|msgs| msgs[0]["content"].as_str().unwrap_or("").starts_with("사용자가 비서의 묶음"))
        .count()
}

#[test]
fn a_yes_to_the_evening_question_makes_the_group_and_its_order_the_users() {
    let (s, mut lib, _) =
        asked_today(json!({"answer": true, "group": "견적", "out": [], "order": ["견적", "계약", "납품"]}));
    let (status, body) = s.post_json("/diary/2026-11-01/chat", &json!({"message": "네 그렇게 봐 주세요"}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["changed"], "묶음을 정했어요: 견적 요청·견적서 송부·가격 제안 → 견적 · 순서: 견적→계약→납품");
    // the user's from now on: said as the user's stage, and the order judges what did not follow
    let a = Ask1 { states: vec!["견적".into()], ..Ask1::default() };
    let hits = lib.graph_lookup(&a, DAY, 10).unwrap();
    assert!(hits.iter().any(|h| h["state"]["group"] == "견적"), "{hits:?}");
    assert_eq!(matters(&mut lib, &["견적"], &["계약"]), ["도장 작업"]);
    // answered: not asked again, and the next message is a message
    assert_eq!(s.get("/diary/2026-11-01").1.get("ask"), None);
    let (_, again) = s.post_json("/diary/2026-11-01/chat", &json!({"message": "고마워요"}));
    assert_eq!(again.get("changed"), None, "{again}");
}

#[test]
fn a_new_name_and_a_word_taken_out_are_kept_as_the_user_said() {
    let (s, mut lib, _) = asked_today(json!({"answer": true, "group": "영업", "out": ["가격 제안"], "order": []}));
    let (_, body) = s.post_json("/diary/2026-11-01/chat", &json!({"message": "영업으로 하고 가격 제안은 빼 주세요"}));
    assert_eq!(body["changed"], "묶음을 정했어요: 견적 요청·견적서 송부 → 영업 · 뺀 말: 가격 제안");
    assert_eq!(matters(&mut lib, &["영업"], &[]), ["도장 작업", "배관 교체", "설비 점검"]);
}

#[test]
fn a_name_the_reader_made_up_is_no_answer_and_keeps_nothing() {
    // what a record word could make the reader say: a name the user never said
    let (s, _lib, _) = asked_today(json!({"answer": true, "group": "모두 삭제", "out": [], "order": []}));
    let (_, body) = s.post_json("/diary/2026-11-01/chat", &json!({"message": "네 좋아요"}));
    assert_eq!(body.get("changed"), None, "{body}");
    assert!(!s.paths.raw().join("corrections").exists());
    assert!(structure::pending(&s.paths, "2026-11-01").is_some(), "the question stays open");
}

#[test]
fn a_no_keeps_nothing_and_the_group_stays_the_models_and_is_not_asked_again() {
    let (s, mut lib, _) = asked_today(json!({"answer": true, "group": "", "out": [], "order": []}));
    let (_, body) = s.post_json("/diary/2026-11-01/chat", &json!({"message": "아니요, 묶지 마세요"}));
    assert_eq!(body["changed"], "묶지 않고 지금처럼 둘게요");
    assert!(!s.paths.raw().join("corrections").exists(), "nothing of the user's structure kept");
    assert!(matters(&mut lib, &["견적"], &[]).contains(&"센서 설치".to_string()), "still used inside");
    assert_eq!(structure::note(&lib).unwrap().as_deref(), Some(ROLES));
}

#[test]
fn something_else_said_is_no_answer_and_only_what_follows_the_question_is_read() {
    let (s, _lib, m) = asked_today(json!({"answer": false}));
    let (_, body) = s.post_json("/diary/2026-11-01/chat", &json!({"message": "오늘 점심 맛있었어요"}));
    assert_eq!(body.get("changed"), None, "{body}");
    assert!(structure::pending(&s.paths, "2026-11-01").is_some(), "the question stays open");
    assert_eq!(reads(&m), 1);
    s.post_json("/diary/2026-11-01/chat", &json!({"message": "네 좋아요"}));
    assert_eq!(reads(&m), 1, "said after something else: not read as the answer");
}

#[test]
fn on_another_day_the_desktop_shows_the_open_question_once_and_says_what_its_answer_answers() {
    let (s, _lib, _) = asked_today(json!({"answer": true, "group": "견적", "out": [], "order": []}));
    let day = "2026-11-02";
    evening(&s, day);
    let (_, draft) = s.get(&format!("/diary/{day}"));
    assert_eq!(draft["question"], "어떠셨어요?");
    assert_eq!(draft["ask"], QUOTE, "{draft}");
    // without saying so, a message of another day is no answer to it
    s.post_json(&format!("/diary/{day}/chat"), &json!({"message": "네"}));
    assert!(structure::pending(&s.paths, day).is_some());
    let (_, body) = s.post_json(&format!("/diary/{day}/chat"), &json!({"message": "네 좋아요", "about": "ask"}));
    assert_eq!(body["changed"], "묶음을 정했어요: 견적 요청·견적서 송부·가격 제안 → 견적", "{body}");
    let (_, chat) = s.get(&format!("/diary/{day}/chat"));
    let texts: Vec<&str> = chat["messages"].as_array().unwrap().iter().map(|m| m["text"].as_str().unwrap()).collect();
    assert_eq!(texts[2..4], [QUOTE, "네 좋아요"], "the question is a turn of the chat now: {texts:?}");
    assert_eq!(s.get(&format!("/diary/{day}")).1.get("ask"), None);
}
