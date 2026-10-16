//! UNRUN genuine original activity files; no producer projection/profile oracle.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_source_activity_fixture.rs"]
mod data;
#[path = "../filtered_flat_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
fn bytes(binding: data::Binding) -> Vec<u8> {
    let b = std::fs::read(fixture::workspace().join(binding.path)).unwrap();
    assert_eq!(b.len(), binding.bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&b)), binding.sha256);
    b
}
fn import(case: &data::Case) -> photocraft_io::OwnedXcfImport {
    let input = bytes(case.source); // external immutable test input, actual importer retains custody
    let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 256 << 20,
        index: 64 << 20,
        scratch: 0,
    });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let mut options = photocraft_xcf::DecodeOptions::default();
    options.limits.resident_bytes = 64 << 20;
    options.limits.input_bytes = 16 << 20;
    let input = photocraft_io::import_owned_xcf_bytes_in_quota(case.source.path, Arc::new(input), &options, &limits, q, &fixture::ctl()).unwrap();
    assert!(matches!(&input, photocraft_io::OwnedXcfImport::FilteredFlat(_)));
    input
}
fn observed(doc: &DocumentSnapshot, case: &data::Case) {
    assert!(doc.accounted().unwrap().is_readonly_filtered_flat_point_storage());
    assert_eq!(doc.layers.len(), 4);
    assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, case.precision);
    assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
    let source = bytes(case.source);
    for (index, (layer, expected)) in doc.layers.iter().zip(&case.roots).enumerate() {
        assert_eq!(layer.name, expected.name);
        assert_eq!(layer.visible, case.visible[index]);
        assert_eq!(layer.opacity.to_bits(), case.opacity_bits[index]);
        assert_eq!(fixture::raw(layer), bytes(expected.native));
        for property in &expected.layer_properties {
            let at = property.offset as usize;
            assert_eq!(&source[at..at + 4], &property.bytes);
        }
        let stack = doc.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|stack| stack.owner == ItemRef::Layer(layer.id));
        if let Some(operation) = expected.operation {
            let node = &stack.unwrap().nodes[0];
            assert_eq!(node.operation.name, operation);
            assert_eq!(Some(node.enabled), case.enabled[index]);
            assert!(photocraft_engine::filtered_flat_filter_edit_candidate(doc.accounted().unwrap(), layer.id, node));
            let policy = node.composition.as_ref().unwrap();
            assert_eq!(Some([policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator]), expected.raw3);
            assert!(policy.resolved.is_none());
            let mask = doc.xcf_state.as_ref().unwrap().filter_masks.iter().find(|mask| mask.owner == node.id).unwrap();
            assert_eq!(mask.surface.try_read_interleaved(doc.bounds()).unwrap(), bytes(expected.mask_native.unwrap()));
        } else {
            assert!(stack.is_none());
        }
    }
}
fn preview(app: &PhotocraftApp, doc: &DocumentSnapshot) -> egui::ColorImage {
    let display = app.session.color.canvas_display_for(doc, None).unwrap();
    let (image, fee) =
        crate::canvas_backed_preview::backed_snapshot_display_image(doc, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
    let copy = image.clone();
    drop((image, fee));
    copy
}
#[test]
#[ignore = "UNRUN all12 genuine originalactivity typedsource/current fullproof, exacttarget enable+compoundmetadata, raw/native/profile/history and native display continuation; no producer numericoracle"]
fn original_activity_survives_targeted_controls_and_native_continuation() {
    for (index, case) in data::CASES.iter().enumerate() {
        let input = import(case);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, case.source.path, &input, &fixture::ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        observed(&before, case);
        let owner = before.accounted().unwrap();
        let proof =
            photocraft_compose::backed_filtered_flat_point_document::PreparedBackedFilteredFlatPointDocument::new(owner, Default::default(), &fixture::ctl())
                .unwrap();
        proof.validate_operative(&fixture::ctl()).unwrap();
        drop(proof);
        let initial = preview(&app, &before);
        let layer = before.layers[1 + index % 3].id;
        let target = capture(&app, layer).unwrap();
        assert!(validate(&app, &target));
        let retained = node(&before, layer).unwrap().0;
        let original_enabled = retained.enabled;
        submit(&mut app, &target, "xcf.filter.enable", json!({"filterId":retained.id.0.to_string(),"enabled":!original_enabled}));
        assert!(!app.ui.status_error);
        let changed = app.session.active().unwrap().doc.clone();
        assert_eq!(node(&changed, layer).unwrap().0.enabled, !original_enabled);
        assert_eq!(node(&changed, layer).unwrap().0.arguments, retained.arguments);
        assert_eq!(node(&changed, layer).unwrap().0.composition, retained.composition);
        assert_eq!(changed.icc_profile, before.icc_profile);
        assert_eq!(changed.source_color_context, before.source_color_context);
        assert_eq!(changed.xcf_state.as_ref().unwrap().filter_masks, before.xcf_state.as_ref().unwrap().filter_masks);
        for (a, b) in before.layers.iter().zip(&changed.layers) {
            assert_eq!(a.id, b.id);
            assert_eq!(fixture::raw(a), fixture::raw(b));
        }
        app.run("edit.undo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(preview(&app, &before), initial);
        app.run("edit.redo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&changed));
        app.run("layer.setProps", json!({"layer":layer.0,"name":"Activity CURRENT","visible":true,"opacity":0.375})).unwrap();
        let current = app.session.active().unwrap().doc.clone();
        let root = current.layers.iter().find(|l| l.id == layer).unwrap();
        assert_eq!(root.name, "Activity CURRENT");
        assert!(root.visible);
        assert_eq!(root.opacity.to_bits(), 0.375f32.to_bits());
        let expected = preview(&app, &current);
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "activity.pcraft", fixture::archive(current.accounted().unwrap()), &fixture::ctl()).unwrap();
        let snapshot = reopened.session.active().unwrap().doc.clone();
        fixture::same(&current, &snapshot);
        assert_eq!(preview(&reopened, &snapshot), expected);
        let titled = snapshot.accounted().unwrap().try_with_document_name(case.id, &fixture::ctl()).unwrap();
        assert_eq!(titled.document().layers.len(), 4);
        // Even an all-hidden/all-zero/all-disabled original retains three actual
        // stacks. It is neither empty storage nor permission for extra actions.
        assert_eq!(snapshot.xcf_state.as_ref().unwrap().filter_stacks.len(), 3);
        assert!(reopened.run("xcf.filter.remove", json!({"filterId":node(&snapshot,layer).unwrap().0.id.0.to_string()})).is_err());
        assert!(reopened.session.active().unwrap().doc.shares_state(&snapshot));
    }
}
