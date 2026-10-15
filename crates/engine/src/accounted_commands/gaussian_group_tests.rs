//! UNRUN genuine isolated small-Auto Gaussian group engine workflows. Source/native buffers are external harness captures; no new numerical oracle.
use super::*;
use photocraft_doc::{Layer, LayerContent};
use photocraft_format::owned_document::OwnedDocumentArchive;
use photocraft_raster::{
    Interrupt,
    backing::{
        NativePlaneDescriptor,
        quota::{BackingQuota, QuotaKind, QuotaLimits},
    },
    checked_view::NativeTileBytes,
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: [(&str, &str, usize); 2] = [
    ("composition-matrix/group-28-group-mask.xcf", "740064c2b008a9785801b9de4425f685dc2618a7fcc938d37728723dd10ccc49", 10313),
    ("composition-matrix/group-28-group-mask.edit.xcf", "bf9f61f61cedbfdd6e3e3c25cb5066e4deef0c46eef16629d6546cfd1018706b", 10514),
];
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn root() -> PathBuf {
    PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap())
}
fn imported(case: usize, q: &BackingQuota) -> photocraft_io::OwnedIsolatedGaussianGroupXcfImport {
    let (path, pin, len) = CASES[case];
    let bytes = std::fs::read(root().join(path)).unwrap();
    assert_eq!(bytes.len(), len);
    assert_eq!(hash(&bytes), pin);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let result = photocraft_io::import_owned_xcf_from_reader_in_quota(
        path,
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::IsolatedGaussianGroup(owner) = result else { panic!("single selected original group carrier") };
    owner
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
fn native(s: &Session) -> (Vec<NativePlaneDescriptor>, String) {
    fn visit(layers: &[Layer], depth: usize, descriptors: &mut Vec<NativePlaneDescriptor>, digest: &mut Sha256) {
        assert!(depth <= 1);
        for layer in layers {
            match &layer.content {
                LayerContent::BackedRaster(backed) => {
                    descriptors.push(backed.descriptor());
                    digest.update(backed.descriptor().default_bytes());
                    for coord in backed.operative_bounds().tiles() {
                        let pin = backed.checked_view().pin_tile(coord, &ctl()).unwrap();
                        digest.update(coord.tx.to_ne_bytes());
                        digest.update(coord.ty.to_ne_bytes());
                        match pin.encoded() {
                            NativeTileBytes::FullTile(bytes) => digest.update(bytes),
                            NativeTileBytes::DefaultPixel(pixel) => {
                                for _ in 0..65536 {
                                    digest.update(pixel);
                                }
                            }
                        }
                    }
                }
                LayerContent::Group(group) => visit(&group.children, depth + 1, descriptors, digest),
                _ => panic!("closed group content"),
            }
            if let Some(mask) = &layer.mask {
                digest.update(mask.surface.default_bytes());
                for (coordinate, tile) in mask.surface.tiles() {
                    digest.update(coordinate.tx.to_ne_bytes());
                    digest.update(coordinate.ty.to_ne_bytes());
                    digest.update(tile.bytes());
                }
            }
        }
    }
    let mut descriptors = Vec::new();
    let mut digest = Sha256::new();
    visit(&s.active().unwrap().doc.layers, 0, &mut descriptors, &mut digest);
    (descriptors, digest.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
fn archive(bytes: &[u8], q: &BackingQuota) -> OwnedDocumentArchive {
    photocraft_io::accounted_native::open_accounted_native_from_reader(&mut Cursor::new(bytes), bytes.len() as u64, Default::default(), q.clone(), &ctl())
        .unwrap()
}

fn session(case: usize) -> (Session, BackingQuota) {
    let q = quota();
    let source = imported(case, &q);
    let mut s = Session::new();
    s.add_owned_isolated_gaussian_group_xcf_import(&source, CASES[case].0, Some(CASES[case].0), &ctl()).unwrap();
    drop(source);
    (s, q)
}
fn id(s: &Session) -> FilterId {
    s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id
}
fn enable(s: &Session, value: bool) -> Value {
    serde_json::json!({"filterId":id(s).0.to_string(),"enabled":value})
}
#[test]
fn bounded_activation_wire_is_exact_for_both_existing_ids() {
    for cmd in ["xcf.filter.set", "xcf.filter.enable"] {
        assert!(wire(cmd, &serde_json::json!({"filterId":"18446744073709551615","enabled":false})).is_some());
        for p in [
            serde_json::json!({"filterId":"01","enabled":true}),
            serde_json::json!({"filterId":"0","enabled":true}),
            serde_json::json!({"filterId":"18446744073709551616","enabled":true}),
            serde_json::json!({"filterId":"1","enabled":0}),
            serde_json::json!({"filterId":"1","enabled":true,"arguments":[]}),
            serde_json::json!({"filterId":"1","enabled":true,"opacityF64Bits":"0x3ff0000000000000"}),
        ] {
            assert!(wire(cmd, &p).is_none());
        }
    }
    assert!(wire("xcf.filter.move", &serde_json::json!({"filterId":"1","enabled":false})).is_none());
}
#[test]
#[ignore = "UNRUN: two genuine mode28 packets through both existing activation IDs; exact group label/raw conservation, one undo/noopredo and disabled native continuation"]
fn actual_group_activation_keeps_source_raw_tuples_and_native_owners() {
    for case in 0..CASES.len() {
        let (mut s, q) = session(case);
        let initial = s.active().unwrap().doc.clone();
        let raw = native(&s);
        let source = s.active().unwrap().source.clone();
        let cmd = if case == 0 { "xcf.filter.enable" } else { "xcf.filter.set" };
        let p = enable(&s, false);
        assert_eq!(s.execute(cmd, p.clone()).unwrap()["updated"], true);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Set GIMP Group Filter Enabled"));
        let mut expected = initial.xcf_state.clone().unwrap();
        expected.filter_stacks[0].nodes[0].enabled = false;
        expected.filter_stacks[0].revision += 1;
        assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap(), &expected);
        assert_eq!(native(&s), raw);
        assert_eq!(s.active().unwrap().source, source);
        assert_eq!(s.active().unwrap().doc.icc_profile, initial.icc_profile);
        assert_eq!(s.active().unwrap().doc.source_color_context, initial.source_color_context);
        assert!(std::sync::Arc::ptr_eq(&initial.layers[1].mask.as_ref().unwrap().surface, &s.active().unwrap().doc.layers[1].mask.as_ref().unwrap().surface));
        assert!(s.undo_with_control(&ctl()).unwrap());
        let h = s.active().unwrap().history.clone();
        let rev = s.active().unwrap().revision;
        let same = enable(&s, true);
        assert_eq!(s.execute(cmd, same).unwrap()["updated"], false);
        assert_eq!(s.active().unwrap().revision, rev);
        assert!(s.active().unwrap().history.shares_accounted_history(&h));
        assert!(s.active().unwrap().history.can_redo());
        assert!(s.redo_with_control(&ctl()).unwrap());
        let bytes = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "DisabledGaussian.pcraft", &ctl())
            .unwrap()
            .write_and_publish(&mut s, &ctl(), |snapshot, _| {
                photocraft_format::owned_document::save_current_document_archive(snapshot, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                    .map(|w| w.into_inner())
                    .map_err(|e| EngineError::Other(e.to_string()))
            })
            .unwrap();
        let mut reopened = Session::new();
        reopened.add_owned_native_isolated_gaussian_group_archive(archive(&bytes, &q), "current", Some("actual/DisabledGaussian.pcraft"), &ctl()).unwrap();
        assert_eq!(native(&reopened), raw);
        assert_eq!(reopened.active().unwrap().doc.xcf_state, s.active().unwrap().doc.xcf_state);
        let p = enable(&reopened, true);
        assert_eq!(reopened.execute("xcf.filter.enable", p).unwrap()["updated"], true);
    }
}
// External test harness creates a CURRENT native metadata control. It does not
// claim an authored lock/disabled packet or use the generic structural Session
// adoption bypass. The supplemental clone receipts and strict context pay on the real q.
fn current_control(lock: usize, alternate: bool) -> (Session, BackingQuota) {
    let (s, q) = session(0);
    let original = s.active().unwrap().doc.accounted().unwrap().clone();
    // Actual model-capacity census is an external harness planning capture.
    // Pay the clone before allocation; original fees cannot pay two containers.
    let model = photocraft_io::retained_native_model_census(original.document(), Default::default(), &Interrupt::NONE).unwrap();
    let masks = &original.document().xcf_state.as_ref().unwrap().filter_masks;
    let maps = masks.iter().map(|m| m.surface.tile_count() as u64).sum::<u64>() * 256;
    let index_envelope = (model.retained_bytes() as u64).checked_mul(2).and_then(|n| n.checked_add(maps * 2 + 8192)).unwrap();
    let payload_envelope = (original.document().icc_profile.as_ref().map_or(0, Vec::capacity) as u64)
        .checked_add(masks.iter().map(|m| m.surface.resident_payload_bytes().unwrap() as u64).sum::<u64>())
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_add(8192))
        .unwrap();
    let clone_index = q.reserve(QuotaKind::Index, index_envelope).unwrap();
    let clone_payload = q.reserve(QuotaKind::Resident, payload_envelope).unwrap();
    let mut doc = original.document().clone();
    let actual = photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert!((actual.retained_bytes() as u64).checked_add(maps).unwrap() <= clone_index.bytes());
    assert!(doc.icc_profile.as_ref().map_or(0, Vec::capacity) as u64 <= clone_payload.bytes());
    let group = doc.layers[1].id;
    match lock {
        1 => doc.layers[1].locks.all = true,
        2 => doc.layers[1].locks.pixels = true,
        3 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(group)).unwrap().locks.content = true,
        4 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(group)).unwrap().locks.visibility = true,
        _ => {}
    }
    if alternate {
        let n = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        n.enabled = false;
        let c = n.composition.as_mut().unwrap();
        c.stored_blend_space = -1474592175;
        c.stored_composite_space = -1474592175;
        c.stored_operator = -1474592175;
    }
    let owner =
        AccountedDocument::from_owned_readonly_gaussian_group(doc, std::sync::Arc::new((original, clone_index, clone_payload)), q.clone(), &ctl()).unwrap();
    validate_recipe(&owner, &ctl()).unwrap();
    let mut current = Session::new();
    let states = current.prefs().performance.history_states.max(1) as usize;
    let state = crate::DocState::new_accounted(owner, None, states, &ctl()).unwrap();
    current.publish_document_state(state, &ctl()).unwrap();
    drop(s);
    (current, q)
}
#[test]
#[ignore = "UNRUN: actual paid CURRENT native/content/visibility locks and existing false-only alternate tuple; no rawtuple rewrite"]
fn group_activation_preserves_locks_and_disabled_only_tuple() {
    for lock in 1..=4 {
        let (mut s, _) = current_control(lock, false);
        let before = s.active().unwrap().doc.clone();
        let h = s.active().unwrap().history.clone();
        let p = enable(&s, true);
        if lock < 4 {
            assert!(s.execute("xcf.filter.enable", p).is_err());
        } else {
            assert_eq!(s.execute("xcf.filter.enable", p).unwrap()["updated"], false);
        }
        let p = enable(&s, false);
        assert!(s.execute("xcf.filter.enable", p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&h));
    }
    let (mut s, _) = current_control(0, true);
    let before = s.active().unwrap().doc.clone();
    let p = enable(&s, false);
    assert_eq!(s.execute("xcf.filter.set", p).unwrap()["updated"], false);
    let p = enable(&s, true);
    assert!(s.execute("xcf.filter.enable", p).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
}
#[test]
#[ignore = "UNRUN: full prepared history/source/view/channel/journal fences, finite work/cancel/quota rollback before publication"]
fn group_activation_stale_and_resource_failures_leave_live_state_unchanged() {
    for fault in 0..5 {
        let (mut s, _) = session(0);
        let before = s.active().unwrap().doc.clone();
        let p = enable(&s, false);
        let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.filter.enable", &p, &ctl()).unwrap();
        match fault {
            0 => s.active_mut().unwrap().history.max_states += 1,
            1 => s.active_mut().unwrap().source_read_only = false,
            2 => s.active_mut().unwrap().view_revision += 1,
            3 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
            _ => s.journal.push(("external".into(), Value::Null)),
        }
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    let (mut s, q) = session(0);
    let before = s.active().unwrap().doc.clone();
    let h = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let p = enable(&s, false);
    let stop = || true;
    let c = ReadControl { interrupt: &Interrupt::cancel_only(&stop), work: &|_| Ok(()) };
    assert!(crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.filter.enable", &p, &c).is_err());
    let work = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.filter.enable", &p, &c).is_err());
    let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.execute("xcf.filter.enable", p).is_err());
    drop(hold);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&h));
    assert_eq!(s.journal, journal);
}

#[path = "gaussian_group_parameter_tests.rs"]
mod parameter_tests;
