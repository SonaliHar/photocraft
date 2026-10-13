//! A very small blocking HTTP client (native only) shared by model downloads and providers.

use std::time::Duration;

use crate::{MlError, Result};

/// A response: status and body. Non-2xx statuses are returned, not turned into errors, so the
/// caller can show the provider's own message.
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// A readable failure: status plus the start of the body (providers explain themselves).
    pub fn describe(&self) -> String {
        let text = String::from_utf8_lossy(&self.body);
        let text: String = text.chars().take(400).collect();
        format!("HTTP {}: {}", self.status, text.trim())
    }
}

/// A multipart/form-data body.
#[derive(Default)]
pub struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    pub fn new() -> Self {
        Self { boundary: "----photocraft-7d1f3c2a9b4e".into(), body: Vec::new() }
    }

    pub fn text(&mut self, name: &str, value: &str) -> &mut Self {
        let head = format!("--{}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n", self.boundary);
        self.body.extend_from_slice(head.as_bytes());
        self.body.extend_from_slice(value.as_bytes());
        self.body.extend_from_slice(b"\r\n");
        self
    }

    pub fn file(&mut self, name: &str, file_name: &str, mime: &str, bytes: &[u8]) -> &mut Self {
        let head = format!("--{}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\nContent-Type: {mime}\r\n\r\n", self.boundary);
        self.body.extend_from_slice(head.as_bytes());
        self.body.extend_from_slice(bytes);
        self.body.extend_from_slice(b"\r\n");
        self
    }

    pub fn content_type(&self) -> String {
        format!("multipart/form-data; boundary={}", self.boundary)
    }

    pub fn finish(mut self) -> Vec<u8> {
        let tail = format!("--{}--\r\n", self.boundary);
        self.body.extend_from_slice(tail.as_bytes());
        self.body
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io::Read;

    use super::*;

    /// Largest response body accepted (a generated image, or the model file).
    const MAX_BODY: u64 = 400 * 1024 * 1024;

    fn agent(timeout: Duration) -> ureq::Agent {
        ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).build().into()
    }

    fn net_err(what: &str, e: ureq::Error) -> MlError {
        MlError::msg(format!("{what}: {e}"))
    }

    fn read_body(resp: &mut ureq::http::Response<ureq::Body>) -> Result<Vec<u8>> {
        resp.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| net_err("reading the response", e))
    }

    /// POST `body` with the given content type and extra headers.
    pub fn post(url: &str, headers: &[(&str, String)], content_type: &str, body: &[u8], timeout: Duration) -> Result<Reply> {
        let mut req = agent(timeout).post(url);
        for (k, v) in headers {
            req = req.header(*k, v.as_str());
        }
        let mut resp = req.header("Content-Type", content_type).send(body).map_err(|e| net_err("could not reach the provider", e))?;
        let status = resp.status().as_u16();
        Ok(Reply { status, body: read_body(&mut resp)? })
    }

    /// GET `url` into `sink` in chunks, reporting `(received, total)`; stops when `keep_going`
    /// returns false.
    pub fn download(url: &str, sink: &mut dyn FnMut(&[u8]) -> Result<()>, progress: &dyn Fn(u64, Option<u64>), keep_going: &dyn Fn() -> bool) -> Result<()> {
        let mut resp = agent(Duration::from_secs(3600)).get(url).call().map_err(|e| net_err("download failed", e))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(MlError::msg(format!("download failed: HTTP {status}")));
        }
        let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
        let mut reader = resp.body_mut().with_config().limit(MAX_BODY).reader();
        let mut buf = vec![0u8; 256 * 1024];
        let mut got = 0u64;
        loop {
            if !keep_going() {
                return Err(MlError::Cancelled);
            }
            let n = reader.read(&mut buf).map_err(|e| MlError::msg(format!("download interrupted: {e}")))?;
            if n == 0 {
                break;
            }
            sink(buf.get(..n).unwrap_or_default())?;
            got += n as u64;
            progress(got, total);
        }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{download, post};

#[cfg(target_arch = "wasm32")]
pub fn post(_: &str, _: &[(&str, String)], _: &str, _: &[u8], _: Duration) -> Result<Reply> {
    Err(MlError::msg("network access is not available in the web build"))
}
