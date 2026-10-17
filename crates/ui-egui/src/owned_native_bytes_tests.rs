//! UNRUN browser native family routing and typed Session adoption drafts.
use super::*;
use crate::Services;
use std::{cell::RefCell, io::Cursor, rc::Rc};
#[path = "../../xcf/tests/common/mod.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod xcf;

fn native_bytes() -> Vec<u8> {
    let imported = photocraft_io::import_owned_staged_plain_bytes(
        "saved-source.xcf",
        Arc::new(xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        &Interrupt::NONE,
    )
    .unwrap();
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let owner = imported.document().try_with_document_name("Saved current title", &ctl).unwrap();
    // Serialize actual accounted BackedRaster storage, not an unqualified
    // resident Raster fixture. The native Session class requires real backing.
    photocraft_format::owned_document::save_current_document_archive(owner.document(), Cursor::new(Vec::new()), Default::default(), owner.quota(), &ctl)
        .unwrap()
        .into_inner()
}
fn capture(request: Request, bytes: &[u8]) -> Capture {
    let mut builder = request.begin_capture("different-browser-label.pcraft", bytes.len()).unwrap();
    for part in bytes.chunks(65_536) {
        builder.append(part).unwrap();
    }
    builder.finish().unwrap()
}
#[test]
fn native_policy_selects_one_picker_keeps_saved_title_and_reads_after_pending_cancellation() {
    let bytes = native_bytes();
    let seen = Rc::new(RefCell::new(None));
    let observed = seen.clone();
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services {
            owned_native_bytes_picker: Some(Box::new(move |request, reply| {
                assert_eq!(request.extension(), ".pcraft");
                *observed.borrow_mut() = Some(request.clone());
                reply.send(Ok(Some(capture(request, &bytes))));
                None
            })),
            owned_xcf_bytes_picker: Some(Box::new(|_, _| panic!("native policy must not try XCF"))),
            owned_native_open: Some(Box::new(|_, _, _| panic!("byte picker must not use path service"))),
            ..Default::default()
        },
    );
    crate::owned_native_open::invoke(&mut app, &json!({"nativeProfilePolicy":"keepCurrent"})).unwrap();
    assert!(app.file_dialog_open());
    assert!(start(&mut app).is_err());
    poll(&mut app, &egui::Context::default());
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let state = app.session.active().unwrap();
    assert_eq!(state.doc.name, "Saved current title");
    assert!(state.path.is_none());
    assert!(state.source.as_ref().unwrap().path.is_none());
    assert!(app.ui.recent_files.is_empty());
    let request = seen.borrow();
    let request = request.as_ref().unwrap();
    assert!(request.work.cancelled.load(Ordering::Relaxed));
    assert!(state.doc.accounted().unwrap().quota().shares_ledger(&request.quota));
    let photocraft_doc::LayerContent::BackedRaster(layer) = &state.doc.layers[0].content else { panic!() };
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let mut raw = [0; 4];
    layer.checked_view().read_logical_pixel_native_into(0, 0, &mut raw, &ctl).unwrap();
    assert_eq!(raw, [31, 61, 91, 255]);
    assert!(!app.file_dialog_open());
}
#[test]
fn native_capture_suffix_corruption_and_cross_request_never_fall_back_or_publish() {
    let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 512 << 20,
        index: 64 << 20,
        scratch: 0,
    });
    let request = Request::with_family(q, Family::Native).unwrap();
    assert!(request.clone().begin_capture("other.xcf", 8).is_err());
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services {
            owned_native_bytes_picker: Some(Box::new(move |_, reply| {
                reply.send(Ok(Some(capture(request.clone(), &[1; 8]))));
                None
            })),
            owned_xcf_bytes_picker: Some(Box::new(|_, _| panic!("no decoder fallback"))),
            ..Default::default()
        },
    );
    start_native(&mut app).unwrap();
    poll(&mut app, &egui::Context::default());
    assert!(app.ui.status_error);
    assert!(app.session.documents().is_empty());
    app.services.owned_native_bytes_picker = Some(Box::new(|request, reply| {
        reply.send(Ok(Some(capture(request, &[1; 8]))));
        None
    }));
    start_native(&mut app).unwrap();
    poll(&mut app, &egui::Context::default());
    assert!(app.ui.status_error);
    assert!(app.session.documents().is_empty());
}
#[test]
fn scoped_automation_and_explicit_paths_cannot_borrow_interactive_picker_authority() {
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services { owned_native_bytes_picker: Some(Box::new(|_, _| panic!("must refuse before picker"))), ..Default::default() },
    );
    app.automation_input = true;
    assert!(crate::owned_native_open::invoke(&mut app, &json!({"nativeProfilePolicy":"keepCurrent"})).is_err());
    app.automation_input = false;
    assert!(crate::owned_native_open::invoke(&mut app, &json!({"path":"ungranted.pcraft","nativeProfilePolicy":"keepCurrent"})).is_err());
    assert!(app.session.documents().is_empty());
    assert!(!app.file_dialog_open());
}
