//! The librarian: an index over every record, searched across all days. It finds and does not judge.
//! Ported from the Python server's test_librarian.py. Every test gets its own vault, data and index.
//!
//! What it must keep: Korean words are found inside longer words and as two letters; only records the diary rules allow
//! are indexed (allowed messengers, confirmed voice); the assistant's own words are not records; a changed or removed
//! file is reflected; records that disagree come back together.
mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::ms;
use life_server::assistant::senders::save_allow;
use life_server::config::Paths;
use life_server::librarian::{Exclude, Librarian, Outdated};
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
        for d in [p.vault.join("Mail"), p.vault.join("Events"), p.raw().join("lorelog")] {
            fs::create_dir_all(d).unwrap();
        }
        let lib = Librarian::open(&p, None, Duration::from_secs(5)).unwrap();
        T { _dir: dir, p, lib }
    }

    fn update(&mut self) -> life_server::librarian::Report {
        self.lib.update(false).unwrap()
    }

    fn find(&mut self, q: &str) -> Vec<Value> {
        self.lib.search(q, 10, None).unwrap()
    }

    fn allow(&self, items: &[&str]) {
        save_allow(&self.p, &items.iter().map(|s| json!(s)).collect::<Vec<_>>()).unwrap();
    }

    fn mail(&self, name: &str, subject: &str, body: &str, day: &str) -> PathBuf {
        self.mail_full(name, subject, body, day, None, "[]", "", "철수 <a@x.com>")
    }

    #[allow(clippy::too_many_arguments)]
    fn mail_full(
        &self,
        name: &str,
        subject: &str,
        body: &str,
        day: &str,
        mail_id: Option<&str>,
        references: &str,
        in_reply_to: &str,
        sender: &str,
    ) -> PathBuf {
        let path = self.p.vault.join("Mail").join(&day[..4]).join(name);
        let id = mail_id.map(str::to_string).unwrap_or_else(|| format!("<{name}@x>"));
        write(
            &path,
            &format!(
                "---\ntype: mail\nmail_id: {id}\nmail_source: account/INBOX\nmail_synced: 2024-01-01\ndate: {day}\ntime: 09:00\nfrom: {sender}\nto: 나 <me@x.com>\nsubject: \"{subject}\"\nin_reply_to: {in_reply_to}\nreferences: {references}\nattachments: []\n---\n## Notes\n\n## Message\n{body}\n"
            ),
        );
        path
    }

    fn lorelog(&self, day: &str, recs: &[Value]) {
        let text: Vec<String> = recs.iter().map(|r| r.to_string()).collect();
        write(&self.p.raw().join("lorelog").join(format!("{day}.ndjson")), &(text.join("\n") + "\n"));
    }

    fn diary(&self, day: &str, recs: &[Value]) {
        let text: Vec<String> = recs.iter().map(|r| r.to_string()).collect();
        write(&self.p.raw().join("diary").join(format!("{day}.ndjson")), &(text.join("\n") + "\n"));
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn days(hits: &[Value]) -> Vec<String> {
    hits.iter().map(|h| h["day"].as_str().unwrap().to_string()).collect()
}

fn titled<'a>(hits: &'a [Value], title: &str) -> Vec<&'a Value> {
    hits.iter().filter(|h| h["title"] == title).collect()
}

fn kakao(title: &str, text: &str, ts: i64) -> Value {
    json!({"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": title, "text": text, "ts": ts})
}

fn conv(id: &str, start: i64, end: Value, who: &str, status: &str) -> Value {
    json!({"type": "conversation", "id": id, "start_ms": start, "end_ms": end, "participants": who, "status": status})
}

#[test]
fn a_mail_body_is_found_by_a_word_inside_a_longer_word_and_by_two_letters() {
    let mut t = T::new();
    let path = t.mail("2024-03-05-0900-meet.md", "미팅 결과", "거래처랑 견적 얘기를 했다", "2024-03-05");
    t.update();
    for q in ["거래처", "견적"] {
        let hits = t.find(q);
        let h = titled(&hits, "미팅 결과");
        assert_eq!(h.len(), 1, "{q}");
        assert_eq!((h[0]["source"].as_str(), h[0]["day"].as_str()), (Some("mail"), Some("2024-03-05")));
        assert!(h[0]["snippet"].as_str().unwrap().contains(q));
        assert_eq!(h[0]["ref"], path.to_string_lossy().as_ref());
    }
}

#[test]
fn the_mail_header_is_searched_but_the_snippet_comes_from_the_body() {
    let mut t = T::new();
    t.mail_full(
        "2024-03-05-1100-hdr.md",
        "헤더제목수달 안내",
        "%% mailstone:begin %%\n본문첫줄입니다 수달 일정 공유",
        "2024-03-05",
        None,
        "[]",
        "",
        "보낸이두더지 <mole@x.com>",
    );
    t.update();
    assert!(!t.find("보낸이두더지").is_empty(), "who sent it is searchable");
    let hits = t.find("헤더제목수달");
    let s = titled(&hits, "헤더제목수달 안내")[0]["snippet"].as_str().unwrap().to_string();
    assert!(s.starts_with("본문첫줄입니다"), "{s}");
    for noise in ["mail_source", "date:", "@x.com", "mailstone"] {
        assert!(!s.contains(noise));
    }
}

