//! UNRUN explicit owned-open route drafts. Mock service owns a real reader;
//! no filesystem, GUI runtime, default policy or source qualification is tested.
use super::*;
use crate::{Services, services::OwnedNativeOpenFn};
use photocraft_doc::{LayerContent, accounted_document::AccountedDocument};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLease};
use std::{cell::RefCell, io::Cursor, rc::Rc, sync::Arc};

#[path = "../../xcf/tests/common/mod.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod xcf;
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
struct Packet {
    bytes: Vec<u8>,
    original: AccountedDocument,
    quota: BackingQuota,
    _lease: QuotaLease,
}
fn packet() -> Arc<Packet> {
    let imported = photocraft_io::import_owned_staged_plain_bytes(
        "original.xcf",
        Arc::new(xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        &Interrupt::NONE,
    )
    .unwrap();
    let quota = imported.document().quota().clone();
    let original = imported.document().clone();
    let bytes =
        photocraft_format::owned_document::save_current_document_archive(original.document(), Cursor::new(Vec::new()), Default::default(), &quota, &control())
            .unwrap()
            .into_inner();
    let lease = quota.reserve(QuotaKind::Resident, bytes.capacity() as u64).unwrap();
    Arc::new(Packet { bytes, original, quota, _lease: lease })
}
fn service(packet: Arc<Packet>, seen: Rc<RefCell<Vec<(String, bool)>>>) -> OwnedNativeOpenFn {
    Box::new(move |path, catalog, c| {
        seen.borrow_mut().push((path.into(), catalog.is_some()));
        let quota = catalog.cloned().unwrap_or_else(|| packet.quota.clone());
        photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(packet.bytes.as_slice()),
            packet.bytes.len() as u64,
            Default::default(),
            quota,
            c,
        )
        .map_err(|e| e.to_string())
    })
}
#[test]
fn explicit_registered_open_preserves_saved_current_interpretation_and_actual_runtime_identity() {
    let packet = packet();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services { owned_native_open: Some(service(packet.clone(), seen.clone())), ..Default::default() },
    );
    app.session.color.settings.policy_rgb = photocraft_engine::color_cmds::Policy::Convert;
    app.session.color.settings.working_rgb = "display-p3".into();
    app.session.color.settings.ask_on_mismatch = true;
    assert!(app.session.color.open_question(packet.original.document()).is_some());
    let reply =
        crate::menus::invoke(&mut app, &egui::Context::default(), "file.open", json!({"path":"authorized/current.pcraft","nativeProfilePolicy":"keepCurrent"}))
            .unwrap();
    assert_eq!(reply, json!({"warnings":[]}));
    assert_eq!(seen.borrow().as_slice(), &[("authorized/current.pcraft".into(), false)]);
    let state = app.session.active().unwrap();
    assert_eq!(state.doc.id, packet.original.document().id);
    assert_eq!(state.doc.icc_profile, packet.original.document().icc_profile);
    assert_eq!(state.doc.sample_encoding, packet.original.document().sample_encoding);
    assert_eq!(state.doc.working_profile_interpretation, packet.original.document().working_profile_interpretation);
    assert_eq!(state.doc.source_color_context, packet.original.document().source_color_context);
    assert_eq!(state.doc.xcf_state, packet.original.document().xcf_state);
    assert_eq!(state.path.as_deref(), Some("authorized/current.pcraft"));
    assert_eq!(state.source.as_ref().unwrap().path.as_deref(), Some("authorized/current.pcraft"));
    assert_eq!(state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
    assert!(!state.source_read_only);
    assert_eq!(state.revision, state.saved_revision);
    let LayerContent::BackedRaster(backed) = &state.doc.layers[0].content else {
        panic!("owned native backing");
    };
    let mut raw = [0; 4];
    backed.checked_view().read_logical_pixel_native_into(0, 0, &mut raw, &control()).unwrap();
    assert_eq!(raw, [31, 61, 91, 255]);
    assert!(app.held_opens.is_empty());
    assert_eq!(app.ui.recent_files.first().map(String::as_str), Some("authorized/current.pcraft"));
}
#[test]
fn malformed_explicit_policy_never_calls_any_reader_or_changes_session() {
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services { owned_native_open: Some(Box::new(|_, _, _| panic!("invalid request reached reader"))), ..Default::default() },
    );
    for params in [
        json!({"path":"x.pcraft","nativeProfilePolicy":null}),
        json!({"path":"x.pcraft","nativeProfilePolicy":"convert"}),
        json!({"path":false,"nativeProfilePolicy":"keepCurrent"}),
        json!({"path":"x.png","nativeProfilePolicy":"keepCurrent"}),
        json!({"path":"bad\0.pcraft","nativeProfilePolicy":"keepCurrent"}),
        json!({"path":"x.pcraft","nativeProfilePolicy":"keepCurrent","other":true}),
        json!({"path":format!("{}.pcraft", "x".repeat(4096)),"nativeProfilePolicy":"keepCurrent"}),
    ] {
        assert!(invoke(&mut app, &params).is_err());
        assert!(app.session.documents().is_empty());
    }
}
#[test]
fn default_open_retains_existing_dialog_and_explicit_automation_has_no_ambient_fallback() {
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services {
            file_dialog: Some(Box::new(|_, _, _| {})),
            owned_native_open: Some(Box::new(|_, _, _| panic!("ambient reader reached"))),
            ..Default::default()
        },
    );
    let reply = crate::menus::invoke(&mut app, &egui::Context::default(), "file.open", json!({})).unwrap();
    assert_eq!(reply, json!({"fileDialog":"open"}));
    assert!(app.file_dialog.is_some());
    app.automation_input = true;
    assert!(invoke(&mut app, &json!({"path":"rooted.pcraft","nativeProfilePolicy":"keepCurrent"})).is_err());
    assert!(app.session.documents().is_empty());
}
#[test]
fn actual_catalog_quota_reused_when_last_owned_tab_closed_but_resident_tab_remains() {
    let packet = packet();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services { owned_native_open: Some(service(packet.clone(), seen.clone())), ..Default::default() },
    );
    open_path(&mut app, "first.pcraft", Authority::Interactive).unwrap();
    app.session
        .add_document(
            photocraft_doc::Document::new("resident", photocraft_geom::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8),
            None,
        )
        .unwrap();
    app.session.close(0).unwrap();
    assert!(app.session.active().unwrap().doc.quota().is_none());
    assert!(app.session.accounted_open_quota().unwrap().shares_ledger(&packet.quota));
    open_path(&mut app, "second.pcraft", Authority::Interactive).unwrap();
    assert_eq!(seen.borrow().as_slice(), &[("first.pcraft".into(), false), ("second.pcraft".into(), true)]);
}
#[test]
fn duplicate_identity_and_provider_fault_preserve_open_tab_and_private_quota() {
    let packet = packet();
    let mut app =
        PhotocraftApp::new(photocraft_engine::Session::new(), Services { owned_native_open: Some(service(packet, Rc::default())), ..Default::default() });
    open_path(&mut app, "first.pcraft", Authority::Interactive).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let active = app.session.active_index();
    let quota = before.quota().unwrap().clone();
    let baseline = quota.usage().unwrap();
    assert!(open_path(&mut app, "duplicate.pcraft", Authority::Interactive).is_err());
    assert!(before.shares_state(&app.session.active().unwrap().doc));
    assert_eq!(active, app.session.active_index());
    assert_eq!(quota.usage().unwrap(), baseline);
    app.services.owned_native_open = Some(Box::new(|_, _, _| Err("read refused".into())));
    assert_eq!(open_path(&mut app, "fault.pcraft", Authority::Interactive).unwrap_err(), "read refused");
    assert!(before.shares_state(&app.session.active().unwrap().doc));
}

