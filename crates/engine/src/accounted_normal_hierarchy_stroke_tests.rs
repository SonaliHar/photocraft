//! Deferred actual generated hierarchy Source to named native leaf COW.
//! No GIMP brush oracle, runtime, or performance acceptance.
use super::*;
#[path = "accounted_plain_hierarchy_paint_test_support.rs"]
mod f;
use photocraft_doc::item_identity::ItemRef;
use serde_json::json;
#[path = "accounted_normal_hierarchy_test_support.rs"]
mod filtered_fixture;
fn patch() -> Value {
    json!({"points":[[254.0,254.0,0.7],[256.0,257.0,1.0]],"size":7.0,"hardness":0.6,"spacing":0.3,"color":[0.8,0.2,0.1,0.75],"seed":53})
}
fn session(depth: usize) -> Session {
    let imported = f::import(depth);
    let photocraft_io::OwnedXcfImport::NormalHierarchy(input) = imported else { panic!("hierarchy") };
    let mut s = Session::new();
    s.add_owned_normal_hierarchy_xcf_import(&input, "nested.xcf", Some("authorized/nested.xcf"), &f::ctl()).unwrap();
    let leaf = f::target_leaf(&s.active().unwrap().doc);
    s.execute("layer.select", json!({"layer":leaf.0})).unwrap();
    s.tools.brush.pressure_size = false;
    s
}
fn paid_copy(owner: &AccountedDocument, change: impl FnOnce(&mut photocraft_doc::Document)) -> AccountedDocument {
    let q = owner.quota().clone();
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mut d = owner.document().clone();
    change(&mut d);
    AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(d, std::sync::Arc::new((owner.clone(), index, resident)), q, &f::ctl()).unwrap()
}
fn replace_current(s: &mut Session, owner: AccountedDocument) {
    let st = s.active_mut().unwrap();
    let view = st
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&owner, st.active_layer, st.accounted_view.as_ref().unwrap().selected(), st.layer_anchor, &f::ctl())
        .unwrap();
    let history = DocumentHistory::new_accounted(owner.clone(), 64, "external Current control", view.target(), &f::ctl()).unwrap();
    st.doc = DocumentSnapshot::from_accounted(owner).unwrap();
    st.accounted_view = Some(view);
    st.history = history;
}
#[test]
#[ignore = "UNRUN/uncompiled actual five-depth generated Source nested leaf Round/erase/Clear/pencil, one revision/history action, sibling/source/native precision custody, retained empty anchor and redo-preserving no-op. Internal compatibility only."]
fn hierarchy_stroke_targets_only_the_actual_nested_native_leaf() {
    for depth in 0..5 {
        for variant in 0..4 {
            let mut s = session(depth);
            let leaf = f::target_leaf(&s.active().unwrap().doc);
            let before = s.active().unwrap().doc.clone();
            let original = f::payload(&before);
            let source = s.active().unwrap().source.clone();
            let rev = s.active().unwrap().revision;
            let past = s.active().unwrap().history.past_len();
            let mut p = patch();
            if variant == 1 {
                p["erase"] = json!(true)
            }
            if variant == 2 {
                p["mode"] = json!("clear")
            }
            let cmd = if variant == 3 { "paint.pencil" } else { "paint.stroke" };
            drop(s.execute(cmd, p).unwrap());
            let changed = s.active().unwrap().doc.clone();
            assert!(changed.accounted().unwrap().is_readonly_normal_hierarchy_storage());
            assert_eq!(s.active().unwrap().revision, rev + 1);
            assert_eq!(s.active().unwrap().history.past_len(), past + 1);
            assert_eq!(changed.depth, f::sample(depth));
            let after = f::payload(&changed);
            for (id, raw) in &original {
                let actual = &after.iter().find(|(other, _)| other == id).unwrap().1;
                if *id == leaf { assert_ne!(actual, raw) } else { assert_eq!(actual, raw) }
            }
            assert_eq!(f::payload(&before), original);
            f::source_payload(&before, &changed);
            assert_eq!(before.xcf_state, changed.xcf_state);
            assert_eq!(s.active().unwrap().source, source);
            assert!(s.active().unwrap().source_read_only);
            s.undo_with_control(&f::ctl()).unwrap();
            assert_eq!(f::payload(&s.active().unwrap().doc), original);
            let current = s.active().unwrap().doc.clone();
            let rev = s.active().unwrap().revision;
            let mut no = patch();
            no["points"] = json!([[-1000.0, -1000.0, 1.0]]);
            s.execute("paint.stroke", no).unwrap();
            assert!(current.shares_state(&s.active().unwrap().doc));
            assert_eq!(s.active().unwrap().revision, rev);
            assert!(s.active().unwrap().history.can_redo());
            s.redo_with_control(&f::ctl()).unwrap();
            assert_eq!(f::payload(&s.active().unwrap().doc), after);
        }
    }
    let mut s = session(4);
    let old = s.active().unwrap().doc.accounted().unwrap().clone();
    let anchored = f::anchored(&old);
    replace_current(&mut s, anchored);
    let before = s.active().unwrap().doc.clone();
    s.execute("paint.stroke", patch()).unwrap();
    assert_eq!(before.xcf_state, s.active().unwrap().doc.xcf_state);
    let leaf = f::target_leaf(&before);
    assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.owner == ItemRef::Layer(leaf)).unwrap().revision, 7);
}
#[test]
#[ignore = "UNRUN/uncompiled actual nested target/ancestor alpha and pixel locks, group/generic programme NativePixels closure, hidden invalid OLD and finite-HDR STAGED atomic refusal even no-op, captured original clock/source/view and early Work/cancel. No late cancellation measurement."]
fn hierarchy_stroke_requires_complete_plain_current_and_captured_target() {
    for depth in 0..5 {
        let mut s = session(depth);
        let before = s.active().unwrap().doc.clone();
        let (group, leaf, _) = f::ids(&before);
        let mut p = patch();
        p["layer"] = json!(group.0);
        assert!(s.execute("paint.stroke", p).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
        let locked = paid_copy(before.accounted().unwrap(), |d| {
            let mut todo: Vec<_> = d.layers.iter_mut().collect();
            while let Some(l) = todo.pop() {
                if l.id == group {
                    l.locks.transparency = true;
                }
                if let LayerContent::Group(g) = &mut l.content {
                    todo.extend(g.children.iter_mut());
                }
            }
        });
        replace_current(&mut s, locked);
        let old = f::payload(&s.active().unwrap().doc);
        s.execute("paint.stroke", patch()).unwrap();
        let now = f::payload(&s.active().unwrap().doc);
        let n = f::sample(depth).bytes();
        let a = &old.iter().find(|(id, _)| *id == leaf).unwrap().1;
        let b = &now.iter().find(|(id, _)| *id == leaf).unwrap().1;
        for (a, b) in a.chunks_exact(4 * n).zip(b.chunks_exact(4 * n)) {
            assert_eq!(&a[3 * n..], &b[3 * n..]);
        }
        let owner = s.active().unwrap().doc.accounted().unwrap().clone();
        let locked = paid_copy(&owner, |d| {
            let mut todo: Vec<_> = d.layers.iter_mut().collect();
            while let Some(l) = todo.pop() {
                if l.id == group {
                    l.locks.pixels = true;
                }
                if let LayerContent::Group(g) = &mut l.content {
                    todo.extend(g.children.iter_mut());
                }
            }
        });
        replace_current(&mut s, locked);
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute("paint.stroke", patch()).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
    }
    let mut s = session(4);
    let target = capture_accounted_native_stroke_target(&s).unwrap().unwrap();
    assert_eq!(target.domain, NativeStrokeDomain::NormalHierarchy);
    assert!(target.tree.as_ref().unwrap().parent.is_some());
    let mut p = patch();
    target.admit_samples(3).unwrap();
    target.inject_captured_params(&s, &mut p).unwrap();
    let used = target.work.diagnostics().used;
    target.arm_captured_work(&mut s).unwrap();
    s.execute("paint.stroke", p).unwrap();
    assert!(target.work.diagnostics().used > used);
    assert!(s.native_stroke_work.is_none());
    let changed = s.active().unwrap().doc.clone();
    let target = capture_accounted_native_stroke_target(&s).unwrap().unwrap();
    s.active_mut().unwrap().view_revision += 1;
    assert!(target.inject_captured_params(&s, &mut patch()).is_err());
    assert!(changed.shares_state(&s.active().unwrap().doc));
    let mut hdr = session(4);
    let before = hdr.active().unwrap().doc.clone();
    let mut p = patch();
    p["color"] = json!([1e20, 1e20, 1e20, 1.0]);
    assert!(hdr.execute("paint.stroke", p).is_err());
    assert!(before.shares_state(&hdr.active().unwrap().doc));
    // This direct named structural fixture is deliberately invalid operative
    // CURRENT on hidden LeafB. It is not an import/adoption or repair permission.
    let hidden = f::named(&before, "LeafB").id;
    let owner = before.accounted().unwrap();
    let bytes = [2.0f64, 0.25, 0.5, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
    let bad = owner
        .try_edit_readonly_normal_hierarchy_native(
            AccountedEdit::NativePixels { layer: hidden, rect: photocraft_geom::Rect::new(0, 0, 1, 1), bytes: &bytes },
            &f::ctl(),
        )
        .unwrap();
    replace_current(&mut hdr, bad);
    let bad = hdr.active().unwrap().doc.clone();
    let mut no = patch();
    no["points"] = json!([[-1000.0, -1000.0, 1.0]]);
    assert!(hdr.execute("paint.stroke", no).is_err());
    assert!(bad.shares_state(&hdr.active().unwrap().doc));
    let filtered_input = filtered_fixture::build(0);
    let mut filtered = Session::new();
    filtered.add_owned_native_normal_hierarchy_archive(f::archive(&filtered_input), "filtered.pcraft", None, &f::ctl()).unwrap();
    let raster = filtered.active().unwrap().doc.layers[0].id;
    filtered.execute("layer.select", json!({"layer":raster.0})).unwrap();
    let unchanged = filtered.active().unwrap().doc.clone();
    let owner = unchanged.accounted().unwrap();
    let access = owner.accounted_layer_access(raster, &f::ctl()).unwrap().unwrap();
    let LayerContent::BackedRaster(backed) = &access.layer.content else { panic!("actual raster") };
    let r = backed.operative_bounds();
    let bpp = backed.format().bytes_per_pixel();
    let mut native = vec![0; bpp];
    backed.checked_view().pin_tile(r.tiles().next().unwrap(), &f::ctl()).unwrap().read_pixel_native_into(r.x0, r.y0, &mut native, &f::ctl()).unwrap();
    // Programme-specific named editing is admitted now; generic NativePixels
    // remains closed for this actual filtered hierarchy owner.
    assert!(
        owner
            .try_edit(
                AccountedEdit::NativePixels { layer: raster, rect: photocraft_geom::Rect::new(r.x0, r.y0, r.x0 + 1, r.y0 + 1), bytes: &native[..bpp] },
                &f::ctl()
            )
            .is_err()
    );
    assert!(unchanged.shares_state(&filtered.active().unwrap().doc));
    let deny = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
    assert!(Request::parse(&s, "paint.stroke", &patch(), &c).is_err());
    let yes = || true;
    let interrupt = photocraft_raster::Interrupt::cancel_only(&yes);
    assert!(Request::parse(&s, "paint.stroke", &patch(), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
}