#[test]
fn a_single_letter_query_finds_words_that_start_with_it() {
    let mut t = T::new();
    t.mail("2024-03-05-1000-desk.md", "가구", "책상 주문함", "2024-03-05");
    t.update();
    assert!(!titled(&t.find("책"), "가구").is_empty());
}

#[test]
fn events_are_found_without_their_sync_markers() {
    let mut t = T::new();
    write(
        &t.p.vault.join("Events/2024-03-07 장비 점검.md"),
        "---\ntype: event\ndav_uid: u1\ntitle: 장비 점검\nstart: 2024-03-07T14:00:00\nend: 2024-03-07T15:00:00\nall_day: false\nlocation: 2공장\nattendees:\n  - 영희 <b@x.com>\n---\n%% dav:begin %%\n표지옆메모수리부엉이\n%% dav:end %%\n",
    );
    t.update();
    let hits: Vec<Value> = t.find("장비 점검").into_iter().filter(|h| h["source"] == "event").collect();
    assert_eq!(
        hits.iter().map(|h| (h["title"].as_str().unwrap(), h["day"].as_str().unwrap())).collect::<Vec<_>>(),
        [("장비 점검", "2024-03-07")]
    );
    assert!(!t.find("2공장").is_empty(), "where it happens is searchable too");
    let h = t.find("수리부엉이");
    assert!(!h[0]["snippet"].as_str().unwrap().contains("dav:"));
    assert!(t.find("dav").is_empty());
}

#[test]
fn decomposed_hangul_and_fullwidth_letters_are_found_and_shown_as_written() {
    use unicode_normalization::UnicodeNormalization;
    let mut t = T::new();
    let path = t.mail("2024-02-21-0900-nfd.md", "첨부", "본문", "2024-02-21");
    let nfd: String = "분해견적서_트렌드.pdf".nfd().collect();
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("attachments: []", &format!("attachments:\n  - {nfd}"))
        .replace("본문", "ＡＢＣ전각상사 회신");
    fs::write(&path, text).unwrap();
    t.update();
    assert!(!titled(&t.find("분해견적서"), "첨부").is_empty());
    assert!(!titled(&t.find(&"분해견적서".nfd().collect::<String>()), "첨부").is_empty());
    assert!(!titled(&t.find("abc"), "첨부").is_empty());
    assert!(t.find("전각상사")[0]["snippet"].as_str().unwrap().contains("ＡＢＣ전각상사"));
}

