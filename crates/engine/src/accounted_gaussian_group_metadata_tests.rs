//! UNRUN genuine Gaussian-group metadata/physical-mask transactions. Captures are external harness buffers; product owners and CURRENT fixture clones retain actual quota guards.
use super::*;
use photocraft_doc::{Layer, LayerContent};
use photocraft_doc::{
    LayerId,
    accounted_document::AccountedDocument,
    item_identity::{ItemRef, MaskId},
};
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
    let owner = imported(case, &q);
    let mut s = Session::new();
    s.add_owned_isolated_gaussian_group_xcf_import(&owner, CASES[case].0, Some(CASES[case].0), &ctl()).unwrap();
    drop(owner);
    (s, q)
}
fn ids(s: &Session) -> [LayerId; 4] {
    let d = &s.active().unwrap().doc;
    let LayerContent::Group(g) = &d.layers[1].content else { panic!() };
    [d.layers[0].id, d.layers[1].id, g.children[0].id, g.children[1].id]
}
fn masks(s: &Session) -> (MaskId, MaskId) {
    let state = s.active().unwrap().doc.xcf_state.as_ref().unwrap();
    let filter = state.filter_masks[0].id;
    let physical = state
        .items
        .iter()
        .find_map(|i| match i.item {
            ItemRef::Mask(id) if id != filter => Some(id),
            _ => None,
        })
        .unwrap();
    (physical, filter)
}
#[test]
#[ignore = "UNRUN: both genuine mode28 sources and allfour IDs, combined rawzero/name/visibility, unchanged source/native/filter fields, exact name targeting and typed native continuation"]
fn gaussian_group_metadata_all_targets_and_duplicate_names() {
    for case in 0..CASES.len() {
        for target in 0..4 {
            let (mut s, q) = session(case);
            let before = s.active().unwrap().doc.clone();
            let words = native(&s);
            let source = s.active().unwrap().source.clone();
            let id = ids(&s)[target];
            assert!(crate::assigned_geometry_metadata_candidate(before.accounted().unwrap()));
            assert!(!crate::accounted_commands::supports("not-a-command"));
            let p = serde_json::json!({"layer":id.0,"name":format!("Current-{target}"),"visible":false,"opacity":-0.0});
            assert_eq!(s.execute("layer.setProps", p).unwrap(), serde_json::Value::Null);
            let access = photocraft_doc::accounted_document::accounted_layer_access(&s.active().unwrap().doc, id, &ctl()).unwrap().unwrap();
            assert_eq!(access.layer.name, format!("Current-{target}"));
            assert!(!access.layer.visible);
            assert_eq!(access.layer.opacity.to_bits(), (-0.0f32).to_bits());
            assert_eq!(s.active().unwrap().history.past_len(), 1);
            assert_eq!(native(&s), words);
            assert_eq!(s.active().unwrap().source, source);
            assert_eq!(s.active().unwrap().doc.icc_profile, before.icc_profile);
            assert_eq!(s.active().unwrap().doc.source_color_context, before.source_color_context);
            assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks, before.xcf_state.as_ref().unwrap().filter_stacks);
            let saved = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "MetadataGaussian.pcraft", &ctl())
                .unwrap()
                .write_and_publish(&mut s, &ctl(), |doc, _| {
                    photocraft_format::owned_document::save_current_document_archive(doc, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                        .map(|w| w.into_inner())
                        .map_err(|e| EngineError::Other(e.to_string()))
                })
                .unwrap();
            let mut reopened = Session::new();
            reopened.add_owned_native_isolated_gaussian_group_archive(archive(&saved, &q), "current", None, &ctl()).unwrap();
            assert_eq!(reopened.active().unwrap().doc.xcf_state, s.active().unwrap().doc.xcf_state);
            assert_eq!(native(&reopened), words);
        }
    }
    let (mut s, _) = session(0);
    let ids = ids(&s);
    for id in ids {
        s.execute("layer.setProps", serde_json::json!({"layer":id.0,"name":"same","coalesce":"duplicates"})).unwrap();
    }
    s.execute("layer.setProps", serde_json::json!({"layer":"same","visible":false})).unwrap();
    for (index, id) in ids.iter().enumerate() {
        let a = photocraft_doc::accounted_document::accounted_layer_access(&s.active().unwrap().doc, *id, &ctl()).unwrap().unwrap();
        assert_eq!(a.layer.visible, index != 3);
    }
}
#[test]
#[ignore = "UNRUN: opaque mixed-target gesture keeps firstlabel and no-op redo, directtyped labels and exact parser wrongtypes; Move remains unsupported"]
fn gaussian_group_metadata_gesture_and_direct_transaction() {
    let (mut s, _) = session(0);
    let ids = ids(&s);
    let initial = s.active().unwrap().doc.clone();
    s.execute("layer.setProps", serde_json::json!({"layer":ids[3].0,"visible":false,"coalesce":"same\u{0}key"})).unwrap();
    s.execute("layer.setProps", serde_json::json!({"layer":ids[0].0,"name":"Combined background","opacity":0.375,"coalesce":"same\u{0}key"})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Layer Visibility"));
    assert!(s.undo_with_control(&ctl()).unwrap());
    let h = s.active().unwrap().history.clone();
    let rev = s.active().unwrap().revision;
    s.execute("layer.setProps", serde_json::json!({"layer":ids[0].0,"name":false,"visible":"wrong","opacity":null,"coalesce":false})).unwrap();
    assert!(s.active().unwrap().doc.shares_state(&initial));
    assert_eq!(s.active().unwrap().revision, rev);
    assert!(s.active().unwrap().history.shares_accounted_history(&h));
    assert!(s.active().unwrap().history.can_redo());
    assert!(s.redo_with_control(&ctl()).unwrap());
    s.edit_accounted(
        "Direct Current Metadata",
        AccountedEdit::LayerMetadata { layer: ids[2], name: Some("Direct child"), visible: None, opacity: Some(0.25) },
        &ctl(),
    )
    .unwrap();
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Direct Current Metadata"));
    for p in [serde_json::json!({"dx":1,"dy":0}), serde_json::json!({"dx":0,"dy":0})] {
        assert!(s.execute("layer.translate", p).is_err());
    }
}
#[test]
#[ignore = "UNRUN: physical mask only despite shared owner; effectmask explicitrefusal/directlayer success, nojournal/noopredo and disabled native continuation"]
fn gaussian_physical_mask_never_targets_filter_selection() {
    for case in 0..CASES.len() {
        let (mut s, q) = session(case);
        let group = ids(&s)[1];
        let (physical, filter) = masks(&s);
        let before = s.active().unwrap().doc.clone();
        let raw = native(&s);
        let journal = s.journal.clone();
        let bad = serde_json::json!({"maskId":filter.0.to_string(),"enabled":false});
        assert!(s.execute("xcf.mask.enabled.set", bad).unwrap_err().to_string().contains("unsupported-filter-mask"));
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.journal, journal);
        let p = serde_json::json!({"maskId":physical.0.to_string(),"enabled":false});
        let reply = s.execute("xcf.mask.enabled.set", p).unwrap();
        assert_eq!(reply["changed"], true);
        assert_eq!(reply["layerId"], group.0.to_string());
        assert_eq!(s.journal, journal);
        assert_eq!(native(&s), raw);
        assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks, before.xcf_state.as_ref().unwrap().filter_stacks);
        assert!(!s.active().unwrap().doc.layers[1].mask.as_ref().unwrap().enabled);
        let state = s.active().unwrap().doc.xcf_state.as_ref().unwrap();
        assert!(!state.items.iter().find(|i| i.item == ItemRef::Mask(physical)).unwrap().mask.as_ref().unwrap().enabled);
        assert!(state.items.iter().find(|i| i.item == ItemRef::Mask(filter)).unwrap().mask.as_ref().unwrap().enabled);
        assert!(s.undo_with_control(&ctl()).unwrap());
        let h = s.active().unwrap().history.clone();
        let p = serde_json::json!({"maskId":physical.0.to_string(),"enabled":true});
        assert_eq!(s.execute("xcf.mask.enabled.set", p).unwrap()["changed"], false);
        assert!(s.active().unwrap().history.shares_accounted_history(&h));
        assert!(s.active().unwrap().history.can_redo());
        assert!(s.redo_with_control(&ctl()).unwrap());
        let saved = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "Physical-mask.pcraft", &ctl())
            .unwrap()
            .write_and_publish(&mut s, &ctl(), |doc, _| {
                photocraft_format::owned_document::save_current_document_archive(doc, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                    .map(|w| w.into_inner())
                    .map_err(|e| EngineError::Other(e.to_string()))
            })
            .unwrap();
        let mut reopened = Session::new();
        reopened.add_owned_native_isolated_gaussian_group_archive(archive(&saved, &q), "mask", None, &ctl()).unwrap();
        assert_eq!(native(&reopened), raw);
        reopened.edit_accounted("Direct physical mask", AccountedEdit::MaskEnabled { layer: group, enabled: true }, &ctl()).unwrap();
        assert_eq!(reopened.active().unwrap().history.undo_label(), Some("Direct physical mask"));
    }
}

