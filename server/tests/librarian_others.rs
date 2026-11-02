//! A matter is one hit; the other mails of it that differ come with it as `others` (ADR 0003, #94).
use std::fs;
use std::time::Duration;

use life_server::config::Paths;
use life_server::librarian::eval::{self, Kind};
use life_server::librarian::{Exclude, Librarian};

fn mail(p: &Paths, name: &str, subject: &str, body: &str, day: &str, refs: &str) {
    let path = p.vault.join("Mail").join(&day[..4]).join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(
            "---\ntype: mail\nmail_id: <{name}@x>\nmail_source: account/INBOX\ndate: {day}\ntime: 09:00\nfrom: 영희 <yh@x.com>\nto: 나 <me@x.com>\nsubject: \"{subject}\"\nin_reply_to: \nreferences: {refs}\nattachments: []\n---\n## Notes\n\n## Message\n{body}\n"
        ),
    )
    .unwrap();
}

fn open(p: &Paths) -> Librarian {
    fs::create_dir_all(p.vault.join("Events")).unwrap();
    let mut lib = Librarian::open(p, None, Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    lib
}

fn refs_of(h: &serde_json::Value) -> Vec<String> {
    let mut v = vec![h["ref"].as_str().unwrap().to_string()];
    v.extend(h["others"].as_array().into_iter().flatten().map(|o| o["ref"].as_str().unwrap().to_string()));
    v
}

#[test]
fn a_mail_folded_behind_the_hit_of_its_matter_comes_with_it_and_counts_as_shown() {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    mail(&p, "a1.md", "견적 금액", "견적 금액은 100만원입니다", "2026-04-01", "[]");
    mail(&p, "a2.md", "RE: 견적 금액", "견적 금액을 120만원으로 바꿉니다", "2026-04-03", "[<a1.md@x>]");
    mail(&p, "z1.md", "견적 금액 문의", "다른 건의 견적 금액은 900만원", "2026-05-01", "[]");
    let mut lib = open(&p);
    let hits = lib.search("견적 금액", 8, None).unwrap();
    assert_eq!(hits.len(), 2, "one hit per matter, as before");
    let thread = hits.iter().find(|h| h["count"] == 2).expect("the two-mail matter");
    let others = thread["others"].as_array().expect("the folded mail comes along");
    assert_eq!(others.len(), 1);
    let o = &others[0];
    for k in ["day", "title", "snippet", "ref"] {
        assert!(o[k].as_str().is_some_and(|s| !s.is_empty()), "{k}");
    }
    assert!(o["snippet"].as_str().unwrap().contains("만원"));
    // a matter of its own never leaks into another's others
    let single = hits.iter().find(|h| h["count"] == 1).unwrap();
    assert!(single.get("others").is_none_or(|o| o.as_array().unwrap().is_empty()));
    assert!(!refs_of(thread).contains(&single["ref"].as_str().unwrap().to_string()));
    // the hit's own mail is not among its others
    assert_ne!(o["ref"], thread["ref"]);
    assert_eq!(refs_of(thread).len(), 2);
    // the conflict is scored as shown
    let f = p.data.join("c.toml");
    fs::create_dir_all(&p.data).unwrap();
    fs::write(&f, "[[case]]\nkind = \"conflict\"\nq = \"견적 금액\"\nrecords = [\"a1.md@x\", \"a2.md@x\"]\n").unwrap();
    let cs = eval::load(&f).unwrap();
    let out = eval::run(&mut lib, None, &p.vault, &cs, 8).unwrap();
    assert!(out[0].raw_rank.is_some(), "both mails of the matter are shown: found");
    assert_eq!(eval::summary(&out)[&Kind::Conflict].raw, (1, 1));
}

#[test]
fn others_are_at_most_three_a_matter_and_their_snippets_are_short() {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    let long = "가".repeat(600);
    mail(&p, "t0.md", "일정 조율", &format!("일정 조율 {long}"), "2026-04-01", "[]");
    for i in 1..6 {
        mail(
            &p,
            &format!("t{i}.md"),
            "RE: 일정 조율",
            &format!("일정 조율 {long}"),
            &format!("2026-04-0{}", i + 1),
            &format!("[<t{}.md@x>]", i - 1),
        );
    }
    let mut lib = open(&p);
    let hits = lib.search("일정 조율", 8, None).unwrap();
    assert_eq!(hits.len(), 1);
    let others = hits[0]["others"].as_array().unwrap();
    assert_eq!(others.len(), 3);
    for o in others {
        assert!(o["snippet"].as_str().unwrap().chars().count() <= 205, "same length as the hit's own snippet");
        assert_ne!(o["ref"], hits[0]["ref"]);
    }
    // five other mails, three shown: the rest is counted
    assert_eq!(hits[0]["others_more"], 2);
}

#[test]
fn the_others_of_a_file_hit_are_the_mails_of_its_matter_but_the_mail_it_came_with() {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    mail(&p, "a1.md", "건 하나", "첫 안은 100만원", "2026-04-01", "[]");
    mail(&p, "a2.md", "RE: 건 하나", "둘째 안은 120만원", "2026-04-03", "[<a1.md@x>]");
    let sha = "ab".repeat(32);
    let f = p.raw().join("mailfile").join("2026-04-03.ndjson");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(
        &f,
        format!("{{\"type\":\"file\",\"sha256\":\"{sha}\",\"name\":\"도면표.pdf\",\"mail_id\":\"<a2.md@x>\"}}\n"),
    )
    .unwrap();
    let mut lib = open(&p);
    let hits = lib.search("도면표", 8, None).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["source"], "file");
    let refs = refs_of(&hits[0]);
    assert_eq!(refs.len(), 2, "{refs:?}");
    assert!(refs[0].ends_with("a2.md") && refs[1].ends_with("a1.md"), "{refs:?}: its own mail is the hit's ref");
}

