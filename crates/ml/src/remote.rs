//! A remote image-AI service (the PhotoCraft GPU API), used instead of or beside the local models
//! when the user configures one. Optional and off by default: nothing is sent anywhere unless a
//! URL and key are set (Preferences › AI services, or the `PHOTOCRAFT_REMOTE_URL` /
//! `PHOTOCRAFT_REMOTE_KEY` environment variables).
//!
//! * `POST <base>/segment`: point / box prompts, answered synchronously with masks.
//! * `POST <base>/decompose` + `GET /jobs/{id}` + `GET /jobs/{id}/result` + `DELETE /jobs/{id}`:
//!   split an image into layers; a job that takes minutes, so it is submitted, polled and fetched.
//! * `GET <base>/models`: what the server offers (also the "test connection" call).
//!
//! The key only travels over HTTPS (plain HTTP is accepted for loopback addresses, for testing),
//! and is never logged or put in an error message.

use base64::Engine as _;

use crate::{Error, Result};

/// Where the remote service is and how to authenticate.
#[derive(Clone)]
pub struct RemoteConfig {
    pub base_url: String,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    api_key: String,
}

// The key must never reach a log through `{:?}`.
impl std::fmt::Debug for RemoteConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteConfig").field("base_url", &self.base_url).field("api_key", &"<hidden>").finish()
    }
}

/// Whether `url` may carry an API key: HTTPS, or HTTP to a loopback host.
fn url_allowed(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else { return false };
    let host = rest.split(['/', ':']).next().unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

impl RemoteConfig {
    /// A configuration for `base_url` (trailing slashes ignored); `None` if the key is empty or the
    /// URL could expose it (not HTTPS and not loopback).
    pub fn new(base_url: &str, api_key: &str) -> Option<Self> {
        let base_url = base_url.trim().trim_end_matches('/').to_string();
        let api_key = api_key.trim().to_string();
        (!api_key.is_empty() && url_allowed(&base_url)).then_some(Self { base_url, api_key })
    }

    /// From `PHOTOCRAFT_REMOTE_URL` and `PHOTOCRAFT_REMOTE_KEY`; `None` when unset or unsafe.
    pub fn from_env() -> Option<Self> {
        Self::new(&std::env::var("PHOTOCRAFT_REMOTE_URL").ok()?, &std::env::var("PHOTOCRAFT_REMOTE_KEY").ok()?)
    }

    /// Whether `url` is acceptable as a base URL at all (for validating a settings field).
    pub fn url_is_allowed(url: &str) -> bool {
        url_allowed(url.trim())
    }
}

// ---------------------------------------------------------------------------------------------
// segment

/// One prompt in the uploaded image's own pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SegmentPrompt {
    Point { x: f32, y: f32, positive: bool },
    Box { x0: f32, y0: f32, x1: f32, y1: f32 },
}

/// A mask the service returned: an 8-bit greyscale PNG at the uploaded image's size.
#[derive(Clone, Debug)]
pub struct RemoteMask {
    pub png: Vec<u8>,
    pub score: f32,
    pub width: u32,
    pub height: u32,
}

/// The `prompts` JSON the service takes.
fn prompts_json(prompts: &[SegmentPrompt]) -> Result<String> {
    let mut points = Vec::new();
    let mut boxed = None;
    for p in prompts {
        match *p {
            SegmentPrompt::Point { x, y, positive } => {
                if !x.is_finite() || !y.is_finite() {
                    return Err(Error::BadInput("prompt coordinates must be finite".into()));
                }
                points.push(serde_json::json!({"x": x, "y": y, "positive": positive}));
            }
            SegmentPrompt::Box { x0, y0, x1, y1 } => {
                if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
                    return Err(Error::BadInput("prompt coordinates must be finite".into()));
                }
                boxed = Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]);
            }
        }
    }
    if points.is_empty() && boxed.is_none() {
        return Err(Error::BadInput("needs at least one prompt".into()));
    }
    let mut v = serde_json::json!({"model": "sam2_large", "points": points, "multimask": false});
    if let Some(b) = boxed {
        v["box"] = serde_json::json!(b);
    }
    Ok(v.to_string())
}

/// Reads the service's answer (`{"masks":[{"png_base64","score",...}],"width","height"}`).
fn parse_segment(body: &str) -> Result<Vec<RemoteMask>> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|_| Error::Remote("unexpected answer".into()))?;
    let (w, h) = (v["width"].as_u64().unwrap_or(0) as u32, v["height"].as_u64().unwrap_or(0) as u32);
    let masks = v["masks"].as_array().ok_or_else(|| Error::Remote("unexpected answer".into()))?;
    masks
        .iter()
        .map(|m| {
            let b64 = m["png_base64"].as_str().ok_or_else(|| Error::Remote("mask missing".into()))?;
            let png = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|_| Error::Remote("mask is not base64".into()))?;
            Ok(RemoteMask { png, score: m["score"].as_f64().unwrap_or(0.0) as f32, width: w, height: h })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// decompose

/// What a decomposition returns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecomposeMode {
    /// The model's own RGBA layers, including content completed behind occluders.
    #[default]
    Generative,
    /// Alpha masks only: the client applies them to the original pixels.
    Mask,
}

impl DecomposeMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Generative => "generative",
            Self::Mask => "mask",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "generative" => Some(Self::Generative),
            "mask" => Some(Self::Mask),
            _ => None,
        }
    }
}

