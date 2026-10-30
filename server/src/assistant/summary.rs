//! The day summary: the prompt, filler filtering, time ordering, and writing the draft into the day's note.
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use super::context::build_context;
use super::diary::{render, write_note};
use crate::config::Paths;
use crate::llm::{Ask, Model, parse_json};

pub const SYSTEM: &str = r#"너는 사용자의 개인 비서다. 오늘 수집된 기록을 보고 하루를 정리해서 사용자에게 말하듯 전한다.
규칙:
- 기록에 없는 것은 쓰지 않는다. 추측은 "~로 보여요", "~한 것 같아요"로 표시하고 단정하지 않는다.
- 직함, 감정, 관계는 기록에 적힌 그대로만 쓴다. 기록에 없는 직함을 붙이지 않는다.
- 다른 사람이 보낸 메시지나 한 말은 사용자가 한 말로 쓰지 않는다. "~님이 ~라고 하셨어요"처럼 주체를 구분한다.
- 대화 전사는 사용자가 본인 대화로 확인한 것이지만 오류가 있다. 상대의 사적인 내용(가족, 건강 등)은 자세히 옮기지 않는다.
- 하루의 흐름이 보이게 쓴다. 오전, 오후, 저녁 순서로 이어지는 5~7개의 짧은 문장(각각 한 줄)이다. 같은 종류의 일(메일 여러 통 등)은 한 문장으로 묶고, 가장 의미 있는 일(통화, 일정, 결정이 필요한 요청)을 먼저 구체적으로 쓴다.
- "[내가 보냄]"이 붙은 메일은 사용자가 보낸 것이다("~에게 메일을 보내셨어요"). 받은 메일과 구분한다. "[대량 발송]"이 붙은 메일, 광고, 시스템 알림은 쓰지 않는다. 앱 사용 횟수나 장소 이동 횟수는 쓰지 않는다.
- 쓰지 않는 빈말: "다양한 앱을 활용하셨어요", "여러 곳을 이동하셨어요", "업무 관련 대화를 나누셨어요"처럼 무엇인지 알 수 없는 문장. 구체적인 주제를 모르면 그 문장을 빼거나 "동료분들과 연락이 오갔어요"처럼 사실만 쓴다.
- 메신저 알림은 제목보다 내용을 근거로 한다. 점심·잡담은 업무로 쓰지 않고 한 줄 이내로 쓰거나 뺀다.
- 시각은 기록에 적힌 시각을 그대로 옮긴다. 24시간제를 오전/오후로 바꿀 때 틀리지 않게 한다(19:15 = 저녁 7시 15분, 08:34 = 오전 8시 34분).
- 정리는 존댓말로 비서가 말하듯 쓰고 "~했어요", "~셨어요", "~보여요"로 끝낸다. 글머리 기호를 붙이지 않는다.
- 질문은 하나만. 오늘 가장 의미 있는 일(통화, 일정 변경, 결정이 필요한 요청) 하나를 골라 "그때 어떠셨어요?"를 묻는 식으로, 사용자의 마음이나 선택 이유를 부드러운 존댓말 한 문장으로 묻는다. 다른 사람의 말을 사용자의 말로 단정하는 질문은 하지 않는다.
JSON만 출력: {"summary": "문장1\n문장2\n...", "question": "..."}"#;

static FILLER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(다양한 앱|앱과 메신저를 활용|곳의 장소|장소를 이동|여러 곳을 이동|업무 관련 대화를 나누|메시지를 주고받으셨어요$)").unwrap()
});
static CLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(오전|오후|저녁|밤|아침|점심|정오|새벽)?\s*(\d{1,2})\s*시\s*(?:(\d{1,2})\s*분)?").unwrap()
});
static NOON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"점심|정오").unwrap());

/// Minutes since midnight of the first clock time in the sentence; 점심/정오 without a number is 12:00; else `fallback`.
fn minutes(line: &str, fallback: u32) -> u32 {
    let Some(m) = CLOCK.captures(line) else {
        return if NOON.is_match(line) { 12 * 60 } else { fallback };
    };
    let mut h: u32 = m[2].parse().unwrap_or(0);
    let mi: u32 = m.get(3).and_then(|x| x.as_str().parse().ok()).unwrap_or(0);
    if matches!(m.get(1).map(|x| x.as_str()), Some("오후" | "저녁" | "밤")) && h < 12 {
        h += 12;
    }
    h * 60 + mi
}

/// Remove filler sentences and put the rest in time order (the model often mixes the order). A sentence without a
/// time stays after the one before it.
pub fn drop_filler(summary: &str) -> String {
    let mut last = 0;
    let mut keyed: Vec<(u32, usize, &str)> = Vec::new();
    for (i, l) in summary.lines().filter(|l| !l.trim().is_empty() && !FILLER.is_match(l)).enumerate() {
        last = minutes(l, last);
        keyed.push((last, i, l));
    }
    keyed.sort();
    keyed.iter().map(|(_, _, l)| *l).collect::<Vec<_>>().join("\n")
}

/// Ask the model for the day's summary and question. -> {summary, question}
pub fn ask(model: &dyn Model, context: &str, day: &str) -> anyhow::Result<Value> {
    let messages = [
        json!({"role": "system", "content": SYSTEM}),
        json!({"role": "user", "content": format!("날짜: {day}\n\n{context}")}),
    ];
    let text = model.complete(&messages, Ask::new(0.3, 1500, true, 180))?;
    let mut data = parse_json(&text)?;
    let raw = data.get("summary").and_then(Value::as_str).unwrap_or("").to_string();
    let kept = drop_filler(&raw);
    data["summary"] = Value::String(if kept.is_empty() { raw } else { kept });
    Ok(data)
}

/// Build the day's draft with the model and write it into the day's note (unless `dry_run`). -> the note text
pub fn generate(paths: &Paths, model: &dyn Model, day: &str, dry_run: bool) -> anyhow::Result<String> {
    let ai = ask(model, &build_context(paths, day, true), day)?;
    let path = paths.vault.join("Diary").join(format!("{day}.md"));
    let existing = std::fs::read(&path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let s = |k: &str| ai.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let text = render(day, &s("summary"), &s("question"), &existing);
    if !dry_run {
        write_note(paths, day, &text)?;
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filler_sentences_are_removed() {
        let out = drop_filler("오전 9시 회의가 있었어요.\n오늘은 다양한 앱을 활용하셨어요.\n오후 3시 통화를 하셨어요.");
        assert!(!out.contains("다양한 앱"));
        assert_eq!(out.lines().count(), 2);
    }

    #[test]
    fn sentences_are_sorted_by_clock_time_and_afternoon_is_converted() {
        let out = drop_filler(
            "오후 7시 15분 메일이 왔어요.\n오전 8시 34분 회신을 보냈어요.\n점심시간 잡담을 나누셨어요.\n오후 1시 45분 대화가 있었어요.",
        );
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("오전 8시"));
        assert!(lines[1].starts_with("점심시간"));
        assert!(lines[3].starts_with("오후 7시"));
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn a_sentence_without_a_time_stays_after_its_neighbour() {
        let out = drop_filler("오후 2시 회의가 있었어요.\n결론이 났어요.\n오전 9시 출근했어요.");
        let heads: Vec<String> = out.lines().map(|l| l.chars().take(5).collect()).collect();
        assert_eq!(heads, ["오전 9시", "오후 2시", "결론이 났"]);
    }
}
