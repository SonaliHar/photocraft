//! The JSON and multipart shapes of SPEC amendment 11, and the checks that keep a hostile
//! manager from steering the client (job ids go into a URL, error text goes into a message).

use std::sync::atomic::{AtomicU64, Ordering};

use photocraft_engine::ai::{AiError, AiJobState, AiRequest, AiTask};
use serde::Serialize;
use serde_json::{Value, json};

use crate::http::MAX_JSON_BODY_BYTES;

/// The provider the client asks for; the manager picks the backend.
const PROVIDER_AUTO: &str = "auto";
/// Job ids are opaque, but they are put into a URL path, so they are checked before use.
const MAX_JOB_ID_BYTES: usize = 512;
const MAX_PROMPT_BYTES: usize = 4096;
const MAX_CLIENT_BYTES: usize = 256;
const MAX_SENTENCE_CHARS: usize = 300;
static BOUNDARY_COUNTER: AtomicU64 = AtomicU64::new(1);

/// One part of the `multipart/form-data` body of `POST /v1/jobs`.
pub struct Part<'a> {
    pub name: &'static str,
    pub filename: Option<&'static str>,
    pub content_type: &'static str,
    pub data: &'a [u8],
}

/// The `job` part: task, window, depth, channels, prompt, and who is asking.
pub fn job_json(req: &AiRequest, bit_depth: u8, channels: u8) -> Result<Vec<u8>, AiError> {
    let too_large = || AiError::Other("The area to fill is too large for Craft AI Manager.".into());
    let size = req.window.size();
    let Ok(width) = i32::try_from(size.width) else {
        return Err(too_large());
    };
    let Ok(height) = i32::try_from(size.height) else {
        return Err(too_large());
    };
    let prompt = usable_prompt(req.prompt.as_deref())?;
    #[derive(Serialize)]
    struct JobJson<'a> {
        task: &'a str,
        image_size: [u32; 2],
        region: [i32; 4],
        bit_depth: u8,
        channels: u8,
        prompt: Option<&'a str>,
        settings: Value,
        provider: &'static str,
        client: &'a str,
    }
    let body = JobJson {
        task: req.task.id(),
        image_size: [size.width, size.height],
        region: [req.window.x0, req.window.y0, width, height],
        bit_depth,
        channels,
        prompt,
        settings: json!({}),
        provider: PROVIDER_AUTO,
        client: client_label(&req.client_version),
    };
    let bytes = serde_json::to_vec(&body).map_err(|_| AiError::Other("PhotoCraft could not build the job request for Craft AI Manager.".into()))?;
    if bytes.len() > MAX_JSON_BODY_BYTES {
        return Err(AiError::Other("The job request is larger than the 64 KiB the protocol allows.".into()));
    }
    Ok(bytes)
}

fn usable_prompt(prompt: Option<&str>) -> Result<Option<&str>, AiError> {
    match prompt {
        None | Some("") => Ok(None),
        Some(text) if text.len() <= MAX_PROMPT_BYTES && !text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t')) => Ok(Some(text)),
        Some(_) => Err(AiError::Other("The prompt is not usable: keep it under 4 KiB of plain text, without control characters.".into())),
    }
}

/// The `client` field is our own build version, but it is data: fall back rather than trust it.
fn client_label(client: &str) -> &str {
    if client.is_empty() || client.len() > MAX_CLIENT_BYTES || client.chars().any(char::is_control) { "photocraft" } else { client }
}

/// A boundary that the parts do not contain. It is drawn per submit, so a document cannot know it
/// in advance; the scan is a cheap guard, not a security measure.
pub fn boundary(parts: &[Part<'_>]) -> Result<String, AiError> {
    for attempt in 0..8u32 {
        let seed = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().hash(&mut hasher);
            BOUNDARY_COUNTER.fetch_add(1, Ordering::Relaxed).hash(&mut hasher);
            std::thread::current().id().hash(&mut hasher);
            hasher.finish()
        };
        let candidate = format!("photocraft-caim-{:016x}-{attempt}", seed & 0xffff_ffff);
        let marker = format!("--{candidate}");
        if parts.iter().all(|part| !contains_bytes(part.data, marker.as_bytes())) {
            return Ok(candidate);
        }
    }
    Err(AiError::Other("PhotoCraft could not find a multipart boundary that the image and mask do not contain.".into()))
}

