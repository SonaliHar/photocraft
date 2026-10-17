//! UNRUN local GIMP research originals; no corpus enrollment or numeric acceptance.
use super::*;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: [(i32, &str); 4] = [
    (0, "d0dec8e32a34448890f6ee2bbf57266d9f4ef8fa98fcd45d3f73409056e728bc"),
    (1, "5e007491dc795b3165033432b0519a016f0a56828d6c5108543dc9cec1b5df1c"),
    (127, "31a271d275f75f5e996044c49f1cfb5c4e5d2ff92ba254d024b3962803187fc7"),
    (1000, "5abc6b3d6d95d3b1f5aeb341f2e969466e74e7f8ec8d93f1c9a1af8b0f6d1cd3"),
];
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
fn bytes(i: usize) -> Vec<u8> {
    let root = std::env::var_os("PHOTOCRAFT_BOX_RADIUS_RESEARCH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol3-box-radius-domain-gimp-20261016"));
    let b = std::fs::read(root.join("author-v1/outputs").join(format!("radius{}.xcf", CASES[i].0))).unwrap();
    assert_eq!(b.len(), 2692);
    assert_eq!(hash(&b), CASES[i].1);
    b
}
fn file(i: usize) -> XcfFile {
    photocraft_xcf::decode(&bytes(i), &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new_persistent(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None).unwrap()
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[test]
#[ignore = "UNRUN: four authored radius endpoints/interiors use observed-only source/prep/promotion, exact radius/raw/native/mask/title custody"]
fn genuine_radius_originals_map_without_identity_shortcut() {
    for (i, case_row) in CASES.iter().enumerate() {
        let f = file(i);
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert_eq!(NeighborhoodSourceFamily::hint(&f), Some(NeighborhoodSourceFamily::BoxGeometryF32));
        let q = BackingQuota::new(QuotaLimits { resident: 512 << 20, index: 128 << 20, scratch: 0 });
        let b = bytes(i);
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("scratchBytes", 0)]).unwrap();
        // Retain the old overlarge preparation profile as a genuine refusal.
        let before = q.usage().unwrap();
        let denied = crate::import_owned_xcf_from_reader_in_quota(
            "Own Box radius.xcf",
            &mut Cursor::new(&b),
            b.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &control(),
        );
        assert!(denied.is_err());
        assert_eq!(q.usage().unwrap(), before);
        let limits = limits.lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20)]).unwrap();
        let crate::OwnedXcfImport::Point(source) = crate::import_owned_xcf_from_reader_in_quota(
            "Own Box radius.xcf",
            &mut Cursor::new(&b),
            b.len() as u64,
            &Default::default(),
            &limits,
            q,
            &control(),
        )
        .unwrap() else {
            panic!("strict observed Point")
        };
        let owner = source.document();
        owner.validate_readonly_point_storage(&control()).unwrap();
        let d = owner.document();
        let x = d.xcf_state.as_ref().unwrap();
        let n = &x.filter_stacks[0].nodes[0];
        assert_eq!(n.operation.schema_epoch, photocraft_compose::backed_box_source::BOX_GEOMETRY_EPOCH);
        assert!(
            matches!(n.arguments[0].value,photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Signed{width:32,value})if value==i64::from(case_row.0))
        );
        let c = n.composition.as_ref().unwrap();
        assert!(c.resolved.is_none());
        assert_eq!((c.stored_blend_space, c.stored_composite_space, c.stored_operator), (1654948433, 1654948433, 1654948433));
        let photocraft_doc::LayerContent::BackedRaster(l) = &d.layers[0].content else { panic!("native source") };
        assert_eq!(l.operative_bounds(), Rect::new(3, -2, 19, 10));
        assert_eq!(x.filter_masks[0].bounds, Rect::new(0, 0, 21, 17));
        assert!(x.filter_masks[0].is_empty_selection_with(&Interrupt::NONE).unwrap());
        let mut actual = Vec::new();
        for y in -2..10 {
            for x in 3..19 {
                let mut p = [0u8; 16];
                l.checked_view().read_pixel_native_into(x, y, &mut p, &control()).unwrap();
                actual.extend_from_slice(&p)
            }
        }
        assert_eq!(actual, f.layers[0].plane.as_ref().unwrap().bytes);
        let title = owner.try_with_document_name("Observed radius.pcraft", &control()).unwrap();
        assert_eq!(title.document().xcf_state, d.xcf_state);
    }
}
#[test]
#[ignore = "UNRUN: exact integer range, explicit activity envelope, opaque source preservation and oldidentity final refusal"]
fn range_and_old_identity_fail_closed() {
    for r in [0, 1, 2, 126, 127, 999, 1000] {
        let mut f = file(0);
        f.layers[0].effects[0].arguments[0].value = ArgumentValue::Integer(r);
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    }
    for r in [-1, 1001, i32::MIN, i32::MAX] {
        let mut f = file(0);
        f.layers[0].effects[0].arguments[0].value = ArgumentValue::Integer(r);
        assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    }
    let mut f = file(0);
    f.layers[0].effects[0].arguments[0].value = ArgumentValue::Float32Bits(1f32.to_bits());
    assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    let mut f = file(3);
    f.layers[0].effects[0].state.enabled = None;
    assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    for r in [0, 127, 1000] {
        let mut f = file(0);
        f.width = 16;
        f.height = 12;
        f.layers[0].offset = (0, 0);
        let s = &mut f.layers[0].effects[0].state;
        s.blend_space = Some(-1474592175);
        s.composite_space = Some(-1474592175);
        s.composite_mode = Some(-1474592175);
        f.layers[0].effects[0].arguments[0].value = ArgumentValue::Integer(r);
        assert!(!hint(&f));
        let family = NeighborhoodSourceFamily::hint(&f).unwrap();
        assert_eq!(family, NeighborhoodSourceFamily::BoxFinite);
        // The old native/source envelope is selected; its exact old radius
        // dictionary still refuses these values during semantic mapping.
        assert!(family.qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert!(
            crate::xcf_filters::qualified_owned_neighborhood_stack(
                &f.layers[0].effects,
                &[None],
                photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1)),
                1,
                22,
                family.domain()
            )
            .is_err()
        );
    }
}
