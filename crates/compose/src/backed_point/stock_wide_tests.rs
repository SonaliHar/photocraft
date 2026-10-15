//! UNRUN sealed StockWide input/custody tests. Synthetic CURRENT metadata is
//! not independently authored source evidence or a default renderer grant.
use super::*;
use photocraft_doc::{
    ColorMode, LayerContent, SampleEncoding, SampleType,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
};
use photocraft_raster::Interrupt;
use std::sync::atomic::{AtomicU64, Ordering};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
// Reuse a genuinely prepaid native source/model fixture, then change ONLY its
// current operation under the already held conservative model fee. No source
// epoch, original bytes or lineage assertion is manufactured by this fixture.
fn fixture(
    sample: SampleType,
) -> (
    photocraft_doc::Document,
    photocraft_raster::backing::quota::BackingQuota,
    std::sync::Arc<dyn std::any::Any + Send + Sync>,
    std::sync::Arc<AtomicU64>,
    Vec<u8>,
    PreparedStockPointPipeline,
) {
    let (mut doc, q, guard, reads, profile, raw) = crate::backed_threshold_context::tests::fixture(ColorMode::Rgb, sample, SampleEncoding::Linear);
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation.name = "gegl:exposure".into();
    node.operation.stored_version = "1:0".into();
    node.operation.schema_epoch = crate::gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH.into();
    node.composition.as_mut().unwrap().compatibility_epoch = crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into();
    node.arguments = vec![
        FilterArgument { name: "exposure".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.25f32.to_bits())) },
        FilterArgument { name: "black-level".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0f32.to_bits())) },
    ];
    let pipeline = PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).unwrap();
    (doc, q, guard, reads, raw, pipeline)
}
fn layer(doc: &photocraft_doc::Document) -> &BackedLayer {
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!() };
    layer
}
#[test]
fn wide_u16_f32_f64_output_matches_pipeline_without_native_intermediate_rounding() {
    for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
        let (doc, q, _guard, _reads, raw, pipeline) = fixture(sample);
        let source = layer(&doc);
        let rect = Rect::new(1, 1, 7, 5);
        let before = q.usage().unwrap();
        let band = render_stock_wide_band(source, source.descriptor(), Default::default(), &pipeline, rect, &ctl()).unwrap();
        assert_eq!(band.rect(), rect);
        assert_eq!(band.pixels().len(), 24);
        assert!(q.usage().unwrap().resident > before.resident);
        // Expected values come from the existing paid semantic stage, not native
        // quantization. Charge test-owned oracle storage before its allocation.
        let _oracle = q.reserve(QuotaKind::Resident, 4096).unwrap();
        let mut expected = Vec::new();
        let bpp = source.format().bytes_per_pixel();
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let at = (y as usize * 8 + x as usize) * bpp;
                expected.push(pipeline.evaluate_native(&raw[at..at + bpp], source.format(), &ctl()).unwrap());
            }
        }
        assert_eq!(band.pixels(), expected);
        let fractional = band.pixels().iter().find(|pixel| pixel[0] > 0.).unwrap()[0];
        if sample == SampleType::U16 {
            assert_ne!((fractional * 65535.).round() / 65535., fractional);
        }
        if sample == SampleType::F32 {
            assert_ne!(f64::from(fractional as f32), fractional);
        }
        let a = render_stock_wide_band(source, source.descriptor(), Default::default(), &pipeline, Rect::new(1, 1, 7, 3), &ctl()).unwrap();
        let b = render_stock_wide_band(source, source.descriptor(), Default::default(), &pipeline, Rect::new(1, 3, 7, 5), &ctl()).unwrap();
        assert_eq!([a.pixels(), b.pixels()].concat(), band.pixels());
    }
}
#[test]
fn exact_descriptor_same_quota_and_lower_roi_limits_precede_native_reads() {
    let (doc, q, _guard, reads, _raw, pipeline) = fixture(SampleType::F64);
    let source = layer(&doc);
    let before = reads.load(Ordering::Relaxed);
    let wrong = NativePlaneDescriptor::new(source.format(), Rect::new(0, 0, 1, 1), source.default_bytes()).unwrap();
    assert!(render_stock_wide_band(source, wrong, Default::default(), &pipeline, Rect::new(0, 0, 1, 1), &ctl()).is_err());
    for rect in [Rect::EMPTY, Rect::new(-1, 0, 0, 1), source.operative_bounds()] {
        assert!(render_stock_wide_band(source, source.descriptor(), NormalBandLimits { max_rows: 1, max_pixels: 1 }, &pipeline, rect, &ctl()).is_err());
    }
    let (_other, _oq, _og, _or, _ob, foreign) = fixture(SampleType::F64);
    assert!(render_stock_wide_band(source, source.descriptor(), Default::default(), &foreign, Rect::new(0, 0, 1, 1), &ctl()).is_err());
    assert_eq!(reads.load(Ordering::Relaxed), before);
    assert!(pipeline.shares_quota(&q));
}
#[test]
fn shared_control_work_cancel_quota_and_retained_wide_output_refund_privately() {
    let (doc, q, _guard, _reads, _raw, pipeline) = fixture(SampleType::F64);
    let source = layer(&doc);
    let rect = doc.bounds();
    drop(source.checked_view().pin_tile(TileCoord::new(0, 0), &ctl()).unwrap());
    let before = q.usage().unwrap();
    let deny = |_| Err(BackingError::Work);
    assert!(matches!(
        render_stock_wide_band(source, source.descriptor(), Default::default(), &pipeline, rect, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }),
        Err(BackedNormalError::Backing(BackingError::Work))
    ));
    assert_eq!(q.usage().unwrap(), before);
    let zeroes = AtomicU64::new(0);
    let late = |units| if units == 0 && zeroes.fetch_add(1, Ordering::Relaxed) > 24 { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(
        render_stock_wide_band(source, source.descriptor(), Default::default(), &pipeline, rect, &ReadControl { interrupt: &Interrupt::NONE, work: &late })
            .is_err()
    );
    assert_eq!(q.usage().unwrap(), before);
    let envelope = 48 * 2 * std::mem::size_of::<[f64; 4]>() + std::mem::size_of::<OwnedStockPointWideBand>() + 128;
    let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - before.resident - envelope as u64 + 1).unwrap();
    assert!(render_stock_wide_band(source, source.descriptor(), Default::default(), &pipeline, rect, &ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
    let output = render_stock_wide_band(source, source.descriptor(), Default::default(), &pipeline, rect, &ctl()).unwrap();
    assert!(q.usage().unwrap().resident > before.resident);
    drop(output);
    assert_eq!(q.usage().unwrap(), before);
}
#[test]
fn sealed_kernel_output_mismatch_fails_before_source_pins() {
    let (doc, _q, _guard, reads, _raw, pipeline) = fixture(SampleType::F64);
    let source = layer(&doc);
    let before = reads.load(Ordering::Relaxed);
    assert!(walk(source, doc.bounds(), KernelRef::Stock(&pipeline), OutputRef::Native(None), &ctl()).is_err());
    assert_eq!(reads.load(Ordering::Relaxed), before);
}
