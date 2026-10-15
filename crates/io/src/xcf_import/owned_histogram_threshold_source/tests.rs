//! UNRUN exact Histogram Threshold promotion/capacity/failure drafts.
use super::super::owned_plain_source::census;
use super::*;
use photocraft_doc::{
    SourceColorContext,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, XcfState},
};
use photocraft_raster::backing::quota::{QuotaKind, QuotaLimits};
use photocraft_xcf::{Budget, ResourceLimits};
fn limits() -> ResourceLimits {
    ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)]).unwrap()
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn census_fixture() -> Document {
    let mut doc = Document::new("census-only, not original source authority", Size::new(8, 6), ColorMode::Rgb, SampleType::F64);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 750 });
    let layer =
        Layer::new("native", LayerContent::Raster(Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), &[0; 32]).unwrap()));
    let id = layer.id;
    doc.layers.push(layer);
    let mut state = XcfState::default();
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
    state.items.push(state.items[0].clone());
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(id),
        revision: 0,
        nodes: vec![FilterNode {
            id: FilterId(1),
            operation: OperationIdentity {
                name: "gimp:threshold".into(),
                stored_version: "0:0".into(),
                schema_epoch: "xcf-22-stock-threshold-precision-v1".into(),
            },
            source_display_name: None,
            source_icon: None,
            arguments: vec![
                FilterArgument { name: "channel".into(), value: FilterValue::Enum { domain: "GimpHistogramChannel".into(), value: 0 } },
                FilterArgument { name: "low".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.25f32.to_bits())) },
                FilterArgument { name: "high".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.75f32.to_bits())) },
            ],
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(MaskId(2))),
            auxiliary: Vec::new(),
            composition: None,
        }],
    });
    state.filter_masks.push(photocraft_doc::FilterMask {
        id: MaskId(2),
        owner: FilterId(1),
        bounds: doc.bounds(),
        surface: Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false)),
    });
    doc.xcf_state = Some(state);
    doc
}
#[test]
fn own_census_prices_enum_capacity_and_refuses_cross_family_or_widened_shape() {
    let budget = Budget::new(limits(), None);
    let mut doc = census_fixture();
    let a = census::measure_histogram_threshold(&doc, &budget, &Interrupt::NONE).unwrap();
    assert!(census::measure_threshold(&doc, &budget, &Interrupt::NONE).is_err());
    let FilterValue::Enum { domain, .. } = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value else { panic!() };
    domain.reserve_exact(4096);
    let b = census::measure_histogram_threshold(&doc, &budget, &Interrupt::NONE).unwrap();
    assert!(b.model >= a.model + 4096);
    doc.sample_encoding = Some(SampleEncoding::Linear);
    assert!(census::measure_histogram_threshold(&doc, &budget, &Interrupt::NONE).is_err());
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.size = Size::new(9, 6);
    assert!(census::measure_histogram_threshold(&doc, &budget, &Interrupt::NONE).is_err());
    let exhausted = Budget::new(limits().lowered(&[("sampleWorkUnits", 0)]).unwrap(), None);
    assert!(census::measure_histogram_threshold(&census_fixture(), &exhausted, &Interrupt::NONE).is_err());
}
#[test]
fn selected_entry_denials_leave_existing_catalog_fees_and_source_intact() {
    let q = quota();
    let outside = q.reserve(QuotaKind::Index, 4096).unwrap();
    let baseline = q.usage().unwrap();
    let bytes = Arc::new(b"gimp xcf v022\0".to_vec());
    let original = bytes.clone();
    let work = |_| Err(photocraft_raster::backing::BackingError::Work);
    assert!(
        super::super::owned_xcf_source::import_owned_xcf_bytes_in_quota(
            "x",
            bytes.clone(),
            &Default::default(),
            &limits(),
            q.clone(),
            &ReadControl { interrupt: &Interrupt::NONE, work: &work }
        )
        .is_err()
    );
    assert_eq!(q.usage().unwrap(), baseline);
    let cancelled = Interrupt::cancel_only(&|| true);
    assert!(
        super::super::owned_xcf_source::import_owned_xcf_bytes_in_quota(
            "x",
            bytes,
            &Default::default(),
            &limits(),
            q.clone(),
            &ReadControl { interrupt: &cancelled, work: &|_| Ok(()) }
        )
        .is_err()
    );
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(original.as_slice(), b"gimp xcf v022\0");
    drop(outside);
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "UNRUN: requires pinned independently saved Histogram Threshold precision originals"]
fn real_sources_select_histogram_once_and_preserve_native_owner_mask_and_original_identity() {
    use sha2::{Digest, Sha256};
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect::<String>();
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("threshold-precision-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "23015e327f45c490ed6dbeaa6f4a9541b13dbe8526e9abfff26da13d38c56142");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    let q = quota();
    let outside = q.reserve(QuotaKind::Index, 4096).unwrap();
    let baseline = q.usage().unwrap();
    let mut depths = std::collections::BTreeSet::new();
    for row in proof["checks"].as_array().unwrap() {
        let name = format!("{}.xcf", row["id"].as_str().unwrap());
        let bytes = Arc::new(std::fs::read(root.join(&name)).unwrap());
        assert_eq!(hash(&bytes), manifest[&name].as_str().unwrap());
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let supported = raw.model == Model::Rgb && raw.encoding == Encoding::NonlinearProfile && matches!(raw.precision, 250 | 650 | 750);
        let result =
            super::super::owned_xcf_source::import_owned_xcf_bytes_in_quota(&name, bytes.clone(), &Default::default(), &limits(), q.clone(), &control());
        if !supported {
            assert!(result.is_err());
            assert_eq!(q.usage().unwrap(), baseline);
            continue;
        }
        let owned = result.unwrap();
        assert!(matches!(&owned, super::super::owned_xcf_source::OwnedXcfImport::HistogramThreshold(_)));
        owned.document().validate_readonly_threshold_storage(&control()).unwrap();
        assert!(owned.document().quota().shares_ledger(&q));
        depths.insert(raw.precision);
        let doc = owned.document().document();
        let source = raw.layers[0].plane.as_ref().unwrap();
        let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!() };
        let mut native = vec![0; source.bytes.len()];
        let bpp = layer.format().bytes_per_pixel();
        for y in 0..6 {
            for x in 0..8 {
                let offset = (y * 8 + x) * bpp;
                layer.checked_view().read_logical_pixel_native_into(x as i32, y as i32, &mut native[offset..offset + bpp], &control()).unwrap();
            }
        }
        assert_eq!(native, source.bytes);
        let state = doc.xcf_state.as_ref().unwrap();
        let mask = &state.filter_masks[0];
        assert_eq!(mask.surface.try_read_interleaved(doc.bounds()).unwrap(), raw.layers[0].effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap().bytes);
        let node = &state.filter_stacks[0].nodes[0];
        assert_eq!(node.arguments.len(), 3);
        assert_eq!(node.operation.schema_epoch, "xcf-22-stock-threshold-precision-v1");
        assert_eq!(node.composition.as_ref().unwrap().stored_operator, -1565818287);
        assert_eq!(doc.source_color_context.as_ref().unwrap().original_icc, None);
        assert!(owned.source_read_only());
        let sourcebytes = bytes.clone();
        drop(owned);
        assert_eq!(q.usage().unwrap(), baseline);
        assert_eq!(hash(&sourcebytes), manifest[&name].as_str().unwrap());
    }
    assert_eq!(depths, std::collections::BTreeSet::from([250, 650, 750]));
    drop(outside);
}
