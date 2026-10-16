//! Ignored UNRUN actual paid native CURRENT/Source-preparation contracts.
//! Synthetic descriptors/metadata are not original GIMP import or native-codec oracles.
use super::*;
use crate::gimp_masked_scalar::{Kind, Parameters};
use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, OperationIdentity};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
fn ctl() -> ReadControl<'static> {
    tests::ctl()
}
fn number(name: &str, v: f32) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(v.to_bits())) }
}
fn configured(kind: Kind, sample: SampleType) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, Arc<AtomicU64>, Vec<u8>) {
    let (mut d, q, guard, reads, _, raw) = tests::fixture(2, ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
    let state = d.xcf_state.as_mut().unwrap();
    state.provenance.format_version = 22;
    state.current_filter_output_precision = photocraft_doc::FilterOutputPrecision::Canonical;
    state.items[0].composition.as_mut().unwrap().compatibility_epoch = "xcf-22-normal".into();
    let node = &mut state.filter_stacks[0].nodes[0];
    node.operation = OperationIdentity { name: kind.operation_name().into(), stored_version: kind.version().into(), schema_epoch: kind.epoch().into() };
    node.composition.as_mut().unwrap().compatibility_epoch = kind.epoch().into();
    node.arguments = match kind {
        Kind::InvertLinear | Kind::InvertGamma => vec![],
        Kind::BrightnessContrast => vec![number("contrast", 0.75), number("brightness", -0.125)],
        Kind::GeglLevels => vec![number("in-low", 0.125), number("in-high", 0.875), number("out-low", 0.0625), number("out-high", 0.9375)],
    };
    // Existing fixture2MiB prehold precedes these bounded argument/map/tile edits.
    state.filter_masks[0].surface.try_write_pixel_f64(0, 0, &[0.375]).unwrap();
    state.filter_masks[0].surface.try_write_pixel_f64(7, 5, &[if sample == SampleType::F64 { f64::from_bits(0x3ff0000009b25010) } else { 1. }]).unwrap();
    (d, q, guard, reads, raw)
}
struct Budget {
    q: BackingQuota,
    used: u64,
}
impl PreparationBudget for Budget {
    type Lease = QuotaLease;
    fn reserve_prepared_bytes(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        self.q.reserve(QuotaKind::Resident, n).map_err(|_| photocraft_raster::RasterInputError::LimitExceeded.into())
    }
    fn consume_preparation_work(&mut self, n: u64) -> Result<(), CompositionError> {
        self.used = self.used.checked_add(n).ok_or(CompositionError::Invalid("fixturework"))?;
        Ok(())
    }
}
// Public old lower operation + public TRC, independently selected full mask math.
// All reference/profile/expected clones are external harness, not quota receipts.
fn reference(p: Parameters, before: [f64; 4], raw: f64) -> [f64; 4] {
    use photocraft_algo::gimp::point::PointOperation;
    let curve = photocraft_cms::srgb_trc();
    let linear = p.kind() != Kind::InvertGamma;
    let mut working = before;
    if linear {
        for v in &mut working[..3] {
            *v = curve.eval64(*v);
        }
    }
    let op = match p {
        Parameters::InvertLinear | Parameters::InvertGamma => PointOperation::Invert,
        Parameters::BrightnessContrast(p) => {
            let [brightness, contrast] = p.values();
            PointOperation::BrightnessContrast { brightness, contrast }
        }
        Parameters::GeglLevels(p) => {
            let [input_low, input_high, output_low, output_high] = p.values();
            PointOperation::Levels { input_low, input_high, output_low, output_high }
        }
    };
    let mut after = [0.; 4];
    op.prepare().unwrap().try_eval_pixel(&working, 3, &mut after).unwrap();
    if linear {
        for v in &mut after[..3] {
            *v = curve.eval_inverse64(*v);
        }
    }
    let b = [curve.eval64(before[0]), curve.eval64(before[1]), curve.eval64(before[2])];
    let a = [curve.eval64(after[0]), curve.eval64(after[1]), curve.eval64(after[2])];
    let t = f64::from(raw as f32);
    let alpha = before[3] + t * (after[3] - before[3]);
    let mut out = [0.; 4];
    for k in 0..3 {
        let v = if alpha == 0. {
            b[k] + t * (a[k] - b[k])
        } else {
            let bp = b[k] * before[3];
            let ap = a[k] * after[3];
            (bp + t * (ap - bp)) / alpha
        };
        out[k] = f64::from(curve.eval_inverse64(v) as f32);
    }
    out[3] = f64::from(alpha as f32);
    out
}
#[test]
#[ignore = "UNRUN actualsameq nativeprovider/fourkindSourceprepare andCurrentPoint/fullmaskretention afterdocdrop, independentoldscalar+maskreference atlocal0/1/fraction andrawarg/mask custody"]
fn source_current_and_retained_mask_lifetime() {
    for sample in [SampleType::F32, SampleType::F64] {
        for kind in [Kind::InvertLinear, Kind::InvertGamma, Kind::BrightnessContrast, Kind::GeglLevels] {
            let (d, q, guard, reads, raw) = configured(kind, sample);
            let before = q.usage().unwrap();
            let r = reads.load(Ordering::Relaxed);
            let mut budget = Budget { q: q.clone(), used: 0 };
            drop(PreparedStockPointPipeline::new_budgeted(&d, &Interrupt::NONE, &mut budget, guard.clone()).unwrap());
            assert_eq!(q.usage().unwrap(), before);
            assert_eq!(reads.load(Ordering::Relaxed), r);
            let params = crate::masked_scalar_parameters(kind, &d.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments).unwrap();
            let nodes = d.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.clone();
            let mask = d.xcf_state.as_ref().unwrap().filter_masks[0].surface.clone();
            let format = d.pixel_format();
            let pipeline = PreparedStockPointPipeline::prepare(&d, Builtin::Srgb.profile(), &q, &ctl()).unwrap();
            assert_eq!(d.xcf_state.as_ref().unwrap().filter_stacks[0].nodes, nodes);
            let pixel = &raw[..format.bytes_per_pixel()];
            let input: [f64; 4] = std::array::from_fn(|c| try_read_sample_f64(pixel, sample, c).unwrap());
            for (position, coverage) in
                [((0, 0), 0.375), ((1, 0), 0.), ((7, 5), if sample == SampleType::F64 { f64::from_bits(0x3ff0000009b25010) } else { 1. })]
            {
                assert_eq!(
                    pipeline.evaluate_native_at(pixel, format, position, &ctl()).unwrap().map(f64::to_bits),
                    reference(params, input, coverage).map(f64::to_bits)
                );
            }
            drop(d);
            drop(guard);
            let result = pipeline.evaluate_native_at(pixel, format, (0, 0), &ctl()).unwrap();
            assert!(result.iter().all(|v| v.is_finite()));
            let StockPointProgram::MaskedScalar { mask: retained, .. } = &pipeline.plan.stack else { panic!("program") };
            assert_eq!(retained, &mask);
            drop(mask);
            drop(pipeline);
        }
    }
}
#[test]
#[ignore = "UNRUN exactold/mixed/wrongepoch/typed/opacity/geometry refusals, fullinactivephysical/default/padding/checkedFloat/nonempty proof and16MiB O1 capbeforeactualscan"]
fn strict_selected_role_and_full_physical_refusal() {
    for fault in 0..8 {
        let (mut d, q, _guard, reads, _raw) = configured(Kind::GeglLevels, SampleType::F64);
        let state = d.xcf_state.as_mut().unwrap();
        let node = &mut state.filter_stacks[0].nodes[0];
        match fault {
            0 => node.operation.schema_epoch = crate::GEGL_LEVELS_MIXED_EPOCH.into(),
            1 => node.operation.stored_version = "1:0".into(),
            2 => node.arguments[0] = number("in-low", 1.),
            3 => node.opacity_f64_bits = 0.375f64.to_bits(),
            4 => state.filter_masks[0].surface.try_write_pixel_f64(255, 255, &[-0.]).unwrap(),
            5 => state.filter_masks[0].surface.try_write_pixel_f64(0, 0, &[f64::NAN]).unwrap(),
            6 => state.filter_masks[0].surface = photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false)),
            _ => state.items[0].raster_bounds = Some(Rect::new(1, 0, 9, 6)),
        }
        let before = q.usage().unwrap();
        let r = reads.load(Ordering::Relaxed);
        assert!(PreparedStockPointPipeline::prepare(&d, Builtin::Srgb.profile(), &q, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(reads.load(Ordering::Relaxed), r);
    }
    for fault in 0..4 {
        let (mut d, q, _guard, _reads, _raw) = configured(Kind::InvertGamma, SampleType::F64);
        d.layers[0].visible = false;
        d.layers[0].opacity = 0.;
        let state = d.xcf_state.as_mut().unwrap();
        state.items[0].visible = false;
        state.filter_stacks[0].nodes[0].enabled = false;
        match fault {
            0 => state.filter_masks[0].surface.try_write_pixel_f64(0, 0, &[-0.1]).unwrap(),
            1 => state.filter_masks[0].surface.try_write_pixel_f64(0, 0, &[1.001]).unwrap(),
            2 => state.filter_stacks[0].nodes[0].source_region = Some(1),
            _ => d.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
        }
        let before = q.usage().unwrap();
        assert!(PreparedStockPointPipeline::prepare(&d, Builtin::Srgb.profile(), &q, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let (mut d, q, _guard, reads, _raw) = configured(Kind::InvertLinear, SampleType::F64);
    let _map_index = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let _map_resident = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    let mask = &mut d.xcf_state.as_mut().unwrap().filter_masks[0].surface;
    let tile = Arc::clone(mask.tiles().next().unwrap().1);
    for tx in 1..65 {
        mask.put_tiles(std::iter::once((photocraft_geom::TileCoord { tx, ty: 0 }, tile.clone())));
    }
    let baseline = q.usage().unwrap();
    let r = reads.load(Ordering::Relaxed);
    let work = AtomicU64::new(0);
    let count = |n| {
        work.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    assert!(PreparedStockPointPipeline::prepare(&d, Builtin::Srgb.profile(), &q, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).is_err());
    assert!(work.load(Ordering::Relaxed) < 1 << 20);
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(reads.load(Ordering::Relaxed), r); // cap precedes invalid-coordinate/payload scans
}
#[test]
#[ignore = "UNRUN genuineaccountedPoint/fullnativeoperative proof anddisabledwidepass, warmedband Work/cancel typedcleanup onactualprovider; no nativecodec execution"]
fn actual_point_owner_and_late_control_cleanup() {
    for kind in [Kind::InvertLinear, Kind::InvertGamma, Kind::BrightnessContrast, Kind::GeglLevels] {
        let (d, q, guard, reads, raw) = configured(kind, SampleType::F64);
        let owner = AccountedDocument::new_readonly_point_storage(d, guard, q.clone(), &ctl()).unwrap();
        let context = PreparedBackedStockPointContext::new(&owner, Builtin::Srgb.profile(), Default::default(), &ctl()).unwrap();
        context.validate_operative(&ctl()).unwrap();
        let rect = Rect::new(0, 0, 2, 1);
        drop(context.render_wide_band(rect, &ctl()).unwrap());
        assert!(reads.load(Ordering::Relaxed) > 0);
        let baseline = q.usage().unwrap();
        let total = AtomicU64::new(0);
        let count = |n| {
            total.fetch_add(n, Ordering::Relaxed);
            Ok(())
        };
        drop(context.render_wide_band(rect, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap());
        let limit = total.load(Ordering::Relaxed) - 1;
        let used = AtomicU64::new(0);
        let deny = |n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > limit { Err(BackingError::Work) } else { Ok(()) };
        assert!(matches!(
            context.render_wide_band(rect, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }),
            Err(BackedNormalError::Backing(BackingError::Work))
        ));
        assert_eq!(q.usage().unwrap(), baseline);
        let polls = AtomicU64::new(0);
        let observe = || {
            polls.fetch_add(1, Ordering::Relaxed);
            false
        };
        let i = Interrupt::cancel_only(&observe);
        drop(context.render_wide_band(rect, &ReadControl { interrupt: &i, work: &|_| Ok(()) }).unwrap());
        let accepted = polls.load(Ordering::Relaxed);
        let polls = AtomicU64::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= accepted - 1;
        let i = Interrupt::cancel_only(&cancel);
        assert!(matches!(
            context.render_wide_band(rect, &ReadControl { interrupt: &i, work: &|_| Ok(()) }),
            Err(BackedNormalError::Backing(BackingError::Cancelled))
        ));
        assert_eq!(q.usage().unwrap(), baseline);
        drop(context);
        let (mut d, q, _guard, _reads, _) = configured(kind, SampleType::F64);
        d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        let pipe = PreparedStockPointPipeline::prepare(&d, Builtin::Srgb.profile(), &q, &ctl()).unwrap();
        let format = d.pixel_format();
        let input: [f64; 4] = std::array::from_fn(|c| try_read_sample_f64(&raw[..format.bytes_per_pixel()], SampleType::F64, c).unwrap());
        assert_eq!(pipe.evaluate_native_at(&raw[..format.bytes_per_pixel()], format, (0, 0), &ctl()).unwrap().map(f64::to_bits), input.map(f64::to_bits));
    }
}
