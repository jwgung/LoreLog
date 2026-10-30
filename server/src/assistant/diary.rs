//! The day's diary note (`<vault>/Diary/<day>.md`): the AI part, the user's corrections and reflection, and the raw copy
//! of what the user said (`raw/diary/<day>.ndjson`).
use std::fs;
use std::sync::LazyLock;

use chrono::{DateTime, Local};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::config::Paths;
use crate::fsutil::{append_private, set_mode};

pub const AI_BEGIN: &str = "<!-- ai:begin -->";
pub const AI_END: &str = "<!-- ai:end -->";

const USER_PART: &str = "
## 정정
<!-- 틀린 줄이 있으면 \"틀린 것 → 맞는 것\" 한 줄씩 -->

## 회고
<!-- 오늘 어땠는지 자유롭게 -->
";

static COMMENT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
static AI_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        "(?s){}(.*?){}",
        regex::escape(AI_BEGIN),
        regex::escape(AI_END)
    ))
    .unwrap()
});
static GENERATED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^ai_generated:[ \t]*(.+)$").unwrap());
static GENERATED_LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^ai_generated:.*$").unwrap());
static CORRECTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)^\(정리의 이 문장이 틀렸다고 알려 주셨어요: "(.*?)"\)\n(.*)$"#).unwrap()
});

// ---------- corrections said in chat ----------

/// A correction said in chat is stored as one user line: the summary sentence it corrects, then what the user said.
pub fn wrap_correction(wrong: &str, message: &str) -> String {
    format!("(정리의 이 문장이 틀렸다고 알려 주셨어요: \"{wrong}\")\n{message}")
}

/// (wrong sentence, what the user said) for a stored correction line, else None.
pub fn split_correction(text: &str) -> Option<(String, String)> {
    CORRECTION_RE
        .captures(text)
        .map(|c| (c[1].to_string(), c[2].to_string()))
}

// ---------- sections ----------

/// Where the body of `## <name>` lies: from after its heading line to the next line that starts with `## `, or the end.
fn find_section(text: &str, name: &str) -> Option<(usize, usize, usize)> {
    let head = format!("## {name}");
    let mut at = 0;
    while at <= text.len() {
        let line_end = text[at..].find('\n').map(|i| at + i);
        let line = &text[at..line_end.unwrap_or(text.len())];
        if let (Some(rest), Some(nl)) = (line.strip_prefix(&head), line_end)
            && rest.chars().all(|c| c == ' ' || c == '\t')
        {
            let body_start = nl + 1;
            return Some((at, body_start, next_heading(text, body_start)));
        }
        match line_end {
            Some(nl) => at = nl + 1,
            None => break,
        }
    }
    None
}

/// The start of the first line at or after `from` that begins with `## `, else the end.
fn next_heading(text: &str, from: usize) -> usize {
    let mut at = from;
    loop {
        if text[at..].starts_with("## ") {
            return at;
        }
        match text[at..].find('\n') {
            Some(i) => at += i + 1,
            None => return text.len(),
        }
    }
}

/// Body of `## <name>` up to the next `## ` heading or the end, without placeholder comments.
pub fn section(text: &str, name: &str) -> String {
    match find_section(text, name) {
        Some((_, s, e)) => COMMENT_RE.replace_all(&text[s..e], "").trim().to_string(),
        None => String::new(),
    }
}

/// The text with the body of `## <name>` replaced (the section is added at the end when missing).
pub fn set_section(text: &str, name: &str, body: &str) -> String {
    let new = if body.trim().is_empty() {
        String::new()
    } else {
        format!("{}\n\n", body.trim())
    };
    match find_section(text, name) {
        Some((_, s, e)) => format!("{}{new}{}", &text[..s], &text[e..]),
        None => format!("{}\n\n## {name}\n{new}", text.trim_end_matches('\n')),
    }
}

// ---------- reading and writing the day ----------

fn note_path(paths: &Paths, day: &str) -> std::path::PathBuf {
    paths.vault.join("Diary").join(format!("{day}.md"))
}

