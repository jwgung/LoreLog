//! What the assistant reads about a day. Ported from the Python server's test_context.py.
mod common;

use std::fs;

use common::ms;
use life_server::assistant::context::{
    self, build_context, gather_lorelog, gather_mail, gather_pc, replied, subj_key,
};
use life_server::assistant::senders::save_allow;
use life_server::config::Paths;
use serde_json::json;

fn paths() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    fs::create_dir_all(p.vault.join("Mail/2026")).unwrap();
    fs::create_dir_all(p.vault.join("Events")).unwrap();
    fs::create_dir_all(p.raw().join("lorelog")).unwrap();
    (dir, p)
}

/// A mail note: [source folder, subject, sender, day, time, mail id].
fn write_mail(p: &Paths, name: &str, [source, subject, sender, day, time, mail_id]: [&str; 6]) {
    let note = format!(
        "---\ntype: mail\nmail_id: {mail_id}\nmail_source: account/{source}\ndate: {day}\ntime: {time}\nfrom: {sender}\nsubject: \"{subject}\"\nin_reply_to: \n---\nbody\n"
    );
    fs::write(p.vault.join("Mail/2026").join(name), note).unwrap();
}

#[test]
fn the_subject_key_ignores_reply_prefixes() {
    assert_eq!(
        subj_key("Fwd: RE: 견적 검토 요청의 건"),
        subj_key("견적 검토 요청의 건")
    );
}

#[test]
fn a_reply_counts_only_when_sent_after_the_mail_in_the_same_thread() {
    let sent = vec![
        (subj_key("RE: 견적 건"), String::new(), 1100.0),
        (String::new(), "abc@h".to_string(), 1500.0),
    ];
    assert!(replied(&sent, "견적 건", "x", 1000.0));
    assert!(
        !replied(&sent, "견적 건", "x", 1200.0),
        "a reply sent before the mail arrived is not an answer"
    );
    assert!(!replied(&sent, "다른 건", "x", 1000.0));
    assert!(
        replied(&sent, "무관", "<abc@h>", 1000.0),
        "in-reply-to pointing at the mail id counts"
    );
}

#[test]
fn mail_is_marked_bulk_sent_or_replied() {
    let (_d, p) = paths();
    let day = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string(); // replies are looked for in the last 21 days
    let name = |hm: &str, s: &str| format!("{day}-{hm}-{s}.md");
    write_mail(
        &p,
        &name("0900", "a"),
        ["INBOX", "견적 건", "철수 <a@x.com>", &day, "09:00", "m1"],
    );
    write_mail(
        &p,
        &name("1000", "b"),
        [
            "INBOX",
            "광고입니다",
            "홍보 <news@shop.com>",
            &day,
            "10:00",
            "",
        ],
    );
    write_mail(
        &p,
        &name("1100", "c"),
        [
            "Sent Messages",
            "RE: 견적 건",
            "나 <me@x.com>",
            &day,
            "11:00",
            "",
        ],
    );
    write_mail(
        &p,
        &name("1200", "d"),
        ["INBOX", "회의 자료", "영희 <b@x.com>", &day, "12:00", "m4"],
    );
    let rows = gather_mail(&p, &day);
    let joined = rows.join("\n");
    assert!(joined.contains("견적 건 [이미 회신함]"), "{joined}");
    assert!(joined.contains("[대량 발송]"));
    assert!(joined.contains("[내가 보냄]"));
    assert!(
        rows.iter()
            .find(|r| r.contains("회의 자료"))
            .unwrap()
            .ends_with("회의 자료"),
        "unanswered mail carries no mark"
    );
}

