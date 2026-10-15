//! UNRUN FlatComposite CURRENT Move. The local source is hash-bound research
//! input with profile-byte HOLD, not a new producer translation/math oracle.
//! Lock/reference variants are genuinely prepaid CURRENT-only model copies.
//! Native capture buffers remain external harness data.
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
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn fixture(lock_last: bool) -> Session {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-flat-multilayer-gimp-20261016/author-v1/outputs/three-auto.xcf");
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(bytes.len(), 46062);
    assert_eq!(
        Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "547fccca9b828174af95882ac7ffd5c367fdfee1425f557b59d59168b00c0497"
    );
    let q = BackingQuota::new(QuotaLimits { resident: 512 << 20, index: 64 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 256 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
        .unwrap();
    let imported = photocraft_io::import_owned_xcf_from_reader_in_quota(
        "research.xcf",
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::SeparableBlend(imported) = imported else { panic!("actual strict source route") };
    let source = imported.document().clone();
    // Reserve real supplemental model-copy storage BEFORE the harness clone;
    // retained original still owns shared native stores and source bytes.
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = source.document().clone();
    assert_eq!(doc.layers.len(), 3);
    doc.layers[0].effects.reference = Some((-0.0, 1.25));
    doc.layers[2].effects.reference = Some((3.5, -0.0));
    doc.layers[2].locks.position = lock_last;
    // Hidden selected peer is allowed when active numeric dispatch target is visible.
    doc.layers[0].visible = false;
    let id = doc.layers[0].id;
    doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
    assert!(photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap().retained_bytes() <= 2 << 20);
    let owner = AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((source, fee)), q, &ctl()).unwrap();
    crate::accounted_xcf_open::separable_blend::validate_current_separable_blend_storage(&owner, &ctl()).unwrap();
    let mut s = Session::new();
    let state = crate::DocState::new_accounted(owner, None, 64, &ctl()).unwrap();
    // Internal CURRENT fixture only, after full operative proof, not generic source adoption.
    s.publish_document_state(state, &ctl()).unwrap();
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
    out.add_owned_native_separable_blend_archive(archive, "Bulk.pcraft", Some("actual/Bulk.pcraft"), &ctl()).unwrap();
    out
}

#[test]
#[ignore = "UNRUN: actual paid flat selected doc-order onecopy/history/native continuation, unrelated input retained"]
fn selected_move_and_native_continuation() {
    let mut s = fixture(false);
    let rev = s.active().unwrap().revision;
    let original = [raw(&s, 0), raw(&s, 1), raw(&s, 2)];
    let ids: Vec<_> = s.active().unwrap().doc.layers.iter().map(|l| l.id).collect();
    let targets = Targets::prepare(s.active().unwrap(), TargetMode::Selected, &ctl()).unwrap();
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
    let r0 = s.active().unwrap().doc.layers[0].effects.reference.unwrap();
    assert_eq!((r0.0.to_bits(), r0.1.to_bits()), (3f64.to_bits(), 1.25f64.to_bits()));
    let reopened = reopen(&s);
    assert!(reopened.active().unwrap().doc.accounted().unwrap().is_readonly_flat_composite_storage());
    for i in 0..3 {
        assert_eq!(raw(&reopened, i), raw(&s, i));
    }
    s.execute("edit.undo", json!({})).unwrap();
    for i in 0..3 {
        assert_eq!(raw(&s, i), original[i]);
    }
    s.execute("edit.redo", json!({})).unwrap();
}
#[test]
#[ignore = "UNRUN: explicit override and shared metadata/Move key retain first label and zero preserves redo"]
fn explicit_override_mixed_gesture_and_zero() {
    let mut s = fixture(true);
    let id = s.active().unwrap().doc.layers[1].id;
    let before = raw(&s, 0);
    s.execute("layer.setProps", json!({"layer":id.0,"name":"Moved middle","coalesce":"same"})).unwrap();
    s.execute(ID, json!({"layer":"Moved middle","dx":2.5,"dy":null,"units":false,"coalesce":"same"})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Layer Properties"));
    assert_eq!(raw(&s, 0), before);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().history.can_redo());
    let rev = s.active().unwrap().revision;
    // Unknown numeric ID bypasses active-hidden guard; rounded zero precedes target/lock lookup.
    s.execute(ID, json!({"layer":99999999u64,"dx":0.1,"dy":"ignored"})).unwrap();
    assert_eq!(s.active().unwrap().revision, rev);
    assert!(s.active().unwrap().history.can_redo());
}
#[test]
#[ignore = "UNRUN: final selected lock refuses atomically; no mask/linked permission and original outer hidden ordering"]
fn target_and_wire_refusals() {
    let mut s = fixture(true);
    let before = s.active().unwrap().doc.clone();
    let rev = s.active().unwrap().revision;
    assert!(s.execute(ID, json!({"dx":1})).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(s.active().unwrap().revision, rev);
    for p in [
        json!({"target":"mask","dx":1}),
        json!({"target":"quickMask","dx":1}),
        json!({"units":"hostile"}),
        json!({"layer":s.active().unwrap().doc.layers[0].id.0,"dx":0}),
    ] {
        assert!(s.execute(ID, p).is_err());
    }
    assert!(s.active().unwrap().doc.shares_state(&before));
    let mut s = fixture(false);
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    assert!(s.execute(ID, json!({"dx":2147483647,"dy":1})).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(s.journal, journal);
}
#[test]
#[ignore = "UNRUN: same finite work/cancel/quota and stale actual view/history/channel fences preserve live state"]
fn failed_work_and_prepared_fences() {
    let mut s = fixture(false);
    let before = s.active().unwrap().doc.clone();
    let rev = s.active().unwrap().revision;
    let left = AtomicU64::new(32);
    let debit = |n| left.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
    assert!(PreparedAccountedCommand::prepare(&s, ID, &json!({"dx":1}), &c).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(s.active().unwrap().revision, rev);
    let cost = AtomicU64::new(0);
    let work = |n| {
        cost.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    MovePolicy::FlatComposite.current(before.accounted().unwrap(), &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap();
    let limit = cost.load(Ordering::Relaxed);
    let spent = AtomicU64::new(0);
    let stop = |n| if spent.fetch_add(n, Ordering::Relaxed).saturating_add(n) > limit + limit / 4 { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(s.translate_accounted_imported_selection(1, 0, None, &ReadControl { interrupt: &Interrupt::NONE, work: &stop }).is_err());
    assert!(spent.load(Ordering::Relaxed) > limit);
    assert!(s.active().unwrap().doc.shares_state(&before));
    let q = before.quota().unwrap();
    let hold = q.reserve(QuotaKind::Index, (64u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.execute(ID, json!({"dx":1})).is_err());
    drop(hold);
    assert!(s.active().unwrap().doc.shares_state(&before));
    for changed in 0..3 {
        let mut s = fixture(false);
        let params = json!({"dx":1});
        let prepared = PreparedAccountedCommand::prepare(&s, ID, &params, &ctl()).unwrap();
        match changed {
            0 => set_view(&mut s, 1, &[1]),
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
