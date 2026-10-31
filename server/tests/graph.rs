//! The graph (#45): who and what the records are about and how they stand to each other. Rules make what is sure
//! (people by address, organizations by mail domain, who belongs where, the matters); the model adds the rest from
//! each record, each edge with the record it came from and the words that say it. The model's answers are kept by the
//! record's hash: the graph is made again without asking it twice.
mod common;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use common::FakeModel;
use life_server::config::Paths;
use life_server::librarian::Librarian;
use life_server::librarian::graph;
use serde_json::json;

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

    #[allow(clippy::too_many_arguments)]
    fn mail(&self, name: &str, day: &str, from: &str, to: &str, subject: &str, body: &str, sent: bool) {
        let path = self.p.vault.join("Mail").join(&day[..4]).join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = if sent { "account/Sent Messages" } else { "account/INBOX" };
        fs::write(
            path,
            format!("---\ntype: mail\nmail_id: <{name}@x>\nmail_source: {source}\ndate: {day}\ntime: 09:00\nfrom: {from}\nto: [{to}]\ncc: []\nsubject: \"{subject}\"\nreferences: []\n---\n## Message\n{body}\n"),
        )
        .unwrap();
    }

    fn lib(&self) -> Librarian {
        let mut lib = Librarian::open(&self.p, None, Duration::from_secs(5)).unwrap();
        lib.update(false).unwrap();
        lib
    }
}

const ME: &str = "나 <me@mine.co.kr>";

fn world() -> T {
    let t = T::new();
    t.mail(
        "a.md",
        "2026-03-02",
        "김담당 <kim@client.co.kr>",
        ME,
        "설비 점검 견적",
        "점검 견적서 보내 주세요. 다음 주까지 부탁드립니다.",
        false,
    );
    t.mail("b.md", "2026-03-03", ME, "김담당 <kim@client.co.kr>", "RE: 설비 점검 견적", "견적서 첨부합니다.", true);
    t.mail("c.md", "2026-03-04", "친구 <pal@gmail.com>", ME, "주말", "밥 먹자", false);
    t
}

/// A model that answers every record with the same relations (or fails).
fn model(rels: serde_json::Value) -> Arc<FakeModel> {
    FakeModel::new(move |_| Ok(json!({"r": rels}).to_string()))
}

#[test]
fn the_rules_know_people_by_address_and_organizations_by_mail_domain_but_not_a_public_one() {
    let t = world();
    let lib = t.lib();
    let kim = lib.node("person:kim@client.co.kr").unwrap();
    assert_eq!(kim.name, "김담당");
    assert!(lib.node("org:client.co.kr").is_some());
    assert!(lib.node("org:gmail.com").is_none(), "a public mail service is not where anyone works");
    let member = lib.edges_from("person:kim@client.co.kr", Some("member"));
    assert_eq!(member.len(), 1);
    assert_eq!((member[0].dst.as_str(), member[0].origin.as_str()), ("org:client.co.kr", "rule"));
    assert!(lib.edges_from("person:me@mine.co.kr", Some("member")).is_empty(), "the user is not one of the people");
}

#[test]
fn the_models_relations_come_with_their_record_and_words_and_wrong_ones_are_turned_or_dropped() {
    let t = world();
    let mut lib = t.lib();
    let m = model(json!([
        [
            "김담당 <kim@client.co.kr>:person",
            "requested",
            "점검 견적서:doc",
            "견적서",
            "2026-03-02",
            "점검 견적서 보내 주세요"
        ],
        ["설비 점검:case", "role", "김담당 <kim@client.co.kr>:person", "고객", "2026-03-02", "점검 견적서 보내 주세요"],
        ["설비 점검:case", "state", "2026-03-02", "견적 요청", "2026-03-02", "견적서 보내 주세요"],
        ["김담당:person", "member", "고객전자:org", "", "2026-03-02", "점검 견적서"],
        ["설비 점검:case", "uses", "점검 견적서:doc", "", "2026-03-02", "점검 견적서"],
        ["김담당:person", "sent", "가짜:doc", "", "2026-03-02", "기록에 없는 말"]
    ]));
    let done = graph::extract(&mut lib, m.as_ref(), &graph::Run { limit: 10, workers: 2 }).unwrap();
    assert_eq!(done.asked, 3, "every mail that is not a mass mail: {done:?}");
    let role = lib.edges_from("person:kim@client.co.kr", Some("role"));
    assert_eq!(role.len(), 1, "turned the right way (the person has the role in the matter), once: {role:?}");
    let r = &role[0];
    assert_eq!((r.label.as_str(), r.since.as_str()), ("고객", "2026-03-02"));
    assert!(r.dst.starts_with("case:"), "the matter is the mail's own: {}", r.dst);
    assert!(r.evidence.contains("점검 견적서"));
    assert!(r.doc.ends_with("a.md"), "where it was said: {}", r.doc);
    let state = lib.edges_from(&r.dst, Some("state"));
    assert_eq!(state[0].label, "견적 요청");
    assert!(
        lib.edges_from("person:kim@client.co.kr", Some("sent")).iter().all(|e| !e.dst.contains("가짜")),
        "words not in the record"
    );
    assert!(lib.edges_from(&r.dst, Some("uses")).is_empty(), "a matter does not use a document: not an allowed edge");
    // a name the model gives an organization is the name of the one its people mail from
    assert!(lib.node("org:client.co.kr").unwrap().aliases.contains(&"고객전자".to_string()));
}