#[test]
fn scoped_open_uses_only_held_root_callback_and_prebuilt_reply() {
    let packet = packet();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services {
            owned_native_open: Some(Box::new(|_, _, _| panic!("ambient reader reached"))),
            automation_owned_native_open: Some(service(packet.clone(), seen.clone())),
            ..Default::default()
        },
    );
    let reply = invoke_authorized(&mut app, &json!({"path":"rooted/current.pcraft","nativeProfilePolicy":"keepCurrent"})).unwrap();
    assert_eq!(reply, json!({"path":"rooted/current.pcraft","name":"current.pcraft","warnings":[]}));
    assert_eq!(app.session.active().unwrap().doc.name, packet.original.document().name);
    assert!(app.ui.recent_files.is_empty());
    assert_eq!(seen.borrow().as_slice(), &[("rooted/current.pcraft".into(), false)]);
}
#[test]
fn scoped_open_requires_path_and_capability_without_picker_or_ambient_fallback() {
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services {
            owned_native_open: Some(Box::new(|_, _, _| panic!("ambient reader reached"))),
            file_dialog: Some(Box::new(|_, _, _| panic!("picker reached"))),
            ..Default::default()
        },
    );
    assert!(invoke_authorized(&mut app, &json!({"nativeProfilePolicy":"keepCurrent"})).is_err());
    assert!(invoke_authorized(&mut app, &json!({"path":"rooted.pcraft","nativeProfilePolicy":"keepCurrent"})).is_err());
    assert!(app.session.documents().is_empty());
    assert!(app.file_dialog.is_none());
}

