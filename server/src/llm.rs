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
        Ask { temperature, max_tokens, json_mode, timeout: Duration::from_secs(timeout_s) }
    }
}

pub trait Model: Send + Sync {
    /// Send a chat (`[{role, content}]`) and return the model's text.
    fn complete(&self, messages: &[Value], ask: Ask) -> anyhow::Result<String>;
}

/// The user's model server (`LIFE_LLM_URL`, `LIFE_LLM_MODEL`: see [`crate::host::Models`]). There is no built-in one:
/// with no address it answers that none is set.
pub struct Http {
    pub url: String,
    pub model: String,
}

impl Http {
    /// thinking is off: the answers are short and the wait matters. vLLM and llama.cpp read the chat template's
    /// switch, Ollama reads `reasoning_effort`.
    fn body(&self, messages: &[Value], ask: Ask) -> Value {
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": ask.temperature,
            "max_tokens": ask.max_tokens,
            "chat_template_kwargs": {"enable_thinking": false},
            "reasoning_effort": "none",
        });
        if ask.json_mode {
            body["response_format"] = json!({"type": "json_object"});
        }
        body
    }
}

impl Model for Http {
    fn complete(&self, messages: &[Value], ask: Ask) -> anyhow::Result<String> {
        if self.url.is_empty() {
            anyhow::bail!("대화 모델 주소가 설정되지 않았습니다");
        }
        let body = self.body(messages, ask);
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(ask.timeout)).build().into();
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
    let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) else { anyhow::bail!("no JSON object in the answer") };
    if b < a {
        anyhow::bail!("no JSON object in the answer");
    }
    Ok(serde_json::from_str(&text[a..=b])?)
}

// ---------- embedding (ADR 0007) ----------

pub trait Embedder: Send + Sync {
    /// The model's name: a vector is compared only with vectors of the same model.
    fn model(&self) -> String;
    /// One vector per text, in order.
    fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>>;
}

/// How long one embedding request may take.
pub const EMBED_TIMEOUT: Duration = Duration::from_secs(30);

/// The user's embedding model (`LIFE_EMBED_MODEL`) on the user's model server (`LIFE_LLM_URL`; OpenAI-compatible
/// `POST {url}/embeddings`). Only that server is asked: there is no built-in one.
pub struct HttpEmbedder {
    pub url: String,
    pub model: String,
}

impl Embedder for HttpEmbedder {
    fn model(&self) -> String {
        self.model.clone()
    }

    fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        if self.url.is_empty() || self.model.is_empty() {
            anyhow::bail!("임베딩 모델이 설정되지 않았습니다");
        }
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(EMBED_TIMEOUT)).build().into();
        let url = format!("{}/embeddings", self.url.trim_end_matches('/'));
        let v: Value =
            agent.post(&url).send_json(json!({"model": self.model, "input": texts}))?.body_mut().read_json()?;
        let data = v["data"].as_array().ok_or_else(|| anyhow::anyhow!("no data in the answer"))?;
        let mut out: Vec<(u64, Vec<f32>)> = Vec::with_capacity(data.len());
        for (i, d) in data.iter().enumerate() {
            let vec: Vec<f32> = d["embedding"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("no embedding in the answer"))?
                .iter()
                .map(|x| x.as_f64().map(|x| x as f32).ok_or_else(|| anyhow::anyhow!("not a number")))
                .collect::<anyhow::Result<_>>()?;
            out.push((d["index"].as_u64().unwrap_or(i as u64), vec));
        }
        out.sort_by_key(|(i, _)| *i);
        if out.len() != texts.len() || out.iter().any(|(_, v)| v.is_empty()) {
            anyhow::bail!("the answer has {} vectors for {} texts", out.len(), texts.len());
        }
        Ok(out.into_iter().map(|(_, v)| v).collect())
    }
}

/// A fake for tests and trial runs: each word of a text (split at spaces and line breaks) adds one to a place chosen
/// by its hash, so texts sharing words are near. The same text always gives the same vector.
pub struct WordsEmbedder {
    pub model: String,
    pub dim: usize,
}

impl WordsEmbedder {
    pub fn new(model: &str) -> WordsEmbedder {
        WordsEmbedder { model: model.into(), dim: 256 }
    }
}

