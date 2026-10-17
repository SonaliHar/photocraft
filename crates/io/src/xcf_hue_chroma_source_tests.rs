//! UNRUN authored RGB Hue-Chroma source lifecycle; FMA/Babl numerical schedule remains HOLD.
use super::*;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: [(&str, usize, &str, bool, [u32; 3]); 12] = [
    (
        "f32-author-v1/outputs/f32-neutral-to-source.source.xcf",
        1981,
        "0f63702cdd76521303d869ca8ac952103c75a6abf69961555201dc635c615edf",
        false,
        [0x00000000, 0x00000000, 0x00000000],
    ),
    (
        "f32-author-v1/outputs/f32-neutral-to-source.edit.xcf",
        1981,
        "9ff2a32b41c705b766789128dc615345b27c332c20aeafa092da5e578d4f55c9",
        false,
        [0x42160000, 0xc1480000, 0x41040000],
    ),
    (
        "f32-author-v1/outputs/f32-hue-rounding.source.xcf",
        1981,
        "8d415ba95da8b234895f26ed3e43013491807bdc71131b97705f689837ee7b50",
        false,
        [0x42160000, 0xc1480000, 0x41040000],
    ),
    (
        "f32-author-v1/outputs/f32-hue-rounding.edit.xcf",
        1981,
        "7adac66d030a51866ea0055c56fc2e6c25c79278677cc752526fe93eefc7f1a4",
        false,
        [0xc2868000, 0xc1480000, 0x41040000],
    ),
    (
        "f32-author-v1/outputs/f32-bounded-extremes.source.xcf",
        1981,
        "b59dcc15af9cf3eb1c5c8a1e85da8d7db24804ea72d055507cf064e7a6156180",
        false,
        [0xc3340000, 0xc2c80000, 0xc2c80000],
    ),
    (
        "f32-author-v1/outputs/f32-bounded-extremes.edit.xcf",
        1981,
        "6bb46b5917356fc9cc897a681da0f35194d9e332e8e2560e48b6cea3ae4a8555",
        false,
        [0x43340000, 0x42c80000, 0x42c80000],
    ),
    (
        "f64-author-v1/outputs/f64-neutral-to-source.source.xcf",
        2109,
        "3514e59d95223345ec4dc73d0c53374af13e25a92ff6b4277a97284528b574f9",
        true,
        [0x00000000, 0x00000000, 0x00000000],
    ),
    (
        "f64-author-v1/outputs/f64-neutral-to-source.edit.xcf",
        2109,
        "527f53d962be754627f8f39675dc019cbeba80868546d3ce980fc3cd5d52fe14",
        true,
        [0x42160000, 0xc1480000, 0x41040000],
    ),
    (
        "f64-author-v1/outputs/f64-hue-rounding.source.xcf",
        2109,
        "d0002164f32bf0b4d294a58b4704d9dfea9218de14e1c1e403e15df3974cae36",
        true,
        [0x42160000, 0xc1480000, 0x41040000],
    ),
    (
        "f64-author-v1/outputs/f64-hue-rounding.edit.xcf",
        2109,
        "723440020b7d91f7785651c5c347514b0af22410ac133e311f0b022a1f6a3b28",
        true,
        [0xc2868000, 0xc1480000, 0x41040000],
    ),
    (
        "f64-author-v1/outputs/f64-bounded-extremes.source.xcf",
        2109,
        "ecbc1e085a72cfc2eef1ccbc4a6d1738727ef68345aa8a80b55fd2db163adadf",
        true,
        [0xc3340000, 0xc2c80000, 0xc2c80000],
    ),
    (
        "f64-author-v1/outputs/f64-bounded-extremes.edit.xcf",
        2109,
        "66798160686b0a6936e752c195f931971e4f3886034c13219efb8d5351bdb992",
        true,
        [0x43340000, 0x42c80000, 0x42c80000],
    ),
];

