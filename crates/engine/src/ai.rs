//! Backend-neutral AI jobs. Providers run blocking calls on a job worker; no network lives here.
use std::time::Duration;

use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use serde::{Deserialize, Serialize};

/// Stable task identifiers shared with providers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiTask {
    #[serde(rename = "fill_masked_area")]
    FillMaskedArea,
}

impl AiTask {
    pub const fn id(self) -> &'static str {
        match self {
            Self::FillMaskedArea => "fill_masked_area",
        }
    }
}

/// A row-major window in document coordinates. Samples are interleaved, straight-alpha
/// f32 in the document's colour space; PixelFormat retains the original storage depth.
/// Coverage has one finite value in 0..=1 per pixel. No colour conversion is implied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AiRequest {
    pub task: AiTask,
    pub window: Rect,
    pub pixels: Vec<f32>,
    pub format: PixelFormat,
    pub coverage: Vec<f32>,
    pub prompt: Option<String>,
    pub client_version: String,
}

/// Provider-owned identifier: callers must not interpret its contents.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AiJobId(String);

impl AiJobId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Pending states carry progress and a plain status message. Done implies progress 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AiJobState {
    Queued { progress: f32, message: String },
    Running { progress: f32, message: String },
    Done,
    Failed(String),
    Cancelled,
}

/// Pixels for exactly the requested window, plus execution provenance (B08).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AiResult {
    pub window: Rect,
    pub pixels: Vec<f32>,
    pub format: PixelFormat,
    pub model: String,
    pub device: String,
    /// Identifies where the job ran, including local/cloud provider identity.
    pub provider: String,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AiError {
    #[error("Craft AI Manager isn't running")]
    NoProvider,
    #[error("The AI job was cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

impl From<AiError> for crate::EngineError {
    fn from(error: AiError) -> Self {
        match error {
            AiError::Cancelled => Self::Cancelled,
            other => Self::Other(other.to_string()),
        }
    }
}

/// Blocking, object-safe backend contract. Implementations validate their own inputs and
/// return errors for unknown jobs; calls must not panic. Cancellation is best-effort.
pub trait AiProvider: Send + Sync {
    fn tasks(&self) -> Result<Vec<AiTask>, AiError>;
    fn submit(&self, req: &AiRequest) -> Result<AiJobId, AiError>;
    fn poll(&self, id: &AiJobId) -> Result<AiJobState, AiError>;
    fn result(&self, id: &AiJobId) -> Result<AiResult, AiError>;
    fn cancel(&self, id: &AiJobId) -> Result<(), AiError>;
}

fn sample_count(window: Rect, format: PixelFormat) -> Result<(usize, usize), AiError> {
    let invalid = || AiError::Other("The AI window is empty or too large".into());
    if window.is_empty() {
        return Err(invalid());
    }
    let pixels = usize::try_from(window.size().area()).map_err(|_| invalid())?;
    let samples = pixels.checked_mul(format.channels()).filter(|n| *n <= isize::MAX as usize / size_of::<f32>()).ok_or_else(invalid)?;
    Ok((pixels, samples))
}

impl AiRequest {
    /// Check geometry and buffer layout before a provider receives the request.
    pub fn validate(&self) -> Result<(), AiError> {
        let (pixels, samples) = sample_count(self.window, self.format)?;
        if self.pixels.len() != samples || self.coverage.len() != pixels {
            return Err(AiError::Other("The AI image or selection size does not match its window".into()));
        }
        if self.coverage.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(AiError::Other("The AI selection coverage must be between zero and one".into()));
        }
        Ok(())
    }
}

/// Submit and wait on a worker thread. Zero poll interval supports deterministic fast tests.
/// Native waits check cancellation at least every 10 ms. On wasm pending jobs return an
/// unsupported error (and request cancellation), since blocking polling cannot yield to JS.
pub fn run_to_completion(provider: &dyn AiProvider, req: &AiRequest, ctx: &crate::jobs::JobCtx, poll_every: Duration) -> Result<AiResult, AiError> {
    if ctx.cancelled() {
        return Err(AiError::Cancelled);
    }
    req.validate()?;
    let id = provider.submit(req)?;
    loop {
        check_cancel(provider, &id, ctx)?;
        let state = provider.poll(&id);
        check_cancel(provider, &id, ctx)?;
        match state? {
            AiJobState::Queued { progress, message } | AiJobState::Running { progress, message } => ctx.progress(progress, &message),
            AiJobState::Done => {
                let result = provider.result(&id);
                check_cancel(provider, &id, ctx)?;
                let result = result?;
                if result.window != req.window || result.format != req.format || result.pixels.len() != req.pixels.len() {
                    return Err(AiError::Other("The AI result size or format does not match the requested window".into()));
                }
                ctx.progress(1.0, "AI job complete");
                return Ok(result);
            }
            AiJobState::Failed(message) => return Err(AiError::Other(message)),
            AiJobState::Cancelled => return Err(AiError::Cancelled),
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut remaining = poll_every;
            while !remaining.is_zero() {
                check_cancel(provider, &id, ctx)?;
                let step = remaining.min(Duration::from_millis(10));
                std::thread::sleep(step);
                remaining = remaining.saturating_sub(step);
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = poll_every;
            let _ = provider.cancel(&id);
            return Err(AiError::Other("Waiting for an AI provider is not available in the web build".into()));
        }
    }
}

fn check_cancel(provider: &dyn AiProvider, id: &AiJobId, ctx: &crate::jobs::JobCtx) -> Result<(), AiError> {
    if ctx.cancelled() {
        // Even if the remote cancellation fails, discard the result locally.
        let _ = provider.cancel(id);
        return Err(AiError::Cancelled);
    }
    Ok(())
}

#[cfg(test)]
#[path = "ai_testing.rs"]
pub(crate) mod testing;
#[cfg(test)]
#[path = "ai_tests.rs"]
mod tests;