impl Embedder for WordsEmbedder {
    fn model(&self) -> String {
        self.model.clone()
    }

    fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        use sha1::Digest;
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0f32; self.dim];
                for w in t.split_whitespace() {
                    let h = sha1::Sha1::digest(w.as_bytes());
                    v[u32::from_le_bytes([h[0], h[1], h[2], h[3]]) as usize % self.dim] += 1.0;
                }
                v
            })
            .collect())
    }
}

// ---------- speech ----------

pub trait Speech: Send + Sync {
    /// The text of a recording (wav bytes).
    fn transcribe(&self, audio: &[u8]) -> anyhow::Result<String>;
}

/// The user's speech model (`LIFE_ASR_URL`, `LIFE_ASR_MODEL`). The audio is forwarded and never written to disk.
pub struct HttpSpeech {
    pub url: String,
    pub model: String,
}

static TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[A-Za-z][A-Za-z ]{1,30}\]").unwrap());
static SPACES_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s{2,}").unwrap());

/// The model's text without sound tags such as `[Environmental Sounds]`, spaces collapsed.
pub fn clean_transcript(text: &str) -> String {
    SPACES_RE.replace_all(&TAG_RE.replace_all(text, ""), " ").trim().to_string()
}

impl Speech for HttpSpeech {
    fn transcribe(&self, audio: &[u8]) -> anyhow::Result<String> {
        use sha1::Digest;
        if self.url.is_empty() {
            anyhow::bail!("음성 모델 주소가 설정되지 않았습니다");
        }
        let boundary = format!("----lifeasr{}", &hex::encode(sha1::Sha1::digest(&audio[..audio.len().min(64)]))[..16]);
        let mut body = Vec::new();
        for (name, value) in [("model", self.model.as_str()), ("language", "ko")] {
            body.extend(
                format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                    .as_bytes(),
            );
        }
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
        body.extend_from_slice(audio);
        body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
        let agent: ureq::Agent =
            ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(300))).build().into();
        let v: Value = agent
            .post(&self.url)
            .header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
            .send(&body[..])?
            .body_mut()
            .read_json()?;
        Ok(clean_transcript(v.get("text").and_then(Value::as_str).unwrap_or("")))
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
    fn an_address_not_set_is_said_and_no_server_is_asked() {
        // a fresh install has no built-in model server (#66)
        let ask = Ask::new(0.0, 10, false, 1);
        let chat = Http { url: String::new(), model: String::new() }.complete(&[], ask).unwrap_err();
        assert_eq!(chat.to_string(), "대화 모델 주소가 설정되지 않았습니다");
        let speech = HttpSpeech { url: String::new(), model: String::new() }.transcribe(b"x").unwrap_err();
        assert_eq!(speech.to_string(), "음성 모델 주소가 설정되지 않았습니다");
    }

    #[test]
    fn thinking_is_turned_off_the_ways_the_usual_servers_read() {
        // vLLM and llama.cpp read the chat template's switch; Ollama reads reasoning_effort (#66)
        let m = Http { url: "http://127.0.0.1:9/v1".into(), model: "qwen3:8b".into() };
        let b = m.body(&[json!({"role": "user", "content": "안녕"})], Ask::new(0.0, 10, true, 1));
        assert_eq!(b["chat_template_kwargs"], json!({"enable_thinking": false}));
        assert_eq!(b["reasoning_effort"], "none");
        assert_eq!(b["response_format"], json!({"type": "json_object"}));
        assert_eq!((b["model"].as_str(), b["max_tokens"].as_u64()), (Some("qwen3:8b"), Some(10)));
    }

    #[test]
    fn the_words_fake_is_the_same_for_the_same_text_and_nearer_for_shared_words() {
        let e = WordsEmbedder::new("fake");
        let v = e.embed(&["금형 견적".into(), "금형 견적".into(), "금형 단가".into(), "출장 보고".into()]).unwrap();
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        assert_eq!(v[0], v[1]);
        assert!(dot(&v[0], &v[2]) > dot(&v[0], &v[3]));
    }

    #[test]
    fn sound_tags_are_dropped_from_a_transcript() {
        assert_eq!(clean_transcript("[Environmental Sounds] 회의   시작 [Music]"), "회의 시작");
    }
}
