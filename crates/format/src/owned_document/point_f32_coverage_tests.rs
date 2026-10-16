//! UNRUN/uncompiled structural native contracts, not rendering/source qualification.
use super::*;
use photocraft_doc::{
    FilterMask,
    item_identity::{FilterId, MaskId},
    xcf_filters::{FilterNode, OperationIdentity},
    xcf_state::MaskState,
};
use photocraft_raster::Surface;
struct PointRetention {
    _original: photocraft_doc::accounted_document::AccountedDocument,
    _hold: Arc<FixtureHold>,
}
fn point(q: &BackingQuota, sample: SampleType, covered: bool) -> photocraft_doc::accounted_document::AccountedDocument {
    let original = current_empty(q, sample);
    // Real prospective envelope before model/profile/map/mask allocations.
    let hold = Arc::new(FixtureHold { _index: q.reserve(QuotaKind::Index, 1 << 20).unwrap(), _resident: q.reserve(QuotaKind::Resident, 8 << 20).unwrap() });
    let mut d = original.document().clone();
    if covered {
        let id = FilterId(31);
        let mid = MaskId(32);
        let bounds = d.bounds();
        let layer = d.layers[0].id;
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
        let raw = if sample == SampleType::F64 { f64::from_bits(0x3ff0000009b25010) } else { 0.375 };
        surface.try_write_pixel_f64(0, 0, &[raw]).unwrap();
        let state = d.xcf_state.as_mut().unwrap();
        state.filter_stacks[0].nodes.push(FilterNode {
            id,
            operation: OperationIdentity {
                name: "structural:coverage-only".into(),
                stored_version: "1:0".into(),
                schema_epoch: "not-source-eligibility".into(),
            },
            source_display_name: None,
            source_icon: None,
            arguments: vec![],
            enabled: false,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(mid)),
            auxiliary: vec![],
            composition: Some(photocraft_doc::import_compositing::GimpComposition {
                compatibility_epoch: "structural-custody-only".into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: 0,
                stored_composite_space: 0,
                stored_operator: 0,
                resolved: None,
                pass_through: false,
            }),
        });
        state.filter_masks.push(FilterMask { id: mid, owner: id, surface, bounds });
        let bytes = Arc::new(vec![0x3f, 0x80, 0, 0]);
        state.opaque.push(photocraft_doc::xcf_provenance::OpaqueRecord {
            owner: Some(ItemRef::Mask(mid)),
            namespace: "gimp/xcf/source-property".into(),
            identity: "33@fixture:4".into(),
            source_version: 22,
            order: 7,
            flags: 0x80000001,
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
            rendering_required: false,
        });
        state.items.push(ItemState {
            item: ItemRef::Mask(mid),
            name: "effect mask".into(),
            custom_name: false,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(bounds),
            composition: None,
            mask: Some(MaskState {
                owner: ItemRef::Layer(layer),
                bounds,
                offset: [0, 0],
                outside_f64_bits: 0,
                enabled: true,
                shown: false,
                editing: false,
                linked: false,
            }),
        });
    }
    photocraft_doc::accounted_document::AccountedDocument::new_readonly_point_storage(
        d,
        Arc::new(PointRetention { _original: original, _hold: hold }),
        q.clone(),
        &ctl(),
    )
    .unwrap()
}
#[test]
#[ignore = "UNRUN/uncompiled: exact coverage native11 roundtrip, forced actual Point handoff, generic raw/save denial"]
fn selected_native_roundtrip_preserves_raw_white_and_native_owner() {
    for sample in [SampleType::F32, SampleType::F64] {
        let q = quota();
        let owner = point(&q, sample, true);
        let memory = save_accounted_point_f32_coverage_document_archive(&owner, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
        let header = manifest(&memory);
        assert_eq!(header.format_version, 11);
        assert_eq!(header.minimum_reader_version, 11);
        assert_eq!(header.current_storage_hint, Some(crate::NativeCurrentStorageHint::PointF32Coverage));
        assert!(crate::load_from_bytes(&memory.bytes).is_err());
        assert!(load(memory.clone(), &q).unwrap().into_accounted_document(&ctl()).is_err());
        assert!(load(memory.clone(), &q).unwrap().into_accounted_readonly_filtered_flat_storage(&ctl()).is_err());
        let current = load(memory.clone(), &q).unwrap().into_accounted_readonly_point_storage(&ctl()).unwrap();
        assert!(current.is_readonly_point_storage());
        assert_eq!(current.document().xcf_state, owner.document().xcf_state);
        assert_eq!(current.document().icc_profile, owner.document().icc_profile);
        let again = save_accounted_point_f32_coverage_document_archive(&current, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
        assert_eq!(manifest(&again).current_storage_hint, header.current_storage_hint);
        if sample == SampleType::F64 {
            assert!(save_current_document_archive(owner.document(), Writer::new(&q), Default::default(), &q, &ctl()).is_err());
            assert!(save_accounted_current_document_archive(&owner, Writer::new(&q), Default::default(), &q, &ctl()).is_err());
            let m = &current.document().xcf_state.as_ref().unwrap().filter_masks[0];
            let mut raw = [0.0];
            m.surface.try_read_pixel_f64(0, 0, &mut raw).unwrap();
            assert_eq!(raw[0].to_bits(), 0x3ff0000009b25010);
        }
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: original header floors before migration, unknown hint, no invention, empty strict continuation"]
fn floor11_is_not_recovered_by_migration_or_a_name_hint() {
    let q = quota();
    let owner = point(&q, SampleType::F64, true);
    let raw = save_accounted_point_f32_coverage_document_archive(&owner, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    for (version, minimum) in [(10, 11), (11, 10), (1, 11)] {
        let invalid = rewritten_header(&raw, &q, |v| {
            v["format_version"] = version.into();
            v["minimum_reader_version"] = minimum.into();
        });
        assert!(load(invalid, &q).is_err());
    }
    let unknown = rewritten_header(&raw, &q, |v| v["current_storage_hint"] = "point_f32_unknown".into());
    assert!(load(unknown, &q).is_err());
    let unmarked = rewritten_header(&raw, &q, |v| v["current_storage_hint"] = serde_json::Value::Null);
    assert!(load(unmarked, &q).is_err());
    let mut v = serde_json::json!({"format_version":10,"minimum_reader_version":2,"document":{}});
    crate::migrate::migrate(&mut v).unwrap();
    assert_eq!(v["format_version"], 11);
    assert!(v.get("current_storage_hint").is_none());
    let before = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0];
    let empty = owner
        .try_remove_readonly_point_filter(
            photocraft_doc::accounted_document::PointFilterRemovePatch {
                layer: owner.document().layers[0].id,
                filter: before.nodes[0].id,
                expected_stack_revision: before.revision,
            },
            &ctl(),
        )
        .unwrap();
    assert!(empty.document().xcf_state.as_ref().unwrap().filter_masks.is_empty());
    let archived = &empty.document().xcf_state.as_ref().unwrap().opaque[0];
    assert_eq!(archived.owner, None);
    assert!(archived.identity.starts_with("deleted-filter-mask/32/"));
    assert_eq!(archived.bytes, owner.document().xcf_state.as_ref().unwrap().opaque[0].bytes);
    assert_eq!(archived.blake3, owner.document().xcf_state.as_ref().unwrap().opaque[0].blake3);
    let bytes = save_accounted_point_f32_coverage_document_archive(&empty, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    assert_eq!(manifest(&bytes).current_storage_hint, None);
    let current = load(bytes, &q).unwrap().into_accounted_readonly_point_storage(&ctl()).unwrap();
    assert!(current.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
    assert_eq!(current.document().xcf_state.as_ref().unwrap().opaque, empty.document().xcf_state.as_ref().unwrap().opaque);
    assert_eq!(current.document().source_color_context, empty.document().source_color_context);
    assert_eq!(current.document().icc_profile, empty.document().icc_profile);
    let (LayerContent::BackedRaster(a), LayerContent::BackedRaster(b)) = (&current.document().layers[0].content, &owner.document().layers[0].content) else {
        panic!()
    };
    assert_eq!(a.descriptor(), b.descriptor());
    // Removed mask pixels/undo history are intentionally not serialized. The
    // still-held pre-remove owner keeps its exact operative mask words.
    assert!(owner.document().checked_f32_filter_mask_coverage_with(&ctl()).unwrap().unwrap().raw_nonempty());
}
#[test]
#[ignore = "UNRUN/uncompiled: same-ledger writer, actual controlled scan, prepublication work/cancellation failures"]
fn selected_writer_and_loader_preserve_external_controls() {
    let q = quota();
    let owner = point(&q, SampleType::F64, true);
    let other = quota();
    assert!(save_accounted_point_f32_coverage_document_archive(&owner, Writer::new(&other), Default::default(), &other, &ctl()).is_err());
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(save_accounted_point_f32_coverage_document_archive(&owner, Writer::new(&q), Default::default(), &q, &deny).is_err());
    let memory = save_accounted_point_f32_coverage_document_archive(&owner, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    assert!(load_document_from_owned_archive(memory.clone(), &Default::default(), Default::default(), q.clone(), &deny).is_err());
    let cancel = || true;
    let interrupt = Interrupt::cancel_only(&cancel);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(load_document_from_owned_archive(memory, &Default::default(), Default::default(), q.clone(), &c), Err(FormatError::Cancelled)));
    assert!(owner.document().checked_f32_filter_mask_coverage_with(&ctl()).unwrap().unwrap().raw_nonempty());
}

#[path = "masked_filtered_flat_tests.rs"]
mod masked_filtered_flat_tests;

#[path="normal_hierarchy_tests.rs"]
mod normal_hierarchy_tests;
