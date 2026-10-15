//! UNRUN registered imported metadata regressions; no runtime qualification.
use super::*;
use photocraft_raster::Interrupt;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn session() -> Session {
    let imported = photocraft_io::import_owned_staged_plain_bytes(
        "source.xcf",
        Arc::new(xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        &Interrupt::NONE,
    )
    .unwrap();
    let mut s = Session::new();
    s.add_owned_plain_xcf_import(&imported, "source.xcf", Some("authorized/source.xcf"), &control()).unwrap();
    s
}
#[test]
fn registered_compound_ignored_types_clamp_and_noop_preserve_redo_and_source() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let source = s.active().unwrap().source.clone();
    s.execute("layer.setProps", serde_json::json!({"name":"renamed","visible":false,"opacity":1e100})).unwrap();
    let layer = &s.active().unwrap().doc.layers[0];
    assert_eq!(layer.name, "renamed");
    assert!(!layer.visible);
    assert_eq!(layer.opacity, 1.0);
    assert!(s.undo_with_control(&control()).unwrap());
    assert!(before.shares_state(&s.active().unwrap().doc));
    s.execute("layer.setProps", serde_json::json!({"name":false,"visible":"ignored","opacity":[]})).unwrap();
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert!(s.redo_with_control(&control()).unwrap());
    assert_eq!(s.active().unwrap().source, source);
    assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().items[0].name, "renamed");
    s.execute("layer.setProps", serde_json::json!({"opacity":-3.0})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 0.0);
}
#[test]
fn whole_request_refusal_does_not_apply_supported_prefix() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    for p in [
        serde_json::json!({"name":"must not land","blend":"Multiply"}),
        serde_json::json!({"visible":false,"locks":{}}),
        serde_json::json!({"layer":0,"name":"absent"}),
    ] {
        assert!(s.execute("layer.setProps", p).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert_eq!(s.journal, journal);
    }
}
#[test]
fn actual_name_and_null_targets_match_existing_registered_policy() {
    let mut s = session();
    let name = s.active().unwrap().doc.layers[0].name.clone();
    s.execute("layer.setProps", serde_json::json!({"layer":name,"name":"named"})).unwrap();
    s.execute("layer.setProps", serde_json::json!({"layer":null,"opacity":0.25})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 0.25);
}
#[test]
fn prepared_view_and_channel_changes_refuse_before_edit_or_journal() {
    let mut s = session();
    let p = serde_json::json!({"name":"stale"});
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    let prepared = PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &control()).unwrap();
    s.active_mut().unwrap().channel_view.quick_mask_hidden = true;
    assert!(prepared.execute(&mut s, &control()).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert_eq!(s.journal, journal);
    let prepared = PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &control()).unwrap();
    s.active_mut().unwrap().view_revision += 1;
    assert!(prepared.execute(&mut s, &control()).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
}
#[test]
fn late_cancel_keeps_snapshot_journal_and_returns_all_private_charges() {
    for stop_after in 0..96 {
        let mut s = session();
        let before = s.active().unwrap().doc.clone();
        let journal = s.journal.clone();
        let quota = before.quota().unwrap().clone();
        let baseline = quota.usage().unwrap();
        let count = AtomicUsize::new(0);
        let stop = || count.fetch_add(1, Ordering::Relaxed) >= stop_after;
        let interrupt = Interrupt::cancel_only(&stop);
        let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        let p = serde_json::json!({"name":"cancelled","visible":false,"opacity":0.5});
        if super::super::dispatch(&mut s, "layer.setProps", &p, &ctl).is_err() {
            assert!(before.shares_state(&s.active().unwrap().doc));
            assert_eq!(s.journal, journal);
            assert_eq!(quota.usage().unwrap(), baseline);
        }
    }
}
#[test]
fn actual_index_exhaustion_refuses_compound_before_mutation() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let quota = before.quota().unwrap().clone();
    let used = quota.usage().unwrap();
    let max = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().get(photocraft_xcf::BudgetKind::Index);
    let filler = quota.reserve(QuotaKind::Index, max - used.index).unwrap();
    let journal = s.journal.clone();
    assert!(s.execute("layer.setProps", serde_json::json!({"name":"quota","visible":false,"opacity":0.5})).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert_eq!(s.journal, journal);
    drop(filler);
}

