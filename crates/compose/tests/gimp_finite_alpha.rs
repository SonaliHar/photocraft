use photocraft_compose::gimp_composite::WorkingSpace;
use photocraft_doc::{
    SampleEncoding,
    import_compositing::{GimpOperator, GimpSpace},
};
#[test]
fn finite_alpha_is_separate_from_unit_coverage() {
    let space = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    let transparent = [0.0; 4];
    let source = [-2.0, 4.0, -0.0, -0.01];
    let output = space.normal_finite_source(transparent, source, 1.0, GimpOperator::Union).unwrap();
    assert_eq!(output.map(f64::to_bits), source.map(f64::to_bits));
    assert!(space.normal(transparent, source, 1.0, GimpOperator::Union).is_err());
    let backdrop = [0.2, 0.4, 0.6, 1.0];
    assert_eq!(space.normal_finite_source(backdrop, source, 1.0, GimpOperator::Union).unwrap(), backdrop);
    let over = space.normal_finite_source(backdrop, [0.3, 0.5, 0.7, 1.03], 1.0, GimpOperator::Union).unwrap();
    assert!((over[0] - 0.303).abs() < 1e-15);
    assert_eq!(over[3], 1.0);
    for operator in [GimpOperator::ClipToBackdrop, GimpOperator::ClipToLayer, GimpOperator::Intersection] {
        assert!(space.normal_finite_source(backdrop, source, 1.0, operator).is_err());
    }
    let profile_space = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
    assert!(profile_space.normal_finite_source(backdrop, source, 1.0, GimpOperator::Union).is_err());
    assert!(space.normal_finite_source(backdrop, source, f64::NAN, GimpOperator::Union).is_err());
    assert!(space.normal_finite_source(backdrop, [f64::INFINITY, 0.0, 0.0, 1.0], 1.0, GimpOperator::Union).is_err());
}
