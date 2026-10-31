//! Copies to go back to (#16): the export (#11), sealed with the user's password, in a folder they chose, once a day
//! with the hourly sync; each one opened again and checked against its hashes; 7 days and 4 weeks kept.
use std::fs;
use std::path::Path;

use chrono::{NaiveDate, TimeZone};
use life_server::backup;
use life_server::config::Paths;

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn vault() -> (tempfile::TempDir, Paths, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    put(&p.raw().join("lorelog/2026-03-01.ndjson"), "{\"type\":\"voice\",\"text\":\"백업문구고래\"}\n");
    put(&p.vault.join("Diary/2026-03-01.md"), "# 하루");
    let to = dir.path().join("external");
    fs::create_dir_all(&to).unwrap();
    (dir, p, to)
}

fn at(day: &str, h: u32) -> chrono::DateTime<chrono::Local> {
    let d = NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap();
    chrono::Local.from_local_datetime(&d.and_hms_opt(h, 0, 0).unwrap()).unwrap()
}

#[test]
fn a_backup_is_sealed_checked_and_opens_only_with_the_password() {
    let (dir, p, to) = vault();
    assert!(!backup::due(&p, at("2026-03-02", 3)), "not set up: never due");
    backup::set(&p, &to, "correct horse battery").unwrap();
    assert!(backup::due(&p, at("2026-03-02", 3)));
    let done = backup::run_at(&p, at("2026-03-02", 3), backup::TEST_WORK).unwrap();
    assert_eq!(done.files, 2);
    let name = done.file.file_name().unwrap().to_string_lossy().to_string();
    assert_eq!(name, "lorelog-backup-2026-03-02-0300.zip.age");
    let sealed = fs::read(&done.file).unwrap();
    assert!(sealed.starts_with(b"age-encryption.org/v1"), "an age file");
    assert!(!String::from_utf8_lossy(&sealed).contains("Diary"), "names are sealed too, not only contents");
    let out = dir.path().join("restored.zip");
    assert_eq!(backup::restore(&done.file, "correct horse battery", &out).unwrap(), 2);
    assert!(backup::restore(&done.file, "wrong", &dir.path().join("x.zip")).is_err());
    assert!(!dir.path().join("x.zip").exists(), "nothing is left of a failed restore");
    let st = life_server::collect::setup::load_status(&life_server::collect::sync::status_file(&p));
    assert_eq!(st["backup"]["ok"], true, "life status shows a copy made by hand too: {st:?}");
    assert!(!backup::due(&p, at("2026-03-02", 20)), "once a day");
    assert!(backup::due(&p, at("2026-03-03", 4)));
    let left: Vec<_> = fs::read_dir(&to).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(left.len(), 1, "no plain copy and no part left where it goes: {left:?}");
    assert!(fs::read_dir(p.data.join("backup-tmp")).map(|d| d.count()).unwrap_or(0) == 0, "nothing left here either");
}

#[test]
fn a_folder_inside_the_vault_or_one_that_is_not_there_is_refused() {
    let (dir, p, _) = vault();
    assert!(backup::set(&p, &p.data.join("b"), "pw-long-enough").is_err());
    assert!(backup::set(&p, &p.vault, "pw-long-enough").is_err());
    assert!(backup::set(&p, &dir.path().join("nowhere"), "pw-long-enough").is_err());
    assert!(backup::set(&p, dir.path(), "short").is_err(), "a password of at least 8 letters");
}

#[test]
fn seven_days_and_four_weeks_are_kept() {
    let names: Vec<String> = (0..60)
        .map(|i| {
            let d = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap() + chrono::Days::new(i);
            format!("lorelog-backup-{}-0300.zip.age", d.format("%Y-%m-%d"))
        })
        .chain(["lorelog-backup-2026-04-29-1500.zip.age".to_string(), "unrelated.txt".to_string()])
        .collect();
    let mut kept = backup::kept(&names);
    kept.sort();
    // the newest of each of the last 7 days, and the newest of each of the 4 weeks before
    assert_eq!(kept.len(), 11, "{kept:?}");
    assert!(kept.contains(&"lorelog-backup-2026-04-29-1500.zip.age".to_string()));
    assert!(!kept.contains(&"lorelog-backup-2026-04-29-0300.zip.age".to_string()), "one a day");
    assert!(!kept.iter().any(|k| k.starts_with("lorelog-backup-2026-03-0")), "too old: {kept:?}");
    assert!(!kept.contains(&"unrelated.txt".to_string()));
}