#[test]
fn imported_visibility_lock_blocks_changed_visibility_atomically_but_not_name_opacity() {
    let options = xcf::FixtureOptions { layer_properties: vec![(42, 1u32.to_be_bytes().to_vec())], ..Default::default() };
    let imported = photocraft_io::import_owned_staged_plain_bytes(
        "locked.xcf",
        Arc::new(xcf::image_custom(13, 150, 0, 1, 1, &[31, 61, 91, 255], &options)),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        &Interrupt::NONE,
    )
    .unwrap();
    let mut s = Session::new();
    s.add_owned_plain_xcf_import(&imported, "locked.xcf", None, &control()).unwrap();
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    assert!(s.execute("layer.setProps", serde_json::json!({"name":"refused","visible":false,"opacity":0.25})).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert_eq!(s.journal, journal);
    s.execute("layer.setProps", serde_json::json!({"name":"allowed","visible":true,"opacity":0.25})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[0].name, "allowed");
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 0.25);
    assert!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().items[0].locks.visibility);
}

#[test]
fn nul_name_shape_passes_but_paid_prepare_refuses_before_journal_or_edit() {
    let mut s = session();
    let p = serde_json::json!({"name":"bad\0name","visible":false,"opacity":0.25});
    assert!(parse(&p).is_ok());
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    let quota = before.quota().unwrap().clone();
    let baseline = quota.usage().unwrap();
    assert!(PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &control()).is_err());
    assert!(s.execute("layer.setProps", p).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert_eq!(s.journal, journal);
    assert_eq!(quota.usage().unwrap(), baseline);
}

#[test]
fn coalesce_keys_are_borrowed_exact_bounded_strings_and_other_types_are_ignored() {
    for key in ["", "gesture\0exact", "layer-opacity-slider:42"] {
        let p = serde_json::json!({"opacity":0.5,"coalesce":key});
        let parsed = parse(&p).unwrap();
        let supplied = p["coalesce"].as_str().unwrap();
        assert_eq!(parsed.coalesce, Some(key));
        assert_eq!(parsed.coalesce.unwrap().as_ptr(), supplied.as_ptr());
    }
    for value in [Value::Null, serde_json::json!(false), serde_json::json!(17), serde_json::json!([]), serde_json::json!({})] {
        let p = serde_json::json!({"opacity":0.5,"coalesce":value});
        assert!(parse(&p).unwrap().coalesce.is_none());
    }
    assert!(parse(&serde_json::json!({"opacity":0.5,"coalesce":"x".repeat(129)})).is_err());
}

#[test]
fn registered_compound_same_key_has_one_anchor_and_failures_or_noops_preserve_it() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    s.execute("layer.setProps", serde_json::json!({"opacity":0.2,"coalesce":"exact\0key"})).unwrap();
    let current = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    assert!(s.execute("layer.setProps", serde_json::json!({"name":"must not land","blend":"Multiply","coalesce":"exact\0key"})).is_err());
    assert!(current.shares_state(&s.active().unwrap().doc));
    assert_eq!(journal, s.journal);
    // Ignored typed fields and a different key are an actual no-op; neither
    // arms a new gesture nor changes the existing paid anchor.
    s.execute("layer.setProps", serde_json::json!({"opacity":"ignored","coalesce":"different"})).unwrap();
    assert!(current.shares_state(&s.active().unwrap().doc));
    s.execute("layer.setProps", serde_json::json!({"name":"renamed","visible":false,"opacity":0.4,"coalesce":"exact\0key"})).unwrap();
    assert!(s.undo_with_control(&control()).unwrap());
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert!(!s.active().unwrap().history.can_undo());
    let undone = s.active().unwrap().doc.clone();
    s.execute("layer.setProps", serde_json::json!({"name":false,"opacity":[],"coalesce":"new"})).unwrap();
    assert!(undone.shares_state(&s.active().unwrap().doc));
    assert!(s.redo_with_control(&control()).unwrap());
}

