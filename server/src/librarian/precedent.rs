//! "전에 어떻게 했지" (#95, ADR 0005): the earlier matters like the one at hand. Like it is, first, with the same
//! counterparty (the organization of the other side, by its mail domain, as the graph's rules know it); when it has
//! none, or none earlier, in the same situation: gone through the same stage, and someone with the same part in it on
//! the other side. No relation is added to the graph for it.
//!
//! Each earlier matter is a hit as the graph gives it, with how it went (its states with their days, oldest first) and
//! the mails it holds (`others`, ADR 0003: a revision and a changed sum are shown both before and after), up to the day
//! asked.
use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use super::Librarian;
use super::graph::{Structure, domain, mine, own_domains};
use super::index::{MASS_MAIL, name_key};

/// how many earlier matters are shown side by side (ADR 0005: two or three; the user's own first, then the latest)
pub const MAX_PRECEDENTS: usize = 3;
/// how many states of a matter's course are shown, the latest
pub const MAX_FLOW: usize = 6;

/// What the earlier matters have in common with the one at hand.
#[derive(Clone, Debug, PartialEq)]
pub enum Like {
    /// the same organization on the other side, by its name
    Counterparty(String),
    /// no counterparty, or no earlier matter with it: the same stage and the same part on the other side (either may be
    /// empty when the matter at hand has none)
    Situation { state: String, role: String },
}

/// [`Like::kind`] of the earlier matters with the same counterparty
pub const BY_COUNTERPARTY: &str = "counterparty";
/// [`Like::kind`] of the earlier matters in the same situation
pub const BY_SITUATION: &str = "situation";

/// On one line: a name the records gave (the model may have read it from a mail) makes no line of its own.
fn one(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Like {
    /// What they are gathered by, for a measure to read (the label's words may change).
    pub fn kind(&self) -> &'static str {
        match self {
            Like::Counterparty(_) => BY_COUNTERPARTY,
            Like::Situation { .. } => BY_SITUATION,
        }
    }

    /// How it is said in the quoted section, on one line.
    pub fn label(&self) -> String {
        match self {
            Like::Counterparty(name) => format!("같은 거래처({})의 지난 건", one(name)),
            Like::Situation { state, role } => {
                let mut what = Vec::new();
                if !state.is_empty() {
                    what.push(format!("상태 {}", one(state)));
                }
                if !role.is_empty() {
                    what.push(format!("상대 역할 {}", one(role)));
                }
                format!("거래처가 같은 지난 건이 없어, 같은 상황({})의 지난 건", what.join(", "))
            }
        }
    }
}

/// The earlier matters like one, and how they are like it.
#[derive(Clone, Debug, PartialEq)]
pub struct Precedents {
    pub like: Like,
    pub hits: Vec<Value>,
}

impl Librarian {
    /// The organization on the other side of a matter up to `day` (`org:<domain>`, and the name it goes by): the one
    /// whose people are in most of its records. Not the user's own organization, not a public mail service, not mass
    /// mail; None for a matter only among the user's own people.
    pub fn counterparty(&self, case: &str, day: &str) -> anyhow::Result<Option<(String, String)>> {
        let me = mine(&self.db)?;
        let own = own_domains(&self.db)?;
        let mut st = self.db.prepare(&format!(
            "with mass as (select n from people where role in ('to', 'cc') group by n having count(*) > {MASS_MAIL})
             select p.addr, count(distinct d.n) from people p join docs d on d.n = p.n
             where d.case_key = ?1 and d.day <= ?2 and d.source != 'file' and d.n not in (select n from mass)
             group by p.addr"
        ))?;
        let mut count: HashMap<String, i64> = HashMap::new();
        for row in st.query_map(params![case, day], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (addr, n) = row?;
            let addr = addr.to_lowercase();
            if me.contains(&addr) {
                continue;
            }
            if let Some(d) = domain(&addr).filter(|d| !own.contains(*d)) {
                *count.entry(format!("org:{d}")).or_default() += n;
            }
        }
        let mut by: Vec<(String, i64)> = count.into_iter().collect();
        by.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let Some((org, _)) = by.into_iter().next() else { return Ok(None) };
        // the name the address book or the records give it, else its domain
        let name = self
            .node(&org)
            .map(|n| n.aliases.first().cloned().filter(|a| !a.is_empty()).unwrap_or(n.name))
            .unwrap_or_else(|| org.trim_start_matches("org:").to_string());
        Ok(Some((org, name)))
    }

