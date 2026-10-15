//! UNRUN synthetic CURRENT storage/recipe drafts. These exercise real paid
//! native backing and the runtime class, not GIMP source qualification.
use super::*;
use crate::gimp_filters::PointStack;
use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, OperationIdentity};
use photocraft_raster::Interrupt;
use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub(crate) fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn number(name: &str, value: f32) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(value.to_bits())) }
}
pub(crate) fn configure(doc: &mut Document, member: usize) {
    let state = doc.xcf_state.as_mut().unwrap();
    let node = &mut state.filter_stacks[0].nodes[0];
    let (name, epoch, arguments) = match member {
        0 => ("gegl:brightness-contrast", gimp_filters::BRIGHTNESS_CONTRAST_PRECISION_EPOCH, vec![number("brightness", 0.125), number("contrast", 1.125)]),
        1 => (
            "gegl:levels",
            gimp_filters::GEGL_LEVELS_PRECISION_EPOCH,
            vec![number("in-low", 0.125), number("in-high", 0.875), number("out-low", 0.0625), number("out-high", 0.9375)],
        ),
        2 => ("gegl:exposure", gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH, vec![number("exposure", 0.5), number("black-level", 0.03125)]),
        3 => (
            "gegl:color-temperature",
            gimp_filters::COLOR_TEMPERATURE_PRECISION_EPOCH,
            vec![number("original-temperature", 6500.0), number("intended-temperature", 9500.0)],
        ),
        4 => (
            "gegl:sepia",
            gimp_filters::SEPIA_PRECISION_EPOCH,
            vec![number("scale", 0.375), FilterArgument { name: "srgb".into(), value: FilterValue::Bool(true) }],
        ),
        5 | 6 => {
            let mut arguments = vec![FilterArgument {
                name: "color-model".into(),
                value: FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: if member == 6 { 1 } else { 0 } },
            }];
            for name in ["cpn-1-frequency", "cpn-2-frequency", "cpn-3-frequency"] {
                arguments.push(number(name, 1.25));
            }
            for name in ["cpn-1-phaseshift", "cpn-2-phaseshift", "cpn-3-phaseshift"] {
                arguments.push(number(name, 25.0));
            }
            for name in ["cpn-1-keep", "cpn-2-keep", "cpn-3-keep"] {
                arguments.push(FilterArgument { name: name.into(), value: FilterValue::Bool(false) });
            }
            ("gegl:alien-map", if member == 6 { gimp_filters::ALIEN_MAP_HSL_PRECISION_EPOCH } else { gimp_filters::ALIEN_MAP_PRECISION_EPOCH }, arguments)
        }
        7 => ("gegl:invert-linear", gimp_filters::INVERT_LINEAR_PRECISION_EPOCH, vec![]),
        8 => ("gegl:invert-gamma", gimp_filters::INVERT_GAMMA_PRECISION_EPOCH, vec![]),
        _ => panic!("fixed fixture member"),
    };
    node.operation = OperationIdentity { name: name.into(), stored_version: if member == 2 { "1:0".into() } else { "0:0".into() }, schema_epoch: epoch.into() };
    node.arguments = arguments;
    node.composition.as_mut().unwrap().compatibility_epoch = gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into();
}
pub(crate) fn fixture(
    member: usize,
    mode: ColorMode,
    sample: SampleType,
    stored: SampleEncoding,
) -> (Document, BackingQuota, Arc<dyn Any + Send + Sync>, Arc<AtomicU64>, &'static Profile, Vec<u8>) {
    let (mut doc, q, guard, reads, profile, raw) = crate::backed_threshold_context::tests::fixture(mode, sample, stored);
    doc.source_color_context.as_mut().unwrap().source_precision = match (sample, stored) {
        (SampleType::U8, SampleEncoding::Linear) => 100,
        (SampleType::U8, SampleEncoding::NonlinearProfile) => 150,
        (SampleType::U8, _) => 175,
        (SampleType::U16, SampleEncoding::Linear) => 200,
        (SampleType::U16, SampleEncoding::NonlinearProfile) => 250,
        (SampleType::U16, _) => 275,
        (SampleType::U32, SampleEncoding::Linear) => 300,
        (SampleType::U32, SampleEncoding::NonlinearProfile) => 350,
        (SampleType::U32, _) => 375,
        (SampleType::F32, SampleEncoding::Linear) => 600,
        (SampleType::F32, SampleEncoding::NonlinearProfile) => 650,
        (SampleType::F32, _) => 675,
        (SampleType::F64, SampleEncoding::Linear) => 700,
        (SampleType::F64, SampleEncoding::NonlinearProfile) => 750,
        _ => 775,
    };
    // Match the actual mapper: current ICC is the effective stored encoding,
    // while PointStack's operation basis remains the nonlinear builtin.
    let effective = photocraft_cms::xcf_color::effective_profile(
        None,
        if mode == ColorMode::Rgb { photocraft_cms::ColorSpace::Rgb } else { photocraft_cms::ColorSpace::Gray },
        crate::gimp_composite::encoding(stored),
    )
    .unwrap();
    doc.icc_profile = Some(effective.to_bytes());
    doc.working_profile_interpretation = if mode == ColorMode::Rgb {
        WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap())
    } else {
        WorkingProfileInterpretation::implicit_sgray(doc.icc_profile.as_ref().unwrap())
    };
    configure(&mut doc, member);
    (doc, q, guard, reads, profile, raw)
}
pub(crate) fn adopt(doc: Document, q: BackingQuota, guard: Arc<dyn Any + Send + Sync>) -> AccountedDocument {
    AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()).unwrap()
}

