//! UNRUN L2 contracts only. Source/paint Surfaces, expected raw arrays and
//! reference coverage vectors are external harness, not document authority.
use super::*;
use photocraft_color::{ColorMode, try_read_sample_f64, try_write_sample_f64};
use photocraft_raster::{RasterInputError, backing::quota::QuotaLimits};
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
fn source_values(mode: ColorMode, alpha: bool) -> Vec<f64> {
    let mut v = if mode == ColorMode::Rgb { vec![0.2 + 2f64.powi(-43), 0.5 + 2f64.powi(-40), 0.7 + 2f64.powi(-41)] } else { vec![0.5 + 2f64.powi(-40)] };
    if alpha {
        v.push(0.75 + 2f64.powi(-42));
    }
    v
}
fn paint_values(mode: ColorMode, alpha: bool) -> Vec<f64> {
    let mut v = if mode == ColorMode::Rgb { vec![0.9 + 2f64.powi(-42), 0.1 + 2f64.powi(-41), 0.3 + 2f64.powi(-43)] } else { vec![0.3 + 2f64.powi(-43)] };
    if alpha {
        v.push(0.4 + 2f64.powi(-42));
    }
    v
}
#[test]
#[ignore = "UNRUN exact old wide scalar association, U32 last quantization/F64 lowbits, raw alpha locks, zero/sourcealpha shortcuts and Preserve/ClearAtZero atomicity"]
fn pure_wide_pixels_preserve_scalar_order_and_raw_shortcuts() {
    for sample in [SampleType::U32, SampleType::F64] {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for lock in [false, true] {
                let f = PixelFormat::new(mode, sample, true);
                let original = raw(f, &source_values(mode, true));
                let src = paint_values(mode, true);
                let a = mode.color_channels();
                let da = try_read_sample_f64(&original, sample, a).unwrap();
                let sa = src[a] * 0.375;
                let oa = sa + da * (1. - sa);
                let mut expected = original.clone();
                for (ch, source_color) in src.iter().copied().enumerate().take(a) {
                    let d = try_read_sample_f64(&original, sample, ch).unwrap();
                    let value = (source_color * sa + d * da * (1. - sa)) / oa;
                    try_write_sample_f64(&mut expected, sample, ch, value).unwrap();
                }
                if !lock {
                    try_write_sample_f64(&mut expected, sample, a, oa).unwrap();
                }
                let mut got = original.clone();
                assert_eq!(crate::wide::try_normal_native_pixel(&mut got, f, &src, 0.375, lock).unwrap(), got != original);
                assert_eq!(got, expected);
                if lock {
                    assert_eq!(&got[a * sample.bytes()..], &original[a * sample.bytes()..]);
                }
            }
        }
    }
    let f = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let original = raw(f, &[f64::from_bits(0x7ff8000000000042), -0., 0.5, -0.]);
    let mut got = original.clone();
    assert!(!crate::wide::try_normal_native_pixel(&mut got, f, &[0.1, 0.2, 0.3, 0.], 1., false).unwrap());
    assert_eq!(got, original);
    assert!(!crate::wide::try_normal_native_pixel(&mut got, f, &[0.1, 0.2, 0.3, 1.], 1., true).unwrap());
    assert_eq!(got, original);
    let locked = raw(f, &[f64::from_bits(0x7ff8000000000042), -0., 0.5, f64::NAN]);
    let mut got = locked.clone();
    assert!(!crate::wide_erase::try_erase_native_pixel(&mut got, f, 1., true, crate::wide_erase::EraseColorPolicy::Preserve).unwrap());
    assert_eq!(got, locked);
    let mut preserved = raw(f, &[f64::from_bits(0x7ff8000000000042), -0., 0.5, 0.75]);
    let colors = preserved[..24].to_vec();
    crate::wide_erase::try_erase_native_pixel(&mut preserved, f, 1., false, crate::wide_erase::EraseColorPolicy::Preserve).unwrap();
    assert_eq!(&preserved[..24], colors.as_slice());
    assert_eq!(try_read_sample_f64(&preserved, SampleType::F64, 3).unwrap().to_bits(), 0f64.to_bits());
    crate::wide_erase::try_erase_native_pixel(&mut preserved, f, 1., false, crate::wide_erase::EraseColorPolicy::ClearAtZero).unwrap();
    assert!(preserved.iter().all(|v| *v == 0));
    let mut invalid = raw(f, &[0.2, 0.3, 0.4, 1.5]);
    let before = invalid.clone();
    assert_eq!(crate::wide::try_normal_native_pixel(&mut invalid, f, &[0.1, 0.2, 0.3, 1.], 0.5, false), Err(RasterInputError::InvalidSample));
    assert_eq!(invalid, before);
    let hdr_color = [-0.125 - 2f64.powi(-42), 2.0 + 2f64.powi(-40), 3.0, 1.0];
    assert!(NativeWideColor::new(f, &hdr_color).is_ok());
    let mut hdr = raw(f, &[0.2, 0.3, 0.4, 1.]);
    crate::wide::try_normal_native_pixel(&mut hdr, f, &hdr_color, 1., false).unwrap();
    assert_eq!(hdr, raw(f, &hdr_color)); // No native F64 HDR clamp or color narrowing.
    let f32_format = PixelFormat::RGBA32F;
    let mut overflow = raw(f32_format, &[0.2, 0.3, 0.4, 1.]);
    let overflow_before = overflow.clone();
    assert!(crate::wide::try_normal_native_pixel(&mut overflow, f32_format, &[0.5, f64::MAX, 0.25, 1.], 1., false).is_err());
    assert_eq!(overflow, overflow_before); // Earlier local writes never publish.
    let f = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, true);
    let mut pixel = raw(f, &[0.5, 1.]);
    let color = [f64::from(0x80000001u32) / f64::from(u32::MAX), 1.];
    crate::wide::try_normal_native_pixel(&mut pixel, f, &color, 1., false).unwrap();
    assert_eq!(u32::from_ne_bytes(pixel[..4].try_into().unwrap()), 0x80000001);
}
#[test]
#[ignore = "UNRUN borrowed U32/F64 RGB/Gray full F32 coverage schedule, lowbit native paint, clipping/tilecrossing, normal/erase/pencil/rawlock and noalpha compatibility against existing wide Surface staging"]
fn borrowed_wide_stroke_matches_existing_wide_surface_bytes() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U32, SampleType::F64] {
            for alpha in [false, true] {
                for erase in [false, true] {
                    for lock in [false, true] {
                        for aliased in [false, true] {
                            let f = PixelFormat::new(mode, sample, alpha);
                            let native = raw(f, &source_values(mode, alpha));
                            let source = Surface::try_with_default_bytes(f, &native).unwrap();
                            let before = source.clone();
                            let mut expected = source.clone();
                            let color = paint_values(mode, alpha);
                            let paint_format = PixelFormat::new(mode, SampleType::F64, alpha);
                            let paint = Surface::try_with_default_bytes(paint_format, &raw(paint_format, &color)).unwrap();
                            let bounds = Rect::new(-3, -2, 258, 260);
                            let brush = BrushSettings {
                                aliased,
                                size: 7.,
                                hardness: 0.6,
                                pressure_size: false,
                                opacity: 0.7,
                                flow: 0.8,
                                erase,
                                seed: 53,
                                ..Default::default()
                            };
                            let points = [StrokePoint::new(254.25, 255.5, 0.7), StrokePoint::new(256., 257., 1.)];
                            let q = quota();
                            let base = q.usage().unwrap();
                            let plan = prepare_clipped_native_wide_stroke(
                                CheckedRasterView::resident(&source, bounds).unwrap(),
                                bounds,
                                &brush,
                                &points,
                                NativeWideColor::new(f, &color).unwrap(),
                                lock,
                                1.,
                                NativeStrokeLimits::STANDARD,
                                &q,
                                &ctl(),
                            )
                            .unwrap();
                            let r = plan.bounds();
                            let mut coverage = Vec::new();
                            for y in r.y0..r.y1 {
                                for x in r.x0..r.x1 {
                                    let tx = x.div_euclid(COV_TILE);
                                    let ty = y.div_euclid(COV_TILE);
                                    let value =
                                        plan.cov.tiles.get(&(tx, ty)).map_or(0., |t| t.cov[((y - ty * COV_TILE) * COV_TILE + x - tx * COV_TILE) as usize]);
                                    let k = (plan.ctx.stroke_mask_wet(value, None, None, x, y) * plan.ctx.brush.opacity.clamp(0., 1.)).min(1.);
                                    coverage.push(f64::from(k));
                                }
                            }
                            if erase {
                                if alpha {
                                    crate::wide_erase::try_erase_coverage(
                                        &mut expected,
                                        r,
                                        &coverage,
                                        1.,
                                        None,
                                        lock,
                                        crate::wide_erase::EraseColorPolicy::Preserve,
                                        Default::default(),
                                        &Interrupt::NONE,
                                    )
                                    .unwrap();
                                }
                            } else {
                                crate::wide::try_apply_normal_coverage(&mut expected, &paint, r, &coverage, 1., None, lock, &Interrupt::NONE).unwrap();
                            }
                            let patch = plan.render(&ctl()).unwrap();
                            let bpp = f.bytes_per_pixel();
                            let mut old = vec![0; bpp];
                            let mut wanted = vec![0; bpp];
                            let mut any = false;
                            for y in r.y0..r.y1 {
                                for x in r.x0..r.x1 {
                                    let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * bpp;
                                    expected.try_read_pixel_bytes(x, y, &mut wanted).unwrap();
                                    source.try_read_pixel_bytes(x, y, &mut old).unwrap();
                                    assert_eq!(&patch.bytes()[at..at + bpp], wanted.as_slice());
                                    any |= old != wanted;
                                }
                            }
                            assert_eq!(patch.changed(), any);
                            assert_eq!(source, before);
                            drop(patch);
                            assert_eq!(q.usage().unwrap(), base);
                        }
                    }
                }
            }
        }
    }
}
#[test]
#[ignore = "UNRUN actual same-q prepare/render peak overlap, cumulative Work/cancel, invalid/native finite failures and locked Preserve raw acquisition; no publication/refund before payload drop"]
fn wide_native_failures_refund_original_quota_and_keep_source() {
    let f = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let source = Surface::try_with_default_bytes(f, &raw(f, &source_values(ColorMode::Rgb, true))).unwrap();
    let before = source.clone();
    let bounds = Rect::new(0, 0, 8, 8);
    let brush = BrushSettings { size: 3., hardness: 1., pressure_size: false, ..Default::default() };
    let points = [StrokePoint::new(2.5, 2.5, 1.)];
    let color = NativeWideColor::new(f, &paint_values(ColorMode::Rgb, true)).unwrap();
    let q = quota();
    let base = q.usage().unwrap();
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(
        prepare_clipped_native_wide_stroke(
            CheckedRasterView::resident(&source, bounds).unwrap(),
            bounds,
            &brush,
            &points,
            color,
            false,
            1.,
            NativeStrokeLimits::STANDARD,
            &q,
            &deny
        ),
        Err(NativeStrokeError::Backing(BackingError::Work))
    ));
    assert_eq!(q.usage().unwrap(), base);
    let total = AtomicU64::new(0);
    let plan = prepare_clipped_native_wide_stroke(
        CheckedRasterView::resident(&source, bounds).unwrap(),
        bounds,
        &brush,
        &points,
        color,
        false,
        1.,
        NativeStrokeLimits::STANDARD,
        &q,
        &ctl(),
    )
    .unwrap();
    drop(
        plan.render(&ReadControl {
            interrupt: &Interrupt::NONE,
            work: &|n| {
                total.fetch_add(n, Ordering::Relaxed);
                Ok(())
            },
        })
        .unwrap(),
    );
    assert_eq!(q.usage().unwrap(), base);
    let plan = prepare_clipped_native_wide_stroke(
        CheckedRasterView::resident(&source, bounds).unwrap(),
        bounds,
        &brush,
        &points,
        color,
        false,
        1.,
        NativeStrokeLimits::STANDARD,
        &q,
        &ctl(),
    )
    .unwrap();
    let used = AtomicU64::new(0);
    assert!(
        plan.render(&ReadControl {
            interrupt: &Interrupt::NONE,
            work: &|n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > total.load(Ordering::Relaxed) / 2 {
                Err(BackingError::Work)
            } else {
                Ok(())
            }
        })
        .is_err()
    );
    assert_eq!(q.usage().unwrap(), base);
    let plan = prepare_clipped_native_wide_stroke(
        CheckedRasterView::resident(&source, bounds).unwrap(),
        bounds,
        &brush,
        &points,
        color,
        false,
        1.,
        NativeStrokeLimits::STANDARD,
        &q,
        &ctl(),
    )
    .unwrap();
    let stop = || true;
    let i = Interrupt::cancel_only(&stop);
    assert!(plan.render(&ReadControl { interrupt: &i, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), base);
    let plan = prepare_clipped_native_wide_stroke(
        CheckedRasterView::resident(&source, bounds).unwrap(),
        bounds,
        &brush,
        &points,
        color,
        false,
        1.,
        NativeStrokeLimits::STANDARD,
        &q,
        &ctl(),
    )
    .unwrap();
    let held = q.usage().unwrap().resident;
    let filler = q.reserve(QuotaKind::Resident, (64 << 20) - held - 32).unwrap();
    let fill_bytes = q.usage().unwrap().resident - held;
    assert!(plan.render(&ctl()).is_err());
    assert_eq!(q.usage().unwrap().resident, fill_bytes);
    drop(filler);
    assert_eq!(q.usage().unwrap(), base);
    assert!(NativeWideColor::new(f, &[0., 0., 0., f64::NAN]).is_err());
    assert!(NativeWideColor::new(PixelFormat::RGBA32F, &[0., 0., 0., 1.]).is_err());
    assert_eq!(source, before);
    let bad = Surface::try_with_default_bytes(f, &raw(f, &[f64::NAN, 0., 0., f64::NAN])).unwrap();
    let plan = prepare_clipped_native_wide_stroke(
        CheckedRasterView::resident(&bad, bounds).unwrap(),
        bounds,
        &brush,
        &points,
        color,
        false,
        1.,
        NativeStrokeLimits::STANDARD,
        &q,
        &ctl(),
    )
    .unwrap();
    assert!(plan.render(&ctl()).is_err());
    assert_eq!(q.usage().unwrap(), base);
    let mut eraser = brush;
    eraser.erase = true;
    let plan = prepare_clipped_native_wide_stroke(
        CheckedRasterView::resident(&bad, bounds).unwrap(),
        bounds,
        &eraser,
        &points,
        color,
        true,
        1.,
        NativeStrokeLimits::STANDARD,
        &q,
        &ctl(),
    )
    .unwrap();
    let patch = plan.render(&ctl()).unwrap();
    assert!(!patch.changed());
    drop(patch);
    assert_eq!(q.usage().unwrap(), base);
}
