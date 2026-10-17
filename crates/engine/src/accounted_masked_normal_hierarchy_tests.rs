//! Synthetic masked hierarchy CURRENT contracts over genuinely retained masked-flat
//! Sources. No producer-authored nested mask or numerical acceptance claim.
use super::*;
use crate::accounted_native_open::{AccountedNativeOpenPolicy, is_owned_current_filtered_flat_candidate, is_owned_current_normal_hierarchy_candidate};
use photocraft_raster::backing::{BackingError, ReadControl};
#[path = "accounted_masked_normal_hierarchy_test_support.rs"]
mod fixture;
use photocraft_doc::{accounted_document::PointFilterRemovePatch, item_identity::ItemRef};
use std::{
    io::{Seek, Write},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[test]
#[ignore = "UNRUN/uncompiled: synthetic nested masked CURRENT exact scalar controls, inactive nonempty opacity refusal, history and group metadata/disclosure; source custody only, no producer masked hierarchy oracle"]
fn masked_normal_hierarchy_nested_targets_keep_full_current_proof() {
    for case in fixture::cases() {
        let owner = fixture::build(&case);
        let mut session = Session::new();
        session.add_owned_native_normal_hierarchy_archive(fixture::archive(&owner), "masked-tree.pcraft", None, &fixture::ctl()).unwrap();
        let original = session.active().unwrap().doc.clone();
        for stack in &original.xcf_state.as_ref().unwrap().filter_stacks {
            let ItemRef::Layer(layer) = stack.owner else { panic!("leaf stack") };
            for node in &stack.nodes {
                assert!(crate::normal_hierarchy_filter_edit_candidate(original.accounted().unwrap(), layer, node));
                let mut params = serde_json::json!({"filterId":node.id.0.to_string(),"enabled":!node.enabled});
                let changes: &[(&str, f32)] = match node.operation.name.as_str() {
                    "gegl:exposure" => &[("exposure", 0.5), ("black-level", 0.0)],
                    "gegl:brightness-contrast" => &[("brightness", 0.125), ("contrast", 0.25)],
                    "gegl:levels" => &[("in-low", 0.05), ("in-high", 0.9), ("out-low", 0.1), ("out-high", 0.8)],
                    _ => &[],
                };
                if !changes.is_empty() {
                    params["arguments"] =
                        changes.iter().map(|(name, value)| serde_json::json!({"name":name,"value":{"Number":{"Float32Bits":value.to_bits()}}})).collect();
                }
                session.execute("xcf.filter.set", params).unwrap();
                fixture::payload(&original, &session.active().unwrap().doc);
                let changed = session.active().unwrap().doc.clone();
                let epoch = node.operation.schema_epoch.as_str();
                let nonempty = matches!(
                    epoch,
                    photocraft_compose::EXPOSURE_MASKED_MIXED_EPOCH
                        | photocraft_compose::INVERT_LINEAR_MASKED_MIXED_EPOCH
                        | photocraft_compose::INVERT_GAMMA_MASKED_MIXED_EPOCH
                        | photocraft_compose::BRIGHTNESS_CONTRAST_MASKED_MIXED_EPOCH
                        | photocraft_compose::GEGL_LEVELS_MASKED_MIXED_EPOCH
                );
                if nonempty {
                    // Request validation must still refuse this when the target
                    // was disabled by the preceding successful transaction.
                    assert!(
                        session
                            .execute(
                                "xcf.filter.set",
                                serde_json::json!({"filterId":node.id.0.to_string(),"enabled":false,"opacityF64Bits":"0x3fe0000000000000"})
                            )
                            .is_err()
                    );
                    assert!(session.active().unwrap().doc.shares_state(&changed));
                }
                session.execute("edit.undo", serde_json::json!({})).unwrap();
                fixture::same(&original, &session.active().unwrap().doc);
                session.execute("edit.redo", serde_json::json!({})).unwrap();
                fixture::same(&changed, &session.active().unwrap().doc);
                session.execute("edit.undo", serde_json::json!({})).unwrap();
            }
        }
        session.execute("layer.setProps", serde_json::json!({"layer":fixture::OUTER.0,"opacity":0.375,"visible":false})).unwrap();
        session.execute("layer.setExpanded", serde_json::json!({"all":true,"expanded":false})).unwrap();
        let edited = session.active().unwrap().doc.clone();
        fixture::payload(&original, &edited);
        let mut reopened = Session::new();
        reopened.add_owned_native_normal_hierarchy_archive(fixture::archive(edited.accounted().unwrap()), "continued.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(&edited, &reopened.active().unwrap().doc);
    }
}

#[derive(Debug)]
struct CountWriter {
    bytes: std::io::Cursor<Vec<u8>>,
    writes: Arc<AtomicU64>,
}
impl Write for CountWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.bytes.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.bytes.flush()
    }
}
impl Seek for CountWriter {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        self.bytes.seek(position)
    }
}

