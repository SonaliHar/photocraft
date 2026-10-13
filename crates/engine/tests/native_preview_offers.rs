//! Unrun regression drafts for command-attempt lifetime and job cancellation.
use photocraft_engine::{EngineError, Session, brush_cmds, doc, paint};
use photocraft_raster as raster;
use serde_json::{Value, json};

fn session() -> Session {
    let mut document = doc::Document::new("Preview offer", doc::Size::new(8, 8), doc::ColorMode::Rgb, doc::SampleType::U32);
    document.sample_encoding = Some(doc::SampleEncoding::Linear);
    document.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
    document.layers.push(doc::Layer::raster("Pixels", document.pixel_format()));
    let bounds = document.bounds();
    document.layers.first_mut().unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.5, 0.5, 0.5, 1.0]);
    let mut session = Session::new();
    session.add_document(document, None).unwrap();
    session.tools.brush.pressure_size = false;
    session.tools.brush.smoothing.enabled = false;
    session
}

fn params(x: i32) -> Value {
    json!({"mode":"luminosity", "points":[[x,2,1]], "seed":71, "color":[0.8,0.8,0.8,1],
        "size":3, "flow":1, "smoothing":0, "sampling":"once", "limits":"discontiguous", "tolerance":100})
}

fn offer(session: &mut Session) {
    let preview = brush_cmds::try_native_color_replacement_preview(session, &params(2), &raster::Interrupt::NONE).unwrap().unwrap();
    let commit = preview.finish_for_session(session, paint::StrokeLimits::default(), &raster::Interrupt::NONE).unwrap();
    session.offer_native_color_replacement(commit).unwrap();
}

#[test]
fn unknown_or_malformed_attempt_discards_offer_before_later_different_stroke() {
    for malformed in [false, true] {
        let mut session = session();
        offer(&mut session);
        let before = session.active().unwrap().doc.clone();
        let failure = if malformed {
            session.execute("paint.colorReplacement", json!("invalid envelope"))
        } else {
            session.execute("unknown.preview.command", json!({}))
        };
        assert!(failure.is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        // A retained offer would reject these points as a mismatch. A new
        // authorized stroke must render through the normal registered route.
        assert!(session.execute("paint.colorReplacement", params(5)).is_ok());
    }
}

#[test]
fn rejected_authorization_cannot_retain_pixels_for_later_dispatch() {
    fn deny(_: &str, _: &Value) -> photocraft_engine::Result<()> {
        Err(EngineError::Other("denied by regression policy".into()))
    }
    let mut session = session();
    offer(&mut session);
    let before = session.active().unwrap().doc.clone();
    session.authorize = Some(deny);
    assert!(session.execute("paint.colorReplacement", params(2)).is_err());
    assert!(session.active().unwrap().doc.shares_state(&before));
    session.authorize = None;
    assert!(session.execute("paint.colorReplacement", params(5)).is_ok());
}

#[test]
fn offered_native_result_obeys_inline_job_cancellation() {
    let mut session = session();
    offer(&mut session);
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let context = photocraft_engine::jobs::JobCtx::default();
    context.cancel();
    session.set_inline_job_ctx(Some(context));
    assert!(matches!(session.execute("paint.colorReplacement", params(2)), Err(EngineError::Cancelled)));
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(session.active().unwrap().revision, revision);
    assert!(!session.active().unwrap().history.can_undo());
}
