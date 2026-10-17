//! UNRUN local research-input/source policy tests. Decoded buffers are external harness data.
//! These bind literal raw tuples, not effective-profile-byte or numeric acceptance.
use super::super::{SelectedStagedFamily, SourceSelectedPolicy, StagePolicy};
use super::*;
use sha2::{Digest, Sha256};
type CompositionTuple = (i32, i32, i32, i32, i32, i32, i32);
type SourceCase = (&'static str, &'static str, usize, &'static [CompositionTuple]);
const CASES: &[SourceCase] = &[
    (
        "temp/sol1-flat-multilayer-gimp-20261016/author-v1/outputs/three-auto.xcf",
        "547fccca9b828174af95882ac7ffd5c367fdfee1425f557b59d59168b00c0497",
        46062,
        &[(31, -4, -1, -2, 4, 1, 2), (30, -1, -1, -2, 1, 1, 2), (28, 0, -1, -1, 1, 1, 1)],
    ),
    (
        "temp/sol1-flat-multilayer-gimp-20261016/author-v1/outputs/three-hidden.xcf",
        "bb99313c91ea121d7c8b5e436718ddfdbdde43f00610f8599747c4933acab0b9",
        46062,
        &[(31, -4, -1, -2, 4, 1, 2), (30, -1, -1, -2, 1, 1, 2), (28, 0, -1, -1, 1, 1, 1)],
    ),
    (
        "temp/sol1-flat-multilayer-gimp-20261016/author-v1/outputs/three-zero.xcf",
        "3cd559eb1756e88a12f9c1adb54c5de34436d466a21b75d028f5279577815085",
        46062,
        &[(31, -4, -1, -2, 4, 1, 2), (30, -1, -1, -2, 1, 1, 2), (28, 0, -1, -1, 1, 1, 1)],
    ),
    (
        "temp/sol1-flat-multilayer-gimp-20261016/author-v1/outputs/four-auto.xcf",
        "48667ae4bbdaabbaceab109e2a2d237ee5f21e725633227476b91c0046c9382e",
        25089,
        &[(36, 0, -1, -2, 2, 1, 2), (34, -1, -1, -2, 1, 1, 2), (23, -4, -1, -2, 4, 1, 2), (28, 0, -1, -1, 1, 1, 1)],
    ),
    (
        "temp/sol1-flat-multilayer-gimp-20261016/author-v1/outputs/four-explicit.xcf",
        "14360b4c79e1b5804ef0911c1a754079173ada764c9e534b698053231a4ce9b0",
        64997,
        &[(44, 1, 4, 4, 1, 4, 4), (31, 4, 2, 3, 4, 2, 3), (30, 2, 1, 2, 2, 1, 2), (28, 0, 1, 1, 1, 1, 1)],
    ),
    (
        "temp/sol1-flat-multilayer-gimp-20261016/sixth-author-v1/outputs/sixth-overlapping-explicit.xcf",
        "8ec81353a220f3f1a671aae05b90c59c704f92b779d944b8e3c47c478317a113",
        133907,
        &[(44, 1, 4, 4, 1, 4, 4), (31, 4, 2, 3, 4, 2, 3), (30, 2, 1, 2, 2, 1, 2), (28, 0, 1, 1, 1, 1, 1)],
    ),
];
fn source(i: usize) -> XcfFile {
    let (path, pin, len, _) = CASES[i];
    let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(path)).unwrap();
    assert_eq!(bytes.len(), len);
    assert_eq!(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), pin);
    photocraft_xcf::decode(&bytes, &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new_persistent(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None).unwrap()
}
fn space(n: i32) -> photocraft_doc::import_compositing::GimpSpace {
    use photocraft_doc::import_compositing::GimpSpace::*;
    match n {
        1 => LinearRgb,
        2 => ProfileRgb,
        4 => PerceptualRgb,
        _ => panic!("literal expected space"),
    }
}
fn operator(n: i32) -> photocraft_doc::import_compositing::GimpOperator {
    use photocraft_doc::import_compositing::GimpOperator::*;
    match n {
        1 => Union,
        2 => ClipToBackdrop,
        3 => ClipToLayer,
        4 => Intersection,
        _ => panic!("literal expected operator"),
    }
}
#[test]
#[ignore = "UNRUN: six hash-bound research inputs; literal raw/resolved source policy only, profile-byte HOLD remains"]
fn six_research_inputs_literal_policy() {
    for (i, case_row) in CASES.iter().enumerate() {
        let f = source(i);
        assert_eq!(f.layers.len(), case_row.3.len());
        assert_eq!(SourceSelectedPolicy::family_controlled(&f, &Interrupt::NONE, Some(&budget())).unwrap(), SelectedStagedFamily::SeparableBlend);
        assert_eq!(qualify_selected(&f, &Interrupt::NONE, &budget()).unwrap(), Some(SourceKind::General));
        for (d, &(m, b, c, o, rb, rc, ro)) in f.layers.iter().zip(case_row.3) {
            assert_eq!((d.mode, d.blend_space, d.composite_space, d.composite_mode), (m, b, c, o));
            let actual = photocraft_doc::import_compositing::flat_modern_source_rule(f.version, m, b, c, o).unwrap().resolved();
            assert_eq!((actual.blend_space, actual.composite_space, actual.operator), (space(rb), space(rc), operator(ro)));
        }
    }
}
#[test]
#[ignore = "UNRUN: all-Normal priority, old50 strict native failure has no general retry"]
fn family_priority_and_strict_failure() {
    let mut f = source(0);
    for d in &mut f.layers {
        d.mode = 28;
        d.blend_space = 0;
        d.composite_space = -1;
        d.composite_mode = -1;
    }
    assert_eq!(SourceSelectedPolicy::family_controlled(&f, &Interrupt::NONE, Some(&budget())).unwrap(), SelectedStagedFamily::Plain);
    let bytes =
        std::fs::read(std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("composition-matrix/mode-grain-merge.xcf"))
            .unwrap();
    assert_eq!(bytes.len(), 5542);
    assert_eq!(
        Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "c6d004d91d69fae7a76a7173d70f4f885e8a6d7cf2384b4de4582dec7f7f2c3f"
    );
    let mut old = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
    assert_eq!(qualify_selected(&old, &Interrupt::NONE, &budget()).unwrap(), Some(SourceKind::Strict(Domain::Basic)));
    old.layers[1].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&1.5f32.to_ne_bytes());
    assert!(qualify_selected(&old, &Interrupt::NONE, &budget()).unwrap().is_none());
}
#[test]
#[ignore = "UNRUN: hidden/zero/offcanvas operative native words and hostile raw/profile metadata remain checked"]
fn skipped_inputs_and_source_refusals() {
    for case in 0..7 {
        let mut f = source(0);
        f.layers[0].visible = false;
        f.layers[0].opacity = 0.0;
        f.layers[0].offset = (-1000, -1000);
        match case {
            0 => f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
            1 => f.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&1.25f32.to_ne_bytes()),
            2 => {
                f.layers[0].mode = 41;
                f.layers[0].blend_space = 1;
                f.layers[0].composite_space = 1;
                f.layers[0].composite_mode = 2;
                f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&0f32.to_ne_bytes());
            }
            3 => f.layers[0].blend_space = -99,
            4 => f.icc = vec![0; 132],
            5 => f.layers[0].plane.as_mut().unwrap().bytes.truncate(16),
            _ => f.layers[0].mode = 24,
        }
        assert!(qualify_selected(&f, &Interrupt::NONE, &budget()).unwrap().is_none());
    }
}

#[test]
#[ignore = "UNRUN: hint and native scans use cumulative supplied work/cancellation; finite HDR Normal companion stays allowed"]
fn controlled_hint_and_native_scan() {
    let mut f = source(0);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 2)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(general_hint(&f, &Interrupt::NONE, &b).is_err());
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(qualify_selected(&f, &Interrupt::NONE, &b).is_err());
    let stop = || true;
    assert!(matches!(general_hint(&f, &Interrupt::cancel_only(&stop), &budget()), Err(IoError::Cancelled)));
    let last = f.layers.len() - 1;
    f.layers[last].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&1.5f32.to_ne_bytes());
    assert_eq!(qualify_selected(&f, &Interrupt::NONE, &budget()).unwrap(), Some(SourceKind::General));
}
