//! UNRUN authentic GIMP-saved disabled originals; no synthesized XCF.
//! Harness captures remain external references; production native/source owners
//! retain the actual shared quota fees through the selected importer.
use super::*;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
fn root() -> std::path::PathBuf {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_DISABLED_NEIGHBORHOOD_EVIDENCE").unwrap());
    assert_eq!(hash(&std::fs::read(root.join("disabled-inputs-v1.json")).unwrap()), "f67c3ade2fd1580f5d9846f8e76f64392f1bfa7264b451f5b4af3ede0000ce91");
    assert_eq!(hash(&std::fs::read(root.join("combined-receipt-v1.json")).unwrap()), "cd83e415537d4085f15eeee7eaea4000fe6f184983d8190dc3840dbe115a9f22");
    // Accepted role: eight actual saved disabled sources and independent fresh
    // storage/activation controls. Full effective ICC timestamp differences stay
    // HOLD; retained-filter merge attempts are not flattened output oracles.
    root
}
fn fixtures() -> [(&'static str, &'static str); 8] {
    [
        ("box-finite.disabled.xcf", "7b5f482433a3de6ad293ab5166c1544eba11a649001be44c0329bf8bb59f33c4"),
        ("box-original.disabled.xcf", "20687b9b01684c90f662d04041a1121b39c106680c21e0536169ab8f08a1f39b"),
        ("median-finite.disabled.xcf", "5650ab5415fe0bf3c105862204d5329f89e74e815d5d235395cc5fd837fe6f45"),
        ("median-gray.disabled.xcf", "2167f16fccc9fc06d50a3c07a5cf40484ca8ba1e8606d32e787a7e5954247cd5"),
        ("median-original.disabled.xcf", "fbc935b0e9e75252b5881209d80b9fd930576a3704488d63747e5b361a21c18f"),
        ("motion-original.disabled.xcf", "d92ae206abaf560d4f39fef2331a8745b86f4eb6729d67856b0092a3acdebb50"),
        ("pixelize-original.disabled.xcf", "3633599e6bf83231e5329a93b8ebb99f54aef7b4d7170bd6cef7fc13fb75303e"),
        ("raster-finite.disabled.xcf", "45125afc92651786a148d29302a812df3bdcac20224d21ce4bd2beee40d916ce"),
    ]
}
fn limits() -> photocraft_xcf::ResourceLimits {
    photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap()
}
fn q() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[test]
#[ignore = "UNRUN: actual eight saved disabled originals; accepted fresh storage/activation controls pinned"]
fn actual_disabled_saved_sources_use_owned_only_mapping_and_preserve_all_native_words() {
    let packet = root();
    for (name, pin) in fixtures() {
        let bytes = std::fs::read(packet.join("author-v4/outputs").join(name)).unwrap();
        assert_eq!(hash(&bytes), pin);
        let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let family = NeighborhoodSourceFamily::hint(&original).unwrap();
        assert!(family.qualify(&original, &Interrupt::NONE, None).unwrap());
        let effect = &original.layers[0].effects[0];
        assert_eq!(effect.state.enabled, Some(false));
        // Existing default/eager path deliberately retains its stricter policy.
        assert!(crate::import(name, &bytes).is_err());
        let quota = q();
        let before = quota.usage().unwrap();
        let starts = AtomicU64::new(0);
        let progress = |v: f32| {
            if v.to_bits() == 0.05f32.to_bits() {
                starts.fetch_add(1, Ordering::Relaxed);
            }
        };
        let options = DecodeOptions { progress: Some(&progress), ..Default::default() };
        let imported = crate::import_owned_xcf_bytes_in_quota(name, Arc::new(bytes.clone()), &options, &limits(), quota.clone(), &control()).unwrap();
        assert_eq!(starts.load(Ordering::Relaxed), 1);
        let crate::OwnedXcfImport::Point(imported) = imported else { panic!("same genuine structural holder") };
        let owner = imported.document();
        owner.validate_readonly_point_storage(&control()).unwrap();
        assert!(owner.quota().shares_ledger(&quota));
        let doc = owner.document();
        let state = doc.xcf_state.as_ref().unwrap();
        let [stack] = state.filter_stacks.as_slice() else { panic!("singleton stack") };
        let [node] = stack.nodes.as_slice() else { panic!("singleton node") };
        assert!(!node.enabled);
        assert_eq!(node.operation.name, effect.operation);
        assert_eq!(Some(node.operation.stored_version.as_str()), effect.version.as_deref());
        assert_eq!(node.source_clipping, effect.state.clipping);
        assert_eq!(node.source_region, effect.state.region);
        assert_eq!(state.current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
        let composition = node.composition.as_ref().unwrap();
        assert_eq!(Some(composition.stored_blend_space), effect.state.blend_space);
        assert_eq!(Some(composition.stored_composite_space), effect.state.composite_space);
        assert_eq!(Some(composition.stored_operator), effect.state.composite_mode);
        let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("actual charged native backing") };
        let raw = original.layers[0].plane.as_ref().unwrap();
        let bpp = layer.checked_view().format().bytes_per_pixel();
        let mut actual = vec![0; raw.bytes.len()];
        for y in 0..doc.size.height {
            for x in 0..doc.size.width {
                let at = (y as usize * doc.size.width as usize + x as usize) * bpp;
                layer.checked_view().read_logical_pixel_native_into(x as i32, y as i32, &mut actual[at..at + bpp], &control()).unwrap();
            }
        }
        assert_eq!(actual, raw.bytes);
        assert_eq!(state.filter_masks.len(), usize::from(effect.mask.is_some()));
        if let Some(original_mask) = &effect.mask {
            let current = &state.filter_masks[0];
            assert_eq!(current.surface.try_read_interleaved(current.bounds).unwrap(), original_mask.plane.as_ref().unwrap().bytes);
        }
        assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, original.precision);
        assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
        assert!(imported.source_read_only());
        drop(imported);
        assert_eq!(quota.usage().unwrap(), before);
        assert_eq!(hash(&std::fs::read(packet.join("author-v4/outputs").join(name)).unwrap()), pin);
    }
}
#[test]
#[ignore = "UNRUN: actual saved disabled bytes fail atomically under caller control and real quota denial"]
fn saved_disabled_source_control_and_quota_refusals_refund_real_holds() {
    let packet = root();
    let (name, pin) = fixtures()[0];
    let bytes = Arc::new(std::fs::read(packet.join("author-v4/outputs").join(name)).unwrap());
    assert_eq!(hash(&bytes), pin);
    let quota = q();
    let before = quota.usage().unwrap();
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(crate::import_owned_xcf_bytes_in_quota(name, bytes.clone(), &Default::default(), &limits(), quota.clone(), &deny).is_err());
    assert_eq!(quota.usage().unwrap(), before);
    let stop = || true;
    let token = Interrupt::cancel_only(&stop);
    let cancelled = ReadControl { interrupt: &token, work: &|_| Ok(()) };
    assert!(crate::import_owned_xcf_bytes_in_quota(name, bytes.clone(), &Default::default(), &limits(), quota.clone(), &cancelled).is_err());
    assert_eq!(quota.usage().unwrap(), before);
    let fee = quota.reserve(QuotaKind::Index, (32 << 20) - before.index).unwrap();
    assert!(crate::import_owned_xcf_bytes_in_quota(name, bytes.clone(), &Default::default(), &limits(), quota.clone(), &control()).is_err());
    drop(fee);
    assert_eq!(quota.usage().unwrap(), before);
    assert_eq!(hash(&bytes), pin);
}
