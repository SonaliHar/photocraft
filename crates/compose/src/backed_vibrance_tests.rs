//! Ignored UNRUN declared physical-ICC/LCh compatibility contracts. These are
//! independent public CMS/transfer/scalar/lower references, not GIMP RGB rounding acceptance.
use super::tests::{adopt, ctl, fixture};
use super::*;
use crate::backed_stock_point_document::PreparedBackedStockPointDocument;
use photocraft_algo::gimp::hue_chroma::{self, Parameters};
use photocraft_doc::{
    BackedLayer,
    xcf_filters::{ExactNumber, FilterArgument, OperationIdentity},
};
use photocraft_raster::backing::{BackedRaster, NativePlaneStore};
use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

fn vibrance_fixture(sample: SampleType, values: [f32; 2]) -> (Document, BackingQuota, Arc<dyn Any + Send + Sync>, Arc<AtomicU64>, &'static Profile, Vec<u8>) {
    let (mut doc, q, guard, reads, basis, raw) = fixture(0, ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation = OperationIdentity { name: "gegl:vibrance".into(), stored_version: "0:0".into(), schema_epoch: crate::VIBRANCE_EPOCH.into() };
    node.arguments = ["vibrance", "saturation"]
        .into_iter()
        .zip(values)
        .map(|(name, v)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(v.to_bits())) })
        .collect();
    let policy = node.composition.as_mut().unwrap();
    policy.compatibility_epoch = crate::VIBRANCE_EPOCH.into();
    policy.stored_blend_space = i32::MIN;
    policy.stored_composite_space = 17;
    policy.stored_operator = i32::MAX;
    (doc, q, guard, reads, basis, raw)
}
// External harness Parts/reference buffers are not production quota receipts.
fn parts() -> photocraft_cms::owned_lab::Parts {
    photocraft_cms::owned_linear_srgb_lab::construct_with_control(Default::default(), &|| false).unwrap()
}
// Public fixed transfer equations + explicit Float arithmetic form the
// independent declared-policy reference, not a Babl or producer oracle.
fn reference(parts: &photocraft_cms::owned_lab::Parts, input: [f64; 4], values: [f32; 2]) -> [f64; 4] {
    let curve = photocraft_cms::curve::srgb_trc();
    let linear = [curve.eval64(input[0]) as f32, curve.eval64(input[1]) as f32, curve.eval64(input[2]) as f32, input[3] as f32];
    let mut saturated = linear;
    if values[1] != 1. {
        let gray = linear[0] * f32::from_bits(0x3e63d3e1) + linear[1] * f32::from_bits(0x3f378703) + linear[2] * f32::from_bits(0x3d78405d);
        for i in 0..3 {
            saturated[i] = gray + (linear[i] - gray) * values[1];
        }
    }
    let mut lab = [0.; 3];
    parts.physical_forward.try_eval_f64(&[f64::from(saturated[0]), f64::from(saturated[1]), f64::from(saturated[2])], &mut lab).unwrap();
    let chroma = lab[1].hypot(lab[2]);
    let hue = if chroma == 0. { 0. } else { lab[2].atan2(lab[1]).to_degrees().rem_euclid(360.) };
    let lch = [lab[0] as f32, chroma as f32, hue as f32, saturated[3]];
    let changed = hue_chroma::evaluate_lch_float(lch, Parameters { hue: 0., chroma: f64::from(values[0]), lightness: 0. }).unwrap();
    let angle = f64::from(changed[2]).to_radians();
    let lab = [f64::from(changed[0]), f64::from(changed[1]) * angle.cos(), f64::from(changed[1]) * angle.sin()];
    let mut rgb = [0.; 3];
    parts.physical_reverse.try_eval_f64(&lab, &mut rgb).unwrap();
    let rgb = rgb.map(|v| curve.eval_inverse64(f64::from(v as f32)));
    [f64::from(rgb[0] as f32), f64::from(rgb[1] as f32), f64::from(rgb[2] as f32), f64::from(changed[3])]
}
fn native(pixel: [f64; 4], sample: SampleType) -> Vec<u8> {
    let mut bytes = vec![0; 4 * sample.bytes()];
    for (i, v) in pixel.into_iter().enumerate() {
        photocraft_color::try_write_sample_f64(&mut bytes, sample, i, v).unwrap();
    }
    bytes
}
struct CurrentRetention {
    _owner: AccountedDocument,
    _index: QuotaLease,
    _resident: QuotaLease,
}
fn clone_retention(owner: &AccountedDocument) -> Arc<dyn Any + Send + Sync> {
    // Fixed tiny fixture model, not arbitrary input. Both charges precede the
    // clone; retaining the original also preserves shared native/profile roots.
    let index = owner.quota().reserve(QuotaKind::Index, 2 << 20).unwrap();
    let resident = owner.quota().reserve(QuotaKind::Resident, 2 << 20).unwrap();
    Arc::new(CurrentRetention { _owner: owner.clone(), _index: index, _resident: resident })
}
fn check_closed_clone(doc: &Document, quota: &BackingQuota) {
    // Conservative envelope checks for this specific generated fixture, not a
    // general model census. No physical native/mask payload is copied here.
    assert_eq!(doc.layers.len(), 1);
    assert!(doc.layers.capacity() <= 4);
    let state = doc.xcf_state.as_ref().unwrap();
    assert!(state.items.capacity() <= 8);
    assert!(state.filter_stacks.capacity() <= 4);
    assert!(state.filter_masks.capacity() <= 4);
    assert!(doc.layers[0].name.capacity() <= 4096);
    assert!(doc.layers[0].effects.items.is_empty());
    assert!(doc.layers[0].psd_blocks.is_empty());
    assert!(doc.icc_profile.as_ref().unwrap().len() <= 65536);
    for item in &state.items {
        assert!(item.name.capacity() <= 4096);
    }
    for stack in &state.filter_stacks {
        assert!(stack.nodes.capacity() <= 4);
        for node in &stack.nodes {
            assert!(node.arguments.capacity() <= 8);
            assert!(node.operation.name.capacity() <= 4096);
            assert!(node.operation.stored_version.capacity() <= 4096);
            assert!(node.operation.schema_epoch.capacity() <= 4096);
            for arg in &node.arguments {
                assert!(arg.name.capacity() <= 4096);
                assert!(matches!(arg.value, FilterValue::Number(ExactNumber::Float32Bits(_))));
            }
        }
    }
    for mask in &state.filter_masks {
        assert_eq!(mask.surface.tile_count(), 0);
    }
    // This existing L1 traversal validates the full measured class/canonical
    // shape; its returned number is only the additional owner header.
    AccountedDocument::readonly_point_storage_requirements(doc, quota, &ctl()).unwrap();
}
#[test]
#[ignore = "UNRUN declared NL/LinearFloat/Saturation/LinearLab/LChFloat/inverse/LinearFloat/NLFloat schedule; no producer oracle"]
fn ordered_linear_float_schedule_retains_generated_alpha_and_finite_hdr() {
    let reference_parts = parts();
    let mut saw_hdr = false;
    for sample in [SampleType::F32, SampleType::F64] {
        for values in [[0., 1.], [25., 0.25], [100., 10.], [-100., 0.]] {
            let (doc, q, _guard, reads, basis, _) = vibrance_fixture(sample, values);
            let pipeline = PreparedStockPointPipeline::prepare(&doc, basis, &q, &ctl()).unwrap();
            assert_eq!(reads.load(Ordering::Relaxed), 0);
            let nodes = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes;
            assert!(crate::gimp_filters::PointStack::from_nodes_at(nodes, basis, SampleEncoding::NonlinearProfile, 22, doc.bounds()).is_err());
            for alpha in [0., -0., 0.125, 1., f64::from_bits(1f64.to_bits() - 1)] {
                let raw = native([0.125, 0.5 + 2f64.powi(-45), 0.875, alpha], sample);
                let input = std::array::from_fn(|i| try_read_sample_f64(&raw, sample, i).unwrap());
                let actual = pipeline.evaluate_native(&raw, doc.pixel_format(), &ctl()).unwrap();
                assert_eq!(actual.map(f64::to_bits), reference(&reference_parts, input, values).map(f64::to_bits));
                assert_eq!(actual[3].to_bits(), f64::from(input[3] as f32).to_bits());
                assert!(actual.iter().all(|v| v.is_finite()));
                assert!(actual[..3].iter().all(|v| f64::from(*v as f32).to_bits() == v.to_bits()));
                saw_hdr |= actual[..3].iter().any(|v| !(0.0..=1.0).contains(v));
            }
        }
    }
    assert!(saw_hdr);
}
#[test]
#[ignore = "UNRUN paid provider/ROI/display, disabled native identity and honest generic derived empty role"]
fn provider_and_derived_empty_preserve_owner_and_activity_boundaries() {
    let reference_parts = parts();
    for sample in [SampleType::F32, SampleType::F64] {
        let values = [25., 0.25];
        let (mut doc, q, guard, _, basis, raw) = vibrance_fixture(sample, values);
        doc.layers[0].opacity = 0.375;
        let id = doc.layers[0].id;
        let owner = adopt(doc, q.clone(), guard);
        let context = PreparedBackedStockPointContext::new(&owner, basis, NormalBandLimits { max_rows: 2, max_pixels: 8 }, &ctl()).unwrap();
        context.validate_operative(&ctl()).unwrap();
        let part = context.render_wide_band(Rect::new(2, 1, 6, 3), &ctl()).unwrap();
        for y in 1..3usize {
            for x in 2..6usize {
                let bpp = 4 * sample.bytes();
                let input = std::array::from_fn(|i| try_read_sample_f64(&raw[(y * 8 + x) * bpp..(y * 8 + x + 1) * bpp], sample, i).unwrap());
                assert_eq!(part.pixels()[(y - 1) * 4 + x - 2].map(f64::to_bits), reference(&reference_parts, input, values).map(f64::to_bits));
            }
        }
        let display = PreparedBackedStockPointDocument::new(&owner, basis, Default::default(), &ctl()).unwrap();
        let band = display.render_band(Rect::new(2, 1, 6, 3), &ctl()).unwrap();
        for (actual, filtered) in band.pixels().iter().zip(part.pixels()) {
            assert_eq!(actual.map(f64::to_bits), crate::gimp_composite::normal_over_zero_backdrop(*filtered, 0.375).unwrap().map(f64::to_bits));
        }
        drop(display);
        drop(context);
        assert_eq!(part.pixels().len(), 8);
        // Clone is a prospectively prepaid harness operation, not the production edit.
        let clone_guard = clone_retention(&owner);
        let mut disabled = owner.document().clone();
        disabled.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        check_closed_clone(&disabled, &q);
        let disabled_owner = adopt(disabled, q.clone(), clone_guard);
        let disabled = PreparedBackedStockPointContext::new(&disabled_owner, basis, Default::default(), &ctl()).unwrap();
        disabled.validate_operative(&ctl()).unwrap();
        let identity = disabled.render_wide_band(Rect::new(0, 0, 8, 6), &ctl()).unwrap();
        for (actual, bytes) in identity.pixels().iter().zip(raw.chunks_exact(4 * sample.bytes())) {
            let expected: [f64; 4] = std::array::from_fn(|i| try_read_sample_f64(bytes, sample, i).unwrap());
            assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
        }
        let clone_guard = clone_retention(&owner);
        let mut empty = owner.document().clone();
        let state = empty.xcf_state.as_mut().unwrap();
        state.filter_stacks[0].nodes.clear();
        state.filter_masks.clear();
        state.items.retain(|item| item.item == ItemRef::Layer(id));
        check_closed_clone(&empty, &q);
        let empty_owner = adopt(empty, q.clone(), clone_guard);
        let empty = PreparedBackedStockPointContext::new(&empty_owner, basis, Default::default(), &ctl()).unwrap();
        empty.validate_operative(&ctl()).unwrap();
        assert!(empty.node_id().is_none());
    }
}
#[derive(Debug)]
struct DefaultStore {
    descriptor: NativePlaneDescriptor,
    quota: BackingQuota,
    _fee: QuotaLease,
}
impl NativePlaneStore for DefaultStore {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, _: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        out.fill(0);
        Ok(())
    }
}
#[test]
#[ignore = "UNRUN inactive complete native/default/mask/profile and recipe refusals"]
fn inactive_flags_do_not_hide_malformed_inputs_or_foreign_roles() {
    for defect in 0..13 {
        let (mut doc, q, guard, _, basis, _) = vibrance_fixture(SampleType::F64, [25., 0.25]);
        // Prospective fixture-only physical mask replacement / model clone
        // allowance; it remains held with the actual owner before allocation.
        let guard: Arc<dyn Any + Send + Sync> = Arc::new((guard, q.reserve(QuotaKind::Resident, 3 << 20).unwrap()));
        doc.layers[0].visible = false;
        doc.layers[0].opacity = 0.;
        let id = doc.layers[0].id;
        let format = doc.pixel_format();
        let bounds = doc.bounds();
        let state = doc.xcf_state.as_mut().unwrap();
        state.items.iter_mut().find(|item| item.item == ItemRef::Layer(id)).unwrap().visible = false;
        state.filter_stacks[0].nodes[0].enabled = false;
        match defect {
            0 | 1 | 2 => {
                let bytes = native(
                    [
                        if defect == 0 {
                            f64::NAN
                        } else if defect == 1 {
                            f64::INFINITY
                        } else {
                            1.25
                        },
                        0.5,
                        0.75,
                        0.5,
                    ],
                    SampleType::F64,
                );
                let LayerContent::BackedRaster(backed) = &doc.layers[0].content else { unreachable!() };
                doc.layers[0].content = LayerContent::BackedRaster(backed.try_edited_native(Rect::new(7, 5, 8, 6), &bytes, &ctl()).unwrap());
            }
            3 => state.filter_masks[0].surface.try_write_interleaved(Rect::new(255, 255, 256, 256), &f64::NAN.to_ne_bytes()).unwrap(),
            4 => doc.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
            5 => {
                let default = native([0., 0., 0., f64::NAN], SampleType::F64);
                let descriptor = NativePlaneDescriptor::new(format, bounds, &default).unwrap();
                let fee = q.reserve(QuotaKind::Resident, 512).unwrap();
                let store = Arc::new(DefaultStore { descriptor, quota: q.clone(), _fee: fee });
                let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
                doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
            }
            6 => {
                state.filter_stacks[0].nodes[0].composition.as_mut().unwrap().resolved = Some(photocraft_doc::import_compositing::ResolvedGimpComposition {
                    blend_space: GimpSpace::LinearRgb,
                    composite_space: GimpSpace::LinearRgb,
                    operator: GimpOperator::Union,
                })
            }
            7 => state.filter_stacks[0].nodes[0].arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(0.5f64.to_bits())),
            8 => doc.source_color_context.as_mut().unwrap().source_precision = 700,
            9 => state.filter_stacks[0].nodes[0].arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())),
            10 => state.filter_stacks[0].nodes[0].arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(11f32.to_bits())),
            11 => state.filter_stacks[0].nodes[0].arguments[1].name = "vibrance".into(),
            _ => state.filter_stacks[0].nodes[0].operation.schema_epoch = crate::HUE_CHROMA_EPOCH.into(),
        }
        // Some structural defects are rejected even before adoption; neither
        // route is allowed to publish a usable unchecked CURRENT renderer.
        match AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()) {
            Ok(owner) => {
                let result = PreparedBackedStockPointContext::new(&owner, basis, Default::default(), &ctl());
                match result {
                    Ok(context) => assert!(context.validate_operative(&ctl()).is_err()),
                    Err(_) => {}
                }
            }
            Err(_) => {}
        }
    }
}