    /// The earlier matters like this one up to `day`, at most `n`, the user's own and the latest first; None when
    /// nothing is like it. `q`: the words asked, for the excerpts.
    pub fn precedents(&self, case: &str, day: &str, q: &str, n: usize) -> anyhow::Result<Option<Precedents>> {
        let structure = Structure::load(&self.paths);
        if let Some((org, name)) = self.counterparty(case, day)? {
            let ids = self.with_same(vec![org])?;
            let cases = self.cases_with(&ids)?;
            let hits = self.earlier(cases, case, day, q, n, &structure)?;
            if !hits.is_empty() {
                return Ok(Some(Precedents { like: Like::Counterparty(name), hits }));
            }
        }
        // the situation: the stage it is in, and the parts of the other side
        let state = self.states(case, day).and_then(|(s, _)| s["label"].as_str().map(str::to_string));
        let roles = self.their_roles(case, day)?;
        if state.is_none() && roles.is_empty() {
            return Ok(None);
        }
        let same = |a: &str, b: &str| {
            name_key(a) == name_key(b)
                || matches!((structure.groups.get(a), structure.groups.get(b)), (Some(x), Some(y)) if name_key(x) == name_key(y))
        };
        let mut cases: Option<HashSet<String>> = None;
        if let Some(s) = &state {
            let mut st = self
                .db
                .prepare("select src, label from edges where rel = 'state' and src like 'case:%' and since <= ?")?;
            let rows: Vec<(String, String)> =
                st.query_map([day], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            cases = Some(rows.into_iter().filter(|(_, l)| same(l, s)).map(|(c, _)| c).collect());
        }
        if !roles.is_empty() {
            let me = mine(&self.db)?;
            let mut st = self.db.prepare(
                "select src, dst, label from edges where rel = 'role' and dst like 'case:%'
                 and (since is null or since = '' or since <= ?)",
            )?;
            let rows: Vec<(String, String, String)> =
                st.query_map([day], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
            let by_role: HashSet<String> = rows
                .into_iter()
                .filter(|(src, _, l)| !is_me(src, &me) && roles.iter().any(|r| same(l, r)))
                .map(|(_, c, _)| c)
                .collect();
            cases = Some(match cases {
                Some(c) => c.intersection(&by_role).cloned().collect(),
                None => by_role,
            });
        }
        let hits = self.earlier(cases.unwrap_or_default(), case, day, q, n, &structure)?;
        if hits.is_empty() {
            return Ok(None);
        }
        let role = roles.first().cloned().unwrap_or_default();
        Ok(Some(Precedents { like: Like::Situation { state: state.unwrap_or_default(), role }, hits }))
    }

    /// The parts the other side has in a matter (the labels of its roles not the user's), as the records name them.
    fn their_roles(&self, case: &str, day: &str) -> anyhow::Result<Vec<String>> {
        let me = mine(&self.db)?;
        let mut st = self.db.prepare(
            "select src, label from edges where rel = 'role' and dst = ? and (since is null or since = '' or since <= ?)
             order by rowid",
        )?;
        let rows: Vec<(String, String)> =
            st.query_map(params![case, day], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut out: Vec<String> = Vec::new();
        for (src, label) in rows {
            if !is_me(&src, &me) && !label.trim().is_empty() && !out.contains(&label) {
                out.push(label);
            }
        }
        Ok(out)
    }

    /// The matters among `cases` other than `except` with a record up to `day`, as hits with how they went: the user's
    /// own first, then the latest, at most `n`.
    fn earlier(
        &self,
        cases: HashSet<String>,
        except: &str,
        day: &str,
        q: &str,
        n: usize,
        structure: &Structure,
    ) -> anyhow::Result<Vec<Value>> {
        let mut ranked: Vec<(i64, String, i64, String)> = Vec::new();
        for c in cases.into_iter().filter(|c| c != except) {
            let row: Option<(i64, Option<String>, Option<i64>)> = self
                .db
                .query_row(
                    "select coalesce(max(involved), 0), max(day), (select n from docs where case_key = ?1 and day <= ?2 and source != 'file' order by day desc, n desc limit 1)
                     from docs where case_key = ?1 and day <= ?2 and source != 'file'",
                    params![c, day],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            if let Some((inv, Some(last), Some(at))) = row {
                ranked.push((inv, last, at, c));
            }
        }
        ranked.sort_by(|x, y| y.0.cmp(&x.0).then(y.1.cmp(&x.1)).then(x.3.cmp(&y.3)));
        let mut out = Vec::new();
        for (_, _, at, c) in ranked.into_iter().take(n) {
            let mut h = self.known_hit(&c, at, q, day, structure)?;
            let flow = self.flow(&c, day)?;
            if !flow.is_empty() {
                h["flow"] = json!(flow);
            }
            out.push(h);
        }
        Ok(out)
    }

    /// How a matter went up to `day`: its states, oldest first (a state said again right after is one), the latest
    /// [`MAX_FLOW`]: `[{label, since, origin}]`.
    pub fn flow(&self, case: &str, day: &str) -> anyhow::Result<Vec<Value>> {
        let mut st = self.db.prepare(
            "select label, since, coalesce(origin, '') from edges where src = ? and rel = 'state' and since <= ?
             order by since, rowid",
        )?;
        let rows: Vec<(String, String, String)> = st
            .query_map(params![case, day], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out: Vec<(String, String, String)> = Vec::new();
        for r in rows {
            if out.last().is_some_and(|l| name_key(&l.0) == name_key(&r.0)) {
                continue;
            }
            out.push(r);
        }
        let skip = out.len().saturating_sub(MAX_FLOW);
        Ok(out.into_iter().skip(skip).map(|(l, s, o)| json!({"label": l, "since": s, "origin": o})).collect())
    }

    /// The matters under way on `day` that the user takes part in, the latest first, at most `n`: a record in the last
    /// [`crate::assistant::period::RECENT_DAYS`] days, a last state that is no end.
    pub fn open_cases(&self, day: &str, n: usize) -> anyhow::Result<Vec<String>> {
        let since = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .map(|t| (t - chrono::Duration::days(crate::assistant::period::RECENT_DAYS)).format("%Y-%m-%d").to_string())
            .unwrap_or_else(|_| day.to_string());
        let mut out = Vec::new();
        for m in self.under_way(&since, day)? {
            let involved: i64 = self.db.query_row(
                "select coalesce(max(involved), 0) from docs where case_key = ? and day <= ?",
                params![m.case, day],
                |r| r.get(0),
            )?;
            if involved == 1 && !out.contains(&m.case) {
                out.push(m.case);
            }
            if out.len() >= n {
                break;
            }
        }
        Ok(out)
    }
}

fn is_me(src: &str, me: &HashSet<String>) -> bool {
    src.strip_prefix("person:").is_some_and(|a| me.contains(a))
}
