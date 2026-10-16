//! Deferred genuine hierarchy Source portable writer/native13 and complete-tree
//! internal renderer continuity contracts. No producer numerical acceptance.
use super::*;
#[path = "accounted_normal_hierarchy_test_support.rs"]
mod fixture;
use photocraft_doc::{accounted_document::PointFilterRemovePatch, item_identity::ItemRef};
use std::{
    io::{Seek, Write},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

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
#[ignore = "UNRUN/uncompiled: genuine12 normal hierarchy sources; portable native writer typed early Work/no output/refund; exact marker13 and wrong old/generic handoff denial. No numerical oracle."]
fn normal_hierarchy_portable_writer_preserves_typed_refusal_and_marker() {
    for case in fixture::cases() {
        let source = fixture::import(&case);
        let owner = source.document();
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
        fixture::observed(document, &case);
    }
}

#[test]
#[ignore = "UNRUN/uncompiled: remove all genuine12 operative filters; retain actual empty-stack class and marker13 through repeated portable native saves/reopen; old handoffs stay closed."]
fn normal_hierarchy_empty_current_retains_its_native_purpose() {
    for case in fixture::cases() {
        let source = fixture::import(&case);
        let mut current = source.document().clone();
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
#[ignore = "UNRUN/uncompiled: genuine12 normal hierarchy owners route through the shared document renderer, match the dedicated facade word-for-word and refuse a foreign supplied basis. No producer numerical oracle."]
fn normal_hierarchy_general_renderer_requires_its_actual_owner_and_basis() {
    for case in fixture::cases() {
        let source = fixture::import(&case);
        let owner = source.document();
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let before = owner.quota().usage().unwrap();
        assert!(
            photocraft_compose::backed_document::PreparedBackedDocument::new_accounted(
                owner,
                &photocraft_cms::Builtin::DisplayP3.profile(),
                Default::default(),
                &fixture::ctl(),
            )
            .is_err()
        );
        assert_eq!(owner.quota().usage().unwrap(), before);
        {
            let general =
                photocraft_compose::backed_document::PreparedBackedDocument::new_accounted(owner, &profile, Default::default(), &fixture::ctl()).unwrap();
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