#[test]
fn an_index_never_filled_is_not_searched() {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    let mut fresh = Librarian::open(&p, Some(dir.path().join("new/librarian.sqlite")), Duration::from_secs(1)).unwrap();
    assert!(fresh.search("무엇이든", 10, None).unwrap_err().downcast_ref::<Outdated>().is_some());
    assert_eq!(fresh.stats()["outdated"], true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(dir.path().join("new")).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(&fresh.path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    fresh.update(false).unwrap();
    assert!(fresh.search("아무데도없는말코끼리상어", 10, None).unwrap().is_empty());
}

#[test]
fn only_allowed_messengers_and_confirmed_voice_are_indexed() {
    let mut t = T::new();
    t.allow(&["카카오톡: 동료사서"]);
    let day = "2024-03-06";
    t.lorelog(
        day,
        &[
            kakao("동료사서", "도면 수정본 보냈어요", ms(2024, 3, 6, 9, 0)),
            kakao("엄마사서", "고래밥 사와", ms(2024, 3, 6, 10, 0)),
            json!({"type": "voice", "ts": ms(2024, 3, 6, 11, 5), "text": "부품 단가 협의"}),
            json!({"type": "voice", "ts": ms(2024, 3, 6, 15, 5), "text": "옆자리 잡담바나나"}),
            conv("lib-c1", ms(2024, 3, 6, 11, 0), json!(ms(2024, 3, 6, 11, 30)), "동료사서", "tagged"),
            conv("lib-c2", ms(2024, 3, 6, 15, 0), json!(ms(2024, 3, 6, 15, 30)), "?", "ambient"),
        ],
    );
    t.update();
    let kinds = |hits: Vec<Value>| {
        hits.into_iter()
            .filter(|h| h["day"] == day)
            .map(|h| h["kind"].as_str().unwrap().to_string())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(kinds(t.find("도면 수정본")), BTreeSet::from(["메신저".to_string()]));
    assert_eq!(kinds(t.find("부품 단가")), BTreeSet::from(["대화".to_string()]));
    assert!(t.find("고래밥").is_empty(), "a sender the user did not allow is not indexed");
    assert!(t.find("잡담바나나").is_empty(), "voice in a conversation that is not the user's is not indexed");
    t.allow(&["카카오톡: 동료사서", "카카오톡: 엄마사서"]);
    t.update();
    assert!(!t.find("고래밥").is_empty(), "allowing the sender later makes it findable");
    t.allow(&["카카오톡: 동료사서"]);
    t.update();
    assert!(t.find("고래밥").is_empty(), "taking it back hides it again");
}

#[test]
fn pc_windows_are_found() {
    let mut t = T::new();
    write(
        &t.p.raw().join("pc/2024-03-09.ndjson"),
        &format!(
            "{}\n",
            json!({"type": "window", "ts": ms(2024, 3, 9, 9, 0), "end": ms(2024, 3, 9, 9, 40), "dur_s": 2400, "app": "EXCEL.EXE", "title": "견적서_v3.xlsx - Excel"})
        ),
    );
    t.update();
    let hits: Vec<(String, String)> = t
        .find("견적서")
        .into_iter()
        .filter(|h| h["day"] == "2024-03-09")
        .map(|h| (h["source"].as_str().unwrap().into(), h["kind"].as_str().unwrap().into()))
        .collect();
    assert_eq!(hits, [("trace".to_string(), "PC".to_string())]);
}

#[test]
fn the_users_words_are_indexed_but_not_the_assistants() {
    let mut t = T::new();
    let day = "2024-03-08";
    t.diary(
        day,
        &[
            json!({"type": "chat", "ts": ms(2024, 3, 8, 21, 0), "date": day, "role": "user", "text": "김부장님이랑 일정 미뤘어"}),
            json!({"type": "chat", "ts": ms(2024, 3, 8, 21, 1), "date": day, "role": "assistant", "text": "알겠어요 비서답변기린"}),
            json!({"type": "reflection", "ts": ms(2024, 3, 8, 22, 0), "date": day, "reflection": "오늘은 지쳤다호랑이", "corrections": [{"wrong": "회의 세 번", "right": "회의 두 번"}]}),
        ],
    );
    write(
        &t.p.vault.join(format!("Diary/{day}.md")),
        "---\ntype: diary\n---\n<!-- ai:begin -->\n## 오늘의 정리 (ai)\n- 정리문장코끼리\n<!-- ai:end -->\n",
    );
    t.update();
    assert_eq!(
        t.find("김부장").iter().map(|h| h["source"].as_str().unwrap()).collect::<BTreeSet<_>>(),
        BTreeSet::from(["word"])
    );
    assert!(!t.find("지쳤다호랑이").is_empty());
    assert!(!t.find("회의 두 번").is_empty(), "a correction is the user's word");
    assert!(t.find("비서답변기린").is_empty(), "the assistant's reply is not a record");
    assert!(t.find("정리문장코끼리").is_empty(), "the assistant's summary is derived, not a record");
}

#[test]
fn a_changed_file_is_indexed_again_and_a_removed_file_is_forgotten() {
    let mut t = T::new();
    let path = t.mail("2024-03-10-0900-edit.md", "수정", "낡은문구가오리", "2024-03-10");
    t.update();
    assert!(!t.find("가오리").is_empty());
    std::thread::sleep(Duration::from_millis(20));
    t.mail("2024-03-10-0900-edit.md", "수정", "새문구해마", "2024-03-10");
    t.update();
    assert!(t.find("가오리").is_empty());
    assert!(!t.find("해마").is_empty());
    fs::remove_file(&path).unwrap();
    assert_eq!(t.update().removed, 1);
    assert!(t.find("해마").is_empty());
}

#[test]
fn a_second_update_with_nothing_new_touches_nothing() {
    let mut t = T::new();
    t.mail("2024-03-11-0900-same.md", "그대로", "변함없음", "2024-03-11");
    t.update();
    assert_eq!(t.update().indexed, 0);
}

#[test]
fn a_mail_thread_comes_back_as_one_hit_with_its_span() {
    let mut t = T::new();
    let root = "<root-penguin@x>";
    t.mail_full(
        "2024-04-01-0900-t1.md",
        "브로커서버 정보 공유 건",
        "스레드원문펭귄",
        "2024-04-01",
        Some(root),
        "[]",
        "",
        "a <a@x>",
    );
    t.mail_full(
        "2024-04-03-0900-t2.md",
        "RE: 브로커서버 정보 공유 건",
        "답장 스레드원문펭귄",
        "2024-04-03",
        Some("<r1-penguin@x>"),
        &format!("[{root}]"),
        root,
        "a <a@x>",
    );
    t.mail_full(
        "2024-04-09-0900-t3.md",
        "Re: RE: 브로커서버 정보 공유 건",
        "또 답장 스레드원문펭귄",
        "2024-04-09",
        Some("<r2-penguin@x>"),
        &format!("[\"{root}\", \"<r1-penguin@x>\"]"),
        "<r1-penguin@x>",
        "a <a@x>",
    );
    t.mail("2024-04-02-0900-other.md", "다른 건", "스레드원문펭귄 언급", "2024-04-02");
    t.update();
    let hits = t.find("스레드원문펭귄");
    let thread: Vec<&Value> = hits.iter().filter(|h| h["title"].as_str().unwrap().contains("브로커서버")).collect();
    assert_eq!(thread.len(), 1, "replies in one thread do not take every slot");
    assert_eq!(
        (thread[0]["count"].as_i64(), thread[0]["first"].as_str(), thread[0]["last"].as_str()),
        (Some(3), Some("2024-04-01"), Some("2024-04-09"))
    );
    assert!(!titled(&hits, "다른 건").is_empty(), "another mail still has its own place");
}

#[test]
fn mails_that_only_share_a_subject_are_not_one_thread() {
    let mut t = T::new();
    t.mail("2024-05-01-0900-a.md", "견적", "같은제목물개 1월 견적", "2024-05-01");
    t.mail("2024-06-07-0900-b.md", "견적", "같은제목물개 6월 견적", "2024-06-07");
    t.update();
    let hits = t.find("같은제목물개");
    let mut d: Vec<String> = titled(&hits, "견적").iter().map(|h| h["day"].as_str().unwrap().to_string()).collect();
    d.sort();
    assert_eq!(d, ["2024-05-01", "2024-06-07"]);
    assert!(titled(&hits, "견적").iter().all(|h| h["count"] == 1));
}

#[test]
fn a_conversation_confirmed_on_a_later_day_makes_the_earlier_voice_findable_and_taking_it_back_hides_it() {
    let mut t = T::new();
    let (s, e) = (ms(2024, 3, 14, 10, 0), json!(ms(2024, 3, 14, 10, 30)));
    t.lorelog(
        "2024-03-14",
        &[
            json!({"type": "voice", "ts": ms(2024, 3, 14, 10, 5), "text": "나중확인대화하마"}),
            conv("lib-c3", s, e.clone(), "?", "pending"),
        ],
    );
    t.update();
    assert!(t.find("나중확인대화하마").is_empty());
    t.lorelog("2024-03-15", &[conv("lib-c3", s, e.clone(), "팀장", "tagged")]); // the new state arrives in the next day's file
    t.update();
    assert_eq!(days(&t.find("나중확인대화하마")), ["2024-03-14"]);
    t.lorelog("2024-03-15", &[conv("lib-c3", s, e, "?", "ambient")]);
    t.update();
    assert!(t.find("나중확인대화하마").is_empty());
}

#[cfg(unix)]
fn unreadable(path: &Path, yes: bool) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(if yes { 0 } else { 0o600 })).unwrap();
}

#[cfg(unix)]
fn root() -> bool {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

#[cfg(unix)]
#[test]
fn a_later_conversation_state_that_cannot_be_read_again_is_not_forgotten_until_its_file_goes() {
    if root() {
        return; // file modes do not bind root
    }
    let mut t = T::new();
    let (s, e) = (ms(2024, 6, 1, 8, 0), json!(ms(2024, 6, 1, 10, 0)));
    t.lorelog(
        "2024-06-01",
        &[
            json!({"type": "voice", "ts": ms(2024, 6, 1, 9, 0), "text": "비밀회의코끼리"}),
            conv("lib-later", s, e.clone(), "팀", "tagged"),
        ],
    );
    t.lorelog("2024-06-02", &[conv("lib-later", s, e, "팀", "ambient")]);
    t.update();
    assert!(t.find("비밀회의코끼리").is_empty());
    let path = t.p.raw().join("lorelog/2024-06-02.ndjson");
    t.allow(&["카카오톡: 다시읽기용"]); // every phone file is read again
    unreadable(&path, true);
    assert!(t.update().skipped > 0);
    assert!(t.find("비밀회의코끼리").is_empty(), "the last state read still holds: the voice stays out");
    unreadable(&path, false);

    let (s, e) = (ms(2024, 6, 3, 8, 0), json!(ms(2024, 6, 3, 10, 0)));
    t.lorelog(
        "2024-06-03",
        &[
            json!({"type": "voice", "ts": ms(2024, 6, 3, 9, 0), "text": "고아상태코뿔소"}),
            conv("lib-later2", s, e.clone(), "팀", "ambient"),
        ],
    );
    let later = t.p.raw().join("lorelog/2024-06-05.ndjson");
    t.lorelog("2024-06-05", &[conv("lib-later2", s, e, "팀", "tagged")]);
    t.update();
    assert!(!t.find("고아상태코뿔소").is_empty());
    t.allow(&["카카오톡: 다시읽기용2"]);
    unreadable(&later, true);
    t.update(); // 06-05 cannot be read: its tagged state, last read, still holds
    assert!(!t.find("고아상태코뿔소").is_empty());
    unreadable(&later, false);
    fs::remove_file(&later).unwrap(); // then it is removed: the state it held goes with it, as the diary sees it
    t.update();
    assert!(t.find("고아상태코뿔소").is_empty(), "a removed file's states do not linger in the index");
}

#[cfg(unix)]
#[test]
fn a_phone_file_that_cannot_be_read_after_taking_a_sender_back_is_not_left_searchable() {
    if root() {
        return;
    }
    let mut t = T::new();
    t.allow(&["카카오톡: 철회대상"]);
    t.lorelog("2024-03-16", &[kakao("철회대상", "철회문구너구리", ms(2024, 3, 16, 9, 0))]);
    t.update();
    assert!(!t.find("철회문구너구리").is_empty());
    let path = t.p.raw().join("lorelog/2024-03-16.ndjson");
    t.allow(&[]);
    unreadable(&path, true);
    t.update();
    assert!(
        t.find("철회문구너구리").is_empty(),
        "a file that could not be read again is forgotten, not kept as it was"
    );
    unreadable(&path, false);
    t.update();
    assert!(t.find("철회문구너구리").is_empty(), "once readable it is read under the new rules");
}

#[test]
fn a_record_that_cannot_be_understood_skips_that_file_and_indexes_the_rest() {
    let mut t = T::new();
    t.diary(
        "2024-03-17",
        &[json!({"type": "reflection", "date": 20240317, "reflection": "깨진기록바퀴", "corrections": 5})],
    );
    t.mail("2024-03-17-0900-ok.md", "멀쩡함", "멀쩡한문구고슴도치", "2024-03-17");
    assert_eq!(t.update().skipped, 1);
    assert!(!t.find("고슴도치").is_empty(), "the other files are indexed");
    assert_eq!(t.update().skipped, 1, "the skipped file is tried again next time");
}

#[test]
fn a_number_where_a_date_should_be_falls_back_to_the_file_day() {
    let mut t = T::new();
    t.diary("2024-03-19", &[json!({"type": "reflection", "date": 20240319, "reflection": "숫자날짜올빼미"})]);
    t.update();
    assert_eq!(days(&t.find("숫자날짜올빼미")), ["2024-03-19"]);
}

#[test]
fn an_index_of_another_version_is_not_searched_and_is_built_again_in_one_go() {
    let mut t = T::new();
    t.mail("2024-03-18-0900-v.md", "버전", "버전문구다람쥐", "2024-03-18");
    t.update();
    let path = t.lib.path.clone();
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute("update meta set value = 'old' where key = 'schema'", []).unwrap();
    }
    let mut old = Librarian::open(&t.p, Some(path.clone()), Duration::from_secs(5)).unwrap();
    let n: i64 =
        rusqlite::Connection::open(&path).unwrap().query_row("select count(*) from docs", [], |r| r.get(0)).unwrap();
    assert!(n > 0, "opening it does not empty it before it can be filled again");
    assert!(
        old.search("다람쥐", 10, None).unwrap_err().downcast_ref::<Outdated>().is_some(),
        "it may hold what the current rules leave out"
    );
    assert_eq!(old.stats()["outdated"], true);
    old.update(false).unwrap();
    assert_eq!(old.stats()["outdated"], false);
    assert!(!old.search("다람쥐", 10, None).unwrap().is_empty());
}

#[test]
fn the_stats_of_an_index_with_other_tables_say_it_is_outdated() {
    let mut t = T::new();
    t.update();
    {
        let db = rusqlite::Connection::open(&t.lib.path).unwrap();
        db.execute_batch("drop table docs; update meta set value = 'old' where key = 'schema';").unwrap();
    }
    assert_eq!(t.lib.stats()["outdated"], true);
    t.update();
    assert_eq!(t.lib.stats()["outdated"], false);
}

#[test]
fn a_broken_line_in_a_phone_file_does_not_stop_the_index() {
    let mut t = T::new();
    t.allow(&["카카오톡: 깨진줄동료"]);
    let mut bytes = b"\xea\xb0 cut off\n[1]\n".to_vec();
    bytes.extend(format!("{}\n", kakao("깨진줄동료", "깨진줄옆문구수리부엉이", ms(2024, 3, 20, 9, 0))).as_bytes());
    fs::write(t.p.raw().join("lorelog/2024-03-20.ndjson"), bytes).unwrap();
    t.mail("2024-03-20-0900-fine.md", "멀쩡", "옆메일문구너구리개", "2024-03-20");
    assert_eq!(t.update().skipped, 0);
    assert!(!t.find("수리부엉이").is_empty(), "the good lines of the file are indexed");
    assert!(!t.find("너구리개").is_empty(), "and the other files too");
}

#[test]
fn a_new_line_in_one_phone_file_reads_only_that_file() {
    let mut t = T::new();
    t.allow(&["카카오톡: 증분동료"]);
    for d in [21, 22, 23] {
        t.lorelog(&format!("2024-03-{d}"), &[kakao("증분동료", &format!("증분문구{d}"), ms(2024, 3, d, 9, 0))]);
    }
    t.update();
    std::thread::sleep(Duration::from_millis(20));
    let path = t.p.raw().join("lorelog/2024-03-23.ndjson");
    let mut text = fs::read_to_string(&path).unwrap();
    text += &format!("{}\n", kakao("증분동료", "새로온증분고양이", ms(2024, 3, 23, 10, 0)));
    fs::write(&path, text).unwrap();
    assert_eq!(t.update().indexed, 1, "only the phone file that changed is read again");
    assert!(!t.find("새로온증분고양이").is_empty());
}

#[test]
fn the_allowed_senders_used_and_the_ones_remembered_are_the_same() {
    // the allow list changes while an update runs: what was indexed must match what is remembered, so the next update
    // still sees the change
    let mut t = T::new();
    t.lorelog("2024-03-24", &[kakao("경합상대", "경합문구두루미", ms(2024, 3, 24, 9, 0))]);
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = calls.clone();
    t.lib.allowed = Box::new(move |_| {
        let first = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
        if first { BTreeSet::from(["카카오톡: 경합상대".to_string()]) } else { BTreeSet::new() }
    });
    t.update();
    assert!(!t.find("두루미").is_empty(), "indexed under the list read at the start");
    t.update();
    assert!(t.find("두루미").is_empty(), "the sender taken back is dropped at the next update");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2, "read once per update");
}