#[test]
fn the_model_is_asked_once_per_record_and_a_rebuild_or_a_forgotten_record_needs_it_no_more() {
    let t = world();
    let mut lib = t.lib();
    let m =
        model(json!([["설비 점검:case", "role", "김담당 <kim@client.co.kr>:person", "고객", "2026-03-02", "견적"]]));
    graph::extract(&mut lib, m.as_ref(), &graph::Run { limit: 10, workers: 1 }).unwrap();
    let asked = m.asked.lock().unwrap().len();
    assert_eq!(graph::extract(&mut lib, m.as_ref(), &graph::Run { limit: 10, workers: 1 }).unwrap().asked, 0);
    lib.update(true).unwrap(); // the tables made again
    assert_eq!(m.asked.lock().unwrap().len(), asked, "the answers are kept by the record");
    assert!(!lib.edges_from("person:kim@client.co.kr", Some("role")).is_empty(), "and put back from there");
    fs::remove_file(t.p.vault.join("Mail/2026/a.md")).unwrap();
    fs::remove_file(t.p.vault.join("Mail/2026/b.md")).unwrap();
    lib.update(false).unwrap();
    assert!(lib.edges_from("person:kim@client.co.kr", Some("role")).is_empty(), "gone with its records");
}

#[test]
fn a_model_that_fails_or_says_nonsense_leaves_the_graph_as_it_was() {
    let t = world();
    let mut lib = t.lib();
    let down = FakeModel::new(|_| anyhow::bail!("model down"));
    let done = graph::extract(&mut lib, down.as_ref(), &graph::Run { limit: 10, workers: 2 }).unwrap();
    assert_eq!((done.asked, done.failed), (3, 3));
    let junk = FakeModel::new(|_| Ok("not json at all".into()));
    graph::extract(&mut lib, junk.as_ref(), &graph::Run { limit: 10, workers: 2 }).unwrap();
    assert!(lib.edges_from("person:kim@client.co.kr", Some("role")).is_empty());
    assert!(lib.node("org:client.co.kr").is_some(), "the rules' part stands");
}

#[test]
fn a_role_that_only_says_who_a_mail_went_to_is_no_role() {
    let t = world();
    let mut lib = t.lib();
    let m = model(json!([
        ["김담당 <kim@client.co.kr>:person", "role", "설비 점검:case", "수신자", "2026-03-02", "점검 견적서"],
        ["김담당 <kim@client.co.kr>:person", "role", "설비 점검:case", "CC", "2026-03-02", "점검 견적서"],
        ["김담당 <kim@client.co.kr>:person", "role", "설비 점검:case", "수석", "2026-03-02", "점검 견적서"],
        ["김담당 <kim@client.co.kr>:person", "role", "설비 점검:case", "고객", "2026-03-02", "점검 견적서"]
    ]));
    graph::extract(&mut lib, m.as_ref(), &graph::Run { limit: 10, workers: 1 }).unwrap();
    let labels: Vec<String> =
        lib.edges_from("person:kim@client.co.kr", Some("role")).into_iter().map(|e| e.label).collect();
    assert_eq!(labels, ["고객"]);
}