#[test]
fn the_others_keep_to_what_the_search_kept() {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    mail(&p, "a1.md", "견적 금액", "견적 금액은 100만원입니다", "2026-04-01", "[]");
    mail(&p, "a2.md", "RE: 견적 금액", "견적 금액은 110만원입니다", "2026-04-03", "[<a1.md@x>]");
    mail(&p, "a3.md", "RE: 견적 금액", "견적 금액은 120만원입니다", "2026-04-05", "[<a1.md@x>]");
    let mut lib = open(&p);
    // the day shown in the prompt another way is not in the others either
    let hits = lib.search("견적 금액", 8, Some(Exclude::Day("2026-04-05"))).unwrap();
    let all = refs_of(&hits[0]);
    assert_eq!(all.len(), 2, "{all:?}");
    assert!(all.iter().all(|r| !r.ends_with("a3.md")), "{all:?}");
    // what is later than the day asked from is not a past record
    let hits = lib.with_people(&["yh@x.com".to_string()], "견적 금액", "2026-04-03", 8).unwrap();
    let all = refs_of(&hits[0]);
    assert_eq!(all.len(), 2, "{all:?}");
    assert!(all.iter().all(|r| !r.ends_with("a3.md")), "{all:?}");
}

#[test]
fn a_copy_of_a_mail_is_one_other_and_a_matter_of_one_mail_has_none() {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    mail(&p, "a1.md", "견적 금액", "견적 금액은 100만원입니다", "2026-04-01", "[]");
    mail(&p, "a2.md", "RE: 견적 금액", "견적 금액은 120만원입니다", "2026-04-03", "[<a1.md@x>]");
    mail(&p, "a2copy.md", "RE: 견적 금액", "견적 금액은 120만원입니다", "2026-04-03", "[<a1.md@x>]");
    mail(&p, "z1.md", "다른 건", "다른 건의 견적 금액", "2026-05-01", "[]");
    let mut lib = open(&p);
    let hits = lib.search("견적 금액", 8, None).unwrap();
    let thread = hits.iter().find(|h| h["case"].is_string() && h["count"].as_u64() > Some(1)).unwrap();
    assert_eq!(thread["others"].as_array().unwrap().len(), 1, "the copy of the hit is not an other: {thread}");
    assert!(thread.get("others_more").is_none());
    let one = hits.iter().find(|h| h["count"] == 1).unwrap();
    assert!(one.get("others").is_none());
}

#[test]
fn two_mails_of_a_day_that_differ_only_beyond_the_snippet_are_two_others() {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    let long = "가".repeat(400);
    mail(&p, "a1.md", "견적 금액", "견적 금액은 100만원입니다", "2026-04-01", "[]");
    mail(&p, "a2.md", "RE: 견적 금액", &format!("견적 금액 {long} 120만원"), "2026-04-03", "[<a1.md@x>]");
    mail(&p, "a3.md", "RE: 견적 금액", &format!("견적 금액 {long} 130만원"), "2026-04-03", "[<a1.md@x>]");
    let mut lib = open(&p);
    let hits = lib.search("견적 금액", 8, None).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(refs_of(&hits[0]).len(), 3, "{}", hits[0]);
}

#[test]
fn a_record_whose_copy_is_shown_counts_as_shown() {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    mail(&p, "a1.md", "견적 금액", "견적 금액은 100만원입니다", "2026-04-01", "[]");
    mail(&p, "a2.md", "RE: 견적 금액", "견적 금액은 120만원입니다", "2026-04-03", "[<a1.md@x>]");
    mail(&p, "a2copy.md", "RE: 견적 금액", "견적 금액은 120만원입니다", "2026-04-03", "[<a1.md@x>]");
    let mut lib = open(&p);
    let f = p.data.join("c.toml");
    fs::create_dir_all(&p.data).unwrap();
    fs::write(
        &f,
        "[[case]]\nkind = \"conflict\"\nq = \"견적 금액\"\nrecords = [\"a1.md@x\", \"a2.md@x\", \"a2copy.md@x\"]\n",
    )
    .unwrap();
    let cs = eval::load(&f).unwrap();
    let out = eval::run(&mut lib, None, &p.vault, &cs, 8).unwrap();
    assert_eq!(eval::summary(&out)[&Kind::Conflict].raw, (1, 1), "the copy says what its twin says");
}