/// The `multipart/form-data` body, with the parts in the order the protocol asks for.
pub fn multipart(boundary: &str, parts: &[Part<'_>]) -> Vec<u8> {
    let mut body = Vec::new();
    for part in parts {
        body.extend_from_slice(b"--");
        body.extend_from_slice(boundary.as_bytes());
        body.extend_from_slice(b"\r\nContent-Disposition: form-data; name=\"");
        body.extend_from_slice(part.name.as_bytes());
        body.extend_from_slice(b"\"");
        if let Some(filename) = part.filename {
            body.extend_from_slice(b"; filename=\"");
            body.extend_from_slice(filename.as_bytes());
            body.extend_from_slice(b"\"");
        }
        body.extend_from_slice(b"\r\nContent-Type: ");
        body.extend_from_slice(part.content_type.as_bytes());
        body.extend_from_slice(b"\r\n\r\n");
        body.extend_from_slice(part.data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");
    body
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    let (first, second) = (needle[0], needle.get(1).copied().unwrap_or(needle[0]));
    let mut start = 0;
    while let Some(offset) = haystack[start..].windows(2).position(|pair| pair[0] == first && pair[1] == second) {
        let at = start + offset;
        if haystack.len() - at >= needle.len() && &haystack[at..at + needle.len()] == needle {
            return true;
        }
        start = at + 1;
        if start >= haystack.len() {
            break;
        }
    }
    false
}

/// `{"id": "<opaque>"}` from `POST /v1/jobs`.
pub fn parse_job_id(body: &[u8]) -> Result<String, AiError> {
    let value = json_body(body, "a job id")?;
    let id = value.get("id").and_then(Value::as_str).unwrap_or_default();
    if !is_safe_id(id) {
        return Err(AiError::Other("Craft AI Manager returned a job id that cannot be used.".into()));
    }
    Ok(id.to_string())
}

/// The path for a job. The id came from the network, so it is checked before it reaches a URL.
pub fn job_path(id: &str) -> Result<String, AiError> {
    if !is_safe_id(id) {
        return Err(AiError::Other("That job id did not come from Craft AI Manager.".into()));
    }
    Ok(format!("/v1/jobs/{id}"))
}

fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_JOB_ID_BYTES
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
        // A dot-dot id would walk the path the manager serves.
        && !id.contains("..")
}

/// `{"state": ..., "progress": 0-1, "message": "..."}` from `GET /v1/jobs/{id}`.
pub fn parse_state(body: &[u8]) -> Result<AiJobState, AiError> {
    let value = json_body(body, "a job state")?;
    let state = value.get("state").and_then(Value::as_str).ok_or_else(|| AiError::Other("Craft AI Manager did not say what the job is doing.".into()))?;
    let progress = value.get("progress").and_then(Value::as_f64).map(|p| (p as f32).clamp(0.0, 1.0)).unwrap_or(0.0);
    let message = value.get("message").and_then(Value::as_str).map(trim_sentence).filter(|text| !text.is_empty()).unwrap_or_else(|| match state {
        "queued" => "Waiting for Craft AI Manager".to_string(),
        "running" => "Craft AI Manager is working".to_string(),
        _ => String::new(),
    });
    match state {
        "queued" => Ok(AiJobState::Queued { progress, message }),
        "running" => Ok(AiJobState::Running { progress, message }),
        "done" => Ok(AiJobState::Done),
        "failed" => Ok(AiJobState::Failed(message)),
        "cancelled" => Ok(AiJobState::Cancelled),
        other => Err(AiError::Other(format!("Craft AI Manager reported a job state PhotoCraft does not know: \"{}\".", trim_sentence(other)))),
    }
}

/// Where a finished job ran: model, device, provider, timing, and the size it claims to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub model: String,
    pub device: String,
    pub provider: String,
    pub elapsed_ms: u64,
    pub image_size: Option<[u32; 2]>,
}

/// `GET /v1/jobs/{id}/result`. Missing fields say "unknown" rather than failing a finished job.
pub fn parse_provenance(body: &[u8]) -> Result<Provenance, AiError> {
    let value = json_body(body, "a job result")?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(trim_sentence).filter(|text| !text.is_empty()).unwrap_or_else(|| "unknown".to_string());
    let image_size = match value.get("image_size") {
        Some(Value::Array(pair)) if pair.len() == 2 => {
            let width = pair.first().and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok());
            let height = pair.get(1).and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok());
            match (width, height) {
                (Some(width), Some(height)) => Some([width, height]),
                _ => None,
            }
        }
        _ => None,
    };
    Ok(Provenance {
        model: text("model"),
        device: text("device"),
        provider: text("provider"),
        elapsed_ms: value.get("elapsed_ms").and_then(Value::as_u64).unwrap_or(0),
        image_size,
    })
}

/// `GET /v1/tasks`: the tasks this manager offers now. Unknown ids are skipped, not fatal.
pub fn parse_tasks(body: &[u8]) -> Result<Vec<AiTask>, AiError> {
    let value = json_body(body, "a task list")?;
    let tasks = value.get("tasks").and_then(Value::as_array).ok_or_else(|| AiError::Other("Craft AI Manager did not send its list of tasks.".into()))?;
    let mut out: Vec<AiTask> = Vec::new();
    for task in tasks {
        let id = task.get("id").and_then(Value::as_str).unwrap_or_default();
        if let Ok(parsed) = serde_json::from_value::<AiTask>(Value::String(id.to_string()))
            && !out.contains(&parsed)
        {
            out.push(parsed);
        }
    }
    Ok(out)
}

/// The `{"error": "<plain sentence>"}` body the protocol defines, or a plain fallback.
pub fn error_sentence(status: u16, body: &[u8]) -> String {
    json_body(body, "an error")
        .ok()
        .and_then(|value| value.get("error").and_then(Value::as_str).map(trim_sentence))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| format!("Craft AI Manager answered HTTP {status}."))
}

fn json_body(body: &[u8], what: &str) -> Result<Value, AiError> {
    let text = String::from_utf8_lossy(body);
    serde_json::from_str::<Value>(&text).map_err(|_| AiError::Other(format!("Craft AI Manager answered with something that is not {what}.")))
}

fn trim_sentence(text: &str) -> String {
    let clipped: String = text.chars().take(MAX_SENTENCE_CHARS).filter(|c| !c.is_control()).collect();
    clipped.trim().to_string()
}
