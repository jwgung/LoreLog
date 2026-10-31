//! Copies to go back to (#16). The vault lives in one place (docs/adr/0002), so that place's disk must not be the only
//! copy. Once a day, with the hourly sync, the export (#11: records, files, notes, a list of hashes; not the index) is
//! sealed with the user's password (age, passphrase) into a folder they chose: an external disk, a NAS, a synced
//! folder. The seal covers the names too (mail notes are named by their subjects). Each copy is opened again and
//! checked against its hashes before it counts; 7 days and 4 weeks are kept. The same on the home server and in the
//! desktop app.
//!
//! The folder lives in sources.toml (`[backup] dir`), the password in env (`BACKUP_PASSWORD`, mode 600): the copy is
//! made unattended, and whoever holds the folder (the NAS, a cloud) cannot read it. Open one with `life restore` or
//! the `age` tool (`age -d`).
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use age::secrecy::SecretString;
use chrono::{DateTime, Datelike, Local, NaiveDate};
use regex::Regex;
use serde_json::{Value, json};

use crate::collect::settings;
use crate::config::Paths;

/// scrypt work factor of the seal (about a second to open on a desktop)
pub const WORK: u8 = 18;
/// for tests: fast, and nothing else
pub const TEST_WORK: u8 = 2;
/// a new copy once this long after the last good one
pub const EVERY_HOURS: i64 = 20;
pub const KEEP_DAYS: usize = 7;
pub const KEEP_WEEKS: usize = 4;
pub const MIN_PASSWORD: usize = 8;

static NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^lorelog-backup-(\d{4}-\d{2}-\d{2})-(\d{4})\.zip\.age$").unwrap());

/// One copy made.
#[derive(Debug)]
pub struct Done {
    pub file: PathBuf,
    pub files: usize,
    pub bytes: u64,
    /// copies taken away by the keeping rule
    pub removed: usize,
}

/// The folder and password, when set up.
pub fn configured(paths: &Paths) -> Option<(PathBuf, String)> {
    let dir = settings::read_sources(paths)
        .get("backup")
        .and_then(|t| t.get("dir"))
        .and_then(toml::Value::as_str)
        .filter(|d| !d.trim().is_empty())
        .map(PathBuf::from)?;
    let pw = crate::config::read_env_file(&settings::env_file(paths)).get("BACKUP_PASSWORD").cloned()?;
    (!pw.is_empty()).then_some((dir, pw))
}

/// Where the copies go (without the password), for the screens.
pub fn folder(paths: &Paths) -> Option<PathBuf> {
    configured(paths).map(|(d, _)| d)
}

fn inside(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a.starts_with(b),
        _ => false,
    }
}

/// Set where the copies go and the password. The folder must be there and outside the vault (a copy inside the
/// vault would go into the next copy, and dies with the same disk).
pub fn set(paths: &Paths, dir: &Path, password: &str) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("{}: 없는 폴더입니다", dir.display()));
    }
    if inside(dir, &paths.data) || inside(dir, &paths.vault) || inside(dir, &paths.conf) {
        return Err("백업은 금고 밖의 폴더(외장 디스크, NAS 등)에 둡니다".into());
    }
    if password.chars().count() < MIN_PASSWORD {
        return Err(format!("비밀번호는 {MIN_PASSWORD}자 이상입니다"));
    }
    let dir = dir.canonicalize().map_err(|e| format!("{}: {:?}", dir.display(), e.kind()))?;
    crate::config::set_env_keys(&settings::env_file(paths), &[("BACKUP_PASSWORD", Some(password))])?;
    let mut src = settings::read_sources(paths);
    let mut t = toml::Table::new();
    t.insert("dir".into(), toml::Value::String(dir.to_string_lossy().into_owned()));
    src.insert("backup".into(), toml::Value::Table(t));
    crate::collect::sources::write_sources(paths, &src).map_err(|e| format!("설정을 쓰지 못했습니다: {:?}", e.kind()))
}

fn status_file(paths: &Paths) -> PathBuf {
    paths.data.join("backup.json")
}

/// What the screens show: `{folder, last_run, last_ok, file, error}` (no password).
pub fn status(paths: &Paths) -> Value {
    let mut v: Value = fs::read_to_string(status_file(paths))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    v["folder"] = json!(folder(paths).map(|d| d.to_string_lossy().into_owned()));
    v
}

fn last_ok(paths: &Paths) -> Option<DateTime<chrono::FixedOffset>> {
    let v: Value = serde_json::from_str(&fs::read_to_string(status_file(paths)).ok()?).ok()?;
    DateTime::parse_from_rfc3339(v["last_ok"].as_str()?).ok()
}

/// Is a copy due at `now`: set up, and no good copy in the last [`EVERY_HOURS`] hours?
pub fn due(paths: &Paths, now: DateTime<Local>) -> bool {
    configured(paths).is_some() && last_ok(paths).is_none_or(|t| (now.fixed_offset() - t).num_hours() >= EVERY_HOURS)
}

/// Make a copy now.
pub fn run(paths: &Paths) -> anyhow::Result<Done> {
    run_at(paths, Local::now(), WORK)
}

