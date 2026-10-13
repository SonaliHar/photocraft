//! Generative Fill providers.
//!
//! Every provider turns the same request (an RGB crop, a mask of the pixels to generate and a
//! prompt) into its own HTTP call and returns the encoded image it answers with. The user picks
//! the provider and brings the credentials; nothing is sent anywhere until Generate is pressed,
//! and only the crop around the selection leaves the machine.
//!
//! | id           | service                                   | cost                         |
//! |--------------|-------------------------------------------|------------------------------|
//! | `cloudflare` | Workers AI, Stable Diffusion 1.5 inpaint  | free daily allowance         |
//! | `sdwebui`    | AUTOMATIC1111 / Forge on this machine     | free (your GPU)              |
//! | `gemini`     | Google Gemini image model                 | per image, free tier varies  |
//! | `openai`     | OpenAI `images/edits` (or compatible)     | per image                    |
//! | `stability`  | Stability AI inpaint                      | credits                      |

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::http::{self, Multipart};
use crate::{MlError, Result};

/// What to put in the masked area when the user types nothing (object removal).
pub const REMOVE_PROMPT: &str = "Remove what is in the masked area and fill it so it blends seamlessly with the surrounding scene";

const TIMEOUT: Duration = Duration::from_secs(240);

/// The supported services.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Cloudflare,
    #[serde(rename = "sdwebui")]
    SdWebUi,
    Gemini,
    #[serde(rename = "openai")]
    OpenAi,
    Stability,
}

impl Provider {
    pub const ALL: [Provider; 5] = [Provider::Cloudflare, Provider::SdWebUi, Provider::Gemini, Provider::OpenAi, Provider::Stability];

    pub fn id(self) -> &'static str {
        match self {
            Provider::Cloudflare => "cloudflare",
            Provider::SdWebUi => "sdwebui",
            Provider::Gemini => "gemini",
            Provider::OpenAi => "openai",
            Provider::Stability => "stability",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Cloudflare => "Cloudflare Workers AI (free tier)",
            Provider::SdWebUi => "Local Stable Diffusion (AUTOMATIC1111 / Forge)",
            Provider::Gemini => "Google Gemini",
            Provider::OpenAi => "OpenAI / compatible",
            Provider::Stability => "Stability AI",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Provider::Cloudflare => "https://api.cloudflare.com/client/v4",
            Provider::SdWebUi => "http://127.0.0.1:7860",
            Provider::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            Provider::OpenAi => "https://api.openai.com/v1",
            Provider::Stability => "https://api.stability.ai",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Cloudflare => "@cf/runwayml/stable-diffusion-v1-5-inpainting",
            Provider::SdWebUi => "",
            Provider::Gemini => "gemini-2.5-flash-image",
            Provider::OpenAi => "gpt-image-1",
            Provider::Stability => "",
        }
    }

    /// Does it need an API key?
    pub fn needs_key(self) -> bool {
        !matches!(self, Provider::SdWebUi)
    }

    /// Does it need an account id as well?
    pub fn needs_account(self) -> bool {
        matches!(self, Provider::Cloudflare)
    }

    /// Longest side (pixels) the crop is resized to before it is sent.
    pub fn default_side(self) -> u32 {
        match self {
            Provider::Cloudflare | Provider::SdWebUi => 512,
            _ => 1024,
        }
    }

    fn env_prefix(self) -> String {
        format!("PHOTOCRAFT_{}", self.id().to_ascii_uppercase())
    }
}

/// One provider's saved settings. Empty fields mean "the default".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProviderConfig {
    pub api_key: String,
    pub account_id: String,
    pub base_url: String,
    pub model: String,
    /// Longest side sent to the service; 0 = the provider's default.
    pub max_side: u32,
}

fn env_or(prefix: &str, suffix: &str, saved: &str) -> String {
    let saved = saved.trim();
    if !saved.is_empty() {
        return saved.to_string();
    }
    std::env::var(format!("{prefix}_{suffix}")).map(|v| v.trim().to_string()).unwrap_or_default()
}

