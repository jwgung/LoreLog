//! The collection loop body: sample, segment, spool, send. The clock and the OS sampler are injected so it can be tested.
use crate::config::Config;
use crate::segments::{Sample, Segmenter};
use crate::sender::send_spool;
use crate::spool::Spool;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const LOG_MAX: u64 = 1_000_000;

/// The OS side: what is in front now.
pub trait Sampler {
    fn sample(&mut self) -> Result<Sample, String>;
}

pub fn system_clock() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

pub fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

/// Append to agent.log (the background process has no console). Never write the token here.
pub fn log(dir: &Path, msg: &str) {
    let path = dir.join("agent.log");
    let _ = std::fs::create_dir_all(dir);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_MAX) {
        let _ = std::fs::write(&path, "");
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{:.0} {msg}", system_clock());
    }
}

pub struct Agent<S: Sampler> {
    cfg: Config,
    sampler: S,
    seg: Segmenter,
    spool: Spool,
    clock: Box<dyn FnMut() -> f64>,
    last: Option<f64>,
    last_flush: f64,
    failing: String,
}

impl<S: Sampler> Agent<S> {
    pub fn new(cfg: Config, sampler: S, spool: Spool, mut clock: Box<dyn FnMut() -> f64>) -> Self {
        let seg = Segmenter::new(&cfg, &hostname());
        let last_flush = clock();
        Agent {
            cfg,
            sampler,
            seg,
            spool,
            clock,
            last: None,
            last_flush,
            failing: String::new(),
        }
    }

    pub fn spool(&self) -> &Spool {
        &self.spool
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.cfg
    }

    fn log(&self, msg: &str) {
        log(self.spool.dir(), msg);
    }

    fn store(&self, records: &[crate::segments::Record]) {
        if let Err(e) = self.spool.add(records) {
            self.log(&format!("cannot write the spool: {e}"));
        }
    }

    pub fn step(&mut self) {
        let now = (self.clock)();
        // the machine slept or the loop stalled: what happened in the gap is unknown, so end the segment where it was last seen
        if let Some(last) = self.last
            && now - last > (4 * self.cfg.interval_s).max(30) as f64
        {
            let done = self.seg.finish(last);
            self.store(&done);
        }
        self.last = Some(now);
        match self.sampler.sample() {
            Ok(s) => {
                let done = self.seg.feed(now, &s);
                self.store(&done);
            }
            Err(e) => {
                self.log(&format!("sample failed: {e}"));
                return;
            }
        }
        if now - self.last_flush >= self.cfg.flush_every_s as f64 {
            self.flush();
        }
    }

    pub fn flush(&mut self) -> (usize, String) {
        self.last_flush = (self.clock)();
        let (sent, err) = send_spool(&self.cfg, &self.spool);
        if sent > 0 {
            self.log(&format!("sent {sent} records"));
        }
        if err != self.failing {
            self.log(&if err.is_empty() {
                "send recovered".to_string()
            } else {
                format!("send problem: {err}")
            });
            self.failing = err.clone();
        }
        (sent, err)
    }

    /// Close the running segment and send what is left (the agent is stopping).
    pub fn stop(&mut self) -> (usize, String) {
        let done = self.seg.finish((self.clock)());
        self.store(&done);
        self.flush()
    }
}
