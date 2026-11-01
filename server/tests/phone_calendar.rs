//! The phone's calendar (#77): every event the phone's calendars hold (a year back and a year ahead) comes as
//! `calendar_event` records in the phone's raw files, again when it changes and once more, `deleted`, when it goes. The
//! index keeps the latest state of each (a later file, then a later line, wins) and indexes the events still there
//! as events, with their attendees as people. An event the vault also has from CalDAV counts once: by its UID when
//! both have one, else by its start to the minute and its title's letters.
mod common;

use std::fs;
use std::path::Path;
use std::time::Duration;

use common::ms;
use life_server::config::Paths;
use life_server::librarian::Librarian;
use serde_json::{Value, json};

struct T {
    _dir: tempfile::TempDir,
    p: Paths,
    lib: Librarian,
}

impl T {
    fn new() -> T {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::under(dir.path());
        for d in [p.vault.join("Events"), p.raw().join("lorelog")] {
            fs::create_dir_all(d).unwrap();
        }
        let lib = Librarian::open(&p, None, Duration::from_secs(5)).unwrap();
        T { _dir: dir, p, lib }
    }

    fn update(&mut self) {
        self.lib.update(false).unwrap();
    }

    fn events(&mut self, q: &str) -> Vec<(String, String)> {
        self.lib
            .search(q, 20, None)
            .unwrap()
            .iter()
            .filter(|h| h["source"] == "event")
            .map(|h| (h["title"].as_str().unwrap().to_string(), h["day"].as_str().unwrap().to_string()))
            .collect()
    }

    fn lorelog(&self, day: &str, recs: &[Value]) {
        let text: Vec<String> = recs.iter().map(|r| r.to_string()).collect();
        write(&self.p.raw().join("lorelog").join(format!("{day}.ndjson")), &(text.join("\n") + "\n"));
    }

