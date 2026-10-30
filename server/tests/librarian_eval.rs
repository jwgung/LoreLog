//! Measuring the librarian: a file of questions with the record each should find, the rank it is found at with the
//! user's words and with the words the model picks, and the share per kind of question.
use std::fs;
use std::path::Path;
use std::time::Duration;

use life_server::config::Paths;
use life_server::librarian::Librarian;
use life_server::librarian::eval::{self, Kind};
use life_server::llm::{Ask, Model};
use serde_json::Value;

fn mail(p: &Paths, name: &str, subject: &str, body: &str, day: &str, from: &str, refs: &str) {
    let path = p.vault.join("Mail").join(&day[..4]).join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(
            "---\ntype: mail\nmail_id: <{name}@x>\nmail_source: account/INBOX\ndate: {day}\ntime: 09:00\nfrom: {from}\nto: 나 <me@x.com>\nsubject: \"{subject}\"\nin_reply_to: \nreferences: {refs}\nattachments: []\n---\n## Notes\n\n## Message\n{body}\n"
        ),
    )
    .unwrap();
}

/// Picks the words a person would not have typed: "포트" for anything about opening a port.
struct Picker;
impl Model for Picker {
    fn complete(&self, messages: &[Value], _: Ask) -> anyhow::Result<String> {
        let q = messages[1]["content"].as_str().unwrap_or("");
        Ok(if q.contains("열어") { r#"{"search": true, "words": ["포트"]}"# } else { r#"{"search": false}"# }.into())
    }
}

fn setup() -> (tempfile::TempDir, Paths, Librarian) {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    fs::create_dir_all(p.vault.join("Events")).unwrap();
    mail(&p, "a1.md", "포트 포워딩 요청", "방화벽에서 포트를 열어 주세요", "2026-04-01", "보안팀 <sec@x.com>", "[]");
    mail(&p, "a2.md", "RE: 포트 포워딩 요청", "처리했습니다", "2026-04-03", "보안팀 <sec@x.com>", "[<a1.md@x>]");
    mail(&p, "b1.md", "주간 회의록", "다음 주 일정 공유", "2026-05-01", "영희 <yh@x.com>", "[]");
    mail(&p, "b2.md", "견적 요청", "견적서를 보내 드립니다", "2026-06-01", "영희 <yh@x.com>", "[]");
    let mut lib = Librarian::open(&p, None, Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    (dir, p, lib)
}

fn cases(p: &Paths, text: &str) -> Vec<eval::Case> {
    let f = p.data.join("recall.toml");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(&f, text).unwrap();
    eval::load(&f).unwrap()
}

fn thread(lib: &Librarian, vault: &Path, name: &str) -> String {
    let f = vault.join("Mail/2026").join(name);
    lib.thread_of(&f.to_string_lossy()).unwrap()
}

#[test]
fn a_question_is_found_with_the_words_the_model_picks_even_when_its_own_words_miss() {
    let (_d, p, lib) = setup();
    let t = thread(&lib, &p.vault, "a1.md");
    let cs =
        cases(&p, &format!("[[case]]\nkind = \"status\"\nq = \"방화벽 열어 달라던 거 됐나?\"\nthread = \"{t}\"\n"));
    let mut lib = lib;
    let out = eval::run(&mut lib, Some(&Picker), &p.vault, &cs, 8).unwrap();
    assert_eq!(out[0].kind, Kind::Status);
    assert_eq!(out[0].model_words.as_deref(), Some(&["포트".to_string()][..]));
    assert_eq!(out[0].model_rank, Some(1), "the thread is found through the picked word");
    let s = eval::summary(&out);
    assert_eq!(s[&Kind::Status].model, (1, 1));
}

#[test]
fn a_person_is_found_through_any_of_their_latest_threads_and_the_share_is_counted() {
    let (_d, p, lib) = setup();
    let cs = cases(&p, "[[case]]\nkind = \"person\"\nq = \"영희\"\nperson = \"YH@x.com\"\n");
    let mut lib = lib;
    let out = eval::run(&mut lib, None, &p.vault, &cs, 8).unwrap();
    assert_eq!(out[0].raw_rank, Some(1));
    assert_eq!(out[0].person_share, Some((2, 2)), "both hits are with that person");
    assert_eq!(out[0].model_rank, None, "no model: only the user's words are measured");
}

#[test]
fn a_question_whose_record_is_not_found_is_a_miss_and_pending_questions_are_not_scored() {
    let (_d, p, lib) = setup();
    let t = thread(&lib, &p.vault, "b2.md");
    let cs = cases(
        &p,
        &format!(
            "[[case]]\nkind = \"precedent\"\nq = \"방화벽\"\nthread = \"{t}\"\n\n[[case]]\nkind = \"pending\"\nq = \"이번 달에 뭐 있지\"\n"
        ),
    );
    let mut lib = lib;
    let out = eval::run(&mut lib, None, &p.vault, &cs, 8).unwrap();
    assert_eq!(out[0].raw_rank, None);
    let s = eval::summary(&out);
    assert_eq!(s[&Kind::Precedent].raw, (0, 1));
    assert!(!s.contains_key(&Kind::Pending));
}

#[test]
fn a_case_without_its_target_is_refused() {
    let (_d, p, _lib) = setup();
    let f = p.data.join("bad.toml");
    fs::write(&f, "[[case]]\nkind = \"status\"\nq = \"뭐지\"\n").unwrap();
    assert!(eval::load(&f).unwrap_err().to_string().contains("thread"));
    fs::write(&f, "[[case]]\nkind = \"weird\"\nq = \"뭐지\"\n").unwrap();
    assert!(eval::load(&f).is_err());
}