#[test]
fn all_nine_current_members_match_old_stack_and_preserve_wide_precision_and_raw_alpha() {
    for member in 0..9 {
        for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            for stored in [SampleEncoding::Linear, SampleEncoding::NonlinearProfile, SampleEncoding::Perceptual] {
                let (doc, q, _guard, _, profile, raw) = fixture(member, ColorMode::Rgb, sample, stored);
                let pipeline = PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).unwrap();
                let nodes = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes;
                let mut old = PointStack::from_nodes_at(nodes, profile, stored, 22, doc.bounds()).unwrap();
                old.set_current_sample(Some(sample));
                for pixel in raw.chunks_exact(doc.pixel_format().bytes_per_pixel()) {
                    let input = std::array::from_fn(|channel| try_read_sample_f64(pixel, sample, channel).unwrap());
                    let expected = old.apply_current(input, sample == SampleType::U32, &mut |_| Ok(())).unwrap();
                    let expected = gimp_filters::current_point_storage_result(
                        expected,
                        match sample {
                            SampleType::U8 => Some(255),
                            SampleType::U32 => Some(4294967295),
                            _ => None,
                        },
                        false,
                    );
                    let actual = pipeline.evaluate_native(pixel, doc.pixel_format(), &ctl()).unwrap();
                    assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
                    assert_eq!(actual[3].to_bits(), input[3].to_bits());
                }
            }
        }
    }
}

#[test]
fn integer_gray_temperature_uses_existing_live_branch_and_disabled_identity_keeps_f64_low_bits() {
    for sample in [SampleType::U16, SampleType::U32, SampleType::F64] {
        let (mut doc, q, _guard, _, profile, raw) = fixture(3, ColorMode::Grayscale, sample, SampleEncoding::NonlinearProfile);
        let pipeline = PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).unwrap();
        let nodes = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes;
        let old = PointStack::from_nodes_at(nodes, profile, doc.sample_encoding.unwrap(), 22, doc.bounds()).unwrap();
        for pixel in raw.chunks_exact(doc.pixel_format().bytes_per_pixel()) {
            let gray = try_read_sample_f64(pixel, sample, 0).unwrap();
            let alpha = try_read_sample_f64(pixel, sample, 1).unwrap();
            let expected = old.apply_current([gray, gray, gray, alpha], sample == SampleType::U32, &mut |_| Ok(())).unwrap();
            let expected = gimp_filters::current_point_storage_result(expected, if sample == SampleType::U32 { Some(4294967295) } else { None }, false);
            assert_eq!(pipeline.evaluate_native(pixel, doc.pixel_format(), &ctl()).unwrap().map(f64::to_bits), expected.map(f64::to_bits));
        }
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        let bypass = PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).unwrap();
        for pixel in raw.chunks_exact(doc.pixel_format().bytes_per_pixel()) {
            let gray = try_read_sample_f64(pixel, sample, 0).unwrap();
            let alpha = try_read_sample_f64(pixel, sample, 1).unwrap();
            assert_eq!(bypass.evaluate_native(pixel, doc.pixel_format(), &ctl()).unwrap().map(f64::to_bits), [gray, gray, gray, alpha].map(f64::to_bits));
        }
    }
}

