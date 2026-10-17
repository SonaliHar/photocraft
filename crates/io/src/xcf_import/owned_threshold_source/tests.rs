//! UNRUN same-catalog promotion, capacity census and atomic-failure drafts.
use super::*;
use photocraft_doc::{
    Size,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, XcfState},
};
use photocraft_raster::backing::quota::{QuotaKind, QuotaLimits};
use std::sync::atomic::{AtomicU64, Ordering};
fn limits() -> photocraft_xcf::ResourceLimits {
    photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap()
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn synthetic() -> Document {
    let mut doc = Document::new("census-only", Size::new(8, 6), ColorMode::Rgb, SampleType::F64);
    let surface = Surface::try_from_interleaved(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), doc.bounds(), &vec![0; 8 * 6 * 32]).unwrap();
    let layer = Layer::new("native", LayerContent::Raster(surface));
    let id = layer.id;
    doc.layers.push(layer);
    let mut state = XcfState::default();
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(id),
        revision: 0,
        nodes: vec![FilterNode {
            id: FilterId(1),
            operation: OperationIdentity {
                name: "gimp:threshold-alpha".into(),
                stored_version: "0:0".into(),
                schema_epoch: "xcf-22-stock-threshold-alpha-precision-v1".into(),
            },
            source_display_name: Some("display".into()),
            source_icon: None,
            arguments: vec![FilterArgument { name: "value".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits())) }],
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(2),
            source_region: Some(0),
            mask: None,
            auxiliary: Vec::new(),
            composition: None,
        }],
    });
    state.items.push(ItemState {
        item: ItemRef::Layer(id),
        name: "native".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(doc.bounds()),
        mask: None,
        composition: None,
    });
    doc.xcf_state = Some(state);
    doc
}
#[test]
fn threshold_capacity_census_prices_spare_nested_storage_without_widening_plain() {
    let budget = photocraft_xcf::Budget::new(limits(), None);
    let mut doc = synthetic();
    assert!(census::measure_plain_for_test(&doc, &budget, &Interrupt::NONE).is_err());
    let a = census::measure_threshold(&doc, &budget, &Interrupt::NONE).unwrap();
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.arguments.reserve_exact(37);
    node.source_display_name.as_mut().unwrap().reserve_exact(4096);
    let b = census::measure_threshold(&doc, &budget, &Interrupt::NONE).unwrap();
    assert!(b.model > a.model + 4096);
    let mut surface = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &(-0.0f64).to_ne_bytes()).unwrap();
    surface.try_write_pixel_f64(0, 0, &[0.5 + 2f64.powi(-40)]).unwrap();
    let state = doc.xcf_state.as_mut().unwrap();
    state.filter_masks.push(photocraft_doc::FilterMask { id: MaskId(2), owner: FilterId(1), surface, bounds: Rect::new(0, 0, 8, 6) });
    state.items.push(state.items[0].clone());
    let c = census::measure_threshold(&doc, &budget, &Interrupt::NONE).unwrap();
    assert!(c.model > b.model + 256);
    assert!(census::measure_plain_for_test(&doc, &budget, &Interrupt::NONE).is_err());
    // Full padded F64 mask, not its 48 logical samples, raises native phase work.
    let withmask = phase_work(&doc, c.model, c.payload, 0, &budget).unwrap();
    state_ref_remove_mask(&mut doc);
    let without = phase_work(&doc, c.model, c.payload, 0, &budget).unwrap();
    assert!(withmask - without >= 2 * 65536 * 8);
}
fn state_ref_remove_mask(doc: &mut Document) {
    doc.xcf_state.as_mut().unwrap().filter_masks.clear();
}
#[test]
fn parent_quota_and_work_denials_refund_storage_without_resetting_catalog() {
    let q = quota();
    let outside = q.reserve(QuotaKind::Index, 4096).unwrap();
    let baseline = q.usage().unwrap();
    let malformed = Arc::new(b"gimp xcf v022\0".to_vec());
    let small = limits().lowered(&[("residentBytes", 1024), ("indexBytes", 1024)]).unwrap();
    let block = q.reserve(QuotaKind::Resident, (256 << 20) - 4096 - 512).unwrap();
    assert!(import_owned_staged_threshold_alpha_bytes_in_quota("x", malformed.clone(), &Default::default(), &small, q.clone(), &control()).is_err());
    drop(block);
    assert_eq!(q.usage().unwrap(), baseline);
    let spent = AtomicU64::new(0);
    let work = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Err(BackingError::Work)
    };
    assert!(
        import_owned_staged_threshold_alpha_bytes_in_quota(
            "x",
            malformed.clone(),
            &Default::default(),
            &limits(),
            q.clone(),
            &ReadControl { interrupt: &Interrupt::NONE, work: &work }
        )
        .is_err()
    );
    assert!(spent.load(Ordering::Relaxed) > 0);
    assert_eq!(q.usage().unwrap(), baseline);
    let stop = || true;
    assert!(matches!(
        import_owned_staged_threshold_alpha_bytes_in_quota(
            "x",
            malformed,
            &Default::default(),
            &limits(),
            q.clone(),
            &ReadControl { interrupt: &Interrupt::cancel_only(&stop), work: &|_| Ok(()) }
        ),
        Err(IoError::Xcf(_)) | Err(IoError::Cancelled)
    ));
    assert_eq!(q.usage().unwrap(), baseline);
    drop(outside);
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "UNRUN: requires independently saved pinned Threshold Alpha precision sources"]
fn actual_pinned_sources_promote_same_native_layer_mask_and_refund_catalog() {
    use sha2::{Digest, Sha256};
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect::<String>();
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("alpha-threshold-precision-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    let q = quota();
    let outside = q.reserve(QuotaKind::Index, 4096).unwrap();
    let initial = q.usage().unwrap();
    for row in proof["checks"].as_array().unwrap() {
        let name = format!("{}.xcf", row["id"].as_str().unwrap());
        let bytes = Arc::new(std::fs::read(root.join(&name)).unwrap());
        assert_eq!(hash(&bytes), manifest[&name].as_str().unwrap());
        let raw = photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap();
        let owned = import_owned_staged_threshold_alpha_bytes_in_quota(&name, bytes, &Default::default(), &limits(), q.clone(), &control()).unwrap();
        assert!(owned.document().quota().shares_ledger(&q));
        owned.document().validate_readonly_threshold_alpha_storage(&control()).unwrap();
        let doc = owned.document().document();
        let source = raw.layers[0].plane.as_ref().unwrap();
        let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("not backed") };
        let mut pixels = vec![0u8; source.bytes.len()];
        let bpp = layer.format().bytes_per_pixel();
        for y in 0..6 {
            for x in 0..8 {
                let offset = (y * 8 + x) * bpp;
                layer.checked_view().read_pixel_native_into(x as i32, y as i32, &mut pixels[offset..offset + bpp], &control()).unwrap();
            }
        }
        assert_eq!(pixels, source.bytes);
        let mask = &doc.xcf_state.as_ref().unwrap().filter_masks[0];
        let original = raw.layers[0].effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap();
        assert_eq!(mask.surface.try_read_interleaved(doc.bounds()).unwrap(), original.bytes);
        let node = doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
        assert_eq!(node.operation.schema_epoch, "xcf-22-stock-threshold-alpha-precision-v1");
        let detached = owned.into_accounted_document();
        detached.validate_readonly_threshold_alpha_storage(&control()).unwrap();
        assert!(q.usage().unwrap().resident > initial.resident);
        drop(detached);
        assert_eq!(q.usage().unwrap(), initial);
    }
    drop(outside);
}

#[path = "geometry_tests.rs"]
mod geometry;