#[test]
fn a_broken_line_in_a_phone_file_does_not_hide_the_conversations() {
    let (_d, p) = paths();
    let good = json!({"type": "conversation", "id": "ctx-ok", "start_ms": "1", "end_ms": "2", "status": "tagged"});
    let mut bytes = b"\xea\xb0 broken\n[1]\n".to_vec();
    bytes.extend(
        format!(
            "{}\n{good}\n",
            json!({"type": "conversation", "status": "x"})
        )
        .as_bytes(),
    );
    fs::write(p.raw().join("lorelog/2026-09-01.ndjson"), bytes).unwrap();
    let convs = context::load_conversations(&p);
    assert!(convs.contains_key("\"ctx-ok\""));
    let bad = serde_json::from_value(json!({"status": "tagged"})).unwrap();
    let all: Vec<&serde_json::Map<String, serde_json::Value>> =
        std::iter::once(&bad).chain(convs.values()).collect();
    assert_eq!(context::confirmed_voice_label(all, 1).as_deref(), Some("?"));
}

#[test]
fn only_allowed_notifications_and_confirmed_voice_reach_the_prompt() {
    let (_d, p) = paths();
    let day = "2026-10-21";
    let recs = [
        json!({"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": "동료", "text": "회의 내일", "ts": "1792000000000"}),
        json!({"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": "엄마", "text": "밥 먹었니", "ts": "1792000100000"}),
        json!({"type": "voice", "ts": "1792000200000", "text": "확인된 대화"}),
        json!({"type": "voice", "ts": "1792000900000", "text": "주변 소리"}),
        json!({"type": "conversation", "id": "1", "start_ms": "1791999999000", "end_ms": "1792000300000", "participants": "동료", "status": "tagged"}),
        json!({"type": "conversation", "id": "2", "start_ms": "1792000800000", "end_ms": "1792001000000", "participants": "", "status": "ambient"}),
    ];
    let text: Vec<String> = recs.iter().map(|r| r.to_string()).collect();
    fs::write(
        p.raw().join("lorelog").join(format!("{day}.ndjson")),
        text.join("\n"),
    )
    .unwrap();
    save_allow(&p, &[json!("카카오톡: 동료")]).unwrap();
    let out = gather_lorelog(&p, day);
    let keys: Vec<String> = out
        .notif
        .most_common(10)
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(
        keys,
        ["카카오톡: 동료"],
        "family notification must not reach the model"
    );
    assert_eq!(out.voice.len(), 1);
    assert!(out.voice[0].contains("확인된 대화") && out.voice[0].contains("with 동료"));
    let ctx = build_context(&p, day, false);
    assert!(ctx.contains(
        "## 메신저 알림(사용자가 허용한 상대만, 건수: 최근 내용)\n- 카카오톡: 동료 x1: "
    ));
    assert!(!ctx.contains("밥 먹었니"));
}

#[test]
fn short_pc_visits_drop_out_and_breaks_under_five_minutes_merge() {
    let (_d, p) = paths();
    let base = ms(2026, 2, 3, 9, 0);
    let m = 60_000;
    let rows = [
        json!({"type": "window", "ts": base, "end": base + 4 * m, "dur_s": 240, "app": "excel.exe", "title": "견적서"}),
        json!({"type": "window", "ts": base + 6 * m, "end": base + 10 * m, "dur_s": 240, "app": "excel.exe", "title": "견적서"}),
        json!({"type": "window", "ts": base + 11 * m, "end": base + 13 * m, "dur_s": 120, "app": "chrome.exe", "title": "검색"}),
        json!({"type": "idle", "ts": base + 13 * m, "end": base + 40 * m, "dur_s": 1620, "reason": "idle"}),
    ];
    fs::create_dir_all(p.raw().join("pc")).unwrap();
    let text: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
    fs::write(p.raw().join("pc/2026-02-03.ndjson"), text.join("\n") + "\n").unwrap();
    assert_eq!(
        gather_pc(&p, "2026-02-03", 12),
        ["09:00~09:10 excel.exe \"견적서\" (8분)"]
    );
    assert!(gather_pc(&p, "2026-02-04", 12).is_empty());
}

#[test]
fn an_empty_day_says_none_for_every_part() {
    let (_d, p) = paths();
    let ctx = build_context(&p, "2026-05-05", true);
    assert!(ctx.starts_with("## 일정\n- (없음)\n## 받거나 보낸 메일(제목만)\n- (없음)"));
    assert!(
        !ctx.contains("## 위치"),
        "the summary leaves out places and app counts"
    );
}
