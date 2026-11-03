//! The eval and asking back (#93): a question the assistant asks back about is scored by whether the candidates it
//! lists hold the matter meant (it is no miss for having asked), and how often it asked is counted on its own.
use std::fs;
use std::path::Path;
use std::time::Duration;

use life_server::config::Paths;
use life_server::librarian::Librarian;
use life_server::librarian::eval::{self, Kind};
use life_server::llm::{Ask, Model};
use serde_json::Value;

const ME: &str = "나 <me@x.com>";

/// A matter of `n` mails of one thread, a day apart from `day`; the user answers it once.
fn matter(p: &Paths, tag: &str, subject: &str, day: &str, n: usize) {
    let start = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap();
    for i in 0..n {
        let d = (start + chrono::Duration::days(i as i64)).format("%Y-%m-%d").to_string();
        let name = format!("{tag}{i}.md");
        let refs = if i == 0 { "[]".to_string() } else { format!("[<{tag}0.md@x>]") };
        let path = p.vault.join("Mail").join(&d[..4]).join(&name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let subject = if i == 0 { subject.to_string() } else { format!("RE: {subject}") };
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

struct One;
impl Model for One {
    fn complete(&self, _: &[Value], _: Ask) -> anyhow::Result<String> {
        Ok(r#"{"search": true, "words": ["견적"], "one": true}"#.into())
    }
}

fn thread(lib: &Librarian, p: &Paths, name: &str) -> String {
    lib.thread_of(&p.vault.join("Mail/2026").join(name).to_string_lossy()).unwrap()
}

fn status(p: &Paths, t: &str) -> Vec<eval::Case> {
    let f: &Path = &p.data.join("recall.toml");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(f, format!("[[case]]\nkind = \"status\"\nq = \"견적 건 어떻게 됐지\"\nthread = \"{t}\"\n")).unwrap();
    eval::load(f).unwrap()
}

fn built(sizes: &[usize]) -> (tempfile::TempDir, Paths, Librarian) {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    fs::create_dir_all(p.vault.join("Events")).unwrap();
    for (i, n) in sizes.iter().enumerate() {
        let tag = ["a", "b", "c", "d"][i];
        matter(&p, tag, &format!("견적 {tag}"), &format!("2026-0{}-01", i + 3), *n);
    }
    let mut lib = Librarian::open(&p, None, Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    (dir, p, lib)
}

#[test]
fn asked_back_with_the_matter_among_the_candidates_is_found_and_counted_as_asked() {
    let (_d, p, mut lib) = built(&[4, 3, 3, 2]);
    let cs = status(&p, &thread(&lib, &p, "b0.md"));
    let out = eval::run(&mut lib, Some(&One), &p.vault, &cs, 8).unwrap();
    assert!(out[0].asked);
    assert!(out[0].model_rank.is_some(), "the matter is among the candidates: found");
    let s = eval::summary(&out);
    assert_eq!(s[&Kind::Status].model, (1, 1));
    assert_eq!(s[&Kind::Status].asked, 1);
    assert_eq!(s[&Kind::Status].asked_label().as_deref(), Some("되물음 1/1"), "out of the questions asked");
}

#[test]
fn asked_back_without_the_matter_among_the_candidates_is_a_miss_and_still_counted_as_asked() {
    let (_d, p, mut lib) = built(&[4, 3, 3, 2]);
    let cs = status(&p, &thread(&lib, &p, "d0.md")); // the fourth: three candidates at most
    let out = eval::run(&mut lib, Some(&One), &p.vault, &cs, 8).unwrap();
    assert!(out[0].asked);
    assert_eq!(out[0].model_rank, None);
    let s = eval::summary(&out);
    assert_eq!(s[&Kind::Status].model, (0, 1));
    assert_eq!(s[&Kind::Status].asked, 1);
}

#[test]
fn answered_without_asking_is_ranked_as_before_and_not_counted_as_asked() {
    let (_d, p, mut lib) = built(&[11, 2]);
    let cs = status(&p, &thread(&lib, &p, "a0.md"));
    let out = eval::run(&mut lib, Some(&One), &p.vault, &cs, 8).unwrap();
    assert!(!out[0].asked);
    assert_eq!(out[0].model_rank, Some(1));
    let s = eval::summary(&out);
    assert_eq!(s[&Kind::Status].model, (1, 1));
    assert_eq!(s[&Kind::Status].asked, 0);
    assert_eq!(s[&Kind::Status].asked_label().as_deref(), Some("되물음 0/1"), "not asking is shown too");
}

#[test]
fn without_the_model_no_asking_back_is_shown() {
    let (_d, p, mut lib) = built(&[4, 3]);
    let cs = status(&p, &thread(&lib, &p, "a0.md"));
    let out = eval::run(&mut lib, None, &p.vault, &cs, 8).unwrap();
    assert_eq!(eval::summary(&out)[&Kind::Status].asked_label(), None);
}
