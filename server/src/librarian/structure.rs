//! Asking the user back the structure the graph found (#63). The model groups the record's words for roles and states
//! (견적 요청, 견적서 송부, 가격 제안 → 견적); a group that has held still is asked about once, in the user's evening
//! question: "'견적 요청·견적서 송부·가격 제안'을 '견적'으로 묶고 견적→계약→납품 순서로 볼까요?". What the user answers is
//! kept as corrections (`label:`, `order:`), and is the user's structure from then on; a group not answered stays the
//! model's, used inside only.
//!
//! When: after a graph update, a group of three record words or more, in three matters or more, the same in two
//! updates in a row, with nothing of it corrected and never asked before. One question in seven days at most. A group
//! of states asks for an order too, when most of its matters go through the stages in the same order.
use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::Librarian;
use super::graph::Structure;
use crate::config::Paths;

/// the fewest record words, and matters, a group is asked about with
const MIN: usize = 3;
/// days between two questions at least
const EVERY: i64 = 7;
/// record words a question names
const SHOWN: usize = 3;
/// stages an order is asked with at most
const STAGES: usize = 4;

/// A group to ask about.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// the model's name for it
    pub group: String,
    /// its record words, the most used first
    pub words: Vec<String>,
    /// the order of the stages most of its matters go through (empty: none to ask, or a group of roles)
    pub order: Vec<String>,
    pub question: String,
}

/// A question asked.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Asked {
    pub day: String,
    #[serde(flatten)]
    pub what: Candidate,
    /// the user answered it (said yes, said how, or said no)
    #[serde(default)]
    pub answered: bool,
}

/// group -> its record words
pub type Seen = HashMap<String, Vec<String>>;

#[derive(Default, Serialize, Deserialize)]
struct State {
    /// the groups at the last update
    #[serde(default)]
    seen: Seen,
    #[serde(default)]
    due: Option<Candidate>,
    #[serde(default)]
    asked: Vec<Asked>,
}

fn state_file(paths: &Paths) -> std::path::PathBuf {
    paths.data.join("structure.json")
}