#[test]
#[ignore = "UNRUN/uncompiled: synthetic nested CURRENT over genuine8 masked-flat sources normal hierarchy sources; portable native writer typed early Work/no output/refund; exact marker13 and wrong old/generic handoff denial. No numerical oracle."]
fn masked_normal_hierarchy_portable_writer_preserves_typed_refusal_and_marker() {
    for case in fixture::cases() {
        let current = fixture::build(&case);
        let owner = &current;
        assert!(owner.checked_readonly_normal_hierarchy_index(&fixture::ctl()).is_err());
        let before = owner.quota().usage().unwrap();
        let calls = AtomicU64::new(0);
        let work = |_| if calls.fetch_add(1, Ordering::Relaxed) >= 6 { Err(BackingError::Work) } else { Ok(()) };
        let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &work };
        let writes = Arc::new(AtomicU64::new(0));
        let error = photocraft_io::accounted_current_native::save_accounted_current_archive(
            owner,
            CountWriter { bytes: std::io::Cursor::new(Vec::new()), writes: writes.clone() },
            Default::default(),
            owner.quota(),
            &c,
        )
        .unwrap_err();
        let photocraft_format::FormatError::Io(error) = error else { panic!("typed backing refusal") };
        assert_eq!(error.get_ref().and_then(|e| e.downcast_ref::<BackingError>()), Some(&BackingError::Work));
        assert_eq!(writes.load(Ordering::Relaxed), 0);
        assert_eq!(owner.quota().usage().unwrap(), before);
        assert!(is_owned_current_normal_hierarchy_candidate(&fixture::archive(owner), &fixture::ctl()).unwrap());
        assert!(!is_owned_current_filtered_flat_candidate(&fixture::archive(owner), &fixture::ctl()).unwrap());
        let mut wrong = Session::new();
        assert!(wrong.add_owned_native_masked_filtered_flat_archive(fixture::archive(owner), "hierarchy.pcraft", None, &fixture::ctl()).is_err());
        assert!(wrong.add_owned_native_filtered_flat_archive(fixture::archive(owner), "mixed.pcraft", None, &fixture::ctl()).is_err());
        assert!(wrong.add_owned_native_point_archive(fixture::archive(owner), "mixed.pcraft", None, &fixture::ctl()).is_err());
        assert!(
            wrong
                .add_owned_native_archive(fixture::archive(owner), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "mixed.pcraft", None, &fixture::ctl())
                .is_err()
        );
        assert!(wrong.documents().is_empty());
        let mut right = Session::new();
        right.add_owned_native_normal_hierarchy_archive(fixture::archive(owner), "mixed.pcraft", None, &fixture::ctl()).unwrap();
        let document = &right.active().unwrap().doc;
        assert!(document.accounted().unwrap().is_readonly_normal_hierarchy_storage());
        fixture::same(owner.document(), document);
    }
}

