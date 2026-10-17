//! UNRUN assigned F64 geometry controls; external captures are not corpus enrollment.
//! Straight RGB/effective-profile capture limitations remain distinct; no CMS/numeric acceptance claim.
//! Actual source/native/original ICC custody is tested without normalizing raw fields.
use super::*;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const PINS: [&str; 2] =
    ["8fca79d49c1993c2c9408b158f5d0767e41f1f48365631e2075b2e7c6e4f3ecb", "f8fa56b3de1c8a9a6d1339d4f937fb39233ffc9d939f7fa74b355d1113b27363"];
fn root() -> PathBuf {
    std::env::var_os("PHOTOCRAFT_ASSIGNED_F64_BOX_RESEARCH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol4-assigned-f64-box-gimp-20261016"))
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
fn bytes(case: usize) -> Vec<u8> {
    let b = std::fs::read(root().join("author-v1/outputs").join(format!("assigned-f64-hdr-nl-r{}.xcf", case + 4))).unwrap();
    assert_eq!(b.len(), 86036);
    assert_eq!(hash(&b), PINS[case]);
    b
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
        "Own assigned geometry Box.xcf",
        &mut Cursor::new(b),
        b.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &control(),
    )
}
#[test]
#[ignore = "UNRUN: two authored assigned F64 sources retain actual original ICC and native, same-q assigned class/title/opaque epoch"]
fn assigned_geometry_sources_retain_real_profile_and_native() {
    for case in 0..2 {
        let f = file(case);
        assert!(hint(&f));
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(NeighborhoodSourceFamily::hint(&f), Some(NeighborhoodSourceFamily::BoxGeometryAssignedF64));
        let q = quota();
        let crate::OwnedXcfImport::Point(source) = imported(&bytes(case), &q).unwrap() else { panic!("strict assigned Point") };
        let owner = source.document();
        owner.validate_readonly_assigned_point_storage(&control()).unwrap();
        assert!(owner.is_readonly_assigned_point_storage());
        assert!(!owner.is_readonly_point_storage());
        let d = owner.document();
        assert_eq!(d.depth, photocraft_doc::SampleType::F64);
        let profile = d.source_color_context.as_ref().unwrap();
        assert_eq!(profile.source_precision, 750);
        assert_eq!(profile.original_icc.as_ref().unwrap().as_slice(), f.icc.as_slice());
        assert_eq!(hash(profile.original_icc.as_ref().unwrap()), "abae4941f28e3558cb648775352364b2978e07a3b49e78228836b98a4ac05f46");
        let x = d.xcf_state.as_ref().unwrap();
        let node = &x.filter_stacks[0].nodes[0];
        assert!(
            matches!(node.arguments[0].value,photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Signed{width:32,value})if value==case as i64+4)
        );
        let policy = node.composition.as_ref().unwrap();
        assert_eq!(node.operation.schema_epoch, photocraft_compose::backed_box_source::BOX_GEOMETRY_EPOCH);
        assert_eq!(policy.compatibility_epoch, node.operation.schema_epoch);
        assert_eq!((policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator), (1654948433, 1654948433, 1654948433));
        assert!(policy.resolved.is_none());
        let photocraft_doc::LayerContent::BackedRaster(layer) = &d.layers[0].content else { panic!("actual native owner") };
        let rect = Rect::new(13, -9, 270, 250);
        assert_eq!(layer.operative_bounds(), rect);
        assert_eq!(layer.default_bytes().len(), 32);
        assert!(layer.default_bytes().iter().all(|v| *v == 0));
        let mut raw = vec![0u8; 257 * 259 * 32];
        for coord in rect.tiles() {
            let pin = layer.checked_view().pin_tile(coord, &control()).unwrap();
            let part = coord.rect().intersect(&rect);
            for y in part.y0..part.y1 {
                for x in part.x0..part.x1 {
                    let mut pixel = [0; 32];
                    pin.read_pixel_native_into(x, y, &mut pixel, &control()).unwrap();
                    let at = ((y - rect.y0) as usize * 257 + (x - rect.x0) as usize) * 32;
                    for (c, w) in pixel.as_chunks::<8>().0.iter().enumerate() {
                        raw[at + c * 8..at + c * 8 + 8].copy_from_slice(&u64::from_ne_bytes(*w).to_le_bytes());
                    }
                }
            }
        }
        assert_eq!(hash(&raw), "a72a5c127254c9c6310ca8d4da3c9e4b0b15db50d7602e6e6cd4fef6fda08cfe");
        let mask = &x.filter_masks[0];
        assert_eq!(mask.bounds, Rect::new(0, 0, 283, 281));
        assert_eq!(mask.surface.format().sample, photocraft_doc::SampleType::F64);
        assert_eq!(mask.surface.default_bytes().len(), 8);
        assert!(mask.surface.default_bytes().iter().all(|v| *v == 0));
        assert!(mask.is_empty_selection_with(&Interrupt::NONE).unwrap());
        let named = owner.try_with_document_name("Assigned observed.pcraft", &control()).unwrap();
        assert!(named.is_readonly_assigned_point_storage());
        assert_eq!(named.document().icc_profile, d.icc_profile);
        assert_eq!(named.document().source_color_context, d.source_color_context);
        assert_eq!(named.document().xcf_state, d.xcf_state);
    }
}

