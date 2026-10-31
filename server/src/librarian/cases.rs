//! One matter beyond one mail thread (#8): what the user calls "그 건". The librarian joins, when the index changes:
//!   - the mails of one thread (References, In-Reply-To);
//!   - mails of the same subject (RE:, FW:, 회신:, 전달: left out) at most [`CASE_MAIL_DAYS`] apart, with someone in
//!     common who is not the user (a forward to the same client is a new thread, the same matter);
//!   - an event of that subject at most [`CASE_EVENT_DAYS`] from one of its mails, with someone in common;
//!   - a file that came with a mail (same Message-ID and day), with that mail.
//!
//! Mass mail is joined only by its thread. Anything less sure (a todo and a mail, a subject said another way) is left
//! to the question. A matter is `docs.case_key`; documents in none have no key.
use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{Connection, params};

use super::index::{CASE_EVENT_DAYS, CASE_MAIL_DAYS, MASS_MAIL};
use crate::assistant::context::subj_key;

struct Rec {
    n: i64,
    source: String,
    day: String,
    title: String,
    thread: Option<String>,
    mid: Option<String>,
}

/// Union-find over record numbers.
#[derive(Default)]
struct Sets(HashMap<i64, i64>);

impl Sets {
    fn root(&mut self, n: i64) -> i64 {
        let p = *self.0.entry(n).or_insert(n);
        if p == n {
            return n;
        }
        let r = self.root(p);
        self.0.insert(n, r);
        r
    }

    fn join(&mut self, a: i64, b: i64) {
        let (ra, rb) = (self.root(a), self.root(b));
        if ra != rb {
            // the older record names the matter
            self.0.insert(ra.max(rb), ra.min(rb));
        }
    }
}

fn days_apart(a: &str, b: &str) -> Option<i64> {
    let d = |s: &str| chrono::NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok();
    Some((d(a)? - d(b)?).num_days().abs())
}

/// Work out the matters again and write the keys that changed. -> keys written
pub fn relink(db: &Connection) -> rusqlite::Result<usize> {
    let recs: Vec<Rec> = {
        let mut st = db
            .prepare("select n, source, day, title, thread, mid from docs where source in ('mail', 'event', 'file')")?;
        st.query_map([], |r| {
            Ok(Rec {
                n: r.get(0)?,
                source: r.get(1)?,
                day: r.get(2)?,
                title: r.get(3)?,
                thread: r.get(4)?,
                mid: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?
    };
    let mass: HashSet<i64> = {
        let mut st = db.prepare(&format!(
            "select n from people where role in ('to', 'cc') group by n having count(*) > {MASS_MAIL}"
        ))?;
        st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    // who each record is with, the user left out (the senders of their sent mail)
    let mut with: HashMap<i64, HashSet<String>> = HashMap::new();
    {
        let mut st = db.prepare(
            "with mine as (select distinct p.addr from people p join docs d on d.n = p.n where d.kind = '보낸 메일' and p.role = 'from')
             select n, addr from people where addr not in (select addr from mine)",
        )?;
        for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (n, a) = row?;
            with.entry(n).or_default().insert(a);
        }
    }
    let common = |a: i64, b: i64| match (with.get(&a), with.get(&b)) {
        (Some(x), Some(y)) => !x.is_disjoint(y),
        _ => false,
    };
    let mut sets = Sets::default();
    let mut by_thread: HashMap<&str, i64> = HashMap::new();
    let mut by_subject: BTreeMap<String, Vec<&Rec>> = BTreeMap::new();
    let mut by_mid: HashMap<(&str, &str), i64> = HashMap::new();
    for r in recs.iter().filter(|r| r.source == "mail") {
        sets.root(r.n);
        if let Some(t) = &r.thread {
            match by_thread.get(t.as_str()) {
                Some(&first) => sets.join(first, r.n),
                None => {
                    by_thread.insert(t, r.n);
                }
            }
        }
        if let Some(m) = &r.mid {
            by_mid.entry((m.as_str(), r.day.as_str())).or_insert(r.n);
        }
        let key = subj_key(&r.title);
        if key.chars().count() >= 2 && !mass.contains(&r.n) {
            by_subject.entry(key).or_default().push(r);
        }
    }
    for mails in by_subject.values_mut() {
        mails.sort_by(|a, b| a.day.cmp(&b.day).then(a.n.cmp(&b.n)));
        for (i, a) in mails.iter().enumerate() {
            for b in &mails[i + 1..] {
                if days_apart(&a.day, &b.day).is_none_or(|d| d > CASE_MAIL_DAYS) {
                    break;
                }
                if common(a.n, b.n) {
                    sets.join(a.n, b.n);
                }
            }
        }
    }
    let mut joined: HashSet<i64> = HashSet::new(); // events and files that are part of a matter
    for e in recs.iter().filter(|r| r.source == "event") {
        let Some(mails) = by_subject.get(&subj_key(&e.title)) else { continue };
        let near =
            mails.iter().find(|m| days_apart(&m.day, &e.day).is_some_and(|d| d <= CASE_EVENT_DAYS) && common(m.n, e.n));
        if let Some(m) = near {
            sets.join(m.n, e.n);
            joined.insert(e.n);
        }
    }
    for f in recs.iter().filter(|r| r.source == "file") {
        if let Some(&m) = f.mid.as_deref().and_then(|mid| by_mid.get(&(mid, f.day.as_str()))) {
            sets.join(m, f.n);
            joined.insert(f.n);
        }
    }
    let mut written = 0;
    let mut st = db.prepare_cached("update docs set case_key = ? where n = ? and case_key is not ?")?;
    for r in &recs {
        let key = (r.source == "mail" || joined.contains(&r.n)).then(|| format!("case:{}", sets.root(r.n)));
        written += st.execute(params![key, r.n, key])?;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_apart_reads_the_day_of_a_time_too() {
        assert_eq!(days_apart("2026-03-01", "2026-03-31T10:00:00"), Some(30));
        assert_eq!(days_apart("", "2026-03-01"), None);
    }
}
