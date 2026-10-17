//! UNRUN original opaque Gaussian source/current workflows. Source/archive
//! captures are external harness bytes. These assertions establish ownership,
//! raw native preservation and typed command continuity, not a new PNG oracle.
use super::*;
use photocraft_doc::{
    LayerContent,
    item_identity::FilterId,
    xcf_filters::{ExactNumber, FilterValue},
};
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaKind, QuotaLimits},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Cursor,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
const CASES: [(&str, &str, f32); 2] = [
    ("filter-owner-layer.xcf", "63eb43a9ad9135babe8d832891334d691a0b2899e0ebbeea6271f7161d1b91ba", 2.25),
    ("filter-owner-layer.edit.xcf", "9a9d833afdfe29b6386a60059bf10e6336bfa446aea3ed483469c5ea04b60a79", 3.75),
];
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn bytes(case: usize) -> Vec<u8> {
    let b = std::fs::read(PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(CASES[case].0)).unwrap();
    assert_eq!(b.len(), 2215);
    assert_eq!(Sha256::digest(&b).iter().map(|v| format!("{v:02x}")).collect::<String>(), CASES[case].1);
    b
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
fn imported(case: usize, q: &BackingQuota) -> photocraft_io::OwnedPointXcfImport {
    let bytes = bytes(case);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 16 << 20), ("scratchBytes", 0)])
        .unwrap();
    let result = photocraft_io::import_owned_xcf_from_reader_in_quota(
        CASES[case].0,
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::Point(owner) = result else { panic!("actual OriginalGaussian selected once") };
    owner
}
fn session(case: usize) -> Session {
    let owner = imported(case, &quota());
    let mut s = Session::new();
    s.add_owned_point_xcf_import(&owner, CASES[case].0, Some(CASES[case].0), &ctl()).unwrap();
    drop(owner);
    s
}
fn id(s: &Session) -> FilterId {
    s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id
}
fn wire(s: &Session, x: f32) -> Value {
    json!({"filterId":id(s).0.to_string(),"arguments":[{"name":"std-dev-x","value":{"Number":{"Float32Bits":x.to_bits()}}}]})
}
fn native(s: &Session) -> Vec<u8> {
    let LayerContent::BackedRaster(layer) = &s.active().unwrap().doc.layers[0].content else { panic!() };
    let r = layer.operative_bounds();
    assert_eq!(r, photocraft_geom::Rect::new(0, 0, 96, 80));
    let mut b = vec![0; 96 * 80 * 4];
    for coordinate in r.tiles() {
        let pin = layer.checked_view().pin_tile(coordinate, &ctl()).unwrap();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let n = (y as usize * 96 + x as usize) * 4;
                pin.read_pixel_native_into(x, y, &mut b[n..n + 4], &ctl()).unwrap();
            }
        }
    }
    b
}
fn archive(s: &Session) -> OwnedDocumentArchive {
    let q = s.active().unwrap().doc.quota().unwrap();
    let bytes =
        photocraft_format::owned_document::save_current_document_archive(&s.active().unwrap().doc, Cursor::new(Vec::new()), Default::default(), q, &ctl())
            .unwrap()
            .into_inner();
    photocraft_io::accounted_native::open_accounted_native_from_reader(&mut Cursor::new(&bytes), bytes.len() as u64, Default::default(), q.clone(), &ctl())
        .unwrap()
}
#[test]
#[ignore = "UNRUN: two independently pinned original Gaussian packets enter genuine Point class, paid inspect/edit/activation/history/native continuation"]
fn original_gaussian_current_commands_keep_exact_source_and_native_payloads() {
    for (case, entry) in CASES.iter().enumerate().take(2) {
        let mut s = session(case);
        let initial = s.active().unwrap().doc.clone();
        let raw = native(&s);
        assert_eq!(
            Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "7b675a5f14a4f41487dec96442b9495bed9c80d33c4a9576b169d80ce466704a"
        );
        assert!(raw.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        let source = s.active().unwrap().source.clone();
        let state = initial.xcf_state.as_ref().unwrap();
        let node = &state.filter_stacks[0].nodes[0];
        assert_eq!(node.operation.schema_epoch, "xcf-22-stock-gaussian-replace");
        assert_eq!(node.operation.stored_version, "0:0");
        assert_eq!(node.arguments.len(), 5);
        let outer = node.composition.as_ref().unwrap();
        assert_eq!(outer.compatibility_epoch, "xcf-22-stock-gaussian-replace");
        assert_eq!((outer.stored_blend_space, outer.stored_composite_space, outer.stored_operator), (-856260015, -856260015, -856260015));
        assert!(outer.resolved.is_none());
        assert!(node.mask.is_some());
        let request = json!({"filterId":id(&s).0.to_string()});
        let reply = crate::accounted_filter_inspect::prepare(&s, &request, &ctl()).unwrap();
        assert_eq!(reply.value()["filterId"], id(&s).0.to_string());
        assert_eq!(reply.value()["argumentCount"], 5);
        assert!(reply.quota().shares_ledger(s.active().unwrap().doc.quota().unwrap()));
        drop(reply);
        let x = if case == 0 { 3.75 } else { 2.25 };
        let patch = wire(&s, x);
        assert_eq!(s.execute("xcf.filter.set", patch.clone()).unwrap()["updated"], true);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        assert_eq!(native(&s), raw);
        let changed = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        assert_eq!(s.execute("xcf.filter.set", patch).unwrap()["updated"], false);
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(s.undo_with_control(&ctl()).unwrap());
        let history = s.active().unwrap().history.clone();
        let same = wire(&s, entry.2);
        assert_eq!(s.execute("xcf.filter.set", same).unwrap()["updated"], false);
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert!(s.active().unwrap().history.can_redo());
        assert!(s.redo_with_control(&ctl()).unwrap());
        assert_eq!(s.active().unwrap().doc.xcf_state, changed.xcf_state);
        let toggle = json!({"filterId":id(&s).0.to_string(),"enabled":false});
        assert_eq!(s.execute("xcf.filter.enable", toggle).unwrap()["updated"], true);
        let disabled = s.active().unwrap().doc.clone();
        assert!(!disabled.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        assert_eq!(native(&s), raw);
        assert_eq!(s.active().unwrap().source, source);
        assert_eq!(disabled.icc_profile, initial.icc_profile);
        assert_eq!(disabled.source_color_context, initial.source_color_context);
        assert_eq!(disabled.xcf_state.as_ref().unwrap().filter_masks, state.filter_masks);
        let q = s.active().unwrap().doc.quota().unwrap().clone();
        let mut reopened = Session::new();
        reopened.add_owned_native_point_archive(archive(&s), "CurrentGaussian.pcraft", Some("actual/CurrentGaussian.pcraft"), &ctl()).unwrap();
        assert_eq!(reopened.active().unwrap().doc.xcf_state, disabled.xcf_state);
        assert_eq!(native(&reopened), raw);
        assert!(reopened.active().unwrap().doc.quota().unwrap().shares_ledger(&q));
        let toggle = json!({"filterId":id(&reopened).0.to_string(),"enabled":true});
        assert_eq!(reopened.execute("xcf.filter.enable", toggle).unwrap()["updated"], true);
    }
}
#[test]
#[ignore = "UNRUN: originalGaussian typed semantic refusal, stale prepared recipe and finite same-ledger cancellation/quota rollback"]
fn original_gaussian_refusal_never_applies_a_supported_prefix() {
    let mut s = session(0);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    for value in [1.0f32, 4.0, f32::INFINITY] {
        let p = wire(&s, value);
        assert!(s.execute("xcf.filter.set", p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.journal, journal);
    }
    let p = json!({"filterId":id(&s).0.to_string(),"arguments":[{"name":"std-dev-x","value":{"Number":{"Float32Bits":3.75f32.to_bits()}}},{"name":"filter","value":{"Enum":{"domain":"gegl:gaussian-blur::filter","value":1}}}]});
    assert!(s.execute("xcf.filter.set", p).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let p = wire(&s, 3.75);
    let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.filter.set", &p, &ctl()).unwrap();
    s.active_mut().unwrap().history.max_states += 1;
    assert!(prepared.execute(&mut s, &ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    s.active_mut().unwrap().history.max_states = history.max_states;
    let spent = AtomicU64::new(0);
    let work = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    validate_current_owned_filter_storage(before.accounted().unwrap(), &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap();
    let cost = spent.load(Ordering::Relaxed);
    let spent = AtomicU64::new(0);
    let work = |n| if spent.fetch_add(n, Ordering::Relaxed).saturating_add(n) > cost + cost / 4 { Err(BackingError::Cancelled) } else { Ok(()) };
    let p = wire(&s, 3.75);
    assert!(
        crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.filter.set", &p, &ReadControl { interrupt: &Interrupt::NONE, work: &work })
            .and_then(|prepared| prepared.execute(&mut s, &ReadControl { interrupt: &Interrupt::NONE, work: &work }))
            .is_err()
    );
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(s.journal, journal);
    let q = before.quota().unwrap();
    let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.execute("xcf.filter.set", wire(&s, 3.75)).is_err());
    drop(hold);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(s.journal, journal);
    let node = &s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert!(
        matches!(node.arguments.iter().find(|a|a.name=="std-dev-x").unwrap().value,FilterValue::Number(ExactNumber::Float32Bits(bits)) if bits==2.25f32.to_bits())
    );
}
