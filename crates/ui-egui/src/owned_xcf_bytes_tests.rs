//! UNRUN explicit browser capture/adoption/authority regression drafts.
use super::*;
use crate::Services;
use photocraft_raster::backing::quota::{QuotaLimits, QuotaUsage};
use std::{cell::RefCell, rc::Rc};
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 512 << 20, index: 64 << 20, scratch: 0 })
}
fn captured(request: Request, bytes: &[u8]) -> Capture {
    let mut builder = request.begin_capture("browser-original.xcf", bytes.len()).unwrap();
    for part in bytes.chunks(65_536) {
        builder.append(part).unwrap();
    }
    builder.finish().unwrap()
}
fn fixture() -> Vec<u8> {
    xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])
}
#[test]
fn explicit_byte_picker_uses_real_catalog_and_preserves_original_without_path_authority() {
    let raw = Arc::new(fixture());
    let seen = Rc::new(RefCell::new(None));
    let observed = seen.clone();
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services {
            owned_xcf_bytes_picker: Some(Box::new(move |request, reply| {
                *observed.borrow_mut() = Some(request.quota.clone());
                reply.send(Ok(Some(captured(request, &raw))));
                None
            })),
            owned_current_xcf_open: Some(Box::new(|_, _, _| panic!("path reader must not be used"))),
            ..Default::default()
        },
    );
    let response = crate::owned_xcf_open::invoke(&mut app, &json!({"xcfProfilePolicy":"keepCurrent"})).unwrap();
    assert_eq!(response["fileDialog"], "open");
    assert!(app.session.documents().is_empty());
    assert!(app.file_dialog_open());
    poll(&mut app, &egui::Context::default());
    let state = app.session.active().unwrap();
    assert!(state.source_read_only);
    assert!(state.path.is_none());
    assert!(state.source.as_ref().unwrap().path.is_none());
    assert!(state.doc.accounted().unwrap().quota().shares_ledger(seen.borrow().as_ref().unwrap()));
    let photocraft_doc::LayerContent::BackedRaster(layer) = &state.doc.layers[0].content else { panic!() };
    let mut pixel = [0; 4];
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    layer.checked_view().read_logical_pixel_native_into(0, 0, &mut pixel, &ctl).unwrap();
    assert_eq!(pixel, [31, 61, 91, 255]);
    assert!(app.ui.recent_files.is_empty());
    assert!(!app.file_dialog_open());
}
#[test]
fn capture_caps_cumulative_work_consumed_builder_and_failure_cleanup_are_real() {
    let q = quota();
    let request = Request::new(q.clone()).unwrap();
    let baseline = q.usage().unwrap();
    assert!(request.clone().begin_capture("x.xcf", 0).is_err());
    assert!(request.clone().begin_capture("x.xcf", CAP + 1).is_err());
    assert!(request.clone().begin_capture("bad\0.xcf", 1).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let mut builder = request.clone().begin_capture("x.xcf", 2).unwrap();
    builder.append(&[1]).unwrap();
    assert!(builder.finish().is_err());
    builder.append(&[2]).unwrap();
    let capture = builder.finish().unwrap();
    let held = q.usage().unwrap();
    assert!(builder.append(&[3]).is_err());
    assert!(builder.finish().is_err());
    assert_eq!(q.usage().unwrap(), held);
    assert_eq!(capture.bytes.as_slice(), &[1, 2]);
    drop(builder);
    drop(capture);
    assert_eq!(q.usage().unwrap(), baseline);
    request.work.used.store(WORK, Ordering::Relaxed);
    assert!(request.clone().begin_capture("x.xcf", 1).is_err());
    drop(request);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn cancellation_and_provider_failure_never_retry_path_capability_or_adopt() {
    let q = quota();
    let request = Request::new(q.clone()).unwrap();
    let baseline = q.usage().unwrap();
    let mut builder = request.clone().begin_capture("x.xcf", 1).unwrap();
    request.work.cancelled.store(true, Ordering::Relaxed);
    assert!(builder.append(&[1]).is_err());
    drop(builder);
    assert_eq!(q.usage().unwrap(), baseline);
    drop(request);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services {
            owned_xcf_bytes_picker: Some(Box::new(|_, reply| {
                reply.send(Err("provider refused".into()));
                None
            })),
            owned_current_xcf_open: Some(Box::new(|_, _, _| panic!("must not retry after byte refusal"))),
            ..Default::default()
        },
    );
    start(&mut app).unwrap();
    poll(&mut app, &egui::Context::default());
    assert!(app.session.documents().is_empty());
    assert!(app.ui.status_error);
    assert_eq!(app.ui.status, "provider refused");
    app.automation_input = true;
    assert!(crate::owned_xcf_open::invoke(&mut app, &json!({"xcfProfilePolicy":"keepCurrent"})).is_err());
}
#[test]
fn catalog_change_and_simultaneous_picker_refuse_before_publication() {
    let waiting = Rc::new(RefCell::new(None));
    let sink = waiting.clone();
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services {
            owned_xcf_bytes_picker: Some(Box::new(move |request, reply| {
                *sink.borrow_mut() = Some((request, reply));
                None
            })),
            ..Default::default()
        },
    );
    start(&mut app).unwrap();
    assert!(start(&mut app).is_err());
    let other = captured(Request::new(quota()).unwrap(), &fixture());
    apply_xcf(&mut app, &other).unwrap();
    let old = app.session.active().unwrap().doc.clone();
    let (request, reply) = waiting.borrow_mut().take().unwrap();
    reply.send(Ok(Some(captured(request, &fixture()))));
    poll(&mut app, &egui::Context::default());
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.session.active().unwrap().doc.shares_state(&old));
    assert!(app.ui.status_error);
}

#[test]
fn pending_drop_signals_same_work_then_invokes_host_abort_and_releases_capture() {
    let aborted = Rc::new(std::cell::Cell::new(0));
    let observed = aborted.clone();
    let actual_quota = Rc::new(RefCell::new(None));
    let seen = actual_quota.clone();
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services {
            owned_xcf_bytes_picker: Some(Box::new(move |request, reply| {
                *seen.borrow_mut() = Some(request.quota.clone());
                let work = request.clone();
                let mut builder = request.begin_capture("pending.xcf", 1).unwrap();
                let done = observed.clone();
                Some(Box::new(move || {
                    assert!(work.work.cancelled.load(Ordering::Relaxed));
                    assert!(builder.append(&[0]).is_err());
                    done.set(done.get() + 1);
                    drop(builder);
                    drop(reply);
                }))
            })),
            ..Default::default()
        },
    );
    start(&mut app).unwrap();
    assert!(actual_quota.borrow().as_ref().unwrap().usage().unwrap().resident > 65_536);
    app.owned_xcf_bytes_pending = None;
    assert_eq!(aborted.get(), 1);
    assert!(app.session.documents().is_empty());
    let header = BackingQuota::aggregate_header_bytes();
    assert_eq!(actual_quota.borrow().as_ref().unwrap().usage().unwrap(), QuotaUsage { resident: header, index: header, scratch: 0 });
    drop(app);
    assert_eq!(aborted.get(), 1);
}
