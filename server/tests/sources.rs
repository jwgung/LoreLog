//! Setting up what to collect without a terminal (the desktop app's settings screen and `life add` share it): what is
//! configured, saving a mail account and its folders, saving calendars, and the model addresses. Nothing here reaches
//! a mail or calendar server.
use std::fs;

use life_server::collect::sources::{self, MailSetup};
use life_server::config::{Paths, read_env_file};
use life_server::host::{self, Models};

fn paths() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    (dir, p)
}

fn mail(password: Option<&str>) -> MailSetup {
    MailSetup {
        host: "imap.example".into(),
        port: 993,
        user: "me@example".into(),
        password: password.map(str::to_string),
        folders: vec!["INBOX".into(), "&vPSwuA-".into()],
    }
}

#[test]
fn nothing_is_configured_at_first() {
    let (_d, p) = paths();
    let v = sources::view(&p);
    assert!(v.mail.is_none() && v.calendars.is_empty());
}

#[test]
fn a_saved_mail_account_is_shown_without_its_password() {
    let (_d, p) = paths();
    sources::save_mail(&p, &mail(Some("pw-1"))).unwrap();
    let v = sources::view(&p);
    let m = v.mail.unwrap();
    assert_eq!((m.host.as_str(), m.port, m.user.as_str(), m.has_password), ("imap.example", 993, "me@example", true));
    assert_eq!(m.folders, ["INBOX", "&vPSwuA-"]);
    assert_eq!(m.folder_labels, ["INBOX", "보낸"]);
    let shown = serde_json::to_string(&sources::view(&p)).unwrap();
    assert!(!shown.contains("pw-1"), "{shown}");
    let env = read_env_file(&p.conf.join("env"));
    assert_eq!(env.get("IMAP_PASSWORD").map(String::as_str), Some("pw-1"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(p.conf.join("env")).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn no_password_keeps_the_stored_one_and_none_at_all_is_refused() {
    let (_d, p) = paths();
    assert!(sources::save_mail(&p, &mail(None)).unwrap_err().contains("비밀번호"));
    sources::save_mail(&p, &mail(Some("pw-1"))).unwrap();
    let mut again = mail(None);
    again.folders = vec!["INBOX".into()];
    sources::save_mail(&p, &again).unwrap();
    let env = read_env_file(&p.conf.join("env"));
    assert_eq!(env.get("IMAP_PASSWORD").map(String::as_str), Some("pw-1"));
    assert_eq!(sources::view(&p).mail.unwrap().folders, ["INBOX"]);
}

#[test]
fn a_mail_setup_needs_a_server_an_account_and_a_folder() {
    let (_d, p) = paths();
    let mut m = mail(Some("pw"));
    m.folders.clear();
    assert!(sources::save_mail(&p, &m).is_err());
    let mut m = mail(Some("pw"));
    m.host = " ".into();
    assert!(sources::save_mail(&p, &m).is_err());
    assert!(sources::view(&p).mail.is_none(), "nothing saved");
}

#[test]
fn calendars_are_saved_with_the_window_and_other_settings_stay() {
    let (_d, p) = paths();
    sources::save_mail(&p, &mail(Some("pw"))).unwrap();
    sources::save_calendars(&p, &["https://cal.example/dav/a/".to_string()]).unwrap();
    let v = sources::view(&p);
    assert_eq!(v.calendars, ["https://cal.example/dav/a/"]);
    assert!(v.mail.is_some(), "the mail account is kept");
    let text = fs::read_to_string(p.conf.join("sources.toml")).unwrap();
    assert!(text.contains("past_days = 400") && text.contains("future_days = 400"), "{text}");
    assert!(!text.contains("pw"), "no secret in sources.toml");
    assert!(sources::save_calendars(&p, &["http://cal.example/".to_string()]).is_err(), "https only");
    sources::save_calendars(&p, &[]).unwrap();
    assert!(sources::view(&p).calendars.is_empty(), "an empty list stops collecting calendars");
}

#[test]
fn model_addresses_are_saved_next_to_the_token() {
    let (_d, p) = paths();
    let token = host::ensure_token(&p).unwrap();
    let m = Models {
        chat_url: "http://gpu.tail1.ts.net:8000/v1".into(),
        chat_model: "qwen".into(),
        asr_url: String::new(),
        asr_model: String::new(),
    };
    host::save_models(&p, &m).unwrap();
    let env = read_env_file(&host::ingest_env(&p));
    assert_eq!(env.get("INGEST_TOKEN"), Some(&token), "the token stays");
    assert_eq!(env.get("LIFE_LLM_MODEL").map(String::as_str), Some("qwen"));
    assert!(!env.contains_key("LIFE_ASR_URL"), "an empty address is no setting");
    assert_eq!(host::saved_models(&p), m);
    let bad = Models { chat_url: "gpu:8000".into(), ..m };
    assert!(host::save_models(&p, &bad).is_err(), "an address is http(s)");
}

#[test]
fn the_models_can_be_changed_while_the_server_runs() {
    let live = host::LiveModels::new(Models::default());
    let err = life_server::llm::Model::complete(live.as_ref(), &[], life_server::llm::Ask::new(0.0, 1, false, 1))
        .unwrap_err();
    assert!(err.to_string().contains("모델 주소가 설정되지"), "{err}");
    live.set(Models { chat_url: "http://127.0.0.1:9/v1".into(), ..Models::default() });
    let err = life_server::llm::Model::complete(live.as_ref(), &[], life_server::llm::Ask::new(0.0, 1, false, 1))
        .unwrap_err();
    assert!(!err.to_string().contains("모델 주소가 설정되지"), "now it tries the address: {err}");
}
