//! Ignored local witness contracts. Genuine encoded/capture/packet buffers are
//! external test inputs; only actual reader publication claims paid custody.
use super::*;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaLimits},
};
use photocraft_xcf::{BudgetKind, ByteSource, PlainSourceAccounting};
use sha2::{Digest, Sha256};
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn binding(v: &serde_json::Value) -> Vec<u8> {
    let raw = v["path"].as_str().unwrap().replace('\\', "/");
    let suffix = raw.split_once("/temp/").unwrap().1;
    let bytes = std::fs::read(root().join("temp").join(suffix)).unwrap();
    assert_eq!(bytes.len() as u64, v["bytes"].as_u64().unwrap());
    assert_eq!(Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), v["sha256"].as_str().unwrap());
    bytes
}
fn cases() -> serde_json::Value {
    let b = std::fs::read(root().join("temp/root-mixed-mask-research-20261016/literal-index-v1.json")).unwrap();
    assert_eq!(
        Sha256::digest(&b).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "4d64c41a6460a57e96673e04fdfd79d62183f0808d9ffe1e7db0a34ec442d43e"
    );
    serde_json::from_slice(&b).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None)
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn span<'a>(packet: &'a [u8], v: &serde_json::Value) -> &'a [u8] {
    let off = v["offset"].as_u64().unwrap() as usize;
    let len = v["bytes"].as_u64().unwrap() as usize;
    let b = &packet[off..off + len];
    assert_eq!(Sha256::digest(b).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), v["sha256"].as_str().unwrap());
    b
}
#[test]
#[ignore = "UNRUN: eight genuine ordered/multiroot sources actualownednewclass publication/native/raw/mask/profile/sourceorder+sameq; no numerical/reload/defaultpadding theorem"]
fn genuine_sources_preserve_node_mask_and_native_custody() {
    use photocraft_doc::{
        LayerContent,
        item_identity::ItemRef,
        xcf_filters::{ExactNumber, FilterValue},
    };
    let table = cases();
    for case in table["cases"].as_array().unwrap() {
        let bytes = binding(&case["source"]);
        let _capture = binding(&case["capture"]);
        let packet = binding(&case["packet"]);
        let b = budget();
        let plan = preplan(&mut ByteSource::new(&bytes), &Default::default(), &b, PlainSourceAccounting::BorrowedMemory, &Interrupt::NONE).unwrap().unwrap();
        assert!(plan.mixed_upgrade());
        drop(plan);
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
        let c = ctl();
        let source = crate::import_owned_xcf_from_reader_in_quota(
            case["id"].as_str().unwrap(),
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            &Default::default(),
            &photocraft_xcf::ResourceLimits::for_policy(Default::default())
                .unwrap()
                .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
                .unwrap(),
            q.clone(),
            &c,
        )
        .unwrap();
        let crate::OwnedXcfImport::MaskedFilteredFlat(source) = source else { panic!("actual newcarrier") };
        assert!(source.document().is_readonly_masked_filtered_flat_point_storage());
        assert!(!source.document().is_readonly_filtered_flat_point_storage());
        let d = source.document().document();
        let state = d.xcf_state.as_ref().unwrap();
        assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
        assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, case["source_precision"].as_u64().unwrap() as u32);
        let raws = case["raw"]["roots"].as_array().unwrap();
        assert_eq!(d.layers.len(), raws.len());
        for (layer, raw) in d.layers.iter().zip(raws.iter().rev()) {
            let LayerContent::BackedRaster(native) = &layer.content else { panic!("real native") };
            let expected = span(&packet, &raw["stored_source"]);
            let mut actual = Vec::new();
            let mut pixel = vec![0; if d.depth == photocraft_color::SampleType::F64 { 32 } else { 16 }];
            for y in 0..d.size.height {
                for x in 0..d.size.width {
                    native.checked_view().read_pixel_native_into(x as i32, y as i32, &mut pixel, &c).unwrap();
                    actual.extend_from_slice(&pixel);
                }
            }
            assert_eq!(actual, expected);
            let stack = state.filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(layer.id)).unwrap();
            let nodes = raw["canonical_effects"].as_array().unwrap();
            assert_eq!(stack.nodes.len(), nodes.len());
            for (n, r) in stack.nodes.iter().zip(nodes) {
                assert_eq!(n.operation.name, r["operation"].as_str().unwrap());
                assert_eq!(n.operation.stored_version, r["version"].as_str().unwrap());
                assert_eq!(n.opacity_f64_bits, 1f64.to_bits());
                let ItemRef::Mask(id) = n.mask.unwrap() else { panic!("own mask") };
                let mask = state.filter_masks.iter().find(|m| m.id == id && m.owner == n.id).unwrap();
                let expected = span(&packet, &r["stored_mask"]["stored_native"]);
                assert_eq!(mask.surface.try_read_interleaved(mask.bounds).unwrap(), expected);
                let nonempty = expected.chunks_exact(if d.depth == photocraft_color::SampleType::F64 { 8 } else { 4 }).any(|w| w.iter().any(|v| *v != 0));
                assert_eq!(n.operation.schema_epoch.contains("-masked-mixed-wide-v1"), nonempty);
                assert_eq!(n.composition.as_ref().unwrap().compatibility_epoch, n.operation.schema_epoch);
                for prop in r["properties"].as_array().unwrap() {
                    let tag = prop["tag"].as_u64().unwrap();
                    let off = prop["offset"].as_u64().unwrap();
                    let hex = prop["hex"].as_str().unwrap();
                    let kept = state
                        .opaque
                        .iter()
                        .find(|v| v.owner == Some(ItemRef::Layer(layer.id)) && v.identity == format!("{tag}@{off}:{}", hex.len() / 2))
                        .unwrap();
                    assert_eq!(kept.bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(), hex);
                }
                for a in &n.arguments {
                    assert!(matches!(a.value, FilterValue::Number(ExactNumber::Float32Bits(_))));
                }
            }
        }
        assert!(q.usage().unwrap().resident > 0);
        drop(source);
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
    }
}
#[test]
#[ignore = "UNRUN: metadata-only preplan actual descriptors/caps admitted before builders, mismatches/partial counts/work denial remain typed; capmiss oldonly no fabricatedowner"]
fn preallocation_descriptors_and_cap_misses_are_final() {
    let table = cases();
    let bytes = binding(&table["cases"][0]["source"]);
    let b = budget();
    let mut plan = preplan(&mut ByteSource::new(&bytes), &Default::default(), &b, PlainSourceAccounting::BorrowedMemory, &Interrupt::NONE).unwrap().unwrap();
    assert!(plan.complete().is_err());
    let mut wrong = plan.descriptors[0].clone();
    wrong.offset.0 = 1;
    assert!(plan.verify(&wrong, &b).is_err());
    assert_eq!(plan.next, 0);
    let expected = plan.descriptors.clone();
    for d in &expected {
        plan.verify(d, &b).unwrap();
    }
    plan.complete().unwrap();
    assert!(plan.verify(&expected[0], &b).is_err());
    drop(plan);
    let mut huge = bytes.clone();
    huge[14..18].copy_from_slice(&8192u32.to_be_bytes());
    huge[18..22].copy_from_slice(&8192u32.to_be_bytes());
    assert!(preplan(&mut ByteSource::new(&huge), &Default::default(), &budget(), PlainSourceAccounting::BorrowedMemory, &Interrupt::NONE).unwrap().is_none());
    let low = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 1)]).unwrap();
    assert!(
        preplan(
            &mut ByteSource::new(&bytes),
            &Default::default(),
            &photocraft_xcf::Budget::new(low, None),
            PlainSourceAccounting::BorrowedMemory,
            &Interrupt::NONE
        )
        .is_err()
    );
    assert!(matches!(
        preplan(&mut ByteSource::new(&bytes), &Default::default(), &budget(), PlainSourceAccounting::BorrowedMemory, &Interrupt::cancel_only(&|| true)),
        Err(IoError::Cancelled) | Err(IoError::Xcf(_))
    ));
}
#[test]
#[ignore = "UNRUN: decoded external malformed coverage/Source tuples are failclosed, nonempty epochs distinct, signedzero-only sibling and SourceOpacity refused; no fake producer or incomingreceipts"]
fn decoded_contracts_do_not_widen_source_or_identity() {
    let table = cases();
    let bytes = binding(&table["cases"][0]["source"]);
    let mut f = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
    qualify(&f, &Interrupt::NONE, &budget()).unwrap();
    let e = &f.layers[0].effects[0];
    let k = crate::xcf_filters::masked_mixed_source::parameters(e).unwrap();
    assert_ne!(k.epoch(), photocraft_compose::INVERT_GAMMA_MASKED_EPOCH);
    f.layers[0].effects[0].state.opacity_bits = Some(0.375f32.to_bits());
    assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
    let mut f = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
    let p = f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap();
    p.bytes = (-0f32).to_ne_bytes().repeat((p.width * p.height) as usize);
    assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
    let mut f = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
    f.layers[0].effects[0].operation = "gegl:vibrance".into();
    assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
    let mut f = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
    f.layers[0].plane.as_mut().unwrap().bytes.pop();
    assert!(qualify(&f, &Interrupt::NONE, &budget()).is_err());
}
#[test]
#[ignore = "UNRUN: Source proof unique originalIndex lease/no qdoublehold, original cumulative Work/index denial/cancel and refund, no newquota/reset or freeproofVec"]
fn original_index_lease_and_control_are_effective() {
    let table = cases();
    let bytes = binding(&table["cases"][0]["source"]);
    let b = budget();
    let meta =
        photocraft_xcf::read_plain_source_metadata_with_budget(&mut ByteSource::new(&bytes), &Default::default(), &b, PlainSourceAccounting::BorrowedMemory)
            .unwrap();
    let one = b.used(BudgetKind::Work);
    drop(meta);
    let meta =
        photocraft_xcf::read_plain_source_metadata_with_budget(&mut ByteSource::new(&bytes), &Default::default(), &b, PlainSourceAccounting::BorrowedMemory)
            .unwrap();
    assert_eq!(b.used(BudgetKind::Work), 2 * one);
    drop(meta);
    assert_eq!(b.used(BudgetKind::Resident), 0);
    let fee = b.reserve(BudgetKind::Index, 256, "external proof lease contract").unwrap();
    let mut held = CoverageLease(fee);
    assert_eq!(photocraft_doc::xcf_filter_masks::MaskCoverageSetLease::bytes(&held), 256);
    assert!(photocraft_doc::xcf_filter_masks::MaskCoverageSetLease::reconcile_capacity(&mut held, 257).is_err());
    drop(held);
    assert_eq!(b.used(BudgetKind::Index), 0);
    let q = BackingQuota::new(QuotaLimits { resident: 1, index: 1, scratch: 0 });
    assert!(
        crate::import_owned_xcf_from_reader_in_quota(
            "denied",
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            &Default::default(),
            &photocraft_xcf::ResourceLimits::for_policy(Default::default())
                .unwrap()
                .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
                .unwrap(),
            q.clone(),
            &ctl()
        )
        .is_err()
    );
    assert_eq!(q.usage().unwrap().resident, 0);
    assert_eq!(q.usage().unwrap().index, 0);
}
#[test]
#[ignore = "UNRUN: actual ordinary masked5 F64white original intake now uses descriptor parsepermission without mixedupgrade; default eagerdecoder staysStrict and oldPoint Source/backend remain final"]
fn ordinary_f64_preimage_permission_keeps_old_point_purpose() {
    let raw = std::fs::read(root().join("temp/sol1-four-masked-point-literal-bindings-20261016/literal-index-v1.json")).unwrap();
    assert_eq!(
        Sha256::digest(&raw).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "f21b945a5957539a5d645fa87a87a8012bf00de2a32cdb12a3d0b3520716c017"
    );
    let table: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    for case in table["nonempty_cases"].as_array().unwrap().iter().filter(|c| c["source_precision"] == 750) {
        let bytes = binding(&case["source"]);
        assert!(photocraft_xcf::decode(&bytes, &Default::default()).is_err());
        let b = budget();
        let plan = preplan(&mut ByteSource::new(&bytes), &Default::default(), &b, PlainSourceAccounting::BorrowedMemory, &Interrupt::NONE).unwrap().unwrap();
        assert!(!plan.mixed_upgrade());
        drop(plan);
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
        let imported = crate::import_owned_xcf_from_reader_in_quota(
            "ordinary F64",
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            &Default::default(),
            &photocraft_xcf::ResourceLimits::for_policy(Default::default())
                .unwrap()
                .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
                .unwrap(),
            q.clone(),
            &ctl(),
        )
        .unwrap();
        let crate::OwnedXcfImport::Point(source) = imported else { panic!("ordinary Point") };
        assert!(source.document().is_readonly_point_storage());
        assert!(!source.document().is_readonly_masked_filtered_flat_point_storage());
        drop(source);
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
    }
}
