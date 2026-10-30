//! To-dos the assistant proposes from the records; the user finishes or dismisses them. Kept in `<data>/todos.json`
//! (`{items: [...], last_extract}`); unknown fields of an item are kept as they are.
use std::fs;
use std::sync::{LazyLock, Mutex};

use chrono::{Duration, Local, NaiveDate};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha1::{Digest, Sha1};

use super::context::{build_context, sent_index, subj_key};
use super::util::pystr;
use crate::config::Paths;
use crate::fsutil::write_private_atomic;
use crate::llm::{Ask, Model, parse_json};

pub const TODO_SYSTEM: &str = r#"너는 사용자의 개인 비서다. 기록에서 **사용자가 해야 할 일**을 뽑는다.
규칙:
- 기록에 분명히 드러난 것만 뽑는다: 사용자에게 온 요청·문의·마감·약속. 짐작해서 만들지 않는다.
- "[이미 회신함]", "[내가 보냄]", "[대량 발송]"이 붙은 메일은 뽑지 않는다. 광고, 뉴스레터, 단순 알림, 이미 끝난 일도 뽑지 않는다.
- title은 20자 안팎의 짧은 한 줄로, "무엇을 하기"로 끝낸다. 사람은 "OO 님"으로 쓰고 직함·회사명은 뺀다.
  좋은 예: "한도윤 님 도입 문의에 답장하기", "컨설팅 추진일정 확인하고 회신하기", "워크숍 발표자료 보내기", "견적 검토 의견 전달하기".
  나쁜 예(중복): "OO 문의 회신에 답하기", "요청에 응하기", "관련 메일 확인하기".
- source는 메일, 통화, 메신저, 일정, 음성 중 하나.
- source가 메일이면 mail_subject에 기록의 메일 제목을 RE:/FW: 없이 그대로 적는다. 아니면 빈 문자열.
- due는 기록에 기한이 적혀 있을 때만 그 표현을 쓰고(예: "금요일까지"), 없으면 빈 문자열.
- 많아야 5개. 없으면 빈 목록.
JSON만 출력: {"todos": [{"title": "...", "source": "메일", "mail_subject": "", "due": ""}]}"#;

/// Requests run on threads, and extraction merges after a long model call: load-modify-save must not interleave.
static LOCK: Mutex<()> = Mutex::new(());
static NORM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^0-9a-z가-힣]").unwrap());

fn norm(t: &str) -> String {
    NORM_RE.replace_all(&t.to_lowercase(), "").into_owned()
}

