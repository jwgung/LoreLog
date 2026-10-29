//! Segmenter: samples in, closed time segments out.
//!
//! A segment closes when the window changes, when the user goes away or comes back, or when it has lasted
//! `max_segment_s`. Time is epoch seconds as f64.
use crate::config::{Config, HIDDEN_TITLE};
use serde::Serialize;

/// One look at the machine.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sample {
    /// Empty when nothing usable is in front (locked screen, no window, or a window we may not read).
    pub app: String,
    pub title: String,
    /// Seconds since the last keyboard or mouse input.
    pub idle_s: f64,
    /// A window is in front but its process may not be read (an elevated app). Not the same as a locked screen:
    /// the user is working, we just cannot say in what.
    pub unreadable: bool,
}

/// What goes to the server (one NDJSON line).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Record {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub ts: i64,
    pub end: i64,
    pub dur_s: i64,
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Window { app: String, title: String },
    Idle(&'static str),
}

#[derive(Debug, Clone)]
struct Cur {
    kind: Kind,
    start: f64,
}

pub struct Segmenter {
    idle_after_s: f64,
    max_segment_s: f64,
    min_segment_s: f64,
    hide_apps: Vec<String>,
    hide_contains: Vec<String>,
    host: String,
    cur: Option<Cur>,
}

impl Segmenter {
    pub fn new(cfg: &Config, host: &str) -> Self {
        Segmenter {
            idle_after_s: cfg.idle_after_s,
            max_segment_s: cfg.max_segment_s,
            min_segment_s: cfg.min_segment_s,
            hide_apps: cfg
                .hide_title_apps
                .iter()
                .map(|s| s.to_lowercase())
                .collect(),
            hide_contains: cfg
                .hide_title_contains
                .iter()
                .map(|s| s.to_lowercase())
                .collect(),
            host: host.to_string(),
            cur: None,
        }
    }

    /// The title as it will be recorded (private windows and secret managers are hidden).
    pub fn shown_title(&self, app: &str, title: &str) -> String {
        let lower = title.to_lowercase();
        if self.hide_apps.contains(&app.to_lowercase())
            || self.hide_contains.iter().any(|p| lower.contains(p))
        {
            HIDDEN_TITLE.to_string()
        } else {
            title.to_string()
        }
    }

    fn close(&mut self, end: f64) -> Option<Record> {
        let cur = self.cur.take()?;
        if end <= cur.start {
            return None;
        }
        let dur = end - cur.start;
        let mut rec = Record {
            kind: "idle",
            ts: (cur.start * 1000.0) as i64,
            end: (end * 1000.0) as i64,
            dur_s: dur as i64,
            host: self.host.clone(),
            app: None,
            title: None,
            reason: None,
        };
        match cur.kind {
            Kind::Window { app, title } => {
                if dur < self.min_segment_s {
                    return None;
                }
                rec.kind = "window";
                rec.app = Some(app);
                rec.title = Some(title);
            }
            Kind::Idle(reason) => rec.reason = Some(reason),
        }
        Some(rec)
    }

    /// Feed one sample taken at `now`. Returns the segments it closed (0, 1 or 2 records).
    pub fn feed(&mut self, now: f64, s: &Sample) -> Vec<Record> {
        let away = s.app.is_empty() || s.idle_s >= self.idle_after_s; // an unreadable window has an empty app too
        // the last real input, not the moment we noticed: an away period starts there
        let since = (now - s.idle_s).max(self.cur.as_ref().map_or(now, |c| c.start));
        if away {
            let reason = if s.unreadable {
                "unreadable"
            } else if s.app.is_empty() {
                "locked"
            } else {
                "idle"
            };
            if matches!(&self.cur, Some(Cur { kind: Kind::Idle(r), .. }) if *r == reason) {
                return self.cut(now);
            }
            let at = if s.app.is_empty() { now } else { since };
            let out = self.close(at);
            self.cur = Some(Cur {
                kind: Kind::Idle(reason),
                start: at,
            });
            return out.into_iter().collect();
        }
        let want = Kind::Window {
            app: s.app.clone(),
            title: self.shown_title(&s.app, &s.title),
        };
        if self.cur.as_ref().is_some_and(|c| c.kind == want) {
            return self.cut(now);
        }
        // coming back from away: the new window starts at the first input after the break
        let began = match &self.cur {
            Some(c) if matches!(c.kind, Kind::Idle(_)) => (now - s.idle_s).max(c.start),
            _ => now,
        };
        let out = self.close(began);
        self.cur = Some(Cur {
            kind: want,
            start: began,
        });
        out.into_iter().collect()
    }

