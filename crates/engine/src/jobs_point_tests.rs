//! UNRUN registered Point routing drafts. Corpus input is independently pinned;
//! these tests exercise current command routing, not additional source authority.
use super::*;
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits},
    },
};
use sha2::{Digest, Sha256};
use std::io::Cursor;

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn session() -> Session {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-replace-geometry-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "e36bcddcea3021cff305175f70d1bd44f865478d8019a48dcb5c912b9f83bf72");
    let bytes = std::fs::read(root.join("brightness-gray-u16_linear.negative-x.author.xcf")).unwrap();
    assert_eq!(hash(&bytes), "819379f87a2d4ff41fffc9ee31741264e5c6ebc9a757dd52a5f14701c9aa555c");
    let quota = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let imported = photocraft_io::import_owned_xcf_from_reader_in_quota(
        "point.xcf",
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        quota,
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::Point(imported) = imported else { panic!("expected actual Point family") };
    let mut session = Session::new();
    session.add_owned_point_xcf_import(&imported, "point.xcf", None, &ctl()).unwrap();
    session
}

#[test]
#[ignore = "UNRUN: pinned Point original and registered actual job-start refusal"]
fn point_start_refuses_before_closed_wire_and_keeps_snapshot_history_and_journal() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let past = s.active().unwrap().history.past_len();
    let journal = s.journal.len();
    // Deliberately malformed nested wire: synchronous refusal must precede
    // any generic filter parser, clone, paid recipe or job worker.
    for (id, params) in [
        ("layer.setProps", json!({"name":"point", "unsupported":[[[[[0]]]]]})),
        ("xcf.filter.set", json!({"filterId":"not-an-id", "arguments":[[[[[0]]]]]})),
        ("xcf.filter.enable", json!({"filterId":"not-an-id", "enabled":"wrong"})),
    ] {
        let error = s.start(id, params).unwrap_err();
        assert!(matches!(error, EngineError::BadParams { ref msg, .. } if msg.contains("synchronous")));
        assert!(!s.has_jobs());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.active().unwrap().history.past_len(), past);
        assert_eq!(s.journal.len(), journal);
    }
}

#[test]
#[ignore = "UNRUN: translated drawable does not contain the full Clear/Fill ROI; pixel-selection installation remains unsupported"]
fn point_fixed_native_roi_and_selection_refuse_out_of_scope_commands() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.len();
    for id in ["edit.clear", "edit.fill", "select.all"] {
        assert!(s.execute(id, json!({})).is_err(), "{id}");
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.journal.len(), journal);
        assert!(!s.has_jobs());
    }
}
