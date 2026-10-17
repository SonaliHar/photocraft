//! TEMP checked-shape alpha contracts; uncompiled/unrun.
use super::*;
use photocraft_color::Color;
use photocraft_doc::{Fill, GradientStyle, LineCap, LineJoin, ShapeStroke, StrokeAlign};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};

fn shape() -> ShapeLayer {
    ShapeLayer {
        path: Path::new(vec![Subpath::polygon(&[(3.5, 4.25), (31.0, 7.0), (19.5, 30.5), (8.0, 23.0)])]),
        fill: Some(Fill::Solid(Color::rgba(0.2, 0.6, 0.8, 0.35))),
        ..Default::default()
    }
}
fn compare(s: &ShapeLayer) {
    let before = s.clone();
    let area = Rect::new(-2, -3, 40, 38);
    let work = AtomicU64::new(0);
    let mut debit = |n| {
        work.fetch_add(n, AtomicOrdering::Relaxed);
        Ok(())
    };
    let (checked, peak) =
        CheckedShapeAlpha::try_new_with_control(s, crate::DEFAULT_TOLERANCE, area, VectorPreparationLimits::default(), &Interrupt::NONE, &mut debit).unwrap();
    assert!(peak >= checked.checked_preparation_bytes().unwrap());
    let (actual, render_peak) = checked.try_render_with_control(area, VectorPreparationLimits::default(), &Interrupt::NONE, &mut debit).unwrap();
    assert!(render_peak >= checked.checked_preparation_bytes().unwrap() + actual.capacity() * 4);
    assert_eq!(checked.bounds(), crate::CompiledShape::new(s, crate::DEFAULT_TOLERANCE, area).bounds());
    assert_eq!(actual, crate::CompiledShape::new(s, crate::DEFAULT_TOLERANCE, area).render_rgba(area).into_iter().map(|p| p[3]).collect::<Vec<_>>());
    assert_eq!(*s, before);
    assert!(work.load(AtomicOrdering::Relaxed) > 0);
}
#[test]
fn checked_shapes_preserve_fill_stroke_alignment_dash_and_gradient_words() {
    let mut s = shape();
    compare(&s);
    // Alpha is independent of RGB conversion: all model colors preserve the
    // same literal Color.alpha and the exact legacy appearance alpha oracle.
    for mode in [
        photocraft_color::ColorMode::Rgb,
        photocraft_color::ColorMode::Grayscale,
        photocraft_color::ColorMode::Cmyk,
        photocraft_color::ColorMode::Lab,
        photocraft_color::ColorMode::Indexed,
        photocraft_color::ColorMode::Multichannel,
        photocraft_color::ColorMode::Bitmap,
        photocraft_color::ColorMode::Duotone,
    ] {
        s.fill = Some(Fill::Solid(Color { mode, c: [0.2, 0.4, 0.6, 0.1], alpha: 0.375 }));
        compare(&s);
    }
    s = shape();
    for align in [StrokeAlign::Center, StrokeAlign::Inside, StrokeAlign::Outside] {
        for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
            for join in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel] {
                s.stroke = Some(ShapeStroke {
                    width: 3.25,
                    paint: Fill::Solid(Color::rgba(0.7, 0.1, 0.4, 0.6)),
                    opacity: 0.65,
                    align,
                    cap,
                    join,
                    ..Default::default()
                });
                compare(&s);
            }
        }
    }
    for pattern in [vec![2.0, 1.0], vec![0.0, 2.0, 1.0], vec![2.0, 0.0, 1.0, 0.0]] {
        s.stroke = Some(ShapeStroke { width: 2.0, dashes: pattern, dash_offset: -1.25, cap: LineCap::Round, join: LineJoin::Round, ..Default::default() });
        compare(&s);
    }
    s.fill = Some(Fill::Gradient {
        stops: vec![(1.0, Color::rgba(0.1, 0.8, 0.4, 0.2)), (0.0, Color::rgba(0.8, 0.1, 0.4, 0.7)), (0.5, Color::WHITE), (0.5, Color::BLACK)],
        angle: 31.0,
        scale: 0.8,
        style: GradientStyle::Reflected,
        reverse: true,
        opacity_stops: vec![(1.0, 0.7), (0.0, 0.2)],
        midpoints: vec![0.2, 0.8],
        offset: (0.1, -0.05),
        dither: true,
        align: false,
    });
    compare(&s);
    s.path.inverted = true;
    compare(&s);
}
#[test]
fn checked_stroke_count_pass_matches_zero_odd_dash_seams_without_allocating_output() {
    let pl = crate::Polyline { pts: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], knot: vec![true; 4], closed: true };
    for pattern in [vec![8.0, 4.0], vec![0.0, 2.0, 1.0], vec![80.0, 2.0]] {
        let style = crate::StrokeStyle { width: 2.0, dashes: pattern, dash_offset: 2.0, cap: LineCap::Round, join: LineJoin::Round, ..Default::default() };
        let mut work = |_| Ok(());
        let mut ctl = ShapeBudget::new(VectorPreparationLimits::default(), &Interrupt::NONE, &mut work).unwrap();
        let actual = shape_stroke_controlled::polygons(std::slice::from_ref(&pl), &style, 0.01, &mut ctl).unwrap();
        assert_eq!(actual, crate::stroke_polygons(std::slice::from_ref(&pl), &style, 0.01));
    }
}
#[test]
fn checked_shape_original_work_and_inner_cancellation_refuse_unpublished_geometry() {
    let s = shape();
    let before = s.clone();
    // Constant alpha uses no gradient math: a one-sample lower-only work
    // boundary is independent of its canvas size and remains exactly typed.
    let mut debit = |_| Ok(());
    let mut budget = ShapeBudget::new(VectorPreparationLimits { max_work: 32, ..Default::default() }, &Interrupt::NONE, &mut debit).unwrap();
    assert_eq!(alpha_at(&AlphaPaint::Solid(0.375), 0.5, 0.5, &mut budget).unwrap().to_bits(), 0.375f32.to_bits());
    assert_eq!(budget.work, 32);
    let mut denied = ShapeBudget::new(VectorPreparationLimits { max_work: 31, ..Default::default() }, &Interrupt::NONE, &mut debit).unwrap();
    assert!(matches!(alpha_at(&AlphaPaint::Solid(0.375), 0.5, 0.5, &mut denied), Err(E::LimitExceeded)));
    let area = Rect::new(0, 0, 40, 40);
    let calls = AtomicUsize::new(0);
    let mut fail = |_| {
        if calls.fetch_add(1, AtomicOrdering::Relaxed) >= 12 { Err(E::LimitExceeded) } else { Ok(()) }
    };
    assert!(matches!(
        CheckedShapeAlpha::try_new_with_control(&s, 0.01, area, VectorPreparationLimits::default(), &Interrupt::NONE, &mut fail),
        Err(E::LimitExceeded)
    ));
    let mut dense = s.clone();
    dense.stroke = Some(ShapeStroke { width: 1.0, dashes: vec![1e-5, 1e-5], cap: LineCap::Round, ..Default::default() });
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, AtomicOrdering::Relaxed) >= 40;
    let mut work = |_| Ok(());
    assert!(matches!(
        CheckedShapeAlpha::try_new_with_control(&dense, 0.01, area, VectorPreparationLimits::default(), &Interrupt::cancel_only(&cancel), &mut work),
        Err(E::Cancelled)
    ));
    assert_eq!(s, before);
    let mut invalid = s.clone();
    invalid.stroke = Some(ShapeStroke { width: 2.0, dashes: vec![0.0, 0.0], ..Default::default() });
    let invalid_before = invalid.clone();
    assert!(matches!(
        CheckedShapeAlpha::try_new_with_control(&invalid, 0.01, area, VectorPreparationLimits::default(), &Interrupt::NONE, &mut work),
        Err(E::InvalidSample)
    ));
    assert_eq!(invalid, invalid_before);
    for invalid_width in [true, false] {
        let raw = f32::from_bits(0x7fc0_0017);
        let mut malformed = s.clone();
        malformed.stroke =
            Some(ShapeStroke { width: if invalid_width { raw } else { 2.0 }, opacity: if invalid_width { 1.0 } else { raw }, ..Default::default() });
        let path_before = malformed.path.clone();
        assert!(matches!(
            CheckedShapeAlpha::try_new_with_control(&malformed, 0.01, area, VectorPreparationLimits::default(), &Interrupt::NONE, &mut work),
            Err(E::InvalidSample)
        ));
        let stroke = malformed.stroke.as_ref().unwrap();
        assert_eq!(if invalid_width { stroke.width.to_bits() } else { stroke.opacity.to_bits() }, raw.to_bits());
        assert_eq!(malformed.path, path_before);
    }
    // Malformed stored fields are refused without normalizing their raw bits.
    for (angle, offset) in [(f32::from_bits(0x7fc0_0019), (0.0, 0.0)), (0.0, (f32::INFINITY, 0.0))] {
        let mut malformed = s.clone();
        malformed.fill = Some(Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], 0.0, 1.0, GradientStyle::Linear, false));
        if let Some(Fill::Gradient { angle: stored_angle, offset: stored_offset, .. }) = &mut malformed.fill {
            *stored_angle = angle;
            *stored_offset = offset;
        } else {
            panic!("gradient fixture")
        }
        let path_before = malformed.path.clone();
        assert!(matches!(
            CheckedShapeAlpha::try_new_with_control(&malformed, 0.01, area, VectorPreparationLimits::default(), &Interrupt::NONE, &mut work),
            Err(E::InvalidSample)
        ));
        let Some(Fill::Gradient { angle: after_angle, offset: after_offset, .. }) = &malformed.fill else { panic!("gradient retained") };
        assert_eq!(after_angle.to_bits(), angle.to_bits());
        assert_eq!(after_offset.0.to_bits(), offset.0.to_bits());
        assert_eq!(malformed.path, path_before);
    }
    // Directly reach layout with finite stored fields and finite frame extents:
    // scale multiplication overflows inside the old snapping equations.
    let huge = Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], 0.0, f32::MAX, GradientStyle::Linear, false);
    let huge_before = huge.clone();
    let frame = (0.0, 0.0, 1e300, 1e300);
    let old_layout = crate::gradient_layout(GradientStyle::Linear, 0.0, f32::MAX, (0.0, 0.0), frame);
    assert!(![old_layout.0, old_layout.1, old_layout.2.0, old_layout.2.1].iter().all(|v| v.is_finite()));
    let mut budget = ShapeBudget::new(VectorPreparationLimits::default(), &Interrupt::NONE, &mut work).unwrap();
    assert!(matches!(alpha_paint(&huge, frame, frame, &mut budget), Err(E::InvalidSample)));
    assert_eq!(huge, huge_before);
    // Sampling itself refuses nonfinite t rather than selecting a finite stop.
    let paint = AlphaPaint::Gradient {
        stops: vec![(0.0, 0.25), (1.0, 0.75)],
        midpoints: Vec::new(),
        opacity_stops: Vec::new(),
        style: GradientStyle::Linear,
        layout: (0.0, 1.0, (0.0, 0.0)),
        reverse: false,
        frame: (0.0, 0.0, 40.0, 40.0),
    };
    assert!(matches!(alpha_at(&paint, f64::NAN, 0.5, &mut budget), Err(E::InvalidSample)));
    assert!(polls.load(AtomicOrdering::Relaxed) > 40);
    let tiny = VectorPreparationLimits { max_bytes: 4096, ..VectorPreparationLimits::default() };
    assert!(matches!(CheckedShapeAlpha::try_new_with_control(&s, 0.01, area, tiny, &Interrupt::NONE, &mut work), Err(E::LimitExceeded)));
}
#[test]
fn checked_shape_render_polls_gradient_search_and_preserves_compiled_owner_on_failure() {
    let mut s = shape();
    s.fill = Some(Fill::gradient((0..4000).map(|i| (i as f32 / 3999.0, Color::rgba(0.3, 0.7, 0.2, 0.4))).collect(), 0.0, 1.0, GradientStyle::Linear, false));
    let area = Rect::new(0, 0, 40, 40);
    let mut debit = |_| Ok(());
    let (compiled, _) = CheckedShapeAlpha::try_new_with_control(&s, 0.01, area, VectorPreparationLimits::default(), &Interrupt::NONE, &mut debit).unwrap();
    let before = compiled.checked_preparation_bytes().unwrap();
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, AtomicOrdering::Relaxed) >= 100;
    assert!(matches!(
        compiled.try_render_with_control(area, VectorPreparationLimits::default(), &Interrupt::cancel_only(&cancel), &mut debit),
        Err(E::Cancelled)
    ));
    assert_eq!(compiled.checked_preparation_bytes().unwrap(), before);
    let (mut actual, _) =
        compiled.try_render_with_control(Rect::new(10, 10, 11, 11), VectorPreparationLimits::default(), &Interrupt::NONE, &mut debit).unwrap();
    assert_eq!(actual.pop().unwrap(), crate::CompiledShape::new(&s, 0.01, area).render_rgba(Rect::new(10, 10, 11, 11)).pop().unwrap()[3]);
}
