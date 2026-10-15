//! UNRUN independent Perceptual source controls. Capture profile dates remain distinct HOLD;
//! these lifecycle checks establish no PhotoCraft/GIMP numerical equivalence or enrollment.
use super::*;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const PINS: [&str; 4] = [
    "d0c36e08f741d46c37135d77b7fe8bcdd9c04379900d7d7aab75657d1c78f519",
    "723d0836f83df8a64080c0ca7a7b63b605a573ee8d4af3c5a1bd3a0ec9d3ac89",
    "af6d52a942939d69dab552ad793f85d320a9a3c9a1ce3f9a0f1f1ee94bdaabb6",
    "2138b14e45f7fd945630b3a359e12d57f539ccd810375b5de1b42a837eae7e0e",
];
fn bytes(case: usize) -> Vec<u8> {
    let root = std::env::var_os("PHOTOCRAFT_PERCEPTUAL_BOX_RESEARCH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol4-perceptual-box-gimp-20261016"));
    let role = if case < 2 { "f32" } else { "f64" };
    let radius = 4 + case % 2;
    let b = std::fs::read(root.join(format!("{role}-author-v1/outputs/implicit-{role}-perceptual-hdr-r{radius}.xcf"))).unwrap();
    assert_eq!(b.len(), if case < 2 { 421309 } else { 604315 });
    assert_eq!(hash(&b), PINS[case]);
    b
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
fn file(case: usize) -> XcfFile {
    photocraft_xcf::decode(&bytes(case), &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new_persistent(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None).unwrap()
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 })
}
fn imported(b: &[u8], q: &BackingQuota) -> Result<crate::OwnedXcfImport, IoError> {
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
        .unwrap();
    crate::import_owned_xcf_from_reader_in_quota(
        "Own Perceptual Box.xcf",
        &mut Cursor::new(b),
        b.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &control(),
    )
}
#[test]
#[ignore = "UNRUN: all four authored Perceptual originals retain exact whole native words/mask depth/generated current ICC and implicit title custody"]
fn genuine_perceptual_sources_preserve_roles_and_native_words() {
    for case in 0..4 {
        let f = file(case);
        let q = quota();
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(
            NeighborhoodSourceFamily::hint(&f),
            Some(if case < 2 { NeighborhoodSourceFamily::BoxGeometryPerceptualF32 } else { NeighborhoodSourceFamily::BoxGeometryPerceptualF64 })
        );
        let crate::OwnedXcfImport::Point(source) = imported(&bytes(case), &q).unwrap() else { panic!("Point") };
        let owner = source.document();
        assert!(owner.is_readonly_point_storage());
        assert!(!owner.is_readonly_assigned_point_storage());
        owner.validate_readonly_point_storage(&control()).unwrap();
        let d = owner.document();
        assert_eq!(d.sample_encoding, Some(photocraft_doc::SampleEncoding::Perceptual));
        let source_context = d.source_color_context.as_ref().unwrap();
        assert!(source_context.original_icc.is_none());
        assert_eq!(source_context.source_precision, if case < 2 { 675 } else { 775 });
        let expected = photocraft_cms::xcf_color::effective_profile(None, photocraft_cms::ColorSpace::Rgb, photocraft_cms::xcf_color::Encoding::Perceptual)
            .unwrap()
            .to_bytes();
        assert_eq!(d.icc_profile.as_ref().unwrap().as_slice(), expected.as_slice());
        assert_ne!(expected, photocraft_cms::Builtin::Srgb.profile().to_bytes());
        assert_eq!(d.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::implicit_srgb(expected.as_slice()));
        let x = d.xcf_state.as_ref().unwrap();
        let n = &x.filter_stacks[0].nodes[0];
        let policy = n.composition.as_ref().unwrap();
        assert_eq!(n.operation.schema_epoch, photocraft_compose::backed_box_source::BOX_GEOMETRY_EPOCH);
        assert!(policy.resolved.is_none());
        assert_eq!((policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator), (0x30038251, 0x30038251, 0x30038251));
        let photocraft_doc::LayerContent::BackedRaster(layer) = &d.layers[0].content else { panic!("native") };
        let src = &f.layers[0];
        let r = layer.operative_bounds();
        assert_eq!(r, Rect::new(src.offset.0, src.offset.1, src.offset.0 + src.width as i32, src.offset.1 + src.height as i32));
        let word = if case < 2 { 4 } else { 8 };
        let bpp = word * 4;
        assert_eq!(layer.default_bytes().len(), bpp);
        assert!(layer.default_bytes().iter().all(|v| *v == 0));
        let mut actual = vec![0; src.width as usize * src.height as usize * bpp];
        for coord in r.tiles() {
            let pin = layer.checked_view().pin_tile(coord, &control()).unwrap();
            let part = coord.rect().intersect(&r);
            for y in part.y0..part.y1 {
                for xx in part.x0..part.x1 {
                    let at = ((y - r.y0) as usize * src.width as usize + (xx - r.x0) as usize) * bpp;
                    pin.read_pixel_native_into(xx, y, &mut actual[at..at + bpp], &control()).unwrap();
                }
            }
        }
        assert_eq!(actual, src.plane.as_ref().unwrap().bytes);
        let m = &x.filter_masks[0];
        assert_eq!(m.surface.default_bytes().len(), word);
        assert!(m.surface.default_bytes().iter().all(|v| *v == 0));
        assert!(m.is_empty_selection_with(&Interrupt::NONE).unwrap());
        assert_eq!(m.bounds, Rect::new(0, 0, f.width as i32, f.height as i32));
        let named = owner.try_with_document_name("Perceptual.pcraft", &control()).unwrap();
        assert!(named.is_readonly_point_storage());
        assert_eq!(named.document().source_color_context, d.source_color_context);
        assert_eq!(named.document().icc_profile, d.icc_profile);
        assert_eq!(named.document().xcf_state, d.xcf_state);
    }
}
#[test]
#[ignore = "UNRUN: exact implicit Perceptual pairs; hidden zero opacity disabled still scan all native/mask words without narrowing"]
fn role_and_inactive_word_refusals() {
    work_cancel_and_quota_failures();
    for case in [0, 2] {
        for fault in 0..9 {
            let mut f = file(case);
            f.layers[0].visible = false;
            f.layers[0].opacity = 0.;
            f.layers[0].effects[0].state.enabled = Some(false);
            match fault {
                0 => f.icc = vec![0; 640],
                1 => f.precision = if case == 0 { 775 } else { 675 },
                2 => f.encoding = Encoding::Linear,
                3 => f.model = Model::Gray,
                4 => {
                    let p = f.layers[0].plane.as_mut().unwrap();
                    if case == 0 { p.bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()) } else { p.bytes[..8].copy_from_slice(&f64::INFINITY.to_ne_bytes()) }
                }
                5 => {
                    let p = f.layers[0].plane.as_mut().unwrap();
                    if case == 0 {
                        p.bytes[12..16].copy_from_slice(&1.01f32.to_ne_bytes())
                    } else {
                        p.bytes[24..32].copy_from_slice(&(1. + 2f64.powi(-40)).to_ne_bytes())
                    }
                }
                6 => {
                    let p = f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap();
                    if case == 0 {
                        p.bytes[..4].copy_from_slice(&f32::MIN_POSITIVE.to_ne_bytes())
                    } else {
                        p.bytes[..8].copy_from_slice(&2f64.powi(-200).to_ne_bytes())
                    }
                }
                7 => {
                    f.layers[0].plane.as_mut().unwrap().bytes.pop();
                }
                _ => f.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
            }
            assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap(), "{case}/{fault}");
        }
    }
}
#[test]
#[ignore = "UNRUN: old identity selection is final and never retried as observed; opaque raw/activity cross-product remains unchanged"]
fn old_identity_and_opaque_policy() {
    for case in [0, 2] {
        let mut f = file(case);
        f.width = 16;
        f.height = 12;
        f.layers[0].width = 16;
        f.layers[0].height = 12;
        f.layers[0].offset = (0, 0);
        let e = &mut f.layers[0].effects[0].state;
        e.blend_space = Some(-1474592175);
        e.composite_space = Some(-1474592175);
        e.composite_mode = Some(-1474592175);
        assert!(!hint(&f));
        assert_eq!(NeighborhoodSourceFamily::hint(&f), Some(NeighborhoodSourceFamily::BoxOriginal));
        assert!(!NeighborhoodSourceFamily::BoxOriginal.qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        let mut f = file(case);
        let e = &mut f.layers[0].effects[0].state;
        e.blend_space = Some(i32::MIN);
        e.composite_space = Some(0);
        e.composite_mode = Some(i32::MAX);
        e.enabled = Some(false);
        f.layers[0].visible = false;
        f.layers[0].opacity = 0.375;
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    }
}
fn work_cancel_and_quota_failures() {
    let f = file(0);
    let small = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(small, None).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, Some(&b)).is_err());
    let stop = || true;
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&stop), Some(&budget())), Err(IoError::Cancelled)));
    let q = quota();
    let hold = q.reserve(QuotaKind::Index, 64 << 20).unwrap();
    assert!(imported(&bytes(0), &q).is_err());
    drop(hold);
}