/// Options of one decomposition (the service validates the ranges).
#[derive(Clone, Debug, PartialEq)]
pub struct DecomposeParams {
    /// Number of layers, 2 to 10.
    pub layers: u32,
    /// Generation size of the long side: 640 or 1024.
    pub resolution: u32,
    pub steps: u32,
    pub cfg: f32,
    pub seed: Option<u64>,
    /// Optional description of the image; empty lets the model caption it.
    pub prompt: String,
    pub mode: DecomposeMode,
}

impl Default for DecomposeParams {
    fn default() -> Self {
        Self { layers: 4, resolution: 640, steps: 40, cfg: 4.0, seed: None, prompt: String::new(), mode: DecomposeMode::Generative }
    }
}

/// Where a job is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Expired,
}

impl JobState {
    /// No further change will happen.
    pub fn is_final(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

/// A job's progress.
#[derive(Clone, Debug, PartialEq)]
pub struct JobStatus {
    pub state: JobState,
    pub queue_position: Option<u32>,
    pub step: u32,
    pub steps: u32,
    pub eta_seconds: Option<f32>,
    /// The service's problem title when `Failed`.
    pub error: Option<String>,
}

/// One returned layer, back to front by `z`.
#[derive(Clone, Debug)]
pub struct DecomposedLayer {
    pub z: i32,
    pub caption: Option<String>,
    /// `[x0, y0, x1, y1]` in generated pixels.
    pub bbox: [i32; 4],
    pub alpha_coverage: f32,
    /// RGBA PNG (generative) or 8-bit greyscale PNG (mask).
    pub png: Vec<u8>,
}

/// A finished decomposition.
#[derive(Clone, Debug)]
pub struct Decomposition {
    pub mode: DecomposeMode,
    /// Size of the generated layers.
    pub width: u32,
    pub height: u32,
    /// Mean difference (0..1) between the stacked layers and the input, when reported.
    pub composite_error: Option<f32>,
    /// Back to front.
    pub layers: Vec<DecomposedLayer>,
}

/// What the service offers (`GET /models`).
#[derive(Clone, Debug, PartialEq)]
pub struct ServerInfo {
    pub api_version: String,
    pub models: Vec<String>,
    pub queue_length: u32,
    /// The ways to fill a hole ([`InpaintParams::method`]): `"fast"` when a fast filler is
    /// installed, then each generative model id. Empty when the service has no inpainting.
    pub fill_methods: Vec<String>,
    /// The service can rewrite a short prompt into a better one (`POST /prompt/enhance`).
    pub prompt_enhance: bool,
}

/// What `POST /inpaint` is asked to do.
#[derive(Clone, Debug, PartialEq)]
pub struct InpaintParams {
    /// `"fast"` (a quick filler that invents nothing) or the id of a generative model
    /// (`"sdxl"`, `"juggernaut"`, whatever [`ServerInfo::fill_methods`] lists).
    pub method: String,
    /// Pixels the service grows the mask by; PhotoCraft grows it itself and sends 0.
    pub dilate: u32,
    /// Diffusion steps (generative methods only).
    pub steps: u32,
    pub prompt: String,
    pub seed: Option<u64>,
    /// How much of the picture around the hole the model sees, 0..=1 (generative methods only;
    /// the service's default, 0.618, when `None`).
    pub context: Option<f32>,
    /// Strength of a second "improve detail" pass over the filled area, 0..=1 (generative methods
    /// only; 0 or `None` is off, 0.5 is Fooocus's Improve Detail).
    pub refine: Option<f32>,
}

impl Default for InpaintParams {
    fn default() -> Self {
        Self { method: "fast".into(), dilate: 0, steps: 30, prompt: String::new(), seed: None, context: None, refine: None }
    }
}

/// Whether `method` is something safe to send as a model id.
pub fn method_is_valid(method: &str) -> bool {
    !method.is_empty() && method.len() <= 40 && method.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A finished inpaint: the whole uploaded image with the masked area filled.
#[derive(Clone, Debug)]
pub struct Inpainting {
    pub width: u32,
    pub height: u32,
    /// The model that did the fill, as the service reports it.
    pub model: String,
    /// PNG, exactly the size of the uploaded image.
    pub png: Vec<u8>,
}

const BOUNDARY: &str = "----photocraft-boundary-7f3a9c";

/// One part of a `multipart/form-data` body.
struct Part<'a> {
    name: &'a str,
    /// `(filename, content type)` for a file part.
    file: Option<(&'a str, &'a str)>,
    data: &'a [u8],
}

fn multipart(boundary: &str, parts: &[Part<'_>]) -> Vec<u8> {
    let mut b = Vec::with_capacity(parts.iter().map(|p| p.data.len() + 200).sum());
    for p in parts {
        b.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"", p.name).as_bytes());
        if let Some((filename, mime)) = p.file {
            b.extend(format!("; filename=\"{filename}\"\r\nContent-Type: {mime}").as_bytes());
        }
        b.extend(b"\r\n\r\n");
        b.extend(p.data);
        b.extend(b"\r\n");
    }
    b.extend(format!("--{boundary}--\r\n").as_bytes());
    b
}

/// The service's problem title (RFC 9457), if the body is one.
fn problem_title(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body).ok().and_then(|v| v["title"].as_str().map(|t| t.chars().take(120).collect())).unwrap_or_default()
}

fn parse_state(s: &str) -> Option<JobState> {
    Some(match s {
        "queued" => JobState::Queued,
        "running" => JobState::Running,
        "succeeded" => JobState::Succeeded,
        "failed" => JobState::Failed,
        "cancelled" => JobState::Cancelled,
        "expired" => JobState::Expired,
        _ => return None,
    })
}

