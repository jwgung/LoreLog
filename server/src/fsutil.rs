//! Private files: everything the server writes is the user's own record, so directories are 0700 and files 0600 where the
//! OS has such modes (on Windows the folder ACL decides).
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// Make the directory (and its parents) and set it to 0700. A mode that cannot be set is left to the OS.
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    set_mode(dir, 0o700);
    Ok(())
}

/// Set a Unix mode, quietly doing nothing where there are no such modes.
pub fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
}

/// Open for appending; a new file is created 0600 (it never exists with a wider mode, even for a moment).
pub fn open_append_private(path: &Path) -> io::Result<File> {
    let mut o = OpenOptions::new();
    o.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

/// Append text to a private file.
pub fn append_private(path: &Path, text: &str) -> io::Result<()> {
    let mut f = open_append_private(path)?;
    f.write_all(text.as_bytes())?;
    set_mode(path, 0o600);
    Ok(())
}

/// Replace a file's content all at once: written to a unique temp file next to it (0600), then renamed over it, so a
/// reader sees the old content or the new, never half of it. The directory is created 0700 if missing.
pub fn write_private_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mut tmp = tempfile_in(dir, path.file_name().and_then(|n| n.to_str()).unwrap_or("file"))?;
    let result = (|| {
        tmp.1.write_all(data)?;
        tmp.1.sync_all()?;
        set_mode(&tmp.0, 0o600);
        fs::rename(&tmp.0, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp.0);
    }
    result
}

/// A new file with a unique name in `dir` (`<prefix>.<random>.tmp`), created exclusively.
fn tempfile_in(dir: &Path, prefix: &str) -> io::Result<(std::path::PathBuf, File)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    for _ in 0..100 {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let name = format!("{prefix}.{}-{nanos}-{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed));
        let p = dir.join(name);
        let mut o = OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        match o.open(&p) {
            Ok(f) => return Ok((p, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free temp name"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(p: &Path) -> u32 {
        fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn written_files_and_dirs_are_private() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("a/b");
        ensure_private_dir(&d).unwrap();
        assert_eq!(mode(&d), 0o700);
        let f = d.join("x.ndjson");
        append_private(&f, "1\n").unwrap();
        append_private(&f, "2\n").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "1\n2\n");
        assert_eq!(mode(&f), 0o600);
        let g = d.join("y.json");
        write_private_atomic(&g, b"old").unwrap();
        write_private_atomic(&g, b"new").unwrap();
        assert_eq!(fs::read(&g).unwrap(), b"new");
        assert_eq!(mode(&g), 0o600);
        assert_eq!(fs::read_dir(&d).unwrap().count(), 2, "no temp file is left behind");
    }
}