#[test]
fn an_update_reads_the_allowed_senders_under_the_write_lock() {
    let mut t = T::new();
    let index = t.lib.path.clone();
    let held = std::sync::Arc::new(std::sync::Mutex::new(None));
    let h = held.clone();
    t.lib.allowed = Box::new(move |paths| {
        // another connection cannot take the write lock now
        let other = rusqlite::Connection::open(&index).unwrap();
        other.busy_timeout(Duration::from_millis(0)).unwrap();
        *h.lock().unwrap() = Some(other.execute_batch("begin immediate").is_err());
        life_server::assistant::senders::load_allow(paths)
    });
    t.update();
    assert_eq!(*held.lock().unwrap(), Some(true));
}

#[test]
fn a_query_word_with_a_particle_still_finds_the_word_and_english_finds_longer_forms() {
    let mut t = T::new();
    t.mail("2024-03-25-0900-p.md", "조사", "견적서 보냄 그리고 여수항만 일정", "2024-03-25");
    t.mail("2024-03-26-0900-e.md", "brokerz", "two Brokers and a brokerage", "2024-03-26");
    t.update();
    for q in ["견적서는", "여수항만에서", "견적서를"] {
        assert!(!titled(&t.find(q), "조사").is_empty(), "{q}");
    }
    assert!(!titled(&t.find("broker"), "brokerz").is_empty());
}

