//! UNRUN paid synthetic storage regressions, not source/profile/blend oracles.
//! Expected Document clones and native output buffers are external harness data.
use super::*;
use crate::{Layer, PixelFormat, Size, SourceColorContext, import_compositing::*, xcf_state::*};
use photocraft_raster::{
    Interrupt,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
pub(super) fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 16 << 20, scratch: 0 })
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    reads: Arc<AtomicU64>,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(output.len() as u64)?;
        if offset.checked_add(output.len() as u64).ok_or(BackingError::Length)? > self.descriptor.logical_bytes() {
            return Err(BackingError::Bounds);
        }
        self.reads.fetch_add(1, Ordering::Relaxed);
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = ((offset + index as u64) % 251) as u8;
        }
        Ok(())
    }
}
pub(super) fn fixture(q: &BackingQuota, mode: ColorMode, sample: SampleType) -> (Document, Arc<dyn Any + Send + Sync>, Arc<AtomicU64>) {
    // Real prospective same-ledger source model/blob preholds precede all fixture
    // allocations, including the later bounded negative/opaque additions.
    let index = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let payload = q.reserve(QuotaKind::Resident, 2 << 20).unwrap();
    let reads = Arc::new(AtomicU64::new(0));
    let mut doc = Document::new("original", Size::new(513, 289), mode, sample);
    doc.icc_profile = Some(Arc::new(vec![7; 132]));
    doc.source_color_context = Some(SourceColorContext { source_precision: 650, original_icc: Some(Arc::new(vec![9; 132])) });
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.original_locator = Some("inert/source.xcf".into());
    for (index, bounds) in [Rect::new(-3, -2, 260, 259), Rect::new(270, 7, 513, 289)].into_iter().enumerate() {
        let format = PixelFormat::new(mode, sample, true);
        let descriptor = NativePlaneDescriptor::new(format, bounds, &[0x80; 40][..format.bytes_per_pixel()]).unwrap();
        let raster = BackedRaster::new(Arc::new(Store { descriptor, reads: reads.clone() }), q.clone(), 2, &ctl()).unwrap();
        let layer = Layer::new(format!("leaf{index}"), LayerContent::BackedRaster(crate::BackedLayer::new(raster, descriptor, &ctl()).unwrap()));
        state.items.insert(
            0,
            ItemState {
                item: ItemRef::Layer(layer.id),
                name: layer.name.clone(),
                custom_name: false,
                visible: layer.visible,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: Some(index as u32 + 1),
                source_flags: 0,
                raster_bounds: Some(bounds),
                mask: None,
                // A structural unknown epoch and physical Normal surrogate are
                // deliberately not a semantic eligibility claim.
                composition: Some(GimpComposition {
                    compatibility_epoch: "synthetic-current".into(),
                    source_mode: 47,
                    legacy: false,
                    stored_blend_space: -4,
                    stored_composite_space: -1,
                    stored_operator: -2,
                    resolved: Some(ResolvedGimpComposition {
                        blend_space: GimpSpace::LinearRgb,
                        composite_space: GimpSpace::LinearRgb,
                        operator: GimpOperator::ClipToBackdrop,
                    }),
                    pass_through: false,
                }),
            },
        );
        state.selection.selected.push(ItemRef::Layer(layer.id));
        doc.layers.push(layer);
    }
    state.selection.active = Some(ItemRef::Layer(doc.layers[0].id));
    doc.xcf_state = Some(state);
    (doc, Arc::new((index, payload)), reads)
}
pub(super) fn owned(q: &BackingQuota, mode: ColorMode, sample: SampleType) -> (AccountedDocument, Arc<AtomicU64>) {
    let (doc, guard, reads) = fixture(q, mode, sample);
    (AccountedDocument::from_owned_readonly_flat_composite(doc, guard, q.clone(), &ctl()).unwrap(), reads)
}
pub(super) fn leaf(owner: &AccountedDocument, index: usize) -> &crate::BackedLayer {
    let LayerContent::BackedRaster(backed) = &owner.document().layers[index].content else { panic!("fixture") };
    backed
}

