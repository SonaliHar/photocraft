//! UNRUN explicit owned-XCF UI/adoption drafts; actual reader and retained source
//! are used, but no filesystem, GUI, or runtime execution has occurred.
use super::*;
use crate::{Services, services::OwnedPlainXcfOpenFn};
use photocraft_doc::LayerContent;
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use std::{cell::RefCell, io::Cursor, rc::Rc, sync::Arc};
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
fn raw() -> Arc<Vec<u8>> {
    Arc::new(xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255]))
}
fn service(raw: Arc<Vec<u8>>, seen: Rc<RefCell<Vec<(String, bool)>>>) -> OwnedPlainXcfOpenFn {
    Box::new(move |path, catalog, control| {
        seen.borrow_mut().push((path.into(), catalog.is_some()));
        let quota = catalog.cloned().unwrap_or_else(|| BackingQuota::new(QuotaLimits { resident: 512 << 20, index: 64 << 20, scratch: 0 }));
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("scratchBytes", 0), ("residentBytes", 32 << 20), ("indexBytes", 8 << 20)])
            .unwrap();
        photocraft_io::import_owned_staged_plain_from_reader_in_quota(
            path.rsplit(['/', '\\']).next().unwrap(),
            &mut Cursor::new(raw.as_slice()),
            raw.len() as u64,
            &Default::default(),
            &limits,
            quota,
            control,
        )
        .map_err(|e| e.to_string())
    })
}
fn app(service: OwnedPlainXcfOpenFn) -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), Services { owned_xcf_open: Some(service), ..Default::default() })
}
#[test]
fn explicit_registered_xcf_open_retains_current_profile_readonly_identity_and_native_source() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut app = app(service(raw(), seen.clone()));
    app.session.color.settings.policy_rgb = photocraft_engine::color_cmds::Policy::Convert;
    app.session.color.settings.working_rgb = "display-p3".into();
    app.session.color.settings.ask_on_mismatch = true;
    let reply =
        crate::menus::invoke(&mut app, &egui::Context::default(), "file.open", json!({"path":"selected/original.xcf","xcfProfilePolicy":"keepCurrent"}))
            .unwrap();
    assert_eq!(reply["sourceReadOnly"], true);
    assert!(reply["warnings"].is_array());
    assert!(reply["assessment"].is_object());
    let state = app.session.active().unwrap();
    assert!(state.source_read_only);
    assert_eq!(state.source.as_ref().unwrap().path.as_deref(), Some("selected/original.xcf"));
    assert!(state.path.is_none());
    assert!(state.doc.source_color_context.is_some());
    assert!(state.doc.xcf_state.is_some());
    assert!(state.doc.icc_profile.is_some());
    assert!(!state.is_dirty());
    let LayerContent::BackedRaster(layer) = &state.doc.layers[0].content else { panic!() };
    let mut pixel = [0; 4];
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    layer.checked_view().read_logical_pixel_native_into(0, 0, &mut pixel, &c).unwrap();
    assert_eq!(pixel, [31, 61, 91, 255]);
    assert_eq!(seen.borrow().as_slice(), &[("selected/original.xcf".into(), false)]);
    assert!(app.held_opens.is_empty());
    assert_eq!(app.ui.recent_files.first().map(String::as_str), Some("selected/original.xcf"));
}
#[test]
fn malformed_combined_policy_and_missing_authority_never_read_or_mutate() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut app = app(service(raw(), seen.clone()));
    for params in [
        json!({"path":"x.xcf","xcfProfilePolicy":"convert"}),
        json!({"path":"x.xcf","xcfProfilePolicy":false}),
        json!({"path":"x.pcraft","xcfProfilePolicy":"keepCurrent"}),
        json!({"path":"bad\0.xcf","xcfProfilePolicy":"keepCurrent"}),
        json!({"path":"x.xcf","xcfProfilePolicy":"keepCurrent","nativeProfilePolicy":"keepCurrent"}),
        json!({"path":"x.xcf","xcfProfilePolicy":"keepCurrent","extra":true}),
    ] {
        assert!(crate::menus::invoke(&mut app, &egui::Context::default(), "file.open", params).is_err());
        assert!(app.session.documents().is_empty());
    }
    assert!(seen.borrow().is_empty());
    assert!(invoke_authorized(&mut app, &json!({"path":"x.xcf","xcfProfilePolicy":"keepCurrent"})).is_err());
    assert!(invoke_authorized(&mut app, &json!({"xcfProfilePolicy":"keepCurrent"})).is_err());
    assert!(seen.borrow().is_empty());
}
#[test]
fn scoped_reader_is_independent_and_reuses_actual_catalog_quota_without_recent_hooks() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services {
            owned_xcf_open: Some(Box::new(|_, _, _| panic!("ambient reader reached"))),
            automation_owned_xcf_open: Some(service(raw(), seen.clone())),
            ..Default::default()
        },
    );
    let params = json!({"path":"safe/a.xcf","xcfProfilePolicy":"keepCurrent"});
    let reply = invoke_authorized(&mut app, &params).unwrap();
    assert_eq!(reply["path"], "safe/a.xcf");
    assert_eq!(reply["name"], "a.xcf");
    assert_eq!(reply["sourceReadOnly"], true);
    let first = app.session.accounted_open_quota().unwrap().clone();
    invoke_authorized(&mut app, &json!({"path":"safe/b.xcf","xcfProfilePolicy":"keepCurrent"})).unwrap();
    assert!(app.session.active().unwrap().doc.quota().unwrap().shares_ledger(&first));
    assert_eq!(seen.borrow().as_slice(), &[("safe/a.xcf".into(), false), ("safe/b.xcf".into(), true)]);
    assert!(app.ui.recent_files.is_empty());
}
#[test]
fn failed_source_and_foreign_returned_ledger_preserve_existing_tab_and_history() {
    let mut app = app(service(raw(), Rc::default()));
    invoke(&mut app, &json!({"path":"original.xcf","xcfProfilePolicy":"keepCurrent"})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let active = app.session.active_index();
    app.services.owned_xcf_open = Some(Box::new(|_, _, _| Err("unsupported original class".into())));
    assert!(invoke(&mut app, &json!({"path":"unsupported.xcf","xcfProfilePolicy":"keepCurrent"})).is_err());
    app.services.owned_xcf_open = Some(Box::new(move |p, _, c| service(raw(), Rc::default())(p, None, c)));
    assert!(invoke(&mut app, &json!({"path":"foreign.xcf","xcfProfilePolicy":"keepCurrent"})).is_err());
    assert_eq!(app.session.active_index(), active);
    assert!(app.session.active().unwrap().doc.shares_state(&before));
}
#[test]
fn diagnostics_reply_matches_actual_import_assessment_and_failed_copy_refunds_local_lease() {
    let q = BackingQuota::new(QuotaLimits { resident: 512 << 20, index: 64 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("scratchBytes", 0), ("residentBytes", 32 << 20), ("indexBytes", 8 << 20)])
        .unwrap();
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let bytes = raw();
    let owned = photocraft_io::import_owned_staged_plain_from_reader_in_quota(
        "literal.xcf",
        &mut Cursor::new(bytes.as_slice()),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &c,
    )
    .unwrap();
    let owned = photocraft_io::OwnedXcfImport::Plain(owned);
    let baseline = q.usage().unwrap();
    let prepared = PreparedReply::new(&owned, "safe/literal.xcf", "literal.xcf", Authority::HeldRoot, &c).unwrap();
    assert_eq!(prepared.value["assessment"], serde_json::to_value(owned.assessment()).unwrap());
    assert_eq!(prepared.value["sourceReadOnly"], true);
    drop(prepared);
    assert_eq!(q.usage().unwrap(), baseline);
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(PreparedReply::new(&owned, "safe/literal.xcf", "literal.xcf", Authority::HeldRoot, &denied).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
}
