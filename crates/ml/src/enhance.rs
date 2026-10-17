//! Prompt enhancement: a language model rewrites the few words a user types ("empty tiled floor")
//! into the fuller description an image model paints from. It speaks the OpenAI-compatible chat
//! API (`POST <base>/chat/completions`), which hosted services and local servers (Ollama, LM
//! Studio, llama.cpp) all offer, so the model can be a small cloud one or a small local one.
//!
//! Optional and off unless the user sets a URL, model and (for anything but a local server) a key.
//! **Only the prompt text is sent**, never an image. As with the AI service, the key only travels
//! over HTTPS (plain HTTP is accepted for loopback addresses) and is never logged or put in an
//! error message.

use crate::{Error, Result};

/// Longest prompt taken or returned (the image models take at most this).
pub const MAX_PROMPT_CHARS: usize = 500;

/// What the model is told. It must answer with the prompt alone.
const SYSTEM: &str = "You write prompts for an image inpainting model. The user gives a few words about what should \
fill a region of a photo. Rewrite them as one concrete, photographic description of what is visible in that region \
(at most 50 words): materials, lighting, colours, texture. No text or watermarks, no quotes, no preamble. Output \
only the prompt.";

/// Where the language model is and how to authenticate.
#[derive(Clone)]
pub struct EnhancerConfig {
    pub base_url: String,
    pub model: String,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    api_key: String,
}

// The key must never reach a log through `{:?}`.
impl std::fmt::Debug for EnhancerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnhancerConfig").field("base_url", &self.base_url).field("model", &self.model).field("api_key", &"<hidden>").finish()
    }
}

fn is_loopback(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else { return false };
    matches!(rest.split(['/', ':']).next().unwrap_or(""), "localhost" | "127.0.0.1" | "[::1]")
}

/// A model id as services spell them (`gpt-4.1-nano`, `llama3.2:3b`, `org/model`).
fn model_is_valid(model: &str) -> bool {
    !model.is_empty() && model.len() <= 80 && model.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
}

impl EnhancerConfig {
    /// A configuration; `None` when the URL could expose the key (not HTTPS and not loopback), the
    /// model name is odd, or there is no key for a remote service (a local server needs none).
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Option<Self> {
        let base_url = base_url.trim().trim_end_matches('/').to_string();
        let (api_key, model) = (api_key.trim().to_string(), model.trim().to_string());
        let local = is_loopback(&base_url);
        let url_ok = base_url.starts_with("https://") || local;
        let key_ok = if api_key.is_empty() { local } else { api_key.chars().all(|c| c.is_ascii_graphic()) };
        (url_ok && key_ok && model_is_valid(&model)).then_some(Self { base_url, model, api_key })
    }

    /// Whether `url` may be used at all (for validating a settings field).
    pub fn url_is_allowed(url: &str) -> bool {
        let url = url.trim();
        url.starts_with("https://") || is_loopback(url)
    }

    /// Rewrites `prompt` into a fuller one for the image model (at most [`MAX_PROMPT_CHARS`]).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn enhance(&self, prompt: &str) -> Result<String> {
        use std::time::Duration;
        let text = prompt.trim();
        if text.is_empty() || text.chars().count() > MAX_PROMPT_CHARS {
            return Err(Error::BadInput(format!("the prompt must be 1 to {MAX_PROMPT_CHARS} characters")));
        }
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": text}],
            "max_completion_tokens": 300,
        })
        .to_string();
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(3))).timeout_global(Some(Duration::from_secs(45))).http_status_as_error(false).build().into();
        let url = format!("{}/chat/completions", self.base_url);
        let mut req = agent.post(&url).content_type("application/json");
        if !self.api_key.is_empty() {
            req = req.header("Authorization", &format!("Bearer {}", self.api_key));
        }
        // Transport errors describe the connection, never the request headers.
        let mut resp = req.send(body.as_bytes()).map_err(|e| Error::Remote(format!("request failed: {e}")))?;
        let status = resp.status().as_u16();
        let bytes = resp.body_mut().with_config().limit(1 << 20).read_to_vec().map_err(|e| Error::Remote(format!("reading the answer failed: {e}")))?;
        if !(200..300).contains(&status) {
            return Err(Error::Remote(format!("HTTP {status} {}", self.redact(&error_message(&bytes))).trim().to_string()));
        }
        parse_chat(&bytes)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn enhance(&self, _: &str) -> Result<String> {
        Err(Error::Unsupported)
    }

    /// `text` with the key (or what a service echoes of it) removed.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn redact(&self, text: &str) -> String {
        if self.api_key.is_empty() { text.to_string() } else { text.replace(&self.api_key, "<key>") }
    }
}

/// The service's error message (`{"error":{"message":…}}`), shortened.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn error_message(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body).ok().and_then(|v| v["error"]["message"].as_str().map(|m| m.chars().take(160).collect())).unwrap_or_default()
}