    fn caldav(&self, uid: &str, start: &str, title: &str) {
        write(
            &self.p.vault.join("Events").join(format!("{} {title}.md", &start[..10])),
            &format!(
                "---\ntype: event\ndav_uid: {uid}\ndav_source: cal\ndav_state: live\ntitle: {title}\nstart: {start}\nend: {start}\nall_day: false\nlocation: \"\"\n---\n%% dav:begin %%\n"
            ),
        );
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A phone event as the phone sends it.
fn ev(key: &str, uid: &str, title: &str, start: i64, status: &str) -> Value {
    json!({"type": "calendar_event", "ts": ms(2026, 11, 1, 8, 0), "key": key, "uid": uid, "calendar": "업무",
           "title": title, "start": start, "end": start + 3_600_000, "all_day": false, "location": "본사 3층",
           "description": "", "attendees": ["이담당 <Lee@Client.co.kr>", "park@client.co.kr"],
           "status": status})
}

#[test]
fn the_phones_events_are_indexed_as_events_with_their_time_place_and_people() {
    let mut t = T::new();
    let mut e = ev("7@1", "", "분기 회의", ms(2026, 12, 3, 14, 0), "upsert");
    e["description"] = json!("분기 실적 검토");
    t.lorelog("2026-11-01", &[e]);
    t.update();
    assert_eq!(t.events("분기 회의"), [("분기 회의".to_string(), "2026-12-03".to_string())]);
    assert_eq!(t.events("실적"), [("분기 회의".to_string(), "2026-12-03".to_string())], "the description is searched");
    let hits = t.lib.search("분기 회의", 5, None).unwrap();
    assert!(hits[0]["snippet"].as_str().unwrap().starts_with("2026-12-03 14:00~15:00"), "{}", hits[0]["snippet"]);
    assert!(hits[0]["snippet"].as_str().unwrap().contains("장소 본사 3층"));
    let with = t.lib.with_people(&["lee@client.co.kr".into()], "", "2026-12-31", 10).unwrap();
    assert_eq!(with.len(), 1, "the attendees are who the event is with");
    assert_eq!(with[0]["title"], "분기 회의");
}

#[test]
fn a_later_state_of_an_event_wins_and_a_deleted_one_goes() {
    let mut t = T::new();
    t.lorelog(
        "2026-11-01",
        &[
            ev("7@1", "", "분기 회의", ms(2026, 12, 3, 14, 0), "upsert"),
            ev("8@1", "", "점심 약속", ms(2026, 12, 4, 12, 0), "upsert"),
        ],
    );
    t.update();
    assert_eq!(t.events("점심 약속").len(), 1);
    // the next day: one renamed, the other deleted on the phone
    t.lorelog(
        "2026-11-02",
        &[
            ev("7@1", "", "분기 결산 회의", ms(2026, 12, 3, 14, 0), "upsert"),
            ev("8@1", "", "점심 약속", ms(2026, 12, 4, 12, 0), "deleted"),
        ],
    );
    t.update();
    assert_eq!(t.events("결산"), [("분기 결산 회의".to_string(), "2026-12-03".to_string())]);
    assert!(t.events("점심 약속").is_empty(), "deleted on the phone");
    assert_eq!(t.events("분기 회의").len(), 1, "one event, in its latest state");
    // the later file goes: the state before it is what is known again
    fs::remove_file(t.p.raw().join("lorelog").join("2026-11-02.ndjson")).unwrap();
    t.update();
    assert_eq!(t.events("점심 약속").len(), 1);
}

#[test]
fn an_event_the_vault_also_has_from_caldav_counts_once() {
    let mut t = T::new();
    // the same UID; the phone's title written a little differently
    t.caldav("abc@google.com", "2026-12-03T14:00:00", "분기 회의");
    // no UID on the phone: the same start minute and the same letters of the title
    t.caldav("u-2", "2026-12-05T09:30:00", "Weekly Sync");
    t.lorelog(
        "2026-11-01",
        &[
            ev("7@1", "abc@google.com", "분기회의 (본사)", ms(2026, 12, 3, 14, 0), "upsert"),
            ev("9@1", "", "weekly  sync!", ms(2026, 12, 5, 9, 30), "upsert"),
            ev("10@1", "", "Weekly Sync", ms(2026, 12, 12, 9, 30), "upsert"),
        ],
    );
    t.update();
    assert_eq!(t.events("분기"), [("분기 회의".to_string(), "2026-12-03".to_string())]);
    let weekly = t.events("weekly sync");
    assert_eq!(weekly.len(), 2, "{weekly:?}");
    assert!(weekly.contains(&("Weekly Sync".to_string(), "2026-12-05".to_string())));
    assert!(weekly.contains(&("Weekly Sync".to_string(), "2026-12-12".to_string())), "another week is another event");
    // the CalDAV note goes: the phone's event is counted then
    fs::remove_file(t.p.vault.join("Events").join("2026-12-03 분기 회의.md")).unwrap();
    t.update();
    assert_eq!(t.events("분기"), [("분기회의 (본사)".to_string(), "2026-12-03".to_string())]);
}

#[test]
fn an_all_day_event_is_on_its_day_without_a_time() {
    let mut t = T::new();
    // the phone keeps an all-day event at midnight UTC
    let utc = chrono::NaiveDate::from_ymd_opt(2026, 12, 25)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    let mut e = ev("11@1", "", "성탄절", utc, "upsert");
    e["all_day"] = json!(true);
    e["end"] = json!(utc + 86_400_000);
    t.lorelog("2026-11-01", &[e]);
    t.update();
    assert_eq!(t.events("성탄절"), [("성탄절".to_string(), "2026-12-25".to_string())]);
    let hits = t.lib.search("성탄절", 5, None).unwrap();
    assert!(hits[0]["snippet"].as_str().unwrap().starts_with("2026-12-25 장소"), "{}", hits[0]["snippet"]);
}

#[test]
fn a_record_without_a_key_or_a_start_is_left_out_alone() {
    let mut t = T::new();
    let mut no_key = ev("x", "", "키 없음", ms(2026, 12, 3, 14, 0), "upsert");
    no_key.as_object_mut().unwrap().remove("key");
    let mut no_start = ev("y", "", "시작 없음", 0, "upsert");
    no_start["start"] = json!("soon");
    t.lorelog("2026-11-01", &[no_key, no_start, ev("z", "", "멀쩡한 일정", ms(2026, 12, 3, 15, 0), "upsert")]);
    t.update();
    assert!(t.events("키 없음").is_empty());
    assert!(t.events("시작 없음").is_empty());
    assert_eq!(t.events("멀쩡한").len(), 1);
}