#[test]
fn header_only_adoption_and_repeated_titles_preserve_native_policy_profiles_and_flat_custody() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in SampleType::ALL {
            let q = quota();
            let baseline = q.usage().unwrap();
            let (doc, guard, reads) = fixture(&q, mode, sample);
            let before = q.usage().unwrap();
            assert_eq!(
                AccountedDocument::readonly_flat_composite_storage_requirements(&doc, &q, &ctl()).unwrap(),
                AccountedDocument::owned_model_reservation_bytes()
            );
            assert_eq!(q.usage().unwrap(), before);
            let original = AccountedDocument::from_owned_readonly_flat_composite(doc, guard, q.clone(), &ctl()).unwrap();
            assert_eq!(q.usage().unwrap().index - before.index, AccountedDocument::owned_model_reservation_bytes());
            let mut current = original.clone();
            for title in ["one.pcraft", "two.pcraft", "three.pcraft"] {
                let next = current.try_with_document_name(title, &ctl()).unwrap();
                assert!(next.is_readonly_flat_composite_storage());
                next.validate_readonly_flat_composite_storage(&ctl()).unwrap();
                let mut expected = original.document().clone();
                expected.name = title.into();
                assert_eq!(next.document(), &expected);
                assert!(Arc::ptr_eq(&next.owner.retention, &original.owner.retention));
                assert!(Arc::ptr_eq(next.document().icc_profile.as_ref().unwrap(), original.document().icc_profile.as_ref().unwrap()));
                assert!(Arc::ptr_eq(
                    next.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
                    original.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
                ));
                current = next;
            }
            assert_eq!(reads.load(Ordering::Relaxed), 0); // custody never interprets native samples
            let mut a = [0; 40];
            let mut b = [0; 40];
            let bytes = leaf(&original, 0).format().bytes_per_pixel();
            leaf(&original, 0).checked_view().read_pixel_native_into(-3, -2, &mut a[..bytes], &ctl()).unwrap();
            leaf(&current, 0).checked_view().read_pixel_native_into(-3, -2, &mut b[..bytes], &ctl()).unwrap();
            assert_eq!(&a[..bytes], &b[..bytes]);
            drop(original);
            current.validate_readonly_flat_composite_storage(&ctl()).unwrap();
            drop(current);
            assert_eq!(q.usage().unwrap(), baseline);
        }
    }
}
#[test]
fn runtime_class_closes_generic_edits_history_noops_selection_move_and_normal_surrogates() {
    let q = quota();
    let (owner, _) = owned(&q, ColorMode::Rgb, SampleType::F32);
    let id = owner.document().layers[0].id;
    let mut history = AccountedHistory::new(owner.clone(), 4, &ctl()).unwrap();
    let before = q.usage().unwrap();
    for edit in [
        AccountedEdit::LayerName { layer: id, name: "leaf0" },
        AccountedEdit::LayerVisibility { layer: id, visible: true },
        AccountedEdit::LayerOpacity { layer: id, opacity: 1.0 },
        AccountedEdit::MaskLinked { layer: id, linked: true },
        AccountedEdit::NativePixels { layer: id, rect: Rect::new(-3, -2, -2, -1), bytes: &[0; 16] },
    ] {
        assert!(owner.try_edit(edit, &ctl()).is_err());
        assert!(history.try_commit(edit, &ctl()).is_err());
    }
    assert!(owner.try_replace_selection(None, &ctl()).is_err());
    assert!(history.try_replace_selection(None, &ctl()).is_err());
    assert!(owner.try_with_imported_layer_translation(id, 0, 0, &ctl()).is_err());
    assert!(owner.try_with_readonly_layer_name(ReadonlyLayerNamePatch { layer: id, name: "leaf0" }, &ctl()).is_err());
    assert!(
        owner
            .try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { layer: id, name: None, visible: None, opacity_f32_bits: None }, &ctl())
            .is_err()
    );
    assert_eq!(history.len(), 1);
    assert!(!history.can_undo());
    assert_eq!(q.usage().unwrap(), before);
}
#[test]
fn malformed_relationships_text_hashes_precision_and_foreign_ledger_refuse_without_native_reads() {
    for fault in 0..13 {
        let q = quota();
        let (mut doc, guard, reads) = fixture(&q, ColorMode::Rgb, SampleType::F32);
        let state = doc.xcf_state.as_mut().unwrap();
        match fault {
            0 => state.items[0].item = state.items[1].item,
            1 => state.items[0].visible = false,
            2 => state.items[0].name = "diverged".into(),
            3 => state.selection.selected.push(state.selection.selected[0]),
            4 => state.selection.active = Some(ItemRef::Layer(LayerId(999999))),
            5 => state.current_filter_output_precision = FilterOutputPrecision::Half,
            6 => state.provenance.source_label = "bad\0label".into(),
            7 => state.provenance.source_hash = Some("f".repeat(63)),
            8 => state.items[0].composition.as_mut().unwrap().compatibility_epoch = "bad\0epoch".into(),
            9 => {
                let p = state.items[0].composition.as_mut().unwrap();
                p.stored_operator = 1;
            }
            10 => doc.size = Size::new(262145, 1),
            11 => doc.size = Size::new(65536, 65536),
            _ => doc.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "g".repeat(64) },
        }
        assert!(AccountedDocument::readonly_flat_composite_storage_requirements(&doc, &q, &ctl()).is_err());
        assert!(AccountedDocument::from_owned_readonly_flat_composite(doc, guard, q.clone(), &ctl()).is_err());
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        assert_eq!(q.usage().unwrap(), Default::default());
    }
    let q = quota();
    let other = quota();
    let (doc, guard, _) = fixture(&q, ColorMode::Rgb, SampleType::F32);
    assert!(AccountedDocument::from_owned_readonly_flat_composite(doc, guard, other.clone(), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), Default::default());
    assert_eq!(other.usage().unwrap(), Default::default());
}
#[test]
fn opaque_payload_is_exactly_checked_and_failed_work_is_transactional() {
    let q = quota();
    let (mut doc, guard, _) = fixture(&q, ColorMode::Rgb, SampleType::F32);
    let bytes = Arc::new(vec![0x71; 65539]);
    doc.xcf_state.as_mut().unwrap().opaque.push(crate::xcf_provenance::OpaqueRecord {
        owner: Some(ItemRef::Layer(doc.layers[0].id)),
        namespace: "source-property".into(),
        identity: "kept".into(),
        source_version: 12,
        order: 0,
        flags: 0,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
        rendering_required: false,
    });
    let owner = AccountedDocument::from_owned_readonly_flat_composite(doc, guard, q.clone(), &ctl()).unwrap();
    let titled = owner.try_with_document_name("kept.pcraft", &ctl()).unwrap();
    assert!(Arc::ptr_eq(&owner.document().xcf_state.as_ref().unwrap().opaque[0].bytes, &titled.document().xcf_state.as_ref().unwrap().opaque[0].bytes));
    let before = q.usage().unwrap();
    let remaining = AtomicU64::new(32768);
    let work = |n| remaining.try_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    let limited = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(owner.try_with_document_name("denied.pcraft", &limited).is_err());
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().name, "original");
    // External malformed clone remains covered by the fixture's retained 2MiB
    // model/payload preholds; this is storage validation, not source evidence.
    let mut bad = owner.document().clone();
    bad.xcf_state.as_mut().unwrap().opaque[0].blake3 = "0".repeat(64);
    assert!(AccountedDocument::readonly_flat_composite_storage_requirements(&bad, &q, &ctl()).is_err());
}
#[test]
fn full_quota_noop_and_post_prehold_cancellation_refund_the_entire_private_copy() {
    let q = quota();
    let (owner, _) = owned(&q, ColorMode::Rgb, SampleType::F64);
    let used = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - used.resident).unwrap();
    let before = q.usage().unwrap();
    assert!(owner.try_with_document_name("original", &ctl()).unwrap().shares_model(&owner));
    assert!(owner.try_with_document_name("changed", &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    drop(blocker);
    let baseline = q.usage().unwrap();
    let work = |_| if q.usage().map_err(|_| BackingError::Quota)?.index > baseline.index { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(owner.try_with_document_name("cancelled", &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(owner.document().name, "original");
}
