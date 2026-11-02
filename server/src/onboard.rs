//! What a first run needs (#69): checking that the model address the user typed answers, the one-line connection code
//! a phone pastes (the vault's address and token together), and how far the first sync and graph got.
//!
//! The code holds the token: it is made only for an address the token may go to ([`safe_for_token`], the phone's rule
//! for manual entry too), and nothing here logs it.
use std::net::IpAddr;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;

use crate::config::Paths;
use crate::host::{MIN_TOKEN, is_private_lan_ip, is_tailnet_ip};
use crate::llm::{Ask, Http, Model};

/// What every connection code starts with.
pub const CODE_PREFIX: &str = "lorelog://connect?";

/// How long the check waits: a small model on a CPU may first have to load (Ollama loads on the first request).
const CHECK_TIMEOUT_S: u64 = 120;

/// May the token be sent to this address? Over https anywhere, or over plain http only inside the tailnet
/// (`*.ts.net`, 100.64.0.0/10, fd7a:115c:a1e0::/48, encrypted by WireGuard), to this device (loopback), or to a
/// private IPv4 address in the same network (10/8, 172.16/12, 192.168/16: not encrypted, the user's choice for a
/// trusted Wi-Fi). An IPv4 address counts only written plainly (four decimal parts, no leading zeros): `012.0.0.1`
/// or `10.1` mean different addresses to different readers.
pub fn safe_for_token(u: &str) -> bool {
    let u = u.trim();
    let Ok(parsed) = url::Url::parse(u) else { return false };
    let inside = match parsed.host() {
        Some(url::Host::Domain(d)) => {
            let d = d.strip_suffix('.').unwrap_or(d);
            d.ends_with(".ts.net") || d == "localhost"
        }
        Some(url::Host::Ipv4(ip)) => {
            let ip4 = IpAddr::V4(ip);
            written_host(u) == ip.to_string() && (ip.is_loopback() || is_tailnet_ip(ip4) || is_private_lan_ip(ip4))
        }
        Some(url::Host::Ipv6(ip)) => ip.is_loopback() || is_tailnet_ip(IpAddr::V6(ip)),
        None => return false,
    };
    match parsed.scheme() {
        "https" => true,
        "http" => inside,
        _ => false,
    }
}

