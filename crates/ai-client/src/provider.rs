//! [`CaimProvider`]: the engine's `AiProvider` spoken over the loopback job protocol
//! (SPEC > Job protocol v1, amendment 11). Blocking by design - the engine runs it on a job worker.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use photocraft_color::PixelFormat;
use photocraft_engine::ai::{AiError, AiJobId, AiJobState, AiProvider, AiRequest, AiResult, AiTask};
use photocraft_geom::Rect;

use crate::endpoint::{self, Endpoint};
use crate::http;
use crate::pixels;
use crate::protocol;

/// How many jobs one provider remembers. Each record is small (a window and a format), but a
/// caller that stopped tracking jobs must not grow the map forever.
const MAX_TRACKED_JOBS: usize = 16;

/// What a job needs to be polled, fetched and cancelled: the manager it went to, and the window it
/// described. The endpoint is kept with the job because the token changes on every manager start.
#[derive(Clone, Debug)]
struct JobRecord {
    endpoint: Endpoint,
    window: Rect,
    format: PixelFormat,
    samples: usize,
    sequence: u64,
}

/// Talks to Craft AI Manager. Cheap to construct: the endpoint file is read per `submit`, so a
/// manager that restarts (new port, new token) is picked up without restarting PhotoCraft.
pub struct CaimProvider {
    endpoint_file: Option<PathBuf>,
    jobs: Mutex<HashMap<String, JobRecord>>,
    next_sequence: AtomicU64,
}

impl CaimProvider {
    /// Reads `CRAFT_AI_ENDPOINT` if it names a file, otherwise the platform default. A platform
    /// with nowhere to put the file simply has no manager.
    pub fn new() -> Self {
        Self { endpoint_file: endpoint::endpoint_file_from_env(), jobs: Mutex::new(HashMap::new()), next_sequence: AtomicU64::new(1) }
    }

    /// Which endpoint file to read. Tests point this at a per-test file instead of setting the
    /// environment variable, so they can run in parallel.
    pub fn with_endpoint_file(path: impl Into<PathBuf>) -> Self {
        Self { endpoint_file: Some(path.into()), ..Self::new() }
    }

    /// The path this provider reads, when the platform has one.
    pub fn endpoint_file(&self) -> Option<&std::path::Path> {
        self.endpoint_file.as_deref()
    }

    fn endpoint(&self) -> Result<Endpoint, AiError> {
        match self.endpoint_file.as_deref() {
            Some(path) => endpoint::load(path),
            None => Err(AiError::NoProvider),
        }
    }

    /// One request, with a non-success status mapped to an error before the caller sees the body.
    fn call(
        &self,
        endpoint: &Endpoint,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: &[u8],
        max_body_bytes: usize,
    ) -> Result<http::Reply, AiError> {
        let reply = http::request(endpoint, method, path, content_type, body, max_body_bytes)?;
        if reply.is_success() {
            return Ok(reply);
        }
        if reply.status == 401 {
            return Err(AiError::Other("Craft AI Manager refused the request (token mismatch); restart Craft AI Manager".into()));
        }
        Err(AiError::Other(protocol::error_sentence(reply.status, &reply.body)))
    }

    fn get(&self, endpoint: &Endpoint, path: &str, max_body_bytes: usize) -> Result<http::Reply, AiError> {
        self.call(endpoint, "GET", path, None, &[], max_body_bytes)
    }

    fn record(&self, id: &AiJobId) -> Result<JobRecord, AiError> {
        let jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        jobs.get(id.as_str()).cloned().ok_or_else(|| AiError::Other("PhotoCraft has no record of that AI job; it was not started by this session.".into()))
    }

    fn remember(&self, id: &str, record: JobRecord) {
        let mut jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        while jobs.len() + 1 > MAX_TRACKED_JOBS {
            // Evict the oldest job rather than the newest: the newest is the one still in use.
            match jobs.iter().min_by_key(|(_, job)| job.sequence).map(|(id, _)| id.clone()) {
                Some(oldest) => {
                    jobs.remove(&oldest);
                }
                None => break,
            }
        }
        jobs.insert(id.to_string(), record);
    }
}

