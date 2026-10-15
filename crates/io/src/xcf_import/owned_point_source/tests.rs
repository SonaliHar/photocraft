//! UNRUN Point storage, accounting, geometry and sealed-source promotion drafts.
use super::*;
use photocraft_doc::{
    SampleEncoding, SourceColorContext,
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
    let mut mask_item = state.items[0].clone();
    mask_item.item = ItemRef::Mask(MaskId(2));
    state.items.push(mask_item);
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(id),
        revision: 0,
        nodes: vec![FilterNode {
            id: FilterId(1),
            operation: OperationIdentity {
                name: "gegl:alien-map".into(),
                stored_version: "0:0".into(),
                schema_epoch: "census-only-not-source-authority".into(),
            },
            source_display_name: None,
            source_icon: None,
            arguments: vec![
                FilterArgument { name: "color-model".into(), value: FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: 0 } },
                FilterArgument { name: "cpn-1-frequency".into(), value: FilterValue::Number(ExactNumber::Float32Bits(2f32.to_bits())) },
                FilterArgument { name: "cpn-1-keep".into(), value: FilterValue::Bool(false) },
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
fn actual_spare_nested_capacity_is_counted_without_widening_plain_or_threshold() {
    let budget = Budget::new(limits(), None);
    let mut doc = census_fixture();
    let first = census::measure(&doc, &budget, &Interrupt::NONE).unwrap();
    assert!(super::super::owned_plain_source::census::measure(&doc, &budget, &Interrupt::NONE).is_err());
    assert!(super::super::owned_plain_source::census::measure_threshold(&doc, &budget, &Interrupt::NONE).is_err());
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.arguments.reserve_exact(32);
    node.source_display_name = Some(String::with_capacity(4096));
    let FilterValue::Enum { domain, .. } = &mut node.arguments[0].value else { panic!() };
    domain.reserve_exact(8192);
    let next = census::measure(&doc, &budget, &Interrupt::NONE).unwrap();
    assert!(next.model >= first.model + 12288);
    assert_eq!(next.layers, 1);
}
#[test]
fn negative_drawable_rectangle_is_not_canvas_or_sparse_occupancy() {
    let budget = Budget::new(limits(), None);
    let mut doc = census_fixture();
    let extent = Rect::new(-7, 5, 12, 18);
    doc.xcf_state.as_mut().unwrap().items[0].raster_bounds = Some(extent);
    let id = doc.layers[0].id;
    assert_eq!(census::raster_bounds(&doc, id).unwrap(), extent);
    census::measure(&doc, &budget, &Interrupt::NONE).unwrap();
    let canvas = doc.bounds();
    let LayerContent::Raster(surface) = &mut doc.layers[0].content else { panic!() };
    surface.try_write_pixel_f64(-7, 5, &[0.25, 0.5, 0.75, 1.0]).unwrap();
    let actual = store::validate(surface, extent, &control(), &|| Ok(())).unwrap();
    assert_eq!(actual.bounds(), extent);
    assert!(store::validate(surface, canvas, &control(), &|| Ok(())).is_err());
}
#[test]
fn flat_argument_and_mask_identity_refusals_precede_any_promotion() {
    let budget = Budget::new(limits(), None);
    for change in 0..4 {
        let mut doc = census_fixture();
        let state = doc.xcf_state.as_mut().unwrap();
        match change {
            0 => state.filter_stacks[0].nodes[0].arguments[0].value = FilterValue::Config(Vec::new()),
            1 => state.filter_stacks[0].nodes[0].mask = None,
            2 => state.filter_masks[0].bounds = Rect::new(1, 0, 9, 6),
            _ => state.items.push(state.items[0].clone()),
        }
        assert!(census::measure(&doc, &budget, &Interrupt::NONE).is_err());
    }
}
#[test]
fn full_native_mask_tiles_are_prepaid_and_failed_work_is_not_reset() {
    let budget = Budget::new(limits(), None);
    let mut doc = census_fixture();
    let before = phase_work(&doc, 0, 0, 0, &budget).unwrap();
    doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(0, 0, &[0.5]).unwrap();
    let after = phase_work(&doc, 0, 0, 0, &budget).unwrap();
    assert!(after - before >= 2 * 65536 * 8 + 512);
    let low = Budget::new(limits().lowered(&[("sampleWorkUnits", after - 1)]).unwrap(), None);
    assert!(prepay(&low, after).is_err());
    assert_eq!(low.used(BudgetKind::Work), 0);
    prepay(&budget, after).unwrap();
    assert!(budget.used(BudgetKind::Work) >= after);
    let cancelled = Interrupt::cancel_only(&|| true);
    assert!(matches!(census::measure(&doc, &budget, &cancelled), Err(IoError::Cancelled)));
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "UNRUN: pinned independently saved Point originals, real selected owner and negative geometry"]
fn original_nine_branches_keep_actual_drawable_native_words_and_catalog_identity() {
    use sha2::{Digest, Sha256};
    let hash = |b: &[u8]| Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect::<String>();
    let base = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let q = quota();
    let retained = q.reserve(QuotaKind::Index, 4096).unwrap();
    let baseline = q.usage().unwrap();
    let mut families = std::collections::BTreeSet::new();
    let mut saw_offset = false;
    for (folder, pin) in [
        ("point-replace-geometry-evidence", "e36bcddcea3021cff305175f70d1bd44f865478d8019a48dcb5c912b9f83bf72"),
        ("point-invert-linear-evidence", "d74dd597c9ebc953ebce9c04722c3ce3145c0b495623ff0554590b7ff68eefd8"),
        ("point-invert-gamma-evidence", "6c905ae85375807b57df97b47579461d5986c41c2e008498abd82e4ac56b5daf"),
    ] {
        let root = base.join(folder);
        let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(hash(&manifest), pin);
        let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        for asset in manifest["artifacts"].as_array().unwrap() {
            let path = asset["path"].as_str().unwrap();
            if !path.ends_with(".xcf") {
                continue;
            }
            assert!(std::fs::metadata(root.join(path)).unwrap().len() <= 16 << 20);
            let bytes = Arc::new(std::fs::read(root.join(path)).unwrap());
            assert_eq!(hash(&bytes), asset["sha256"].as_str().unwrap());
            let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(family)) =
                crate::xcf_point_replace_source::classify(&raw, &Interrupt::NONE, None).unwrap()
            else {
                continue;
            };
            use crate::xcf_filters::PointReplaceFamily::*;
            if !matches!(family, Brightness | Levels | Exposure | Temperature | Sepia | AlienRgb | AlienHsl | InvertLinear | InvertGamma) {
                continue;
            }
            families.insert(match family {
                Brightness => "Brightness",
                Levels => "Levels",
                Exposure => "Exposure",
                Temperature => "Temperature",
                Sepia => "Sepia",
                AlienRgb => "AlienRgb",
                AlienHsl => "AlienHsl",
                InvertLinear => "InvertLinear",
                InvertGamma => "InvertGamma",
                _ => panic!(),
            });
            let owned =
                super::super::owned_xcf_source::import_owned_xcf_bytes_in_quota(path, bytes.clone(), &Default::default(), &limits(), q.clone(), &control())
                    .unwrap();
            assert!(matches!(&owned, super::super::owned_xcf_source::OwnedXcfImport::Point(_)));
            owned.document().validate_readonly_point_storage(&control()).unwrap();
            assert!(owned.document().quota().shares_ledger(&q));
            let doc = owned.document().document();
            let source = raw.layers[0].plane.as_ref().unwrap();
            let original = &raw.layers[0];
            let rect = Rect::from_xywh(original.offset.0, original.offset.1, original.width, original.height);
            saw_offset |= rect != doc.bounds();
            let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!() };
            assert_eq!(layer.operative_bounds(), rect);
            let bpp = layer.format().bytes_per_pixel();
            let mut actual = vec![0; source.bytes.len()];
            for y in 0..original.height {
                for x in 0..original.width {
                    let at = ((y * original.width + x) as usize) * bpp;
                    layer.checked_view().read_logical_pixel_native_into(rect.x0 + x as i32, rect.y0 + y as i32, &mut actual[at..at + bpp], &control()).unwrap();
                }
            }
            assert_eq!(actual, source.bytes);
            let state = doc.xcf_state.as_ref().unwrap();
            assert_eq!(
                state.filter_masks[0].surface.try_read_interleaved(doc.bounds()).unwrap(),
                original.effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap().bytes
            );
            assert_eq!(state.filter_stacks[0].nodes[0].operation.name, original.effects[0].operation);
            assert_eq!(state.filter_stacks[0].nodes[0].enabled, original.effects[0].state.enabled.unwrap());
            assert!(owned.source_read_only());
            drop(owned);
            assert_eq!(q.usage().unwrap(), baseline);
            assert_eq!(hash(&bytes), asset["sha256"].as_str().unwrap());
        }
    }
    assert_eq!(families.len(), 9);
    assert!(saw_offset);
    drop(retained);
}

#[test]
fn point_name_is_not_authority_and_refusal_never_retries_or_resets_catalog() {
    use super::super::plane::fixture;
    use std::sync::atomic::{AtomicU64, Ordering};
    let mut record = Vec::new();
    for text in ["unqualified zero-argument point", "", "gegl:invert-linear", "0:0"] {
        fixture::u32be(&mut record, text.len() as u32 + 1);
        record.extend_from_slice(text.as_bytes());
        record.push(0);
    }
    fixture::prop(&mut record, 0, &[]);
    fixture::pointer(&mut record, 0, 22);
    let options = fixture::FixtureOptions { effect_records: vec![record], ..Default::default() };
    let bytes = Arc::new(fixture::image_custom(22, 650, 0, 8, 6, &0.5f32.to_be_bytes().repeat(48 * 4), &options));
    let original = bytes.clone();
    let q = quota();
    let outside = q.reserve(QuotaKind::Index, 4096).unwrap();
    let baseline = q.usage().unwrap();
    let starts = AtomicU64::new(0);
    let progress = |p: f32| {
        if p.to_bits() == 0.05f32.to_bits() {
            starts.fetch_add(1, Ordering::Relaxed);
        }
    };
    let options = DecodeOptions { progress: Some(&progress), ..Default::default() };
    assert!(super::super::owned_xcf_source::import_owned_xcf_bytes_in_quota("not-qualified.xcf", bytes, &options, &limits(), q.clone(), &control()).is_err());
    assert_eq!(starts.load(Ordering::Relaxed), 1);
    assert_eq!(q.usage().unwrap(), baseline);
    let stopped = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(
        super::super::owned_xcf_source::import_owned_xcf_bytes_in_quota("not-qualified.xcf", original, &Default::default(), &limits(), q.clone(), &stopped)
            .is_err()
    );
    assert_eq!(q.usage().unwrap(), baseline);
    drop(outside);
}
