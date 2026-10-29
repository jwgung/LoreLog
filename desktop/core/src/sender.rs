//! Sends the spooled lines to the server. A line leaves the spool only after the server accepted its batch.
use crate::config::Config;
use crate::intake::{Uploaded, UploadError, Uploader};
use crate::spool::Spool;
use std::time::Duration;

const BATCH: usize = 500;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .max_redirects(0) // this API never redirects; do not forward the Authorization header anywhere
        .build()
        .into()
}

/// The server's file endpoint, used by the watched-folder intake. The address and token come from the config.
pub struct ServerUploader {
    agent: ureq::Agent,
    base: String,
    token: String,
}

impl ServerUploader {
    pub fn new(cfg: &Config) -> Self {
        ServerUploader {
            agent: agent(),
            base: cfg.url.trim_end_matches('/').to_string(),
            token: cfg.token.clone(),
        }
    }

    fn name_header(name: &str) -> String {
        // headers are latin-1; percent-encode everything outside unreserved ASCII so non-ASCII names survive
        let mut out = String::new();
        for b in name.as_bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }

    /// Classify a non-success status: a 4xx (except 401) is the server refusing this file for good; a 401 or 5xx is
    /// transient (wrong token now, or server trouble) and worth retrying.
    fn status_error(code: u16) -> UploadError {
        if (400..500).contains(&code) && code != 401 {
            UploadError::Rejected(format!("server refused the file ({code})"))
        } else if code == 401 {
            UploadError::Transient("the server refused the token".into())
        } else {
            UploadError::Transient(format!("server answered {code}"))
        }
    }

    /// The common shape of a file request: set the bearer token and the file headers, send the body, map the result.
    fn file_request(&self, extra: &[(&str, &str)], sha: &str, name: &str, mime: &str, when_ms: i64, body: &[u8]) -> Result<bool, UploadError> {
        let url = format!("{}/ingest/file", self.base);
        let mut req = self
            .agent
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("X-File-Sha256", sha)
            .header("X-File-Name", Self::name_header(name))
            .header("X-File-Mime", mime)
            .header("X-File-Ts", when_ms.to_string());
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let res = req
            .send(body)
            .map_err(|e| UploadError::Transient(format!("cannot reach the server ({e})")))?;
        let code = res.status().as_u16();
        if !res.status().is_success() {
            return Err(Self::status_error(code));
        }
        let text = res
            .into_body()
            .read_to_string()
            .map_err(|e| UploadError::Transient(format!("bad reply ({e})")))?;
        let parsed: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| UploadError::Transient(format!("bad reply ({e})")))?;
        Ok(parsed.get("duplicate").and_then(|v| v.as_bool()).unwrap_or(false))
    }
}

impl Uploader for ServerUploader {
    fn has(&self, sha: &str) -> Result<bool, String> {
        let url = format!("{}/ingest/file/{sha}", self.base);
        let res = self
            .agent
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .call()
            .map_err(|e| format!("cannot reach the server ({e})"))?;
        if !res.status().is_success() {
            return Err(format!("server answered {}", res.status().as_u16()));
        }
        let text = res
            .into_body()
            .read_to_string()
            .map_err(|e| format!("bad reply ({e})"))?;
        let body: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("bad reply ({e})"))?;
        Ok(body.get("present").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    fn put(&self, sha: &str, name: &str, mime: &str, data: &[u8], when_ms: i64) -> Result<Uploaded, UploadError> {
        let extra = [("Content-Type", "application/octet-stream")];
        let duplicate = self.file_request(&extra, sha, name, mime, when_ms, data)?;
        Ok(Uploaded { duplicate })
    }

    fn sight(&self, sha: &str, name: &str, mime: &str, when_ms: i64) -> Result<(), UploadError> {
        let extra = [("X-File-Sighting", "1")];
        self.file_request(&extra, sha, name, mime, when_ms, b"")?;
        Ok(())
    }
}

/// -> (lines delivered, error text or ""). Stops at the first failure and keeps the rest.
pub fn send_spool(cfg: &Config, spool: &Spool) -> (usize, String) {
    if cfg.url.is_empty() || cfg.token.is_empty() {
        return (0, "url and token are not set in the config file".into());
    }
    let agent = agent();
    let url = format!("{}/ingest/{}", cfg.url.trim_end_matches('/'), cfg.source);
    let mut sent = 0;
    loop {
        let lines: Vec<String> = spool.read().into_iter().take(BATCH).collect();
        if lines.is_empty() {
            return (sent, String::new());
        }
        let body = lines.join("\n") + "\n";
        let res = agent
            .post(&url)
            .header("Authorization", format!("Bearer {}", cfg.token))
            .header("Content-Type", "application/x-ndjson")
            .send(body.as_bytes());
        match res {
            Ok(r) if r.status().is_success() => {}
            Ok(r) if r.status().as_u16() == 401 => {
                return (sent, "the server refused the token".into());
            }
            Ok(r) => return (sent, format!("server answered {}", r.status().as_u16())),
            Err(e) => return (sent, format!("cannot reach the server ({e})")),
        }
        if let Err(e) = spool.drop_first(lines.len()) {
            return (sent, format!("cannot update the spool ({e})"));
        }
        sent += lines.len();
    }
}
