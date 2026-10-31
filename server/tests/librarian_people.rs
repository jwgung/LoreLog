//! Finding by person (#22): the people in mail headers and event attendees, a name matched to their addresses (never
//! two people merged by a name alone), and what was with them: mail they sent or received and events they attended,
//! newest first, one hit per thread; copied-only mail after, mass mail left out.
use std::fs;
use std::time::Duration;

use life_server::assistant::recall;
use life_server::config::Paths;
use life_server::librarian::Librarian;
use life_server::llm::{Ask, Model};
use serde_json::Value;

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
    fn mail(
        &self,
        name: &str,
        day: &str,
        from: &str,
        to: &str,
        cc: &str,
        subject: &str,
        body: &str,
        refs: &str,
        sent: bool,
    ) {
        let path = self.p.vault.join("Mail").join(&day[..4]).join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = if sent { "account/Sent Messages" } else { "account/INBOX" };
        fs::write(
            path,
            format!(
                "---\ntype: mail\nmail_id: <{name}@x>\nmail_source: {source}\ndate: {day}\ntime: 09:00\nfrom: {from}\nto: [{to}]\ncc: [{cc}]\nsubject: \"{subject}\"\nin_reply_to: \nreferences: {refs}\nattachments: []\n---\n## Message\n{body}\n"
            ),
        )
        .unwrap();
    }

    fn event(&self, name: &str, day: &str, title: &str, attendees: &[&str]) {
        let list: String = attendees.iter().map(|a| format!("  - {a}\n")).collect();
        fs::write(
            self.p.vault.join("Events").join(name),
            format!("---\ntype: event\ndav_uid: {name}\ntitle: {title}\nstart: {day}T10:00:00\nend: {day}T11:00:00\nattendees:\n{list}---\n"),
        )
        .unwrap();
    }

    fn lib(&self) -> Librarian {
        let mut lib = Librarian::open(&self.p, None, Duration::from_secs(5)).unwrap();
        lib.update(false).unwrap();
        lib
    }
}

const ME: &str = "나 <me@x.com>";
const YH: &str = "김영희 수석 <yh@x.com>";

fn many(n: usize) -> String {
    (0..n).map(|i| format!("사람{i} <p{i}@x.com>")).collect::<Vec<_>>().join(", ")
}

fn world() -> T {
    let t = T::new();
    t.mail("a1.md", "2026-03-01", YH, ME, "", "견적 요청", "견적서 부탁드립니다", "[]", false);
    t.mail("a2.md", "2026-03-02", ME, YH, "", "RE: 견적 요청", "보내 드립니다", "[<a1.md@x>]", true);
    t.mail("b1.md", "2026-05-01", ME, YH, "", "회의록", "지난 회의 정리", "[]", true);
    t.mail(
        "c1.md",
        "2026-06-01",
        "총무 <hr@x.com>",
        &many(12),
        YH,
        "전사 공지",
        "김영희 수석 승진을 축하합니다",
        "[]",
        false,
    );
    t.mail("d1.md", "2026-06-02", "총무 <hr@x.com>", ME, YH, "출장 안내", "출장 일정", "[]", false);
    t.mail("e1.md", "2026-06-03", "김영희 <yh2@other.com>", ME, "", "다른 회사 영희", "안녕하세요", "[]", false);
    t.event("ev1.md", "2026-04-01", "주간 점검", &["김영희 수석 <yh@x.com>", ME]);
    t.event("ev0.md", "2026-03-25", "주간 점검", &["김영희 수석 <yh@x.com>", ME]); // the same meeting a week before: one hit
    t
}

fn titles(hits: &[Value]) -> Vec<String> {
    hits.iter().map(|h| h["title"].as_str().unwrap_or("").to_string()).collect()
}

#[test]
fn a_name_finds_each_address_it_was_written_with_and_never_merges_them() {
    let t = world();
    let lib = t.lib();
    let mut found = lib.people("영희").unwrap();
    found.sort_by(|a, b| a.addr.cmp(&b.addr));
    let addrs: Vec<&str> = found.iter().map(|p| p.addr.as_str()).collect();
    assert_eq!(addrs, ["yh2@other.com", "yh@x.com"], "same name, two addresses: two people");
    assert!(found[1].names.iter().any(|n| n == "김영희 수석"));
    assert_eq!(found[1].last, "2026-06-02", "the latest mail or event with them (mass mail not counted)");
    assert!(lib.people("나").unwrap().iter().all(|p| p.addr != "me@x.com"), "the user is not one of the people");
    assert!(lib.people("없는사람").unwrap().is_empty());
}

#[test]
fn what_was_with_a_person_is_their_own_mail_and_events_newest_first_without_mass_mail() {
    let t = world();
    let mut lib = t.lib();
    let hits = lib.with_people(&["yh@x.com".to_string()], "", "2026-12-31", 8).unwrap();
    assert_eq!(
        titles(&hits),
        ["회의록", "주간 점검", "RE: 견적 요청", "출장 안내"],
        "direct first, newest first, copied last"
    );
    assert!(!titles(&hits).contains(&"전사 공지".to_string()), "mass mail is left out");
    assert_eq!(hits[2]["count"], 2, "the thread is one hit");
    let before = lib.with_people(&["yh@x.com".to_string()], "", "2026-04-30", 8).unwrap();
    assert_eq!(titles(&before), ["주간 점검", "RE: 견적 요청"], "nothing after the day asked about");
    let only = lib.with_people(&["yh@x.com".to_string()], "견적", "2026-12-31", 8).unwrap();
    assert_eq!(titles(&only), ["RE: 견적 요청"], "other words narrow it");
}

struct Picks(&'static str);
impl Model for Picks {
    fn complete(&self, _: &[Value], _: Ask) -> anyhow::Result<String> {
        Ok(self.0.into())
    }
}

#[test]
fn the_assistant_asks_by_person_when_the_model_names_one_and_by_words_otherwise() {
    let t = world();
    t.lib(); // built
    let section = recall::find(
        &t.p,
        &Picks(r#"{"search": true, "people": ["김영희"], "words": []}"#),
        "영희 님이랑 뭐 있었지",
        "2026-10-31",
        &[],
    )
    .unwrap();
    assert!(section.contains("회의록") && !section.contains("전사 공지"), "{section}");
    assert!(section.contains("다른 회사 영희"), "the other 김영희 is a candidate too: {section}");
    let words = recall::find(
        &t.p,
        &Picks(r#"{"search": true, "people": ["모르는이"], "words": ["출장"]}"#),
        "출장 건",
        "2026-10-31",
        &[],
    )
    .unwrap();
    assert!(words.contains("출장 안내"), "a name nobody has: the words are searched: {words}");
}
