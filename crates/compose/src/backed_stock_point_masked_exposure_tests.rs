//! Deferred synthetic CURRENT proof using an actual prepaid native provider;
//! no original-source correspondence or producer numerical oracle is claimed.
use super::*;
use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument};
fn ctl() -> ReadControl<'static> {
    tests::ctl()
}
fn configured(sample: SampleType) -> (Document, BackingQuota, std::sync::Arc<dyn std::any::Any + Send + Sync>, Vec<u8>) {
    let (mut d, q, guard, _reads, _basis, raw) = tests::fixture(2, ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
    let state = d.xcf_state.as_mut().unwrap();
    state.provenance.format_version = 22;
    state.current_filter_output_precision = photocraft_doc::FilterOutputPrecision::Canonical;
    state.items[0].composition.as_mut().unwrap().compatibility_epoch = "xcf-22-normal".into();
    let node = &mut state.filter_stacks[0].nodes[0];
    node.operation.schema_epoch = crate::gimp_masked_exposure::EPOCH.into();
    node.composition.as_mut().unwrap().compatibility_epoch = crate::gimp_masked_exposure::EPOCH.into();
    node.arguments = vec![
        FilterArgument { name: "exposure".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits())) },
        FilterArgument { name: "black-level".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.0625f32.to_bits())) },
    ];
    let surface = &mut state.filter_masks[0].surface;
    // Fixture's preheld2MiB covers model/map growth and the physical scalar tile.
    surface.try_write_pixel_f64(0, 0, &[0.375]).unwrap();
    surface.try_write_pixel_f64(7, 5, &[if sample == SampleType::F64 { f64::from_bits(0x3ff0000009b25010) } else { 1. }]).unwrap();
    (d, q, guard, raw)
}
#[test]
#[ignore = "UNRUN actual prepaid native CURRENT provider, independently retained scalar payload afterdocdrop, bothdepths/fullphysical inactivepoison/nonempty/strictparameters/workcancel guards; no Source acceptance"]
fn mask_lifetime_and_full_inactive_proof() {
    for sample in [SampleType::F32, SampleType::F64] {
        let (d, q, guard, raw) = configured(sample);
        let stable = q.usage().unwrap();
        let format = d.pixel_format();
        let pipe = PreparedStockPointPipeline::prepare(&d, Builtin::Srgb.profile(), &q, &ctl()).unwrap();
        let mask_before = d.xcf_state.as_ref().unwrap().filter_masks[0].surface.clone(); // externalexpected, fixturepreheld
        drop(d);
        drop(guard);
        let result = pipe.evaluate_native_at(&raw[..format.bytes_per_pixel()], format, (0, 0), &ctl()).unwrap();
        assert!(result.iter().all(|v| v.is_finite()));
        let StockPointProgram::MaskedExposure { mask, .. } = &pipe.plan.stack else { panic!("maskedprogram") };
        assert_eq!(mask, &mask_before);
        drop(mask_before); // External expected map must not imply the plan retains its receipt.
        drop(pipe);
        assert!(q.usage().unwrap().resident <= stable.resident);
        for poison in 0..5 {
            let (mut d, q, _guard, _raw) = configured(sample);
            d.layers[0].visible = false;
            let state = d.xcf_state.as_mut().unwrap();
            state.items[0].visible = false;
            state.filter_stacks[0].nodes[0].enabled = false;
            match poison {
                0 => state.filter_masks[0].surface.try_write_pixel_f64(255, 255, &[-0.]).unwrap(),
                1 => state.filter_masks[0].surface.try_write_pixel_f64(0, 0, &[f64::NAN]).unwrap(),
                2 => state.filter_masks[0].surface.try_write_pixel_f64(0, 0, &[-0.1]).unwrap(),
                3 => state.filter_masks[0].surface.try_write_pixel_f64(0, 0, &[1.001]).unwrap(),
                _ => state.filter_masks[0].surface = photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false)),
            }
            let before = q.usage().unwrap();
            assert!(PreparedStockPointPipeline::prepare(&d, Builtin::Srgb.profile(), &q, &ctl()).is_err());
            assert_eq!(q.usage().unwrap(), before);
        }
        let (d, q, _guard, _raw) = configured(sample);
        let deny = |_| Err(BackingError::Work);
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
        assert!(PreparedStockPointPipeline::prepare(&d, Builtin::Srgb.profile(), &q, &c).is_err());
        let cancel = || true;
        let i = Interrupt::cancel_only(&cancel);
        let c = ReadControl { interrupt: &i, work: &|_| Ok(()) };
        assert!(PreparedStockPointPipeline::prepare(&d, Builtin::Srgb.profile(), &q, &c).is_err());
    }
}
