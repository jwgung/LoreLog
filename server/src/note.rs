//! Vault notes as the collectors write them: a `---` frontmatter of `key: value` lines, then the body. Every reader of
//! notes (assistant, librarian, collectors) goes through here, each choosing how much of the note to look at and how
//! to unquote values.
use std::fs;
use std::io;
use std::path::Path;

/// The whole note as text; bytes that are not UTF-8 become U+FFFD instead of failing the read.
pub fn read(path: &Path) -> io::Result<String> {
    Ok(String::from_utf8_lossy(&fs::read(path)?).into_owned())
}

/// The first `chars` characters of a note (enough for its frontmatter), or "" when it cannot be read.
pub fn head(path: &Path, chars: usize) -> String {
    read(path).map(|t| t.chars().take(chars).collect()).unwrap_or_default()
}

/// The frontmatter (with its trailing newline) and the body. A note without one is all body.
pub fn split(text: &str) -> (&str, &str) {
    if let Some(rest) = text.strip_prefix("---\n")
        && let Some(i) = rest.find("\n---")
    {
        return (&rest[..=i], &rest[i + 4..]);
    }
    ("", text)
}

/// The value of the first `key:` line, trimmed but otherwise as written; `None` when no line has the key.
pub fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.split('\n').find_map(|l| l.strip_prefix(key)?.strip_prefix(':').map(str::trim))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_trimmed_and_left_quoted() {
        let t = "---\ntitle: \"회의\"\r\nstart:\t2026-01-01T10:00:00\nempty:\nsubject_x: no\n---\nbody title: x\n";
        assert_eq!(field(t, "title"), Some("\"회의\""));
        assert_eq!(field(t, "start"), Some("2026-01-01T10:00:00"));
        assert_eq!(field(t, "empty"), Some(""));
        assert_eq!(field(t, "subject"), None, "a longer key is not the key");
        assert_eq!(field(t, "missing"), None);
    }

    #[test]
    fn split_separates_frontmatter_from_body() {
        assert_eq!(split("---\na: 1\n---\nbody\n"), ("a: 1\n", "\nbody\n"));
        assert_eq!(split("no front\n"), ("", "no front\n"));
    }

    #[test]
    fn head_reads_lossily_and_never_fails() {
        let dir = std::env::temp_dir().join(format!("note-head-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("n.md");
        fs::write(&p, b"a: \xff\xfe\nb: 2\n").unwrap();
        assert_eq!(head(&p, 6), "a: \u{fffd}\u{fffd}\n");
        assert_eq!(head(&dir.join("missing.md"), 10), "");
        fs::remove_dir_all(&dir).unwrap();
    }
}
