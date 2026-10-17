//! Deferred one common native brush transaction across actual programme classes.
//! Genuine Source and paid synthetic CURRENT evidence remain distinct in helper.
use super::*;
#[path = "accounted_programme_paint_test_support.rs"]
mod f;
use serde_json::json;
fn session(case: f::Case) -> Session {
    let input = f::load(case);
    let mut s = Session::new();
    f::adopt(&mut s, case, &input);
    s.tools.brush.pressure_size = false;
    s
}
#[test]
#[ignore = "UNRUN/uncompiled nine actual nonLegacy classes/ten retained Source-or-paidCURRENT cases, common native COW exact programme/mask/source/provider preservation, one history action and no-op redo. Internal compatibility, no producer brush oracle."]
fn programme_stroke_preserves_the_complete_stored_programme() {
    for &case in f::cases() {
        let mut s = session(case);
        let before = s.active().unwrap().doc.clone();
        let id = f::target(&before);
        let raw = f::raw(&before, id);
        let witness = f::Witness::capture(before.accounted().unwrap(), &f::ctl());
        let source = s.active().unwrap().source.clone();
        let kind = accounted_programme_current::kind(before.accounted().unwrap()).unwrap();
        let revision = s.active().unwrap().revision;
        let past = s.active().unwrap().history.past_len();
        let p = f::patch(&before, id);
        let prepared = Request::parse(&s, "paint.stroke", &p, &f::ctl()).unwrap().prepare(&s, &f::ctl()).unwrap();
        assert_eq!(prepared.request.domain, NativeStrokeDomain::StoredProgramme(kind));
        let target = prepared.programme.as_ref().unwrap();
        assert_eq!(target.layer, id);
        assert_eq!(target.kind, kind);
        let borrowed = before.accounted().unwrap().checked_programme_native_target(id, &f::ctl()).unwrap();
        assert_eq!(target.ordinal, borrowed.ordinal());
        assert_eq!(target.parent, borrowed.parent());
        assert_eq!(target.canonical, borrowed.canonical_item_index());
        drop(borrowed);
        drop(prepared);
        assert!(before.shares_state(&s.active().unwrap().doc));
        drop(s.execute("paint.stroke", p).unwrap());
        let changed = s.active().unwrap().doc.clone();
        assert_eq!(s.active().unwrap().revision, revision + 1);
        assert_eq!(s.active().unwrap().history.past_len(), past + 1);
        assert_eq!(s.active().unwrap().source, source);
        assert_eq!(accounted_programme_current::kind(changed.accounted().unwrap()), Some(kind));
        assert_ne!(f::raw(&changed, id), raw);
        assert_eq!(f::raw(&before, id), raw);
        witness.preserved(&changed, id, &f::ctl());
        assert!(s.undo_with_control(&f::ctl()).unwrap());
        assert_eq!(f::raw(&s.active().unwrap().doc, id), raw);
        let current = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let mut noop = f::patch(&current, id);
        noop["points"] = json!([[-100000.0, -100000.0, 1.0]]);
        s.execute("paint.stroke", noop).unwrap();
        assert!(current.shares_state(&s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(s.active().unwrap().history.can_redo());
        witness.preserved(&s.active().unwrap().doc, id, &f::ctl());
        assert!(s.redo_with_control(&f::ctl()).unwrap());
        assert_eq!(f::raw(&s.active().unwrap().doc, id), f::raw(&changed, id));
    }
}
#[test]
#[ignore = "UNRUN/uncompiled exact programme target/provider/rawalpha, final captured source/view refusal, original gesture meter where active raster, inactive-other-programme OLD/no-op and HDR STAGED refusal, early Work/cancel and unsupported brush boundary; no late cancellation claim."]
fn programme_stroke_requires_current_proof_and_exact_borrowed_alpha() {
    for &case in f::cases() {
        let mut s = session(case);
        let before = s.active().unwrap().doc.clone();
        let id = f::target(&before);
        let owner = before.accounted().unwrap();
        let raw = f::raw(&before, id);
        let LayerContent::BackedRaster(b) = &f::layer(&before, id).content else { panic!("native") };
        // Reference buffers are external harness values. The exact proof owns its
        // fixed scratch and validates the actual retained owner on every invocation.
        if b.format().alpha {
            crate::accounted_session::verify_programme_native_stroke_alpha(owner, id, b.operative_bounds(), &raw, &f::ctl()).unwrap();
            let mut changed_alpha = raw.clone();
            let alpha = b.format().bytes_per_pixel() - b.format().sample.bytes();
            changed_alpha[alpha] ^= 1;
            assert!(crate::accounted_session::verify_programme_native_stroke_alpha(owner, id, b.operative_bounds(), &changed_alpha, &f::ctl()).is_err());
        } else {
            assert!(crate::accounted_session::verify_programme_native_stroke_alpha(owner, id, b.operative_bounds(), &raw, &f::ctl()).is_err());
        }
        let mut p = f::patch(&before, id);
        let prepared = Request::parse(&s, "paint.stroke", &p, &f::ctl()).unwrap().prepare(&s, &f::ctl()).unwrap();
        s.active_mut().unwrap().view_revision += 1;
        assert!(prepared.execute(&mut s, &f::ctl()).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
        p["autoErase"] = json!(true);
        assert!(s.execute("paint.pencil", p).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
        let work = |_| Err(BackingError::Work);
        let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &work };
        assert!(Request::parse(&s, "paint.stroke", &f::patch(&before, id), &c).is_err());
        let yes = || true;
        let interrupt = photocraft_raster::Interrupt::cancel_only(&yes);
        assert!(Request::parse(&s, "paint.stroke", &f::patch(&before, id), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
        // An explicit stored-programme request cannot be substituted for a different
        // actual storage-kind route, even when its pixels/parameters are plausible.
        let route = accounted_programme_current::kind(owner).unwrap();
        let wrong = if route == ProgrammeKind::Point { ProgrammeKind::FilteredFlatPoint } else { ProgrammeKind::Point };
        assert!(validate_owner(owner, NativeStrokeDomain::StoredProgramme(wrong), &f::ctl()).is_err());
    }
    // Paid synthetic malformed CURRENT on an inactive OTHER programme. It is
    // deliberately never published through Source or typed native adoption.
    let mut invalid = session(f::Case::Hierarchy);
    let old = invalid.active().unwrap().doc.accounted().unwrap().clone();
    // The genuine first version22 packet has only one filtered leaf. Target
    // a distinct actual plain raster so that its sole Exposure remains OTHER.
    let tree = old.checked_readonly_normal_hierarchy_current_index(&f::ctl()).unwrap();
    let target = tree
        .rows()
        .iter()
        .find(|row| {
            matches!(row.layer.content, LayerContent::BackedRaster(_))
                && !old
                    .document()
                    .xcf_state
                    .as_ref()
                    .unwrap()
                    .filter_stacks
                    .iter()
                    .any(|st| st.owner == photocraft_doc::item_identity::ItemRef::Layer(row.id) && !st.nodes.is_empty())
        })
        .unwrap()
        .id;
    drop(tree);
    let q = old.quota().clone();
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mut d = old.document().clone();
    let x = d.xcf_state.as_mut().unwrap();
    let other = x
        .filter_stacks
        .iter_mut()
        .find(|st| {
            st.owner != photocraft_doc::item_identity::ItemRef::Layer(target)
                && st.nodes.iter().any(|n| {
                    n.arguments
                        .iter()
                        .any(|a| matches!(a.value, photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_))))
                })
        })
        .unwrap();
    let n = other
        .nodes
        .iter_mut()
        .find(|n| {
            n.arguments
                .iter()
                .any(|a| matches!(a.value, photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_))))
        })
        .unwrap();
    n.enabled = false;
    let a = n
        .arguments
        .iter_mut()
        .find(|a| matches!(a.value, photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_))))
        .unwrap();
    a.value = photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(f32::NAN.to_bits()));
    let bad = AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(d, std::sync::Arc::new((old, index, resident)), q, &f::ctl()).unwrap();
    let st = invalid.active_mut().unwrap();
    let view = st
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&bad, st.active_layer, st.accounted_view.as_ref().unwrap().selected(), st.layer_anchor, &f::ctl())
        .unwrap();
    let history = DocumentHistory::new_accounted(bad.clone(), 64, "external invalid programme CURRENT", view.target(), &f::ctl()).unwrap();
    st.doc = DocumentSnapshot::from_accounted(bad).unwrap();
    st.accounted_view = Some(view);
    st.history = history;
    let before = invalid.active().unwrap().doc.clone();
    assert!(invalid.active().unwrap().history.current().unwrap().shares_state(&before));
    let mut noop = f::patch(&before, target);
    noop["points"] = json!([[-100000.0, -100000.0, 1.0]]);
    assert!(invalid.execute("paint.stroke", noop).is_err());
    assert!(before.shares_state(&invalid.active().unwrap().doc));
    let mut hdr = session(f::Case::Hierarchy);
    let before = hdr.active().unwrap().doc.clone();
    let target = f::target(&before);
    let mut patch = f::patch(&before, target);
    patch["color"] = json!([1e20, 1e20, 1e20, 1.0]);
    assert!(hdr.execute("paint.stroke", patch).is_err());
    assert!(before.shares_state(&hdr.active().unwrap().doc));
    let mut s = session(f::Case::Point);
    let before = s.active().unwrap().doc.clone();
    let target = capture_accounted_native_stroke_target(&s).unwrap().unwrap();
    assert!(matches!(target.domain, NativeStrokeDomain::StoredProgramme(ProgrammeKind::Point)));
    let mut p = f::patch(&before, s.active().unwrap().active_layer.unwrap());
    target.admit_samples(2).unwrap();
    target.inject_captured_params(&s, &mut p).unwrap();
    let work = target.work.diagnostics().used;
    target.arm_captured_work(&mut s).unwrap();
    s.execute("paint.stroke", p).unwrap();
    assert!(target.work.diagnostics().used > work);
    assert!(s.native_stroke_work.is_none());
    let target = capture_accounted_native_stroke_target(&s).unwrap().unwrap();
    let now = s.active().unwrap().doc.clone();
    let read_only = s.active().unwrap().source_read_only;
    s.active_mut().unwrap().source_read_only = !read_only;
    assert!(target.inject_captured_params(&s, &mut json!({})).is_err());
    assert!(now.shares_state(&s.active().unwrap().doc));
}
