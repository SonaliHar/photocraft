use super::*;
use crate::ai::testing::{FakeProvider, request};
use crate::{EngineError, Session, jobs::JobCtx};
use std::sync::Arc;

#[test]
fn ai_submit_poll_result_happy_path() {
    let p = FakeProvider { polls: 2, ..Default::default() };
    assert_eq!(p.tasks().unwrap(), vec![AiTask::FillMaskedArea]);
    let req = request(PixelFormat::RGBA32F);
    let id = p.submit(&req).unwrap();
    assert!(matches!(p.poll(&id).unwrap(), AiJobState::Queued { .. }));
    assert!(matches!(p.poll(&id).unwrap(), AiJobState::Running { .. }));
    assert_eq!(p.poll(&id).unwrap(), AiJobState::Done);
    let r = p.result(&id).unwrap();
    assert_eq!(r.pixels, vec![0.25, 0.25, 0.25, 0.25, 0.5, 0.5, 0.5, 0.25, 0.75, 0.75, 0.75, 0.25]);
    assert_eq!(req.pixels, vec![0.25; 12]);
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["model"], "solid-fill");
    assert_eq!(json["device"], "CPU");
    assert_eq!(json["provider"], "local fake");
    assert_eq!(json["elapsed_ms"], 12);
    assert_eq!((r.model.as_str(), r.device.as_str(), r.provider.as_str(), r.elapsed_ms), ("solid-fill", "CPU", "local fake", 12));
}

#[test]
fn ai_completion_progress_reaches_one_at_all_depths_and_layouts() {
    for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAY8, PixelFormat::CMYKA8] {
        let p = FakeProvider { polls: 3, ..Default::default() };
        let ctx = JobCtx::new();
        let req = request(format);
        let r = run_to_completion(&p, &req, &ctx, Duration::ZERO).unwrap();
        assert_eq!(ctx.fraction(), 1.0);
        assert_eq!(ctx.message(), "AI job complete");
        assert_eq!(r.format, format);
        assert_eq!(r.window, req.window);
    }
}

#[test]
fn ai_cancel_mid_job_calls_provider_even_when_cancel_fails() {
    for cancel_fails in [false, true] {
        let ctx = JobCtx::new();
        let p = FakeProvider { never_finish: true, polls: 4, cancel_at: Some((2, ctx.clone())), cancel_fails, ..Default::default() };
        assert_eq!(run_to_completion(&p, &request(PixelFormat::RGBA8), &ctx, Duration::ZERO), Err(AiError::Cancelled));
        assert!(p.state.lock().unwrap().cancelled);
        assert!(ctx.fraction() > 0.0 && ctx.fraction() < 1.0);
    }
}

#[test]
fn ai_already_cancelled_does_not_submit() {
    let p = FakeProvider::default();
    let ctx = JobCtx::new();
    ctx.cancel();
    assert_eq!(run_to_completion(&p, &request(PixelFormat::RGBA8), &ctx, Duration::ZERO), Err(AiError::Cancelled));
    assert!(p.state.lock().unwrap().request.is_none());
    assert!(matches!(EngineError::from(AiError::Cancelled), EngineError::Cancelled));
}

#[test]
fn ai_failure_message_passes_through() {
    let p = FakeProvider { fail: Some("The model could not run on this device".into()), ..Default::default() };
    let e = run_to_completion(&p, &request(PixelFormat::RGBA8), &JobCtx::new(), Duration::ZERO).unwrap_err();
    assert_eq!(e.to_string(), "The model could not run on this device");
    assert_eq!(EngineError::from(e).to_string(), "The model could not run on this device");
}

#[test]
fn ai_provider_cancelled_is_cancelled() {
    let p = FakeProvider { remote_cancelled: true, ..Default::default() };
    assert_eq!(run_to_completion(&p, &request(PixelFormat::RGBA8), &JobCtx::new(), Duration::ZERO), Err(AiError::Cancelled));
}