fn parse_status(body: &[u8]) -> Result<JobStatus> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| Error::Remote("unexpected answer".into()))?;
    let state = v["state"].as_str().and_then(parse_state).ok_or_else(|| Error::Remote("unexpected answer".into()))?;
    let error = match &v["error"] {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.chars().take(120).collect()),
        o => o["title"].as_str().map(|t| t.chars().take(120).collect()),
    };
    Ok(JobStatus {
        state,
        queue_position: v["queue_position"].as_u64().map(|n| n as u32),
        step: v["step"].as_u64().unwrap_or(0) as u32,
        steps: v["steps"].as_u64().unwrap_or(0) as u32,
        eta_seconds: v["eta_seconds"].as_f64().map(|e| e as f32),
        error,
    })
}

/// Entries larger than this, or archives with more entries than [`MAX_ZIP_ENTRIES`], are refused
/// (a hostile or broken server cannot make the app allocate without bound).
const MAX_ENTRY_BYTES: u64 = 128 << 20;
const MAX_ZIP_ENTRIES: usize = 64;

/// Reads a result archive into `(file name, bytes)`; names are only lookup keys, never paths.
fn read_zip(zip_bytes: &[u8]) -> Result<std::collections::HashMap<String, Vec<u8>>> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|_| Error::Remote("the result is not a zip file".into()))?;
    if zip.len() > MAX_ZIP_ENTRIES {
        return Err(Error::Remote("the result has too many files".into()));
    }
    let mut files = std::collections::HashMap::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|_| Error::Remote("the result zip is damaged".into()))?;
        if f.size() > MAX_ENTRY_BYTES {
            return Err(Error::Remote("a file in the result is too large".into()));
        }
        let full = f.name().map(|n| n.to_string()).unwrap_or_default();
        let name = full.rsplit('/').next().unwrap_or("").to_string();
        let mut data = Vec::with_capacity(f.size() as usize);
        f.by_ref().take(MAX_ENTRY_BYTES).read_to_end(&mut data).map_err(|_| Error::Remote("the result zip is damaged".into()))?;
        files.insert(name, data);
    }
    Ok(files)
}

/// Reads an inpaint result: `manifest.json` plus `fill.png`.
fn parse_inpainting(zip_bytes: &[u8]) -> Result<Inpainting> {
    let files = read_zip(zip_bytes)?;
    let manifest: serde_json::Value = serde_json::from_slice(files.get("manifest.json").ok_or_else(|| Error::Remote("the result has no manifest".into()))?).map_err(|_| Error::Remote("the manifest is not valid".into()))?;
    let png = files.get("fill.png").ok_or_else(|| Error::Remote("the result has no fill image".into()))?.clone();
    let dim = |k: &str| manifest["input"][k].as_u64().unwrap_or(0) as u32;
    Ok(Inpainting { width: dim("width"), height: dim("height"), model: manifest["model"].as_str().unwrap_or("").chars().take(60).collect(), png })
}

/// Reads a result archive: `manifest.json` plus the layer files it names.
fn parse_decomposition(zip_bytes: &[u8]) -> Result<Decomposition> {
    let files = read_zip(zip_bytes)?;
    let manifest: serde_json::Value = serde_json::from_slice(files.get("manifest.json").ok_or_else(|| Error::Remote("the result has no manifest".into()))?).map_err(|_| Error::Remote("the manifest is not valid".into()))?;
    let mode = manifest["mode"].as_str().and_then(DecomposeMode::parse).unwrap_or_default();
    let (width, height) = (manifest["generated"]["width"].as_u64().unwrap_or(0) as u32, manifest["generated"]["height"].as_u64().unwrap_or(0) as u32);
    let entries = manifest["layers"].as_array().ok_or_else(|| Error::Remote("the manifest lists no layers".into()))?;
    let mut layers = Vec::with_capacity(entries.len());
    for (i, e) in entries.iter().enumerate() {
        let file = e["file"].as_str().ok_or_else(|| Error::Remote("the manifest names no file for a layer".into()))?;
        let png = files.get(file.rsplit('/').next().unwrap_or("")).ok_or_else(|| Error::Remote(format!("layer file {file} is missing")))?.clone();
        let bbox = e["bbox"].as_array().map(|a| [0, 1, 2, 3].map(|k| a.get(k).and_then(serde_json::Value::as_i64).unwrap_or(0) as i32)).unwrap_or([0; 4]);
        layers.push(DecomposedLayer {
            z: e["z"].as_i64().unwrap_or(i as i64) as i32,
            caption: e["caption"].as_str().filter(|c| !c.trim().is_empty()).map(|c| c.chars().take(200).collect()),
            bbox,
            alpha_coverage: e["alpha_coverage"].as_f64().unwrap_or(0.0) as f32,
            png,
        });
    }
    if layers.is_empty() {
        return Err(Error::Remote("the result has no layers".into()));
    }
    layers.sort_by_key(|l| l.z);
    Ok(Decomposition { mode, width, height, composite_error: manifest["composite_error"].as_f64().map(|e| e as f32), layers })
}

/// The job id of a `202` answer, checked to be safe in a URL path.
fn job_id_of(body: &[u8]) -> Result<String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| Error::Remote("unexpected answer".into()))?;
    let id = v["job_id"].as_str().filter(|id| !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).ok_or_else(|| Error::Remote("the service returned no job id".into()))?;
    Ok(id.to_string())
}

// ---------------------------------------------------------------------------------------------
// client

/// An HTTP verb of the few the service uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    Get,
    Post,
    Delete,
}

