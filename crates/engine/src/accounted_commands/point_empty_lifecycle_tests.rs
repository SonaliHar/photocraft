//! UNRUN empty witnessed Point lifecycle. Source/archive Vecs are external
//! harness captures; genuine decoder/native/model owners retain their fees.
use super::stack_tests::session;
use super::*;
use photocraft_doc::{FilterOutputPrecision, LayerContent};
use photocraft_raster::{Interrupt, checked_view::NativeTileBytes};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::Cursor;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn raster(s: &Session) -> String {
    let LayerContent::BackedRaster(layer) = &s.active().unwrap().doc.layers[0].content else { panic!("real native owner") };
    let mut h = Sha256::new();
    h.update(layer.descriptor().default_bytes());
    for coord in layer.descriptor().bounds().tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        match pin.encoded() {
            NativeTileBytes::FullTile(b) | NativeTileBytes::DefaultPixel(b) => h.update(b),
        }
    }
    h.finalize().iter().map(|v| format!("{v:02x}")).collect()
}
#[test]
#[ignore = "UNRUN: genuine witnessed sequential removals, half survivor, empty identity/history/native reopen"]
fn sequential_remove_retains_empty_stack_and_native_current_identity_without_reclassification() {
    for name in ["stack.source.xcf", "stack.reverse.xcf", "stack.opacity-half.xcf"] {
        for first in 0..2 {
            let (mut s, source) = session(name);
            let sourcehash = Sha256::digest(&source);
            let initial = s.active().unwrap().doc.clone();
            let initial_state = initial.xcf_state.as_ref().unwrap();
            let stack = &initial_state.filter_stacks[0];
            let firstid = stack.nodes[first].id.0.to_string();
            let last = &stack.nodes[1 - first];
            let lastid = last.id.0.to_string();
            let raw = raster(&s);
            let source_identity = s.active().unwrap().source.clone();
            assert_eq!(s.execute("xcf.filter.remove", json!({"filterId":firstid})).unwrap()["updated"], true);
            let single = s.active().unwrap().doc.clone();
            assert_eq!(single.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.as_slice(), std::slice::from_ref(last));
            let revision = s.active().unwrap().revision;
            assert_eq!(s.execute("xcf.filter.remove", json!({"filterId":lastid})).unwrap(), json!({"filterId":lastid,"updated":true}));
            let empty = s.active().unwrap().doc.clone();
            assert!(empty.accounted().unwrap().is_readonly_point_storage());
            let state = empty.xcf_state.as_ref().unwrap();
            assert_eq!(state.filter_stacks.len(), 1);
            assert_eq!(state.filter_stacks[0].owner, stack.owner);
            assert!(state.filter_stacks[0].nodes.is_empty());
            assert_eq!(state.filter_stacks[0].revision, stack.revision + 2);
            assert!(state.filter_masks.is_empty());
            assert_eq!(state.items.len(), 1);
            assert_eq!(state.current_filter_output_precision, FilterOutputPrecision::Canonical);
            assert_eq!(state.provenance, initial_state.provenance);
            assert_eq!(state.opaque.len(), initial_state.opaque.len());
            for (old, new) in initial_state.opaque.iter().zip(&state.opaque) {
                assert!(std::sync::Arc::ptr_eq(&old.bytes, &new.bytes));
                assert_eq!(old.blake3, new.blake3);
            }
            assert_eq!(empty.depth, initial.depth);
            assert_eq!(empty.icc_profile, initial.icc_profile);
            assert_eq!(empty.source_color_context, initial.source_color_context);
            assert_eq!(empty.working_profile_interpretation, initial.working_profile_interpretation);
            assert_eq!(raster(&s), raw);
            assert_eq!(s.active().unwrap().source, source_identity);
            assert_eq!(s.active().unwrap().revision, revision + 1);
            assert_eq!(s.active().unwrap().history.past_len(), 2);
            assert_eq!(s.active().unwrap().history.undo_label(), Some("Remove GIMP Stock Filter"));
            assert!(s.undo_with_control(&ctl()).unwrap());
            assert!(s.active().unwrap().doc.shares_state(&single));
            let history = s.active().unwrap().history.clone();
            assert_eq!(s.execute("xcf.filter.enable", json!({"filterId":lastid,"enabled":last.enabled})).unwrap()["updated"], false);
            assert!(s.active().unwrap().history.shares_accounted_history(&history));
            assert!(s.redo_with_control(&ctl()).unwrap());
            assert!(s.active().unwrap().doc.shares_state(&empty));
            assert!(s.undo_with_control(&ctl()).unwrap());
            assert!(s.undo_with_control(&ctl()).unwrap());
            assert!(s.active().unwrap().doc.shares_state(&initial));
            assert!(s.redo_with_control(&ctl()).unwrap());
            assert!(s.redo_with_control(&ctl()).unwrap());
            let history = s.active().unwrap().history.clone();
            let journal = s.journal.clone();
            for (command, params) in [
                ("xcf.filter.remove", json!({"filterId":lastid})),
                ("xcf.filter.enable", json!({"filterId":lastid,"enabled":true})),
                ("xcf.filter.set", json!({"filterId":lastid,"opacityF64Bits":"0x3ff0000000000000"})),
                ("xcf.filter.move", json!({"filterId":lastid,"index":0})),
            ] {
                assert!(s.execute(command, params).is_err());
                assert!(s.active().unwrap().doc.shares_state(&empty));
                assert!(s.active().unwrap().history.shares_accounted_history(&history));
                assert_eq!(s.journal, journal);
            }
            let q = empty.quota().unwrap().clone();
            let saved = photocraft_format::owned_document::save_current_document_archive(&empty, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                .unwrap()
                .into_inner();
            let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
                &mut Cursor::new(saved.as_slice()),
                saved.len() as u64,
                Default::default(),
                q.clone(),
                &ctl(),
            )
            .unwrap();
            let mut reopened = Session::new();
            reopened.add_owned_native_point_archive(archive, "empty-current.pcraft", None, &ctl()).unwrap();
            let current = reopened.active().unwrap();
            assert!(current.doc.accounted().unwrap().is_readonly_point_storage());
            assert!(current.doc.quota().unwrap().shares_ledger(&q));
            assert_eq!(current.doc.xcf_state, empty.xcf_state);
            assert_eq!(raster(&reopened), raw);
            assert_eq!(current.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
            assert!(current.source.as_ref().unwrap().path.is_none());
            assert!(!current.source_read_only);
            assert_eq!(Sha256::digest(&source), sourcehash);
        }
    }
}
#[test]
#[ignore = "UNRUN: last-node cancellation, quota and stale fences preserve singleton payload/history before empty publication"]
fn failed_last_removal_keeps_singleton_and_undo_anchor() {
    let (mut s, _) = session("stack.opacity-half.xcf");
    let initial = s.active().unwrap().doc.clone();
    let nodes = &initial.xcf_state.as_ref().unwrap().filter_stacks[0].nodes;
    s.execute("xcf.filter.remove", json!({"filterId":nodes[0].id.0.to_string()})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let p = json!({"filterId":nodes[1].id.0.to_string()});
    let raw = raster(&s);
    let q = before.quota().unwrap().clone();
    let usage = q.usage().unwrap();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let revision = s.active().unwrap().revision;
    let stop = Interrupt::cancel_only(&|| true);
    assert!(Request::parse(&s, "xcf.filter.remove", &p, &ReadControl { interrupt: &stop, work: &|_| Ok(()) }).is_err());
    let prepared = Request::parse(&s, "xcf.filter.remove", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
    s.active_mut().unwrap().channel_view.quick_mask_hidden = true;
    assert!(prepared.execute(&mut s, &ctl()).is_err());
    s.active_mut().unwrap().channel_view.quick_mask_hidden = false;
    let fee = q.reserve(QuotaKind::Index, (32 << 20) - q.usage().unwrap().index).unwrap();
    assert!(Request::parse(&s, "xcf.filter.remove", &p, &ctl()).unwrap().prepare(&s, &ctl()).is_err());
    drop(fee);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.active().unwrap().revision, revision);
    assert_eq!(s.journal, journal);
    assert_eq!(raster(&s), raw);
    assert_eq!(q.usage().unwrap(), usage);
}

#[test]
#[ignore = "UNRUN: genuine paid empty storage remains structural, hidden/fractional CURRENT recipes fail named semantic validation"]
fn invalid_empty_current_recipe_is_not_published_or_treated_as_plain() {
    let (mut s, _) = session("stack.source.xcf");
    let initial = s.active().unwrap().doc.clone();
    let stack = &initial.xcf_state.as_ref().unwrap().filter_stacks[0];
    for node in &stack.nodes {
        s.execute("xcf.filter.remove", json!({"filterId":node.id.0.to_string()})).unwrap();
    }
    let empty = s.active().unwrap().doc.clone();
    let owner = empty.accounted().unwrap();
    let ItemRef::Layer(layer) = stack.owner else { panic!("actual layer owner") };
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let usage = owner.quota().usage().unwrap();
    for (visible, opacity_f32_bits) in [(Some(false), None), (None, Some(0.5f32.to_bits()))] {
        // The existing lower typed copier is genuinely charged and deliberately
        // structural. It does not certify the stricter empty rendering recipe.
        let invalid = owner
            .try_with_readonly_layer_properties(
                photocraft_doc::accounted_document::ReadonlyLayerPropertiesPatch { layer, name: None, visible, opacity_f32_bits },
                &ctl(),
            )
            .unwrap();
        assert!(invalid.is_readonly_point_storage());
        invalid.validate_readonly_point_storage(&ctl()).unwrap();
        let error = crate::accounted_native_open::validate_current_stock_point_storage(&invalid, &ctl()).unwrap_err();
        assert!(error.to_string().contains("owned stock Point current recipe"), "{error}");
        drop(invalid);
        assert!(s.active().unwrap().doc.shares_state(&empty));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.journal, journal);
        assert_eq!(owner.quota().usage().unwrap(), usage);
    }
}
