//! What the assistant asked the librarian is kept, so real questions can join the evaluation (#7): the question, the
//! words searched and where the hits are, never what they say. Off with LIBRARIAN_ASK_LOG=off; older than 90 days
//! goes away.
use std::fs;

use life_server::assistant::recall;
use life_server::config::Paths;
use life_server::librarian::asks;
use life_server::llm::{Ask, Model};
use serde_json::{Value, json};

struct Picker(&'static str);
impl Model for Picker {
    fn complete(&self, _: &[Value], _: Ask) -> anyhow::Result<String> {
        Ok(self.0.into())
    }
}

fn setup() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    let m = p.vault.join("Mail/2026/a.md");
    fs::create_dir_all(m.parent().unwrap()).unwrap();
    fs::create_dir_all(p.vault.join("Events")).unwrap();
    fs::write(&m, "---\ntype: mail\nmail_id: <a@x>\ndate: 2026-04-01\ntime: 09:00\nfrom: 보안팀 <sec@x.com>\nsubject: \"포트 포워딩 요청\"\nreferences: []\n---\n## Message\n방화벽 비밀문구사과\n").unwrap();
    (dir, p)
}

fn lines(p: &Paths) -> Vec<Value> {
    fs::read_to_string(asks::file(p)).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn a_question_is_kept_with_its_words_and_where_the_hits_are_but_not_what_they_say() {
    let (_d, p) = setup();
    let section =
        recall::find(&p, &Picker(r#"{"search": true, "words": ["포트"]}"#), "포트 열어 준 거 됐나", "2026-10-30", &[]);
    assert!(section.unwrap().contains("포트 포워딩 요청"));
    let got = lines(&p);
    assert_eq!(got.len(), 1);
    let l = &got[0];
    assert_eq!(
        (l["q"].as_str(), l["day"].as_str(), l["found"].as_str()),
        (Some("포트 열어 준 거 됐나"), Some("2026-10-30"), Some("found"))
    );
    assert_eq!(l["words"], json!(["포트"]));
    let mail = p.vault.join("Mail/2026/a.md");
    assert_eq!(l["refs"], json!([asks::ref_id(&mail.to_string_lossy())]), "an id, not the file name");
    assert!(!fs::read_to_string(asks::file(&p)).unwrap().contains("비밀문구사과"), "no text of the records");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(asks::file(&p)).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn a_question_the_model_does_not_look_up_and_one_that_finds_nothing_are_kept_as_such() {
    let (_d, p) = setup();
    assert!(recall::find(&p, &Picker(r#"{"search": false}"#), "좋은 아침", "2026-10-30", &[]).is_none());
    recall::find(&p, &Picker(r#"{"search": true, "words": ["없는말무지개"]}"#), "무지개 건", "2026-10-30", &[]);
    let got = lines(&p);
    assert_eq!(got[0]["found"], "not searched");
    assert_eq!((got[1]["found"].as_str(), got[1]["refs"].as_array().map(Vec::len)), (Some("nothing"), Some(0)));
}

#[test]
fn keeping_questions_can_be_turned_off() {
    let (_d, p) = setup();
    fs::create_dir_all(&p.conf).unwrap();
    fs::write(p.conf.join("ingest.env"), "INGEST_PORT=9000\n").unwrap();
    asks::set_enabled(&p, false).unwrap();
    assert!(!asks::enabled(&p));
    recall::find(&p, &Picker(r#"{"search": true, "words": ["포트"]}"#), "포트", "2026-10-30", &[]);
    assert!(!asks::file(&p).exists());
    asks::set_enabled(&p, true).unwrap();
    assert!(asks::enabled(&p));
    assert!(fs::read_to_string(p.conf.join("ingest.env")).unwrap().contains("INGEST_PORT=9000"), "other settings kept");
}

#[test]
fn questions_older_than_the_window_are_dropped() {
    let (_d, p) = setup();
    fs::create_dir_all(&p.data).unwrap();
    let old = json!({"ts": "2026-01-01T09:00:00+09:00", "day": "2026-01-01", "q": "옛 질문"});
    let new = json!({"ts": "2026-10-29T09:00:00+09:00", "day": "2026-10-29", "q": "새 질문"});
    fs::write(asks::file(&p), format!("{old}\n{new}\nnot json\n")).unwrap();
    assert_eq!(asks::prune(&p, "2026-10-30", 90).unwrap(), 2);
    let got = lines(&p);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["q"], "새 질문");
}