/// A client for one [`RemoteConfig`].
pub struct RemoteClient {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    cfg: RemoteConfig,
}

impl RemoteClient {
    pub fn new(cfg: RemoteConfig) -> Self {
        Self { cfg }
    }

    /// The base URL (no key).
    pub fn base_url(&self) -> &str {
        &self.cfg.base_url
    }

    /// One request: `(status, body)`. `body` is `(content type, bytes)`; `timeout_s` bounds the
    /// whole exchange.
    #[cfg(not(target_arch = "wasm32"))]
    fn send(&self, method: Method, path: &str, body: Option<(String, &[u8])>, timeout_s: u64) -> Result<(u16, Vec<u8>)> {
        use std::time::Duration;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(3)))
            .timeout_global(Some(Duration::from_secs(timeout_s)))
            .http_status_as_error(false)
            .build()
            .into();
        let url = format!("{}{path}", self.cfg.base_url);
        // Transport errors describe the connection, never the request headers.
        let fail = |e: ureq::Error| Error::Remote(format!("request failed: {e}"));
        let mut resp = match (method, body) {
            (Method::Get, _) => agent.get(&url).header("X-API-Key", &self.cfg.api_key).call(),
            (Method::Delete, _) => agent.delete(&url).header("X-API-Key", &self.cfg.api_key).call(),
            (Method::Post, Some((ct, bytes))) => agent.post(&url).header("X-API-Key", &self.cfg.api_key).content_type(ct).send(bytes),
            (Method::Post, None) => agent.post(&url).header("X-API-Key", &self.cfg.api_key).send_empty(),
        }
        .map_err(fail)?;
        let status = resp.status().as_u16();
        let bytes = resp.body_mut().with_config().limit(MAX_ENTRY_BYTES).read_to_vec().map_err(|e| Error::Remote(format!("reading the answer failed: {e}")))?;
        Ok((status, bytes))
    }

    #[cfg(target_arch = "wasm32")]
    fn send(&self, _: Method, _: &str, _: Option<(String, &[u8])>, _: u64) -> Result<(u16, Vec<u8>)> {
        let _ = (BOUNDARY, Method::Get, Method::Post, Method::Delete);
        Err(Error::Unsupported)
    }

    /// Like [`Self::send`], turning any non-2xx status into a clean error.
    fn ok(&self, method: Method, path: &str, body: Option<(String, &[u8])>, timeout_s: u64) -> Result<Vec<u8>> {
        let (status, bytes) = self.send(method, path, body, timeout_s)?;
        if !(200..300).contains(&status) {
            return Err(Error::Remote(format!("HTTP {status} {}", problem_title(&bytes)).trim().to_string()));
        }
        Ok(bytes)
    }

    fn form(&self, parts: &[Part<'_>]) -> (String, Vec<u8>) {
        (format!("multipart/form-data; boundary={BOUNDARY}"), multipart(BOUNDARY, parts))
    }

    /// Segments `image` (JPEG or PNG bytes) with `prompts` given in that image's pixels.
    pub fn segment(&self, image: &[u8], mime: &str, prompts: &[SegmentPrompt]) -> Result<Vec<RemoteMask>> {
        let prompts = prompts_json(prompts)?;
        let (ct, body) = self.form(&[Part { name: "image", file: Some(("image", mime)), data: image }, Part { name: "prompts", file: None, data: prompts.as_bytes() }]);
        let out = self.ok(Method::Post, "/segment", Some((ct, &body)), 30)?;
        parse_segment(&String::from_utf8_lossy(&out))
    }

    /// What the service offers; also the connection test (checks the URL, the key and the scopes).
    pub fn info(&self) -> Result<ServerInfo> {
        let out = self.ok(Method::Get, "/models", None, 15)?;
        let v: serde_json::Value = serde_json::from_slice(&out).map_err(|_| Error::Remote("unexpected answer".into()))?;
        let models = v["models"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).collect()).unwrap_or_default();
        let listed = |k: &str| -> Vec<String> { v["inpaint_models"][k].as_array().map(|a| a.iter().filter_map(|m| m.as_str().filter(|m| method_is_valid(m)).map(str::to_string)).collect()).unwrap_or_default() };
        let mut fill_methods = Vec::new();
        if !listed("fast").is_empty() {
            fill_methods.push("fast".to_string());
        }
        fill_methods.extend(listed("generative"));
        Ok(ServerInfo { api_version: v["api_version"].as_str().unwrap_or("").to_string(), models, queue_length: v["queue_length"].as_u64().unwrap_or(0) as u32, fill_methods, prompt_enhance: v["prompt_enhance"].as_bool().unwrap_or(false) })
    }

    /// Starts a decomposition of `image` (JPEG or PNG bytes); returns the job id.
    pub fn submit_decompose(&self, image: &[u8], mime: &str, p: &DecomposeParams) -> Result<String> {
        let (layers, resolution, steps, cfg) = (p.layers.to_string(), p.resolution.to_string(), p.steps.to_string(), p.cfg.to_string());
        let seed = p.seed.map(|s| s.to_string());
        let mut parts = vec![
            Part { name: "image", file: Some(("image", mime)), data: image },
            Part { name: "layers", file: None, data: layers.as_bytes() },
            Part { name: "resolution", file: None, data: resolution.as_bytes() },
            Part { name: "steps", file: None, data: steps.as_bytes() },
            Part { name: "cfg", file: None, data: cfg.as_bytes() },
            Part { name: "mode", file: None, data: p.mode.name().as_bytes() },
        ];
        if let Some(s) = &seed {
            parts.push(Part { name: "seed", file: None, data: s.as_bytes() });
        }
        if !p.prompt.trim().is_empty() {
            parts.push(Part { name: "prompt", file: None, data: p.prompt.trim().as_bytes() });
        }
        let (ct, body) = self.form(&parts);
        let out = self.ok(Method::Post, "/decompose", Some((ct, &body)), 120)?;
        job_id_of(&out)
    }

    /// Starts filling the masked area of `image` (JPEG or PNG bytes). `mask` is an 8-bit greyscale
    /// PNG of the same size (255 = fill). Returns the job id.
    pub fn submit_inpaint(&self, image: &[u8], mime: &str, mask: &[u8], p: &InpaintParams) -> Result<String> {
        if !method_is_valid(&p.method) {
            return Err(Error::BadInput("the fill method is not a valid model name".into()));
        }
        let fast = p.method == "fast";
        let (dilate, steps) = (p.dilate.to_string(), p.steps.to_string());
        let seed = p.seed.map(|s| s.to_string());
        let mut parts = vec![
            Part { name: "image", file: Some(("image", mime)), data: image },
            Part { name: "mask", file: Some(("mask", "image/png")), data: mask },
            Part { name: "mode", file: None, data: if fast { b"fast" } else { b"generative" } },
            Part { name: "dilate", file: None, data: dilate.as_bytes() },
        ];
        let context = p.context.filter(|c| c.is_finite()).map(|c| format!("{:.3}", c.clamp(0.0, 1.0)));
        let refine = p.refine.filter(|r| r.is_finite() && *r > 0.0).map(|r| format!("{:.3}", r.clamp(0.0, 1.0)));
        if !fast {
            parts.push(Part { name: "model", file: None, data: p.method.as_bytes() });
            parts.push(Part { name: "steps", file: None, data: steps.as_bytes() });
            if let Some(c) = &context {
                parts.push(Part { name: "context", file: None, data: c.as_bytes() });
            }
            if let Some(r) = &refine {
                parts.push(Part { name: "refine", file: None, data: r.as_bytes() });
            }
        }
        if let Some(s) = &seed {
            parts.push(Part { name: "seed", file: None, data: s.as_bytes() });
        }
        if !p.prompt.trim().is_empty() {
            parts.push(Part { name: "prompt", file: None, data: p.prompt.trim().as_bytes() });
        }
        let (ct, body) = self.form(&parts);
        let out = self.ok(Method::Post, "/inpaint", Some((ct, &body)), 120)?;
        job_id_of(&out)
    }

    /// Asks the service to rewrite `prompt` (a few words describing what to paint) into a fuller
    /// one for the image model. Needs [`ServerInfo::prompt_enhance`]. Only the text is sent.
    pub fn enhance_prompt(&self, prompt: &str) -> Result<String> {
        let text = prompt.trim();
        if text.is_empty() || text.chars().count() > 500 {
            return Err(Error::BadInput("the prompt must be 1 to 500 characters".into()));
        }
        let body = serde_json::json!({ "prompt": text, "task": "inpaint" }).to_string();
        let out = self.ok(Method::Post, "/prompt/enhance", Some(("application/json".to_string(), body.as_bytes())), 60)?;
        let v: serde_json::Value = serde_json::from_slice(&out).map_err(|_| Error::Remote("unexpected answer".into()))?;
        let better = v["prompt"].as_str().map(str::trim).filter(|p| !p.is_empty() && p.chars().count() <= 2000).ok_or_else(|| Error::Remote("the service returned no prompt".into()))?;
        // The generative models take at most 500 characters.
        Ok(better.chars().take(500).collect())
    }

    /// The result of a succeeded inpaint job.
    pub fn fetch_inpainting(&self, id: &str) -> Result<Inpainting> {
        parse_inpainting(&self.ok(Method::Get, &format!("/jobs/{id}/result"), None, 300)?)
    }

    /// The state of a job. `id` comes from [`Self::submit_decompose`].
    pub fn job_status(&self, id: &str) -> Result<JobStatus> {
        parse_status(&self.ok(Method::Get, &format!("/jobs/{id}"), None, 20)?)
    }

    /// The result of a succeeded job.
    pub fn fetch_decomposition(&self, id: &str) -> Result<Decomposition> {
        parse_decomposition(&self.ok(Method::Get, &format!("/jobs/{id}/result"), None, 300)?)
    }

    /// Cancels (or, once finished, deletes) a job. Idempotent.
    pub fn cancel_job(&self, id: &str) -> Result<()> {
        self.ok(Method::Delete, &format!("/jobs/{id}"), None, 20).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_only_travel_over_https_or_loopback() {
        assert!(RemoteConfig::new("https://example.com/v1/", "k").is_some());
        assert!(RemoteConfig::new("http://127.0.0.1:8700/v1", "k").is_some());
        assert!(RemoteConfig::new("http://localhost/v1", "k").is_some());
        assert!(RemoteConfig::new("http://example.com/v1", "k").is_none());
        assert!(RemoteConfig::new("http://127.0.0.1.evil.com/v1", "k").is_none());
        assert!(RemoteConfig::new("ftp://x", "k").is_none());
        assert!(RemoteConfig::new("https://example.com", "  ").is_none());
        assert_eq!(RemoteConfig::new("https://example.com/v1///", "k").map(|c| c.base_url), Some("https://example.com/v1".into()));
        assert!(RemoteConfig::url_is_allowed(" https://example.com ") && !RemoteConfig::url_is_allowed("http://example.com"));
    }

    #[test]
    fn the_key_is_never_printed() {
        let c = RemoteConfig::new("https://example.com", "pc_secret_value").unwrap();
        assert!(!format!("{c:?}").contains("pc_secret_value"));
    }

    #[test]
    fn prompts_become_the_services_json() {
        let j: serde_json::Value = serde_json::from_str(&prompts_json(&[SegmentPrompt::Point { x: 10.0, y: 20.5, positive: true }, SegmentPrompt::Box { x0: 50.0, y0: 40.0, x1: 5.0, y1: 6.0 }]).unwrap()).unwrap();
        assert_eq!(j["points"][0]["x"], 10.0);
        assert_eq!(j["points"][0]["positive"], true);
        assert_eq!(j["box"], serde_json::json!([5.0, 6.0, 50.0, 40.0]));
        assert_eq!(j["multimask"], false);
        assert!(prompts_json(&[]).is_err());
        assert!(prompts_json(&[SegmentPrompt::Point { x: f32::NAN, y: 0.0, positive: true }]).is_err());
    }

    #[test]
    fn multipart_has_all_parts() {
        let b = String::from_utf8(multipart("B", &[Part { name: "image", file: Some(("image", "image/jpeg")), data: b"IMG" }, Part { name: "layers", file: None, data: b"4" }])).unwrap();
        assert!(b.contains("name=\"image\"; filename=\"image\"") && b.contains("Content-Type: image/jpeg") && b.contains("IMG"));
        assert!(b.contains("name=\"layers\"\r\n\r\n4\r\n") && b.ends_with("--B--\r\n"));
    }

    #[test]
    fn answers_are_parsed_and_junk_is_an_error() {
        let png = base64::engine::general_purpose::STANDARD.encode(b"PNGDATA");
        let m = parse_segment(&format!(r#"{{"masks":[{{"png_base64":"{png}","score":0.9}}],"width":8,"height":6}}"#)).unwrap();
        assert_eq!((m[0].png.as_slice(), m[0].width, m[0].height), (&b"PNGDATA"[..], 8, 6));
        assert!(parse_segment("not json").is_err());
        assert!(parse_segment(r#"{"masks":[{"png_base64":"@@"}]}"#).is_err());
    }

    #[test]
    fn job_status_is_parsed() {
        let s = parse_status(br#"{"job_id":"x","state":"running","queue_position":null,"step":12,"steps":40,"eta_seconds":95.5,"error":null}"#).unwrap();
        assert_eq!((s.state, s.step, s.steps, s.queue_position, s.eta_seconds), (JobState::Running, 12, 40, None, Some(95.5)));
        assert!(!s.state.is_final());
        let f = parse_status(br#"{"state":"failed","error":{"title":"Out of memory","status":500}}"#).unwrap();
        assert!(f.state.is_final() && f.error.as_deref() == Some("Out of memory"));
        let q = parse_status(br#"{"state":"queued","queue_position":2}"#).unwrap();
        assert_eq!(q.queue_position, Some(2));
        assert!(parse_status(br#"{"state":"weird"}"#).is_err() && parse_status(b"junk").is_err());
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn manifest(mode: &str) -> String {
        format!(
            r#"{{"mode":"{mode}","generated":{{"width":8,"height":6}},"composite_error":0.034,"layers":[
                {{"index":1,"file":"layer_01.png","z":1,"bbox":[1,1,5,5],"alpha_coverage":0.3,"caption":"inflatable clown"}},
                {{"index":0,"file":"layer_00.png","z":0,"bbox":[0,0,8,6],"alpha_coverage":1.0}}]}}"#
        )
    }

    #[test]
    fn a_result_zip_becomes_layers_back_to_front() {
        let z = zip_of(&[("manifest.json", manifest("generative").as_bytes()), ("layer_00.png", b"BG"), ("layer_01.png", b"CLOWN")]);
        let d = parse_decomposition(&z).unwrap();
        assert_eq!((d.mode, d.width, d.height), (DecomposeMode::Generative, 8, 6));
        assert_eq!(d.composite_error, Some(0.034));
        assert_eq!(d.layers.iter().map(|l| (l.z, l.png.as_slice())).collect::<Vec<_>>(), vec![(0, &b"BG"[..]), (1, &b"CLOWN"[..])]);
        assert_eq!(d.layers[1].caption.as_deref(), Some("inflatable clown"));
        assert_eq!(d.layers[1].bbox, [1, 1, 5, 5]);
        assert_eq!(d.layers[0].caption, None);
    }

    #[test]
    fn bad_result_zips_are_errors_not_panics() {
        assert!(parse_decomposition(b"not a zip").is_err());
        assert!(parse_decomposition(&zip_of(&[("layer_00.png", b"x")])).is_err(), "no manifest");
        assert!(parse_decomposition(&zip_of(&[("manifest.json", manifest("mask").as_bytes()), ("layer_00.png", b"x")])).is_err(), "a listed file is missing");
        assert!(parse_decomposition(&zip_of(&[("manifest.json", br#"{"layers":[]}"#)])).is_err(), "no layers");
        // Names are lookup keys only: a path-like name cannot escape.
        let z = zip_of(&[("manifest.json", manifest("mask").as_bytes()), ("../../x/layer_00.png", b"A"), ("layer_01.png", b"B")]);
        assert_eq!(parse_decomposition(&z).unwrap().mode, DecomposeMode::Mask);
    }

    #[test]
    fn an_inpaint_result_zip_gives_the_fill_image() {
        let m = br#"{"kind":"inpaint","model":"big-lama","mode":"fast","input":{"width":8,"height":6},"filled_fraction":0.1}"#;
        let r = parse_inpainting(&zip_of(&[("manifest.json", m), ("fill.png", b"FILL")])).unwrap();
        assert_eq!((r.width, r.height, r.model.as_str(), r.png.as_slice()), (8, 6, "big-lama", &b"FILL"[..]));
        assert!(parse_inpainting(&zip_of(&[("manifest.json", m)])).is_err(), "no fill image");
        assert!(parse_inpainting(&zip_of(&[("fill.png", b"x")])).is_err(), "no manifest");
        assert!(parse_inpainting(b"nope").is_err());
    }

    #[test]
    fn fill_methods_are_safe_names() {
        for ok in ["fast", "sdxl", "juggernaut", "fooocus-v2", "model_1.5"] {
            assert!(method_is_valid(ok), "{ok}");
        }
        for bad in ["", "a b", "x/../y", "a;b", "é", &"m".repeat(41)] {
            assert!(!method_is_valid(bad), "{bad}");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod http {
        use super::*;
        use std::io::{Read, Write};
        use std::net::TcpListener;

        /// One scripted answer.
        struct Reply {
            status: &'static str,
            content_type: &'static str,
            body: Vec<u8>,
        }

        fn json(status: &'static str, body: impl Into<String>) -> Reply {
            Reply { status, content_type: "application/json", body: body.into().into_bytes() }
        }

        /// A loopback HTTP server that answers the next requests with `replies`, in order, and returns
        /// each request's head (request line and headers) and body when joined.
        /// Each request's head and body, as the server saw them.
        type Seen = Vec<(String, Vec<u8>)>;

        fn serve(replies: Vec<Reply>) -> (String, std::thread::JoinHandle<Seen>) {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/photocraft/v1", l.local_addr().unwrap());
            let h = std::thread::spawn(move || {
                let mut seen = Vec::new();
                for r in replies {
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
                    write!(s, "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", r.status, r.content_type, r.body.len()).unwrap();
                    s.write_all(&r.body).unwrap();
                    seen.push((head, got[head_end..].to_vec()));
                }
                seen
            });
            (url, h)
        }

        fn client(url: &str) -> RemoteClient {
            RemoteClient::new(RemoteConfig::new(url, "pc_test_key").unwrap())
        }

        #[test]
        fn segment_round_trip_against_a_local_server() {
            let png = base64::engine::general_purpose::STANDARD.encode(b"MASK");
            let (url, h) = serve(vec![json("200 OK", format!(r#"{{"masks":[{{"png_base64":"{png}","score":0.5,"bbox":[0,0,1,1]}}],"width":4,"height":3,"model":"sam2_large"}}"#))]);
            let m = client(&url).segment(b"JPEGBYTES", "image/jpeg", &[SegmentPrompt::Point { x: 1.0, y: 2.0, positive: true }]).unwrap();
            assert_eq!(m[0].png, b"MASK");
            let (head, body) = &h.join().unwrap()[0];
            let head = head.to_ascii_lowercase();
            assert!(head.starts_with("post /photocraft/v1/segment") && head.contains("x-api-key: pc_test_key") && head.contains("multipart/form-data"));
            assert!(String::from_utf8_lossy(body).contains("JPEGBYTES"));
        }

        #[test]
        fn server_errors_become_clean_errors_without_the_key() {
            let (url, h) = serve(vec![json("401 Unauthorized", r#"{"title":"Invalid API key","status":401}"#)]);
            let e = client(&url).segment(b"X", "image/jpeg", &[SegmentPrompt::Point { x: 1.0, y: 2.0, positive: true }]).unwrap_err().to_string();
            h.join().unwrap();
            assert!(e.contains("401") && e.contains("Invalid API key") && !e.contains("pc_test_key"), "{e}");
        }

        #[test]
        fn the_connection_test_reads_models_and_queue() {
            let (url, h) = serve(vec![json("200 OK", r#"{"api_version":"1","models":[{"id":"Qwen/Qwen-Image-Layered","kind":"decompose"},{"id":"facebook/sam2.1-hiera-large","kind":"segment"}],"queue_length":2}"#)]);
            let i = client(&url).info().unwrap();
            assert_eq!(i, ServerInfo { api_version: "1".into(), models: vec!["Qwen/Qwen-Image-Layered".into(), "facebook/sam2.1-hiera-large".into()], queue_length: 2, fill_methods: vec![], prompt_enhance: false });
            assert!(h.join().unwrap()[0].0.starts_with("GET /photocraft/v1/models"));
        }

        #[test]
        fn the_connection_test_lists_the_fill_methods() {
            let (url, h) = serve(vec![json("200 OK", r#"{"api_version":"1","models":[],"inpaint_models":{"fast":["big-lama"],"generative":["sdxl","juggernaut","bad name"]},"queue_length":0}"#)]);
            assert_eq!(client(&url).info().unwrap().fill_methods, vec!["fast", "sdxl", "juggernaut"]);
            h.join().unwrap();
        }

        #[test]
        fn an_inpaint_is_submitted_with_the_chosen_model_and_fetched() {
            let m = br#"{"model":"juggernaut","input":{"width":4,"height":3}}"#;
            let z = zip_of(&[("manifest.json", m), ("fill.png", b"FILL")]);
            let (url, h) = serve(vec![json("202 Accepted", r#"{"job_id":"01JFILL"}"#), Reply { status: "200 OK", content_type: "application/zip", body: z }, json("202 Accepted", r#"{"job_id":"01JFAST"}"#)]);
            let c = client(&url);
            let p = InpaintParams { method: "juggernaut".into(), steps: 20, prompt: " empty road ".into(), seed: Some(7), context: Some(0.4), refine: Some(0.5), ..Default::default() };
            let id = c.submit_inpaint(b"JPEG", "image/jpeg", b"MASKPNG", &p).unwrap();
            assert_eq!(id, "01JFILL");
            assert_eq!(c.fetch_inpainting(&id).unwrap().model, "juggernaut");
            c.submit_inpaint(b"JPEG", "image/jpeg", b"MASKPNG", &InpaintParams::default()).unwrap();
            let seen = h.join().unwrap();
            let (head, body) = &seen[0];
            let body = String::from_utf8_lossy(body);
            assert!(head.starts_with("POST /photocraft/v1/inpaint"), "{head}");
            for want in ["name=\"context\"\r\n\r\n0.400", "name=\"refine\"\r\n\r\n0.500", "name=\"mask\"; filename=\"mask\"", "MASKPNG", "name=\"mode\"\r\n\r\ngenerative", "name=\"model\"\r\n\r\njuggernaut", "name=\"steps\"\r\n\r\n20", "name=\"seed\"\r\n\r\n7", "empty road"] {
                assert!(body.contains(want), "{want} in {body}");
            }
            let fast = String::from_utf8_lossy(&seen[2].1).to_string();
            assert!(fast.contains("name=\"mode\"\r\n\r\nfast") && !fast.contains("name=\"model\"") && !fast.contains("name=\"steps\"") && !fast.contains("name=\"context\"") && !fast.contains("name=\"refine\""), "{fast}");
        }

        #[test]
        fn a_prompt_is_enhanced_by_the_service_and_capped() {
            let long = "x".repeat(900);
            let (url, h) = serve(vec![json("200 OK", r#"{"prompt":"  an empty tiled kitchen floor, soft daylight, photo  "}"#), json("200 OK", format!(r#"{{"prompt":"{long}"}}"#)), json("200 OK", r#"{"prompt":" "}"#)]);
            let c = client(&url);
            assert_eq!(c.enhance_prompt(" empty floor ").unwrap(), "an empty tiled kitchen floor, soft daylight, photo");
            assert_eq!(c.enhance_prompt("floor").unwrap().chars().count(), 500, "capped for the image model");
            assert!(c.enhance_prompt("floor").is_err(), "an empty answer is an error");
            let seen = h.join().unwrap();
            assert!(seen[0].0.starts_with("POST /photocraft/v1/prompt/enhance") && seen[0].0.to_ascii_lowercase().contains("application/json"));
            assert!(String::from_utf8_lossy(&seen[0].1).contains("empty floor"));
            assert!(c.enhance_prompt("  ").is_err() && c.enhance_prompt(&"y".repeat(501)).is_err(), "bad input never reaches the network");
        }

        #[test]
        fn a_bad_fill_method_is_refused_before_sending() {
            let c = client("http://127.0.0.1:1/photocraft/v1");
            let p = InpaintParams { method: "x y".into(), ..Default::default() };
            assert!(c.submit_inpaint(b"J", "image/jpeg", b"M", &p).is_err());
        }

        #[test]
        fn a_decomposition_is_submitted_polled_fetched_and_cancellable() {
            let z = zip_of(&[("manifest.json", manifest("generative").as_bytes()), ("layer_00.png", b"BG"), ("layer_01.png", b"CLOWN")]);
            let (url, h) = serve(vec![
                json("202 Accepted", r#"{"job_id":"01JTEST","state":"queued","queue_position":0}"#),
                json("200 OK", r#"{"job_id":"01JTEST","state":"running","step":3,"steps":40,"eta_seconds":60}"#),
                Reply { status: "200 OK", content_type: "application/zip", body: z },
                json("204 No Content", ""),
            ]);
            let c = client(&url);
            let p = DecomposeParams { layers: 5, seed: Some(7), prompt: " a living room ".into(), mode: DecomposeMode::Mask, ..Default::default() };
            let id = c.submit_decompose(b"JPEG", "image/jpeg", &p).unwrap();
            assert_eq!(id, "01JTEST");
            assert_eq!(c.job_status(&id).unwrap().step, 3);
            assert_eq!(c.fetch_decomposition(&id).unwrap().layers.len(), 2);
            c.cancel_job(&id).unwrap();
            let seen = h.join().unwrap();
            let form = String::from_utf8_lossy(&seen[0].1).to_string();
            for want in ["name=\"layers\"\r\n\r\n5", "name=\"resolution\"\r\n\r\n640", "name=\"mode\"\r\n\r\nmask", "name=\"seed\"\r\n\r\n7", "name=\"prompt\"\r\n\r\na living room\r\n"] {
                assert!(form.contains(want), "missing {want:?} in {form}");
            }
            assert!(seen[1].0.starts_with("GET /photocraft/v1/jobs/01JTEST ") && seen[2].0.starts_with("GET /photocraft/v1/jobs/01JTEST/result") && seen[3].0.starts_with("DELETE /photocraft/v1/jobs/01JTEST"));
            assert!(seen.iter().all(|(h, _)| h.to_ascii_lowercase().contains("x-api-key: pc_test_key")));
        }

        #[test]
        fn a_job_id_with_odd_characters_is_refused() {
            let (url, h) = serve(vec![json("202 Accepted", r#"{"job_id":"../../etc/passwd"}"#)]);
            assert!(client(&url).submit_decompose(b"X", "image/jpeg", &DecomposeParams::default()).is_err());
            h.join().unwrap();
        }
    }
}
