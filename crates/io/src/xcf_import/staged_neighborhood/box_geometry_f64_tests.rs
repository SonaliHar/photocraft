//! UNRUN genuine local F64 Box source controls; external captures are not corpus enrollment.
//! Producer effectiveICC header bytes33/35 differ across process and remain unnormalized HOLD.
//! Assigned F32 is a separate source role; straight-RGB/effectiveICC capture limitations remain distinct.
use super::*;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const PINS: [&str; 2] =
    ["a787c697cb537f30f226f5dada3cd3edc9f0c649d897610269b84ae183bf14c0", "c47607b2368b217c40b48e402cc4ebcce91c8c75159782b5e09b070731a0d58a"];
fn root() -> PathBuf {
    std::env::var_os("PHOTOCRAFT_BOX_WIDE_GEOMETRY_RESEARCH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-box-wide-geometry-gimp-20261016"))
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
fn bytes(case: usize) -> Vec<u8> {
    let b = std::fs::read(root().join("author-v1/outputs").join(format!("implicit-f64-nl-r{}.xcf", case + 4))).unwrap();
    assert_eq!(b.len(), 85370);
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
    crate::import_owned_xcf_from_reader_in_quota("Own F64 Box.xcf", &mut Cursor::new(b), b.len() as u64, &Default::default(), &limits, q.clone(), &control())
}
#[test]
#[ignore = "UNRUN: two independently authored F64NL750 sources preserve8byte native andcanvasmask, actualimplicitstorage/profile role andopaque epoch"]
fn genuine_f64_sources_preserve_actual_word_precision() {
    for case in 0..2 {
        let f = file(case);
        assert!(hint(&f));
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(NeighborhoodSourceFamily::hint(&f), Some(NeighborhoodSourceFamily::BoxGeometryF64));
        let q = quota();
        let crate::OwnedXcfImport::Point(source) = imported(&bytes(case), &q).unwrap() else { panic!("strict F64 Point") };
        let owner = source.document();
        owner.validate_readonly_point_storage(&control()).unwrap();
        assert!(!owner.is_readonly_assigned_point_storage());
        let d = owner.document();
        assert_eq!(d.depth, photocraft_doc::SampleType::F64);
        let profile = d.source_color_context.as_ref().unwrap();
        assert_eq!(profile.source_precision, 750);
        assert!(profile.original_icc.is_none());
        let x = d.xcf_state.as_ref().unwrap();
        let node = &x.filter_stacks[0].nodes[0];
        let policy = node.composition.as_ref().unwrap();
        assert_eq!(node.operation.schema_epoch, photocraft_compose::backed_box_source::BOX_GEOMETRY_EPOCH);
        assert_eq!(policy.compatibility_epoch, node.operation.schema_epoch);
        assert_eq!((policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator), (1654948433, 1654948433, 1654948433));
        assert!(policy.resolved.is_none());
        let photocraft_doc::LayerContent::BackedRaster(layer) = &d.layers[0].content else { panic!("actual native owner") };
        let rect = Rect::new(-7, 11, 250, 270);
        assert_eq!(layer.operative_bounds(), rect);
        assert_eq!(layer.default_bytes().len(), 32);
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
        assert_eq!(hash(&raw), "beb9953b2a67867b3f754c383bc8a57ef8a762ce79e44a8950162902f7a84d1f");
        let mask = &x.filter_masks[0];
        assert_eq!(mask.bounds, Rect::new(0, 0, 283, 281));
        assert_eq!(mask.surface.format().sample, photocraft_doc::SampleType::F64);
        assert_eq!(mask.surface.default_bytes().len(), 8);
        assert!(mask.is_empty_selection_with(&Interrupt::NONE).unwrap());
        let named = owner.try_with_document_name("F64 observed.pcraft", &control()).unwrap();
        assert!(named.is_readonly_point_storage());
        assert_eq!(named.document().icc_profile, d.icc_profile);
        assert_eq!(named.document().source_color_context, d.source_color_context);
        assert_eq!(named.document().xcf_state, d.xcf_state);
    }
}
#[test]
#[ignore = "UNRUN: F64 nonfinite/sub-F32unit violation/mask tinywords andcross-role refusals, oldidentityfinalfailure, realwork/cancel"]
fn f64_precision_controls_and_assigned_role() {
    for case in 0..14 {
        let mut f = file(0);
        match case {
            0 => f.layers[0].plane.as_mut().unwrap().bytes[..8].copy_from_slice(&f64::NAN.to_ne_bytes()),
            1 => f.layers[0].plane.as_mut().unwrap().bytes[..8].copy_from_slice(&f64::INFINITY.to_ne_bytes()),
            2 => f.layers[0].plane.as_mut().unwrap().bytes[24..32].copy_from_slice(&(1.0 + 2f64.powi(-40)).to_ne_bytes()),
            3 => f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[..8].copy_from_slice(&2f64.powi(-200).to_ne_bytes()),
            4 => f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().sample = Sample::F32,
            5 => f.precision = 650,
            6 => f.icc.resize((16 << 20) + 1, 0),
            7 => f.layers[0].effects[0].state.enabled = None,
            8 => f.layers[0].effects[0].state.mode = Some(28),
            9 => f.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
            10 => f.layers[0].effects[0].arguments[0].value = ArgumentValue::Integer(1001),
            11 => {
                f.layers[0].plane.as_mut().unwrap().bytes.pop();
            }
            12 => f.layers[0].effects[0].mask.as_mut().unwrap().offset = (1, 0),
            _ => f.encoding = Encoding::Linear,
        }
        assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap(), "case{case}");
    }
    // F64 low-order finite color bits remain admissible, never converted toF32.
    let mut finite = file(0);
    finite.layers[0].plane.as_mut().unwrap().bytes[..8].copy_from_slice(&(1.0 + 2f64.powi(-40)).to_ne_bytes());
    assert!(qualify(&finite, &Interrupt::NONE, Some(&budget())).unwrap());
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
    assert_eq!(family, NeighborhoodSourceFamily::BoxFinite);
    assert!(!family.qualify(&old, &Interrupt::NONE, Some(&budget())).unwrap());
    let assigned = std::fs::read(root().join("author-v1/outputs/assigned-f32-nl-r4.xcf")).unwrap();
    assert_eq!(hash(&assigned), "74bbd0fc33951f617c11bce5fb8613243ae1595a2c5d849759a5f889769c654a");
    let f = photocraft_xcf::decode(&assigned, &Default::default()).unwrap();
    assert!(hint(&f));
    assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    assert_eq!(NeighborhoodSourceFamily::hint(&f), Some(NeighborhoodSourceFamily::BoxGeometryAssignedF32));
    let f = file(0);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, Some(&b)).is_err());
    let stop = || true;
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&stop), Some(&budget())), Err(IoError::Cancelled)));
}
