//! Ignored UNRUN CURRENT-contract drafts. Public CMS + dedicated lower math
//! references are not a GIMP oracle, source enrollment, or numerical acceptance.
use super::tests::{adopt, ctl, fixture};
use super::*;
use crate::backed_stock_point_document::PreparedBackedStockPointDocument;
use photocraft_algo::gimp::point::PointOperation;
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

fn semi_fixture(
    sample: SampleType,
    encoding: SampleEncoding,
    alpha: f32,
) -> (Document, BackingQuota, Arc<dyn Any + Send + Sync>, Arc<AtomicU64>, &'static Profile, Vec<u8>) {
    let (mut doc, q, guard, reads, profile, raw) = fixture(0, ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation =
        OperationIdentity { name: "gimp:semi-flatten".into(), stored_version: "0:0".into(), schema_epoch: gimp_filters::SEMI_FLATTEN_EPOCH.into() };
    node.arguments = vec![FilterArgument {
        name: "color".into(),
        value: FilterValue::Color {
            components: [0.75f32, 0.25, 0.125, alpha].map(|v| ExactNumber::Float32Bits(v.to_bits())).to_vec(),
            encoding,
            profile: None,
        },
    }];
    let policy = node.composition.as_mut().unwrap();
    policy.compatibility_epoch = gimp_filters::SEMI_FLATTEN_EPOCH.into();
    // These three words are opaque provenance, not a mode/space alias. Other
    // values in this same closed recipe must survive without normalization.
    policy.stored_blend_space = i32::MIN;
    policy.stored_composite_space = 17;
    policy.stored_operator = i32::MAX;
    (doc, q, guard, reads, profile, raw)
}
fn reference(input: [f64; 4], encoding: SampleEncoding, literal_alpha: f32) -> [f64; 4] {
    let curve = photocraft_cms::curve::srgb_trc();
    let mut color = [0.75f32, 0.25, 0.125, literal_alpha];
    if encoding == SampleEncoding::Linear {
        for c in color.iter_mut().take(3) {
            *c = curve.eval_inverse64(f64::from(*c)) as f32;
        }
    }
    let working = [curve.eval64(input[0]), curve.eval64(input[1]), curve.eval64(input[2]), input[3]].map(|v| f64::from(v as f32));
    let mut filtered = [0.; 4];
    PointOperation::SemiFlatten { color }.try_eval_pixel(&working, 3, &mut filtered).unwrap();
    [curve.eval_inverse64(filtered[0]), curve.eval_inverse64(filtered[1]), curve.eval_inverse64(filtered[2]), filtered[3]]
}
fn native(pixel: [f64; 4], sample: SampleType) -> Vec<u8> {
    let mut bytes = vec![0; 4 * sample.bytes()];
    for (i, v) in pixel.into_iter().enumerate() {
        photocraft_color::try_write_sample_f64(&mut bytes, sample, i, v).unwrap();
    }
    bytes
}

#[test]
#[ignore = "UNRUN tagged literal/Float working/alpha-changing CURRENT contract, no producer oracle"]
fn literal_tags_and_alpha_endpoints_follow_explicit_working_schedule() {
    for sample in [SampleType::F32, SampleType::F64] {
        for encoding in [SampleEncoding::Linear, SampleEncoding::NonlinearProfile] {
            let mut previous = None;
            for literal_alpha in [0., 0.375, 1.] {
                let (doc, q, _guard, reads, basis, _) = semi_fixture(sample, encoding, literal_alpha);
                let original = doc.xcf_state.as_ref().unwrap().clone();
                let pipeline = PreparedStockPointPipeline::prepare(&doc, basis, &q, &ctl()).unwrap();
                assert_eq!(reads.load(Ordering::Relaxed), 0);
                let mut results = Vec::new();
                for alpha in [0., -0., 0.125, 0.375, 0.5, 0.875, 1., f64::from_bits(1f64.to_bits() - 1)] {
                    let raw = native([0.125, 0.5, 0.875, alpha], sample);
                    let input = std::array::from_fn(|i| try_read_sample_f64(&raw, sample, i).unwrap());
                    let actual = pipeline.evaluate_native(&raw, doc.pixel_format(), &ctl()).unwrap();
                    assert_eq!(actual.map(f64::to_bits), reference(input, encoding, literal_alpha).map(f64::to_bits));
                    if input[3] == 0. {
                        assert_eq!(actual[3].to_bits(), input[3].to_bits());
                    } else if (input[3] as f32) < 1. {
                        assert_eq!(actual[3], 1.);
                    }
                    results.push(actual.map(f64::to_bits));
                }
                if let Some(previous) = &previous {
                    assert_eq!(previous, &results);
                }
                previous = Some(results);
                assert_eq!(doc.xcf_state.as_ref().unwrap(), &original);
            }
        }
        let (linear, q1, _g1, _, basis, _) = semi_fixture(sample, SampleEncoding::Linear, 1.);
        let (nonlinear, q2, _g2, _, _, _) = semi_fixture(sample, SampleEncoding::NonlinearProfile, 1.);
        let raw = native([0.125, 0.5, 0.875, 0.125], sample);
        let a = PreparedStockPointPipeline::prepare(&linear, basis, &q1, &ctl()).unwrap().evaluate_native(&raw, linear.pixel_format(), &ctl()).unwrap();
        let b = PreparedStockPointPipeline::prepare(&nonlinear, basis, &q2, &ctl()).unwrap().evaluate_native(&raw, nonlinear.pixel_format(), &ctl()).unwrap();
        assert_ne!(a.map(f64::to_bits), b.map(f64::to_bits));
    }
}

#[test]
#[ignore = "UNRUN full provider/display/ROI and explicit derived empty ownership"]
fn paid_native_owner_display_and_empty_after_remove_keep_distinct_semantics() {
    for sample in [SampleType::F32, SampleType::F64] {
        let (mut doc, q, guard, _, basis, raw) = semi_fixture(sample, SampleEncoding::Linear, 0.375);
        doc.layers[0].opacity = 0.375;
        let id = doc.layers[0].id;
        let owner = adopt(doc, q.clone(), guard);
        let context = PreparedBackedStockPointContext::new(&owner, basis, NormalBandLimits { max_rows: 2, max_pixels: 8 }, &ctl()).unwrap();
        context.validate_operative(&ctl()).unwrap();
        let part = context.render_wide_band(Rect::new(2, 1, 6, 3), &ctl()).unwrap();
        for y in 1..3usize {
            for x in 2..6usize {
                let bpp = owner.document().pixel_format().bytes_per_pixel();
                let pixel = &raw[(y * 8 + x) * bpp..(y * 8 + x + 1) * bpp];
                let input = std::array::from_fn(|i| try_read_sample_f64(pixel, sample, i).unwrap());
                assert_eq!(part.pixels()[(y - 1) * 4 + x - 2].map(f64::to_bits), reference(input, SampleEncoding::Linear, 0.375).map(f64::to_bits));
            }
        }
        let display = PreparedBackedStockPointDocument::new(&owner, basis, Default::default(), &ctl()).unwrap();
        let band = display.render_band(Rect::new(2, 1, 6, 3), &ctl()).unwrap();
        for (actual, input) in band.pixels().iter().zip(part.pixels()) {
            let expected = crate::gimp_composite::normal_over_zero_backdrop(*input, 0.375).unwrap();
            assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
        }
        drop(display);
        drop(context);
        assert_eq!(part.pixels().len(), 8); // independently retained output lease
        assert_eq!(owner.document().layers[0].id, id);

        // Standalone test-model clone is an external harness operation. The
        // production remove copy/native persistence is covered by engine/hosts.
        let guard: Arc<dyn Any + Send + Sync> = Arc::new(q.reserve(QuotaKind::Resident, 2 << 20).unwrap());
        let mut empty = owner.document().clone();
        let state = empty.xcf_state.as_mut().unwrap();
        state.filter_stacks[0].nodes.clear();
        state.filter_masks.clear();
        state.items.retain(|item| item.item == ItemRef::Layer(id));
        let empty_owner = adopt(empty, q, guard);
        let empty = PreparedBackedStockPointContext::new(&empty_owner, basis, Default::default(), &ctl()).unwrap();
        empty.validate_operative(&ctl()).unwrap();
        assert!(empty.node_id().is_none());
        let plain = empty.render_wide_band(Rect::new(0, 0, 8, 6), &ctl()).unwrap();
        for (actual, bytes) in plain.pixels().iter().zip(raw.chunks_exact(4 * sample.bytes())) {
            let expected: [f64; 4] = std::array::from_fn(|i| try_read_sample_f64(bytes, sample, i).unwrap());
            assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
        }
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
    for defect in 0..9 {
        let (mut doc, q, guard, _, basis, _) = semi_fixture(SampleType::F64, SampleEncoding::NonlinearProfile, 1.);
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
            7 => {
                state.filter_stacks[0].nodes[0].arguments[0].value =
                    FilterValue::Color { components: vec![ExactNumber::Float64Bits(0.5f64.to_bits()); 4], encoding: SampleEncoding::Linear, profile: None }
            }
            _ => doc.source_color_context.as_mut().unwrap().source_precision = 700,
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
    let (doc, q, guard, reads, basis, raw) = semi_fixture(SampleType::F64, SampleEncoding::Linear, 0.375);
    let baseline = q.usage().unwrap();
    let token = q.reserve(QuotaKind::Resident, 8192).unwrap();
    let mut budget = Budget { quota: q.clone(), work: 0 };
    let pipeline = PreparedStockPointPipeline::new_budgeted(&doc, &Interrupt::NONE, &mut budget, token).unwrap();
    assert!(budget.work > crate::gimp_semi_flatten::LITERAL_PREPARATION_WORK);
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
    assert!(exact >= 3 * 128 + 9 * 128 + 128);
}
