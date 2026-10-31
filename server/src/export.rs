//! The vault taken out (#11): one zip of the user's records as they are kept, so they can leave with them.
//!
//!   data/raw/...            every record as it came (NDJSON, one JSON object a line) and the files kept by hash
//!                           (data/raw/files/<sha256[:2]>/<sha256>; their names are in raw/mailfile and raw/pcfile)
//!   data/todos.json         the to-dos
//!   data/librarian-asks.ndjson  the questions kept to measure the librarian (if any)
//!   notes/Mail, Events, Diary   the notes made of them (Markdown)
//!   settings/diary.json     which messenger senders may feed the diary
//!   MANIFEST.sha256         the hash of every file above, as `sha256sum -c` reads it
//!   README.md               this, for whoever opens the zip
//!
//! Left out: what can be made again from these (the librarian's index, caches), the run state, and secrets (the
//! account passwords and the token live in the config folder, which is not taken). Other notes of the vault (the
//! user's own) are theirs already and not taken either.
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::config::Paths;

pub const MANIFEST: &str = "MANIFEST.sha256";
pub const README: &str = "README.md";

const README_TEXT: &str = "# LoreLog vault export

This zip is everything LoreLog keeps of your records, in open formats.

- `data/raw/<source>/<YYYY-MM-DD>.ndjson`: every record as it came, one JSON object a line. Nothing was edited.
- `data/raw/files/<aa>/<sha256>`: files that went through your mail or folders, under their SHA-256. Their names and
  where they were seen are in `data/raw/mailfile/` and `data/raw/pcfile/`.
- `data/todos.json`: your to-dos. `data/librarian-asks.ndjson`: questions kept to measure search (if you allowed it).
- `notes/`: the Markdown notes made of your mail, calendars and diary.
- `settings/diary.json`: which messenger senders may feed the diary.
- `MANIFEST.sha256`: the hash of every file. Check it with `sha256sum -c MANIFEST.sha256` after unzipping.

Not included: the search index and caches (made again from the above), and your passwords and token.

# LoreLog 금고 내보내기

LoreLog가 가진 기록 전부를 열린 형식으로 담았습니다. 위 목록과 같습니다: 원본 기록(NDJSON, 고치지 않음),
해시로 보관한 파일, 할 일, 노트(Markdown), 설정, 그리고 확인용 해시 목록(`sha256sum -c MANIFEST.sha256`).
검색 색인과 캐시(다시 만들 수 있는 것), 비밀번호와 토큰은 넣지 않았습니다.
";

/// The notes folders of the vault that are made of the records (the rest of the vault is the user's own).
const NOTE_DIRS: [&str; 3] = ["Mail", "Events", "Diary"];
/// Files of the data folder that are the user's (beside raw/).
const DATA_FILES: [&str; 2] = ["todos.json", "librarian-asks.ndjson"];

/// How much went out.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub files: usize,
    pub bytes: u64,
}

/// What goes in, as (path on disk, name in the zip), in name order.
fn contents(paths: &Paths) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    walk(&paths.raw(), "data/raw", &mut out);
    for f in DATA_FILES {
        let p = paths.data.join(f);
        if p.is_file() {
            out.push((p, format!("data/{f}")));
        }
    }
    for d in NOTE_DIRS {
        walk(&paths.vault.join(d), &format!("notes/{d}"), &mut out);
    }
    if paths.allow_file.is_file() {
        out.push((paths.allow_file.clone(), "settings/diary.json".into()));
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// Every regular file under `dir` (links are not followed: a link may point anywhere).
fn walk(dir: &Path, name: &str, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let Ok(kind) = e.file_type() else { continue };
        let Some(file) = e.file_name().to_str().map(str::to_string) else { continue }; // a name zip cannot carry as is
        let inner = format!("{name}/{file}");
        if kind.is_dir() {
            walk(&e.path(), &inner, out);
        } else if kind.is_file() {
            out.push((e.path(), inner));
        }
    }
}

