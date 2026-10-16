//! Ignored UNRUN data-only custody contracts. Synthetic same-q native/mask
//! owners do not qualify an operation, profile, source recipe or numeric result.
use super::*;
use crate::xcf_filter_masks::{FilterMaskCoverageClass as Class, MaskCoverageSetLimits as Limits};
struct PaidStore {
    descriptor: NativePlaneDescriptor,
    quota: BackingQuota,
    _guard: Arc<Guard>,
}
impl std::fmt::Debug for PaidStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaidMaskFixture").finish_non_exhaustive()
    }
}
impl NativePlaneStore for PaidStore {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        if offset.checked_add(out.len() as u64).is_none_or(|n| n > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        out.fill(0);
        Ok(())
    }
}
fn raw(sample: SampleType, counts: &[usize]) -> (Document, Arc<Guard>, BackingQuota) {
    let q = quota();
    let (mut d, g) = ordered_fixture(&q, sample, counts);
    d.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(d.icc_profile.as_ref().unwrap());
    // New stores explicitly validate the SAME retained ledger; fixture preholds
    // precede their headers and every replacement mask/map allocation.
    for layer in &mut d.layers {
        let LayerContent::BackedRaster(old) = &layer.content else { panic!() };
        let desc = old.descriptor();
        let r = BackedRaster::new(Arc::new(PaidStore { descriptor: desc, quota: q.clone(), _guard: g.clone() }), q.clone(), 1, &ctl()).unwrap();
        layer.content = LayerContent::BackedRaster(BackedLayer::new(r, desc, &ctl()).unwrap());
    }
    let state = d.xcf_state.as_mut().unwrap();
    for (i, m) in state.filter_masks.iter_mut().enumerate() {
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
        if i != 1 {
            surface.try_write_pixel_f64(0, 0, &[if sample == SampleType::F64 && i == 0 { f64::from_bits(0x3ff0000009b25010) } else { 0.375 }]).unwrap();
        }
        m.surface = surface;
    }
    let m = state.filter_masks[0].id;
    let bytes = Arc::new(vec![0x12, 0x34, 0x91]);
    state.opaque.push(crate::xcf_provenance::OpaqueRecord {
        owner: Some(ItemRef::Mask(m)),
        namespace: "gimp/xcf/source-property".into(),
        identity: "mask-property/raw".into(),
        source_version: 22,
        order: 7,
        flags: 9,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
        rendering_required: false,
    });
    (d, g, q)
}
fn owner(sample: SampleType) -> AccountedDocument {
    let (d, g, q) = raw(sample, &[2, 1]);
    AccountedDocument::new_readonly_masked_filtered_flat_point_storage(d, g, q, &ctl()).unwrap()
}
#[test]
#[ignore = "UNRUN/uncompiled: complete actual-Doc proof, exact classifications and one prepaid whole-Doc validation; no operation/source grant"]
fn complete_proof_is_bound_and_keeps_raw_domains() {
    for sample in [SampleType::F32, SampleType::F64] {
        let (d, _g, q) = raw(sample, &[2, 1]);
        let state = d.xcf_state.as_ref().unwrap();
        let before = q.usage().unwrap();
        let set = d.checked_f32_filter_mask_coverages_with(Limits::STANDARD, &q, &ctl()).unwrap();
        assert_eq!(set.len(), 3);
        assert_eq!(set.classification_for_mask(0, &state.filter_masks[0]).unwrap(), Class::RawNonemptyCoverage);
        assert_eq!(set.classification_for_mask(1, &state.filter_masks[1]).unwrap(), Class::ByteZeroSentinel);
        assert!(set.classification_for_mask(0, &state.filter_masks[2]).is_err());
        let foreign = d.clone(); // external expected/malformed harness, within original prehold
        assert!(foreign.validate_xcf_state_with_checked_filter_masks(&set, &Interrupt::NONE).is_err());
        d.validate_xcf_state_with_checked_filter_masks(&set, &Interrupt::NONE).unwrap();
        assert!(d.validate_xcf_state_with_checked_filter_masks(&set, &Interrupt::NONE).is_err());
        if sample == SampleType::F64 {
            assert!(d.validate_xcf_state().is_err());
            let mut v = [0.];
            state.filter_masks[0].surface.try_read_pixel_f64(0, 0, &mut v).unwrap();
            assert_eq!(v[0].to_bits(), 0x3ff0000009b25010);
        }
        drop(set);
        assert_eq!(q.usage().unwrap(), before);
    }
    let (mut d, _g, q) = raw(SampleType::F64, &[2]);
    d.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(0, 0, &[f64::from_bits(1)]).unwrap();
    let set = d.checked_f32_filter_mask_coverages_with(Limits::STANDARD, &q, &ctl()).unwrap();
    assert_eq!(set.classification_for_mask(0, &d.xcf_state.as_ref().unwrap().filter_masks[0]).unwrap(), Class::RawNonemptyCoverage);
    drop(set);
    d.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(0, 0, &[-0.0]).unwrap();
    assert!(d.checked_f32_filter_mask_coverages_with(Limits::STANDARD, &q, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN/uncompiled: Source ALL+1 vs Current nonempty+1 even disabled, zero siblings keep finiteunit opacity; third/default/padding/foreign owners refuse"]
fn structural_purpose_opacity_and_invalid_prestate() {
    let (mut d, g, q) = raw(SampleType::F64, &[2, 1]);
    d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[1].opacity_f64_bits = 0.375f64.to_bits();
    assert!(AccountedDocument::readonly_masked_filtered_flat_point_storage_requirements(&d, &q, &ctl()).is_err());
    let current = AccountedDocument::new_readonly_masked_filtered_flat_point_current_storage(d, g, q, &ctl()).unwrap();
    current.validate_readonly_masked_filtered_flat_point_storage(&ctl()).unwrap();
    let p = target(&current, 0, 0, &[], None);
    assert!(current.try_with_readonly_masked_filtered_flat_point_filter_with_opacity(p, Some(0.375f64.to_bits()), &ctl()).is_err());
    let p = target(&current, 0, 1, &[], None);
    let changed = current.try_with_readonly_masked_filtered_flat_point_filter_with_opacity(p, Some((-0f64).to_bits()), &ctl()).unwrap();
    assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 18);
    assert!(changed.try_with_readonly_filtered_flat_point_filter(target(&changed, 0, 1, &[], None), &ctl()).is_err());
    for variant in 0..5 {
        let (mut d, _g, q) = raw(SampleType::F64, &[2, 1]);
        let s = d.xcf_state.as_mut().unwrap();
        match variant {
            0 => {
                s.filter_stacks[0].nodes[0].enabled = false;
                s.filter_stacks[0].nodes[0].opacity_f64_bits = 0f64.to_bits();
            }
            1 => s.filter_masks[0].owner = FilterId(99999),
            2 => {
                s.filter_masks[0].surface =
                    Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &(-0f64).to_ne_bytes()).unwrap()
            }
            3 => *s.filter_masks[0].surface.try_tile_mut(TileCoord::new(1, 1)).unwrap().bytes_mut().last_mut().unwrap() = 1,
            _ => s.filter_stacks[1].nodes[0].id = s.filter_stacks[0].nodes[0].id,
        }
        assert!(AccountedDocument::readonly_masked_filtered_flat_point_current_storage_requirements(&d, &q, &ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: actual nonfirst target/one revision, whole paid copy/native/profile/source custody, move/remove to empty retains distinct purpose"]
fn target_copy_and_empty_continuation_keep_class_and_custody() {
    let original = owner(SampleType::F64);
    let args = [PointArgumentPatch { name: "scalar", value: PointArgumentValue::Float32Bits(0.75f32.to_bits()) }];
    let changed = original.try_with_readonly_masked_filtered_flat_point_filter(target(&original, 1, 0, &args, Some(true)), &ctl()).unwrap();
    assert!(changed.is_readonly_masked_filtered_flat_point_storage());
    assert!(!changed.is_readonly_filtered_flat_point_storage());
    assert!(!changed.is_readonly_point_storage());
    assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0], original.document().xcf_state.as_ref().unwrap().filter_stacks[0]);
    assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[1].revision, 18);
    assert!(Arc::ptr_eq(&changed.owner.retention, &original.owner.retention));
    let state = changed.document().xcf_state.as_ref().unwrap();
    let layer = changed.document().layers[0].id;
    let moved = changed
        .try_move_readonly_masked_filtered_flat_point_filter(
            PointFilterMovePatch { layer, filter: state.filter_stacks[0].nodes[0].id, expected_stack_revision: state.filter_stacks[0].revision, index: 1 },
            &ctl(),
        )
        .unwrap();
    let mut current = moved.try_with_document_name("masked-current.pcraft", &ctl()).unwrap();
    for _ in 0..3 {
        let s = current.document().xcf_state.as_ref().unwrap();
        let stack = s.filter_stacks.iter().find(|s| !s.nodes.is_empty()).unwrap();
        let ItemRef::Layer(layer) = stack.owner else { panic!() };
        current = current
            .try_remove_readonly_masked_filtered_flat_point_filter(
                PointFilterRemovePatch { layer, filter: stack.nodes[0].id, expected_stack_revision: stack.revision },
                &ctl(),
            )
            .unwrap();
    }
    let s = current.document().xcf_state.as_ref().unwrap();
    assert!(s.filter_masks.is_empty());
    assert!(s.filter_stacks.iter().all(|s| s.nodes.is_empty()));
    current.validate_readonly_masked_filtered_flat_point_storage(&ctl()).unwrap();
    assert!(current.is_readonly_masked_filtered_flat_point_storage());
    assert!(AccountedDocument::readonly_masked_filtered_flat_point_storage_requirements(current.document(), current.quota(), &ctl()).is_err());
    assert!(Arc::ptr_eq(current.document().icc_profile.as_ref().unwrap(), original.document().icc_profile.as_ref().unwrap()));
    assert_eq!(current.document().source_color_context, original.document().source_color_context);
    assert!(Arc::ptr_eq(&current.owner.retention, &original.owner.retention));
    let old = &original.document().xcf_state.as_ref().unwrap().opaque[0];
    let archived = &s.opaque[0];
    assert!(archived.owner.is_none());
    assert!(Arc::ptr_eq(&old.bytes, &archived.bytes));
    assert_eq!((&old.blake3, old.order, old.flags), (&archived.blake3, archived.order, archived.flags));
    let edited =
        current.try_edit(AccountedEdit::LayerMetadata { layer, name: Some("still empty"), visible: Some(false), opacity: Some(-0.0) }, &ctl()).unwrap();
    assert!(edited.is_readonly_masked_filtered_flat_point_storage());
    assert!(edited.try_edit(AccountedEdit::LayerVisibility { layer, visible: false }, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN/uncompiled: descriptor-only16/64MiB denial before samplewalk/Vec, actual proof/copy quota/work/cancel refund; no large payload success claim"]
fn preflight_limits_and_unpublished_failures() {
    let (mut d, _g, q) = raw(SampleType::F64, &[5]);
    let s = d.xcf_state.as_mut().unwrap();
    let tile = s.filter_masks[0].surface.tiles().next().unwrap().1.clone();
    // External malformed negative maps share one genuinely prepaid tile Arc.
    // Out-of-canvas coordinates would fail later; aggregate must fail first.
    for m in &mut s.filter_masks {
        m.surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
        for x in 0..31 {
            m.surface.put_tiles(std::iter::once((TileCoord::new(x, 0), tile.clone())));
        }
    }
    let before = q.usage().unwrap();
    assert_eq!(d.checked_f32_filter_mask_coverage_set_requirements(Limits::STANDARD, &ctl()).unwrap_err(), BackingError::Length);
    assert_eq!(q.usage().unwrap(), before);
    let s = d.xcf_state.as_mut().unwrap();
    s.filter_masks.truncate(1);
    s.filter_stacks[0].nodes.truncate(1);
    s.filter_masks[0].surface.put_tiles(std::iter::once((TileCoord::new(31, 0), tile)));
    assert_eq!(d.checked_f32_filter_mask_coverage_set_requirements(Limits::STANDARD, &ctl()).unwrap_err(), BackingError::Length);
    let original = owner(SampleType::F32);
    let before = original.quota().usage().unwrap();
    let work = |_| Err(BackingError::Work);
    assert!(original.try_with_document_name("changed", &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(original.quota().usage().unwrap(), before);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(original.validate_readonly_masked_filtered_flat_point_storage(&ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(original.quota().usage().unwrap(), before);
    let full = original.quota().reserve(QuotaKind::Index, (64 << 20) - before.index).unwrap();
    assert!(original.try_with_document_name("changed", &ctl()).is_err());
    drop(full);
    assert_eq!(original.quota().usage().unwrap(), before);
    assert_eq!(original.document().name, "initial");
}

struct SourceLease(QuotaLease);
impl crate::xcf_filter_masks::MaskCoverageSetLease for SourceLease {
    fn bytes(&self) -> u64 {
        self.0.bytes()
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: non-Sync original-work reservation transfers one distinct real parent Index receipt; fullquota does not doublehold or mint simultaneous tokens"]
fn source_lease_adapter_retains_real_unique_receipt() {
    use std::cell::{Cell, RefCell};
    let (d, _g, q) = raw(SampleType::F32, &[2, 1]);
    let baseline = q.usage().unwrap();
    let charged = Cell::new(0u64);
    let work = |n| {
        charged.set(charged.get().checked_add(n).ok_or(BackingError::Work)?);
        Ok(())
    };
    let req = d.checked_f32_filter_mask_coverage_set_requirements_with_local_work_for_lease::<SourceLease>(Limits::STANDARD, &Interrupt::NONE, &work).unwrap();
    let source = RefCell::new(Some(SourceLease(q.reserve(QuotaKind::Index, req.proof_index_bytes).unwrap())));
    let beforeblock = q.usage().unwrap();
    let block = q.reserve(QuotaKind::Index, (64 << 20) - beforeblock.index).unwrap();
    let reserve = |bytes| {
        let lease = source.borrow_mut().take().ok_or(BackingError::Quota)?;
        assert_eq!(bytes, req.proof_index_bytes);
        Ok(lease)
    };
    assert!(d.checked_f32_filter_mask_coverages_with(Limits::STANDARD, &q, &ctl()).is_err());
    let set = d.checked_f32_filter_mask_coverages_with_reservation(Limits::STANDARD, &Interrupt::NONE, &work, &reserve).unwrap();
    assert!(d.checked_f32_filter_mask_coverages_with_reservation(Limits::STANDARD, &Interrupt::NONE, &work, &reserve).is_err());
    d.validate_xcf_state_with_checked_filter_masks(&set, &Interrupt::NONE).unwrap();
    assert!(charged.get() >= req.validation_work_units);
    drop(set);
    drop(block);
    assert_eq!(q.usage().unwrap(), baseline);
}