#[test]
fn the_users_own_past_questions_do_not_crowd_out_the_records() {
    let mut t = T::new();
    for d in 1..=5 {
        let day = format!("2024-02-0{d}");
        t.diary(&day, &[json!({"type": "chat", "ts": ms(2024, 2, d, 21, 0), "date": day, "role": "user", "text": "펠리컨견적 언제 왔지?"})]);
    }
    t.mail("2024-01-15-0900-pel.md", "펠리컨 건", "펠리컨견적 송부드립니다", "2024-01-15");
    t.update();
    let hits = t.lib.search("펠리컨견적", 3, None).unwrap();
    assert!(hits.iter().any(|h| h["source"] == "mail"), "the record itself is among the top hits");
    let words = hits.iter().filter(|h| h["source"] == "word").count();
    assert!((1..=2).contains(&words), "what the user said is still findable, in two slots at most");
}

#[test]
fn a_reset_empties_the_index_without_removing_the_file_and_a_rebuild_never_shows_it_empty() {
    let mut t = T::new();
    t.mail("2024-03-27-0900-r.md", "리셋", "리셋문구해달", "2024-03-27");
    t.update();
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&fs::metadata(&t.lib.path).unwrap());
    t.lib.reset().unwrap();
    assert!(
        t.lib.search("해달", 10, None).unwrap_err().downcast_ref::<Outdated>().is_some(),
        "emptied, not searched as if nothing were there"
    );
    #[cfg(unix)]
    assert_eq!(std::os::unix::fs::MetadataExt::ino(&fs::metadata(&t.lib.path).unwrap()), inode);
    t.update();
    assert!(!t.find("해달").is_empty());
    let r = t.lib.update(true).unwrap();
    assert!(r.indexed > 0);
    assert!(!t.find("해달").is_empty());
}