#[test]
fn source_recipe_is_not_inferred_from_class_and_failed_preparation_refunds_without_reads() {
    for mutation in 0..6 {
        let (mut doc, q, _guard, reads, profile, _) = fixture(2, ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
        let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        match mutation {
            0 => node.operation.name = "gimp:brightness-contrast".into(),
            1 => node.operation.stored_version = "0:0".into(),
            2 => node.operation.schema_epoch = "unknown".into(),
            3 => node.opacity_f64_bits = 0.5f64.to_bits(),
            4 => node.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(0.5f64.to_bits())),
            _ => node.arguments[1] = node.arguments[0].clone(),
        }
        let before = q.usage().unwrap();
        assert!(PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(reads.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn actual_work_cancel_and_quota_fail_before_native_reads() {
    let (doc, q, _guard, reads, profile, _) = fixture(2, ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    let before = q.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(PreparedStockPointPipeline::prepare(&doc, profile, &q, &denied), Err(BackedNormalError::Backing(BackingError::Work))));
    let cancelled = || true;
    let token = Interrupt::cancel_only(&cancelled);
    let cancelled = ReadControl { interrupt: &token, work: &|_| Ok(()) };
    assert!(matches!(PreparedStockPointPipeline::prepare(&doc, profile, &q, &cancelled), Err(BackedNormalError::Backing(BackingError::Cancelled))));
    let _blocker = q.reserve(QuotaKind::Resident, (64 << 20) - before.resident).unwrap();
    let full = q.usage().unwrap();
    assert!(PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), full);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}

#[test]
fn actual_class_context_validates_complete_operative_extent_and_retains_outputs_after_plan_drop() {
    let (doc, q, guard, _, profile, _) = fixture(8, ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    let owner = adopt(doc, q.clone(), guard);
    let before = q.usage().unwrap();
    let context = PreparedBackedStockPointContext::new(&owner, profile, NormalBandLimits { max_rows: 2, max_pixels: 6 }, &ctl()).unwrap();
    context.validate_operative(&ctl()).unwrap();
    let band = context.render_wide_band(Rect::new(0, 0, 3, 2), &ctl()).unwrap();
    assert_eq!(band.pixels().len(), 6);
    assert!(band.resident_bytes() >= 6 * 32);
    drop(context);
    let held = q.usage().unwrap();
    assert!(held.resident >= before.resident + band.resident_bytes());
    let _unchanged = owner.document().layers[0].id;
    drop(band);
    // Native source cache may remain in its actual owner; only the derivative
    // output fee is expected to refund here.
    assert!(q.usage().unwrap().resident < held.resident);
}

#[test]
fn io_preparation_owns_real_source_and_actual_generic_header_charges_without_profile_parse() {
    struct Budget {
        quota: BackingQuota,
        last: Arc<AtomicU64>,
    }
    impl PreparationBudget for Budget {
        type Lease = QuotaLease;
        fn reserve_prepared_bytes(&mut self, bytes: u64) -> Result<QuotaLease, CompositionError> {
            self.last.store(bytes, Ordering::Relaxed);
            self.quota.reserve(QuotaKind::Resident, bytes).map_err(|_| photocraft_raster::RasterInputError::LimitExceeded.into())
        }
        fn consume_preparation_work(&mut self, _: u64) -> Result<(), CompositionError> {
            Ok(())
        }
    }
    struct LargeSource {
        _inline: [u8; 8192],
        _source: QuotaLease,
    }
    let (doc, q, _guard, reads, _, _) = fixture(1, ColorMode::Rgb, SampleType::U16, SampleEncoding::Linear);
    let baseline = q.usage().unwrap();
    let source = LargeSource { _inline: [0; 8192], _source: q.reserve(QuotaKind::Resident, 8192).unwrap() };
    let last = Arc::new(AtomicU64::new(0));
    let mut budget = Budget { quota: q.clone(), last: last.clone() };
    let pipeline = PreparedStockPointPipeline::new_budgeted(&doc, &Interrupt::NONE, &mut budget, source).unwrap();
    assert!(last.load(Ordering::Relaxed) > std::mem::size_of::<LeasedStockPointPipeline<LargeSource, QuotaLease>>() as u64);
    assert!(q.usage().unwrap().resident > baseline.resident + 8192);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    drop(pipeline);
    assert_eq!(q.usage().unwrap(), baseline);
}
