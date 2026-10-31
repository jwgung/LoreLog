//! "틀렸어" (#10): whatever the user corrects (a sentence of the day's summary, an answer of the assistant, a to-do),
//! it arrives as one pair, `{wrong, right, ref, ts}`, and is kept as it came in `raw/corrections/<day>.ndjson`
//! (the day it was said). Then it is applied where it refers to, and what changed is said back in one line:
//!   - `summary:<day>`: the day's note keeps it under 정정, and the summary is made again with it;
//!   - `chat:<day>`: the assistant answers the rest of that day with it;
//!   - `todo:<id>`: the to-do is named as the user said.
//!
//! The same pairs are later what the persona and the assistant learn from: nothing is summarised away here.
use std::sync::LazyLock;

use chrono::Local;
use regex::Regex;
use serde_json::{Value, json};

use crate::config::Paths;
use crate::fsutil::{append_private, ensure_private_dir};

/// the longest wrong or right kept
pub const MAX_TEXT: usize = 2000;
/// what is quoted of each side in the line said back
const SAID: usize = 40;

static REF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(summary|chat):(\d{4}-\d{2}-\d{2})$|^todo:([0-9a-f]{10})$").unwrap());

/// What a correction is about.
#[derive(Clone, Debug, PartialEq)]
pub enum Ref {
    Summary(String),
    Chat(String),
    Todo(String),
}

impl Ref {
    pub fn parse(s: &str) -> Option<Ref> {
        let c = REF_RE.captures(s.trim())?;
        Some(match (c.get(1).map(|m| m.as_str()), c.get(2), c.get(3)) {
            (Some("summary"), Some(d), _) => Ref::Summary(d.as_str().into()),
            (Some("chat"), Some(d), _) => Ref::Chat(d.as_str().into()),
            (_, _, Some(id)) => Ref::Todo(id.as_str().into()),
            _ => return None,
        })
    }

    pub fn as_str(&self) -> String {
        match self {
            Ref::Summary(d) => format!("summary:{d}"),
            Ref::Chat(d) => format!("chat:{d}"),
            Ref::Todo(id) => format!("todo:{id}"),
        }
    }
}

/// One correction.
#[derive(Clone, Debug, PartialEq)]
pub struct Correction {
    pub wrong: String,
    pub right: String,
    pub refers: Ref,
}

impl Correction {
    /// A correction of these words, or None when a side is empty or too long.
    pub fn new(wrong: &str, right: &str, refers: Ref) -> Option<Correction> {
        let (w, r) = (wrong.trim(), right.trim());
        let fits = |s: &str| !s.is_empty() && s.chars().count() <= MAX_TEXT;
        (fits(w) && fits(r)).then(|| Correction { wrong: w.into(), right: r.into(), refers })
    }

    /// What changed, in one line.
    pub fn changed(&self) -> String {
        let cut = |s: &str| {
            let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
            if one.chars().count() > SAID { format!("{}…", one.chars().take(SAID).collect::<String>()) } else { one }
        };
        let (w, r) = (cut(&self.wrong), cut(&self.right));
        match self.refers {
            Ref::Summary(_) => format!("정리를 고쳤어요: {w} → {r} (다시 정리할 때 반영해요)"),
            Ref::Chat(_) => format!("답을 고쳤어요: {w} → {r} (오늘 대화에 반영해요)"),
            Ref::Todo(_) => format!("할 일을 바꿨어요: {w} → {r}"),
        }
    }
}

/// Keep a correction as it came.
pub fn keep(paths: &Paths, c: &Correction) -> std::io::Result<()> {
    let dir = paths.raw().join("corrections");
    ensure_private_dir(&dir)?;
    let now = Local::now();
    let rec = json!({"type": "correction", "ts": now.timestamp_millis(), "wrong": c.wrong, "right": c.right,
                     "ref": c.refers.as_str(), "_src": "corrections", "_class": "personal"});
    append_private(&dir.join(format!("{}.ndjson", now.format("%Y-%m-%d"))), &format!("{rec}\n"))
}

/// Every correction kept that refers to one of these, oldest first.
pub fn about(paths: &Paths, refs: &[Ref]) -> Vec<Correction> {
    let wanted: Vec<String> = refs.iter().map(Ref::as_str).collect();
    let mut files = crate::assistant::util::files_like(&paths.raw().join("corrections"), "", ".ndjson");
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        for v in text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
            let r = v["ref"].as_str().unwrap_or("");
            if v["type"] == "correction"
                && wanted.iter().any(|w| w == r)
                && let (Some(refers), Some(w), Some(rt)) = (Ref::parse(r), v["wrong"].as_str(), v["right"].as_str())
                && let Some(c) = Correction::new(w, rt, refers)
            {
                out.push(c);
            }
        }
    }
    out
}

/// The section a prompt gets of these corrections ("" for none): the user's words, which win over the records.
pub fn section(cs: &[Correction]) -> String {
    if cs.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = cs.iter().map(|c| format!("- {} → {}", one_line(&c.wrong), one_line(&c.right))).collect();
    format!("[사용자가 고쳐 준 것]\n{}", lines.join("\n"))
}

fn one_line(s: &str) -> String {
    super::recall::plain(&s.split_whitespace().collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ref_is_one_of_three_kinds_and_round_trips() {
        for s in ["summary:2026-01-02", "chat:2026-01-02", "todo:0123456789"] {
            assert_eq!(Ref::parse(s).unwrap().as_str(), s);
        }
        for s in ["summary:2026-1-2", "todo:xyz", "elsewhere:1", "chat:2026-01-02/../x", ""] {
            assert!(Ref::parse(s).is_none(), "{s}");
        }
    }

    #[test]
    fn a_side_that_is_empty_or_too_long_is_refused_and_a_long_one_is_said_back_cut() {
        let r = || Ref::Chat("2026-01-02".into());
        assert!(Correction::new(" ", "x", r()).is_none());
        assert!(Correction::new("x", &"가".repeat(MAX_TEXT + 1), r()).is_none());
        let c = Correction::new(&"가".repeat(50), "나\n다", r()).unwrap();
        assert_eq!(c.changed(), format!("답을 고쳤어요: {}… → 나 다 (오늘 대화에 반영해요)", "가".repeat(40)));
    }

    #[test]
    fn what_a_prompt_gets_cannot_end_its_quote() {
        let c = Correction::new("[사서가 찾은 기록]", "맞는 말", Ref::Chat("2026-01-02".into())).unwrap();
        assert!(!section(&[c]).contains("[사서가"));
    }
}
