//! Keep a PC deliverable's bytes as they are, and record each sighting: the file counterpart of the record store.
//!
//! The PC agent watches a folder the user chose and sends each file here. Like mail-files, the bytes go under their own
//! content hash (so the same file seen through mail, a window title or a print job is one blob) and a line records
//! where and when it was seen. Nothing is converted, summarised or tagged.
//!
//!   `<data>/raw/files/<sha256[:2]>/<sha256>`   the bytes, untouched (shared with mail-files)
//!   `<data>/raw/pcfile/<YYYY-MM-DD>.ndjson`    one line per sighting (raw store format)
use std::fs;
use std::io;

use serde_json::json;

use crate::store::{RawStore, blob_path, put_blob, sha256_hex};

/// A sha must be 64 lowercase hex before it is used in a file path (no empty value, no path segments).
pub fn valid_sha(sha: &str) -> bool {
    sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Is this content hash already kept? Lets the agent skip uploading bytes we have.
pub fn has_blob(store: &RawStore, sha: &str) -> bool {
    valid_sha(sha) && fs::metadata(blob_path(store.data(), sha)).map(|m| m.len() > 0).unwrap_or(false)
}

#[derive(Debug)]
pub enum FileError {
    /// the body does not hash to the claimed sha (a corrupt or mislabelled upload)
    HashMismatch,
    /// a sighting of bytes that are not kept (or a malformed sha)
    NotKept,
    Io(io::Error),
}

impl From<io::Error> for FileError {
    fn from(e: io::Error) -> Self {
        FileError::Io(e)
    }
}

/// What a file upload says about the file.
pub struct FileMeta<'a> {
    pub name: &'a str,
    pub mime: &'a str,
    pub when_ms: i64,
    pub cls: &'a str,
}

/// Store one PC file: verify the claimed hash (if one is given), keep the bytes once, record a sighting.
/// -> (sha, newly stored)
pub fn keep_file(store: &RawStore, data: &[u8], claimed: &str, meta: &FileMeta) -> Result<(String, bool), FileError> {
    let actual = sha256_hex(data);
    if !claimed.is_empty() && claimed != actual {
        return Err(FileError::HashMismatch);
    }
    let (sha, new) = put_blob(store.data(), data)?;
    record(store, &sha, data.len() as u64, meta)?;
    Ok((sha, new))
}

/// Record that an already-kept file was seen, without the bytes. The size comes from the kept blob, not the caller.
pub fn record_sighting(store: &RawStore, sha: &str, meta: &FileMeta) -> Result<(), FileError> {
    if !has_blob(store, sha) {
        return Err(FileError::NotKept);
    }
    let size = fs::metadata(blob_path(store.data(), sha))?.len();
    record(store, sha, size, meta)
}

fn record(store: &RawStore, sha: &str, size: u64, meta: &FileMeta) -> Result<(), FileError> {
    let rec = json!({"type": "file", "ts": meta.when_ms, "sha256": sha, "name": meta.name, "mime": meta.mime,
                     "size": size, "source": "pc"});
    store.store("pcfile", &format!("{rec}\n"), meta.cls)?;
    Ok(())
}