/// The prompt in a chat completion: the first choice's text, tidied (no wrapping quotes, one line).
fn parse_chat(body: &[u8]) -> Result<String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| Error::Remote("unexpected answer".into()))?;
    let text = v["choices"][0]["message"]["content"].as_str().ok_or_else(|| Error::Remote("the model returned no text".into()))?;
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let tidy = one_line.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '“' | '”')).trim();
    if tidy.is_empty() {
        return Err(Error::Remote("the model returned an empty prompt".into()));
    }
    Ok(tidy.chars().take(MAX_PROMPT_CHARS).collect::<String>().trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_only_travel_over_https_or_to_a_local_server() {
        assert!(EnhancerConfig::new("https://api.openai.com/v1/", "sk-abc", "gpt-4.1-nano").is_some());
        assert_eq!(EnhancerConfig::new("https://api.openai.com/v1///", "k", "m").map(|c| c.base_url), Some("https://api.openai.com/v1".into()));
        assert!(EnhancerConfig::new("http://127.0.0.1:11434/v1", "", "llama3.2:3b").is_some(), "a local server needs no key");
        assert!(EnhancerConfig::new("http://localhost:1234/v1", "k", "org/model").is_some());
        assert!(EnhancerConfig::new("http://example.com/v1", "k", "m").is_none(), "no key in clear text");
        assert!(EnhancerConfig::new("http://127.0.0.1.evil.com/v1", "k", "m").is_none());
        assert!(EnhancerConfig::new("https://api.openai.com/v1", "", "m").is_none(), "a remote service needs a key");
        assert!(EnhancerConfig::new("https://x/v1", "bad key", "m").is_none() && EnhancerConfig::new("https://x/v1", "k\nX: y", "m").is_none());
        assert!(EnhancerConfig::new("https://x/v1", "k", "").is_none() && EnhancerConfig::new("https://x/v1", "k", "a b").is_none());
        assert!(EnhancerConfig::url_is_allowed(" https://x ") && EnhancerConfig::url_is_allowed("http://localhost") && !EnhancerConfig::url_is_allowed("http://x"));
    }

    #[test]
    fn the_key_is_never_printed_or_echoed() {
        let c = EnhancerConfig::new("https://x/v1", "sk-secret-value", "m").unwrap();
        assert!(!format!("{c:?}").contains("sk-secret-value"));
        assert_eq!(c.redact("Incorrect API key: sk-secret-value."), "Incorrect API key: <key>.");
    }

    #[test]
    fn answers_are_tidied_and_junk_is_an_error() {
        let chat = |s: &str| serde_json::json!({"choices": [{"message": {"content": s}}]}).to_string();
        assert_eq!(parse_chat(chat("  \"A smooth tiled floor,\n soft light.\"  ").as_bytes()).unwrap(), "A smooth tiled floor, soft light.");
        let long = parse_chat(chat(&"w ".repeat(400)).as_bytes()).unwrap();
        assert!(long.chars().count() <= MAX_PROMPT_CHARS && !long.ends_with(' '), "{}", long.len());
        for bad in [chat(" \"\" "), "not json".to_string(), r#"{"choices":[]}"#.to_string()] {
            assert!(parse_chat(bad.as_bytes()).is_err(), "{bad}");
        }
        assert_eq!(error_message(br#"{"error":{"message":"Incorrect API key provided"}}"#), "Incorrect API key provided");
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod http {
        use super::*;
        use std::io::{Read, Write};
        use std::net::TcpListener;

        /// A loopback server giving one answer; returns the request head and body when joined.
        fn serve(status: &'static str, body: String) -> (String, std::thread::JoinHandle<(String, String)>) {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/v1", l.local_addr().unwrap());
            let h = std::thread::spawn(move || {
                let (mut s, _) = l.accept().unwrap();
                let mut got = Vec::new();
                let mut buf = [0u8; 8192];
                let head_end = loop {
                    if let Some(p) = got.windows(4).position(|w| w == b"\r\n\r\n") {
                        break p + 4;
                    }
                    let n = s.read(&mut buf).unwrap();
                    if n == 0 {
                        break got.len();
                    }
                    got.extend(&buf[..n]);
                };
                let head = String::from_utf8_lossy(&got[..head_end]).to_string();
                let want = head.to_ascii_lowercase().lines().find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                while got.len() < head_end + want {
                    let n = s.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    got.extend(&buf[..n]);
                }
                write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                (head, String::from_utf8_lossy(&got[head_end..]).to_string())
            });
            (url, h)
        }

        #[test]
        fn a_prompt_round_trips_through_a_chat_completion() {
            let (url, h) = serve("200 OK", r#"{"choices":[{"message":{"content":"A polished beige tile floor with fine grout lines."}}]}"#.into());
            let c = EnhancerConfig::new(&url, "sk-test", "gpt-4.1-nano").unwrap();
            assert_eq!(c.enhance("  empty floor ").unwrap(), "A polished beige tile floor with fine grout lines.");
            let (head, body) = h.join().unwrap();
            let head = head.to_ascii_lowercase();
            assert!(head.starts_with("post /v1/chat/completions") && head.contains("authorization: bearer sk-test"), "{head}");
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!((v["model"].as_str(), v["messages"][1]["content"].as_str()), (Some("gpt-4.1-nano"), Some("empty floor")));
            assert!(v["messages"][0]["content"].as_str().unwrap().contains("inpainting"));
        }

        #[test]
        fn a_local_server_needs_no_key_and_errors_never_leak_one() {
            let (url, h) = serve("200 OK", r#"{"choices":[{"message":{"content":"ok"}}]}"#.into());
            assert_eq!(EnhancerConfig::new(&url, "", "llama3.2:3b").unwrap().enhance("x").unwrap(), "ok");
            assert!(!h.join().unwrap().0.to_ascii_lowercase().contains("authorization"));
            let (url, h) = serve("401 Unauthorized", r#"{"error":{"message":"Incorrect API key provided: sk-test-1234."}}"#.into());
            let e = EnhancerConfig::new(&url, "sk-test-1234", "m").unwrap().enhance("x").unwrap_err().to_string();
            h.join().unwrap();
            assert!(e.contains("401") && e.contains("Incorrect API key") && !e.contains("sk-test-1234"), "{e}");
        }

        #[test]
        fn bad_prompts_never_reach_the_network() {
            let c = EnhancerConfig::new("http://127.0.0.1:1/v1", "", "m").unwrap();
            assert!(c.enhance("  ").is_err() && c.enhance(&"y".repeat(MAX_PROMPT_CHARS + 1)).is_err());
        }
    }
}
