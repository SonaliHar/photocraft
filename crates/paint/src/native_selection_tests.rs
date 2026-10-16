//! Deferred selection contracts. Surfaces, native inputs and expected buffers
//! are external harness allocations, never document/source quota receipts.
use super::*;
use photocraft_color::{ColorMode, try_read_sample_f64, try_write_sample_f64};
use photocraft_raster::{Interrupt, backing::quota::QuotaLimits};
use std::sync::atomic::{AtomicU64, Ordering};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 0, scratch: 0 })
}
fn raw(f: PixelFormat, values: &[f64]) -> Vec<u8> {
    let mut b = vec![0; f.bytes_per_pixel()];
    for (i, v) in values.iter().enumerate() {
        try_write_sample_f64(&mut b, f.sample, i, *v).unwrap();
    }
    b
}
fn brush(erase: bool) -> BrushSettings {
    BrushSettings {
        size: 5.,
        hardness: 0.6,
        pressure_size: false,
        opacity: 0.7,
        flow: 0.8,
        erase,
        seed: 53,
        color: [0.8, 0.2, 0.1, 0.75],
        ..Default::default()
    }
}
fn points() -> [StrokePoint; 2] {
    [StrokePoint::new(255.25, 255.5, 0.7), StrokePoint::new(256.25, 256.75, 1.)]
}
fn bounds() -> Rect {
    Rect::new(254, 254, 258, 258)
}
fn weight(sample: SampleType) -> f64 {
    match sample {
        SampleType::U32 => f64::from(0x80000001u32) / f64::from(u32::MAX),
        SampleType::F64 => 0.5 + 2f64.powi(-42),
        _ => 0.375,
    }
}
fn mask(sample: SampleType) -> Surface {
    let f = PixelFormat::new(ColorMode::Grayscale, sample, false);
    let mut s = Surface::try_with_default_bytes(f, &raw(f, &[weight(sample)])).unwrap();
    s.try_write_pixel_f64(255, 255, &[0.]).unwrap();
    s.try_write_pixel_f64(256, 256, &[1.]).unwrap();
    s
}
#[test]
#[ignore = "UNRUN/uncompiled: five native selection depths, nonzero sparse default/0/1/tileboundary, narrow upstream association and wide manual f64 source-over/erase after original F32base; exact wide lowbits, lock/None/white equivalence. No Source or producer claim."]
fn selected_weights_preserve_native_precision_and_original_association() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for erase in [false, true] {
                for lock in [false, true] {
                    let f = PixelFormat::new(mode, sample, true);
                    let n = f.channels();
                    let mut value = vec![0.25; n];
                    *value.last_mut().unwrap() = 0.75;
                    let source = Surface::try_with_default_bytes(f, &raw(f, &value)).unwrap();
                    let original = source.clone();
                    let selected = mask(sample);
                    let q = quota();
                    let baseline = q.usage().unwrap();
                    let b = brush(erase);
                    let pts = points();
                    let view = CheckedRasterView::resident_with_control(&source, bounds(), &ctl()).unwrap();
                    let checked = CheckedNativeSelection::new(&selected, view, &q, 64 << 20, &ctl()).unwrap();
                    assert_eq!(checked.wide(254, 254, &ctl()).unwrap().to_bits(), try_read_sample_f64(selected.default_bytes(), sample, 0).unwrap().to_bits());
                    if matches!(sample, SampleType::U32 | SampleType::F64) {
                        assert_ne!(checked.wide(254, 254, &ctl()).unwrap(), f64::from(weight(sample) as f32));
                    }
                    drop(checked);
                    let patch = if matches!(sample, SampleType::U32 | SampleType::F64) {
                        let mut paint = vec![0.8; n];
                        if mode == ColorMode::Rgb {
                            paint[1] = 0.2;
                            paint[2] = 0.1;
                        }
                        *paint.last_mut().unwrap() = 0.75 + 2f64.powi(-43);
                        let plan = wide_native::prepare_clipped_native_wide_stroke_selected(
                            view,
                            bounds(),
                            &b,
                            &pts,
                            wide_native::NativeWideColor::new(f, &paint).unwrap(),
                            &selected,
                            lock,
                            1.,
                            NativeStrokeLimits::STANDARD,
                            &q,
                            &ctl(),
                        )
                        .unwrap();
                        // A separately paid narrow geometry plan supplies only the unchanged F32
                        // geometric coverage; manual scalar math below never uses selected render.
                        let geometry_source = Surface::new(PixelFormat::RGBA32F);
                        let geometry = prepare_clipped_native_stroke(
                            CheckedRasterView::resident(&geometry_source, bounds()).unwrap(),
                            bounds(),
                            &b,
                            &pts,
                            None,
                            lock,
                            1.,
                            NativeStrokeLimits::STANDARD,
                            &q,
                            &ctl(),
                        )
                        .unwrap();
                        let patch = plan.render(&ctl()).unwrap();
                        let r = patch.bounds();
                        let mut expected = Vec::new();
                        for y in r.y0..r.y1 {
                            for x in r.x0..r.x1 {
                                let mut pixel = raw(f, &value);
                                let c = geometry.cov.get(x, y);
                                if c > 0. {
                                    let k = (geometry.ctx.stroke_mask_wet(c, None, None, x, y) * b.opacity.clamp(0., 1.)).min(1.);
                                    let mut s = [0.];
                                    selected.try_read_pixel_f64(x, y, &mut s).unwrap();
                                    let w = f64::from(k) * s[0];
                                    if w > 0. {
                                        let a = n - 1;
                                        let da = try_read_sample_f64(&pixel, sample, a).unwrap();
                                        if erase {
                                            if !lock {
                                                try_write_sample_f64(&mut pixel, sample, a, da * (1. - w)).unwrap();
                                            }
                                        } else {
                                            let sa = paint[a] * w;
                                            let oa = sa + da * (1. - sa);
                                            for ch in 0..a {
                                                let d = try_read_sample_f64(&pixel, sample, ch).unwrap();
                                                let v = (paint[ch] * sa + d * da * (1. - sa)) / oa;
                                                try_write_sample_f64(&mut pixel, sample, ch, v).unwrap();
                                            }
                                            if !lock {
                                                try_write_sample_f64(&mut pixel, sample, a, oa).unwrap();
                                            }
                                        }
                                    }
                                }
                                expected.extend_from_slice(&pixel);
                            }
                        }
                        assert_eq!(patch.bytes(), expected);
                        drop(geometry);
                        patch
                    } else {
                        let mut expected = source.clone();
                        crate::render_stroke_with_interrupt(
                            &mut expected,
                            &b,
                            &pts,
                            Some(&selected),
                            lock,
                            1.,
                            crate::StrokeLimits::default(),
                            &Interrupt::NONE,
                        )
                        .unwrap();
                        let patch =
                            prepare_clipped_native_stroke_selected(view, bounds(), &b, &pts, &selected, lock, 1., NativeStrokeLimits::STANDARD, &q, &ctl())
                                .unwrap()
                                .render(&ctl())
                                .unwrap();
                        assert_eq!(patch.bytes(), expected.try_read_interleaved_with(patch.bounds(), &Interrupt::NONE).unwrap());
                        patch
                    };
                    drop(patch);
                    assert_eq!(source, original);
                    assert_eq!(q.usage().unwrap(), baseline);
                }
            }
        }
    }
    // Endpoint masks: selected0 is raw passthrough, selected1 is exact None.
    let f = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let source = Surface::try_with_default_bytes(f, &raw(f, &[0.2, 0.3, 0.4, 0.75])).unwrap();
    let q = quota();
    let b = brush(false);
    let pts = points();
    let color = wide_native::NativeWideColor::new(f, &[0.8, 0.2, 0.1, 0.75]).unwrap();
    let plain = wide_native::prepare_clipped_native_wide_stroke(
        CheckedRasterView::resident(&source, bounds()).unwrap(),
        bounds(),
        &b,
        &pts,
        color,
        false,
        1.,
        NativeStrokeLimits::STANDARD,
        &q,
        &ctl(),
    )
    .unwrap()
    .render(&ctl())
    .unwrap();
    for v in [-0., 0., 1.] {
        let s = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &[v]);
        let p = wide_native::prepare_clipped_native_wide_stroke_selected(
            CheckedRasterView::resident(&source, bounds()).unwrap(),
            bounds(),
            &b,
            &pts,
            color,
            &s,
            false,
            1.,
            NativeStrokeLimits::STANDARD,
            &q,
            &ctl(),
        )
        .unwrap()
        .render(&ctl())
        .unwrap();
        if v == 1. {
            assert_eq!(p.bytes(), plain.bytes());
        } else {
            assert!(!p.changed());
            assert_eq!(p.bytes(), source.try_read_interleaved(p.bounds()).unwrap());
        }
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: complete physical/default selection refusal incl out-of-coverage NaN, wrong shape; prospective combined resident cap, independent runtime q failure, late original Work/cancel atomic cleanup. No allowance/Source claim."]
fn selection_proof_and_allocation_failures_are_atomic() {
    let q = quota();
    let source = Surface::new(PixelFormat::RGBA32F);
    let b = brush(false);
    let pts = points();
    let good = mask(SampleType::F32);
    let baseline = q.usage().unwrap();
    let original = source.clone();
    for fault in 0..4 {
        let mut selected = good.clone();
        match fault {
            0 => selected = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[f32::NAN]),
            1 => selected = Surface::new(PixelFormat::RGBA8),
            2 => selected.try_write_interleaved(Rect::new(0, 0, 1, 1), &f32::NAN.to_ne_bytes()).unwrap(),
            _ => selected = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[1.5]),
        };
        assert!(
            prepare_clipped_native_stroke_selected(
                CheckedRasterView::resident(&source, bounds()).unwrap(),
                bounds(),
                &b,
                &pts,
                &selected,
                false,
                1.,
                NativeStrokeLimits::STANDARD,
                &q,
                &ctl()
            )
            .is_err()
        );
        assert_eq!(q.usage().unwrap(), baseline);
    }
    let plain = prepare_clipped_native_stroke(
        CheckedRasterView::resident(&source, bounds()).unwrap(),
        bounds(),
        &b,
        &pts,
        None,
        false,
        1.,
        NativeStrokeLimits::STANDARD,
        &q,
        &ctl(),
    )
    .unwrap();
    let minimum = plain.prepared_bytes;
    drop(plain);
    let selected = CheckedNativeSelection::new(&good, CheckedRasterView::resident(&source, bounds()).unwrap(), &q, 64 << 20, &ctl()).unwrap();
    let fee = selected.retained_bytes();
    drop(selected);
    let lowered = NativeStrokeLimits::try_lowered(minimum + fee - 1, NativeStrokeLimits::STANDARD.work, 16 << 20).unwrap();
    assert!(
        prepare_clipped_native_stroke_selected(
            CheckedRasterView::resident(&source, bounds()).unwrap(),
            bounds(),
            &b,
            &pts,
            &good,
            false,
            1.,
            lowered,
            &q,
            &ctl()
        )
        .is_err()
    );
    assert_eq!(q.usage().unwrap(), baseline);
    let plan = prepare_clipped_native_stroke_selected(
        CheckedRasterView::resident(&source, bounds()).unwrap(),
        bounds(),
        &b,
        &pts,
        &good,
        false,
        1.,
        NativeStrokeLimits::STANDARD,
        &q,
        &ctl(),
    )
    .unwrap();
    let held = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - held.resident - 32).unwrap();
    assert!(matches!(plan.render(&ctl()), Err(NativeStrokeError::Backing(BackingError::Quota))));
    drop(blocker);
    assert_eq!(q.usage().unwrap(), baseline);
    let total = AtomicU64::new(0);
    let count = |n| {
        total.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &count };
    let p = prepare_clipped_native_stroke_selected(
        CheckedRasterView::resident(&source, bounds()).unwrap(),
        bounds(),
        &b,
        &pts,
        &good,
        false,
        1.,
        NativeStrokeLimits::STANDARD,
        &q,
        &ctl(),
    )
    .unwrap();
    drop(p.render(&c).unwrap());
    let used = AtomicU64::new(0);
    let cutoff = total.load(Ordering::Relaxed) / 2;
    let deny = |n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > cutoff { Err(BackingError::Work) } else { Ok(()) };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
    let p = prepare_clipped_native_stroke_selected(
        CheckedRasterView::resident(&source, bounds()).unwrap(),
        bounds(),
        &b,
        &pts,
        &good,
        false,
        1.,
        NativeStrokeLimits::STANDARD,
        &q,
        &ctl(),
    )
    .unwrap();
    assert!(matches!(p.render(&c), Err(NativeStrokeError::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let yes = || true;
    let interrupt = Interrupt::cancel_only(&yes);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(
        prepare_clipped_native_stroke_selected(
            CheckedRasterView::resident(&source, bounds()).unwrap(),
            bounds(),
            &b,
            &pts,
            &good,
            false,
            1.,
            NativeStrokeLimits::STANDARD,
            &q,
            &c
        ),
        Err(NativeStrokeError::Backing(BackingError::Cancelled))
    ));
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(source, original);
}
