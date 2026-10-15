//! UNRUN disabled CURRENT recipe drafts; no source or padding qualification.
use super::*;

#[test]
fn disabled_native_band_is_raw_identity_not_threshold_zero() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
            let bounds = Rect::new(-2, 255, 6, 257);
            let (original, _) = fixture(mode, sample, bounds, false);
            let mut words = vec![0; original.format().bytes_per_pixel()];
            for channel in 0..mode.color_channels() {
                try_write_sample_f64(&mut words, sample, channel, -0.0).unwrap();
            }
            try_write_sample_f64(&mut words, sample, mode.color_channels(), -0.0).unwrap();
            let layer = original.try_edited_native(Rect::new(0, 255, 1, 256), &words, &ctl()).unwrap();
            let disabled = PreparedBackedThresholdAlpha::new_with_enabled(
                &layer,
                layer.descriptor(),
                ThresholdAlpha { value: 0.0 },
                false,
                NormalBandLimits::default(),
                &ctl(),
            )
            .unwrap();
            let band = disabled.render_band(bounds, &ctl()).unwrap();
            for (index, pixel) in band.bytes().chunks_exact(layer.format().bytes_per_pixel()).enumerate() {
                let x = bounds.x0 + (index % bounds.width() as usize) as i32;
                let y = bounds.y0 + (index / bounds.width() as usize) as i32;
                let mut expected = vec![0; layer.format().bytes_per_pixel()];
                layer.checked_view().read_logical_pixel_native_into(x, y, &mut expected, &ctl()).unwrap();
                assert_eq!(pixel, expected);
            }
            // Original threshold-zero remains active and is observably different.
            let enabled = PreparedBackedThresholdAlpha::new(&layer, layer.descriptor(), ThresholdAlpha { value: 0.0 }, NormalBandLimits::default(), &ctl())
                .unwrap()
                .render_band(bounds, &ctl())
                .unwrap();
            assert_ne!(enabled.bytes(), band.bytes());
        }
    }
}

#[test]
fn disabled_still_rejects_invalid_native_samples_and_honors_control() {
    let bounds = Rect::new(0, 0, 8, 2);
    let (invalid, _) = fixture(ColorMode::Rgb, SampleType::F64, bounds, true);
    let prepared = PreparedBackedThresholdAlpha::new_with_enabled(
        &invalid,
        invalid.descriptor(),
        ThresholdAlpha { value: 0.5 },
        false,
        NormalBandLimits::default(),
        &ctl(),
    )
    .unwrap();
    assert!(prepared.render_band(bounds, &ctl()).is_err());
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Quota) };
    assert!(prepared.render_band(bounds, &denied).is_err());
    assert!(
        PreparedBackedThresholdAlpha::new_with_enabled(
            &invalid,
            invalid.descriptor(),
            ThresholdAlpha { value: f64::NAN },
            false,
            NormalBandLimits::default(),
            &ctl(),
        )
        .is_err()
    );
}
