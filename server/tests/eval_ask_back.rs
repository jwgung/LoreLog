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

// ---- cases that expect asking back (ADR 0006) ----

fn file(p: &Paths, text: &str) -> std::path::PathBuf {
    let f = p.data.join("expect.toml");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(&f, text).unwrap();
    f
}

fn ask_case(candidates: &[String]) -> String {
    let l: Vec<String> = candidates.iter().map(|c| format!("\"{c}\"")).collect();
    format!(
        "[[case]]\nkind = \"status\"\nq = \"견적 건 어떻게 됐지\"\nexpect = \"ask\"\ncandidates = [{}]\n",
        l.join(", ")
    )
}

struct Nothing;
impl Model for Nothing {
    fn complete(&self, _: &[Value], _: Ask) -> anyhow::Result<String> {
        Ok(r#"{"search": false}"#.into())
    }
}

#[test]
fn an_ask_case_is_asked_back_and_scored_by_how_many_of_its_candidates_were_listed() {
    let (_d, p, mut lib) = built(&[4, 3, 3, 2]);
    // three listed at most: a, b, c; the fourth matter is not among them
    let cs = [thread(&lib, &p, "a0.md"), thread(&lib, &p, "d0.md")];
    let cases = eval::load(&file(&p, &ask_case(&cs))).unwrap();
    let out = eval::run(&mut lib, Some(&One), &p.vault, &cases, 8).unwrap();
    assert!(out[0].asked_matters);
    assert_eq!(out[0].candidate_hit, Some(0.5), "one of two listed, over min(3, 2)");
    let s = eval::ask_summary(&out);
    assert_eq!((s.should_ask.0, s.should_ask.1), (1, 1));
    assert_eq!(s.candidates_avg(), Some(0.5));
    assert_eq!(s.should_answer, (0, 0));
    // asking back is no found/miss count of the kind
    assert!(!eval::summary(&out).contains_key(&Kind::Status));
}

#[test]
fn an_ask_case_that_is_answered_is_not_asked_and_has_no_candidate_hit() {
    let (_d, p, mut lib) = built(&[11, 2]);
    let cs = [thread(&lib, &p, "a0.md"), thread(&lib, &p, "b0.md")];
    let cases = eval::load(&file(&p, &ask_case(&cs))).unwrap();
    let out = eval::run(&mut lib, Some(&One), &p.vault, &cases, 8).unwrap();
    assert!(!out[0].asked_matters);
    assert_eq!(out[0].candidate_hit, None);
    let s = eval::ask_summary(&out);
    assert_eq!(s.should_ask, (0, 1));
    assert_eq!(s.candidates_avg(), None);
    // a model that looks nothing up does not ask either
    let out = eval::run(&mut lib, Some(&Nothing), &p.vault, &cases, 8).unwrap();
    assert_eq!(eval::ask_summary(&out).should_ask, (0, 1));
}

#[test]
fn a_case_without_expect_that_is_asked_back_counts_as_asked_unnecessarily() {
    let (_d, p, mut lib) = built(&[4, 3, 3, 2]);
    let cs = status(&p, &thread(&lib, &p, "b0.md"));
    let out = eval::run(&mut lib, Some(&One), &p.vault, &cs, 8).unwrap();
    let s = eval::ask_summary(&out);
    assert_eq!(s.should_answer, (1, 1));
    assert_eq!(s.should_ask, (0, 0));
    // an explicit "answer" is the same
    let f = file(
        &p,
        &format!(
            "[[case]]\nkind = \"status\"\nq = \"견적 건 어떻게 됐지\"\nexpect = \"answer\"\nthread = \"{}\"\n",
            thread(&lib, &p, "b0.md")
        ),
    );
    let out = eval::run(&mut lib, Some(&One), &p.vault, &eval::load(&f).unwrap(), 8).unwrap();
    assert_eq!(eval::ask_summary(&out).should_answer, (1, 1));
    // the status score is as before
    assert_eq!(eval::summary(&out)[&Kind::Status].model, (1, 1));
}

#[test]
fn an_ask_case_with_fewer_than_two_candidates_is_refused_naming_the_case_and_the_field() {
    let (_d, p, lib) = built(&[2, 2]);
    let t = thread(&lib, &p, "a0.md");
    let e = eval::load(&file(&p, &ask_case(std::slice::from_ref(&t)))).unwrap_err().to_string();
    assert!(e.contains("case 1") && e.contains("candidates"), "{e}");
    assert!(!e.contains("견적"), "the question is not quoted: {e}");
    let none = "[[case]]\nkind = \"status\"\nq = \"견적 건 어떻게 됐지\"\nexpect = \"ask\"\nthread = \"x\"\n";
    assert!(eval::load(&file(&p, none)).unwrap_err().to_string().contains("candidates"));
}

#[test]
fn an_ask_case_that_also_names_a_thread_is_refused() {
    let (_d, p, lib) = built(&[2, 2]);
    let (a, b) = (thread(&lib, &p, "a0.md"), thread(&lib, &p, "b0.md"));
    let text = format!("{}thread = \"{a}\"\n", ask_case(&[a.clone(), b]));
    let e = eval::load(&file(&p, &text)).unwrap_err().to_string();
    assert!(e.contains("case 1") && e.contains("thread"), "{e}");
    assert!(!e.contains("견적"), "the question is not quoted: {e}");
}

#[test]
fn the_candidates_hit_are_counted_over_the_most_that_are_listed_when_there_are_more() {
    let (_d, p, mut lib) = built(&[4, 3, 3, 2]);
    let cs: Vec<String> = ["a0.md", "b0.md", "c0.md", "d0.md"].iter().map(|n| thread(&lib, &p, n)).collect();
    let cases = eval::load(&file(&p, &ask_case(&cs))).unwrap();
    let out = eval::run(&mut lib, Some(&One), &p.vault, &cases, 8).unwrap();
    assert!(out[0].asked_matters);
    assert_eq!(out[0].candidate_hit, Some(1.0), "three listed of four, over min(3, 4)");
}

/// Two people of one name with as many mails each: who is meant is asked back.
fn namesakes(p: &Paths) {
    for (who, addr) in [("a", "kim@x.com"), ("b", "kim@y.com")] {
        for i in 0..2 {
            let path = p.vault.join("Mail/2026").join(format!("n{who}{i}.md"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                path,
                format!("---\ntype: mail\nmail_id: <n{who}{i}@x>\nmail_source: account/INBOX\ndate: 2026-06-0{}\ntime: 09:00\nfrom: 김영희 <{addr}>\nto: [{ME}]\ncc: []\nsubject: \"안부 {who}{i}\"\nreferences: []\n---\n## Message\n안녕하세요 {i}\n", i + 1),
            )
            .unwrap();
        }
    }
}

struct Kim;
impl Model for Kim {
    fn complete(&self, _: &[Value], _: Ask) -> anyhow::Result<String> {
        Ok(r#"{"search": true, "people": ["김영희"], "words": []}"#.into())
    }
}

#[test]
fn asking_back_about_people_is_needless_for_a_case_to_answer_and_is_no_asking_for_an_ask_case() {
    let (_d, p, mut lib) = built(&[2, 2]);
    namesakes(&p);
    lib.update(false).unwrap();
    let (a, b) = (thread(&lib, &p, "a0.md"), thread(&lib, &p, "b0.md"));
    let answer = eval::load(&file(
        &p,
        &format!("[[case]]\nkind = \"status\"\nq = \"영희 님 건 어떻게 됐지\"\nthread = \"{a}\"\n"),
    ))
    .unwrap();
    let out = eval::run(&mut lib, Some(&Kim), &p.vault, &answer, 8).unwrap();
    assert!(out[0].asked, "the people were asked about");
    assert_eq!(eval::ask_summary(&out).should_answer, (1, 1));
    let ask = eval::load(&file(&p, &ask_case(&[a, b]))).unwrap();
    let out = eval::run(&mut lib, Some(&Kim), &p.vault, &ask, 8).unwrap();
    assert!(!out[0].asked_matters && out[0].candidate_hit.is_none());
    assert_eq!(eval::ask_summary(&out).should_ask, (0, 1));
}
