//! UNRUN assigned F32 geometry controls; external captures are not corpus enrollment.
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
    ["74bbd0fc33951f617c11bce5fb8613243ae1595a2c5d849759a5f889769c654a", "909efe8d36855c9edbf4f72e7c9e3428e7ba213c6a5dd43c6988e157821cb98c"];
fn root() -> PathBuf {
    std::env::var_os("PHOTOCRAFT_BOX_WIDE_GEOMETRY_RESEARCH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-box-wide-geometry-gimp-20261016"))
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
fn bytes(case: usize) -> Vec<u8> {
    let b = std::fs::read(root().join("author-v1/outputs").join(format!("assigned-f32-nl-r{}.xcf", case + 4))).unwrap();
    assert_eq!(b.len(), 89894);
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
#[ignore = "UNRUN: two authored assigned F32 sources retain actual original ICC and native, same-q assigned class/title/opaque epoch"]
fn assigned_geometry_sources_retain_real_profile_and_native() {
    for case in 0..2 {
        let f = file(case);
        assert!(hint(&f));
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(NeighborhoodSourceFamily::hint(&f), Some(NeighborhoodSourceFamily::BoxGeometryAssignedF32));
        let q = quota();
        let crate::OwnedXcfImport::Point(source) = imported(&bytes(case), &q).unwrap() else { panic!("strict assigned Point") };
        let owner = source.document();
        owner.validate_readonly_assigned_point_storage(&control()).unwrap();
        assert!(owner.is_readonly_assigned_point_storage());
        assert!(!owner.is_readonly_point_storage());
        let d = owner.document();
        assert_eq!(d.depth, photocraft_doc::SampleType::F32);
        let profile = d.source_color_context.as_ref().unwrap();
        assert_eq!(profile.source_precision, 650);
        assert_eq!(profile.original_icc.as_ref().unwrap().as_slice(), f.icc.as_slice());
        assert_eq!(hash(profile.original_icc.as_ref().unwrap()), "abae4941f28e3558cb648775352364b2978e07a3b49e78228836b98a4ac05f46");
        let x = d.xcf_state.as_ref().unwrap();
        let node = &x.filter_stacks[0].nodes[0];
        let policy = node.composition.as_ref().unwrap();
        assert_eq!(node.operation.schema_epoch, photocraft_compose::backed_box_source::BOX_GEOMETRY_EPOCH);
        assert_eq!(policy.compatibility_epoch, node.operation.schema_epoch);
        assert_eq!((policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator), (1654948433, 1654948433, 1654948433));
        assert!(policy.resolved.is_none());
        let photocraft_doc::LayerContent::BackedRaster(layer) = &d.layers[0].content else { panic!("actual native owner") };
        let rect = Rect::new(13, -9, 270, 250);
        assert_eq!(layer.operative_bounds(), rect);
        assert_eq!(layer.default_bytes().len(), 16);
        let mut raw = vec![0u8; 257 * 259 * 16];
        for coord in rect.tiles() {
            let pin = layer.checked_view().pin_tile(coord, &control()).unwrap();
            let part = coord.rect().intersect(&rect);
            for y in part.y0..part.y1 {
                for x in part.x0..part.x1 {
                    let mut pixel = [0; 16];
                    pin.read_pixel_native_into(x, y, &mut pixel, &control()).unwrap();
                    let at = ((y - rect.y0) as usize * 257 + (x - rect.x0) as usize) * 16;
                    for (c, w) in pixel.chunks_exact(4).enumerate() {
                        raw[at + c * 4..at + c * 4 + 4].copy_from_slice(&u32::from_ne_bytes(w.try_into().unwrap()).to_le_bytes());
                    }
                }
            }
        }
        assert_eq!(hash(&raw), "72fc0fa6db04a4c04240d9db9e270517b5f907a6dae4e681c05173fe8567ec2e");
        let mask = &x.filter_masks[0];
        assert_eq!(mask.bounds, Rect::new(0, 0, 283, 281));
        assert_eq!(mask.surface.format().sample, photocraft_doc::SampleType::F32);
        assert_eq!(mask.surface.default_bytes().len(), 4);
        assert!(mask.is_empty_selection_with(&Interrupt::NONE).unwrap());
        let named = owner.try_with_document_name("Assigned observed.pcraft", &control()).unwrap();
        assert!(named.is_readonly_assigned_point_storage());
        assert_eq!(named.document().icc_profile, d.icc_profile);
        assert_eq!(named.document().source_color_context, d.source_color_context);
        assert_eq!(named.document().xcf_state, d.xcf_state);
    }
}

#[test]
#[ignore = "UNRUN: assigned role/domain/profile failures, opaque words, old assigned identity final refusal, cumulative work/cancel"]
fn assigned_geometry_role_and_control_refusals() {
    for case in 0..9 {
        let mut f = file(0);
        match case {
            0 => f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
            1 => f.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&1.25f32.to_ne_bytes()),
            2 => f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[..4].copy_from_slice(&0.5f32.to_ne_bytes()),
            3 => f.sample = Sample::F64,
            4 => f.layers[0].effects[0].state.enabled = Some(false),
            5 => f.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
            6 => f.layers[0].effects[0].arguments[0].value = ArgumentValue::Integer(6),
            7 => f.icc.resize((16 << 20) + 1, 0),
            _ => f.encoding = Encoding::Linear,
        }
        assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap(), "case{case}");
    }
    // Opaque provenance never becomes a resolved-space alias.
    let mut f = file(0);
    let state = &mut f.layers[0].effects[0].state;
    state.blend_space = Some(i32::MIN);
    state.composite_space = Some(0);
    state.composite_mode = Some(i32::MAX);
    assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    let mut old = file(0);
    old.width = 16;
    old.height = 12;
    old.layers[0].width = 16;
    old.layers[0].height = 12;
    old.layers[0].offset = (0, 0);
    let state = &mut old.layers[0].effects[0].state;
    state.blend_space = Some(-1474592175);
    state.composite_space = Some(-1474592175);
    state.composite_mode = Some(-1474592175);
    old.layers[0].visible = false;
    assert!(!hint(&old));
    let family = NeighborhoodSourceFamily::hint(&old).unwrap();
    assert_eq!(family, NeighborhoodSourceFamily::BoxAssignedF32);
    assert!(!family.qualify(&old, &Interrupt::NONE, Some(&budget())).unwrap());
    // Corrupt the actual embedded profile in a same-length external source copy.
    // Selection stays assigned; the controlled profile mapper must fail without implicit retry.
    let f = file(0);
    let mut raw = bytes(0);
    let positions: Vec<_> = raw.windows(f.icc.len()).enumerate().filter_map(|(i, w)| (w == f.icc.as_slice()).then_some(i)).collect();
    assert_eq!(positions.len(), 1);
    raw[positions[0]..positions[0] + 4].copy_from_slice(&0u32.to_be_bytes());
    let q = quota();
    assert!(imported(&raw, &q).is_err());
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, Some(&b)).is_err());
    let stop = || true;
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&stop), Some(&budget())), Err(IoError::Cancelled)));
}
