//! UNRUN alpha RGB/Gray source boundary. Decoded/native captures are external harness buffers.
//! Missing eight Gray producer ICC captures remain HOLD; SGray current basis is
//! an existing typed interpretation, not byte equivalence to absent evidence.
use super::super::{SelectedStagedFamily, SourceSelectedPolicy, StagePolicy};
use super::*;
use photocraft_doc::import_compositing::{GimpOperator, GimpSpace};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: &[(i32, &str, &str, usize, (i32, i32, i32), bool, GimpSpace, GimpSpace, bool)] = &[
    (
        58,
        "composition-matrix/mode-erase.xcf",
        "34b7a66065b6c95d721e07c6702e7cc683a06e420ea0cc3a0a6015a99a9043a9",
        5425,
        (0, -1, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
        false,
    ),
    (
        58,
        "composition-matrix/mode-erase.edit.xcf",
        "8fc7771f5165aa24406a4d6b598b469608d1cbd3fa43d96dd9fe4907d94d4953",
        5425,
        (0, -1, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
        false,
    ),
    (
        59,
        "composition-matrix/mode-merge.xcf",
        "0f47d8bd893192f72373c18277e2f7963a0a8cf96c035eae7237ad191d6a92c6",
        5425,
        (0, -1, -1),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
        false,
    ),
    (
        59,
        "composition-matrix/mode-merge.edit.xcf",
        "39ef3113c93105d28b1831afe76d57eeaa1afc0203481fdedae5efd55880ac8a",
        5425,
        (0, -1, -1),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
        false,
    ),
    (
        60,
        "composition-matrix/mode-split.xcf",
        "b7a06e3b0b55af7c070726a5b4e1773d4878012f098fcaeb24ef445621e2cc98",
        5425,
        (0, 0, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::ProfileRgb,
        false,
    ),
    (
        60,
        "composition-matrix/mode-split.edit.xcf",
        "dd49fc7598f045fcaef77a73ea37e7d6693312c366fefa0df5e4869e7c400bc0",
        5425,
        (0, 0, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::ProfileRgb,
        false,
    ),
    (
        57,
        "composition-matrix/mode-color-erase.xcf",
        "12e80e12cb37b06cc624e0a0a3971b74d905a8686792ecf0661293c94fa9e949",
        5425,
        (-1, -1, -2),
        false,
        GimpSpace::LinearRgb,
        GimpSpace::LinearRgb,
        false,
    ),
    (
        57,
        "composition-matrix/mode-color-erase.edit.xcf",
        "21414630eb9d9cf6f785e5849db724c723d64b0e367eb077b08bcc34f3b3fc41",
        5425,
        (-1, -1, -2),
        false,
        GimpSpace::LinearRgb,
        GimpSpace::LinearRgb,
        false,
    ),
    (
        57,
        "alpha-gray-evidence/mode-57.xcf",
        "e5d2daa85de2ded2009b3930798be7854fb88c16b3a7489d18aab1c18e289c87",
        3153,
        (-1, -1, -2),
        false,
        GimpSpace::LinearRgb,
        GimpSpace::LinearRgb,
        true,
    ),
    (
        57,
        "alpha-gray-evidence/mode-57.edit.xcf",
        "7749efea7978a9b1746cd6e13fa790fbd1311345e3e9163503f8300720dbc872",
        3153,
        (-1, -1, -2),
        false,
        GimpSpace::LinearRgb,
        GimpSpace::LinearRgb,
        true,
    ),
    (
        58,
        "alpha-gray-evidence/mode-58.xcf",
        "4db2455e4a677850d22851b70139f98ee447b7faaeebaf3aa08fdcffdc9f8284",
        3153,
        (0, -1, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
        true,
    ),
    (
        58,
        "alpha-gray-evidence/mode-58.edit.xcf",
        "69917d9f09ac58127090e8cd4a54438540204c5cf4bb034d1189759f36741ab8",
        3153,
        (0, -1, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
        true,
    ),
    (
        59,
        "alpha-gray-evidence/mode-59.xcf",
        "91aab8b6e5e208fb1bd74f4cf118e88a569250d35e1d88a1042f6111c68ee411",
        3153,
        (0, -1, -1),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
        true,
    ),
    (
        59,
        "alpha-gray-evidence/mode-59.edit.xcf",
        "58c51251598a839ffe178cf12e7a60cc529b1910f6a012f9ea9a38e308f43adc",
        3153,
        (0, -1, -1),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
        true,
    ),
    (
        60,
        "alpha-gray-evidence/mode-60.xcf",
        "e07c92d5c54a8a5d6f2864480cdd1d09abce3002f44ef7eb7ab03bb76846c687",
        3153,
        (0, 0, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::ProfileRgb,
        true,
    ),
    (
        60,
        "alpha-gray-evidence/mode-60.edit.xcf",
        "53754c96455d25cd8e9dc17f28b602bb2ba2c5fc568f89869a8f031d1faa516c",
        3153,
        (0, 0, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::ProfileRgb,
        true,
    ),
];
fn expected(_: usize) -> SourceKind {
    SourceKind::AlphaStrict
}
fn operator(i: usize) -> GimpOperator {
    if CASES[i].0 == 59 { GimpOperator::Union } else { GimpOperator::ClipToBackdrop }
}
fn bytes(i: usize) -> Vec<u8> {
    let (_, name, pin, len, ..) = CASES[i];
    let b = std::fs::read(PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(name)).unwrap();
    assert_eq!(b.len(), len);
    assert_eq!(Sha256::digest(&b).iter().map(|b| format!("{b:02x}")).collect::<String>(), pin);
    b
}
fn source(i: usize) -> XcfFile {
    photocraft_xcf::decode(&bytes(i), &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new_persistent(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None).unwrap()
}
#[test]
#[ignore = "UNRUN: all16 genuine source/edit packets retain exact old metadata and independent RGB/Gray alpha raw policy"]
fn exact_old_packets() {
    use photocraft_doc::import_compositing::{GimpOperator, GimpSpace};
    for i in 0..CASES.len() {
        let f = source(i);
        assert_eq!(SourceSelectedPolicy::family_controlled(&f, &Interrupt::NONE, Some(&budget())).unwrap(), SelectedStagedFamily::SeparableBlend);
        assert_eq!(qualify_selected(&f, &Interrupt::NONE, &budget()).unwrap(), Some(expected(i)));
        assert!(admission::classify_late(&f).basic_blend_domain);
        assert_eq!(
            (f.version, f.layers[0].mode, f.layers[0].blend_space, f.layers[0].composite_space, f.layers[0].composite_mode),
            (12, CASES[i].0, CASES[i].4.0, CASES[i].4.1, CASES[i].4.2)
        );
        assert_eq!((f.layers[1].mode, f.layers[1].blend_space, f.layers[1].composite_space, f.layers[1].composite_mode), (28, 0, -1, -1));
        let (b, c, o) = CASES[i].4;
        let r = photocraft_doc::import_compositing::flat_alpha_source_rule(12, CASES[i].0, b, c, o).unwrap().resolved();
        assert_eq!((r.blend_space, r.composite_space, r.operator), (CASES[i].6, CASES[i].7, operator(i)));
    }
}
#[test]
#[ignore = "UNRUN: all16 single selected decode promotes actual paid native/class with original raw epochs, not eager or Plain fallback"]
fn genuine_paid_source_mapping() {
    use photocraft_doc::{
        LayerContent,
        import_compositing::{GimpOperator, GimpSpace},
        item_identity::ItemRef,
    };
    use photocraft_raster::{
        Interrupt,
        backing::{
            ReadControl,
            quota::{BackingQuota, QuotaLimits},
        },
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    for i in 0..CASES.len() {
        let b = bytes(i);
        let eager = photocraft_xcf::decode(&b, &Default::default()).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
            .unwrap();
        let result =
            crate::import_owned_xcf_from_reader_in_quota(CASES[i].1, &mut Cursor::new(&b), b.len() as u64, &Default::default(), &limits, q.clone(), &c)
                .unwrap();
        let crate::OwnedXcfImport::SeparableBlend(imported) = result else { panic!("RGB/Gray alpha strict route") };
        let owner = imported.document();
        assert!(owner.is_readonly_flat_composite_storage());
        let d = owner.document();
        assert_eq!(d.mode.color_channels(), if CASES[i].8 { 1 } else { 3 });
        if CASES[i].8 {
            assert!(matches!(&d.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSGray { .. }));
            assert_eq!(d.icc_profile.as_ref().unwrap().as_slice(), photocraft_cms::Builtin::SGray.profile().to_bytes().as_slice());
        }
        for (mapped, original) in d.layers.iter().zip(eager.layers.iter().rev()) {
            let row = d.xcf_state.as_ref().unwrap().items.iter().find(|r| r.item == ItemRef::Layer(mapped.id)).unwrap();
            let p = row.composition.as_ref().unwrap();
            assert_eq!(
                (p.source_mode, p.stored_blend_space, p.stored_composite_space, p.stored_operator),
                (original.mode, original.blend_space, original.composite_space, original.composite_mode)
            );
            let r = p.resolved.as_ref().unwrap();
            if original.mode == 28 {
                assert!(!p.legacy);
                assert_eq!(p.compatibility_epoch, "xcf-12-normal");
                assert_eq!((r.blend_space, r.composite_space, r.operator), (GimpSpace::LinearRgb, GimpSpace::LinearRgb, GimpOperator::Union));
            } else {
                assert_eq!(p.legacy, CASES[i].5);
                assert_eq!(p.compatibility_epoch, "xcf-12-authored-blend-auto");
                assert_eq!((r.blend_space, r.composite_space, r.operator), (CASES[i].6, CASES[i].7, operator(i)));
            }
            let LayerContent::BackedRaster(native) = &mapped.content else { panic!("paid native") };
            let rect = native.operative_bounds();
            let mut actual = Vec::new();
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    let mut pixel = vec![0; native.format().bytes_per_pixel()];
                    native.checked_view().read_pixel_native_into(x, y, &mut pixel, &c).unwrap();
                    actual.extend_from_slice(&pixel);
                }
            }
            assert_eq!(actual, original.plane.as_ref().unwrap().bytes);
        }
    }
}
#[test]
#[ignore = "UNRUN: RGB/Gray descriptors/channel counts and both full native planes remain checked; no source retry"]
fn strict_refusals() {
    for i in [0, 8] {
        for case in 0..11 {
            let mut f = source(i);
            let channels = if f.model == Model::Gray { 2 } else { 4 };
            match case {
                0 => f.layers[1].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&1.5f32.to_ne_bytes()),
                1 => f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
                2 => f.layers[0].plane.as_mut().unwrap().bytes[channels * 4 - 4..channels * 4].copy_from_slice(&(-0.25f32).to_ne_bytes()),
                3 => {
                    f.layers[0].plane.as_mut().unwrap().bytes.pop();
                }
                4 => f.layers[0].offset = (0, 0),
                5 => f.layers[0].visible = false,
                6 => f.layers[0].mode = 29,
                7 => f.layers[0].blend_space = -99,
                8 => f.icc = vec![0; 132],
                9 => f.layers[0].plane.as_mut().unwrap().channels = if channels == 2 { 4 } else { 2 },
                _ => f.version = 23,
            }
            assert!(qualify_selected(&f, &Interrupt::NONE, &budget()).unwrap().is_none());
        }
    }
}
#[test]
#[ignore = "UNRUN: RGB/Gray alpha endpoints remain unit-only, paid work/cancellation and allNormal Plain priority"]
fn endpoints_control_and_plain_priority() {
    for i in 0..CASES.len() {
        for v in [0f32, 1f32] {
            let mut f = source(i);
            f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&v.to_ne_bytes());
            assert_eq!(qualify_selected(&f, &Interrupt::NONE, &budget()).unwrap(), Some(expected(i)));
        }
    }
    let mut f = source(8);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(qualify_selected(&f, &Interrupt::NONE, &b).is_err());
    let stop = || true;
    assert!(matches!(qualify_selected(&f, &Interrupt::cancel_only(&stop), &budget()), Err(IoError::Cancelled)));
    for d in &mut f.layers {
        d.mode = 28;
        d.blend_space = 0;
        d.composite_space = -1;
        d.composite_mode = -1;
    }
    assert_eq!(SourceSelectedPolicy::family_controlled(&f, &Interrupt::NONE, Some(&budget())).unwrap(), SelectedStagedFamily::Plain);
}
