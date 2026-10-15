//! UNRUN witnessed Point REMOVE2->1 drafts; external source/archive buffers are
//! harness captures, not production memory ownership claims.
use super::*;
use serde_json::json;
#[test]
fn remove_wire_is_exact_and_keeps_the_existing_shallow_journal_limit() {
    for id in ["1", "18446744073709551615"] {
        let p = json!({"filterId":id});
        assert!(remove_wire(&p).is_some());
        assert_eq!(journal_depth("xcf.filter.remove", &p), 4);
    }
    for p in [
        json!({}),
        json!({"filterId":"0"}),
        json!({"filterId":"01"}),
        json!({"filterId":"-1"}),
        json!({"filterId":1}),
        json!({"filterId":"18446744073709551616"}),
        json!({"filterId":"1","index":0}),
        json!({"filterId":"1","coalesce":null}),
    ] {
        assert!(remove_wire(&p).is_none());
    }
}
#[cfg(all(feature = "corpus", not(target_arch = "wasm32")))]
mod actual {
    use super::super::stack_tests::session;
    use super::*;
    use photocraft_doc::LayerContent;
    use photocraft_raster::{Interrupt, checked_view::NativeTileBytes};
    use sha2::{Digest, Sha256};
    use std::{io::Cursor, sync::Arc};
    fn ctl() -> ReadControl<'static> {
        ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
    }
    fn raster(s: &Session) -> String {
        let LayerContent::BackedRaster(layer) = &s.active().unwrap().doc.layers[0].content else { panic!("real native layer") };
        let mut digest = Sha256::new();
        digest.update(layer.descriptor().default_bytes());
        for coord in layer.descriptor().bounds().tiles() {
            let tile = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
            match tile.encoded() {
                NativeTileBytes::FullTile(b) | NativeTileBytes::DefaultPixel(b) => digest.update(b),
            }
        }
        digest.finalize().iter().map(|v| format!("{v:02x}")).collect()
    }
    #[test]
    #[ignore = "UNRUN: authentic witnessed REMOVE each stable ID, archival, one undo/noopredo and native singleton continuation"]
    fn removal_preserves_survivor_native_sources_and_archives_only_deleted_mask_records() {
        for name in ["stack.source.xcf", "stack.reverse.xcf", "stack.disable.xcf", "stack.opacity-half.xcf"] {
            for removed_index in 0..2 {
                let (mut s, source) = session(name);
                let source_hash = Sha256::digest(&source);
                let before = s.active().unwrap().doc.clone();
                let state = before.xcf_state.as_ref().unwrap();
                let stack = &state.filter_stacks[0];
                let removed = &stack.nodes[removed_index];
                let survivor = &stack.nodes[1 - removed_index];
                let id = removed.id.0.to_string();
                let raw = raster(&s);
                let source_identity = s.active().unwrap().source.clone();
                let p = json!({"filterId":id});
                assert_eq!(s.execute("xcf.filter.remove", p.clone()).unwrap(), json!({"filterId":id,"updated":true}));
                let changed = s.active().unwrap().doc.clone();
                let next = changed.xcf_state.as_ref().unwrap();
                assert_eq!(next.filter_stacks[0].nodes.as_slice(), std::slice::from_ref(survivor));
                assert_eq!(next.filter_stacks[0].revision, stack.revision + 1);
                assert_eq!(next.current_filter_output_precision, state.current_filter_output_precision);
                assert_eq!(next.provenance, state.provenance);
                assert_eq!(next.selection, state.selection);
                assert_eq!(next.opaque.len(), state.opaque.len());
                for (old, new) in state.opaque.iter().zip(&next.opaque) {
                    let deleted = state.filter_masks.iter().find(|m| m.owner == removed.id && old.owner == Some(ItemRef::Mask(m.id)));
                    if let Some(mask) = deleted {
                        assert_eq!(new.owner, None);
                        assert_eq!(new.identity, format!("deleted-filter-mask/{}/{}", mask.id.0, old.identity));
                    } else {
                        assert_eq!(new.owner, old.owner);
                        assert_eq!(new.identity, old.identity);
                    }
                    assert_eq!(new.namespace, old.namespace);
                    assert_eq!(new.order, old.order);
                    assert_eq!(new.flags, old.flags);
                    assert_eq!(new.source_version, old.source_version);
                    assert_eq!(new.rendering_required, old.rendering_required);
                    assert_eq!(new.blake3, old.blake3);
                    assert!(Arc::ptr_eq(&new.bytes, &old.bytes));
                }
                assert_eq!(next.filter_masks.len(), state.filter_masks.iter().filter(|m| m.owner != removed.id).count());
                for mask in &next.filter_masks {
                    assert_eq!(Some(mask), state.filter_masks.iter().find(|m| m.id == mask.id));
                    for (coord, tile) in mask.surface.tiles() {
                        let old = state.filter_masks.iter().find(|m| m.id == mask.id).unwrap();
                        assert!(Arc::ptr_eq(tile, old.surface.tiles().find(|(c, _)| *c == coord).unwrap().1));
                    }
                }
                assert!(next.items.iter().all(|i| !state.filter_masks.iter().any(|m| m.owner == removed.id && i.item == ItemRef::Mask(m.id))));
                assert_eq!(raster(&s), raw);
                assert_eq!(changed.icc_profile, before.icc_profile);
                assert_eq!(changed.source_color_context, before.source_color_context);
                assert_eq!(s.active().unwrap().source, source_identity);
                assert_eq!(s.active().unwrap().history.undo_label(), Some("Remove GIMP Stock Filter"));
                assert_eq!(s.active().unwrap().history.past_len(), 1);
                assert!(s.undo_with_control(&ctl()).unwrap());
                assert!(s.active().unwrap().doc.shares_state(&before));
                let history = s.active().unwrap().history.clone();
                assert_eq!(s.execute("xcf.filter.enable", json!({"filterId":id,"enabled":removed.enabled})).unwrap()["updated"], false);
                assert!(s.active().unwrap().history.shares_accounted_history(&history));
                assert!(s.redo_with_control(&ctl()).unwrap());
                assert!(s.active().unwrap().doc.shares_state(&changed));
                let history = s.active().unwrap().history.clone();
                assert!(s.execute("xcf.filter.remove", p).is_err());
                assert!(s.active().unwrap().doc.shares_state(&changed));
                assert!(s.active().unwrap().history.shares_accounted_history(&history));
                let q = changed.quota().unwrap().clone();
                let bytes = photocraft_format::owned_document::save_current_document_archive(&changed, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                    .unwrap()
                    .into_inner();
                let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
                    &mut Cursor::new(bytes.as_slice()),
                    bytes.len() as u64,
                    Default::default(),
                    q,
                    &ctl(),
                )
                .unwrap();
                let mut reopened = Session::new();
                reopened.add_owned_native_point_archive(archive, "removed.pcraft", None, &ctl()).unwrap();
                assert_eq!(reopened.active().unwrap().doc.xcf_state, changed.xcf_state);
                assert_eq!(raster(&reopened), raw);
                assert_eq!(
                    reopened.execute("xcf.filter.enable", json!({"filterId":survivor.id.0.to_string(),"enabled":!survivor.enabled})).unwrap()["updated"],
                    true
                );
                assert_eq!(Sha256::digest(&source), source_hash);
            }
        }
    }
    #[test]
    #[ignore = "UNRUN: authentic REMOVE invalid identity, background/coalesce, stale capture, cancellation and quota rollback"]
    fn refused_removal_does_not_change_history_journal_or_paid_payload() {
        let (mut s, _) = session("stack.source.xcf");
        let before = s.active().unwrap().doc.clone();
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[1].id.0.to_string();
        let p = json!({"filterId":id});
        let raw = raster(&s);
        let q = before.quota().unwrap().clone();
        let usage = q.usage().unwrap();
        let history = s.active().unwrap().history.clone();
        let revision = s.active().unwrap().revision;
        let journal = s.journal.clone();
        assert!(s.execute("xcf.filter.remove", json!({"filterId":"18446744073709551615"})).is_err());
        assert!(crate::xcf_filter_cmds::pre_dispatch_point_remove(&s, "xcf.filter.remove", &p, true).is_err());
        s.coalesce_request = Some("gesture".into());
        assert!(Request::parse(&s, "xcf.filter.remove", &p, &ctl()).is_err());
        assert_eq!(s.coalesce_request.as_deref(), Some("gesture"));
        s.coalesce_request = None;
        let prepared = Request::parse(&s, "xcf.filter.remove", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
        s.active_mut().unwrap().channel_view.quick_mask_hidden = true;
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        s.active_mut().unwrap().channel_view.quick_mask_hidden = false;
        let stop = Interrupt::cancel_only(&|| true);
        assert!(Request::parse(&s, "xcf.filter.remove", &p, &ReadControl { interrupt: &stop, work: &|_| Ok(()) }).is_err());
        let blocker = q.reserve(QuotaKind::Index, (32 << 20) - q.usage().unwrap().index).unwrap();
        assert!(Request::parse(&s, "xcf.filter.remove", &p, &ctl()).unwrap().prepare(&s, &ctl()).is_err());
        drop(blocker);
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.journal, journal);
        assert_eq!(raster(&s), raw);
        assert_eq!(q.usage().unwrap(), usage);
    }
}
