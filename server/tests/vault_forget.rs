//! What the vault holds and forgetting part of it (#12): by source and by days, the records, the notes, the files
//! nothing else names, their cached text, the to-dos not yet taken, and the index go together.
use std::fs;
use std::path::Path;
use std::time::Duration;

use life_server::config::Paths;
use life_server::librarian::Librarian;
use life_server::vault::{self, Span};
use serde_json::json;

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn mail(p: &Paths, name: &str, day: &str, body: &str) {
    put(
        &p.vault.join("Mail").join(&day[..4]).join(name),
        &format!(
            "---\ntype: mail\nmail_id: <{name}@x>\nmail_source: account/INBOX\ndate: {day}\ntime: 09:00\nfrom: 갑 <a@y.com>\nsubject: \"{body}\"\n---\n## Message\n{body}\n"
        ),
    );
}

fn sighting(p: &Paths, day: &str, sha: &str, bytes: &str) {
    put(&life_server::store::blob_path(&p.data, sha), bytes);
    let f = p.raw().join("mailfile").join(format!("{day}.ndjson"));
    let before = fs::read_to_string(&f).unwrap_or_default();
    put(
        &f,
        &format!("{before}{}\n", json!({"type": "file", "ts": 0, "sha256": sha, "name": "q.txt", "mail_id": "<m@x>"})),
    );
}

fn world() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    fs::create_dir_all(p.vault.join("Events")).unwrap();
    mail(&p, "2026-03-01-0900-a.md", "2026-03-01", "지울메일문구수달");
    mail(&p, "2026-03-20-0900-b.md", "2026-03-20", "남길메일문구너구리");
    let (keep, gone) = ("b".repeat(64), "a".repeat(64));
    sighting(&p, "2026-03-01", &gone, "attachment only of march first");
    sighting(&p, "2026-03-01", &keep, "attachment seen twice");
    sighting(&p, "2026-03-20", &keep, "attachment seen twice");
    put(&p.data.join(format!("cache/attach-text-1/aa/{gone}.txt")), "cached");
    put(&p.raw().join("lorelog/2026-03-01.ndjson"), "{\"type\":\"app\",\"ts\":1}\n");
    put(&p.raw().join("lorelog/2026-03-02.ndjson"), "{\"type\":\"app\",\"ts\":2}\n");
    let todos = json!({"items": [
        {"id": "aaaaaaaaaa", "title": "제안된 일", "source": "메일", "status": "proposed", "day": "2026-03-01"},
        {"id": "bbbbbbbbbb", "title": "받은 일", "source": "메일", "status": "open", "day": "2026-03-01"},
        {"id": "cccccccccc", "title": "다른 날 제안", "source": "메일", "status": "proposed", "day": "2026-03-20"}
    ], "last_extract": 0});
    put(&p.todo_file(), &todos.to_string());
    (dir, p)
}

#[test]
fn what_the_vault_holds_is_shown_by_source_with_its_size_and_days() {
    let (_d, p) = world();
    let held = vault::holdings(&p);
    let mail = held.iter().find(|h| h.source == "mail").unwrap();
    assert_eq!(
        (mail.label.as_str(), mail.first.as_str(), mail.last.as_str()),
        ("메일과 첨부", "2026-03-01", "2026-03-20")
    );
    assert_eq!(mail.files, 6, "2 notes, 2 sighting files, 2 kept files");
    let phone = held.iter().find(|h| h.source == "phone").unwrap();
    assert_eq!((phone.files, phone.first.as_str(), phone.last.as_str()), (2, "2026-03-01", "2026-03-02"));
    assert!(held.iter().all(|h| h.files > 0), "a source with nothing is not listed");
}

#[test]
fn forgetting_days_of_mail_takes_their_notes_records_files_nothing_else_names_and_untaken_todos() {
    let (_d, p) = world();
    let mut lib = Librarian::open(&p, None, Duration::from_secs(5)).unwrap();
    lib.update(false).unwrap();
    assert_eq!(lib.search("지울메일문구수달", 5, None).unwrap().len(), 1);
    let span = Span { from: "2026-03-01".into(), to: "2026-03-10".into() };
    let plan = vault::plan(&p, "mail", &span).unwrap();
    assert_eq!(
        plan.files.len(),
        4,
        "the note, the day's sightings, the file only it named, its text: {:?}",
        plan.files
    );
    assert_eq!(plan.todos, 1);
    assert!(!plan.comes_back, "mail sync remembers what it read: forgotten mail is not read again");
    let span_all = Span { from: "2000-01-01".into(), to: "2100-01-01".into() };
    assert!(vault::plan(&p, "calendar", &span_all).unwrap().comes_back, "a calendar is read whole again on each sync");
    assert!(p.vault.join("Mail/2026/2026-03-01-0900-a.md").exists(), "a plan deletes nothing");
    let done = vault::forget(&p, "mail", &span).unwrap();
    assert_eq!((done.files.len(), done.todos), (4, 1));
    assert!(!p.vault.join("Mail/2026/2026-03-01-0900-a.md").exists());
    assert!(p.vault.join("Mail/2026/2026-03-20-0900-b.md").exists(), "another day stays");
    assert!(!life_server::store::blob_path(&p.data, &"a".repeat(64)).exists());
    assert!(life_server::store::blob_path(&p.data, &"b".repeat(64)).exists(), "still named by a mail that stays");
    assert!(!p.data.join(format!("cache/attach-text-1/aa/{}.txt", "a".repeat(64))).exists(), "its text goes too");
    let todos = fs::read_to_string(p.todo_file()).unwrap();
    assert!(!todos.contains("제안된 일") && todos.contains("받은 일") && todos.contains("다른 날 제안"), "{todos}");
    lib.update(false).unwrap();
    assert!(lib.search("지울메일문구수달", 5, None).unwrap().is_empty(), "the index forgets it on its next update");
    assert_eq!(lib.search("남길메일문구너구리", 5, None).unwrap().len(), 1);
}

#[test]
fn forgetting_phone_records_takes_only_those_days_and_a_bad_request_takes_nothing() {
    let (_d, p) = world();
    let span = Span { from: "2026-03-02".into(), to: "2026-03-02".into() };
    assert_eq!(vault::forget(&p, "phone", &span).unwrap().files.len(), 1);
    assert!(p.raw().join("lorelog/2026-03-01.ndjson").exists());
    assert!(!p.raw().join("lorelog/2026-03-02.ndjson").exists());
    assert!(vault::forget(&p, "../etc", &span).is_err());
    assert!(vault::forget(&p, "phone", &Span { from: "2026-03-05".into(), to: "2026-03-01".into() }).is_err());
    assert!(vault::forget(&p, "phone", &Span { from: "x".into(), to: "2026-03-01".into() }).is_err());
}
