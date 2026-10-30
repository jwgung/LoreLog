//! The IMAP side of the mail collectors: always read-only (folders are EXAMINEd and bodies read with BODY.PEEK, so
//! nothing on the server changes, read flags included). Errors carry only their kind, never what was sent.
use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};

pub type Session = imap::Session<imap::Connection>;

/// Log in. The error says only what kind of failure it was, so the password cannot leak into a message.
pub fn connect(host: &str, port: u16, user: &str, password: &str) -> Result<Session, String> {
    let client = imap::ClientBuilder::new(host, port).connect().map_err(|e| kind(&e))?;
    client.login(user, password).map_err(|(e, _)| kind(&e))
}

pub fn kind(e: &imap::Error) -> String {
    format!("{e:?}").split(['(', ' ', '{']).next().unwrap_or("error").to_string()
}

/// A folder opened read-only: (UIDVALIDITY, every UID, ascending). `since`: only mail from that day (YYYY-MM-DD).
pub fn open(s: &mut Session, folder: &str, since: Option<&str>) -> Result<(String, Vec<u32>), String> {
    let mb = s.examine(folder).map_err(|e| kind(&e))?;
    let crit = match since.and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) {
        Some(d) => format!("SINCE {}", d.format("%d-%b-%Y")),
        None => "ALL".into(),
    };
    let mut uids: Vec<u32> = s.uid_search(crit).map_err(|e| kind(&e))?.into_iter().collect();
    uids.sort_unstable();
    Ok((mb.uid_validity.map(|v| v.to_string()).unwrap_or_default(), uids))
}

/// The headers of these mails: uid -> header bytes.
pub fn headers(s: &mut Session, uids: &[u32]) -> Result<HashMap<u32, Vec<u8>>, String> {
    let set: Vec<String> = uids.iter().map(u32::to_string).collect();
    let fetches = s.uid_fetch(set.join(","), "(UID BODY.PEEK[HEADER])").map_err(|e| kind(&e))?;
    Ok(fetches.iter().filter_map(|f| Some((f.uid?, f.header()?.to_vec()))).collect())
}

/// The whole mail.
pub fn body(s: &mut Session, uid: u32) -> Result<Option<Vec<u8>>, String> {
    let fetches = s.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])").map_err(|e| kind(&e))?;
    Ok(fetches.iter().find_map(|f| f.body().map(<[u8]>::to_vec)))
}

/// (size, content type header) of these mails, for a dry run.
pub fn sizes(s: &mut Session, uids: &[u32]) -> Result<Vec<(u32, String)>, String> {
    let set: Vec<String> = uids.iter().map(u32::to_string).collect();
    if set.is_empty() {
        return Ok(Vec::new());
    }
    let fetches = s.uid_fetch(set.join(","), "(UID RFC822.SIZE BODY.PEEK[HEADER])").map_err(|e| kind(&e))?;
    Ok(fetches
        .iter()
        .map(|f| (f.size.unwrap_or(0), String::from_utf8_lossy(f.header().unwrap_or(&[])).into_owned()))
        .collect())
}

/// Per-folder state between runs: `{folder: {uidvalidity, seen: [uids]}}`; a changed UIDVALIDITY starts over.
pub struct State(pub BTreeMap<String, Value>);

impl State {
    pub fn load(path: &std::path::Path) -> State {
        let m = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Map<String, Value>>(&t).ok())
            .map(|m| m.into_iter().collect())
            .unwrap_or_default();
        State(m)
    }

    pub fn seen(&self, folder: &str, uidvalidity: &str) -> std::collections::BTreeSet<u32> {
        match self.0.get(folder) {
            Some(st) if st.get("uidvalidity").and_then(Value::as_str) == Some(uidvalidity) => st["seen"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_u64().map(|u| u as u32)).collect())
                .unwrap_or_default(),
            _ => Default::default(),
        }
    }

    pub fn set(&mut self, folder: &str, uidvalidity: &str, seen: &std::collections::BTreeSet<u32>) {
        self.0.insert(folder.to_string(), json!({"uidvalidity": uidvalidity, "seen": seen}));
    }

    /// Folder names and UIDs are private mail metadata: written 0600, all at once.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        crate::fsutil::write_private_atomic(path, serde_json::to_string(&self.0).expect("JSON serializes").as_bytes())
    }
}
