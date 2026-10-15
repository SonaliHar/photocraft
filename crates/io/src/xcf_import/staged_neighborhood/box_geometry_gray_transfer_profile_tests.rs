//! UNRUN exact four Gray transfer roles use the same controlled mapper factory.
use super::*;
#[test]
#[ignore = "UNRUN: allfour SGray transfer reconstructions prepay planning/actual peak/constructor and preserve distinct current ICC bytes"]
fn gray_transfer_profile_work_and_roles() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol4-gray-transfer-box-gimp-20261016");
    let cases = [
        ("linear-f32", 600, photocraft_cms::xcf_color::Encoding::Linear),
        ("linear-f64", 700, photocraft_cms::xcf_color::Encoding::Linear),
        ("perceptual-f32", 675, photocraft_cms::xcf_color::Encoding::Perceptual),
        ("perceptual-f64", 775, photocraft_cms::xcf_color::Encoding::Perceptual),
    ];
    for (name, precision, ce) in cases {
        let bytes = std::fs::read(root.join(format!("{name}-author-v1/outputs/implicit-gray-{name}-hdr-r4.xcf"))).unwrap();
        let f = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!(f.precision, precision);
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
        let small = limits.lowered(&[("sampleWorkUnits", 32)]).unwrap();
        let b = photocraft_xcf::Budget::new_persistent(small, None).unwrap();
        assert!(implicit_box_effective(&f, photocraft_cms::ColorSpace::Gray, ce, &Interrupt::NONE, &b).is_err());
        assert_eq!(b.used(BudgetKind::Resident), 0);
        let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
        let effective = implicit_box_effective(&f, photocraft_cms::ColorSpace::Gray, ce, &Interrupt::NONE, &b).unwrap();
        assert!(b.used(BudgetKind::Work) >= 65536 * 64);
        assert_eq!(b.used(BudgetKind::Resident), 0);
        assert_eq!(effective.to_bytes(), photocraft_cms::xcf_color::effective_profile(None, photocraft_cms::ColorSpace::Gray, ce).unwrap().to_bytes());
        assert_ne!(effective.to_bytes(), photocraft_cms::Builtin::SGray.profile().to_bytes());
        assert_ne!(effective.to_bytes(), photocraft_cms::Builtin::Srgb.profile().to_bytes());
        assert!(implicit_box_effective(&f, photocraft_cms::ColorSpace::Rgb, ce, &Interrupt::NONE, &b).is_err());
        assert!(
            implicit_box_effective(&f, photocraft_cms::ColorSpace::Gray, photocraft_cms::xcf_color::Encoding::NonlinearProfile, &Interrupt::NONE, &b).is_err()
        );
        let mut assigned = f.clone();
        assigned.icc = vec![0; 640];
        assert!(implicit_box_effective(&assigned, photocraft_cms::ColorSpace::Gray, ce, &Interrupt::NONE, &b).is_err());
        let stop = || true;
        assert!(matches!(implicit_box_effective(&f, photocraft_cms::ColorSpace::Gray, ce, &Interrupt::cancel_only(&stop), &b), Err(IoError::Cancelled)));
    }
}