impl ProviderConfig {
    /// The key: the saved one, else `PHOTOCRAFT_<ID>_API_KEY`.
    pub fn key(&self, p: Provider) -> String {
        env_or(&p.env_prefix(), "API_KEY", &self.api_key)
    }

    pub fn account(&self, p: Provider) -> String {
        env_or(&p.env_prefix(), "ACCOUNT_ID", &self.account_id)
    }

    pub fn base(&self, p: Provider) -> String {
        let b = self.base_url.trim().trim_end_matches('/');
        if b.is_empty() { p.default_base_url().to_string() } else { b.to_string() }
    }

    pub fn model_name(&self, p: Provider) -> String {
        let m = self.model.trim();
        if m.is_empty() { p.default_model().to_string() } else { m.to_string() }
    }

    /// What is still missing to use the provider, if anything.
    pub fn missing(&self, p: Provider) -> Option<String> {
        if p.needs_key() && self.key(p).is_empty() {
            return Some(format!(
                "{} needs an API key (set it in Generative Fill > Provider settings, or in the {}_API_KEY environment variable)",
                p.label(),
                p.env_prefix()
            ));
        }
        if p.needs_account() && self.account(p).is_empty() {
            return Some(format!("{} needs your account id (Provider settings, or the {}_ACCOUNT_ID environment variable)", p.label(), p.env_prefix()));
        }
        None
    }

    fn side(&self, p: Provider) -> u32 {
        if self.max_side >= 64 { self.max_side.min(2048) } else { p.default_side() }
    }
}

/// The resolution to send for a `w x h` crop: the same shape, scaled so the longest side is the
/// provider's working side, in multiples of 8 (some services require it); OpenAI's image models
/// accept three fixed sizes, so the nearest shape is used there.
pub fn canvas_for(p: Provider, cfg: &ProviderConfig, w: u32, h: u32) -> (u32, u32) {
    let (w, h) = (w.max(1), h.max(1));
    if p == Provider::OpenAi {
        let ratio = w as f32 / h as f32;
        if !cfg.model_name(p).starts_with("gpt-image") {
            return (1024, 1024);
        }
        return if ratio > 1.2 {
            (1536, 1024)
        } else if ratio < 0.83 {
            (1024, 1536)
        } else {
            (1024, 1024)
        };
    }
    let side = cfg.side(p) as f32;
    let scale = side / w.max(h) as f32;
    let snap = |v: u32| (((v as f32 * scale) / 8.0).round() as u32).max(8) * 8;
    (snap(w).max(64), snap(h).max(64))
}

/// A generation request. `rgb` is `width * height * 3` bytes; `mask` is `width * height` bytes,
/// 255 where new pixels are wanted.
pub struct FillRequest<'a> {
    pub prompt: &'a str,
    pub width: u32,
    pub height: u32,
    pub rgb: &'a [u8],
    pub mask: &'a [u8],
}

fn png(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| MlError::msg(format!("cannot encode the image: {e}")))?;
        w.write_image_data(data).map_err(|e| MlError::msg(format!("cannot encode the image: {e}")))?;
    }
    Ok(out)
}

impl FillRequest<'_> {
    fn check(&self) -> Result<()> {
        let px = (self.width as usize).checked_mul(self.height as usize).ok_or_else(|| MlError::msg("the image is too large"))?;
        if px == 0 || self.rgb.len() != px.saturating_mul(3) || self.mask.len() != px {
            return Err(MlError::msg("internal error: the image and mask sizes disagree"));
        }
        Ok(())
    }

    fn image_png(&self) -> Result<Vec<u8>> {
        png(self.width, self.height, png::ColorType::Rgb, self.rgb)
    }

    /// Grayscale mask, white = generate (Stable Diffusion convention).
    fn mask_white_png(&self) -> Result<Vec<u8>> {
        png(self.width, self.height, png::ColorType::Grayscale, self.mask)
    }

    /// RGBA mask, transparent = generate (OpenAI convention).
    fn mask_alpha_png(&self) -> Result<Vec<u8>> {
        let mut data = Vec::with_capacity(self.mask.len().saturating_mul(4));
        for (m, px) in self.mask.iter().zip(self.rgb.as_chunks::<3>().0) {
            data.extend_from_slice(&[px[0], px[1], px[2], 255 - *m]);
        }
        png(self.width, self.height, png::ColorType::Rgba, &data)
    }

    fn prompt_or_default(&self) -> &str {
        let p = self.prompt.trim();
        if p.is_empty() { REMOVE_PROMPT } else { p }
    }
}

fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

fn unb64(s: &str) -> Result<Vec<u8>> {
    B64.decode(s.trim()).map_err(|e| MlError::msg(format!("the provider returned an unreadable image: {e}")))
}

fn looks_like_image(b: &[u8]) -> bool {
    b.starts_with(&[0x89, b'P', b'N', b'G']) || b.starts_with(&[0xFF, 0xD8]) || (b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP".as_slice()))
}

/// A readable error from a failed reply (providers answer with JSON describing the problem).
fn failure(p: Provider, r: &http::Reply) -> MlError {
    let hint = match r.status {
        401 | 403 => " (check the API key and its permissions)",
        402 | 429 => " (quota or credits exhausted)",
        _ => "",
    };
    MlError::msg(format!("{} refused the request{hint}: {}", p.label(), r.describe()))
}

fn json_body(r: &http::Reply) -> Result<Value> {
    serde_json::from_slice(&r.body).map_err(|e| MlError::msg(format!("the provider's answer was not JSON ({e}): {}", r.describe())))
}

/// Ask `p` to fill the masked area. Returns the encoded image (PNG/JPEG/WebP) the service made;
/// it may differ in size from the request, the caller resizes it.
pub fn generate(p: Provider, cfg: &ProviderConfig, req: &FillRequest<'_>, keep_going: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    req.check()?;
    if let Some(why) = cfg.missing(p) {
        return Err(MlError::msg(why));
    }
    if !keep_going() {
        return Err(MlError::Cancelled);
    }
    let base = cfg.base(p);
    let model = cfg.model_name(p);
    let key = cfg.key(p);
    let bearer = vec![("Authorization", format!("Bearer {key}"))];
    let prompt = req.prompt_or_default();
    match p {
        Provider::Cloudflare => {
            let account = cfg.account(p);
            let url = format!("{base}/accounts/{account}/ai/run/{model}");
            let body = json!({
                "prompt": prompt,
                "image": req.image_png()?,
                "mask": req.mask_white_png()?,
                "width": req.width,
                "height": req.height,
                "num_steps": 20,
                "strength": 1.0,
                "guidance": 7.5,
            });
            let r = http::post(&url, &bearer, "application/json", body.to_string().as_bytes(), TIMEOUT)?;
            if !r.ok() {
                return Err(failure(p, &r));
            }
            if looks_like_image(&r.body) {
                return Ok(r.body);
            }
            // Some models answer {"result": {"image": "<base64>"}}.
            let v = json_body(&r)?;
            let img =
                v.pointer("/result/image").and_then(Value::as_str).ok_or_else(|| MlError::msg(format!("Cloudflare returned no image: {}", r.describe())))?;
            unb64(img)
        }
        Provider::SdWebUi => {
            let mut body = json!({
                "init_images": [b64(&req.image_png()?)],
                "mask": b64(&req.mask_white_png()?),
                "prompt": prompt,
                "negative_prompt": "blurry, artifacts, text, watermark",
                "width": req.width,
                "height": req.height,
                "denoising_strength": 0.85,
                "inpainting_fill": 1,
                "inpaint_full_res": false,
                "mask_blur": 4,
                "steps": 25,
                "cfg_scale": 7,
                "n_iter": 1,
                "batch_size": 1,
            });
            if !model.is_empty() {
                body["override_settings"] = json!({ "sd_model_checkpoint": model });
            }
            let mut headers = Vec::new();
            if !key.is_empty() {
                headers.push(("Authorization", format!("Bearer {key}")));
            }
            let r = http::post(&format!("{base}/sdapi/v1/img2img"), &headers, "application/json", body.to_string().as_bytes(), TIMEOUT)?;
            if !r.ok() {
                return Err(failure(p, &r));
            }
            let v = json_body(&r)?;
            let img = v.pointer("/images/0").and_then(Value::as_str).ok_or_else(|| MlError::msg("the Stable Diffusion server returned no image"))?;
            unb64(img)
        }
        Provider::Gemini => {
            let instruction = format!(
                "You are editing a photo. The first image is the photo. The second image is a mask of the same size: white marks the only area you may change. \
                 Inside the white area: {prompt}. Keep every pixel outside the white area exactly as it is, and return the complete edited photo at the same framing."
            );
            let body = json!({
                "contents": [{ "parts": [
                    { "text": instruction },
                    { "inline_data": { "mime_type": "image/png", "data": b64(&req.image_png()?) } },
                    { "inline_data": { "mime_type": "image/png", "data": b64(&req.mask_white_png()?) } },
                ] }],
                "generationConfig": { "responseModalities": ["IMAGE"] },
            });
            let url = format!("{base}/models/{model}:generateContent");
            let r = http::post(&url, &[("x-goog-api-key", key)], "application/json", body.to_string().as_bytes(), TIMEOUT)?;
            if !r.ok() {
                return Err(failure(p, &r));
            }
            let v = json_body(&r)?;
            let parts = v.pointer("/candidates/0/content/parts").and_then(Value::as_array);
            let data = parts
                .into_iter()
                .flatten()
                .find_map(|part| part.get("inlineData").or_else(|| part.get("inline_data")).and_then(|d| d.get("data")).and_then(Value::as_str));
            match data {
                Some(d) => unb64(d),
                None => {
                    let why = v
                        .pointer("/promptFeedback/blockReason")
                        .and_then(Value::as_str)
                        .map(|b| format!("blocked ({b})"))
                        .unwrap_or_else(|| "it answered with text only".into());
                    Err(MlError::msg(format!("Gemini returned no image: {why}")))
                }
            }
        }
        Provider::OpenAi => {
            let (w, h) = (req.width, req.height);
            let mut form = Multipart::new();
            form.text("model", &model).text("prompt", prompt).text("n", "1").text("size", &format!("{w}x{h}"));
            form.file("image", "image.png", "image/png", &req.image_png()?);
            form.file("mask", "mask.png", "image/png", &req.mask_alpha_png()?);
            let ct = form.content_type();
            let r = http::post(&format!("{base}/images/edits"), &bearer, &ct, &form.finish(), TIMEOUT)?;
            if !r.ok() {
                return Err(failure(p, &r));
            }
            let v = json_body(&r)?;
            if let Some(d) = v.pointer("/data/0/b64_json").and_then(Value::as_str) {
                return unb64(d);
            }
            Err(MlError::msg(format!("the provider returned no image: {}", r.describe())))
        }
        Provider::Stability => {
            let mut form = Multipart::new();
            form.text("prompt", prompt).text("output_format", "png");
            form.file("image", "image.png", "image/png", &req.image_png()?);
            form.file("mask", "mask.png", "image/png", &req.mask_white_png()?);
            let ct = form.content_type();
            let headers = vec![("Authorization", format!("Bearer {key}")), ("Accept", "image/*".to_string())];
            let r = http::post(&format!("{base}/v2beta/stable-image/edit/inpaint"), &headers, &ct, &form.finish(), TIMEOUT)?;
            if !r.ok() {
                return Err(failure(p, &r));
            }
            if looks_like_image(&r.body) {
                return Ok(r.body);
            }
            let v = json_body(&r)?;
            let img = v.get("image").and_then(Value::as_str).ok_or_else(|| MlError::msg("Stability returned no image"))?;
            unb64(img)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// A one-shot HTTP server: records the request, answers with `status` and `body`.
    fn serve(status: u16, content_type: &'static str, body: Vec<u8>) -> (String, mpsc::Receiver<(String, Vec<u8>)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let Ok((mut s, _)) = listener.accept() else { return };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            let (head_end, len) = loop {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..i]).to_ascii_lowercase();
                    let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                    break (i + 4, len);
                }
            };
            while buf.len() < head_end + len {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
            let _ = tx.send((head, buf[head_end..].to_vec()));
            let reply = format!("HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(reply.as_bytes());
            let _ = s.write_all(&body);
        });
        (format!("http://{addr}"), rx)
    }

    fn request<'a>(rgb: &'a [u8], mask: &'a [u8]) -> FillRequest<'a> {
        FillRequest { prompt: "a red balloon", width: 8, height: 8, rgb, mask }
    }

    fn data() -> (Vec<u8>, Vec<u8>) {
        (vec![120u8; 8 * 8 * 3], (0..64).map(|i| if i % 8 < 4 { 255 } else { 0 }).collect())
    }

    fn tiny_png() -> Vec<u8> {
        png(2, 2, png::ColorType::Grayscale, &[0, 64, 128, 255]).unwrap()
    }

    fn cfg(base: &str) -> ProviderConfig {
        ProviderConfig { api_key: "k-123".into(), account_id: "acct".into(), base_url: base.into(), ..Default::default() }
    }

    #[test]
    fn cloudflare_posts_json_and_accepts_raw_png() {
        let (base, rx) = serve(200, "image/png", tiny_png());
        let (rgb, mask) = data();
        let out = generate(Provider::Cloudflare, &cfg(&base), &request(&rgb, &mask), &|| true).unwrap();
        assert!(looks_like_image(&out));
        let (head, body) = rx.recv().unwrap();
        assert!(head.starts_with("POST /accounts/acct/ai/run/@cf/runwayml/stable-diffusion-v1-5-inpainting "), "{head}");
        assert!(head.to_ascii_lowercase().contains("authorization: bearer k-123"));
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["prompt"], "a red balloon");
        assert_eq!(v["width"], 8);
        assert!(v["image"].as_array().is_some_and(|a| a.starts_with(&[json!(0x89), json!(0x50)])), "image is PNG file bytes");
        assert!(v["mask"].is_array());
    }

    #[test]
    fn sdwebui_sends_base64_and_reads_images() {
        let reply = json!({ "images": [b64(&tiny_png())] }).to_string().into_bytes();
        let (base, rx) = serve(200, "application/json", reply);
        let (rgb, mask) = data();
        let c = ProviderConfig { base_url: base, model: "dreamshaper".into(), ..Default::default() };
        let out = generate(Provider::SdWebUi, &c, &request(&rgb, &mask), &|| true).unwrap();
        assert!(looks_like_image(&out));
        let (head, body) = rx.recv().unwrap();
        assert!(head.starts_with("POST /sdapi/v1/img2img "));
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert!(v["init_images"][0].is_string() && v["mask"].is_string());
        assert_eq!(v["override_settings"]["sd_model_checkpoint"], "dreamshaper");
    }

    #[test]
    fn gemini_reads_inline_data_in_either_casing() {
        let reply =
            json!({ "candidates": [{ "content": { "parts": [{ "text": "ok" }, { "inlineData": { "mimeType": "image/png", "data": b64(&tiny_png()) } }] } }] });
        let (base, rx) = serve(200, "application/json", reply.to_string().into_bytes());
        let (rgb, mask) = data();
        let out = generate(Provider::Gemini, &cfg(&base), &request(&rgb, &mask), &|| true).unwrap();
        assert!(looks_like_image(&out));
        let (head, body) = rx.recv().unwrap();
        assert!(head.starts_with("POST /models/gemini-2.5-flash-image:generateContent "), "{head}");
        assert!(head.to_ascii_lowercase().contains("x-goog-api-key: k-123"));
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["contents"][0]["parts"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn openai_sends_multipart_with_an_alpha_mask() {
        let reply = json!({ "data": [{ "b64_json": b64(&tiny_png()) }] });
        let (base, rx) = serve(200, "application/json", reply.to_string().into_bytes());
        let (rgb, mask) = data();
        let out = generate(Provider::OpenAi, &cfg(&base), &request(&rgb, &mask), &|| true).unwrap();
        assert!(looks_like_image(&out));
        let (head, body) = rx.recv().unwrap();
        assert!(head.starts_with("POST /images/edits "), "{head}");
        assert!(head.to_ascii_lowercase().contains("content-type: multipart/form-data; boundary="));
        let text = String::from_utf8_lossy(&body);
        for field in ["name=\"model\"", "name=\"prompt\"", "name=\"image\"", "name=\"mask\"", "gpt-image-1", "a red balloon"] {
            assert!(text.contains(field), "missing {field}");
        }
    }

    #[test]
    fn stability_returns_raw_image_bytes() {
        let (base, rx) = serve(200, "image/png", tiny_png());
        let (rgb, mask) = data();
        let out = generate(Provider::Stability, &cfg(&base), &request(&rgb, &mask), &|| true).unwrap();
        assert!(looks_like_image(&out));
        let (head, _) = rx.recv().unwrap();
        assert!(head.starts_with("POST /v2beta/stable-image/edit/inpaint "));
        assert!(head.to_ascii_lowercase().contains("accept: image/*"));
    }

    #[test]
    fn provider_errors_are_readable() {
        let (base, _rx) = serve(429, "application/json", br#"{"error":"rate limit"}"#.to_vec());
        let (rgb, mask) = data();
        let e = generate(Provider::OpenAi, &cfg(&base), &request(&rgb, &mask), &|| true).unwrap_err().to_string();
        assert!(e.contains("quota") && e.contains("rate limit"), "{e}");
    }

    #[test]
    fn missing_credentials_are_reported_before_any_request() {
        let (rgb, mask) = data();
        let e = generate(Provider::Gemini, &ProviderConfig::default(), &request(&rgb, &mask), &|| true).unwrap_err().to_string();
        assert!(e.contains("API key"), "{e}");
        let c = ProviderConfig { api_key: "x".into(), ..Default::default() };
        let e = generate(Provider::Cloudflare, &c, &request(&rgb, &mask), &|| true).unwrap_err().to_string();
        assert!(e.contains("account id"), "{e}");
    }

    #[test]
    fn mismatched_buffers_are_an_error_not_a_panic() {
        let e =
            generate(Provider::SdWebUi, &ProviderConfig::default(), &FillRequest { prompt: "", width: 8, height: 8, rgb: &[1, 2, 3], mask: &[0] }, &|| true);
        assert!(e.is_err());
    }

    #[test]
    fn cancel_before_sending() {
        let (rgb, mask) = data();
        let e = generate(Provider::SdWebUi, &ProviderConfig::default(), &request(&rgb, &mask), &|| false);
        assert!(matches!(e, Err(MlError::Cancelled)));
    }

    #[test]
    fn canvas_keeps_the_shape_in_multiples_of_eight() {
        let c = ProviderConfig::default();
        assert_eq!(canvas_for(Provider::Cloudflare, &c, 1000, 500), (512, 256));
        let (w, h) = canvas_for(Provider::SdWebUi, &c, 37, 311);
        assert!(w % 8 == 0 && h % 8 == 0 && h == 512 && w >= 64, "{w}x{h}");
        assert_eq!(canvas_for(Provider::OpenAi, &c, 3000, 1000), (1536, 1024));
        assert_eq!(canvas_for(Provider::OpenAi, &c, 900, 1000), (1024, 1024));
        let dalle = ProviderConfig { model: "dall-e-2".into(), ..Default::default() };
        assert_eq!(canvas_for(Provider::OpenAi, &dalle, 3000, 1000), (1024, 1024));
    }

    #[test]
    fn provider_ids_round_trip() {
        for p in Provider::ALL {
            assert_eq!(Provider::parse(p.id()), Some(p));
            assert_eq!(serde_json::to_value(p).unwrap(), json!(p.id()));
        }
        assert_eq!(Provider::parse("nope"), None);
    }
}