/// Make a copy as of `now`, sealed with this scrypt work factor.
pub fn run_at(paths: &Paths, now: DateTime<Local>, work: u8) -> anyhow::Result<Done> {
    let (dir, pw) = configured(paths).ok_or_else(|| anyhow::anyhow!("backup is not set up (life backup set DIR)"))?;
    let tmp = paths.data.join("backup-tmp");
    crate::fsutil::ensure_private_dir(&tmp)?;
    let plain = tmp.join("export.zip");
    let check = tmp.join("check.zip");
    let name = format!("lorelog-backup-{}.zip.age", now.format("%Y-%m-%d-%H%M"));
    let out = dir.join(&name);
    let part = dir.join(format!("{name}.part"));
    let result = (|| {
        let report = crate::export::export(paths, &plain)?;
        seal(&plain, &part, &pw, work)?;
        fs::remove_file(&plain)?;
        fs::rename(&part, &out)?;
        // it counts once it opens and every file matches its hash
        let checked = restore(&out, &pw, &check)?;
        fs::remove_file(&check)?;
        if checked != report.files {
            anyhow::bail!("the copy holds {checked} files, the vault {}", report.files);
        }
        let removed = prune(&dir)?;
        anyhow::Ok(Done { file: out.clone(), files: report.files, bytes: report.bytes, removed })
    })();
    for f in [&plain, &check, &part] {
        let _ = fs::remove_file(f);
    }
    let stamp = now.format("%Y-%m-%dT%H:%M:%S%:z").to_string();
    let mut status: Value = fs::read_to_string(status_file(paths))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    status["last_run"] = json!(stamp);
    match &result {
        Ok(d) => {
            status["last_ok"] = json!(stamp);
            status["file"] = json!(d.file.to_string_lossy());
            status["error"] = Value::Null;
        }
        Err(e) => {
            let _ = fs::remove_file(&out); // a copy that did not check out is not a copy
            status["error"] = json!(e.to_string().chars().take(300).collect::<String>());
        }
    }
    crate::fsutil::write_private_atomic(&status_file(paths), status.to_string().as_bytes())?;
    result
}

fn seal(plain: &Path, to: &Path, pw: &str, work: u8) -> anyhow::Result<()> {
    let mut recipient = age::scrypt::Recipient::new(SecretString::from(pw.to_string()));
    recipient.set_work_factor(work);
    let enc = age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))?;
    let file = File::create(to)?;
    let mut w = enc.wrap_output(BufWriter::new(file))?;
    std::io::copy(&mut BufReader::new(File::open(plain)?), &mut w)?;
    w.finish()?.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    Ok(())
}

/// Open a copy with the password into `out` (a zip) and check it against its hashes. -> files. A wrong password or a
/// damaged copy leaves nothing at `out`.
pub fn restore(file: &Path, password: &str, out: &Path) -> anyhow::Result<usize> {
    let result = (|| {
        let dec = age::Decryptor::new_buffered(BufReader::new(File::open(file)?))?;
        let mut identity = age::scrypt::Identity::new(SecretString::from(password.to_string()));
        identity.set_max_work_factor(22);
        let mut r = dec.decrypt(std::iter::once(&identity as &dyn age::Identity))?;
        let mut w = File::create(out)?;
        crate::fsutil::set_mode(out, 0o600);
        std::io::copy(&mut r, &mut w)?;
        w.flush()?;
        crate::export::verify(out)
    })();
    if result.is_err() {
        let _ = fs::remove_file(out);
    }
    result
}

/// Of these file names, the copies to keep: the newest of each of the last [`KEEP_DAYS`] days that have one, and the
/// newest of each of the [`KEEP_WEEKS`] weeks before them. Other names are not copies and are left alone.
pub fn kept(names: &[String]) -> Vec<String> {
    let mut by_day: BTreeMap<NaiveDate, &String> = BTreeMap::new();
    for n in names {
        if let Some(c) = NAME_RE.captures(n)
            && let Ok(d) = NaiveDate::parse_from_str(&c[1], "%Y-%m-%d")
        {
            let e = by_day.entry(d).or_insert(n);
            if n > *e {
                *e = n;
            }
        }
    }
    let days: Vec<(NaiveDate, &String)> = by_day.into_iter().rev().collect();
    let mut keep: Vec<String> = days.iter().take(KEEP_DAYS).map(|(_, n)| (*n).clone()).collect();
    let mut weeks = Vec::new();
    for (d, n) in days.iter().skip(KEEP_DAYS) {
        let w = d.iso_week();
        if !weeks.contains(&w) {
            if weeks.len() == KEEP_WEEKS {
                break;
            }
            weeks.push(w);
            keep.push((*n).clone());
        }
    }
    keep
}

/// Take away the copies the keeping rule does not keep. -> how many
fn prune(dir: &Path) -> anyhow::Result<usize> {
    let names: Vec<String> =
        fs::read_dir(dir)?.flatten().filter_map(|e| e.file_name().to_str().map(str::to_string)).collect();
    let keep = kept(&names);
    let mut removed = 0;
    for n in names.iter().filter(|n| NAME_RE.is_match(n) && !keep.contains(n)) {
        fs::remove_file(dir.join(n))?;
        removed += 1;
    }
    Ok(removed)
}