pub fn load(paths: &Paths) -> Value {
    fs::read_to_string(paths.todo_file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(|v| v.get("items").map(Value::is_array).unwrap_or(false))
        .unwrap_or_else(|| json!({"items": [], "last_extract": 0}))
}

pub fn save(paths: &Paths, d: &Value) -> std::io::Result<()> {
    write_private_atomic(&paths.todo_file(), serde_json::to_string_pretty(d).expect("JSON serializes").as_bytes())
}

fn items(d: &mut Value) -> &mut Vec<Value> {
    if !d.get("items").map(Value::is_array).unwrap_or(false) {
        d["items"] = json!([]);
    }
    d["items"].as_array_mut().expect("items is a list")
}

fn status(i: &Value) -> &str {
    i.get("status").and_then(Value::as_str).unwrap_or("")
}

/// Add a to-do unless one with the same title (letters and digits only) exists, also a dismissed one: a dismissed
/// to-do never comes back.
pub fn add(d: &mut Value, title: &str, source: &str, due: &str, day: &str, mail_subject: &str) -> bool {
    let n = norm(title);
    if n.is_empty() || items(d).iter().any(|i| norm(&pystr(i.get("title"))) == n) {
        return false;
    }
    let id = &hex::encode(Sha1::digest(n.as_bytes()))[..10];
    let thread = if source == "메일" { subj_key(mail_subject) } else { String::new() };
    items(d).push(json!({"id": id, "title": title.trim(), "source": source, "due": due, "status": "open", "day": day,
                         "thread": thread, "created": Local::now().timestamp_millis()}));
    true
}

/// Mark open mail to-dos done when the user has since replied in the same thread (recorded as auto-closed).
pub fn autoclose(paths: &Paths, d: &mut Value) -> usize {
    autoclose_with(d, &sent_index(paths, 21))
}

pub fn autoclose_with(d: &mut Value, sent: &[super::context::Sent]) -> usize {
    let mut closed = 0;
    for i in items(d).iter_mut() {
        let thread = i.get("thread").and_then(Value::as_str).unwrap_or("").to_string();
        let created = i.get("created").and_then(Value::as_f64).unwrap_or(0.0);
        if status(i) == "open"
            && !thread.is_empty()
            && sent.iter().any(|(k, _, w)| *k == thread && w * 1000.0 >= created)
        {
            i["status"] = json!("done");
            i["auto"] = json!(true);
            closed += 1;
        }
    }
    closed
}

/// Open to-dos and the last ten done ones (newest first).
pub fn view(paths: &Paths) -> Value {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = load(paths);
    if autoclose(paths, &mut d) > 0 {
        let _ = save(paths, &d);
    }
    let all = items(&mut d).clone();
    let open: Vec<&Value> = all.iter().filter(|i| status(i) == "open").collect();
    let done: Vec<&Value> = all.iter().filter(|i| status(i) == "done").collect();
    let recent: Vec<&Value> = done[done.len().saturating_sub(10)..].iter().rev().copied().collect();
    json!({"open": open, "done": recent})
}

/// Ask the model for to-dos in the day's records and yesterday's. At most once per 30 minutes unless `force`.
/// -> how many were added
pub fn extract(paths: &Paths, model: &dyn Model, day: &str, force: bool) -> anyhow::Result<usize> {
    let now = Local::now().timestamp() as f64;
    let last = load(paths).get("last_extract").and_then(Value::as_f64).unwrap_or(0.0);
    if !force && now - last < 1800.0 {
        return Ok(0);
    }
    let today = NaiveDate::parse_from_str(day, "%Y-%m-%d")?;
    let prev = (today - Duration::days(1)).format("%Y-%m-%d").to_string();
    let ctx = format!(
        "[어제 {prev}]\n{}\n\n[오늘 {day}]\n{}",
        build_context(paths, &prev, false),
        build_context(paths, day, false)
    );
    let messages = [json!({"role": "system", "content": TODO_SYSTEM}), json!({"role": "user", "content": ctx})];
    let found = parse_json(&model.complete(&messages, Ask::new(0.2, 700, true, 180))?)?;
    let found: Vec<Value> = found.get("todos").and_then(Value::as_array).cloned().unwrap_or_default();
    // the model call took minutes: merge into the file as it is now, not as it was then
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = load(paths);
    let s = |t: &Map<String, Value>, k: &str| t.get(k).map(|v| pystr(Some(v))).unwrap_or_default();
    let added = found
        .iter()
        .take(5)
        .filter_map(Value::as_object)
        .filter(|t| add(&mut d, &s(t, "title"), &s(t, "source"), &s(t, "due"), day, &s(t, "mail_subject")))
        .count();
    autoclose(paths, &mut d);
    d["last_extract"] = json!(now);
    save(paths, &d)?;
    Ok(added)
}

/// done | dismiss | reopen. -> false when there is no such to-do
pub fn act(paths: &Paths, id: &str, action: &str) -> std::io::Result<bool> {
    let to = match action {
        "done" => "done",
        "dismiss" => "dismissed",
        "reopen" => "open",
        _ => return Ok(false),
    };
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = load(paths);
    let Some(i) = items(&mut d).iter_mut().find(|i| i.get("id").and_then(Value::as_str) == Some(id)) else {
        return Ok(false);
    };
    i["status"] = json!(to);
    save(paths, &d)?;
    Ok(true)
}

/// The user's own to-do.
pub fn add_user(paths: &Paths, text: &str, day: &str) -> std::io::Result<bool> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = load(paths);
    let ok = add(&mut d, text, "내가 말한 것", "", day, "");
    save(paths, &d)?;
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::under(dir.path());
        (dir, p)
    }

    #[test]
    fn a_dismissed_todo_never_returns() {
        let (_d, p) = paths();
        let mut d = load(&p);
        add(&mut d, "OO 님 문의에 답장하기", "메일", "", "2026-10-28", "");
        save(&p, &d).unwrap();
        let id = load(&p)["items"][0]["id"].as_str().unwrap().to_string();
        assert!(act(&p, &id, "dismiss").unwrap());
        let mut d = load(&p);
        assert!(
            !add(&mut d, "OO 님 문의에 답장하기!", "메일", "", "2026-10-29", ""),
            "same title modulo punctuation is a duplicate"
        );
        assert_eq!(view(&p)["open"], json!([]));
    }

    #[test]
    fn a_users_own_todo_is_listed() {
        let (_d, p) = paths();
        add_user(&p, "세금 서류 챙기기", "2026-10-28").unwrap();
        assert_eq!(view(&p)["open"][0]["title"], "세금 서류 챙기기");
    }

    #[test]
    fn a_reply_closes_a_mail_todo() {
        let mut d = json!({"items": [], "last_extract": 0});
        add(&mut d, "견적 의견 전달하기", "메일", "", "2026-10-28", "RE: 견적 검토");
        let created = d["items"][0]["created"].as_f64().unwrap();
        let sent = vec![(subj_key("견적 검토"), String::new(), created / 1000.0 + 60.0)];
        assert_eq!(autoclose_with(&mut d, &sent), 1);
        assert_eq!((d["items"][0]["status"].as_str(), d["items"][0]["auto"].as_bool()), (Some("done"), Some(true)));
    }
}
