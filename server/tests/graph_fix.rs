//! Correcting the graph (#47), as any correction (#10): one pair {wrong, right, ref}, kept as it came, applied on
//! every update so a rebuild keeps it.
//!   org:<domain>  right a name: the user's name for that organization; right `org:<domain>`: the same organization
//!   edge:<key>    right `-`: the relation is not so; else its label is right
mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use common::{FakeModel, Server};
use life_server::assistant::recall::{self, Named, Picked};
use life_server::librarian::Librarian;
use life_server::librarian::graph;
use serde_json::json;

const ME: &str = "나 <me@mine.co.kr>";

#[allow(clippy::too_many_arguments)]
fn mail(s: &Server, name: &str, day: &str, from: &str, to: &str, subject: &str, body: &str, sent: bool) {
    let path = s.paths.vault.join("Mail").join(&day[..4]).join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let source = if sent { "account/Sent Messages" } else { "account/INBOX" };
    fs::write(
        path,
        format!("---\ntype: mail\nmail_id: <{name}@x>\nmail_source: {source}\ndate: {day}\ntime: 09:00\nfrom: {from}\nto: [{to}]\ncc: []\nsubject: \"{subject}\"\nreferences: []\n---\n## Message\n{body}\n"),
    )
    .unwrap();
}

fn reader() -> Arc<FakeModel> {
    FakeModel::new(|msgs| {
        let text = msgs.last().unwrap()["content"].as_str().unwrap_or("");
        let r = if text.contains("결선도 보내드립니다") {
            json!([["담당 <a@dekist.com>:person", "role", "소음센서:case", "고객", "2026-07-23", "결선도"]])
        } else {
            json!([])
        };
        Ok(json!({"r": r}).to_string())
    })
}

fn world() -> (Server, Librarian) {
    let s = Server::start();
    fs::create_dir_all(s.paths.vault.join("Events")).unwrap();
    mail(&s, "d1.md", "2026-07-23", "담당 <a@dekist.com>", ME, "소음센서 결선도", "결선도 보내드립니다", false);
    mail(&s, "d2.md", "2026-07-24", ME, "담당 <a@dekist.com>", "RE: 소음센서 결선도", "받았습니다", true);
    mail(
        &s,
        "c1.md",
        "2026-01-06",
        "지원 <support@cumulocity.atlassian.net>",
        ME,
        "CST-2630 패치 가이드",
        "패치 안내",
        false,
    );
    mail(&s, "c2.md", "2025-10-15", "청구 <billing@cumulocity.com>", ME, "Overdue Invoice", "청구서", false);
    mail(&s, "c3.md", "2025-10-16", ME, "청구 <billing@cumulocity.com>", "RE: Overdue Invoice", "확인", true);
    let mut lib = Librarian::open(&s.paths, None, Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    graph::extract(&mut lib, reader().as_ref(), &graph::Run { limit: 50, workers: 1 }).unwrap();
    (s, lib)
}

fn titles(lib: &mut Librarian, p: Picked) -> Vec<String> {
    match recall::lookup(lib, &p, 8, "2026-10-31", &[]).unwrap() {
        Named::Hits(h) => {
            h.iter().map(|h| h["title"].as_str().unwrap_or("").trim_start_matches("RE: ").to_string()).collect()
        }
        Named::Ask(_) => panic!("asked back"),
    }
}

fn org(name: &str) -> Picked {
    Picked { orgs: vec![name.into()], ..Picked::default() }
}

#[test]
fn the_users_name_for_an_organization_finds_it_and_stays_through_a_rebuild() {
    let (s, mut lib) = world();
    assert!(!titles(&mut lib, org("디키스트")).contains(&"소음센서 결선도".to_string()));
    let (status, body) =
        s.post_json("/corrections", &json!({"ref": "org:dekist.com", "wrong": "dekist.com", "right": "디키스트"}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["changed"], "조직 이름을 더했어요: dekist.com = 디키스트");
    lib.update(false).unwrap();
    assert!(titles(&mut lib, org("디키스트")).contains(&"소음센서 결선도".to_string()));
    lib.update(true).unwrap();
    assert!(titles(&mut lib, org("디키스트")).contains(&"소음센서 결선도".to_string()), "after a rebuild too");
}

#[test]
fn two_domains_said_to_be_one_organization_are_asked_about_together() {
    let (s, mut lib) = world();
    let (_, body) = s.post_json(
        "/corrections",
        &json!({"ref": "org:cumulocity.atlassian.net", "wrong": "cumulocity.atlassian.net", "right": "org:cumulocity.com"}),
    );
    assert_eq!(body["changed"], "같은 조직으로 이었어요: cumulocity.atlassian.net = cumulocity.com");
    lib.update(false).unwrap();
    let got = titles(&mut lib, org("cumulocity"));
    assert!(
        got.contains(&"CST-2630 패치 가이드".to_string()) && got.contains(&"Overdue Invoice".to_string()),
        "{got:?}"
    );
}

#[test]
fn a_wrong_relation_is_taken_away_or_renamed_and_stays_so() {
    let (s, mut lib) = world();
    let e = lib.edges_from("person:a@dekist.com", Some("role"));
    assert_eq!(e.len(), 1);
    let key = graph::edge_key(&e[0]);
    let (_, body) =
        s.post_json("/corrections", &json!({"ref": format!("edge:{key}"), "wrong": "고객", "right": "공급사"}));
    assert_eq!(body["changed"], "관계를 고쳤어요: 고객 → 공급사");
    lib.update(false).unwrap();
    assert_eq!(lib.edges_from("person:a@dekist.com", Some("role"))[0].label, "공급사");
    s.post_json("/corrections", &json!({"ref": format!("edge:{key}"), "wrong": "공급사", "right": "-"}));
    lib.update(true).unwrap();
    assert!(lib.edges_from("person:a@dekist.com", Some("role")).is_empty(), "gone, after a rebuild too");
}

#[test]
fn a_correction_of_something_not_there_is_refused_and_nothing_is_kept() {
    let (s, _lib) = world();
    for bad in [
        json!({"ref": "org:no.such.org", "wrong": "x", "right": "이름"}),
        json!({"ref": "org:dekist.com", "wrong": "x", "right": "org:no.such.org"}),
        json!({"ref": "edge:0123456789abcdef", "wrong": "x", "right": "-"}),
        json!({"ref": "org:../etc", "wrong": "x", "right": "y"}),
    ] {
        assert!(matches!(s.post_json("/corrections", &bad).0, 400 | 404), "{bad}");
    }
    assert!(!s.paths.raw().join("corrections").exists(), "nothing kept of a refused one");
}
