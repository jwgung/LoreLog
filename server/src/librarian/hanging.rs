//! What hangs in a period's days (#61), as the librarian finds it: the calendar events of the days, the to-dos open or
//! proposed, and from the graph the requests not answered and the matters still under way of the last
//! [`RECENT_DAYS`] days. Ranking and wording are the assistant's ([`crate::assistant::period`]).
use rusqlite::{OptionalExtension, params};

use super::Librarian;
use super::graph::{groups, mine};
use crate::assistant::period::{Event, Gathered, Matter, RECENT_DAYS, Request, Span, Todo};

/// Words of a state that say the matter is over (the label the model gave, or the group it was put in). A simple
/// rule: the groups of [`super::graph::group_labels`] are free words of the model's, not a fixed "ended" mark.
pub const ENDED: [&str; 16] = [
    "완료",
    "종료",
    "취소",
    "반려",
    "마감됨",
    "종결",
    "철회",
    "불합격",
    "끝남",
    "끝났",
    "closed",
    "done",
    "cancel",
    "reject",
    "resolved",
    "finished",
];

/// Does this state (or the group it is in) say the matter is over?
pub fn ended(label: &str, group: Option<&str>) -> bool {
    let said = |s: &str| {
        let s = s.to_lowercase();
        ENDED.iter().any(|w| s.contains(w))
    };
    said(label) || group.is_some_and(said)
}

impl Librarian {
    /// What hangs in `span` asked on `today` (`YYYY-MM-DD`). Requests and matters are None when there is no graph
    /// read by the model yet.
    pub fn hanging(&mut self, span: &Span, today: &str) -> anyhow::Result<Gathered> {
        self.ready_graph()?;
        let fmt = |d: chrono::NaiveDate| d.format("%Y-%m-%d").to_string();
        let (from, to) = (fmt(span.from), fmt(span.to));
        let since = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d")
            .map(|t| fmt(t - chrono::Duration::days(RECENT_DAYS)))
            .unwrap_or_else(|_| today.to_string());
        let mut g = Gathered { events: self.events_between(&from, &to)?, ..Gathered::default() };
        let later: bool = self.db.query_row(
            "select exists(select 1 from docs where source = 'event' and day > ?)",
            [today],
            |r| r.get(0),
        )?;
        g.ahead = later || !crate::collect::sources::view(&self.paths).calendars.is_empty();
        g.todos = todos(&self.paths);
        let read: i64 = self.db.query_row("select count(*) from edges where origin = 'model'", [], |r| r.get(0))?;
        if read > 0 {
            g.requests = Some(self.open_requests(&since, today)?);
            g.matters = Some(self.under_way(&since, today)?);
        }
        Ok(g)
    }

    fn events_between(&self, from: &str, to: &str) -> anyhow::Result<Vec<Event>> {
        let mut st = self.db.prepare(
            "select day, title, text, file, case_key from docs where source = 'event' and day >= ? and day <= ?
             order by day, text, n",
        )?;
        let rows: Vec<(String, String, String, String, Option<String>)> = st
            .query_map(params![from, to], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out: Vec<Event> = Vec::new();
        for (day, title, text, refers, case) in rows {
            // the text starts with when it is: `YYYY-MM-DD HH:MM~HH:MM` (a day's event: the day alone)
            let time: String = text.chars().skip(11).take(5).collect();
            let time = if text.chars().nth(10) == Some(' ') && time.len() == 5 && time.as_bytes()[2] == b':' {
                time
            } else {
                String::new()
            };
            if out.iter().any(|e| e.day == day && e.title == title) {
                continue; // one event written twice (two calendars)
            }
            out.push(Event { day, time, title, refers, case });
        }
        Ok(out)
    }

    /// Requests to the user since `since` with no mail of theirs in the matter after them.
    fn open_requests(&self, since: &str, today: &str) -> anyhow::Result<Vec<Request>> {
        let me = mine(&self.db)?;
        if me.is_empty() {
            return Ok(Vec::new()); // whose answer would count is not known
        }
        let mut st = self.db.prepare(
            "select e.src, d.case_key, e.label, coalesce(e.evidence, ''), coalesce(nullif(e.since, ''), d.day), d.title, d.file
             from edges e join docs d on d.n = e.n
             where e.rel = 'requested' and d.case_key is not null and coalesce(nullif(e.since, ''), d.day) between ? and ?
             order by 5 desc, d.n desc",
        )?;
        let rows: Vec<(String, String, String, String, String, String, String)> = st
            .query_map(params![since, today], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut out: Vec<Request> = Vec::new();
        for (src, case, what, evidence, day, title, refers) in rows {
            let by_me = src.strip_prefix("person:").is_some_and(|a| me.contains(a));
            if by_me || out.iter().any(|r| r.case == case) {
                continue;
            }
            let answered: i64 = self.db.query_row(
                "select count(*) from docs where case_key = ? and kind = '보낸 메일' and day >= ?",
                params![case, day],
                |r| r.get(0),
            )?;
            if answered == 0 {
                out.push(Request { day, title, what, evidence, refers, case });
            }
        }
        Ok(out)
    }

    /// Matters with a record since `since` whose last state up to `today` is not an end.
    fn under_way(&self, since: &str, today: &str) -> anyhow::Result<Vec<Matter>> {
        let groups = groups(&self.paths);
        let mut st = self.db.prepare(
            "select case_key from docs where case_key is not null and source != 'file' and day between ? and ?
             group by case_key order by max(day) desc",
        )?;
        let cases: Vec<String> = st.query_map(params![since, today], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for case in cases {
            let Some((last, _)) = self.states(&case, today) else { continue };
            let s = |k: &str| last.get(k).and_then(serde_json::Value::as_str).unwrap_or("").to_string();
            let state = s("label");
            if state.is_empty() || ended(&state, groups.get(&state).map(String::as_str)) {
                continue;
            }
            let latest: Option<(String, String, String)> = self
                .db
                .query_row(
                    "select day, title, file from docs where case_key = ? and day <= ? and source != 'file'
                     order by day desc, n desc limit 1",
                    params![case, today],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((day, title, refers)) = latest else { continue };
            out.push(Matter {
                day,
                title,
                state,
                since: s("since"),
                evidence: s("evidence"),
                guess: s("origin") == "model",
                refers,
                case,
            });
        }
        Ok(out)
    }
}

/// The to-dos open or proposed.
fn todos(paths: &crate::config::Paths) -> Vec<Todo> {
    let d = crate::assistant::todos::load(paths);
    let s = |i: &serde_json::Value, k: &str| i.get(k).and_then(serde_json::Value::as_str).unwrap_or("").to_string();
    d["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| {
            matches!(s(i, "status").as_str(), crate::assistant::todos::OPEN | crate::assistant::todos::PROPOSED)
        })
        .map(|i| Todo {
            title: s(i, "title"),
            due: s(i, "due"),
            day: s(i, "day"),
            proposed: s(i, "status") == crate::assistant::todos::PROPOSED,
        })
        .collect()
}
