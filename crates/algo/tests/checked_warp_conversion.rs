use photocraft_algo::{
    puppet,
    transform::{Homography, Interp},
    vanishing, warp, wideangle,
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Affine, Rect};
use photocraft_raster::Surface;

#[test]
fn exact_wide_placement_translation_keeps_native_default_and_source_bits() {
    let area = Rect::new(0, 0, 1, 1);
    for sample in [SampleType::U32, SampleType::F64] {
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        let bytes: Vec<u8> = if sample == SampleType::U32 {
            [1_u32, 0x80000001, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect()
        } else {
            [-0.0_f64, 0.5 + 2.0_f64.powi(-40), 1.125, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect()
        };
        let mut source = Surface::new(format);
        source.try_write_interleaved(area, &bytes).unwrap();
        let before = source.clone();
        let placement = Affine { m: [1.0, 0.0, 0.0, 1.0, 7.0, -3.0] };
        let out = warp::try_place_source(&source, area, &placement, None).unwrap();
        assert_eq!(out.try_read_interleaved(area.translate(7, -3)).unwrap(), bytes);
        assert_eq!(out.default_bytes(), source.default_bytes());
        assert!(warp::try_place_source(&source, area, &Affine { m: [1.0, 0.0, 0.0, 1.0, 0.5, 0.0] }, None).is_err());
        assert_eq!(warp::try_place_source_projective(&source, area, &Homography::IDENTITY, None).unwrap().try_read_interleaved(area).unwrap(), bytes);
        assert_eq!(source, before);
    }
}

#[test]
fn checked_auxiliary_deformations_reject_wide_before_solving_or_masking() {
    let area = Rect::new(0, 0, 1, 1);
    let source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
    let before = source.clone();
    assert!(wideangle::try_apply(&source, area, &wideangle::WideAngle::default(), Interp::Nearest).is_err());
    let config = puppet::PuppetWarp { pins: Vec::new(), mode: Default::default(), density: Default::default(), expansion: 2.0 };
    assert!(puppet::try_puppet_warp(&source, area, &config, Interp::Nearest).is_err());
    let scene = vanishing::Scene { planes: Vec::new(), focal: 50.0, center: [0.0, 0.0] };
    assert!(vanishing::try_paste(&scene, 0, &source, area, [0.0, 0.0], 1.0).is_err());
    assert!(warp::try_warp_mesh_gray(&source, &|x, y| (x, y), Interp::Nearest).is_err());
    assert_eq!(source, before);
}

#[test]
fn checked_warps_reject_extreme_nonfinite_and_invalid_geometry_without_panic() {
    let source = Surface::new(PixelFormat::RGBA8);
    let area = Rect::new(0, 0, 1, 1);
    assert!(warp::try_warp_mesh_surface(&source, area, &|_, _| (f64::INFINITY, 0.0), Interp::Nearest).is_err());
    assert!(warp::try_warp_mesh_surface(&source, area, &|x, y| (x * 1e12, y), Interp::Nearest).is_err());
    assert!(photocraft_algo::transform::try_warp_surface(&source, area, &Homography([1., 0., 1e30, 0., 1., 0., 0., 0., 1.]), Interp::Nearest).is_err());
    assert!(puppet::try_render(&source, area, &puppet::PuppetMesh::default(), &[], &[0], Interp::Nearest).is_err());
}
