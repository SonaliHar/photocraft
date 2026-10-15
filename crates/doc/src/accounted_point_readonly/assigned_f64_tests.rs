//! UNRUN structural assigned F64 custody, not an operation/profile/source proof.
//! Generic fixture holds8MiB Resident+8MiB Index before its F64 default/map and
//! 512KiB physical Gray mask tile allocations. Native COW tiles receive separate real
//! same-q backing fees. Expected/output buffers below are external harness data.
use super::*;
fn pair(q: &BackingQuota, sample: SampleType, encoding: SampleEncoding, precision: u32) -> (Document, Arc<SourceGuard>) {
    let (mut doc, guard) = fixture(q, sample, encoding, precision);
    let mut original = Vec::with_capacity(16384);
    original.extend_from_slice(&[1, 2, 3, 4]);
    let mut current = Vec::with_capacity(32768);
    current.extend_from_slice(&[5, 6, 7, 8]);
    doc.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(original));
    doc.icc_profile = Some(Arc::new(current));
    doc.working_profile_interpretation = crate::WorkingProfileInterpretation::Icc;
    (doc, guard)
}
fn owned64(revision: u64) -> AccountedDocument {
    let q = quota();
    let (mut doc, guard) = pair(&q, SampleType::F64, SampleEncoding::NonlinearProfile, 750);
    let stack = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0];
    stack.revision = revision;
    stack.nodes[0].arguments[0].value = FilterValue::Number(ExactNumber::Signed { width: 32, value: 4 });
    let mut owner = AccountedDocument::new_readonly_assigned_point_storage(doc, guard, q, &ctl()).unwrap();
    seed_native(&mut owner);
    owner
}
fn seed_native(owner: &mut AccountedDocument) {
    let bytes: Vec<u8> = [-0.0f64, 1.125, -0.125, 0.5].into_iter().flat_map(f64::to_ne_bytes).collect();
    let before = owner.quota().usage().unwrap();
    let next = leaf(owner).try_edited_native(Rect::new(0, 14, 1, 15), &bytes, &ctl()).unwrap();
    // Actual backing fees cover a 2MiB RGBA F64 tile plus private overlap;
    // the old incoming fixture prehold is not substituted for this COW charge.
    assert!(owner.quota().usage().unwrap().resident >= before.resident + (256 * 256 * 32) as u64);
    Arc::get_mut(&mut owner.owner).unwrap().document.layers[0].content = LayerContent::BackedRaster(next);
    let mut actual = [0u8; 32];
    leaf(owner).checked_view().read_pixel_native_into(0, 14, &mut actual, &ctl()).unwrap();
    assert_eq!(actual.as_slice(), bytes);
}
fn leaf(owner: &AccountedDocument) -> &BackedLayer {
    let LayerContent::BackedRaster(v) = &owner.document().layers[0].content else { panic!("fixture") };
    v
}
fn profiles_and_masks(a: &AccountedDocument, b: &AccountedDocument) {
    assert!(b.is_readonly_assigned_point_storage());
    assert!(!b.is_readonly_point_storage());
    assert!(a.quota().shares_ledger(b.quota()));
    assert!(Arc::ptr_eq(a.document().icc_profile.as_ref().unwrap(), b.document().icc_profile.as_ref().unwrap()));
    assert!(Arc::ptr_eq(
        a.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
        b.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
    ));
    assert_eq!(a.document().source_color_context, b.document().source_color_context);
    let sa = a.document().xcf_state.as_ref().unwrap();
    let sb = b.document().xcf_state.as_ref().unwrap();
    assert_eq!(sa.provenance, sb.provenance);
    assert_eq!(sa.opaque, sb.opaque);
    assert_eq!(sa.filter_masks, sb.filter_masks);
    for (ma, mb) in sa.filter_masks.iter().zip(&sb.filter_masks) {
        assert_eq!(ma.surface.default_bytes(), mb.surface.default_bytes());
        for ((ca, ta), (cb, tb)) in ma.surface.tiles().zip(mb.surface.tiles()) {
            assert_eq!(ca, cb);
            assert!(Arc::ptr_eq(ta, tb));
        }
    }
}
#[test]
#[ignore = "UNRUN exact assigned depth/precision pairs, encoding and independent role closure"]
fn exact_two_pairs_accept_and_cross_pairs_other_depths_and_encodings_refuse() {
    for (sample, encoding, precision, valid) in [
        (SampleType::F32, SampleEncoding::NonlinearProfile, 650, true),
        (SampleType::F64, SampleEncoding::NonlinearProfile, 750, true),
        (SampleType::F32, SampleEncoding::NonlinearProfile, 750, false),
        (SampleType::F64, SampleEncoding::NonlinearProfile, 650, false),
        (SampleType::F32, SampleEncoding::NonlinearProfile, 550, false),
        (SampleType::U16, SampleEncoding::NonlinearProfile, 250, false),
        (SampleType::F64, SampleEncoding::Linear, 700, false),
        (SampleType::F64, SampleEncoding::Perceptual, 775, false),
    ] {
        let q = quota();
        let (doc, guard) = pair(&q, sample, encoding, precision);
        assert_eq!(AccountedDocument::readonly_assigned_point_storage_requirements(&doc, &q, &ctl()).is_ok(), valid);
        assert_eq!(AccountedDocument::new_readonly_assigned_point_storage(doc, guard, q.clone(), &ctl()).is_ok(), valid);
        assert_eq!(q.usage().unwrap(), Default::default());
    }
    for defect in 0..5 {
        let q = quota();
        let (mut doc, guard) = pair(&q, SampleType::F64, SampleEncoding::NonlinearProfile, 750);
        match defect {
            0 => doc.source_color_context.as_mut().unwrap().original_icc = None,
            1 => doc.icc_profile = None,
            2 => doc.icc_profile = Some(Arc::new(Vec::new())),
            3 => doc.working_profile_interpretation = crate::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
            _ => doc.mode = ColorMode::Grayscale,
        }
        assert!(AccountedDocument::new_readonly_assigned_point_storage(doc, guard, q.clone(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), Default::default());
    }
}
#[test]
#[ignore = "UNRUN F64 real profile capacities, physical mask bytes, header facts and old implicit rejection"]
fn f64_census_prices_actual_owners_but_constructor_adds_only_header() {
    let q = quota();
    let (doc, guard) = pair(&q, SampleType::F64, SampleEncoding::NonlinearProfile, 750);
    let original = doc.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap();
    let current = doc.icc_profile.as_ref().unwrap();
    assert!(!Arc::ptr_eq(original, current));
    assert!(original.capacity() >= 16384 && current.capacity() >= 32768);
    let measured = measured_assigned(&doc, &q, &ctl()).unwrap();
    assert!(measured.total().unwrap() >= (original.capacity() + current.capacity()) as u64);
    assert_eq!(doc.xcf_state.as_ref().unwrap().filter_masks[0].surface.tiles().next().unwrap().1.bytes().len(), 256 * 256 * 8);
    // Existing incoming guard pays immutable mask tile payload; model census
    // separately prices the shared map/default/header and both ICC capacities.
    assert_eq!(doc.pixel_format().bytes_per_pixel(), 32);
    assert_eq!(doc.xcf_state.as_ref().unwrap().filter_masks[0].surface.format().bytes_per_pixel(), 8);
    assert!(AccountedDocument::readonly_point_storage_requirements(&doc, &q, &ctl()).is_err());
    let before = q.usage().unwrap();
    let header = AccountedDocument::readonly_assigned_point_storage_requirements(&doc, &q, &ctl()).unwrap();
    assert_eq!(header, AccountedDocument::owned_model_reservation_bytes());
    assert_eq!(q.usage().unwrap(), before);
    let owner = AccountedDocument::new_readonly_assigned_point_storage(doc, guard, q.clone(), &ctl()).unwrap();
    assert_eq!(q.usage().unwrap().index - before.index, header);
    assert!(owner.validate_readonly_point_storage(&ctl()).is_err());
    owner.validate_readonly_assigned_point_storage(&ctl()).unwrap();
    drop(owner);
    assert_eq!(q.usage().unwrap(), Default::default());
}
#[test]
#[ignore = "UNRUN F64 raw native/default/padding and repeated title codec lifetime"]
fn f64_titles_preserve_raw_payloads_and_flat_original_retention() {
    let q = quota();
    let (doc, guard) = pair(&q, SampleType::F64, SampleEncoding::NonlinearProfile, 750);
    let codec = Arc::downgrade(&guard);
    let mut old = AccountedDocument::new_readonly_assigned_point_storage(doc, guard, q.clone(), &ctl()).unwrap();
    seed_native(&mut old);
    let root = Arc::downgrade(&old.owner);
    assert_eq!(&leaf(&old).default_bytes()[..8], &(-0f64).to_ne_bytes());
    let first = old.try_with_document_name("first F64", &ctl()).unwrap();
    let middle = Arc::downgrade(&first.owner);
    let second = first.try_with_document_name("second F64", &ctl()).unwrap();
    profiles_and_masks(&old, &second);
    assert_eq!(old.document().layers, second.document().layers);
    assert_eq!(old.document().xcf_state, second.document().xcf_state);
    let retained = second.owner.retention.downcast_ref::<OriginalRetention>().unwrap();
    assert!(retained.original.shares_model(&old));
    drop(first);
    assert!(middle.upgrade().is_none());
    drop(old);
    assert!(root.upgrade().is_some());
    assert!(codec.upgrade().is_some());
    drop(second);
    assert!(root.upgrade().is_none());
    assert!(codec.upgrade().is_none());
    assert_eq!(q.usage().unwrap(), Default::default());
}
#[test]
#[ignore = "UNRUN F64 existing structural compound/filter/translation copies; no operation semantics"]
fn f64_existing_typed_data_edits_preserve_profiles_mask_canvas_and_revision_rules() {
    let old = owned64(7);
    let id = old.document().layers[0].id;
    let metadata =
        old.try_edit(AccountedEdit::LayerMetadata { layer: id, name: Some("F64 current"), visible: Some(false), opacity: Some(-0.0) }, &ctl()).unwrap();
    profiles_and_masks(&old, &metadata);
    assert_eq!(metadata.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
    let stack = &metadata.document().xcf_state.as_ref().unwrap().filter_stacks[0];
    let args = [PointArgumentPatch { name: "raw-0", value: PointArgumentValue::Signed32(1000) }];
    let changed = metadata
        .try_with_readonly_assigned_point_filter(
            PointFilterPatch { layer: id, filter: stack.nodes[0].id, expected_stack_revision: 7, arguments: &args, enabled: Some(true) },
            &ctl(),
        )
        .unwrap();
    profiles_and_masks(&old, &changed);
    assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 8);
    let moved = changed.try_with_readonly_point_layer_translation(id, -257, 259, &ctl()).unwrap();
    profiles_and_masks(&old, &moved);
    assert_eq!(leaf(&moved).operative_bounds(), Rect::new(-270, 272, -13, 401));
    assert_eq!(moved.document().xcf_state.as_ref().unwrap().filter_stacks, changed.document().xcf_state.as_ref().unwrap().filter_stacks);
    assert_eq!(moved.document().layers[0].opacity.to_bits(), (-0f32).to_bits());
    assert!(moved.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &ctl()).is_err());
    assert!(moved.try_replace_selection(None, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN F64 actual quota/work/cancel failures, full quota no-op and incoming refund"]
fn f64_failure_boundaries_preserve_paid_source_and_refund_private_copies() {
    let old = owned64(7);
    let q = old.quota().clone();
    let id = old.document().layers[0].id;
    let before = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    let full = q.usage().unwrap();
    assert!(old.try_with_document_name(&old.document().name, &ctl()).unwrap().shares_model(&old));
    assert!(old.try_with_readonly_point_layer_translation(id, 0, 0, &ctl()).unwrap().shares_model(&old));
    assert!(old.try_with_document_name("needs model", &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), full);
    drop(blocker);
    let usage = q.usage().unwrap();
    let resident = q.reserve(QuotaKind::Resident, (128 << 20) - usage.resident).unwrap();
    assert!(old.try_with_document_name(&old.document().name, &ctl()).unwrap().shares_model(&old));
    assert!(old.try_with_document_name("changed resident", &ctl()).is_err());
    drop(resident);
    assert_eq!(q.usage().unwrap(), before);
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(old.try_with_document_name("changed", &denied).is_err());
    let allocated = || q.usage().unwrap().index > before.index;
    let stop = Interrupt::cancel_only(&allocated);
    assert!(old.try_with_readonly_point_layer_translation(id, -1, 1, &ReadControl { interrupt: &stop, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    drop(old);
    assert_eq!(q.usage().unwrap(), Default::default());
    for failure in 0..3 {
        let q = quota();
        let (doc, guard) = pair(&q, SampleType::F64, SampleEncoding::NonlinearProfile, 750);
        let before = q.usage().unwrap();
        let blockade = if failure == 0 { Some(q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap()) } else { None };
        let cancel = || failure == 2;
        let stop = Interrupt::cancel_only(&cancel);
        let control = ReadControl { interrupt: &stop, work: &|_| if failure == 1 { Err(BackingError::Work) } else { Ok(()) } };
        assert!(AccountedDocument::new_readonly_assigned_point_storage(doc, guard, q.clone(), &control).is_err());
        drop(blockade);
        assert_eq!(q.usage().unwrap(), Default::default());
    }
}