fn load(paths: &Paths) -> State {
    std::fs::read_to_string(state_file(paths)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(paths: &Paths, s: &State) -> anyhow::Result<()> {
    crate::fsutil::ensure_private_dir(&paths.data)?;
    crate::fsutil::write_private_atomic(&state_file(paths), serde_json::to_string(s)?.as_bytes())?;
    Ok(())
}

/// After a graph update: remember how the groups are made up, and keep the one to ask about next. -> its question
pub fn note(lib: &Librarian) -> anyhow::Result<Option<String>> {
    let mut s = load(&lib.paths);
    let asked: HashSet<String> = s.asked.iter().map(|a| a.what.group.clone()).collect();
    let (seen, due) = pick(lib, &s.seen, &asked)?;
    s.seen = seen;
    s.due = due;
    save(&lib.paths, &s)?;
    Ok(s.due.as_ref().map(|c| c.question.clone()))
}

/// The groups as they are now, and the one to ask about given how they were at the last update (`before`) and the
/// groups asked about already. Reads only.
pub fn pick(lib: &Librarian, before: &Seen, asked: &HashSet<String>) -> anyhow::Result<(Seen, Option<Candidate>)> {
    let users = Structure::load(&lib.paths);
    let model = super::graph::model_groups(&lib.paths);
    // every word of a role or a state, where it is said, in order
    let mut st = lib.db.prepare(
        "select rel, label, src, coalesce(dst, '') from edges where rel in ('role', 'state') and label != ''
         order by since, rowid",
    )?;
    let rows: Vec<(String, String, String, String)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
    let ordered: HashSet<String> = users.orders.iter().flatten().cloned().collect();
    // group -> (record word -> uses, matters, states said, roles said)
    #[derive(Default)]
    struct G {
        uses: HashMap<String, usize>,
        matters: HashSet<String>,
        states: usize,
        roles: usize,
        touched: bool,
    }
    let mut groups: HashMap<String, G> = HashMap::new();
    for (rel, label, src, dst) in &rows {
        let Some(g) = model.get(label) else { continue };
        let e = groups.entry(g.clone()).or_default();
        if users.named.contains(label) {
            e.touched = true; // the user said something of it
            continue;
        }
        *e.uses.entry(label.clone()).or_default() += 1;
        let matter = if rel == "state" { src } else { dst };
        if matter.starts_with("case:") {
            e.matters.insert(matter.clone());
        }
        if rel == "state" { e.states += 1 } else { e.roles += 1 }
    }
    let seen: Seen = groups
        .iter()
        .map(|(g, e)| {
            let mut w: Vec<String> = e.uses.keys().cloned().collect();
            w.sort();
            (g.clone(), w)
        })
        .collect();
    let mut fits: Vec<(&String, &G)> = groups
        .iter()
        .filter(|(g, e)| {
            !e.touched
                && !ordered.contains(*g)
                && !asked.contains(*g)
                && e.uses.len() >= MIN
                && e.matters.len() >= MIN
                && before.get(*g) == seen.get(*g)
        })
        .collect();
    fits.sort_by(|a, b| b.1.matters.len().cmp(&a.1.matters.len()).then(a.0.cmp(b.0)));
    let Some((group, g)) = fits.first() else { return Ok((seen, None)) };
    let mut words: Vec<(&String, &usize)> = g.uses.iter().collect();
    words.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let words: Vec<String> = words.into_iter().map(|(w, _)| w.clone()).collect();
    // how its matters went, by the groups of their states (the user's names where given)
    let mut went: HashMap<String, Vec<String>> = HashMap::new();
    let mut last: HashMap<String, String> = HashMap::new();
    for (rel, label, src, _) in &rows {
        if rel != "state" || !g.matters.contains(src) {
            continue;
        }
        let stage = users.groups.get(label).cloned().unwrap_or_else(|| label.clone());
        let seq = went.entry(src.clone()).or_default();
        if !seq.contains(&stage) {
            seq.push(stage.clone());
        }
        last.insert(src.clone(), stage);
    }
    let order = if g.states >= g.roles { order_of(group, &went) } else { Vec::new() };
    // a matter for the example: the one said last
    let example = rows
        .iter()
        .rev()
        .find_map(|(rel, l, src, dst)| {
            let m = if rel == "state" { src } else { dst };
            (g.uses.contains_key(l) && g.matters.contains(m)).then_some(m)
        })
        .and_then(|m| {
            let name = lib.node(m).map(|n| n.name).filter(|n| !n.is_empty())?;
            Some((name, last.get(m).cloned()))
        });
    let question = question(group, &words, &order, example.as_ref());
    Ok((seen, Some(Candidate { group: (*group).clone(), words, order, question })))
}

/// The order of the stages most matters of this group went through, when there is one: the stages at least half of
/// them reached, and four of five matters with two of those or more went through them in that order.
fn order_of(group: &str, went: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut count: HashMap<&String, usize> = HashMap::new();
    for seq in went.values() {
        for s in seq {
            *count.entry(s).or_default() += 1;
        }
    }
    let half = went.len().div_ceil(2);
    let mut stages: Vec<(&String, usize)> = count.into_iter().filter(|(_, n)| *n >= half).collect();
    stages.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let stages: Vec<&String> = stages.into_iter().take(STAGES).map(|(s, _)| s).collect();
    if stages.len() < 2 || !stages.iter().any(|s| *s == group) {
        return Vec::new();
    }
    // where each stage comes, on average, in the matters that reached it
    let mut at: Vec<(f64, &String)> = stages
        .iter()
        .map(|s| {
            let (sum, n) = went.values().fold((0usize, 0usize), |(sum, n), seq| {
                let kept: Vec<&String> = seq.iter().filter(|x| stages.contains(x)).collect();
                match kept.iter().position(|x| x == s) {
                    Some(i) => (sum + i, n + 1),
                    None => (sum, n),
                }
            });
            (sum as f64 / n.max(1) as f64, *s)
        })
        .collect();
    at.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)));
    let order: Vec<&String> = at.into_iter().map(|(_, s)| s).collect();
    let (mut with_two, mut along) = (0, 0);
    for seq in went.values() {
        let kept: Vec<usize> = seq.iter().filter_map(|x| order.iter().position(|o| *o == x)).collect();
        if kept.len() >= 2 {
            with_two += 1;
            along += usize::from(kept.windows(2).all(|w| w[0] < w[1]));
        }
    }
    if with_two >= MIN && along * 5 >= with_two * 4 { order.into_iter().cloned().collect() } else { Vec::new() }
}