#[test]
#[ignore = "UNRUN: exactassignedF64role/crosspair/native32/mask8 unitAlpha/finiteRGB checks eveninactive, no narrowing"]
fn assigned_f64_role_and_wholeword_domain() {
    for failure in 0..9 {
        let mut f = file(0);
        f.layers[0].visible = false;
        f.layers[0].opacity = 0.0;
        f.layers[0].effects[0].state.enabled = Some(false);
        match failure {
            0 => f.precision = 650,
            1 => f.sample = Sample::F32,
            2 => f.layers[0].plane.as_mut().unwrap().bytes[..8].copy_from_slice(&f64::NAN.to_ne_bytes()),
            3 => f.layers[0].plane.as_mut().unwrap().bytes[..8].copy_from_slice(&f64::INFINITY.to_ne_bytes()),
            4 => f.layers[0].plane.as_mut().unwrap().bytes[24..32].copy_from_slice(&(1.0 + 2f64.powi(-40)).to_ne_bytes()),
            5 => f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[..8].copy_from_slice(&2f64.powi(-200).to_ne_bytes()),
            6 => f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().sample = Sample::F32,
            7 => {
                f.layers[0].plane.as_mut().unwrap().bytes.pop();
            }
            _ => f.icc.resize((16 << 20) + 1, 0),
        }
        assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap(), "fault{failure}");
    }
    let mut f = file(0);
    f.layers[0].plane.as_mut().unwrap().bytes[..8].copy_from_slice(&(1.0 + 2f64.powi(-40)).to_ne_bytes());
    assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    assert_eq!(f64::from_ne_bytes(f.layers[0].plane.as_ref().unwrap().bytes[..8].try_into().unwrap()).to_bits(), (1.0 + 2f64.powi(-40)).to_bits());
    let mut f = file(0);
    f.icc.clear();
    assert_eq!(NeighborhoodSourceFamily::hint(&f), Some(NeighborhoodSourceFamily::BoxGeometryF64));
}
#[test]
#[ignore = "UNRUN: existingobservedactivity/radiuspolicy explicitnewrole, opaque words retained, oldidentity remainsoldBoxFinite finalrefusal"]
fn observed_role_does_not_reclassify_old_assigned_sources() {
    for radius in [0, 1, 4, 5, 127, 1000] {
        let mut f = file(0);
        f.layers[0].visible = false;
        f.layers[0].opacity = 0.375;
        let s = &mut f.layers[0].effects[0].state;
        s.enabled = Some(false);
        s.blend_space = Some(i32::MIN);
        s.composite_space = Some(0);
        s.composite_mode = Some(i32::MAX);
        f.layers[0].effects[0].arguments[0].value = ArgumentValue::Integer(radius);
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(NeighborhoodSourceFamily::hint(&f), Some(NeighborhoodSourceFamily::BoxGeometryAssignedF64));
    }
    let mut f = file(0);
    f.width = 16;
    f.height = 12;
    f.layers[0].width = 16;
    f.layers[0].height = 12;
    f.layers[0].offset = (0, 0);
    let s = &mut f.layers[0].effects[0].state;
    s.blend_space = Some(-1474592175);
    s.composite_space = Some(-1474592175);
    s.composite_mode = Some(-1474592175);
    assert!(!hint(&f));
    let family = NeighborhoodSourceFamily::hint(&f).unwrap();
    assert_eq!(family, NeighborhoodSourceFamily::BoxFinite);
    assert!(!family.qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    // A source whose actual sample/precision is F32 still routes the old assigned
    // identity to its unchanged F32 source validator, never via this F64 variant.
    f.sample = Sample::F32;
    f.precision = 650;
    assert_eq!(NeighborhoodSourceFamily::hint(&f), Some(NeighborhoodSourceFamily::BoxAssignedF32));
}
#[test]
#[ignore = "UNRUN: malformedactualoriginalICC/missingactivation/wrongenvelope andsameqwork/cancel failwithoutimplicitretry"]
fn source_profile_and_controls_cannot_be_bypassed() {
    for fault in 0..4 {
        let mut f = file(0);
        match fault {
            0 => f.layers[0].effects[0].state.enabled = None,
            1 => f.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
            2 => f.layers[0].effects[0].state.mode = Some(28),
            _ => f.encoding = Encoding::Linear,
        }
        assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    }
    let f = file(0);
    let mut raw = bytes(0);
    let positions: Vec<_> = raw.windows(f.icc.len()).enumerate().filter_map(|(i, w)| (w == f.icc.as_slice()).then_some(i)).collect();
    assert_eq!(positions.len(), 1);
    raw[positions[0]..positions[0] + 4].copy_from_slice(&0u32.to_be_bytes());
    let q = quota();
    assert!(imported(&raw, &q).is_err());
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let low = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, Some(&low)).is_err());
    let stop = || true;
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&stop), Some(&budget())), Err(IoError::Cancelled)));
    // Real common-ledger exhaustion before public owned import, not a synthetic receipt.
    let q = quota();
    let blocker = q.reserve(photocraft_raster::backing::quota::QuotaKind::Index, 64 << 20).unwrap();
    assert!(imported(&bytes(0), &q).is_err());
    drop(blocker);
}
