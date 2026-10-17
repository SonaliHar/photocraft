//! UNRUN lower arithmetic/API regressions. Buffers and the old full-horizontal
//! oracle are external test harness allocations, not production quota receipts.
use super::*;
use std::convert::Infallible;

fn pixel(x: i32, y: i32) -> [f64; 4] {
    [
        0.125 + f64::from(x.rem_euclid(13)) / 32.,
        0.0625 + f64::from(y.rem_euclid(11)) / 32.,
        0.25 + f64::from((x + y).rem_euclid(7)) / 16.,
        [0., 0.125, 0.5, 1.][(x - y).rem_euclid(4) as usize],
    ]
}
fn limits(plan: BoxBlurRowPlan) -> GaussianLimits {
    let r = plan.requirements(&Interrupt::NONE).unwrap();
    GaussianLimits { resident_bytes: r.peak_resident_bytes, work_units: r.work_units }
}
fn compute(plan: BoxBlurRowPlan) -> Vec<u8> {
    let mut scratch = vec![[0.; 4]; plan.source_row_width()];
    try_box_blur_row_tile(&plan, &mut scratch, limits(plan), &Interrupt::NONE, &mut |x, y, out| -> Result<(), Infallible> {
        for (i, p) in out.iter_mut().enumerate() {
            *p = pixel(x + i as i32, y);
        }
        Ok(())
    })
    .unwrap()
}
fn old_canonical(plan: BoxBlurRowPlan) -> Vec<u8> {
    let mut units = 0;
    super::super::tile_core(
        plan.tile_output(),
        plan.source_extent(),
        format(),
        4,
        plan.radius() as usize,
        plan.radius() as usize * 2 + 1,
        &mut units,
        &Interrupt::NONE,
        &mut |x, y, out| {
            out.copy_from_slice(&pixel(x, y));
            Ok(())
        },
    )
    .unwrap()
}

#[test]
#[ignore = "UNRUN: independent row scheduling exact words versus old full-O canonical tile arithmetic"]
fn positive_radii_retain_old_canonical_arithmetic_including_maximum() {
    for radius in [1, 4, 5, 31, 127, 1000] {
        let plan = BoxBlurRowPlan::new(Rect::new(-3, 5, 18, 22), TileCoord::new(0, 0), radius, &Interrupt::NONE).unwrap();
        let old = old_canonical(plan);
        let streamed = compute(plan);
        assert_eq!(streamed, old, "radius{radius}");
    }
    // Exact old canonical tile parts, rather than an arbitrary thin legacy ROI,
    // are the reference when negative/global256 boundaries are crossed.
    let source = Rect::new(251, -7, 264, 8);
    for tile in [TileCoord::new(0, -1), TileCoord::new(1, -1), TileCoord::new(0, 0), TileCoord::new(1, 0)] {
        let plan = BoxBlurRowPlan::new(source, tile, 5, &Interrupt::NONE).unwrap();
        assert_eq!(compute(plan), old_canonical(plan));
    }
}

#[test]
#[ignore = "UNRUN: maximum-radius logical planning retains only bounded rows and full tile"]
fn maximum_radius_plan_prices_actual_capacity_without_full_support_allocation() {
    for (source, row_pixels) in [(Rect::new(0, 0, 6000, 6000), 1256), (Rect::new(-3000, -2000, 3000, 2000), 2256)] {
        let plan = BoxBlurRowPlan::new(source, TileCoord::containing(0, 0), 1000, &Interrupt::NONE).unwrap();
        let r = plan.requirements(&Interrupt::NONE).unwrap();
        // The edge case clips half the logical support to the source extent;
        // the interior case reads the complete row.
        assert_eq!(r.row_pixels, row_pixels);
        assert_eq!(r.row_requests, 2511);
        assert_eq!(r.output_bytes, 256 * 256 * 32);
        assert_eq!(r.peak_resident_bytes, 2 * (3 * 256 * 32 + 256 * 256 * 32) + 4096);
        assert!(r.peak_resident_bytes + (r.row_pixels as u64) * 64 + 512 < 16 << 20);
        assert_eq!(plan.support().width(), 2256);
        assert_eq!(plan.support().height(), 2256);
        // Requirements are not a lease, and this group allocates no image or
        // full support. Actual provider/profile/band overlap is caller-owned.
    }
    for radius in [0, 1001, u32::MAX] {
        assert!(BoxBlurRowPlan::new(Rect::new(0, 0, 10, 10), TileCoord::new(0, 0), radius, &Interrupt::NONE).is_err());
    }
    assert!(BoxBlurRowPlan::new(Rect::new(0, 0, 10, 10), TileCoord::new(i32::MAX, 0), 1, &Interrupt::NONE).is_err());
}