/// Does the last syllable of a word end in a consonant (and is it ㄹ)?
fn coda(word: &str) -> (bool, bool) {
    match word.trim().chars().last() {
        Some(c @ '가'..='힣') => {
            let t = (c as u32 - '가' as u32) % 28;
            (t != 0, t == 8)
        }
        Some(c) if c.is_ascii_digit() => (matches!(c, '0' | '1' | '3' | '6' | '7' | '8'), matches!(c, '1' | '7' | '8')),
        _ => (false, false),
    }
}

/// 을/를 after a word.
fn eul(word: &str) -> &'static str {
    if coda(word).0 { "을" } else { "를" }
}

/// 으로/로 after a word.
fn euro(word: &str) -> &'static str {
    match coda(word) {
        (true, false) => "으로",
        _ => "로",
    }
}

/// The question, in the user's most used record words, with a real matter for an example.
fn question(group: &str, words: &[String], order: &[String], example: Option<&(String, Option<String>)>) -> String {
    let mut shown = words.iter().take(SHOWN).cloned().collect::<Vec<_>>().join("·");
    if words.len() > SHOWN {
        shown += &format!(" 외 {}개", words.len() - SHOWN);
    }
    let what = if order.is_empty() {
        format!("'{shown}'{} '{group}'{} 묶어 볼까요?", eul(&shown), euro(group))
    } else {
        format!("'{shown}'{} '{group}'{} 묶고 {} 순서로 볼까요?", eul(&shown), euro(group), order.join("→"))
    };
    match example {
        Some((m, Some(now))) => format!("{what} (예: {m} 건은 지금 {now})"),
        Some((m, None)) => format!("{what} (예: {m} 건)"),
        None => what,
    }
}

fn days(from: &str, to: &str) -> Option<i64> {
    let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok();
    Some((d(to)? - d(from)?).num_days())
}

/// The evening's question of `day` when a structure question is due that day (the same one again for a draft made
/// again that day). With `ask` it is kept as asked; else only looked at.
pub fn due(paths: &Paths, day: &str, ask: bool) -> Option<String> {
    let mut s = load(paths);
    if let Some(last) = s.asked.last() {
        if last.day == day {
            return Some(last.what.question.clone());
        }
        if days(&last.day, day).is_none_or(|d| d < EVERY) {
            return None;
        }
    }
    let c = s.due.clone()?;
    if ask {
        s.due = None;
        s.asked.push(Asked { day: day.into(), what: c.clone(), answered: false });
        if let Err(e) = save(paths, &s) {
            log::warn!("structure question not kept: {}", crate::errors::kind(&e.to_string()));
            return None; // not asked twice for want of remembering it
        }
    }
    Some(c.question)
}

/// The structure question asked and not answered yet, within the days before the next may be asked.
pub fn pending(paths: &Paths, day: &str) -> Option<Asked> {
    let last = load(paths).asked.pop()?;
    (!last.answered && days(&last.day, day).is_some_and(|d| (0..EVERY).contains(&d))).then_some(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_particles_follow_the_last_syllable() {
        assert_eq!((eul("가격 제안"), euro("견적")), ("을", "으로"));
        assert_eq!((eul("견적서 송부·계약"), euro("계약 체결")), ("을", "로"));
        assert_eq!((eul("메일"), euro("메일")), ("을", "로"));
        assert_eq!((eul("지원자"), euro("지원자")), ("를", "로"));
    }

    #[test]
    fn an_order_needs_most_matters_to_go_along() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let went: HashMap<String, Vec<String>> = [
            ("a", s(&["견적", "계약", "납품"])),
            ("b", s(&["견적", "계약"])),
            ("c", s(&["견적", "납품"])),
            ("d", s(&["견적"])),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        assert_eq!(order_of("견적", &went), s(&["견적", "계약", "납품"]));
        let mut mixed = went.clone();
        mixed.insert("e".into(), s(&["계약", "견적"]));
        mixed.insert("f".into(), s(&["납품", "견적"]));
        assert!(order_of("견적", &mixed).is_empty(), "two of five go another way");
    }
}