#[test]
fn ignored_nonstring_changed_edit_and_unavailable_undo_break_old_anchor() {
    let mut s = session();
    assert!(!s.undo_with_control(&control()).unwrap());
    s.execute("layer.setProps", serde_json::json!({"opacity":0.2,"coalesce":"key"})).unwrap();
    s.execute("layer.setProps", serde_json::json!({"opacity":0.3,"coalesce":false})).unwrap();
    s.execute("layer.setProps", serde_json::json!({"opacity":0.4,"coalesce":"key"})).unwrap();
    assert!(s.undo_with_control(&control()).unwrap());
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 0.3);
    assert!(s.undo_with_control(&control()).unwrap());
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 0.2);
    assert!(s.undo_with_control(&control()).unwrap());
    assert!(!s.undo_with_control(&control()).unwrap());
    s.execute("layer.setProps", serde_json::json!({"opacity":0.6,"coalesce":"key"})).unwrap();
    assert!(s.undo_with_control(&control()).unwrap());
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 1.0);
}

// A genuine charged two-layer canonical model, not a claimed GIMP source proof.
fn two_layer_session() -> Session {
    use photocraft_doc::{
        BackedLayer, Document, Layer, LayerContent,
        accounted_document::AccountedDocument,
        item_identity::ItemRef,
        xcf_state::{ItemState, XcfState},
    };
    use photocraft_raster::backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits};
    #[derive(Debug)]
    struct Store(NativePlaneDescriptor);
    impl NativePlaneStore for Store {
        fn descriptor(&self) -> NativePlaneDescriptor {
            self.0
        }
        fn read_exact_at(&self, _: u64, bytes: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
            c.check(bytes.len() as u64)?;
            bytes.fill(0);
            Ok(())
        }
    }
    let quota = BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 16 << 20, index: 8 << 20 });
    let guard = Arc::new(quota.reserve(QuotaKind::Resident, 65536).unwrap());
    let mut doc = Document::new("current.xcf", photocraft_geom::Size::new(2, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::F32);
    let mut state = XcfState::default();
    for name in ["lower", "upper"] {
        let descriptor = NativePlaneDescriptor::new(photocraft_color::PixelFormat::RGBA32F, photocraft_geom::Rect::new(0, 0, 2, 1), &[0; 16]).unwrap();
        let raster = BackedRaster::new(Arc::new(Store(descriptor)), quota.clone(), 1, &control()).unwrap();
        let layer = Layer::new(name, LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control()).unwrap()));
        state.items.push(ItemState {
            item: ItemRef::Layer(layer.id),
            name: name.into(),
            custom_name: false,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(descriptor.bounds()),
            mask: None,
            composition: None,
        });
        doc.layers.push(layer);
    }
    doc.xcf_state = Some(state);
    let owner = AccountedDocument::from_owned(doc, guard, quota, &control()).unwrap();
    let mut s = Session::new();
    s.add_accounted_document(owner, None, &control()).unwrap();
    s
}

#[test]
fn registered_same_key_continues_across_requested_layers_selection_and_tab_visit() {
    let mut s = two_layer_session();
    let first = s.active().unwrap().doc.layers[0].id;
    let second = s.active().unwrap().doc.layers[1].id;
    let target = s.active_index().unwrap();
    s.execute("layer.setProps", serde_json::json!({"layer":first.0,"opacity":0.2,"coalesce":"shared"})).unwrap();
    s.execute("layer.select", serde_json::json!({"layer":second.0})).unwrap();
    let other = photocraft_doc::Document::new("other", photocraft_geom::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    s.add_document(other, None).unwrap();
    assert!(s.set_active(target));
    s.execute("layer.setProps", serde_json::json!({"layer":second.0,"name":"changed","visible":false,"coalesce":"shared"})).unwrap();
    assert!(s.undo_with_control(&control()).unwrap());
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.layers[0].opacity, 1.0);
    assert_eq!(doc.layers[1].name, "upper");
    assert!(doc.layers[1].visible);
    assert!(!s.active().unwrap().history.can_undo());
}