#[test]
#[ignore = "UNRUN actual source/preparation fees, typed work/cancel/quota and cleanup"]
fn preparation_and_execution_share_cumulative_control_and_real_owner_fees() {
    struct Budget {
        quota: BackingQuota,
        work: u64,
    }
    impl PreparationBudget for Budget {
        type Lease = QuotaLease;
        fn reserve_prepared_bytes(&mut self, bytes: u64) -> Result<QuotaLease, CompositionError> {
            self.quota.reserve(QuotaKind::Resident, bytes).map_err(|_| photocraft_raster::RasterInputError::LimitExceeded.into())
        }
        fn consume_preparation_work(&mut self, n: u64) -> Result<(), CompositionError> {
            self.work = self.work.checked_add(n).ok_or(CompositionError::Invalid("draft work"))?;
            Ok(())
        }
    }
    let (doc, q, guard, reads, basis, raw) = vibrance_fixture(SampleType::F64, [25., 0.25]);
    let baseline = q.usage().unwrap();
    let token = q.reserve(QuotaKind::Resident, 8192).unwrap();
    let mut budget = Budget { quota: q.clone(), work: 0 };
    let pipeline = PreparedStockPointPipeline::new_budgeted(&doc, &Interrupt::NONE, &mut budget, token).unwrap();
    assert!(budget.work > photocraft_cms::owned_linear_srgb_lab::planning_work_ceiling());
    assert!(q.usage().unwrap().resident > baseline.resident + 8192);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    drop(pipeline);
    assert_eq!(q.usage().unwrap(), baseline);
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(PreparedStockPointPipeline::prepare(&doc, basis, &q, &denied), Err(BackedNormalError::Backing(BackingError::Work))));
    let stop = || true;
    let token = Interrupt::cancel_only(&stop);
    assert!(matches!(
        PreparedStockPointPipeline::prepare(&doc, basis, &q, &ReadControl { interrupt: &token, work: &|_| Ok(()) }),
        Err(BackedNormalError::Backing(BackingError::Cancelled))
    ));
    assert_eq!(q.usage().unwrap(), baseline);
    {
        let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - baseline.resident).unwrap();
        let full = q.usage().unwrap();
        assert!(PreparedStockPointPipeline::prepare(&doc, basis, &q, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), full);
        drop(blocker);
    }
    let owner = adopt(doc, q.clone(), guard);
    let context = PreparedBackedStockPointContext::new(&owner, basis, Default::default(), &ctl()).unwrap();
    context.validate_operative(&ctl()).unwrap(); // warm persistent native cache before refund baseline
    let baseline = q.usage().unwrap();
    assert!(matches!(context.render_wide_band(Rect::new(0, 0, 8, 6), &denied), Err(BackedNormalError::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let polls = AtomicU64::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) > 6;
    let interrupt = Interrupt::cancel_only(&cancel);
    assert!(matches!(
        context.render_wide_band(Rect::new(0, 0, 8, 6), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }),
        Err(BackedNormalError::Backing(BackingError::Cancelled))
    ));
    assert_eq!(q.usage().unwrap(), baseline);
    let pipeline = PreparedStockPointPipeline::prepare(owner.document(), basis, &q, &ctl()).unwrap();
    let input = &raw[..32];
    let mut exact = 0u64;
    pipeline.evaluate_native(input, owner.document().pixel_format(), &ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }).unwrap();
    let total = std::cell::Cell::new(0u64);
    let charge = |n| {
        total.set(total.get() + n);
        Ok(())
    };
    pipeline.evaluate_native(input, owner.document().pixel_format(), &ReadControl { interrupt: &Interrupt::NONE, work: &charge }).unwrap();
    exact += total.get();
    assert!(exact > 2 * (3 * 128 + 137 + 128)); // actual physical directions plus polar/stage/Float work
}