#[test]
#[ignore = "UNRUN: callback sequence and repeated endpoint rows use original S, no cropped-ROI reset"]
fn exact_recomputation_requests_are_clamped_and_complete() {
    let source = Rect::new(-3, -4, 7, 9);
    let plan = BoxBlurRowPlan::new(source, TileCoord::new(-1, -1), 7, &Interrupt::NONE).unwrap();
    let mut scratch = vec![[0.; 4]; plan.source_row_width()];
    let mut calls = Vec::new();
    let raw = try_box_blur_row_tile(&plan, &mut scratch, limits(plan), &Interrupt::NONE, &mut |x, y, out| -> Result<(), Infallible> {
        calls.push((x, y, out.len()));
        for (i, p) in out.iter_mut().enumerate() {
            *p = pixel(x + i as i32, y);
        }
        Ok(())
    })
    .unwrap();
    let k = plan.radius() as usize * 2 + 1;
    let mut js: Vec<_> = (0..k).collect();
    for y in 0..plan.tile_output().height() as usize - 1 {
        js.extend([y, y + k]);
    }
    let expected: Vec<_> = js
        .into_iter()
        .map(|j| {
            let y = (i64::from(plan.tile_output().y0) + j as i64 - i64::from(plan.radius())).clamp(i64::from(source.y0), i64::from(source.y1) - 1) as i32;
            (plan.source_read().x0, y, plan.source_row_width())
        })
        .collect();
    assert_eq!(calls, expected);
    assert_eq!(calls.len() as u64, plan.row_requests().unwrap());
    assert_eq!(raw, old_canonical(plan));
}

#[derive(Debug, PartialEq, Eq)]
struct OriginalReadFailure {
    phase: u64,
}

#[test]
#[ignore = "UNRUN: source errors remain typed; partial callback fills and insufficient work fail closed"]
fn original_source_failure_and_partial_fill_are_not_retried() {
    let plan = BoxBlurRowPlan::new(Rect::new(-2, -2, 12, 13), TileCoord::new(0, 0), 1, &Interrupt::NONE).unwrap();
    let mut scratch = vec![[0.; 4]; plan.source_row_width()];
    let mut calls = 0;
    let fail_at = 5; // Initial K=3, removal then first addition: private output exists.
    let result = try_box_blur_row_tile(&plan, &mut scratch, limits(plan), &Interrupt::NONE, &mut |x, y, out| {
        calls += 1;
        if calls == fail_at {
            return Err(OriginalReadFailure { phase: 987654321 });
        }
        for (i, p) in out.iter_mut().enumerate() {
            *p = pixel(x + i as i32, y);
        }
        Ok(())
    });
    assert!(matches!(result, Err(BoxBlurRowError::Source(OriginalReadFailure { phase: 987654321 }))));
    assert_eq!(calls, fail_at);
    let partial = try_box_blur_row_tile(&plan, &mut scratch, limits(plan), &Interrupt::NONE, &mut |_, _, out| -> Result<(), Infallible> {
        out[0] = [0.25, 0.5, 0.75, 0.5];
        Ok(())
    });
    assert!(matches!(partial, Err(BoxBlurRowError::Kernel(RasterInputError::InvalidSample))));
    let r = plan.requirements(&Interrupt::NONE).unwrap();
    for low in [
        GaussianLimits { resident_bytes: r.peak_resident_bytes - 1, work_units: r.work_units },
        GaussianLimits { resident_bytes: r.peak_resident_bytes, work_units: r.work_units - 1 },
    ] {
        let mut called = false;
        let result = try_box_blur_row_tile(&plan, &mut scratch, low, &Interrupt::NONE, &mut |_, _, _| -> Result<(), Infallible> {
            called = true;
            Ok(())
        });
        assert!(matches!(result, Err(BoxBlurRowError::Kernel(RasterInputError::LimitExceeded))));
        assert!(!called);
    }
}

#[test]
#[ignore = "UNRUN: late cancellation and finite-input validation abandon private rows/output"]
fn late_cancellation_and_invalid_row_do_not_return_output() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let plan = BoxBlurRowPlan::new(Rect::new(-2, -2, 12, 13), TileCoord::new(0, 0), 1, &Interrupt::NONE).unwrap();
    let cancelled = AtomicBool::new(false);
    let cancel = || cancelled.load(Ordering::Relaxed);
    let interrupt = Interrupt::cancel_only(&cancel);
    let mut scratch = vec![[0.; 4]; plan.source_row_width()];
    let mut calls = 0;
    let result = try_box_blur_row_tile(&plan, &mut scratch, limits(plan), &interrupt, &mut |x, y, out| -> Result<(), Infallible> {
        calls += 1;
        for (i, p) in out.iter_mut().enumerate() {
            *p = pixel(x + i as i32, y);
        }
        if calls == 4 {
            cancelled.store(true, Ordering::Relaxed);
        }
        Ok(())
    });
    assert!(matches!(result, Err(BoxBlurRowError::Kernel(RasterInputError::Cancelled))));
    assert_eq!(calls, 4);
    for invalid in [[f64::INFINITY, 0., 0., 1.], [0., 0., 0., -0.01], [0., 0., 0., 1.01]] {
        let result = try_box_blur_row_tile(&plan, &mut scratch, limits(plan), &Interrupt::NONE, &mut |_, _, out| -> Result<(), Infallible> {
            out.fill(invalid);
            Ok(())
        });
        assert!(matches!(result, Err(BoxBlurRowError::Kernel(RasterInputError::InvalidSample))));
    }
}
