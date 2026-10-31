//! Address books over CardDAV (#22): vCards read into contacts, kept as whole lists in the raw store only when they
//! changed, and each source (the phone, each address book) keeping its own newest list. No CardDAV server is reached.
use std::fs;

use life_server::collect::carddav;
use life_server::collect::sources;
use life_server::config::Paths;
use life_server::librarian::records::newest_contacts;

const CARDS: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:김영희\r\nN:김;영희;;;\r\nORG:한빛상사;영업팀\r\nEMAIL;TYPE=WORK:YH@hanbit.example\r\nEMAIL:yh.home@x.example\r\nTEL;TYPE=CELL:+82 10-1234-5678\r\nNOTE:생일 선물 사 주기\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:4.0\r\nN:Lee;Andy;;;\r\nEMAIL:andy@istec.exa\r\n mple\r\nEND:VCARD\r\nBEGIN:VCARD\r\nFN:번호도 메일도 없는 사람\r\nEND:VCARD\r\n";

#[test]
fn vcards_become_contacts_with_names_mail_and_phones_only() {
    let got = carddav::parse_vcards(CARDS);
    assert_eq!(got.len(), 2, "a card with nothing to reach them by is left out");
    assert_eq!(got[0].name, "김영희");
    assert_eq!(got[0].org, "한빛상사");
    assert_eq!(got[0].emails, ["yh@hanbit.example", "yh.home@x.example"]);
    assert_eq!(got[0].phones, ["+82 10-1234-5678"]);
    assert_eq!(got[1].name, "Andy Lee", "no FN: the name from N");
    assert_eq!(got[1].emails, ["andy@istec.example"], "folded lines are joined");
    let line = carddav::lines(&got, "carddav:abc", 7).join("\n");
    assert!(!line.contains("생일"), "notes are not kept: {line}");
}

#[test]
fn a_book_is_written_only_when_it_changed() {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    let cards = carddav::parse_vcards(CARDS);
    assert!(carddav::keep(&p, "https://dav.example/books/me/", &cards).unwrap(), "first time: written");
    assert!(!carddav::keep(&p, "https://dav.example/books/me/", &cards).unwrap(), "the same list: nothing written");
    assert!(carddav::keep(&p, "https://dav.example/books/me/", &cards[..1]).unwrap(), "one deleted: written");
    let files: Vec<_> = fs::read_dir(p.raw().join("contacts")).unwrap().map(|e| e.unwrap().path()).collect();
    let newest = newest_contacts(&files);
    assert_eq!(newest.len(), 1, "the newest list of that book");
    assert_eq!(newest[0].name, "김영희");
}

#[test]
fn each_source_keeps_its_own_newest_list() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("2026-07-01.ndjson");
    let line = |origin: Option<&str>, recv: &str, snap: i64, name: &str| {
        let mut v = serde_json::json!({"type": "contact", "snap": snap, "_recv": recv, "name": name, "emails": [format!("{name}@x.com")]});
        if let Some(o) = origin {
            v["origin"] = serde_json::json!(o);
        }
        v.to_string()
    };
    fs::write(
        &f,
        [
            line(None, "2026-07-01T09:00:00+09:00", 1, "phone-old"),
            line(Some("carddav:a"), "2026-07-01T10:00:00+09:00", 2, "book"),
            line(None, "2026-07-01T11:00:00+09:00", 3, "phone-new"),
        ]
        .join("\n"),
    )
    .unwrap();
    let mut names: Vec<String> = newest_contacts(&[f]).into_iter().map(|c| c.name).collect();
    names.sort();
    assert_eq!(names, ["book", "phone-new"], "a newer phone list does not replace the address book's");
}

#[test]
fn address_books_are_saved_like_calendars() {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    sources::save_books(&p, &["https://dav.example/books/me/".to_string()]).unwrap();
    assert_eq!(sources::view(&p).books, ["https://dav.example/books/me/"]);
    assert!(sources::save_books(&p, &["http://dav.example/".to_string()]).is_err(), "https only");
    sources::save_books(&p, &[]).unwrap();
    assert!(sources::view(&p).books.is_empty());
}
