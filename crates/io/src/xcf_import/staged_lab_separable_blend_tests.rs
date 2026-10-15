//! UNRUN exact original RGB LCh source boundary. Native/decoded buffers are
//! external harness data; no new numerical acceptance or producer grant.
use super::super::{SelectedStagedFamily, SourceSelectedPolicy, StagePolicy};
use super::*;
use photocraft_doc::import_compositing::{GimpOperator, GimpSpace};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: &[(i32, &str, &str, usize, (i32, i32, i32), bool, GimpSpace, GimpSpace, bool)] = &[
    (
        24,
        "composition-matrix/mode-lch-hue.xcf",
        "1a8e021a09d6e3c1c2ca6d011921d5a2b4220ef0ca63d59fa2ffd20b9a7e6226",
        5425,
        (-3, -1, -2),
        false,
        GimpSpace::Lab,
        GimpSpace::LinearRgb,
        false,
    ),
    (
        24,
        "composition-matrix/mode-lch-hue.edit.xcf",
        "db26f5a2f89d2b7c49796c42da1973392619b4ecc600205b560ea4ccdc6a9618",
        5425,
        (-3, -1, -2),
        false,
        GimpSpace::Lab,
        GimpSpace::LinearRgb,
        false,
    ),
    (
        25,
        "composition-matrix/mode-lch-chroma.xcf",
        "5f98eddc14e34fb9a23daed205c12b7f5acef7f02192bd3431196a4aacba7a1f",
        5425,
        (-3, -1, -2),
        false,
        GimpSpace::Lab,
        GimpSpace::LinearRgb,
        false,
    ),
    (
        25,
        "composition-matrix/mode-lch-chroma.edit.xcf",
        "c109c81bc3d6644ee4821f46aac2c174850e5205a4ec3fa166ef42f3473ad6a8",
        5425,
        (-3, -1, -2),
        false,
        GimpSpace::Lab,
        GimpSpace::LinearRgb,
        false,
    ),
    (
        26,
        "composition-matrix/mode-lch-color.xcf",
        "656e0ac4097e84fbb716046cf51a7a42fdc504d836c0f2306ebbde7c360b6230",
        5425,
        (-3, -1, -2),
        false,
        GimpSpace::Lab,
        GimpSpace::LinearRgb,
        false,
    ),
    (
        26,
        "composition-matrix/mode-lch-color.edit.xcf",
        "31d3339f01dcd990a3c1fdcdf3dfc0b41e4f4cee582f13d59140c56b01e3448c",
        5425,
        (-3, -1, -2),
        false,
        GimpSpace::Lab,
        GimpSpace::LinearRgb,
        false,
    ),
    (
        27,
        "composition-matrix/mode-lch-lightness.xcf",
        "7b87e8941d447077f6a9b8c568051bb43efd0491da8b76e88cacac16d4dcd9aa",
        5425,
        (-3, -1, -2),
        false,
        GimpSpace::Lab,
        GimpSpace::LinearRgb,
        false,
    ),
    (
        27,
        "composition-matrix/mode-lch-lightness.edit.xcf",
        "617e6fad645831455a3832b6e97a36c44fc4f1269052c5f7596648b7cb64eaf4",
        5425,
        (-3, -1, -2),
        false,
        GimpSpace::Lab,
        GimpSpace::LinearRgb,
        false,
    ),
];
fn expected(_: usize) -> SourceKind {
    SourceKind::LabStrict
}
fn operator(_: usize) -> GimpOperator {
    GimpOperator::ClipToBackdrop
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
#[ignore = "UNRUN: all8 genuine source/edit packets retain exact old metadata and independent RGB LCh raw policy"]
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
        let r = photocraft_doc::import_compositing::flat_lab_source_rule(12, CASES[i].0, b, c, o).unwrap().resolved();
        assert_eq!((r.blend_space, r.composite_space, r.operator), (CASES[i].6, CASES[i].7, operator(i)));
    }
}
#[test]
#[ignore = "UNRUN: all8 single selected decode promotes actual paid native/class with original raw epochs, not eager or Plain fallback"]
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
        let crate::OwnedXcfImport::SeparableBlend(imported) = result else { panic!("RGB LCh strict route") };
        let owner = imported.document();
        assert!(owner.is_readonly_flat_composite_storage());
        let d = owner.document();
        assert_eq!(d.mode.color_channels(), 3);
        assert!(matches!(&d.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. }));
        for (mapped, original) in d.layers.iter().zip(eager.layers.iter().rev()) {
            let row = d.xcf_state.as_ref().unwrap().items.iter().find(|r| r.item == ItemRef::Layer(mapped.id)).unwrap();
            let p = row.composition.as_ref().unwrap();
            assert_eq!(
                (p.source_mode, p.stored_blend_space, p.stored_composite_space, p.stored_operator),
                (original.mode, original.blend_space, original.composite_space, original.composite_mode)
            );
            let r = p.resolved.as_ref().unwrap();
            if original.mode != 28 {
                assert!(mapped.import_compositing.is_none());
            }
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
#[ignore = "UNRUN: RGB descriptors/channel counts, explicit Gray refusal and both full native planes remain checked; no source retry"]
fn strict_refusals() {
    for i in [0, 6] {
        for case in 0..12 {
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
                10 => {
                    f.model = Model::Gray;
                    for d in &mut f.layers {
                        d.model = Model::Gray;
                    }
                }
                _ => f.version = 23,
            }
            assert!(qualify_selected(&f, &Interrupt::NONE, &budget()).unwrap().is_none());
        }
    }
}
#[test]
#[ignore = "UNRUN: RGB LCh endpoints remain unit-only, paid work/cancellation and allNormal Plain priority"]
fn endpoints_control_and_plain_priority() {
    for i in 0..CASES.len() {
        for v in [0f32, 1f32] {
            let mut f = source(i);
            f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&v.to_ne_bytes());
            assert_eq!(qualify_selected(&f, &Interrupt::NONE, &budget()).unwrap(), Some(expected(i)));
        }
    }
    let mut f = source(0);
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
