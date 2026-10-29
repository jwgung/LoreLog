//! Sends the spooled lines to the server. A line leaves the spool only after the server accepted its batch.
use crate::config::Config;
use crate::spool::Spool;
use std::time::Duration;

const BATCH: usize = 500;

/// -> (lines delivered, error text or ""). Stops at the first failure and keeps the rest.
pub fn send_spool(cfg: &Config, spool: &Spool) -> (usize, String) {
    if cfg.url.is_empty() || cfg.token.is_empty() {
        return (0, "url and token are not set in the config file".into());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into();
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
