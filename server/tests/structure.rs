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
    json!({"견적 요청": "견적", "견적서 송부": "견적", "가격 제안": "견적", "계약 체결": "계약", "납품 완료": "납품"})
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
        } else {
            json!([])
        };
        Ok(json!({"r": r}).to_string())
    })
}

fn world() -> (Server, Librarian) {
    let s = Server::start();
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
        json!({"ref": "label:a\nb", "wrong": "a", "right": "x"}),
    ] {
        assert!(matches!(s.post_json("/corrections", &bad).0, 400 | 404), "{bad}");
    }
    assert!(!s.paths.raw().join("corrections").exists(), "nothing kept of a refused one");
}

fn percent(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}