/// A file's bytes copied into the zip while their hash is taken.
fn copy_hashed(from: &mut impl Read, to: &mut impl Write) -> std::io::Result<(String, u64)> {
    let mut sha = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut n = 0u64;
    loop {
        let got = from.read(&mut buf)?;
        if got == 0 {
            break;
        }
        sha.update(&buf[..got]);
        to.write_all(&buf[..got])?;
        n += got as u64;
    }
    Ok((hex::encode(sha.finalize()), n))
}

/// Text is packed; the kept files (mostly PDF, Office, images: packed already) are stored as they are.
fn options(name: &str, size: u64) -> zip::write::SimpleFileOptions {
    let method = if name.starts_with("data/raw/files/") {
        zip::CompressionMethod::Stored
    } else {
        zip::CompressionMethod::Deflated
    };
    zip::write::SimpleFileOptions::default()
        .compression_method(method)
        .large_file(size >= u32::MAX as u64)
        .unix_permissions(0o600)
}

/// Write the export to `out` (a `.part` beside it first: a broken run never leaves a zip that looks whole).
pub fn export(paths: &Paths, out: &Path) -> anyhow::Result<Report> {
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let part = out.with_file_name(format!(
        "{}.part",
        out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "export.zip".into())
    ));
    let result = (|| {
        let file = File::create(&part)?;
        crate::fsutil::set_mode(&part, 0o600);
        let mut zip = zip::ZipWriter::new(file);
        let mut manifest = String::new();
        let mut report = Report::default();
        for (path, name) in contents(paths) {
            let Ok(mut f) = File::open(&path) else { continue }; // gone since it was listed
            let size = f.metadata().map(|m| m.len()).unwrap_or(0);
            zip.start_file(&name, options(&name, size))?;
            let (sha, n) = copy_hashed(&mut f, &mut zip)?;
            manifest.push_str(&format!("{sha}  {name}\n"));
            report.files += 1;
            report.bytes += n;
        }
        zip.start_file(MANIFEST, options(MANIFEST, 0))?;
        zip.write_all(manifest.as_bytes())?;
        zip.start_file(README, options(README, 0))?;
        zip.write_all(README_TEXT.as_bytes())?;
        zip.finish()?.sync_all()?;
        anyhow::Ok(report)
    })();
    match result {
        Ok(report) => {
            fs::rename(&part, out)?;
            Ok(report)
        }
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

/// Check an export against its manifest: every file listed is there with its hash, and nothing else is. -> files
pub fn verify(zip_path: &Path) -> anyhow::Result<usize> {
    let mut zip = zip::ZipArchive::new(File::open(zip_path)?)?;
    let listed: Vec<(String, String)> = {
        let m = zip.by_name(MANIFEST).map_err(|_| anyhow::anyhow!("no {MANIFEST} in the export"))?;
        let mut out = Vec::new();
        for line in BufReader::new(m).lines() {
            let line = line?;
            let Some((sha, name)) = line.split_once("  ") else {
                anyhow::bail!("{MANIFEST}: a line that is not `hash  name`")
            };
            out.push((sha.to_string(), name.to_string()));
        }
        out
    };
    for (sha, name) in &listed {
        let mut f = zip.by_name(name).map_err(|_| anyhow::anyhow!("missing: {name}"))?;
        let (got, _) = copy_hashed(&mut f, &mut std::io::sink())?;
        if &got != sha {
            anyhow::bail!("changed: {name}");
        }
    }
    let known: std::collections::HashSet<&str> = listed.iter().map(|(_, n)| n.as_str()).collect();
    for i in 0..zip.len() {
        let name = zip.by_index(i)?.name()?.to_string();
        if name != MANIFEST && name != README && !known.contains(name.as_str()) {
            anyhow::bail!("not in the manifest: {name}");
        }
    }
    Ok(listed.len())
}
