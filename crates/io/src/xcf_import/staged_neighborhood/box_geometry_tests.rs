//! UNRUN local research inputs, not corpus enrollment. Source/native capture
//! buffers are external harness data; typed import owners remain genuinely paid.
use super::*;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: [(&str, &str, usize, [i32; 2], [i32; 2], i32); 6] = [
    ("aligned-r4", "8862b7d94c64c25755a01b741a0f55cbaaadf313698f289748af4fe179c9a16d", 89214, [257, 259], [0, 0], 4),
    ("aligned-r5", "15b7292d92c4ffc2743f0caaeef6657aeca70a3cef71929e957659ef19ea7221", 89214, [257, 259], [0, 0], 5),
    ("negative-x-r4", "425aba7dd99bb00860566c00783981643b5f24da3a52b12a28ddea7f2e0c6d9e", 89274, [283, 281], [-7, 11], 4),
    ("negative-x-r5", "957c26510b6fcc2a98a66b8eeb6a5bb20c16d594bfa740290ed0ceacb206ba2d", 89274, [283, 281], [-7, 11], 5),
    ("positive-r4", "8272989febb342eef70a219a0cde3a6b8e71a27e03090e53458831bd15100f34", 89274, [283, 281], [13, 9], 4),
    ("positive-r5", "f8591f039adf31576e8bbf40da6c60ae2c2adab6dc585da6319b8b1418334561", 89274, [283, 281], [13, 9], 5),
];
fn root() -> PathBuf {
    std::env::var_os("PHOTOCRAFT_BOX_GEOMETRY_RESEARCH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol3-box-geometry-gimp-20261016"))
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
fn bytes(case: usize) -> Vec<u8> {
    let b = std::fs::read(root().join("author-v1/outputs").join(format!("{}.xcf", CASES[case].0))).unwrap();
    assert_eq!(b.len(), CASES[case].2);
    assert_eq!(hash(&b), CASES[case].1);
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
fn imported(b: &[u8], q: &BackingQuota) -> crate::OwnedPointXcfImport {
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
        .unwrap();
    let result = crate::import_owned_xcf_from_reader_in_quota(
        "Own observed Box.xcf",
        &mut Cursor::new(b),
        b.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &control(),
    )
    .unwrap();
    let crate::OwnedXcfImport::Point(owner) = result else { panic!("observational Box must select one strict Point route") };
    owner
}
#[test]
#[ignore = "UNRUN: six independently saved v22 geometry originals map actual S/C/emptyphysicalmask/newreadonlyepoch without source/native fixups"]
fn genuine_sources_keep_independent_source_and_canvas_mask() {
    for case in 0..6 {
        let f = file(case);
        assert!(hint(&f));
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        assert!(!super::super::finite(&f, "gegl:box-blur", Model::Rgb, &Interrupt::NONE, Some(&budget())).unwrap());
        let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 64 << 20, scratch: 0 });
        let source = imported(&bytes(case), &q);
        let owner = source.document();
        assert!(owner.is_readonly_point_storage());
        assert!(!owner.is_readonly_assigned_point_storage());
        owner.validate_readonly_point_storage(&control()).unwrap();
        let d = owner.document();
        let x = d.xcf_state.as_ref().unwrap();
        let node = &x.filter_stacks[0].nodes[0];
        let policy = node.composition.as_ref().unwrap();
        assert_eq!(node.operation.schema_epoch, photocraft_compose::backed_box_source::BOX_GEOMETRY_EPOCH);
        assert_eq!(policy.compatibility_epoch, node.operation.schema_epoch);
        assert!(policy.resolved.is_none());
        assert_eq!((policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator), (1654948433, 1654948433, 1654948433));
        assert!(
            matches!(node.arguments[0].value,photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Signed{width:32,value})if value==i64::from(CASES[case].5))
        );
        let photocraft_doc::LayerContent::BackedRaster(layer) = &d.layers[0].content else { panic!("actual native") };
        let [ox, oy] = CASES[case].4;
        assert_eq!(layer.operative_bounds(), Rect::new(ox, oy, ox + 257, oy + 259));
        assert_eq!(x.filter_masks[0].bounds, Rect::new(0, 0, CASES[case].3[0], CASES[case].3[1]));
        assert!(x.filter_masks[0].is_empty_selection_with(&Interrupt::NONE).unwrap());
        assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
        let maskrow = x.items.iter().find(|i| i.mask.is_some()).unwrap().mask.as_ref().unwrap();
        assert_eq!(maskrow.offset, [0, 0]);
        assert_eq!(maskrow.bounds, x.filter_masks[0].bounds);
    }
}
#[test]
#[ignore = "UNRUN: observational source envelope refuses mode/opacity/activation/profile/sample/mask/native/extent drift, without old-route retry"]
fn source_refusals_and_old_priority() {
    for case in 0..14 {
        let mut f = file(4);
        match case {
            0 => f.layers[0].effects[0].state.mode = Some(28),
            1 => f.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
            2 => f.layers[0].effects[0].state.enabled = Some(false),
            3 => f.layers[0].effects[0].state.blend_space = None,
            4 => f.layers[0].effects[0].state.clipping = Some(1),
            5 => f.layers[0].effects[0].arguments[0].value = ArgumentValue::Integer(6),
            6 => f.layers[0].effects[0].mask.as_mut().unwrap().offset = (1, 0),
            7 => f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[..4].copy_from_slice(&0.25f32.to_ne_bytes()),
            8 => f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
            9 => f.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&1.25f32.to_ne_bytes()),
            10 => f.icc.push(1),
            11 => f.sample = Sample::F64,
            12 => f.layers[0].visible = false,
            _ => f.layers[0].offset = (i32::MAX, 0),
        }
        assert!(!qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap(), "case{case}");
    }
    // Synthetic decoded harness: original16x12 geometry with new opaque words
    // selects the observational policy directly, never by old-policy retry.
    let mut small = file(0);
    small.width = 16;
    small.height = 12;
    small.layers[0].width = 16;
    small.layers[0].height = 12;
    small.layers[0].offset = (0, 0);
    let p = small.layers[0].plane.as_mut().unwrap();
    p.width = 16;
    p.height = 12;
    p.bytes.truncate(16 * 12 * 16);
    let m = small.layers[0].effects[0].mask.as_mut().unwrap();
    m.width = 16;
    m.height = 12;
    let p = m.plane.as_mut().unwrap();
    p.width = 16;
    p.height = 12;
    p.bytes.truncate(16 * 12 * 4);
    assert!(hint(&small));
    assert_eq!(NeighborhoodSourceFamily::hint(&small), Some(NeighborhoodSourceFamily::BoxGeometryF32));
    assert!(qualify(&small, &Interrupt::NONE, Some(&budget())).unwrap());
    let mut old = small.clone();
    let state = &mut old.layers[0].effects[0].state;
    state.blend_space = Some(-1474592175);
    state.composite_space = Some(-1474592175);
    state.composite_mode = Some(-1474592175);
    assert!(!hint(&old));
    assert_eq!(NeighborhoodSourceFamily::hint(&old), Some(NeighborhoodSourceFamily::BoxFinite));
    old.layers[0].visible = false;
    let family = NeighborhoodSourceFamily::hint(&old).unwrap();
    assert_eq!(family, NeighborhoodSourceFamily::BoxFinite);
    assert!(!family.qualify(&old, &Interrupt::NONE, Some(&budget())).unwrap());
    assert!(!hint(&old));
    let f = file(0);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, Some(&b)).is_err());
    let stop = || true;
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&stop), Some(&budget())), Err(IoError::Cancelled)));
}
#[test]
#[ignore = "UNRUN: fifteen independently byte-patched raw-field inputs retain exact three opaque words and source-property payloads under the closed epoch"]
fn opaque_raw_fields_are_retained_without_resolving() {
    let plan_bytes = std::fs::read(root().join("raw-author-v1/plan.json")).unwrap();
    assert_eq!(hash(&plan_bytes), "9577b970d7001d7274078e1acf39e28eed1b1a6103eae55d3834e60be0c2a330");
    let report: serde_json::Value = serde_json::from_slice(&plan_bytes).unwrap();
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 18);
    for case in cases.iter().filter(|v| !v["sensitivity"].as_bool().unwrap()) {
        let b = std::fs::read(case["input"]["path"].as_str().unwrap()).unwrap();
        assert_eq!(hash(&b), case["input"]["sha256"].as_str().unwrap());
        let f = photocraft_xcf::decode(&b, &Default::default()).unwrap();
        let e = &f.layers[0].effects[0];
        let raw = (e.state.blend_space.unwrap(), e.state.composite_space.unwrap(), e.state.composite_mode.unwrap());
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 64 << 20, scratch: 0 });
        let source = imported(&b, &q);
        let d = source.document().document();
        let x = d.xcf_state.as_ref().unwrap();
        let p = x.filter_stacks[0].nodes[0].composition.as_ref().unwrap();
        assert_eq!((p.stored_blend_space, p.stored_composite_space, p.stored_operator), raw);
        assert!(p.resolved.is_none());
        assert!(!p.legacy && !p.pass_through);
        for property in e.properties.iter().filter(|p| matches!(p.tag, 35 | 36 | 37)) {
            let identity = format!("{}@{}:{}", property.tag, property.offset, property.declared_bytes);
            assert!(x.opaque.iter().any(|v| v.identity == identity && v.bytes.as_slice() == property.payload.as_slice()));
        }
    }
}