    fn cut(&mut self, now: f64) -> Vec<Record> {
        let Some(cur) = &self.cur else { return vec![] };
        if now - cur.start < self.max_segment_s {
            return vec![];
        }
        let kind = cur.kind.clone();
        let rec = self.close(now);
        self.cur = Some(Cur { kind, start: now });
        rec.into_iter().collect()
    }

    /// Close the running segment (the agent is stopping, or the machine slept and `now` is the last time it was seen).
    pub fn finish(&mut self, now: f64) -> Vec<Record> {
        self.close(now).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::default()
    }

    fn sample(app: &str, title: &str, idle: f64) -> Sample {
        Sample {
            app: app.into(),
            title: title.into(),
            idle_s: idle,
            ..Sample::default()
        }
    }

    /// Every record closed by the samples, including the last (finished at the final time).
    fn run(samples: &[(f64, &str, &str, f64)]) -> Vec<Record> {
        let mut seg = Segmenter::new(&cfg(), "H");
        let mut out = vec![];
        for (t, app, title, idle) in samples {
            out.extend(seg.feed(*t, &sample(app, title, *idle)));
        }
        out.extend(seg.finish(samples.last().unwrap().0));
        out
    }

    fn apps(recs: &[Record]) -> Vec<(String, i64)> {
        recs.iter()
            .map(|r| (r.app.clone().unwrap_or_default(), r.dur_s))
            .collect()
    }

    #[test]
    fn window_change_closes_the_segment() {
        let out = run(&[
            (0.0, "a.exe", "x", 0.0),
            (10.0, "a.exe", "x", 0.0),
            (20.0, "b.exe", "y", 0.0),
            (40.0, "b.exe", "y", 0.0),
        ]);
        assert_eq!(apps(&out), vec![("a.exe".into(), 20), ("b.exe".into(), 20)]);
        assert_eq!(
            (out[0].ts, out[0].end, out[0].host.as_str()),
            (0, 20_000, "H")
        );
    }

    #[test]
    fn title_change_in_the_same_app_is_a_new_segment() {
        let out = run(&[
            (0.0, "chrome.exe", "메일", 0.0),
            (30.0, "chrome.exe", "견적서", 0.0),
            (60.0, "chrome.exe", "견적서", 0.0),
        ]);
        let titles: Vec<_> = out.iter().map(|r| r.title.clone().unwrap()).collect();
        assert_eq!(titles, vec!["메일", "견적서"]);
    }

    #[test]
    fn alt_tab_flicker_is_not_recorded() {
        let out = run(&[
            (0.0, "a.exe", "x", 0.0),
            (30.0, "b.exe", "y", 0.0),
            (31.0, "a.exe", "x", 0.0),
            (60.0, "a.exe", "x", 0.0),
        ]);
        assert_eq!(apps(&out), vec![("a.exe".into(), 30), ("a.exe".into(), 29)]);
    }

    #[test]
    fn long_stay_is_cut_into_pieces() {
        let samples: Vec<_> = (0..15)
            .map(|i| (i as f64 * 100.0, "a.exe", "x", 0.0))
            .collect();
        let out = run(&samples);
        assert_eq!(
            out.iter().map(|r| r.dur_s).collect::<Vec<_>>(),
            vec![600, 600, 200]
        );
    }