// UNRUN: a stored name selects the strict validator, never successful adoption
// or a retry through the ordinary Legacy class when that validator refuses.
#[test]
fn unqualified_threshold_archive_refuses_both_native_authorities_without_legacy_fallback() {
    use photocraft_doc::{
        item_identity::{FilterId, ItemRef},
        xcf_filters::{FilterNode, FilterStack, OperationIdentity},
    };
    let plain = packet();
    let _model_fee = plain.quota.reserve(QuotaKind::Resident, 2 << 20).unwrap();
    let mut document = plain.original.document().clone();
    let layer = document.layers[0].id;
    document.xcf_state.as_mut().unwrap().filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(layer),
        revision: 0,
        nodes: vec![FilterNode {
            id: FilterId(100),
            operation: OperationIdentity { name: "gimp:threshold-alpha".into(), stored_version: "0:0".into(), schema_epoch: "unqualified".into() },
            arguments: Vec::new(),
            source_display_name: None,
            source_icon: None,
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: Vec::new(),
            composition: None,
        }],
    });
    let bytes =
        photocraft_format::owned_document::save_current_document_archive(&document, Cursor::new(Vec::new()), Default::default(), &plain.quota, &control())
            .unwrap()
            .into_inner();
    let lease = plain.quota.reserve(QuotaKind::Resident, bytes.capacity() as u64).unwrap();
    let source = Arc::new(Packet { bytes, original: plain.original.clone(), quota: plain.quota.clone(), _lease: lease });
    for rooted in [false, true] {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let services = if rooted {
            Services { automation_owned_native_open: Some(service(source.clone(), seen.clone())), ..Default::default() }
        } else {
            Services { owned_native_open: Some(service(source.clone(), seen.clone())), ..Default::default() }
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let before = source.quota.usage().unwrap();
        let result = if rooted {
            invoke_authorized(&mut app, &json!({"path":"rooted/current.pcraft","nativeProfilePolicy":"keepCurrent"}))
        } else {
            invoke(&mut app, &json!({"path":"selected/current.pcraft","nativeProfilePolicy":"keepCurrent"}))
        };
        assert!(result.is_err());
        assert_eq!(seen.borrow().len(), 1);
        assert!(app.session.documents().is_empty());
        assert!(app.ui.recent_files.is_empty());
        assert_eq!(source.quota.usage().unwrap(), before);
    }
}
