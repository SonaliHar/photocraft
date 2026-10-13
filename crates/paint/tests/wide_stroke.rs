use photocraft_color::{BlendMode, PixelFormat};
use photocraft_geom::Rect;
use photocraft_paint::{BrushSettings, StrokePoint, StrokeRenderer};
use photocraft_raster::{Interrupt, Surface};

#[test]
fn forged_retained_tile_origins_reject_normal_and_eraser_before_recomposition() {
    use photocraft_raster::RasterInputError;
    let mut valid = Surface::new(PixelFormat::RGBA64F);
    valid.try_write_pixel_f64(0, 0, &[0.0, 0.0, 0.0, 1.0]).unwrap();
    let tile = valid.tiles().next().unwrap().1.clone();
    let color =
        Surface::try_with_default_bytes(PixelFormat::RGBA64F, &[0.0_f64, 0.0, 0.0, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>()).unwrap();
    for erase in [false, true] {
        let mut pre = Surface::new(PixelFormat::RGBA64F);
        pre.put_tiles([(photocraft_geom::TileCoord::new(i32::MAX, i32::MIN), tile.clone())]);
        let mut target = pre.clone();
        let brush = BrushSettings { size: 1.0, hardness: 1.0, aliased: true, erase, ..Default::default() };
        let mut renderer = StrokeRenderer::new(&brush, Some(pre.format()), 1.0);
        renderer.push(&[StrokePoint::new(0.5, 0.5, 1.0)]);
        renderer.finish();
        assert_eq!(
            renderer.try_composite_wide(&pre, &mut target, &color, None, false, Some(Rect::new(0, 0, 2, 2)), &Interrupt::NONE),
            Err(RasterInputError::InvalidBounds)
        );
        assert_eq!(target, pre);
    }
}

#[test]
fn checked_stroke_clips_extent_recomposes_from_pre_and_cancels_atomically() {
    let raw = [-0.0_f64, 0.5 + 2.0_f64.powi(-40), 2.0, 0.75].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
    let pre = Surface::try_with_default_bytes(PixelFormat::RGBA64F, &raw).unwrap();
    let color =
        Surface::try_with_default_bytes(PixelFormat::RGBA64F, &[0.0_f64, 0.0, 0.0, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>()).unwrap();
    let brush = BrushSettings { size: 1.0, opacity: 0.5, aliased: true, hardness: 1.0, flow: 1.0, ..Default::default() };
    let mut renderer = StrokeRenderer::new(&brush, Some(pre.format()), 1.0);
    renderer.push(&[StrokePoint::new(0.5, 0.5, 1.0), StrokePoint::new(6.5, 0.5, 1.0)]);
    renderer.finish();
    let mut target = pre.clone();
    let clip = Rect::new(0, 0, 3, 2);
    renderer.try_composite_wide(&pre, &mut target, &color, None, false, Some(clip), &Interrupt::NONE).unwrap();
    let once = target.clone();
    renderer.try_composite_wide(&pre, &mut target, &color, None, false, Some(clip), &Interrupt::NONE).unwrap();
    assert_eq!(once, target);
    assert_eq!(target.try_read_interleaved(Rect::new(3, 0, 8, 2)).unwrap(), pre.try_read_interleaved(Rect::new(3, 0, 8, 2)).unwrap());
    assert_eq!(target.default_bytes(), raw);
    assert!(renderer.try_composite_wide(&pre, &mut target, &color, None, false, Some(clip), &Interrupt::cancel_only(&|| true)).is_err());
    assert_eq!(target, once);
    let mut unsupported = renderer.clone();
    unsupported.ctx.brush.mode = BlendMode::Multiply;
    assert!(unsupported.try_composite_wide(&pre, &mut target, &color, None, false, Some(clip), &Interrupt::NONE).is_err());
    assert_eq!(target, once);
}

#[test]
fn symmetry_union_has_one_opacity_ceiling_and_matches_single_pass() {
    let pre =
        Surface::try_with_default_bytes(PixelFormat::RGBA64F, &[0.25_f64, 0.5, 0.75, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>()).unwrap();
    let color =
        Surface::try_with_default_bytes(PixelFormat::RGBA64F, &[0.0_f64, 0.0, 0.0, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>()).unwrap();
    let brush = BrushSettings { size: 3.0, opacity: 0.25, aliased: true, hardness: 1.0, flow: 1.0, ..Default::default() };
    let mut original = StrokeRenderer::new(&brush, Some(pre.format()), 1.0);
    original.push(&[StrokePoint::new(2.5, 2.5, 1.0)]);
    original.finish();
    let mut single = pre.clone();
    original.try_composite_wide(&pre, &mut single, &color, None, false, None, &Interrupt::NONE).unwrap();
    let mut union = pre.clone();
    original.try_composite_union_wide(&original, &pre, &mut union, &color, None, false, None, &Interrupt::NONE).unwrap();
    assert_eq!(single, union);
}