fn bytes(case: usize) -> Vec<u8> {
    let root = std::env::var_os("PHOTOCRAFT_HUE_CHROMA_RESEARCH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-hue-chroma-owned-gimp-20261016"));
    let b = std::fs::read(root.join(CASES[case].0)).unwrap();
    assert_eq!(b.len(), CASES[case].1);
    assert_eq!(Sha256::digest(&b).iter().map(|v| format!("{v:02x}")).collect::<String>(), CASES[case].2);
    b
}
fn file(case: usize) -> XcfFile {
    photocraft_xcf::decode(&bytes(case), &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new_persistent(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None).unwrap()
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 32 << 20, scratch: 0 })
}
fn import(b: &[u8], q: &BackingQuota) -> Result<crate::OwnedXcfImport, IoError> {
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 16 << 20), ("scratchBytes", 0)])
        .unwrap();
    crate::import_owned_xcf_from_reader_in_quota("Own Hue.xcf", &mut Cursor::new(b), b.len() as u64, &Default::default(), &limits, q.clone(), &ctl())
}
#[test]
#[ignore = "UNRUN: twelve authored sources retain threeF32 args/order/native words, exactprofile/mask/raw andtitle custody; no math claim"]
fn actual_sources_keep_three_stored_words() {
    for (case, case_row) in CASES.iter().enumerate() {
        let f = file(case);
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(parameters(&f.layers[0].effects[0]).unwrap(), case_row.4);
        let q = quota();
        let crate::OwnedXcfImport::Point(source) = import(&bytes(case), &q).unwrap() else { panic!("Point") };
        let owner = source.document();
        owner.validate_readonly_point_storage(&ctl()).unwrap();
        let d = owner.document();
        assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
        assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, if case_row.3 { 750 } else { 650 });
        let x = d.xcf_state.as_ref().unwrap();
        let n = &x.filter_stacks[0].nodes[0];
        assert_eq!(n.operation.schema_epoch, photocraft_compose::HUE_CHROMA_EPOCH);
        assert_eq!(n.arguments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["hue", "chroma", "lightness"]);
        let bits = n
            .arguments
            .iter()
            .map(|a| match &a.value {
                photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(v)) => *v,
                _ => panic!("F32"),
            })
            .collect::<Vec<_>>();
        assert_eq!(bits.as_slice(), case_row.4.as_slice());
        let p = n.composition.as_ref().unwrap();
        assert_eq!(p.compatibility_epoch, n.operation.schema_epoch);
        assert!(p.resolved.is_none());
        assert_eq!((p.stored_blend_space, p.stored_composite_space, p.stored_operator), (720732753, 720732753, 720732753));
        let photocraft_doc::LayerContent::BackedRaster(layer) = &d.layers[0].content else { panic!("native") };
        let bpp = if case_row.3 { 32 } else { 16 };
        let r = photocraft_geom::Rect::new(0, 0, 8, 6);
        let pin = layer.checked_view().pin_tile(photocraft_geom::TileCoord::new(0, 0), &ctl()).unwrap();
        let mut raw = vec![0; 48 * bpp];
        for y in 0..6 {
            for xx in 0..8 {
                let at = (y * 8 + xx) as usize * bpp;
                pin.read_pixel_native_into(xx, y, &mut raw[at..at + bpp], &ctl()).unwrap();
            }
        }
        assert_eq!(raw, f.layers[0].plane.as_ref().unwrap().bytes);
        assert_eq!(layer.operative_bounds(), r);
        assert_eq!(layer.default_bytes().len(), bpp);
        assert!(layer.default_bytes().iter().all(|v| *v == 0));
        assert_eq!(x.filter_masks[0].bounds, r);
        assert_eq!(x.filter_masks[0].surface.default_bytes().len(), bpp / 4);
        assert!(x.filter_masks[0].is_empty_selection_with(&Interrupt::NONE).unwrap());
        let title = owner.try_with_document_name("Hue.pcraft", &ctl()).unwrap();
        assert_eq!(title.document().xcf_state, d.xcf_state);
        assert_eq!(title.document().source_color_context, d.source_color_context);
        assert_eq!(title.document().icc_profile, d.icc_profile);
    }
}
#[test]
#[ignore = "UNRUN: source roles/geometry/activity/threeF32 names-types-ranges/rawmissing/native/mask fail closed with final family refusal"]
fn exact_source_refusals() {
    for case in [0, 6] {
        for bad in 0..18 {
            let mut f = file(case);
            match bad {
                0 => f.model = Model::Gray,
                1 => f.icc = vec![0; 640],
                2 => f.width = 9,
                3 => f.layers[0].visible = false,
                4 => f.layers[0].opacity = 0.5,
                5 => f.layers[0].effects[0].state.enabled = Some(false),
                6 => f.layers[0].effects[0].state.blend_space = None,
                7 => f.layers[0].effects[0].state.mode = Some(28),
                8 => f.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
                9 => f.layers[0].effects[0].state.region = Some(1),
                10 => f.layers[0].effects[0].arguments[0].value = ArgumentValue::Float64Bits(0),
                11 => f.layers[0].effects[0].arguments[0].value = ArgumentValue::Float32Bits(180.01f32.to_bits()),
                12 => f.layers[0].effects[0].arguments[1].value = ArgumentValue::Float32Bits((-100.01f32).to_bits()),
                13 => f.layers[0].effects[0].arguments[2].name = "hue".into(),
                14 => {
                    f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1;
                }
                15 => {
                    if CASES[case].3 {
                        f.layers[0].plane.as_mut().unwrap().bytes[..8].copy_from_slice(&1.01f64.to_ne_bytes())
                    } else {
                        f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes())
                    }
                }
                16 => {
                    f.sample = Sample::U8;
                    f.precision = 150;
                }
                _ => f.layers[0].effects[0].properties[0].tag = 99,
            };
            assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap(), "{case}/{bad}");
            assert!(matches!(crate::xcf_point_replace_source::classify_family_hint(&f), Some(crate::xcf_filters::PointReplaceFamily::HueChroma)));
            assert!(crate::xcf_point_replace_source::classify(&f, &Interrupt::NONE, Some(&budget())).unwrap().is_none());
        }
    }
}
fn raw_words(mut b: Vec<u8>, values: [i32; 3]) -> Vec<u8> {
    for (tag, value) in [37u32, 36, 35].into_iter().zip(values) {
        let mut pattern = Vec::new();
        pattern.extend(tag.to_be_bytes());
        pattern.extend(4u32.to_be_bytes());
        pattern.extend([0x2a, 0xf5, 0x82, 0x51]);
        let positions = b.windows(12).enumerate().filter_map(|(i, w)| (w == pattern.as_slice()).then_some(i)).collect::<Vec<_>>();
        assert_eq!(positions.len(), 1);
        let at = positions[0] + 8;
        b[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }
    b
}
#[test]
#[ignore = "UNRUN: separate opaque provenance policy retains mixedMIN/0/MAX only inside exact Hue-Chroma Replace envelope; no enum alias"]
fn opaque_words_are_custody_not_other_policy() {
    for case in [0, 6] {
        let mut f = file(case);
        let mask = f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap();
        if CASES[case].3 {
            mask.bytes[..8].copy_from_slice(&(-0.0f64).to_ne_bytes())
        } else {
            mask.bytes[..4].copy_from_slice(&(-0.0f32).to_ne_bytes())
        };
        let raw = mask.bytes.clone();
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(f.layers[0].effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap().bytes, raw);
    }

    let values = [i32::MIN, 0, i32::MAX];
    for case in [0, 6] {
        let b = raw_words(bytes(case), values);
        let q = quota();
        let crate::OwnedXcfImport::Point(source) = import(&b, &q).unwrap() else { panic!("Point") };
        let n = &source.document().document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let p = n.composition.as_ref().unwrap();
        assert_eq!((p.stored_blend_space, p.stored_composite_space, p.stored_operator), (values[0], values[1], values[2]));
        assert!(p.resolved.is_none());
        let mut f = photocraft_xcf::decode(&b, &Default::default()).unwrap();
        f.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits());
        assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    }
}
#[test]
#[ignore = "UNRUN: source shared work/cancellation/native length/quota refusals retain original ownership without fallback"]
fn resource_and_length_refusals() {
    let f = file(0);
    let small = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(small, None).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, Some(&b)).is_err());
    let stop = || true;
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&stop), Some(&budget())), Err(IoError::Cancelled)));
    let mut f = f;
    f.layers[0].plane.as_mut().unwrap().bytes.pop();
    assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    let q = quota();
    let hold = q.reserve(QuotaKind::Index, 32 << 20).unwrap();
    assert!(import(&bytes(0), &q).is_err());
    drop(hold);
}
