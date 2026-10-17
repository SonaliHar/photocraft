//! UNRUN direct mapper profile work controls, independent of earlier native qualifier work.
use super::*;
#[test]
#[ignore = "UNRUN: implicit Linear mapper prepays planning beforeallocation and refunds temporary peak while returned profile is caller-owned"]
fn controlled_linear_profile_work_and_peak() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../temp/sol4-linear-box-gimp-20261016/f32-author-v1/outputs/implicit-f32-linear-hdr-r4.xcf");
    let bytes = std::fs::read(root).unwrap();
    let f = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
    let small = limits.lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(small, None).unwrap();
    let baseline = b.used(BudgetKind::Resident);
    assert!(implicit_box_effective(&f, photocraft_cms::ColorSpace::Rgb, photocraft_cms::xcf_color::Encoding::Linear, &Interrupt::NONE, &b).is_err());
    assert_eq!(b.used(BudgetKind::Resident), baseline);
    let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    let baseline = b.used(BudgetKind::Resident);
    let effective = implicit_box_effective(&f, photocraft_cms::ColorSpace::Rgb, photocraft_cms::xcf_color::Encoding::Linear, &Interrupt::NONE, &b).unwrap();
    assert!(b.used(BudgetKind::Work) >= 65536 * 64);
    assert_eq!(b.used(BudgetKind::Resident), baseline);
    assert_eq!(
        effective.to_bytes(),
        photocraft_cms::xcf_color::effective_profile(None, photocraft_cms::ColorSpace::Rgb, photocraft_cms::xcf_color::Encoding::Linear).unwrap().to_bytes()
    );
    let stop = || true;
    assert!(matches!(
        implicit_box_effective(&f, photocraft_cms::ColorSpace::Rgb, photocraft_cms::xcf_color::Encoding::Linear, &Interrupt::cancel_only(&stop), &b),
        Err(IoError::Cancelled)
    ));
}
