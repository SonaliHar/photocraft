//! Deferred genuine mixed-source activity, separate from CURRENT edits.
use super::*;
#[path = "accounted_filtered_flat_source_activity_fixture.rs"]
mod data;
#[path = "../../ui-egui/src/filtered_flat_fixture.rs"]
#[allow(dead_code)]
mod native;
use photocraft_doc::item_identity::ItemRef;
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
fn bytes(binding: data::Binding) -> Vec<u8> {
    let b = std::fs::read(native::workspace().join(binding.path)).unwrap();
    assert_eq!(b.len(), binding.bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&b)), binding.sha256);
    b
}
fn import(case: &data::Case, input: Vec<u8>, c: &ReadControl<'_>) -> std::result::Result<photocraft_io::OwnedXcfImport, photocraft_io::IoError> {
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
    photocraft_io::import_owned_xcf_bytes_in_quota(case.id, Arc::new(input), &options, &limits, q, c)
}
fn observed(doc: &photocraft_doc::Document, case: &data::Case) {
    assert_eq!(doc.layers.len(), 4);
    assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, case.precision);
    assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
    let state = doc.xcf_state.as_ref().unwrap();
    assert_eq!(state.filter_stacks.len(), 3);
    assert_eq!(state.filter_masks.len(), 3);
    for (layer, expected) in doc.layers.iter().zip(case.roots) {
        assert_eq!(layer.name, expected.name);
        assert_eq!(layer.visible, expected.visible);
        assert_eq!(layer.opacity.to_bits(), expected.opacity_bits);
        assert_eq!(native::raw(layer), bytes(expected.native));
        for property in expected.layer_properties.iter().chain(expected.effect_properties) {
            let identity = format!("{}@{}:4", property.tag, property.offset);
            let record = state
                .opaque
                .iter()
                .find(|r| r.owner == Some(ItemRef::Layer(layer.id)) && r.namespace == "gimp/xcf/source-property" && r.identity == identity)
                .unwrap();
            assert_eq!(record.bytes.as_slice(), property.bytes);
            assert_eq!(record.order, property.order);
            assert_eq!(record.blake3, blake3::hash(&property.bytes).to_hex().to_string());
        }
        if let Some(operation) = expected.operation {
            let stack = state.filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(layer.id)).unwrap();
            let [node] = stack.nodes.as_slice() else { panic!("one") };
            assert_eq!(node.operation.name, operation);
            assert_eq!(Some(node.enabled), expected.enabled);
            let policy = node.composition.as_ref().unwrap();
            assert_eq!(Some([policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator]), expected.raw3);
            assert!(policy.resolved.is_none());
            let mask = state.filter_masks.iter().find(|m| m.owner == node.id).unwrap();
            assert_eq!(mask.surface.try_read_interleaved(doc.bounds()).unwrap(), bytes(expected.mask_native.unwrap()));
        } else {
            assert!(state.filter_stacks.iter().all(|s| s.owner != ItemRef::Layer(layer.id)));
        }
    }
}
#[test]
#[ignore = "UNRUN:12 genuine source activity inputs→strict carrier→typed Session→native before CURRENT changes; literal raw6/33/8 and effect flags/raw/native/masks, disabled restoration+undo/noopredo, metadata/readonly/source fingerprint and inactive malformed source/control refusal. No projection/profile numerical oracle."]
fn mixed_real_source_activity_survives_native_and_current_restoration() {
    for case in &data::CASES {
        let original_bytes = bytes(case.source);
        drop(bytes(case.capture));
        let value = import(case, original_bytes.clone(), &native::ctl()).unwrap();
        let photocraft_io::OwnedXcfImport::FilteredFlat(imported) = &value else { panic!("strict FilteredFlat") };
        observed(imported.document().document(), case);
        let source_path = native::workspace().join(case.source.path);
        let source_path = source_path.to_str().unwrap();
        let mut s = Session::new();
        s.add_owned_filtered_flat_xcf_import(imported, case.id, Some(source_path), &native::ctl()).unwrap();
        let authored = s.active().unwrap().doc.clone();
        assert!(s.active().unwrap().source_read_only);
        assert_eq!(s.active().unwrap().source.as_ref().unwrap().path.as_deref(), Some(source_path));
        let mut reopened = Session::new();
        reopened.add_owned_native_filtered_flat_archive(native::archive(authored.accounted().unwrap()), "authored.pcraft", None, &native::ctl()).unwrap();
        native::same(&authored, &reopened.active().unwrap().doc);
        observed(&reopened.active().unwrap().doc, case);
        let opaque = authored.xcf_state.as_ref().unwrap().opaque.clone();
        let disabled = authored.xcf_state.as_ref().unwrap().filter_stacks.iter().filter(|st| !st.nodes[0].enabled).map(|st| st.nodes[0].id).collect::<Vec<_>>();
        for id in disabled {
            let p = serde_json::json!({"filterId":id.0.to_string(),"enabled":true});
            assert_eq!(s.execute("xcf.filter.enable", p).unwrap()["updated"], true);
            assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().opaque, opaque);
            s.execute("edit.undo", serde_json::json!({})).unwrap();
            native::same(&authored, &s.active().unwrap().doc);
            assert_eq!(s.execute("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":false})).unwrap()["updated"], false);
            assert!(s.active().unwrap().history.can_redo());
            s.execute("edit.redo", serde_json::json!({})).unwrap();
            s.execute("edit.undo", serde_json::json!({})).unwrap();
            native::same(&authored, &s.active().unwrap().doc);
        }
        for layer in &authored.layers {
            if !layer.visible || layer.opacity.to_bits() != 1.0f32.to_bits() {
                s.execute("layer.setProps", serde_json::json!({"layer":layer.id.0,"visible":true,"opacity":1.0})).unwrap();
                assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().opaque, opaque);
                s.execute("edit.undo", serde_json::json!({})).unwrap();
                native::same(&authored, &s.active().unwrap().doc);
            }
        }
        for (id, p) in [("layer.translate", serde_json::json!({"dx":0})), ("layer.select", serde_json::json!({"index":0}))] {
            assert!(s.execute(id, p).is_err());
            assert!(s.active().unwrap().doc.shares_state(&authored));
        }
        // Malformed derivative of THIS actual source, never a producer witness. Even
        // hidden/zero/disabled roots may not weaken fixed unit FILTER opacity.
        let row =
            case.roots.iter().find(|r| r.operation.is_some() && (!r.visible || r.opacity_bits == 0 || r.enabled == Some(false))).unwrap_or(&case.roots[1]);
        let property = row.effect_properties.iter().find(|p| p.tag == 33).unwrap();
        let mut malformed = original_bytes.clone();
        let at = property.offset as usize;
        assert_eq!(&malformed[at..at + 4], &property.bytes);
        malformed[at..at + 4].copy_from_slice(&0.5f32.to_bits().to_be_bytes());
        assert!(import(case, malformed, &native::ctl()).is_err());
        let calls = AtomicU64::new(0);
        let deny = |_: u64| {
            calls.fetch_add(1, Ordering::Relaxed);
            Err(BackingError::Work)
        };
        let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
        assert!(import(case, original_bytes.clone(), &c).is_err());
        assert!(calls.load(Ordering::Relaxed) > 0);
        let stop = || true;
        let interrupt = photocraft_raster::Interrupt::cancel_only(&stop);
        let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(import(case, original_bytes.clone(), &c).is_err());
        assert!(s.active().unwrap().doc.shares_state(&authored));
        assert_eq!(std::fs::read(source_path).unwrap(), original_bytes);
    }
}