impl Default for CaimProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AiProvider for CaimProvider {
    fn tasks(&self) -> Result<Vec<AiTask>, AiError> {
        let endpoint = self.endpoint()?;
        let reply = self.get(&endpoint, "/v1/tasks", http::MAX_JSON_BODY_BYTES)?;
        protocol::parse_tasks(&reply.body)
    }

    fn submit(&self, req: &AiRequest) -> Result<AiJobId, AiError> {
        req.validate()?;
        let size = req.window.size();
        // Encode before connecting: a document the manager cannot handle should fail without a
        // network round trip, and every size is checked before anything large is allocated.
        let channels = pixels::channels(req.format)?;
        let depth = pixels::bit_depth(req.format);
        let job = protocol::job_json(req, depth, channels as u8)?;
        let image = pixels::encode_image(&req.pixels, req.format, size.width, size.height)?;
        let mask = pixels::encode_mask(&req.coverage, size.width, size.height)?;
        let parts = [
            protocol::Part { name: "job", filename: Some("job.json"), content_type: "application/json", data: &job },
            protocol::Part { name: "image", filename: Some("image.png"), content_type: "image/png", data: &image },
            protocol::Part { name: "mask", filename: Some("mask.png"), content_type: "image/png", data: &mask },
        ];
        let boundary = protocol::boundary(&parts)?;
        let body = protocol::multipart(&boundary, &parts);
        // Re-read the file every time: a manager that restarted has a new port and a new token.
        let endpoint = self.endpoint()?;
        let content_type = format!("multipart/form-data; boundary={boundary}");
        let reply = self.call(&endpoint, "POST", "/v1/jobs", Some(&content_type), &body, http::MAX_JSON_BODY_BYTES)?;
        let id = protocol::parse_job_id(&reply.body)?;
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        self.remember(&id, JobRecord { endpoint, window: req.window, format: req.format, samples: req.pixels.len(), sequence });
        Ok(AiJobId::new(id))
    }

    fn poll(&self, id: &AiJobId) -> Result<AiJobState, AiError> {
        let record = self.record(id)?;
        let path = protocol::job_path(id.as_str())?;
        let reply = self.get(&record.endpoint, &path, http::MAX_JSON_BODY_BYTES)?;
        protocol::parse_state(&reply.body)
    }

    fn result(&self, id: &AiJobId) -> Result<AiResult, AiError> {
        let record = self.record(id)?;
        let base = protocol::job_path(id.as_str())?;
        let size = record.window.size();
        let provenance = {
            let reply = self.get(&record.endpoint, &format!("{base}/result"), http::MAX_JSON_BODY_BYTES)?;
            protocol::parse_provenance(&reply.body)?
        };
        if let Some([width, height]) = provenance.image_size
            && (width != size.width || height != size.height)
        {
            return Err(AiError::Other("Craft AI Manager says its result is a different size from the area PhotoCraft sent.".into()));
        }
        let cap = pixels::result_body_cap(size.width, size.height, record.format)?;
        let reply = self.get(&record.endpoint, &format!("{base}/result.png"), cap)?;
        if !reply.content_type.unwrap_or_default().starts_with("image/png") {
            return Err(AiError::Other("Craft AI Manager's result image is not a PNG.".into()));
        }
        let pixels = pixels::decode_image(&reply.body, record.format, size.width, size.height)?;
        if pixels.len() != record.samples {
            return Err(AiError::Other("Craft AI Manager's result image does not have the samples PhotoCraft asked for.".into()));
        }
        Ok(AiResult {
            window: record.window,
            pixels,
            format: record.format,
            model: provenance.model,
            device: provenance.device,
            provider: provenance.provider,
            elapsed_ms: provenance.elapsed_ms,
        })
    }

    fn cancel(&self, id: &AiJobId) -> Result<(), AiError> {
        // The job's own endpoint, not whatever the file says now: cancelling a job that has since
        // moved to another manager would be worse than doing nothing.
        let record = self.record(id)?;
        let path = protocol::job_path(id.as_str())?;
        self.call(&record.endpoint, "DELETE", &path, None, &[], http::MAX_JSON_BODY_BYTES)?;
        Ok(())
    }
}
