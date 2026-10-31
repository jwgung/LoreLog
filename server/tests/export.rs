//! The vault taken out whole (#11): one zip of what the user's records are, in open formats, with a list of hashes
//! to check it by; what can be made again (the index, caches) and secrets stay behind.
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use life_server::config::Paths;
use life_server::export;

fn put(path: &Path, text: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn vault() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    put(&p.raw().join("lorelog/2026-03-01.ndjson"), b"{\"type\":\"voice\",\"text\":\"hi\"}\n");
    put(&p.raw().join("files/ab/ab12"), b"%PDF-1.4 quote");
    put(&p.raw().join("mailfile/2026-03-01.ndjson"), b"{\"type\":\"file\",\"sha256\":\"ab12\",\"name\":\"q.pdf\"}\n");
    put(&p.todo_file(), b"{\"items\":[]}");
    put(&p.data.join("librarian-asks.ndjson"), b"{\"q\":\"x\"}\n");
    put(&p.vault.join("Mail/2026/a.md"), "---\ntype: mail\n---\n견적".as_bytes());
    put(&p.vault.join("Events/e.md"), b"---\ntype: event\n---\n");
    put(&p.vault.join("Diary/2026-03-01.md"), "# 하루".as_bytes());
    put(&p.vault.join("Private/mine.md"), b"not ours to take"); // the user's own notes elsewhere in the vault
    put(&p.allow_file, b"{\"allow\":[]}");
    // what stays behind
    put(&p.data.join("librarian.sqlite"), b"index");
    put(&p.data.join("cache/attach-text-1/ab/ab12.pdf"), b"text");
    put(&p.data.join("status.json"), b"{}");
    put(&p.conf.join("env"), b"MAIL_PASSWORD=secret");
    put(&p.conf.join("ingest.env"), b"INGEST_TOKEN=secret");
    (dir, p)
}

fn names(zip: &Path) -> Vec<String> {
    let mut z = zip::ZipArchive::new(fs::File::open(zip).unwrap()).unwrap();
    let mut out: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().unwrap().to_string()).collect();
    out.sort();
    out
}

#[test]
fn the_records_notes_files_and_settings_go_out_and_the_index_caches_and_secrets_stay() {
    let (dir, p) = vault();
    let out = dir.path().join("out/lorelog.zip");
    let report = export::export(&p, &out).unwrap();
    assert_eq!(
        names(&out),
        [
            "MANIFEST.sha256",
            "README.md",
            "data/librarian-asks.ndjson",
            "data/raw/files/ab/ab12",
            "data/raw/lorelog/2026-03-01.ndjson",
            "data/raw/mailfile/2026-03-01.ndjson",
            "data/todos.json",
            "notes/Diary/2026-03-01.md",
            "notes/Events/e.md",
            "notes/Mail/2026/a.md",
            "settings/diary.json",
        ]
    );
    assert_eq!(report.files, 9);
    let mut z = zip::ZipArchive::new(fs::File::open(&out).unwrap()).unwrap();
    let mut manifest = String::new();
    z.by_name("MANIFEST.sha256").unwrap().read_to_string(&mut manifest).unwrap();
    let pdf = life_server::store::sha256_hex(b"%PDF-1.4 quote");
    assert!(
        manifest.lines().any(|l| l == format!("{pdf}  data/raw/files/ab/ab12")),
        "sha256sum -c reads it: {manifest}"
    );
    let mut text = String::new();
    z.by_name("notes/Mail/2026/a.md").unwrap().read_to_string(&mut text).unwrap();
    assert_eq!(text, "---\ntype: mail\n---\n견적");
    assert!(!dir.path().join("out/lorelog.zip.part").exists());
    assert_eq!(export::verify(&out).unwrap(), 9);
}

#[test]
fn a_changed_or_missing_file_fails_the_check() {
    let (dir, p) = vault();
    let out = dir.path().join("lorelog.zip");
    export::export(&p, &out).unwrap();
    // the same zip with one file's bytes changed and one left out
    let mut src = zip::ZipArchive::new(fs::File::open(&out).unwrap()).unwrap();
    let bad = dir.path().join("bad.zip");
    let mut w = zip::ZipWriter::new(fs::File::create(&bad).unwrap());
    for i in 0..src.len() {
        let mut f = src.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        if name == "notes/Events/e.md" {
            continue;
        }
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).unwrap();
        if name == "data/todos.json" {
            bytes = b"{\"items\":[1]}".to_vec();
        }
        w.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(&bytes).unwrap();
    }
    w.finish().unwrap();
    let err = export::verify(&bad).unwrap_err().to_string();
    assert!(err.contains("data/todos.json") || err.contains("notes/Events/e.md"), "{err}");
}

#[test]
fn an_empty_vault_still_gives_a_readable_export() {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    let out = dir.path().join("e.zip");
    assert_eq!(export::export(&p, &out).unwrap().files, 0);
    assert_eq!(names(&out), ["MANIFEST.sha256", "README.md"]);
    assert_eq!(export::verify(&out).unwrap(), 0);
}
