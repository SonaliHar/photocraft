//! UNRUN actual activity originals; research captures are external harness data, not corpus enrollment/numeric acceptance.
use super::*;
use photocraft_doc::item_identity::{ItemRef, MaskId};
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: [(&str, &str, usize, bool, u32, bool); 15] = [
    (
        "temp/sol4-observed-box-source-activity-gimp-20261016/author-v1/outputs/baseline.xcf",
        "1cb8852ebbb53b572315fb8f79aa812cf2e6a20f6617b324115ba1b76e1a6cfe",
        2692,
        true,
        1065353216,
        true,
    ),
    (
        "temp/sol4-observed-box-source-activity-gimp-20261016/author-v1/outputs/layer-hidden-only.xcf",
        "af452f113e3c7915f6d66ffd254c8cf81386bc09fd1a515ef8c077fb02347f78",
        2692,
        false,
        1065353216,
        true,
    ),
    (
        "temp/sol4-observed-box-source-activity-gimp-20261016/author-v1/outputs/layer-opacity-only.xcf",
        "89e0de15990b10cd36f70c9b65976a9234d280bea068ee9f408d0d75be0e863d",
        2692,
        true,
        1052770304,
        true,
    ),
    (
        "temp/sol4-observed-box-source-activity-gimp-20261016/author-v1/outputs/layer-zero-opacity-only.xcf",
        "9c8deedc704c8da8409bdb6b9ce362e6ee9d45ee4cbaaa71c2b315bc6fbeb3c0",
        2692,
        true,
        0,
        true,
    ),
    (
        "temp/sol4-observed-box-source-activity-gimp-20261016/author-v1/outputs/filter-disabled-only.xcf",
        "00784239c89ddfb404f56e345d9d6510a1894753bd2bfa021316c8a208ef5e80",
        2692,
        true,
        1065353216,
        false,
    ),
    (
        "temp/sol4-observed-box-wide-activity-gimp-20261016/implicit-f64-nl-author-v1/outputs/implicit-f64-nl-baseline.xcf",
        "7233c501e515fbba86824a06abff8f79f451425cc37d1de0f6d9d0259df64412",
        85358,
        true,
        1065353216,
        true,
    ),
    (
        "temp/sol4-observed-box-wide-activity-gimp-20261016/implicit-f64-nl-author-v1/outputs/implicit-f64-nl-layer-hidden-only.xcf",
        "cd80224ad177d4290b539ffdad06a350d5bd6cf33f2f3e580cf091784ef251a6",
        85358,
        false,
        1065353216,
        true,
    ),
    (
        "temp/sol4-observed-box-wide-activity-gimp-20261016/implicit-f64-nl-author-v1/outputs/implicit-f64-nl-layer-opacity-only.xcf",
        "81cb1fb50e6ee263c2e00a2e012f1957ed5b8fe23aae58db2b918acb1424f1f0",
        85358,
        true,
        1052770304,
        true,
    ),
    (
        "temp/sol4-observed-box-wide-activity-gimp-20261016/implicit-f64-nl-author-v1/outputs/implicit-f64-nl-layer-zero-opacity-only.xcf",
        "da625364c479e5a3a2a3dd8b6c7f4f5109eb8069c546c639837b57056b121b6b",
        85358,
        true,
        0,
        true,
    ),
    (
        "temp/sol4-observed-box-wide-activity-gimp-20261016/implicit-f64-nl-author-v1/outputs/implicit-f64-nl-filter-disabled-only.xcf",
        "9d0decee9013d73220ffea861ac608327d09eb5594ce2fbe1e08f3ed33822d61",
        85358,
        true,
        1065353216,
        false,
    ),
    (
        "temp/sol4-observed-box-wide-activity-gimp-20261016/assigned-f32-nl-author-v1/outputs/assigned-f32-nl-baseline.xcf",
        "e1c0f010f59d0127703cac69f5ea21cdee83b9191e73f7a6ee45259ea1bc36a7",
        89882,
        true,
        1065353216,
        true,
    ),
    (
        "temp/sol4-observed-box-wide-activity-gimp-20261016/assigned-f32-nl-author-v1/outputs/assigned-f32-nl-layer-hidden-only.xcf",
        "985b896b874bfc8a9ebdbc71b9a4caef6dc11e77d3fa7a7bd19ef9f875e51250",
        89882,
        false,
        1065353216,
        true,
    ),
    (
        "temp/sol4-observed-box-wide-activity-gimp-20261016/assigned-f32-nl-author-v1/outputs/assigned-f32-nl-layer-opacity-only.xcf",
        "5ddbf4e65555c2cff7804717e73769fd47b2c77367927de84533bbc374311a90",
        89882,
        true,
        1052770304,
        true,
    ),
    (
        "temp/sol4-observed-box-wide-activity-gimp-20261016/assigned-f32-nl-author-v1/outputs/assigned-f32-nl-layer-zero-opacity-only.xcf",
        "a9401d020ee815c18c83b84fbd8b293083cce2183ba545cbafb7126e12914aaa",
        89882,
        true,
        0,
        true,
    ),
    (
        "temp/sol4-observed-box-wide-activity-gimp-20261016/assigned-f32-nl-author-v1/outputs/assigned-f32-nl-filter-disabled-only.xcf",
        "028168c6194d6389664a6e0f1a0b37737c91453dba0e10cc120b4f38e5856f79",
        89882,
        true,
        1065353216,
        false,
    ),
];
fn root() -> PathBuf {
    std::env::var_os("PHOTOCRAFT_BOX_ACTIVITY_INPUT_ROOT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
fn bytes(i: usize) -> Vec<u8> {
    let b = std::fs::read(root().join(CASES[i].0)).unwrap();
    assert_eq!(b.len(), CASES[i].2);
    assert_eq!(hash(&b), CASES[i].1);
    b
}
fn file(i: usize) -> XcfFile {
    photocraft_xcf::decode(&bytes(i), &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new_persistent(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None).unwrap()
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 512 << 20, index: 128 << 20, scratch: 0 })
}
fn imported(b: &[u8], q: &BackingQuota) -> Result<crate::OwnedPointXcfImport, IoError> {
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("scratchBytes", 0)]).unwrap();
    // The old default-R2GiB profile honestly refuses qR512MiB, even before
    // decoding; retain that negative and its exact same-ledger cleanup.
    let before = q.usage().unwrap();
    let denied = crate::import_owned_xcf_from_reader_in_quota(
        "Own Box activity.xcf",
        &mut Cursor::new(b),
        b.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    );
    assert!(denied.is_err());
    assert_eq!(q.usage().unwrap(), before);
    let limits = limits.lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20)]).unwrap();
    match crate::import_owned_xcf_from_reader_in_quota(
        "Own Box activity.xcf",
        &mut Cursor::new(b),
        b.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )? {
        crate::OwnedXcfImport::Point(p) => Ok(p),
        _ => Err(IoError::Unsupported("expected strict Box Point".into())),
    }
}
#[test]
#[ignore = "UNRUN: all15 genuinely authored activity sources retain effectiveF32layeropacity/rawbothproperties/native/mask/profile/activation andpaidtitle"]
fn genuine_activity_sources_preserve_literal_roles() {
    for (i, case_row) in CASES.iter().enumerate() {
        let f = file(i);
        assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        let q = quota();
        let source = imported(&bytes(i), &q).unwrap();
        let owner = source.document();
        let d = owner.document();
        let x = d.xcf_state.as_ref().unwrap();
        let l = &d.layers[0];
        let layer = ItemRef::Layer(l.id);
        assert_eq!(l.visible, case_row.3);
        assert_eq!(l.opacity.to_bits(), case_row.4);
        assert_eq!(x.filter_stacks[0].nodes[0].enabled, case_row.5);
        assert_eq!(x.filter_stacks[0].nodes[0].opacity_f64_bits, 1f64.to_bits());
        assert_eq!(x.filter_stacks[0].nodes[0].operation.schema_epoch, photocraft_compose::backed_box_source::BOX_GEOMETRY_EPOCH);
        let item = x.items.iter().find(|r| r.item == layer).unwrap();
        assert_eq!(item.visible, l.visible);
        let policy = x.filter_stacks[0].nodes[0].composition.as_ref().unwrap();
        assert!(policy.resolved.is_none());
        assert_eq!((policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator), (1654948433, 1654948433, 1654948433));
        for p in f.layers[0].raw_properties.iter().filter(|p| matches!(p.tag, 6 | 33 | 8)) {
            let prefix = format!("{}@", p.tag);
            let preserved =
                x.opaque.iter().find(|r| r.owner == Some(layer) && r.namespace == "gimp/xcf/source-property" && r.identity.starts_with(&prefix)).unwrap();
            assert_eq!(preserved.bytes.as_slice(), p.payload.as_slice());
        }
        if case_row.4 == 0.375f32.to_bits() {
            assert_ne!(l.opacity.to_bits(), (95f32 / 255.0).to_bits());
            let p = f.layers[0].raw_properties.iter().find(|p| p.tag == 6).unwrap();
            assert_eq!(p.payload, 95u32.to_be_bytes());
        }
        let assigned = !f.icc.is_empty();
        assert_eq!(owner.is_readonly_assigned_point_storage(), assigned);
        if assigned {
            assert_eq!(d.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap().as_slice(), f.icc.as_slice());
        } else {
            assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
        }
        let photocraft_doc::LayerContent::BackedRaster(native) = &l.content else { panic!("native") };
        let rect = native.operative_bounds();
        let mut captured = vec![0u8; (rect.width() as usize) * (rect.height() as usize) * native.default_bytes().len()];
        let bpp = native.default_bytes().len();
        for coord in rect.tiles() {
            let pin = native.checked_view().pin_tile(coord, &ctl()).unwrap();
            let part = coord.rect().intersect(&rect);
            for y in part.y0..part.y1 {
                for x in part.x0..part.x1 {
                    let at = ((y - rect.y0) as usize * rect.width() as usize + (x - rect.x0) as usize) * bpp;
                    pin.read_pixel_native_into(x, y, &mut captured[at..at + bpp], &ctl()).unwrap();
                }
            }
        }
        assert_eq!(captured, f.layers[0].plane.as_ref().unwrap().bytes);
        assert_eq!(x.filter_masks[0].bounds, d.bounds());
        assert!(x.filter_masks[0].is_empty_selection_with(&Interrupt::NONE).unwrap());
        let titled = owner.try_with_document_name("Activity.pcraft", &ctl()).unwrap();
        assert_eq!(titled.document().xcf_state, d.xcf_state);
        assert_eq!(titled.document().source_color_context, d.source_color_context);
        assert_eq!(titled.document().layers[0].opacity.to_bits(), l.opacity.to_bits());
    }
}
#[test]
#[ignore = "UNRUN: policy crossproduct overexisting3roles keepsfullnative/mask/metadata validation evenhidden/zero/disabled"]
fn inactive_sources_cannot_hide_invalid_data() {
    for i in [0, 5, 10] {
        for (visible, opacity, enabled) in [(false, 0.0, false), (false, 0.375, true), (true, -0.0, false)] {
            let mut f = file(i);
            f.layers[0].visible = visible;
            f.layers[0].opacity = opacity;
            f.layers[0].effects[0].state.enabled = Some(enabled);
            assert!(qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
            for failure in 0..6 {
                let mut bad = f.clone();
                match failure {
                    0 => {
                        let p = &mut bad.layers[0].plane.as_mut().unwrap().bytes;
                        if bad.sample == Sample::F64 {
                            p[..8].copy_from_slice(&f64::NAN.to_ne_bytes())
                        } else {
                            p[..4].copy_from_slice(&f32::NAN.to_ne_bytes())
                        }
                    }
                    1 => {
                        let p = &mut bad.layers[0].plane.as_mut().unwrap().bytes;
                        if bad.sample == Sample::F64 {
                            p[24..32].copy_from_slice(&1.25f64.to_ne_bytes())
                        } else {
                            p[12..16].copy_from_slice(&1.25f32.to_ne_bytes())
                        }
                    }
                    2 => {
                        let p = &mut bad.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes;
                        if bad.sample == Sample::F64 { p[..8].copy_from_slice(&0.25f64.to_ne_bytes()) } else { p[..4].copy_from_slice(&0.25f32.to_ne_bytes()) }
                    }
                    3 => bad.layers[0].opacity = f32::NAN,
                    4 => bad.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
                    _ => bad.layers[0].effects[0].state.enabled = None,
                }
                assert!(!qualify(&bad, &Interrupt::NONE, Some(&budget())).unwrap(), "role{i}/fault{failure}");
            }
        }
    }
}
#[test]
#[ignore = "UNRUN: floatopacityauthoritative overlegacy95 regardlesspropertyorder; duplicatefloat rejects; observeddisabledmapperexception staysdomainonly"]
fn opacity_precedence_and_mapper_confinement() {
    let i = 2;
    let f = file(i);
    assert_eq!(f.layers[0].opacity.to_bits(), 0.375f32.to_bits());
    let p6 = f.layers[0].raw_properties.iter().find(|p| p.tag == 6).unwrap();
    let p33 = f.layers[0].raw_properties.iter().find(|p| p.tag == 33).unwrap();
    assert_eq!(p6.payload, 95u32.to_be_bytes());
    assert_eq!(p33.payload, 0.375f32.to_bits().to_be_bytes());
    let mut b = bytes(i);
    let at6 = p6.offset as usize - 8;
    let at33 = p33.offset as usize - 8;
    let a = b[at6..at6 + 12].to_vec();
    let c = b[at33..at33 + 12].to_vec();
    b[at6..at6 + 12].copy_from_slice(&c);
    b[at33..at33 + 12].copy_from_slice(&a);
    let changed = photocraft_xcf::decode(&b, &Default::default()).unwrap();
    assert_eq!(changed.layers[0].opacity.to_bits(), 0.375f32.to_bits());
    let source = imported(&b, &quota()).unwrap();
    assert_eq!(source.document().document().layers[0].opacity.to_bits(), 0.375f32.to_bits());
    b[at33..at33 + 4].copy_from_slice(&33u32.to_be_bytes());
    assert!(photocraft_xcf::decode(&b, &Default::default()).is_err());
    let mut f = file(4);
    let masks = [Some(ItemRef::Mask(MaskId(2)))];
    let owner = ItemRef::Layer(photocraft_doc::LayerId(1));
    let stack = crate::xcf_filters::qualified_stack(&f.layers[0].effects, &masks, owner, 100, 22, crate::xcf_filters::NeighborhoodDomain::BoxGeometry).unwrap();
    assert!(!stack.nodes[0].enabled);
    assert!(stack.nodes[0].composition.as_ref().unwrap().resolved.is_none());
    let s = &mut f.layers[0].effects[0].state;
    s.blend_space = Some(-1474592175);
    s.composite_space = Some(-1474592175);
    s.composite_mode = Some(-1474592175);
    assert!(crate::xcf_filters::qualified_stack(&f.layers[0].effects, &masks, owner, 100, 22, crate::xcf_filters::NeighborhoodDomain::BoxFinite).is_err());
}
#[test]
#[ignore = "UNRUN: inactiveoldidentity remainsselectedfinalrefusal, actualassignedICCcorruption andlowwork/cancel refusewithoutimplicitretry"]
fn identity_profile_and_control_remain_authoritative() {
    for i in [0, 5, 10] {
        let mut f = file(i);
        f.width = 16;
        f.height = 12;
        f.layers[0].width = 16;
        f.layers[0].height = 12;
        f.layers[0].offset = (0, 0);
        f.layers[0].visible = false;
        f.layers[0].opacity = 0.375;
        let s = &mut f.layers[0].effects[0].state;
        s.enabled = Some(false);
        s.blend_space = Some(-1474592175);
        s.composite_space = Some(-1474592175);
        s.composite_mode = Some(-1474592175);
        assert!(!hint(&f));
        let family = NeighborhoodSourceFamily::hint(&f).unwrap();
        assert_eq!(family, if f.icc.is_empty() { NeighborhoodSourceFamily::BoxFinite } else { NeighborhoodSourceFamily::BoxAssignedF32 });
        assert!(!family.qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    }
    let i = 14;
    let f = file(i);
    let mut b = bytes(i);
    let places: Vec<_> = b.windows(f.icc.len()).enumerate().filter_map(|(i, w)| (w == f.icc.as_slice()).then_some(i)).collect();
    assert_eq!(places.len(), 1);
    b[places[0]..places[0] + 4].copy_from_slice(&0u32.to_be_bytes());
    assert!(imported(&b, &quota()).is_err());
    let f = file(14);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let budget = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(qualify(&f, &Interrupt::NONE, Some(&budget)).is_err());
    let stop = || true;
    assert!(matches!(qualify(&f, &Interrupt::cancel_only(&stop), Some(&self::budget())), Err(IoError::Cancelled)));
}
