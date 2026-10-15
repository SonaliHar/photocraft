//! UNRUN exact old legacy14 source boundary; decoded buffers/captures are external harness data.
//! No new source inventory/projection oracle or generalized legacy source claim.
use super::super::{SelectedStagedFamily, SourceSelectedPolicy, StagePolicy};
use super::*;
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: &[(i32, &str, &str, usize)] = &[
    (3, "composition-matrix/mode-multiply-legacy.xcf", "5e360b5ef372f417cb6d9d7bdf4d1ae45ee8404f81c6c3515a770cdfdf260724", 5425),
    (3, "composition-matrix/mode-multiply-legacy.edit.xcf", "e688b4f351ea2cceac7f8aa91c13a8fe171e7b38a9029289ec3fc34e68f1200f", 5425),
    (4, "composition-matrix/mode-screen-legacy.xcf", "5549e0664cb0c5fe4d789f7051df82e5166c827b5cda5399de358e0f77f5f65b", 5425),
    (4, "composition-matrix/mode-screen-legacy.edit.xcf", "2bd243b2c860674bd27b98508b537cb1905cd073f64503ca3065cc3db78a5ef2", 5425),
    (6, "composition-matrix/mode-difference-legacy.xcf", "631a21065ee78a11e27fb30b94cd5e4d6de498cb473ac24c393d957e78955c10", 5425),
    (6, "composition-matrix/mode-difference-legacy.edit.xcf", "25ae92dcddb975ffe28ecd52ca7f6a2e973c15b0bb33ced4a6e7503250db39cc", 5425),
    (7, "composition-matrix/mode-addition-legacy.xcf", "331cf71d7c4190361dc0669587d6db185e6805c599aa89bc5c5264a364dccfe2", 5425),
    (7, "composition-matrix/mode-addition-legacy.edit.xcf", "c8de95694fac48d388fc2784f2af18d278e99fcd81ec58df3d85de281c73d858", 5425),
    (8, "composition-matrix/mode-subtract-legacy.xcf", "a1d3576a425099b68c08e6509b219abae91049036b1f1d02c764c308db50a93a", 5425),
    (8, "composition-matrix/mode-subtract-legacy.edit.xcf", "43700e9b6f15513ed5f3978270e53a81190a0827169dd7fc679bd94a1ad271af", 5425),
    (9, "composition-matrix/mode-darken-only-legacy.xcf", "f131a3acdebcdf21d094113e8de49f11323fac02f1a6b1b2bd2eaac215b60cef", 5425),
    (9, "composition-matrix/mode-darken-only-legacy.edit.xcf", "e7af316d808acdf0c644d9147d3b43c484e337371003dd0682f61d9eec507f79", 5425),
    (10, "composition-matrix/mode-lighten-only-legacy.xcf", "c1227beafcc354d039ad4a1acad97e420aa74bb4b330d729ec8367d5e9a6d63e", 5425),
    (10, "composition-matrix/mode-lighten-only-legacy.edit.xcf", "acc87218736dbc47856d2156477898d724e4cb34cf5e05ba671643ea5a245655", 5425),
    (15, "composition-matrix/mode-divide-legacy.xcf", "4123a81300930c61dcb9d87b3c1b0e345413e12970e4e0aedd077b5b00ef85ad", 5425),
    (15, "composition-matrix/mode-divide-legacy.edit.xcf", "6ef837a41f158cd07181bf99e011c94791d9dd450c26c5e4c0796f4630e93fe2", 5425),
    (16, "composition-matrix/mode-dodge-legacy.xcf", "04a5a327080689e01e912d6ba5b80d35f43740957424e0fdcfc78415cfa00dd2", 5425),
    (16, "composition-matrix/mode-dodge-legacy.edit.xcf", "d701e5871058041b5e85a4bbbe638fb19b57958039e2d84bf9a428afeb2061c4", 5425),
    (17, "composition-matrix/mode-burn-legacy.xcf", "3353f9add2f61f63de5472fb2a05b5715e77b79cd1ad5064879603014827d303", 5425),
    (17, "composition-matrix/mode-burn-legacy.edit.xcf", "c42e0514144502977746f75d89c497263eb84e4fe704c8e4dcedd9bb18d52e32", 5425),
    (18, "composition-matrix/mode-hardlight-legacy.xcf", "8efcdc27963ee1a9c8de0cb0b707b69f8de9f108c419acfbdf3ee2ba13336aaf", 5425),
    (18, "composition-matrix/mode-hardlight-legacy.edit.xcf", "5d77d1c54445c281ba9b8d4e8de91f73df62b1e97e082ee830697d3c045b5952", 5425),
    (19, "composition-matrix/mode-softlight-legacy.xcf", "e55f2a8270fed8c422c074c267eabce53be7381acc917be56c98de4195ec4d26", 5425),
    (19, "composition-matrix/mode-softlight-legacy.edit.xcf", "9a6deffbbd711673870bfc663d4fc991feb7ca2c1e249bb1a15b31c1c8fdef97", 5425),
    (20, "composition-matrix/mode-grain-extract-legacy.xcf", "fe51d57e6edcb8b7e89a0ec8561b856a5395799b99640a32323a24bda019c060", 5425),
    (20, "composition-matrix/mode-grain-extract-legacy.edit.xcf", "ea3dcb513399a814fa1cd066b889e3735a13338f1a9a5a61eaae9878275c4263", 5425),
    (21, "composition-matrix/mode-grain-merge-legacy.xcf", "c9d3e10c9f938508b581fc1e3f3fbc1b75ecf8783c3b1d6ac999f95a795d9f40", 5425),
    (21, "composition-matrix/mode-grain-merge-legacy.edit.xcf", "9128f7d994b69971ab8d50c78a2a0fd420bf37014cc58600bf139c515101249b", 5425),
];
fn bytes(i: usize) -> Vec<u8> {
    let (_, name, pin, len) = CASES[i];
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
#[ignore = "UNRUN: all28 genuine source/edit packets retain exact old metadata and independent legacy raw policy"]
fn exact_old_packets() {
    use photocraft_doc::import_compositing::{GimpOperator, GimpSpace};
    for i in 0..CASES.len() {
        let f = source(i);
        assert_eq!(SourceSelectedPolicy::family_controlled(&f, &Interrupt::NONE, Some(&budget())).unwrap(), SelectedStagedFamily::SeparableBlend);
        assert_eq!(qualify_selected(&f, &Interrupt::NONE, &budget()).unwrap(), Some(SourceKind::LegacyStrict));
        assert!(admission::classify_late(&f).basic_blend_domain);
        assert_eq!(
            (f.version, f.layers[0].mode, f.layers[0].blend_space, f.layers[0].composite_space, f.layers[0].composite_mode),
            (12, CASES[i].0, -2, -2, -2)
        );
        assert_eq!((f.layers[1].mode, f.layers[1].blend_space, f.layers[1].composite_space, f.layers[1].composite_mode), (28, 0, -1, -1));
        let r = photocraft_doc::import_compositing::flat_legacy_source_rule(12, CASES[i].0, -2, -2, -2).unwrap().resolved();
        assert_eq!((r.blend_space, r.composite_space, r.operator), (GimpSpace::ProfileRgb, GimpSpace::ProfileRgb, GimpOperator::ClipToBackdrop));
    }
}
#[test]
#[ignore = "UNRUN: all28 single selected decode promotes actual paid native/class with original raw epochs, not eager or Plain fallback"]
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
        let crate::OwnedXcfImport::SeparableBlend(imported) = result else { panic!("legacy strict route") };
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
                assert!(p.legacy);
                assert_eq!(p.compatibility_epoch, "xcf-12-authored-blend-auto");
                assert_eq!((r.blend_space, r.composite_space, r.operator), (GimpSpace::ProfileRgb, GimpSpace::ProfileRgb, GimpOperator::ClipToBackdrop));
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
    for case in 0..10 {
        let mut f = source(0);
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
            7 => f.layers[0].blend_space = -1,
            8 => f.icc = vec![0; 132],
            _ => f.version = 23,
        }
        assert!(qualify_selected(&f, &Interrupt::NONE, &budget()).unwrap().is_none());
    }
}
#[test]
#[ignore = "UNRUN: legacy endpoints remain unit-only, paid work/cancellation and allNormal Plain priority"]
fn endpoints_control_and_plain_priority() {
    for mode in [15, 16, 17] {
        for v in [0f32, 1f32] {
            let i = CASES.iter().position(|r| r.0 == mode).unwrap();
            let mut f = source(i);
            f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&v.to_ne_bytes());
            assert_eq!(qualify_selected(&f, &Interrupt::NONE, &budget()).unwrap(), Some(SourceKind::LegacyStrict));
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
