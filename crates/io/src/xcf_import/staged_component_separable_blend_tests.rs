//! UNRUN strict component/luma source boundary; external decoded/native harness buffers.
//! No generalized producer or numerical acceptance claim.
use super::super::{SelectedStagedFamily, SourceSelectedPolicy, StagePolicy};
use super::*;
use photocraft_doc::import_compositing::GimpSpace;
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: &[(i32, &str, &str, usize, (i32, i32, i32), bool, GimpSpace, GimpSpace)] = &[
    (
        11,
        "composition-matrix/mode-hsv-hue-legacy.xcf",
        "199a4e935c0a41c269c37e96034fe10c1cd25da37bf5dc429e77d4365d0cd29c",
        5425,
        (-2, -2, -2),
        true,
        GimpSpace::ProfileRgb,
        GimpSpace::ProfileRgb,
    ),
    (
        11,
        "composition-matrix/mode-hsv-hue-legacy.edit.xcf",
        "e6a0b51f5c066affb7410db75b346d77adea642e20d430b13c0db465089a0356",
        5425,
        (-2, -2, -2),
        true,
        GimpSpace::ProfileRgb,
        GimpSpace::ProfileRgb,
    ),
    (
        12,
        "composition-matrix/mode-hsv-saturation-legacy.xcf",
        "d08ef0c7acef8cc8c7e5c4724ef76a3c5b74e619c672f1b7b8c327c554faafb6",
        5425,
        (-2, -2, -2),
        true,
        GimpSpace::ProfileRgb,
        GimpSpace::ProfileRgb,
    ),
    (
        12,
        "composition-matrix/mode-hsv-saturation-legacy.edit.xcf",
        "79254653da78290a7d58938b05d62bc3e8f7d9efee127e90228e11f6ca217237",
        5425,
        (-2, -2, -2),
        true,
        GimpSpace::ProfileRgb,
        GimpSpace::ProfileRgb,
    ),
    (
        13,
        "composition-matrix/mode-hsl-color-legacy.xcf",
        "f44a49fb38ba3a574e5c778b6625f1ee0f4b85f3077fb817123499e59431cb02",
        5425,
        (-2, -2, -2),
        true,
        GimpSpace::ProfileRgb,
        GimpSpace::ProfileRgb,
    ),
    (
        13,
        "composition-matrix/mode-hsl-color-legacy.edit.xcf",
        "c153bb5711c96835e25e95e617e40b0a5a2bd948392c354b3c5a16c08e33d2a5",
        5425,
        (-2, -2, -2),
        true,
        GimpSpace::ProfileRgb,
        GimpSpace::ProfileRgb,
    ),
    (
        14,
        "composition-matrix/mode-hsv-value-legacy.xcf",
        "96e77593f731a6048110e523e84d2dc6bbf5ef92c18bf11cd1d75fdbe2b85331",
        5425,
        (-2, -2, -2),
        true,
        GimpSpace::ProfileRgb,
        GimpSpace::ProfileRgb,
    ),
    (
        14,
        "composition-matrix/mode-hsv-value-legacy.edit.xcf",
        "c927a2f12dda9060456154077c3e29e31940c910f483d07d3d8c0a4f758017e4",
        5425,
        (-2, -2, -2),
        true,
        GimpSpace::ProfileRgb,
        GimpSpace::ProfileRgb,
    ),
    (
        37,
        "composition-matrix/mode-hsv-hue.xcf",
        "c3c7222d1c04228c4424ae6dbe904c5d5f97f838edab0df321dd2b4b64f0202e",
        5425,
        (-2, -1, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
    ),
    (
        37,
        "composition-matrix/mode-hsv-hue.edit.xcf",
        "92bf51dc0e56a8fe79eca0bb1cb3b744d875b60f9772f0fee007fa8828d3e73f",
        5425,
        (-2, -1, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
    ),
    (
        38,
        "composition-matrix/mode-hsv-saturation.xcf",
        "371826088442cf3c034d00abf10c8e91cdbcb502b24ffa836c11775629549d94",
        5425,
        (-2, -1, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
    ),
    (
        38,
        "composition-matrix/mode-hsv-saturation.edit.xcf",
        "720861856428a4eb0b138ab9e8e5a50f3fa61ae236196af9391f3dbd14e72f17",
        5425,
        (-2, -1, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
    ),
    (
        39,
        "composition-matrix/mode-hsl-color.xcf",
        "205e9879b61586f15d39c33ad88f6521aad496b1ae563442119c35dab406bc53",
        5425,
        (-2, -1, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
    ),
    (
        39,
        "composition-matrix/mode-hsl-color.edit.xcf",
        "df1755d7b6427bc620f9bee536156dc833e36d6ac3446ca73e8eb69d5a0349f2",
        5425,
        (-2, -1, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
    ),
    (
        40,
        "composition-matrix/mode-hsv-value.xcf",
        "601898d0fe3152aa58480a8ac4e892eafe7118ba579e96a10bcd2df505ddd696",
        5425,
        (-2, -1, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
    ),
    (
        40,
        "composition-matrix/mode-hsv-value.edit.xcf",
        "63db78fc8007a36d9b151c6116288890b3aabe5511f6e9d2992ff24899b35762",
        5425,
        (-2, -1, -2),
        false,
        GimpSpace::ProfileRgb,
        GimpSpace::LinearRgb,
    ),
    (
        54,
        "composition-matrix/mode-luma-darken-only.xcf",
        "82168822453034ad39ba38bbdc2b161c05062757e22744cb5cf6ce0a5a114f5a",
        5425,
        (-1, -1, -2),
        false,
        GimpSpace::LinearRgb,
        GimpSpace::LinearRgb,
    ),
    (
        54,
        "composition-matrix/mode-luma-darken-only.edit.xcf",
        "2f2a65180e2e9e6f918c02b5892d7cc2528c7de6a8f5d0cb1770b407deecc918",
        5425,
        (-1, -1, -2),
        false,
        GimpSpace::LinearRgb,
        GimpSpace::LinearRgb,
    ),
    (
        55,
        "composition-matrix/mode-luma-lighten-only.xcf",
        "d987678275e33745283f2914543fda48c037f96a7da6781a58e5e5ba387ab21c",
        5425,
        (-1, -1, -2),
        false,
        GimpSpace::LinearRgb,
        GimpSpace::LinearRgb,
    ),
    (
        55,
        "composition-matrix/mode-luma-lighten-only.edit.xcf",
        "394fd6f196f67a6d5ad4ff6299042383c6b392c4591685eb8f7f32b5cf02e76c",
        5425,
        (-1, -1, -2),
        false,
        GimpSpace::LinearRgb,
        GimpSpace::LinearRgb,
    ),
    (
        56,
        "composition-matrix/mode-luminance.xcf",
        "df61e7f07f9a15200cedca0e4ec620786ea6fe9b65f56529e2ed6388976a312b",
        5425,
        (-1, -1, -2),
        false,
        GimpSpace::LinearRgb,
        GimpSpace::LinearRgb,
    ),
    (
        56,
        "composition-matrix/mode-luminance.edit.xcf",
        "110603b2ee7c17aac0cfb944fd1e8123d51da61c80b55a88f115b8ec9b1dbd11",
        5425,
        (-1, -1, -2),
        false,
        GimpSpace::LinearRgb,
        GimpSpace::LinearRgb,
    ),
];
fn expected(i: usize) -> SourceKind {
    if CASES[i].5 { SourceKind::LegacyStrict } else { SourceKind::ComponentStrict }
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
#[ignore = "UNRUN: all22 genuine source/edit packets retain exact old metadata and independent component/luma raw policy"]
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
        let r = if CASES[i].5 {
            photocraft_doc::import_compositing::flat_legacy_source_rule(12, CASES[i].0, b, c, o).unwrap().resolved()
        } else {
            photocraft_doc::import_compositing::flat_component_source_rule(12, CASES[i].0, b, c, o).unwrap().resolved()
        };
        assert_eq!((r.blend_space, r.composite_space, r.operator), (CASES[i].6, CASES[i].7, GimpOperator::ClipToBackdrop));
    }
}
#[test]
#[ignore = "UNRUN: all22 single selected decode promotes actual paid native/class with original raw epochs, not eager or Plain fallback"]
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
        let crate::OwnedXcfImport::SeparableBlend(imported) = result else { panic!("component/luma strict route") };
        let owner = imported.document();
        assert!(owner.is_readonly_flat_composite_storage());
        let d = owner.document();
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
                assert_eq!((r.blend_space, r.composite_space, r.operator), (CASES[i].6, CASES[i].7, GimpOperator::ClipToBackdrop));
            }
            let LayerContent::BackedRaster(native) = &mapped.content else { panic!("paid native") };
            let rect = native.operative_bounds();
            let mut actual = Vec::new();
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    let mut pixel = [0; 16];
                    native.checked_view().read_pixel_native_into(x, y, &mut pixel, &c).unwrap();
                    actual.extend_from_slice(&pixel);
                }
            }
            assert_eq!(actual, original.plane.as_ref().unwrap().bytes);
        }
    }
}
#[test]
#[ignore = "UNRUN: strict native failure final, unsupported geometry/raw/profile/legacy codes never retry general"]
fn strict_refusals() {
    for sample in [0, 8, 16] {
        for case in 0..10 {
            let mut f = source(sample);
            match case {
                0 => f.layers[1].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&1.5f32.to_ne_bytes()),
                1 => f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
                2 => f.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&(-0.25f32).to_ne_bytes()),
                3 => {
                    f.layers[0].plane.as_mut().unwrap().bytes.pop();
                }
                4 => f.layers[0].offset = (0, 0),
                5 => f.layers[0].visible = false,
                6 => f.layers[0].mode = 5,
                7 => f.layers[0].blend_space = -99,
                8 => f.icc = vec![0; 132],
                _ => f.version = 23,
            }
            assert!(qualify_selected(&f, &Interrupt::NONE, &budget()).unwrap().is_none());
        }
    }
}
#[test]
#[ignore = "UNRUN: component/luma endpoints remain unit-only, paid work/cancellation and allNormal Plain priority"]
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