#[test]
fn ai_wrong_size_and_format_are_errors() {
    for wrong_size in [true, false] {
        let p = FakeProvider { wrong_size, wrong_format: !wrong_size, ..Default::default() };
        let ctx = JobCtx::new();
        assert!(run_to_completion(&p, &request(PixelFormat::RGBA8), &ctx, Duration::ZERO).is_err());
        assert_ne!(ctx.fraction(), 1.0);
    }
}

#[test]
fn ai_no_provider_message_and_session_round_trip() {
    for mut s in [Session::new(), Session::default()] {
        let e = s.ai_provider().ok_or(AiError::NoProvider).err().unwrap();
        assert_eq!(e.to_string(), "Craft AI Manager isn't running");
        assert_eq!(EngineError::from(e).to_string(), "Craft AI Manager isn't running");
        let p: Arc<dyn AiProvider> = Arc::new(FakeProvider::default());
        s.set_ai_provider(Some(p.clone()));
        assert!(Arc::ptr_eq(&p, &s.ai_provider().unwrap()));
        s.set_ai_provider(None);
        assert!(s.ai_provider().is_none());
    }
}

#[test]
fn ai_wire_task_and_opaque_id_round_trip() {
    assert_eq!(AiTask::FillMaskedArea.id(), "fill_masked_area");
    assert_eq!(serde_json::to_string(&AiTask::FillMaskedArea).unwrap(), "\"fill_masked_area\"");
    let id = AiJobId::new("provider/opaque:42");
    assert_eq!(serde_json::from_str::<AiJobId>(&serde_json::to_string(&id).unwrap()).unwrap(), id);
}

#[test]
fn ai_invalid_requests_fail_before_submit() {
    let base = request(PixelFormat::RGBA8);
    let mut cases = Vec::new();
    let mut r = base.clone();
    r.window = Rect::EMPTY;
    cases.push(r);
    let mut r = base.clone();
    r.window = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    cases.push(r);
    let mut r = base.clone();
    r.pixels.pop();
    cases.push(r);
    let mut r = base.clone();
    r.coverage.pop();
    cases.push(r);
    for coverage in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        let mut r = base.clone();
        r.coverage[0] = coverage;
        cases.push(r);
    }
    for req in cases {
        let p = FakeProvider::default();
        assert!(run_to_completion(&p, &req, &JobCtx::new(), Duration::ZERO).is_err());
        assert!(p.state.lock().unwrap().request.is_none());
    }
}

#[test]
fn ai_wrong_window_is_an_error() {
    let p = FakeProvider { wrong_window: true, ..Default::default() };
    assert!(run_to_completion(&p, &request(PixelFormat::RGBA8), &JobCtx::new(), Duration::ZERO).is_err());
}

#[test]
fn ai_poll_reports_intermediate_progress() {
    let ctx = JobCtx::new();
    let p = FakeProvider { polls: 4, never_finish: true, cancel_at: Some((3, ctx.clone())), ..Default::default() };
    assert_eq!(run_to_completion(&p, &request(PixelFormat::RGBA8), &ctx, Duration::ZERO), Err(AiError::Cancelled));
    assert_eq!(ctx.fraction(), 0.5);
    assert_eq!(ctx.message(), "Filling");
}

#[test]
fn ai_provider_call_errors_pass_through() {
    for call_error in ["submit", "poll", "result"] {
        let p = FakeProvider { call_error: Some(call_error), ..Default::default() };
        let e = run_to_completion(&p, &request(PixelFormat::RGBA8), &JobCtx::new(), Duration::ZERO).unwrap_err();
        assert_eq!(e.to_string(), "The provider call failed");
    }
}

#[test]
fn ai_cancellation_during_failed_poll_still_calls_cancel() {
    let ctx = JobCtx::new();
    let p = FakeProvider { call_error: Some("poll"), cancel_at: Some((1, ctx.clone())), ..Default::default() };
    assert_eq!(run_to_completion(&p, &request(PixelFormat::RGBA8), &ctx, Duration::ZERO), Err(AiError::Cancelled));
    assert!(p.state.lock().unwrap().cancelled);
}