#[test]
fn a_phone_line_with_values_of_the_wrong_shape_does_not_stop_the_index() {
    let mut t = T::new();
    t.allow(&["카카오톡: 모양동료"]);
    let text = format!(
        "{}\n{}\n{}\n{}\n",
        json!({"type": "app", "label": ["목록"], "pkg": "y"}),
        r#"{"type": "voice", "text": "무한시각", "ts": 1e400}"#,
        r#"{"type": "conversation", "id": "lib-inf", "start_ms": 1, "end_ms": 1e400, "status": "tagged"}"#,
        kakao("모양동료", "모양틀린줄옆문구가마우지", ms(2024, 3, 28, 9, 0))
    );
    fs::write(t.p.raw().join("lorelog/2024-03-28.ndjson"), text).unwrap();
    t.mail("2024-03-28-0900-shape.md", "모양", "모양옆메일물총새", "2024-03-28");
    t.update();
    assert!(!t.find("가마우지").is_empty(), "the good lines of the day are still indexed");
    assert!(!t.find("물총새").is_empty(), "and the rest of the update went through");
    let (s, e) = (ms(2024, 3, 29, 8, 0), json!(ms(2024, 3, 29, 10, 0)));
    t.lorelog(
        "2024-03-29",
        &[
            json!({"type": "voice", "ts": ms(2024, 3, 29, 9, 0), "text": "다음날음성저어새"}),
            conv("lib-ok", s, e, "팀", "tagged"),
        ],
    );
    t.update();
    assert!(!t.find("저어새").is_empty(), "a broken conversation kept from before does not stop other days");
}

