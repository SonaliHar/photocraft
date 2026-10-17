//! UNRUN atomic plain bulk Move. The original two-layer packet is genuinely
//! authored/pinned; a third layer/selection/reference/lock variant is a prepaid
//! CURRENT-only control retaining that real owner, not new producer evidence.
//! Native captures are external harness buffers, not production quota receipts.
use super::*;
use crate::{accounted_commands::PreparedAccountedCommand, accounted_view::AccountedView};
use photocraft_doc::{LayerContent, LayerId, item_identity::ItemRef};
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaLimits},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn fixture(hidden_peer: bool, locked_peer: bool) -> Session {
    let path = "composition-matrix/mode-normal.xcf";
    let bytes = std::fs::read(PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(path)).unwrap();
    assert_eq!(bytes.len(), 5425);
    assert_eq!(
        Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "1cfb20a92da5a2180fa9ff8d77156c06e9bcb312922d488a50fdfdaab2e6d5c3"
    );
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 16 << 20), ("scratchBytes", 0)])
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
    let photocraft_io::OwnedXcfImport::Plain(imported) = result else { panic!("actual plain original") };
    let source = imported.document().clone();
    assert_eq!(source.document().layers.len(), 2);
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = source.document().clone();
    assert!(doc.layers.iter().all(|l| l.id != LayerId(9999)));
    let mut extra = doc.layers[1].clone();
    let old = extra.id;
    extra.id = LayerId(9999);
    extra.name = "Third current".into();
    extra.effects.reference = Some((1.25, -3.5));
    extra.locks.position = locked_peer;
    let mut item = doc.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(old)).unwrap().clone();
    item.item = ItemRef::Layer(extra.id);
    item.name = extra.name.clone();
    item.custom_name = true;
    doc.layers[0].visible = !hidden_peer;
    doc.layers[0].effects.reference = Some((-0.0, -0.0));
    let base = doc.layers[0].id;
    doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(base)).unwrap().visible = !hidden_peer;
    let middle = doc.layers[1].id;
    doc.layers[1].name = "Middle current".into();
    let middle_item = doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(middle)).unwrap();
    middle_item.name = "Middle current".into();
    middle_item.custom_name = true;
    doc.layers.push(extra);
    doc.xcf_state.as_mut().unwrap().items.push(item);
    let owner = AccountedDocument::from_owned(doc, Arc::new((source, fee)), q, &ctl()).unwrap();
    let mut s = Session::new();
    s.add_accounted_document(owner, None, &ctl()).unwrap();
    set_view(&mut s, 2, &[2, 0]);
    s
}
fn set_view(s: &mut Session, active: usize, indices: &[usize]) {
    let state = s.active().unwrap();
    let owner = state.doc.accounted().unwrap();
    let active = state.doc.layers[active].id;
    let selected: Vec<_> = indices.iter().map(|i| state.doc.layers[*i].id).collect();
    let view = AccountedView::new(owner, Some(active), &selected, Some(active), &ctl()).unwrap();
    let state = s.active_mut().unwrap();
    state.accounted_view = Some(view);
    state.active_layer = Some(active);
    state.layer_anchor = Some(active);
    state.selected_layers.clear();
    state.view_revision += 1;
}
fn raw(s: &Session, index: usize) -> (photocraft_geom::Rect, Vec<u8>) {
    let LayerContent::BackedRaster(layer) = &s.active().unwrap().doc.layers[index].content else { panic!() };
    let r = layer.operative_bounds();
    let n = layer.format().bytes_per_pixel();
    let mut out = vec![0; r.width() as usize * r.height() as usize * n];
    for coordinate in r.tiles() {
        let pin = layer.checked_view().pin_tile(coordinate, &ctl()).unwrap();
        let part = coordinate.rect().intersect(&r);
        for y in part.y0..part.y1 {
            for x in part.x0..part.x1 {
                let i = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * n;
                pin.read_pixel_native_into(x, y, &mut out[i..i + n], &ctl()).unwrap();
            }
        }
    }
    (r, out)
}
fn reopen(s: &Session) -> Session {
    let q = s.active().unwrap().doc.quota().unwrap();
    let bytes =
        photocraft_format::owned_document::save_current_document_archive(&s.active().unwrap().doc, Cursor::new(Vec::new()), Default::default(), q, &ctl())
            .unwrap()
            .into_inner();
    let archive =
        photocraft_io::accounted_native::open_accounted_native_from_reader(&mut Cursor::new(&bytes), bytes.len() as u64, Default::default(), q.clone(), &ctl())
            .unwrap();
    let mut out = Session::new();
    out.add_owned_native_archive(
        archive,
        crate::accounted_native_open::AccountedNativeOpenPolicy::KeepCurrentInterpretation,
        "Bulk.pcraft",
        Some("actual/Bulk.pcraft"),
        &ctl(),
    )
    .unwrap();
    out
}
#[test]
#[ignore = "UNRUN: genuine current multi-view in reverse selection order moves exactly doc-order members once, hidden peer allowed and native continuation"]
fn bulk_selection_is_one_history_revision_and_preserves_unselected_native_words() {
    let mut s = fixture(true, false);
    let before = s.active().unwrap().doc.clone();
    let original = [raw(&s, 0), raw(&s, 1), raw(&s, 2)];
    let rev = s.active().unwrap().revision;
    let ids: Vec<_> = s.active().unwrap().doc.layers.iter().map(|l| l.id).collect();
    let targets = targets::Targets::prepare(s.active().unwrap(), TargetMode::Selected, &ctl()).unwrap();
    assert_eq!(targets.ids(), &[ids[0], ids[2]]);
    drop(targets);
    s.execute(ID, json!({"dx":3,"dy":0})).unwrap();
    assert_eq!(s.active().unwrap().revision, rev + 1);
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Move"));
    for i in [0, 2] {
        assert_eq!(raw(&s, i), (original[i].0.translate(3, 0), original[i].1.clone()));
    }
    assert_eq!(raw(&s, 1), original[1]);
    for layer in &s.active().unwrap().doc.layers {
        let LayerContent::BackedRaster(backed) = &layer.content else { panic!() };
        let item = s.active().unwrap().doc.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).unwrap();
        assert_eq!(item.raster_bounds, Some(backed.operative_bounds()));
    }
    assert_eq!(s.active().unwrap().doc.layers[1].effects.reference, before.layers[1].effects.reference);

    assert_eq!(s.active().unwrap().doc.layers[0].effects.reference.unwrap().0.to_bits(), 3.0f64.to_bits());
    assert_eq!(s.active().unwrap().doc.layers[0].effects.reference.unwrap().1.to_bits(), 0.0f64.to_bits());
    assert_eq!(s.active().unwrap().doc.layers[2].effects.reference, Some((4.25, -3.5)));
    assert_eq!(s.active().unwrap().doc.icc_profile, before.icc_profile);
    assert_eq!(s.active().unwrap().doc.source_color_context, before.source_color_context);
    let reopened = reopen(&s);
    for i in 0..3 {
        assert_eq!(raw(&reopened, i), raw(&s, i));
        assert_eq!(reopened.active().unwrap().doc.layers[i].effects.reference, s.active().unwrap().doc.layers[i].effects.reference);
    }
    assert!(s.undo_with_control(&ctl()).unwrap());
    for (i, expected) in original.iter().enumerate() {
        assert_eq!(raw(&s, i), *expected);
    }
    assert!(s.redo_with_control(&ctl()).unwrap());
}
#[test]
#[ignore = "UNRUN: explicit name/ID overrides genuine multi-selection; absent/null selects all; no duplicate public single edits"]
fn explicit_target_overrides_multi_view_and_null_uses_the_paid_selected_set() {
    for by_name in [true, false] {
        let mut s = fixture(false, false);
        let original = [raw(&s, 0), raw(&s, 1), raw(&s, 2)];
        let target = if by_name { json!(s.active().unwrap().doc.layers[1].name) } else { json!(s.active().unwrap().doc.layers[1].id.0) };
        s.execute(ID, json!({"layer":target,"dx":-2,"dy":1})).unwrap();
        assert_eq!(raw(&s, 0), original[0]);
        assert_eq!(raw(&s, 2), original[2]);
        assert_eq!(raw(&s, 1), (original[1].0.translate(-2, 1), original[1].1.clone()));
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        s.execute(ID, json!({"layer":null,"dx":1})).unwrap();
        for i in [0, 2] {
            assert_eq!(raw(&s, i), (original[i].0.translate(1, 0), original[i].1.clone()));
        }
        assert_eq!(s.active().unwrap().history.past_len(), 2);
    }
}
#[test]
#[ignore = "UNRUN: shared property/bulk key retains one first label, complete undo, roundedzero keeps redo, stale view/history fences"]
fn bulk_coalescing_and_prepared_fences_use_the_actual_view_owner() {
    let mut s = fixture(false, false);
    let before = [raw(&s, 0), raw(&s, 1), raw(&s, 2)];
    s.execute("layer.setProps", json!({"name":"First label","coalesce":"bulk"})).unwrap();
    s.execute(ID, json!({"dx":2,"dy":-1,"coalesce":"bulk"})).unwrap();
    s.execute(ID, json!({"dx":1,"coalesce":"bulk"})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Layer Properties"));
    assert!(s.undo_with_control(&ctl()).unwrap());
    for (i, expected) in before.iter().enumerate() {
        assert_eq!(raw(&s, i), *expected);
    }
    let history = s.active().unwrap().history.clone();
    s.execute(ID, json!({"layer":{},"dx":0.1,"dy":false,"coalesce":"other"})).unwrap();
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert!(s.active().unwrap().history.can_redo());
    assert!(s.redo_with_control(&ctl()).unwrap());
    for changed in 0..3 {
        let mut s = fixture(false, false);
        let p = json!({"dx":1});
        let prepared = PreparedAccountedCommand::prepare(&s, ID, &p, &ctl()).unwrap();
        match changed {
            0 => set_view(&mut s, 0, &[0, 1]),
            1 => s.active_mut().unwrap().history.max_states += 1,
            _ => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
        };
        let before = s.active().unwrap().doc.clone();
        let journal = s.journal.clone();
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.journal, journal);
    }
}
#[test]
#[ignore = "UNRUN: any selected lock or endpoint and quota/late cancellation rejects entire bulk transaction with no model/history prefix"]
fn last_target_failure_has_no_prefix_and_exhaustion_is_private() {
    let mut locked = fixture(false, true);
    let before = locked.active().unwrap().doc.clone();
    let history = locked.active().unwrap().history.clone();
    let journal = locked.journal.clone();
    assert!(locked.execute(ID, json!({"dx":1})).is_err());
    assert!(locked.active().unwrap().doc.shares_state(&before));
    assert!(locked.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(locked.journal, journal);
    // An explicitly unrelated middle target still works despite a locked selected peer.
    let id = locked.active().unwrap().doc.layers[1].id;
    locked.execute(ID, json!({"layer":id.0,"dx":1})).unwrap();
    let mut s = fixture(false, false);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    assert!(s.execute(ID, json!({"dx":2147483647,"dy":1})).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let spent = AtomicU64::new(0);
    let work = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    current(before.accounted().unwrap(), &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap();
    let cost = spent.load(Ordering::Relaxed);
    let spent = AtomicU64::new(0);
    let work = |n| if spent.fetch_add(n, Ordering::Relaxed).saturating_add(n) > cost + cost / 4 { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(s.translate_accounted_imported_selection(1, 1, None, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert!(spent.load(Ordering::Relaxed) > cost);
    assert!(s.active().unwrap().doc.shares_state(&before));
    let q = before.quota().unwrap();
    let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.execute(ID, json!({"dx":1})).is_err());
    drop(hold);
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal, journal);
}