fn current_control(lock: usize) -> (Session, BackingQuota) {
    let (s, q) = session(0);
    let original = s.active().unwrap().doc.accounted().unwrap().clone();
    // Actual model-capacity census is an external harness planning capture.
    // Pay the clone before allocation; original fees cannot pay two containers.
    let model = photocraft_io::retained_native_model_census(original.document(), Default::default(), &Interrupt::NONE).unwrap();
    let masks = &original.document().xcf_state.as_ref().unwrap().filter_masks;
    let maps = masks.iter().map(|m| m.surface.tile_count() as u64).sum::<u64>() * 256;
    let index_envelope = (model.retained_bytes() as u64).checked_mul(2).and_then(|n| n.checked_add(maps * 2 + 8192)).unwrap();
    let payload_envelope = (original.document().icc_profile.as_ref().map_or(0, |v| v.capacity()) as u64)
        .checked_add(masks.iter().map(|m| m.surface.resident_payload_bytes().unwrap()).sum::<u64>())
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_add(8192))
        .unwrap();
    let clone_index = q.reserve(QuotaKind::Index, index_envelope).unwrap();
    let clone_payload = q.reserve(QuotaKind::Resident, payload_envelope).unwrap();
    let mut doc = original.document().clone();
    let actual = photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert!((actual.retained_bytes() as u64).checked_add(maps).unwrap() <= clone_index.bytes());
    assert!(doc.icc_profile.as_ref().map_or(0, |v| v.capacity()) as u64 <= clone_payload.bytes());
    let group = doc.layers[1].id;
    match lock {
        1 => doc.layers[1].locks.all = true,
        2 => doc.layers[1].locks.pixels = true,
        3 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(group)).unwrap().locks.content = true,
        4 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(group)).unwrap().locks.visibility = true,
        5 => {
            let state = doc.xcf_state.as_mut().unwrap();
            let filter = state.filter_masks[0].id;
            state.items.iter_mut().find(|i| matches!(i.item,ItemRef::Mask(id) if id!=filter)).unwrap().locks.content = true;
        }
        _ => {}
    }
    let owner =
        AccountedDocument::from_owned_readonly_gaussian_group(doc, std::sync::Arc::new((original, clone_index, clone_payload)), q.clone(), &ctl()).unwrap();
    MetadataPolicy::GaussianGroup.validate_current(&owner, &ctl()).unwrap();
    let mut current = Session::new();
    let states = current.prefs().performance.history_states.max(1) as usize;
    let state = crate::DocState::new_accounted(owner, None, states, &ctl()).unwrap();
    current.publish_document_state(state, &ctl()).unwrap();
    drop(s);
    (current, q)
}
#[test]
#[ignore = "UNRUN: inherited visibility vs content/all mask locks before noop; full prepared fences and finite work/quota rollback"]
fn gaussian_metadata_locks_fences_and_failures() {
    for lock in [1, 3, 5] {
        let (mut s, _) = current_control(lock);
        let id = ids(&s)[2];
        s.execute("layer.setProps", serde_json::json!({"layer":id.0,"name":"Allowed metadata","opacity":0.5})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let p = serde_json::json!({"maskId":masks(&s).0.0.to_string(),"enabled":true});
        assert!(s.execute("xcf.mask.enabled.set", p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    let (mut s, _) = current_control(4);
    let id = ids(&s)[2];
    assert!(s.execute("layer.setProps", serde_json::json!({"layer":id.0,"visible":false})).is_err());
    s.execute("layer.setProps", serde_json::json!({"layer":id.0,"visible":true,"name":"Content permitted","opacity":0.5})).unwrap();
    for mask in [false, true] {
        for fault in 0..5 {
            let (mut s, _) = session(0);
            let before = s.active().unwrap().doc.clone();
            let (cmd, p) = if mask {
                ("xcf.mask.enabled.set", serde_json::json!({"maskId":masks(&s).0.0.to_string(),"enabled":false}))
            } else {
                ("layer.setProps", serde_json::json!({"layer":ids(&s)[3].0,"name":"Staged"}))
            };
            let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, cmd, &p, &ctl()).unwrap();
            match fault {
                0 => s.active_mut().unwrap().history.max_states += 1,
                1 => s.active_mut().unwrap().source_read_only = false,
                2 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
                3 => s.active_mut().unwrap().view_revision += 1,
                _ => s.journal.push(("external".into(), serde_json::Value::Null)),
            };
            assert!(prepared.execute(&mut s, &ctl()).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
        }
    }
    let (mut s, q) = session(0);
    let before = s.active().unwrap().doc.clone();
    let h = s.active().unwrap().history.clone();
    let j = s.journal.clone();
    let p = serde_json::json!({"layer":ids(&s)[0].0,"opacity":0.25});
    let work = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &c).is_err());
    let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.execute("layer.setProps", p).is_err());
    drop(hold);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&h));
    assert_eq!(s.journal, j);
}