#[test]
fn a_conversation_over_several_days_is_decided_again_on_every_day() {
    let mut t = T::new();
    t.lorelog("2024-04-12", &[json!({"type": "voice", "ts": ms(2024, 4, 12, 9, 0), "text": "사흘째음성황새"})]);
    let (s, e) = (ms(2024, 4, 10, 9, 0), json!(ms(2024, 4, 13, 9, 0)));
    t.lorelog("2024-04-10", &[conv("lib-long", s, e.clone(), "?", "pending")]);
    t.update();
    assert!(t.find("황새").is_empty());
    t.lorelog("2024-04-14", &[conv("lib-long", s, e, "팀", "tagged")]);
    t.update();
    assert_eq!(days(&t.find("황새")), ["2024-04-12"]);
}

#[test]
fn a_conversation_whose_end_is_no_date_is_decided_again_on_every_day() {
    let mut t = T::new();
    t.lorelog("2024-05-03", &[json!({"type": "voice", "ts": ms(2024, 5, 3, 9, 0), "text": "끝없는대화음성두루미"})]);
    t.lorelog("2024-05-01", &[conv("lib-nodate", ms(2024, 5, 1, 9, 0), json!(10_i64.pow(17)), "팀", "tagged")]);
    t.update();
    assert!(!t.find("두루미").is_empty());
    t.lorelog("2024-05-04", &[conv("lib-nodate", ms(2024, 5, 1, 9, 0), json!(10_i64.pow(17)), "팀", "skipped")]);
    t.update();
    assert!(t.find("두루미").is_empty(), "voice the user withdrew does not stay in the index");
}

#[test]
fn records_that_disagree_come_back_together() {
    let mut t = T::new();
    t.allow(&["카카오톡: 거래처담당"]);
    t.mail("2024-03-12-0900-ship.md", "납품", "납품일정 확정입니다", "2024-03-12");
    t.lorelog("2024-03-12", &[kakao("거래처담당", "납품일정 보류해 주세요", ms(2024, 3, 12, 14, 0))]);
    t.update();
    let sources: BTreeSet<String> = t
        .find("납품일정")
        .into_iter()
        .filter(|h| h["day"] == "2024-03-12")
        .map(|h| h["source"].as_str().unwrap().into())
        .collect();
    assert_eq!(
        sources,
        BTreeSet::from(["mail".to_string(), "trace".to_string()]),
        "both records are returned; the librarian does not pick one"
    );
}

#[test]
fn nothing_found_is_an_empty_list() {
    let mut t = T::new();
    t.update();
    assert!(t.find("아무데도없는말").is_empty());
    assert!(t.find("").is_empty());
    assert!(t.find("   ...  ").is_empty());
}

#[test]
fn a_day_or_only_some_words_of_a_day_can_be_left_out() {
    let mut t = T::new();
    t.mail("2024-03-13-0900-skip.md", "제외", "하루빼기문어", "2024-03-13");
    let day = "2024-03-14";
    t.mail("2024-03-14-0900-src.md", "출처별", "출처별제외낙타 메일", day);
    t.diary(day, &[json!({"type": "chat", "ts": ms(2024, 3, 14, 21, 0), "date": day, "role": "user", "text": "출처별제외낙타 말"}), json!({"type": "reflection", "date": day, "reflection": "출처별제외낙타 회고"})]);
    t.update();
    assert!(!t.find("문어").is_empty());
    assert!(t.lib.search("문어", 10, Some(Exclude::Day("2024-03-13"))).unwrap().is_empty());
    let said = vec!["출처별제외낙타 말".to_string()];
    let mut kinds: Vec<String> = t
        .lib
        .search("낙타", 10, Some(Exclude::Words(day, &said)))
        .unwrap()
        .iter()
        .map(|h| h["kind"].as_str().unwrap().into())
        .collect();
    kinds.sort();
    assert_eq!(kinds, ["받은 메일", "회고"]);
}