/// The host as written in `u` (no userinfo, port or trailing dot), to tell `10.0.0.1` from `012.0.0.1`, which the
/// URL parser reads as the same address.
fn written_host(u: &str) -> &str {
    let rest = u.split_once("://").map_or(u, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host_port.split(':').next().unwrap_or("");
    host.strip_suffix('.').unwrap_or(host)
}

const NOT_SAFE: &str = "이 주소로는 토큰을 보내지 않습니다. https 주소이거나 tailnet(*.ts.net, 100.x) 또는 같은 와이파이(192.168.x, 10.x, 172.16~31.x) 주소여야 합니다";

fn token_ok(t: &str) -> bool {
    t.chars().count() >= MIN_TOKEN && !t.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The vault's address and token as one line, `lorelog://connect?u=<address>&t=<token>`.
pub fn connect_code(url: &str, token: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    if !safe_for_token(url) {
        return Err(NOT_SAFE.into());
    }
    if !token_ok(token) {
        return Err(format!("토큰이 {MIN_TOKEN}자보다 짧거나 공백이 있습니다"));
    }
    let q = url::form_urlencoded::Serializer::new(String::new()).append_pair("u", url).append_pair("t", token).finish();
    Ok(format!("{CODE_PREFIX}{q}"))
}

/// The address and token in a connection code, checked as manual entry is. Err: why it cannot be used.
pub fn parse_connect_code(code: &str) -> Result<(String, String), String> {
    let Some(q) = code.trim().strip_prefix(CODE_PREFIX) else {
        return Err("LoreLog 연결 코드가 아닙니다(lorelog://connect? 로 시작합니다)".into());
    };
    let (mut url, mut token) = (None, None);
    for (k, v) in url::form_urlencoded::parse(q.as_bytes()) {
        match k.as_ref() {
            "u" => url = Some(v.trim().trim_end_matches('/').to_string()),
            "t" => token = Some(v.into_owned()),
            _ => {}
        }
    }
    let (Some(url), Some(token)) = (url.filter(|u| !u.is_empty()), token) else {
        return Err("연결 코드에 주소나 토큰이 빠졌습니다. 다시 복사해 주세요".into());
    };
    if !safe_for_token(&url) {
        return Err(NOT_SAFE.into());
    }
    if !token_ok(&token) {
        return Err("연결 코드의 토큰이 올바르지 않습니다. 다시 복사해 주세요".into());
    }
    Ok((url, token))
}

/// A model that answered the check.
#[derive(Debug, Serialize)]
pub struct ModelCheck {
    /// its answer, shortened
    pub reply: String,
    pub seconds: f64,
}

/// Ask the model at `url` (OpenAI-compatible, e.g. `http://127.0.0.1:11434/v1`) for a few words. Err: what to look at.
pub fn check_model(url: &str, model: &str) -> Result<ModelCheck, String> {
    let (url, model) = (url.trim(), model.trim());
    if url.is_empty() {
        return Err("모델 주소를 넣어 주세요".into());
    }
    if !matches!(url::Url::parse(url).map(|p| p.scheme().to_string()).as_deref(), Ok("http" | "https")) {
        return Err("모델 주소는 http:// 또는 https:// 로 시작해야 합니다".into());
    }
    if model.is_empty() {
        return Err("모델 이름을 넣어 주세요(예: qwen3:8b)".into());
    }
    let http = Http { url: url.to_string(), model: model.to_string() };
    let messages = [json!({"role": "user", "content": "Reply with the single word OK."})];
    let t = Instant::now();
    match http.complete(&messages, Ask::new(0.0, 16, false, CHECK_TIMEOUT_S)) {
        Ok(reply) => Ok(ModelCheck { reply: reply.trim().chars().take(40).collect(), seconds: secs(t.elapsed()) }),
        Err(e) => Err(why(&e, model)),
    }
}

fn secs(d: Duration) -> f64 {
    (d.as_secs_f64() * 10.0).round() / 10.0
}

fn why(e: &anyhow::Error, model: &str) -> String {
    let not_openai = "답은 왔지만 OpenAI 호환 대화 응답이 아닙니다. 주소가 /v1 로 끝나는지 보세요".to_string();
    let Some(e) = e.downcast_ref::<ureq::Error>() else { return not_openai };
    match e {
        ureq::Error::StatusCode(404) => format!(
            "주소는 닿았지만 찾지 못했습니다(404). 모델 이름 '{model}'이 그 서버에 있는지(ollama pull), 주소가 /v1 로 끝나는지 보세요"
        ),
        ureq::Error::StatusCode(c @ (401 | 403)) => {
            format!(
                "모델 서버가 인증을 요구합니다({c}). 이 앱은 API 키를 보내지 않으니 키 없이 받는 주소를 넣어 주세요"
            )
        }
        ureq::Error::StatusCode(c) => format!("모델 서버가 오류({c})로 답했습니다. 모델 이름과 서버 로그를 보세요"),
        ureq::Error::Timeout(_) => format!(
            "{CHECK_TIMEOUT_S}초 안에 답이 없습니다. 모델을 처음 올리는 중이면 잠시 뒤 다시 확인해 보세요. 계속 느리면 더 작은 모델이나 GPU가 있는 서버를 쓰세요"
        ),
        ureq::Error::Json(_) => not_openai,
        ureq::Error::Io(_) | ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => {
            "모델 주소에 닿지 못했습니다. 모델 서버(Ollama, LM Studio 등)가 켜져 있는지, 주소와 포트가 맞는지 보세요"
                .into()
        }
        other => format!("모델 주소에 닿지 못했습니다: {other}"),
    }
}

/// How far the first sync and the graph got.
#[derive(Debug, Default, Serialize)]
pub struct Progress {
    /// records in the librarian's index (mail, events, phone records, ...)
    pub records: usize,
    /// mails among them: what the graph reads
    pub mail: usize,
    /// mails the model has read for the graph
    pub graph_read: usize,
    /// mails still waiting for it
    pub graph_waiting: usize,
}

/// Counted from the librarian's index, without making one (a new vault has none yet: all zero).
pub fn progress(paths: &Paths) -> Progress {
    let file = crate::librarian::index::default_path(paths);
    if !file.exists() {
        return Progress::default();
    }
    crate::librarian::Librarian::open(paths, Some(file), Duration::from_secs(2))
        .and_then(|lib| lib.progress())
        .unwrap_or_default()
}