fn read_text(path: &std::path::Path) -> String {
    fs::read(path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

/// The day as the app shows it. `stale` when the phone records or the allowed senders changed after the draft.
pub fn read_day(paths: &Paths, day: &str) -> Value {
    let text = read_text(&note_path(paths, day));
    let Some(m) = AI_RE.captures(&text) else {
        return json!({"date": day, "exists": false});
    };
    let ai = m[1].to_string();
    let generated = GENERATED_RE.captures(&text).and_then(|g| {
        DateTime::parse_from_rfc3339(g[1].trim())
            .ok()
            .or_else(|| parse_offsetless(g[1].trim()))
    });
    let stale = match generated {
        None => true,
        Some(g) => {
            let inputs = [
                paths.raw().join("lorelog").join(format!("{day}.ndjson")),
                paths.allow_file.clone(),
            ];
            inputs.iter().any(|p| {
                fs::metadata(p)
                    .and_then(|m| m.modified())
                    .map(|t| DateTime::<Local>::from(t) > g)
                    .unwrap_or(false)
            })
        }
    };
    let summary: Vec<String> = section(&ai, "오늘의 정리 (ai)")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| match l.strip_prefix("- ") {
            Some(rest) => rest.trim().to_string(),
            None => l.trim().to_string(),
        })
        .collect();
    json!({
        "date": day, "exists": true, "stale": stale, "summary": summary,
        "question": section(&ai, "오늘의 질문 (ai)"),
        "corrections": section(&text, "정정"), "reflection": section(&text, "회고"),
    })
}

/// A stamp without an offset is local time.
fn parse_offsetless(s: &str) -> Option<DateTime<chrono::FixedOffset>> {
    let t = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok()?;
    t.and_local_timezone(Local)
        .earliest()
        .map(|t| t.fixed_offset())
}

/// Write the user's corrections and reflection into the day's note and append a raw copy.
/// `{corrections: [{wrong, right}], reflection, append?}`; with `append` the earlier words of the day are kept.
pub fn save_user(paths: &Paths, day: &str, payload: &Value) -> std::io::Result<()> {
    let empty = Map::new();
    let p = payload.as_object().unwrap_or(&empty);
    let path = note_path(paths, day);
    let mut text = read_text(&path);
    if text.is_empty() && !path.exists() {
        text = format!("---\ntype: diary\ndate: {day}\nclass: personal\n---\n# {day}\n");
    }
    let mut lines: Vec<String> = p
        .get("corrections")
        .and_then(Value::as_array)
        .map(|cs| {
            cs.iter()
                .filter_map(|c| {
                    let (w, r) = (c.get("wrong")?.as_str()?, c.get("right")?.as_str()?);
                    (!w.is_empty() && !r.is_empty())
                        .then(|| format!("- {} → {}", w.trim(), r.trim()))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut reflection = p
        .get("reflection")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if p.get("append")
        .map(|v| super::util::truthy(Some(v)))
        .unwrap_or(false)
    {
        let (old_fix, old_ref) = (section(&text, "정정"), section(&text, "회고"));
        if !old_fix.is_empty() {
            lines.insert(0, old_fix);
        }
        reflection = [old_ref, reflection]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    text = set_section(&text, "정정", &lines.join("\n"));
    text = set_section(&text, "회고", &reflection);
    write_note(paths, day, &text)?;
    let raw = paths.raw().join("diary");
    fs::create_dir_all(&raw)?;
    let rec = json!({
        "type": "reflection", "ts": Local::now().timestamp_millis(), "date": day,
        "corrections": p.get("corrections").cloned().unwrap_or_else(|| json!([])),
        "reflection": p.get("reflection").cloned().unwrap_or_else(|| json!("")),
        "_src": "diary", "_class": "personal",
    });
    append_private(&raw.join(format!("{day}.ndjson")), &format!("{rec}\n"))
}

/// Write the day's note (the Diary folder 0700, the note 0600).
pub fn write_note(paths: &Paths, day: &str, text: &str) -> std::io::Result<()> {
    let dir = paths.vault.join("Diary");
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
        set_mode(&dir, 0o700);
    }
    let path = note_path(paths, day);
    fs::write(&path, text)?;
    set_mode(&path, 0o600);
    Ok(())
}

/// The note with the AI part replaced by this draft (a new note when there is none yet).
pub fn render(day: &str, summary: &str, question: &str, existing: &str) -> String {
    let stamp = Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string();
    let lines: Vec<String> = summary
        .trim()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| format!("- {}", l.trim().trim_start_matches(['-', ' ']).trim()))
        .collect();
    let block = format!(
        "{AI_BEGIN}\n## 오늘의 정리 (ai)\n{}\n\n## 오늘의 질문 (ai)\n{}\n{AI_END}",
        lines.join("\n"),
        question.trim()
    );
    if !existing.is_empty() && existing.contains(AI_BEGIN) {
        let text = AI_RE
            .replace_all(existing, regex::NoExpand(&block))
            .into_owned();
        if GENERATED_LINE_RE.is_match(&text) {
            return GENERATED_LINE_RE
                .replacen(&text, 1, regex::NoExpand(&format!("ai_generated: {stamp}")))
                .into_owned();
        }
        return text.replacen(
            "\nclass: personal\n",
            &format!("\nclass: personal\nai_generated: {stamp}\n"),
            1,
        );
    }
    format!(
        "---\ntype: diary\ndate: {day}\nclass: personal\nai_generated: {stamp}\n---\n# {day}\n\n{block}\n{USER_PART}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\ntype: diary\n---\n<!-- ai:begin -->\n## 오늘의 정리 (ai)\n- 하나\n- 둘\n\n## 오늘의 질문 (ai)\n왜요?\n<!-- ai:end -->\n\n## 정정\n<!-- x -->\n\n## 회고\n<!-- y -->\n";

    #[test]
    fn a_section_round_trips_and_placeholders_are_not_content() {
        let text = set_section(NOTE, "회고", "좋은 하루");
        assert_eq!(section(&text, "회고"), "좋은 하루");
        assert_eq!(section(&text, "정정"), "");
        assert_eq!(
            section(&set_section("# x\n", "회고", "새로"), "회고"),
            "새로",
            "a missing section is added"
        );
    }

    #[test]
    fn appending_keeps_earlier_words_and_corrections() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        fs::create_dir_all(paths.vault.join("Diary")).unwrap();
        fs::write(paths.vault.join("Diary/2026-01-01.md"), NOTE).unwrap();
        save_user(&paths, "2026-01-01", &json!({"append": true, "corrections": [{"wrong": "둘", "right": "셋"}], "reflection": "첫째"})).unwrap();
        save_user(&paths, "2026-01-01", &json!({"append": true, "corrections": [{"wrong": "셋", "right": "넷"}], "reflection": "둘째"})).unwrap();
        let day = read_day(&paths, "2026-01-01");
        assert_eq!(day["corrections"], "- 둘 → 셋\n- 셋 → 넷");
        assert_eq!(day["reflection"], "첫째\n\n둘째");
        assert_eq!(
            day["summary"],
            json!(["하나", "둘"]),
            "the AI part is untouched by the user's words"
        );
        let raw = fs::read_to_string(paths.raw().join("diary/2026-01-01.ndjson")).unwrap();
        assert_eq!(raw.lines().count(), 2);
    }

    #[test]
    fn a_new_draft_replaces_only_the_ai_part_and_marks_when() {
        let note = render("2026-01-02", "- 하나\n둘\n\n", " 왜요? ", "");
        assert!(note.contains("ai_generated: "));
        assert!(note.contains(
            "## 오늘의 정리 (ai)\n- 하나\n- 둘\n\n## 오늘의 질문 (ai)\n왜요?\n<!-- ai:end -->"
        ));
        let with_words = set_section(&note, "회고", "내 말");
        let again = render("2026-01-02", "셋", "또?", &with_words);
        assert_eq!(section(&again, "회고"), "내 말");
        assert!(again.contains("- 셋"));
        assert_eq!(again.matches("ai_generated:").count(), 1);
    }

    #[test]
    fn a_correction_line_unwraps() {
        let line = wrap_correction("10초 통화", "20분 했어");
        assert_eq!(
            split_correction(&line),
            Some(("10초 통화".into(), "20분 했어".into()))
        );
        assert_eq!(split_correction("그냥 말"), None);
    }
}