#[test]
fn a_search_is_not_broken_by_an_update_that_lands_meanwhile() {
    let mut t = T::new();
    let b = t.mail("2024-03-15-1000-b.md", "도중 둘", "검색도중바뀜코알라 둘", "2024-03-15");
    let a = t.mail("2024-03-15-0900-a.md", "도중 하나", "검색도중바뀜코알라 하나", "2024-03-15");
    t.update();
    let (p, index) = (t.p.clone(), t.lib.path.clone());
    t.lib.after_first_hit = Some(Box::new(move || {
        // another process removes both mails while the first hit is being made
        fs::remove_file(&a).unwrap();
        fs::remove_file(&b).unwrap();
        Librarian::open(&p, Some(index.clone()), Duration::from_secs(5)).unwrap().update(false).unwrap();
    }));
    assert_eq!(t.find("코알라").len(), 2, "the search sees the index as it was when it started");
    assert!(t.find("코알라").is_empty(), "and the next one sees the update");
}

// ---------- the text of attachments (#15) ----------

fn attachment(t: &T, day: &str, name: &str, bytes: &[u8]) -> String {
    let (sha, _) = life_server::store::put_blob(&t.p.data, bytes).unwrap();
    let line = json!({"type": "file", "ts": ms(day[..4].parse().unwrap(), day[5..7].parse().unwrap(), day[8..].parse().unwrap(), 10, 0), "sha256": sha, "name": name,
                      "mime": "application/octet-stream", "size": bytes.len(), "mail_id": "<M@x>",
                      "mail_source": "account/INBOX", "direction": "received"});
    let dir = t.p.raw().join("mailfile");
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join(format!("{day}.ndjson"));
    let before = fs::read_to_string(&f).unwrap_or_default();
    fs::write(&f, format!("{before}{line}\n")).unwrap();
    sha
}

#[test]
fn the_words_inside_an_attachment_are_found_and_the_file_is_read_once() {
    let mut t = T::new();
    let sha = attachment(&t, "2026-03-04", "단가표.csv", "품목,단가\n첨부속문구두더지,7700\n".as_bytes());
    attachment(&t, "2026-03-04", "단가표 사본.csv", "품목,단가\n첨부속문구두더지,7700\n".as_bytes()); // the same file
    attachment(&t, "2026-03-04", "현장사진.jpg", b"\xff\xd8\xff\xe0");
    t.update();
    let hits = t.find("첨부속문구두더지");
    assert_eq!(hits.len(), 1, "the same file twice in a day is one: {hits:?}");
    assert_eq!((hits[0]["kind"].as_str(), hits[0]["day"].as_str()), (Some("첨부"), Some("2026-03-04")));
    assert!(hits[0]["snippet"].as_str().unwrap().contains("7700"));
    assert_eq!(titled(&t.find("현장사진"), "현장사진.jpg").len(), 1, "a file not read is found by its name");
    // the text is kept by the file's hash: a rebuild does not need the file again
    fs::remove_file(life_server::store::blob_path(&t.p.data, &sha)).unwrap();
    t.lib.update(true).unwrap();
    assert_eq!(t.find("첨부속문구두더지").len(), 1);
}

#[test]
fn a_file_that_came_with_a_mail_is_found_as_that_mails_thread() {
    let mut t = T::new();
    t.mail_full("q1.md", "견적 요청", "견적 부탁드립니다", "2026-03-04", Some("<M@x>"), "[]", "", "철수 <a@x.com>");
    t.mail_full("q2.md", "RE: 견적 요청", "확인했습니다", "2026-03-05", Some("<r@x>"), "[<M@x>]", "", "나 <me@x.com>");
    attachment(&t, "2026-03-04", "견적서.csv", "품목,단가\n메일첨부문구수달,9900\n".as_bytes());
    t.update();
    let hits = t.find("메일첨부문구수달");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["title"], "견적 요청 (첨부 견적서.csv)");
    assert_eq!(hits[0]["count"], 2, "the thread's mails, the file not counted as one");
    assert!(hits[0]["ref"].as_str().unwrap().ends_with("q1.md"), "where the user finds it: the mail");
    assert!(hits[0]["snippet"].as_str().unwrap().contains("9900"));
    assert_eq!(t.find("견적").len(), 1, "the mail and its file are one hit");
}
