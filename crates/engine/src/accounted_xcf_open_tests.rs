//! UNRUN actual synthetic-XCF ownership/publication/source-protection drafts.
use super::*;
use photocraft_raster::{Interrupt, backing::quota::QuotaKind};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[path = "../../xcf/tests/common/mod.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod xcf;

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn imported() -> OwnedPlainXcfImport {
    photocraft_io::import_owned_staged_plain_bytes(
        "renamed.psd",
        Arc::new(xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        &Interrupt::NONE,
    )
    .unwrap()
}
#[test]
fn real_owned_import_keeps_runtime_source_exclusion_and_native_owner_after_result_drops() {
    let imported = imported();
    let expected = imported.document().clone();
    let mut session = Session::new();
    let index = session.add_owned_plain_xcf_import(&imported, "renamed.psd", Some("authorized/renamed.psd"), &control()).unwrap();
    drop(imported);
    let state = session.active().unwrap();
    assert_eq!(index, 0);
    assert!(state.doc.accounted().unwrap().shares_model(&expected));
    assert!(state.path.is_none());
    assert!(state.source_read_only);
    let source = state.source.as_ref().unwrap();
    assert_eq!(source.label, "renamed.psd");
    assert!(source.check_destination("authorized/renamed.psd").is_err());
    assert!(source.check_destination("copy.pcraft").is_ok());
    assert_eq!(state.doc.source_color_context, expected.document().source_color_context);
}
#[test]
fn late_cancel_preserves_old_active_document_and_returns_runtime_source_charges() {
    for fail_after in 0..128 {
        let imported = imported();
        let quota = imported.document().quota().clone();
        let baseline = quota.usage().unwrap();
        let mut session = Session::new();
        session
            .add_document(
                photocraft_doc::Document::new("existing", photocraft_geom::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8),
                None,
            )
            .unwrap();
        let before = session.active().unwrap().doc.clone();
        let calls = AtomicUsize::new(0);
        let stop = || calls.fetch_add(1, Ordering::Relaxed) >= fail_after;
        let interrupt = Interrupt::cancel_only(&stop);
        let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        if session.add_owned_plain_xcf_import(&imported, "renamed.psd", Some("authorized/renamed.psd"), &ctl).is_err() {
            assert_eq!(session.documents().len(), 1);
            assert!(before.shares_state(&session.active().unwrap().doc));
            assert_eq!(quota.usage().unwrap(), baseline);
        }
    }
}
#[test]
fn invalid_identity_and_late_quota_denial_do_not_publish_or_change_input() {
    let imported = imported();
    let quota = imported.document().quota().clone();
    let baseline = quota.usage().unwrap();
    let mut session = Session::new();
    assert!(session.add_owned_plain_xcf_import(&imported, "bad\0label", None, &control()).is_err());
    assert!(session.add_owned_plain_xcf_import(&imported, "ok", Some(&"a".repeat(4097)), &control()).is_err());
    assert!(session.documents().is_empty());
    assert_eq!(quota.usage().unwrap(), baseline);
    // Fill the actual remaining index quota; the model itself stays admitted.
    let max = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().get(photocraft_xcf::BudgetKind::Index);
    let filler = quota.reserve(QuotaKind::Index, max - baseline.index).unwrap();
    assert!(session.add_owned_plain_xcf_import(&imported, "renamed.psd", None, &control()).is_err());
    assert!(session.documents().is_empty());
    drop(filler);
    assert_eq!(quota.usage().unwrap(), baseline);
}

#[test]
fn actual_owned_xcf_registered_color_range_and_undo_keep_native_source_and_runtime_exclusion() {
    let imported = imported();
    let mut session = Session::new();
    session.add_owned_plain_xcf_import(&imported, "renamed.psd", Some("authorized/renamed.psd"), &control()).unwrap();
    drop(imported);
    let before = session.active().unwrap().doc.clone();
    let source = session.active().unwrap().source.clone();
    session.execute("select.colorRange", serde_json::json!({"sampleAllLayers":false,"points":[[0,0]],"fuzziness":0})).unwrap();
    let current = &session.active().unwrap().doc;
    assert_eq!(current.source_color_context, before.source_color_context);
    assert_eq!(current.xcf_state, before.xcf_state);
    assert_eq!(current.working_profile_interpretation, before.working_profile_interpretation);
    let mut coverage = [0];
    current.selection.as_ref().unwrap().try_read_pixel_bytes(0, 0, &mut coverage).unwrap();
    assert_eq!(coverage, [255]);
    let photocraft_doc::LayerContent::BackedRaster(layer) = &current.layers[0].content else { panic!("owned source lost") };
    let mut raw = [0; 4];
    layer.checked_view().read_logical_pixel_native_into(0, 0, &mut raw, &control()).unwrap();
    assert_eq!(raw, [31, 61, 91, 255]);
    session.execute("edit.undo", serde_json::json!({})).unwrap();
    assert!(before.shares_state(&session.active().unwrap().doc));
    assert_eq!(session.active().unwrap().source, source);
    assert!(session.active().unwrap().source_read_only);
}

#[test]
fn actual_owned_xcf_compound_properties_are_one_undo_and_equal_request_preserves_redo() {
    let imported = imported();
    let mut session = Session::new();
    session.add_owned_plain_xcf_import(&imported, "renamed.psd", Some("authorized/renamed.psd"), &control()).unwrap();
    drop(imported);
    let before = session.active().unwrap().doc.clone();
    let source = session.active().unwrap().source.clone();
    session.execute("layer.setProps", serde_json::json!({"name":"Current name","visible":false,"opacity":0.375})).unwrap();
    let changed = session.active().unwrap().doc.clone();
    assert_eq!(changed.layers[0].name, "Current name");
    assert!(!changed.layers[0].visible);
    assert_eq!(changed.layers[0].opacity.to_bits(), 0.375f32.to_bits());
    assert_eq!(changed.source_color_context, before.source_color_context);
    assert_eq!(changed.xcf_state.as_ref().unwrap().provenance, before.xcf_state.as_ref().unwrap().provenance);
    let item =
        changed.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(changed.layers[0].id)).unwrap();
    assert_eq!(item.name, "Current name");
    assert!(!item.visible);
    session.execute("edit.undo", serde_json::json!({})).unwrap();
    assert!(before.shares_state(&session.active().unwrap().doc));
    session
        .execute("layer.setProps", serde_json::json!({"name":before.layers[0].name,"visible":before.layers[0].visible,"opacity":before.layers[0].opacity}))
        .unwrap();
    assert!(before.shares_state(&session.active().unwrap().doc));
    session.execute("edit.redo", serde_json::json!({})).unwrap();
    assert!(changed.shares_state(&session.active().unwrap().doc));
    assert_eq!(session.active().unwrap().source, source);
    let photocraft_doc::LayerContent::BackedRaster(layer) = &session.active().unwrap().doc.layers[0].content else { panic!("owned source lost") };
    let mut raw = [0; 4];
    layer.checked_view().read_logical_pixel_native_into(0, 0, &mut raw, &control()).unwrap();
    assert_eq!(raw, [31, 61, 91, 255]);
}
