//! Records waiting to be sent: an append-only NDJSON file, so nothing is lost while the server is unreachable.
use crate::segments::Record;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Beyond this size the oldest half is dropped rather than filling the disk.
const MAX_BYTES: u64 = 20 * 1024 * 1024;

pub struct Spool {
    dir: PathBuf,
}

impl Spool {
    pub fn new(dir: &Path) -> Self {
        Spool {
            dir: dir.to_path_buf(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join("spool.ndjson")
    }

    pub fn add(&self, records: &[Record]) -> std::io::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        fs::create_dir_all(&self.dir)?;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path())?;
        for r in records {
            writeln!(
                f,
                "{}",
                serde_json::to_string(r).map_err(std::io::Error::other)?
            )?;
        }
        drop(f);
        if fs::metadata(self.path())?.len() > MAX_BYTES {
            let lines = self.read();
            let rest = lines[lines.len() / 2..].join("\n") + "\n";
            fs::write(self.path(), rest)?;
        }
        Ok(())
    }

    pub fn read(&self) -> Vec<String> {
        fs::read_to_string(self.path())
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Remove the first `n` lines (they were delivered).
    pub fn drop_first(&self, n: usize) -> std::io::Result<()> {
        let rest: Vec<String> = self.read().into_iter().skip(n).collect();
        if rest.is_empty() {
            match fs::remove_file(self.path()) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        } else {
            let tmp = self.dir.join("spool.tmp");
            fs::write(&tmp, rest.join("\n") + "\n")?;
            fs::rename(tmp, self.path())
        }
    }
}
