//! The one place that talks to the language model (OpenAI-compatible chat completions) and the speech model.
//!
//! Everything that asks the model takes a `&dyn Model`, so tests answer with a fake and the desktop app can point at
//! another server.
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde_json::{Value, json};

/// How to ask: the knobs the callers set per request.
#[derive(Clone, Copy, Debug)]
pub struct Ask {
    pub temperature: f64,
    pub max_tokens: u32,
    pub json_mode: bool,
    pub timeout: Duration,
}

impl Ask {
    pub fn new(temperature: f64, max_tokens: u32, json_mode: bool, timeout_s: u64) -> Ask {
        Ask {
            temperature,
            max_tokens,
            json_mode,
            timeout: Duration::from_secs(timeout_s),
        }
    }
}

pub trait Model: Send + Sync {
    /// Send a chat (`[{role, content}]`) and return the model's text.
    fn complete(&self, messages: &[Value], ask: Ask) -> anyhow::Result<String>;
}

/// The local model server.
pub struct Http {
    pub url: String,
    pub model: String,
}

impl Http {
    /// From `LIFE_LLM_URL` and `LIFE_LLM_MODEL`, with the home server's defaults.
    pub fn from_env() -> Http {
        let get = |k: &str, d: &str| {
            std::env::var(k)
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| d.to_string())
        };
        Http {
            url: get("LIFE_LLM_URL", "http://10.100.3.232:8000/v1"),
            model: get("LIFE_LLM_MODEL", "ultivis-vl"),
        }
    }
}

impl Model for Http {
    /// thinking is off: the answers are short and the wait matters.
    fn complete(&self, messages: &[Value], ask: Ask) -> anyhow::Result<String> {
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": ask.temperature,
            "max_tokens": ask.max_tokens,
            "chat_template_kwargs": {"enable_thinking": false},
        });
        if ask.json_mode {
            body["response_format"] = json!({"type": "json_object"});
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(ask.timeout))
            .build()
            .into();
        let url = format!("{}/chat/completions", self.url.trim_end_matches('/'));
        let v: Value = agent.post(&url).send_json(&body)?.body_mut().read_json()?;
        v["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("no content in the answer"))
    }
}

/// The first `{...}` object in the model's text (from the first `{` to the last `}`).
pub fn parse_json(text: &str) -> anyhow::Result<Value> {
    let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) else {
        anyhow::bail!("no JSON object in the answer")
    };
    if b < a {
        anyhow::bail!("no JSON object in the answer");
    }
    Ok(serde_json::from_str(&text[a..=b])?)
}

// ---------- speech ----------

pub trait Speech: Send + Sync {
    /// The text of a recording (wav bytes).
    fn transcribe(&self, audio: &[u8]) -> anyhow::Result<String>;
}

/// The local speech model. The audio is forwarded and never written to disk.
pub struct HttpSpeech {
    pub url: String,
    pub model: String,
}

impl HttpSpeech {
    /// From `LIFE_ASR_URL` and `LIFE_ASR_MODEL`.
    pub fn from_env() -> HttpSpeech {
        let get = |k: &str, d: &str| {
            std::env::var(k)
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| d.to_string())
        };
        HttpSpeech {
            url: get(
                "LIFE_ASR_URL",
                "http://10.100.3.232:8000/v1/audio/transcriptions",
            ),
            model: get("LIFE_ASR_MODEL", "vibevoice-asr"),
        }
    }
}

static TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[[A-Za-z][A-Za-z ]{1,30}\]").unwrap());
static SPACES_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s{2,}").unwrap());

/// The model's text without sound tags such as `[Environmental Sounds]`, spaces collapsed.
pub fn clean_transcript(text: &str) -> String {
    SPACES_RE
        .replace_all(&TAG_RE.replace_all(text, ""), " ")
        .trim()
        .to_string()
}

impl Speech for HttpSpeech {
    fn transcribe(&self, audio: &[u8]) -> anyhow::Result<String> {
        use sha1::Digest;
        let boundary = format!(
            "----lifeasr{}",
            &hex::encode(sha1::Sha1::digest(&audio[..audio.len().min(64)]))[..16]
        );
        let mut body = Vec::new();
        for (name, value) in [("model", self.model.as_str()), ("language", "ko")] {
            body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
        }
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
        body.extend_from_slice(audio);
        body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(300)))
            .build()
            .into();
        let v: Value = agent
            .post(&self.url)
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .send(&body[..])?
            .body_mut()
            .read_json()?;
        Ok(clean_transcript(
            v.get("text").and_then(Value::as_str).unwrap_or(""),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_object_in_the_text_is_taken() {
        assert_eq!(
            parse_json("답: {\"summary\": \"a\", \"question\": \"b\"} 끝").unwrap(),
            json!({"summary": "a", "question": "b"})
        );
        assert!(parse_json("없음").is_err());
    }

    #[test]
    fn sound_tags_are_dropped_from_a_transcript() {
        assert_eq!(
            clean_transcript("[Environmental Sounds] 회의   시작 [Music]"),
            "회의 시작"
        );
    }
}
