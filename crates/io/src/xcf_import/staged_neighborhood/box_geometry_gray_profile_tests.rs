//! UNRUN direct mapper profile work controls, independent of earlier native qualifier work.
use super::*;
#[test]
#[ignore = "UNRUN: implicit NonlinearProfile mapper prepays planning beforeallocation and refunds temporary peak while returned profile is caller-owned"]
fn controlled_gray_profile_work_and_peak() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../temp/sol4-gray-box-gimp-20261016/f32-author-v1/outputs/implicit-gray-f32-nl-hdr-r4.xcf");
    let bytes = std::fs::read(root).unwrap();
    let f = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
    let small = limits.lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(small, None).unwrap();
    assert!(implicit_box_effective(&f, photocraft_cms::ColorSpace::Gray, photocraft_cms::xcf_color::Encoding::NonlinearProfile, &Interrupt::NONE, &b).is_err());
    assert_eq!(b.used(BudgetKind::Resident), 0);
    let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    let effective =
        implicit_box_effective(&f, photocraft_cms::ColorSpace::Gray, photocraft_cms::xcf_color::Encoding::NonlinearProfile, &Interrupt::NONE, &b).unwrap();
    assert!(b.used(BudgetKind::Work) >= 65536 * 64);
    assert_eq!(b.used(BudgetKind::Resident), 0);
    assert_eq!(
        effective.to_bytes(),
        photocraft_cms::xcf_color::effective_profile(None, photocraft_cms::ColorSpace::Gray, photocraft_cms::xcf_color::Encoding::NonlinearProfile)
            .unwrap()
            .to_bytes()
    );
    assert!(implicit_box_effective(&f, photocraft_cms::ColorSpace::Gray, photocraft_cms::xcf_color::Encoding::Linear, &Interrupt::NONE, &b).is_err());
    assert!(implicit_box_effective(&f, photocraft_cms::ColorSpace::Rgb, photocraft_cms::xcf_color::Encoding::NonlinearProfile, &Interrupt::NONE, &b).is_err());
    let mut assigned = f.clone();
    assigned.icc = vec![0; 640];
    assert!(
        implicit_box_effective(&assigned, photocraft_cms::ColorSpace::Gray, photocraft_cms::xcf_color::Encoding::NonlinearProfile, &Interrupt::NONE, &b)
            .is_err()
    );
    let stop = || true;
    assert!(matches!(
        implicit_box_effective(&f, photocraft_cms::ColorSpace::Gray, photocraft_cms::xcf_color::Encoding::NonlinearProfile, &Interrupt::cancel_only(&stop), &b),
        Err(IoError::Cancelled)
    ));
}