    #[test]
    fn away_starts_at_the_last_input_not_when_we_noticed() {
        // last input at t=100; noticed at t=400 (idle 300). The window ends at 100 and the idle period starts there.
        let out = run(&[
            (0.0, "a.exe", "x", 0.0),
            (100.0, "a.exe", "x", 0.0),
            (400.0, "a.exe", "x", 300.0),
            (700.0, "a.exe", "x", 600.0),
        ]);
        assert_eq!((out[0].kind, out[0].dur_s), ("window", 100));
        assert_eq!(
            (out[1].kind, out[1].ts, out[1].end, out[1].reason),
            ("idle", 100_000, 700_000, Some("idle"))
        );
    }

    #[test]
    fn coming_back_starts_the_new_window_at_the_first_input() {
        let out = run(&[
            (0.0, "a.exe", "x", 0.0),
            (200.0, "a.exe", "x", 0.0),
            (500.0, "a.exe", "x", 300.0),
            (1000.0, "a.exe", "x", 0.0),
            (1030.0, "a.exe", "x", 0.0),
        ]);
        let got: Vec<_> = out
            .iter()
            .map(|r| (r.kind, r.ts / 1000, r.end / 1000))
            .collect();
        assert_eq!(
            got,
            vec![
                ("window", 0, 200),
                ("idle", 200, 1000),
                ("window", 1000, 1030)
            ]
        );
    }

    #[test]
    fn away_from_the_start_has_no_zero_length_window() {
        let out = run(&[
            (0.0, "a.exe", "x", 400.0),
            (100.0, "a.exe", "x", 500.0),
            (200.0, "a.exe", "x", 0.0),
            (230.0, "a.exe", "x", 0.0),
        ]);
        assert_eq!(
            out.iter().map(|r| (r.kind, r.dur_s)).collect::<Vec<_>>(),
            vec![("idle", 200), ("window", 30)]
        );
    }

    #[test]
    fn locked_screen_is_recorded_as_locked() {
        let out = run(&[
            (0.0, "a.exe", "x", 0.0),
            (50.0, "", "", 50.0),
            (200.0, "", "", 200.0),
        ]);
        assert_eq!(
            out.iter().map(|r| (r.kind, r.reason)).collect::<Vec<_>>(),
            vec![("window", None), ("idle", Some("locked"))]
        );
        assert_eq!(out[1].dur_s, 150);
    }

    #[test]
    fn an_unreadable_window_is_not_a_locked_screen() {
        let mut seg = Segmenter::new(&cfg(), "H");
        let elevated = Sample {
            idle_s: 0.0,
            unreadable: true,
            ..Sample::default()
        };
        let mut out = seg.feed(0.0, &sample("a.exe", "x", 0.0));
        out.extend(seg.feed(50.0, &elevated));
        out.extend(seg.feed(120.0, &elevated));
        out.extend(seg.finish(150.0));
        assert_eq!(
            out.iter().map(|r| (r.kind, r.reason)).collect::<Vec<_>>(),
            vec![("window", None), ("idle", Some("unreadable"))]
        );
        assert_eq!(out[1].dur_s, 100);
    }

    #[test]
    fn private_titles_are_hidden() {
        let out = run(&[
            (0.0, "msedge.exe", "InPrivate - 은행", 0.0),
            (30.0, "KeePass.exe", "비밀번호 db", 0.0),
            (60.0, "a.exe", "x", 0.0),
            (90.0, "a.exe", "x", 0.0),
        ]);
        assert_eq!(out[0].title.as_deref(), Some(HIDDEN_TITLE));
        assert_eq!(out[1].title.as_deref(), Some(HIDDEN_TITLE));
    }

    #[test]
    fn a_record_serializes_to_the_fields_the_server_reads() {
        let out = run(&[(0.0, "a.exe", "문서", 0.0), (20.0, "a.exe", "문서", 0.0)]);
        let v: serde_json::Value = serde_json::to_value(&out[0]).unwrap();
        assert_eq!(v["type"], "window");
        assert_eq!(
            (v["app"].as_str(), v["title"].as_str(), v["dur_s"].as_i64()),
            (Some("a.exe"), Some("문서"), Some(20))
        );
        assert!(v.get("reason").is_none());
    }
}
