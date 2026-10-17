//! UNRUN UI save capability-harness controls. Cursor bytes are external capture;
//! real filesystem atomic/source guards are covered by the Headless child.
use super::*;
#[path = "canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod fixture;
#[path = "../../xcf/tests/common/mod.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod xcf;
use fixture::ctl;
use std::{cell::RefCell, rc::Rc};

#[test]
#[ignore = "UNRUN: actual implicit/assigned Box UI Save promotes one finite work meter before writer, preserves native/source/profile and paid completion; exhausted selected allowance leaves path/history/journal unchanged"]
fn box_ui_save_work_selection_and_failure_preserve_publication() {
    for assigned in [false, true] {
        let (owner, literal) = fixture::fixture(photocraft_doc::SampleType::F32, assigned, 0);
        let capture = Rc::new(RefCell::new(None));
        let observed = capture.clone();
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            Services {
                accounted_native_save: Some(Box::new(move |snapshot, _, _, control| {
                    // Deliberate test debit, not measured serializer work: proves
                    // this exact installed service receives the selected meter.
                    control.check(8_000_000_001).map_err(|e| e.to_string())?;
                    let bytes = photocraft_format::owned_document::save_current_document_archive(
                        snapshot,
                        std::io::Cursor::new(Vec::new()),
                        Default::default(),
                        snapshot.accounted().unwrap().quota(),
                        control,
                    )
                    .map_err(|e| e.to_string())?
                    .into_inner();
                    *observed.borrow_mut() = Some(bytes);
                    Ok(())
                })),
                ..Default::default()
            },
        );
        crate::owned_native_open::adopt_bytes(&mut app, "CURRENT.pcraft", fixture::archive(&owner), &ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.clone();
        let revision = app.session.active().unwrap().revision;
        app.save_to("saved.pcraft".into(), None).unwrap();
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("saved.pcraft"));
        // Save replaces current title/identity without adding an undo entry.
        assert_eq!(app.session.active().unwrap().history.past_len(), history.past_len());
        assert_eq!(app.session.active().unwrap().history.can_redo(), history.can_redo());
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_eq!(fixture::native(&app.session.active().unwrap().doc), literal.raw);
        let bytes = capture.borrow_mut().take().unwrap();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut std::io::Cursor::new(bytes.as_slice()),
            bytes.len() as u64,
            Default::default(),
            owner.quota().clone(),
            &ctl(),
        )
        .unwrap();
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "saved.pcraft", archive, &ctl()).unwrap();
        fixture::exact(&before, &reopened.session.active().unwrap().doc);
        app.services.accounted_native_save = Some(Box::new(|_, _, _, control| {
            // Preparation already consumed positive work. A whole selected
            // profile debit cannot fit: no reset at writer/publication.
            control.check(1u64 << 40).map_err(|e| e.to_string())
        }));
        let state = app.session.active().unwrap();
        let before = state.doc.clone();
        let history = state.history.clone();
        let path = state.path.clone();
        let saved = state.saved_revision;
        let source = state.source.clone();
        let journal = app.session.journal.clone();
        assert!(app.save_to("rejected.pcraft".into(), None).is_err());
        let state = app.session.active().unwrap();
        assert!(state.doc.shares_state(&before));
        assert!(state.history.shares_accounted_history(&history));
        assert_eq!((state.path.clone(), state.saved_revision, state.source.clone()), (path, saved, source));
        assert_eq!(app.session.journal, journal);
        assert!(capture.borrow().is_none());
    }
    // A genuine plain owner still receives the old 8b allowance; installing
    // a save service does not make it a Box policy selection.
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 16 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let mut options = photocraft_xcf::DecodeOptions::default();
    options.limits.resident_bytes = 16 << 20;
    let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 64 << 20,
        index: 16 << 20,
        scratch: 0,
    });
    let imported = photocraft_io::import_owned_xcf_bytes_in_quota(
        "plain.xcf",
        std::sync::Arc::new(xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])),
        &options,
        &limits,
        q,
        &ctl(),
    )
    .unwrap();
    let mut plain = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services { accounted_native_save: Some(Box::new(|_, _, _, c| c.check(8_000_000_001).map_err(|e| e.to_string()))), ..Default::default() },
    );
    crate::owned_xcf_open::adopt_bytes(&mut plain, "plain.xcf", &imported, &ctl()).unwrap();
    let before = plain.session.active().unwrap().doc.clone();
    assert!(plain.save_to("plain.pcraft".into(), None).is_err());
    assert!(plain.session.active().unwrap().doc.shares_state(&before));
    assert!(plain.session.active().unwrap().path.is_none());
}
