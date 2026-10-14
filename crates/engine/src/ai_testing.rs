//! The shared AI test provider: it fills the request with a solid colour and can be told to
//! fail, stall, return bad data or be cancelled. `ai::tests` and the commands that call a
//! provider (Generative Fill) both use it.

use std::sync::{Mutex, PoisonError};

use super::*;
use crate::jobs::JobCtx;

#[derive(Default)]
pub(crate) struct FakeState {
    pub(crate) request: Option<AiRequest>,
    pub(crate) polls: usize,
    pub(crate) cancelled: bool,
}

#[derive(Default)]
pub(crate) struct FakeProvider {
    pub(crate) state: Mutex<FakeState>,
    pub(crate) polls: usize,
    pub(crate) fail: Option<String>,
    pub(crate) never_finish: bool,
    pub(crate) wrong_size: bool,
    pub(crate) wrong_format: bool,
    pub(crate) wrong_window: bool,
    pub(crate) remote_cancelled: bool,
    pub(crate) cancel_at: Option<(usize, JobCtx)>,
    pub(crate) cancel_fails: bool,
    pub(crate) call_error: Option<&'static str>,
    /// Put a NaN sample in the result: provider pixels are outside our input, so callers check.
    pub(crate) bad_pixel: bool,
}

impl AiProvider for FakeProvider {
    fn tasks(&self) -> Result<Vec<AiTask>, AiError> {
        Ok(vec![AiTask::FillMaskedArea])
    }
    fn submit(&self, req: &AiRequest) -> Result<AiJobId, AiError> {
        if self.call_error == Some("submit") {
            return Err(AiError::Other("The provider call failed".into()));
        }
        req.validate()?;
        let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        *s = FakeState { request: Some(req.clone()), ..Default::default() };
        Ok(AiJobId::new("fake-job"))
    }
    fn poll(&self, id: &AiJobId) -> Result<AiJobState, AiError> {
        assert_eq!(id.as_str(), "fake-job");
        let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        s.polls += 1;
        if let Some((n, ctx)) = &self.cancel_at
            && s.polls == *n
        {
            ctx.cancel();
        }
        if self.call_error == Some("poll") {
            return Err(AiError::Other("The provider call failed".into()));
        }
        if let Some(message) = &self.fail {
            return Ok(AiJobState::Failed(message.clone()));
        }
        if s.cancelled || self.remote_cancelled {
            return Ok(AiJobState::Cancelled);
        }
        if !self.never_finish && s.polls > self.polls {
            return Ok(AiJobState::Done);
        }
        let progress = s.polls as f32 / self.polls.max(1) as f32;
        if s.polls == 1 {
            Ok(AiJobState::Queued { progress, message: "Queued".into() })
        } else {
            Ok(AiJobState::Running { progress, message: "Filling".into() })
        }
    }
    fn result(&self, id: &AiJobId) -> Result<AiResult, AiError> {
        if self.call_error == Some("result") {
            return Err(AiError::Other("The provider call failed".into()));
        }
        assert_eq!(id.as_str(), "fake-job");
        let s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let req = s.request.as_ref().ok_or_else(|| AiError::Other("No job was submitted".into()))?;
        let mut pixels = req.pixels.clone();
        // Solid 0.75 in every colour channel; preserve alpha and blend soft coverage.
        for (pixel, coverage) in pixels.chunks_exact_mut(req.format.channels()).zip(&req.coverage) {
            for sample in pixel.iter_mut().take(req.format.mode.color_channels()) {
                *sample += (0.75 - *sample) * coverage;
            }
        }
        if self.bad_pixel
            && let Some(sample) = pixels.first_mut()
        {
            *sample = f32::NAN;
        }
        if self.wrong_size {
            pixels.pop();
        }
        Ok(AiResult {
            window: if self.wrong_window { req.window.translate(1, 0) } else { req.window },
            pixels,
            format: if self.wrong_format { PixelFormat::GRAY8 } else { req.format },
            model: "solid-fill".into(),
            device: "CPU".into(),
            provider: "local fake".into(),
            elapsed_ms: 12,
        })
    }
    fn cancel(&self, id: &AiJobId) -> Result<(), AiError> {
        assert_eq!(id.as_str(), "fake-job");
        self.state.lock().unwrap_or_else(PoisonError::into_inner).cancelled = true;
        if self.cancel_fails { Err(AiError::Other("Cancellation could not reach the provider".into())) } else { Ok(()) }
    }
}

pub(crate) fn request(format: PixelFormat) -> AiRequest {
    AiRequest {
        task: AiTask::FillMaskedArea,
        window: Rect::new(-2, 3, 1, 4),
        pixels: vec![0.25; 3 * format.channels()],
        format,
        coverage: vec![0.0, 0.5, 1.0],
        prompt: None,
        client_version: "photocraft/test".into(),
    }
}
