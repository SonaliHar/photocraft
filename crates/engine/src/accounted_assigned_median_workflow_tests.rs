//! UNRUN authentic assigned Median adoption. Source/archive/native capture buffers
//! belong to the external harness; actual imported models and profiles stay paid.
//! Four absent loose producer ICC captures remain HOLD; no numeric oracle claim.
use super::*;
use photocraft_doc::LayerContent;
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaKind, QuotaLimits},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: [(&str, &str); 2] = [
    ("assigned-unit-f32.source.xcf", "60540a9d83f0c1998ab4bbf6796028b456698973026d942ae880b050412dad94"),
    ("assigned-unit-f32.edit.xcf", "c41d90e4c9ef73874c03ba52a80f173b3b558a1740971aff4c84a4abb49fc7cd"),
];
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
fn imported(case: usize, q: &BackingQuota) -> photocraft_io::OwnedPointXcfImport {
    let b =
        std::fs::read(PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("median-raster-color-evidence").join(CASES[case].0)).unwrap();
    assert_eq!(b.len(), 4998);
    assert_eq!(hash(&b), CASES[case].1);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let result = photocraft_io::import_owned_xcf_from_reader_in_quota(
        CASES[case].0,
        &mut Cursor::new(&b),
        b.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::Point(owner) = result else { panic!("strict assigned Median source family") };
    owner
}
fn session(case: usize) -> Session {
    let owner = imported(case, &quota());
    let mut s = Session::new();
    s.add_owned_point_xcf_import(&owner, CASES[case].0, Some(CASES[case].0), &ctl()).unwrap();
    s
}
fn native(s: &Session) -> Vec<u8> {
    let LayerContent::BackedRaster(layer) = &s.active().unwrap().doc.layers[0].content else { panic!("paid native raster") };
    let mut b = vec![0; 16 * 12 * 16];
    for coord in layer.operative_bounds().tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        for y in 0..12 {
            for x in 0..16 {
                let n = (y * 16 + x) as usize * 16;
                pin.read_pixel_native_into(x, y, &mut b[n..n + 16], &ctl()).unwrap();
            }
        }
    }
    b
}
fn archive(s: &Session) -> OwnedDocumentArchive {
    let q = s.active().unwrap().doc.quota().unwrap();
    let b = photocraft_format::owned_document::save_current_document_archive(&s.active().unwrap().doc, Cursor::new(Vec::new()), Default::default(), q, &ctl())
        .unwrap()
        .into_inner();
    photocraft_io::accounted_native::open_accounted_native_from_reader(&mut Cursor::new(&b), b.len() as u64, Default::default(), q.clone(), &ctl()).unwrap()
}
#[test]
#[ignore = "UNRUN: two actual assigned Median sources, exact HDR native/profile/mask, paid queries/title and typed native continuation"]
fn assigned_median_source_native_and_query_continuation() {
    for case in 0..2 {
        let s = session(case);
        let initial = s.active().unwrap().doc.clone();
        let owner = initial.accounted().unwrap();
        assert!(owner.is_readonly_assigned_point_storage());
        assert!(!owner.is_readonly_point_storage());
        validate_current_owned_filter_storage(owner, &ctl()).unwrap();
        assert!(
            photocraft_compose::backed_neighborhood_context::PreparedBackedNeighborhoodContext::new_assigned_box(owner, Default::default(), &ctl()).is_err()
        );
        let raw = native(&s);
        assert_eq!(hash(&raw), "3102cfa5ba9eb6cf718b767257d474fc865e0b5fa93e8b3622adeebd7fefaadf");
        let mut hdr = 0;
        for pixel in raw.as_chunks::<16>().0.iter() {
            for w in pixel[..12].as_chunks::<4>().0.iter() {
                let v = f32::from_ne_bytes(*w);
                assert!(v.is_finite());
                hdr += usize::from(!(0.0..=1.0).contains(&v));
            }
            let a = f32::from_ne_bytes(pixel[12..16].try_into().unwrap());
            assert!((0.0..=1.0).contains(&a));
        }
        assert_eq!(hdr, 5);
        let state = initial.xcf_state.as_ref().unwrap();
        let node = &state.filter_stacks[0].nodes[0];
        assert_eq!(node.operation.name, "gegl:median-blur");
        assert_eq!(node.operation.schema_epoch, "xcf22-rgb-alpha-median-v1");
        assert_eq!(node.arguments.len(), 6);
        assert!(
            matches!(node.arguments.iter().find(|a|a.name=="radius").unwrap().value,photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Signed{width:32,value})if value==if case==0{3}else{4})
        );
        assert!(
            matches!(node.arguments.iter().find(|a|a.name=="high-precision").unwrap().value,photocraft_doc::xcf_filters::FilterValue::Bool(v)if v==(case==1))
        );
        let id = node.id.0.to_string();
        let reply = crate::accounted_xcf_inspect::prepare(&s, &Value::Null, &ctl()).unwrap();
        assert_eq!(reply.value()["items"]["total"], 2);
        assert_eq!(reply.value()["filters"]["total"], 1);
        let filter = crate::accounted_filter_inspect::prepare(&s, &json!({"filterId":id}), &ctl()).unwrap();
        assert!(filter.value().is_object());
        let titled = owner.try_with_document_name("Assigned Median.pcraft", &ctl()).unwrap();
        assert!(titled.is_readonly_assigned_point_storage());
        assert_eq!(titled.document().icc_profile, initial.icc_profile);
        assert_eq!(titled.document().source_color_context, initial.source_color_context);
        assert_eq!(titled.document().xcf_state, initial.xcf_state);
        let mut reopened = Session::new();
        reopened.add_owned_native_point_archive(archive(&s), "Assigned Median.pcraft", Some("actual/Assigned Median.pcraft"), &ctl()).unwrap();
        assert!(reopened.active().unwrap().doc.accounted().unwrap().is_readonly_assigned_point_storage());
        assert_eq!(native(&reopened), raw);
        assert_eq!(reopened.active().unwrap().doc.icc_profile, initial.icc_profile);
        assert_eq!(reopened.active().unwrap().doc.source_color_context, initial.source_color_context);
        assert_eq!(reopened.active().unwrap().doc.xcf_state, initial.xcf_state);
    }
}
#[test]
#[ignore = "UNRUN: actual assigned owner rejects generic adoption and mutations, cumulative work/cancel/quota publication rollback"]
fn assigned_median_readonly_and_failed_publication() {
    let q = quota();
    let owner = imported(0, &q);
    let mut generic = Session::new();
    assert!(generic.add_accounted_document(owner.document().clone(), None, &ctl()).is_err());
    assert!(generic.active().is_none());
    let mut s = session(0);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let revision = s.active().unwrap().revision;
    let source = s.active().unwrap().source.clone();
    let journal = s.journal.clone();
    let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
    for (cmd, p) in [
        ("layer.setProps", json!({"name":"denied"})),
        ("layer.translate", json!({"dx":1})),
        ("xcf.filter.set", json!({"filterId":id,"enabled":true})),
        ("xcf.filter.enable", json!({"filterId":id,"enabled":false})),
        ("select.all", Value::Null),
        ("edit.clear", json!({"target":"mask"})),
    ] {
        assert!(s.execute(cmd, p).is_err(), "{cmd}");
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.active().unwrap().source, source);
        assert_eq!(s.journal, journal);
    }
    let mut generic = Session::new();
    assert!(generic.add_owned_native_archive(archive(&s), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "wrong", None, &ctl()).is_err());
    assert!(generic.active().is_none());
    let deny = |_| Err(BackingError::Work);
    let limited = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
    let mut empty = Session::new();
    assert!(empty.add_owned_point_xcf_import(&owner, "denied", None, &limited).is_err());
    assert!(empty.active().is_none());
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(empty.add_owned_point_xcf_import(&owner, "cancelled", None, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert!(empty.active().is_none());
    let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(empty.add_owned_point_xcf_import(&owner, "quota", None, &ctl()).is_err());
    assert!(empty.active().is_none());
    drop(hold);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(s.journal, journal);
}