#[test]
#[ignore = "UNRUN/uncompiled: remove all synthetic nested CURRENT over genuine8 masked-flat sources operative filters; retain actual empty-stack class and marker13 through repeated portable native saves/reopen; old handoffs stay closed."]
fn masked_normal_hierarchy_empty_current_retains_its_native_purpose() {
    for case in fixture::cases() {
        let mut current = fixture::build(&case);
        loop {
            let target = current.document().xcf_state.as_ref().unwrap().filter_stacks.iter().find_map(|s| {
                let ItemRef::Layer(layer) = s.owner else { return None };
                s.nodes.first().map(|n| PointFilterRemovePatch { layer, filter: n.id, expected_stack_revision: s.revision })
            });
            let Some(target) = target else { break };
            current = current.try_remove_readonly_normal_hierarchy_filter(target, &fixture::ctl()).unwrap();
        }
        assert!(current.is_readonly_normal_hierarchy_storage());
        assert!(!current.is_readonly_filtered_flat_point_storage());
        assert!(!current.is_readonly_masked_filtered_flat_point_storage());
        assert!(!current.is_readonly_point_storage());
        let state = current.document().xcf_state.as_ref().unwrap();
        assert!(state.filter_stacks.iter().all(|s| s.nodes.is_empty()));
        assert!(state.filter_masks.is_empty());
        for _ in 0..2 {
            let archive = fixture::archive(&current);
            assert!(is_owned_current_normal_hierarchy_candidate(&archive, &fixture::ctl()).unwrap());
            assert!(!is_owned_current_filtered_flat_candidate(&archive, &fixture::ctl()).unwrap());
            assert!(fixture::archive(&current).into_accounted_readonly_filtered_flat_storage(&fixture::ctl()).is_err());
            assert!(fixture::archive(&current).into_accounted_readonly_point_storage(&fixture::ctl()).is_err());
            current = archive.into_accounted_readonly_normal_hierarchy_storage(&fixture::ctl()).unwrap();
            assert!(current.is_readonly_normal_hierarchy_storage());
            let prepared =
                photocraft_compose::backed_normal_hierarchy_document::PreparedBackedNormalHierarchyDocument::new(&current, Default::default(), &fixture::ctl())
                    .unwrap();
            prepared.validate_operative(&fixture::ctl()).unwrap();
        }
    }
}

#[test]
#[ignore = "UNRUN/uncompiled: synthetic nested CURRENT over genuine8 masked-flat sources normal hierarchy owners route through the shared document renderer, match the dedicated facade word-for-word and refuse a foreign supplied basis. No producer numerical oracle."]
fn masked_normal_hierarchy_general_renderer_requires_its_actual_owner_and_basis() {
    for case in fixture::cases() {
        let current = fixture::build(&case);
        let owner = &current;
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let before = owner.quota().usage().unwrap();
        assert!(
            photocraft_compose::backed_document::PreparedBackedDocument::new_accounted(
                owner,
                photocraft_cms::Builtin::DisplayP3.profile(),
                Default::default(),
                &fixture::ctl(),
            )
            .is_err()
        );
        assert_eq!(owner.quota().usage().unwrap(), before);
        {
            let general =
                photocraft_compose::backed_document::PreparedBackedDocument::new_accounted(owner, profile, Default::default(), &fixture::ctl()).unwrap();
            let direct =
                photocraft_compose::backed_normal_hierarchy_document::PreparedBackedNormalHierarchyDocument::new(owner, Default::default(), &fixture::ctl())
                    .unwrap();
            let a = general.render_band(owner.document().bounds(), &fixture::ctl()).unwrap();
            let b = direct.render_band(owner.document().bounds(), &fixture::ctl()).unwrap();
            assert_eq!(a.rect(), b.rect());
            assert_eq!(a.pixels().len(), b.pixels().len());
            for (a, b) in a.pixels().iter().zip(b.pixels()) {
                assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
            }
        }
        assert_eq!(owner.quota().usage().unwrap(), before);
    }
}
