//! Deferred actual guarded programme pixel-selection consumers. Sources are
//! genuine; expected/native buffers below are external harness allocations.
use super::*;
#[path = "accounted_programme_selection_native_test_support.rs"]
mod f;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::accounted_selection::{AccountedSelection, CoverageDomain};
use serde_json::json;
fn session(case: f::Case, depth: usize) -> Session {
    let input = f::load(case, depth);
    let mut s = Session::new();
    f::adopt(&mut s, case, &input);
    s.tools.brush.pressure_size = false;
    s
}
fn selection(s: &Session, value: f64) -> AccountedSelection {
    let d = &s.active().unwrap().doc;
    let sample = d.depth;
    let word = match sample {
        SampleType::F32 => (value as f32).to_ne_bytes().to_vec(),
        SampleType::F64 => value.to_ne_bytes().to_vec(),
        _ => panic!("floating original fixtures"),
    };
    let raw = word.repeat(d.size.width as usize * d.size.height as usize);
    AccountedSelection::try_from_native(
        PixelFormat::new(ColorMode::Grayscale, sample, false),
        d.bounds(),
        &vec![0; word.len()],
        &raw,
        CoverageDomain::FiniteUnit,
        d.quota().unwrap(),
        &f::ctl(),
    )
    .unwrap()
}
fn install(s: &mut Session, value: f64) {
    let selected = selection(s, value);
    let target = AccountedSelectionTarget::capture_with_control(s, &f::ctl()).unwrap();
    s.replace_accounted_selection(target, "guarded selection", Some(&selected), &f::ctl()).unwrap();
}
#[test]
#[ignore = "UNRUN/uncompiled: genuine four-class F32/F64 Sources, guarded zero/fractional native Clear and F32 Fill, raw no-op/redo, registered All/Deselect, complete programme/source custody; internal compatibility not producer selection math"]
fn guarded_selection_consumers_preserve_programmes_and_history() {
    for case in f::CASES {
        for depth in 0..2 {
            let mut s = session(case, depth);
            let original = s.active().unwrap().doc.clone();
            let id = f::base::target(&original);
            let source = s.active().unwrap().source.clone();
            let witness = f::Witness::capture(original.accounted().unwrap(), &f::ctl());
            let raw = f::raw(&original, id);
            install(&mut s, 0.0);
            let zero = s.active().unwrap().doc.clone();
            assert_eq!(f::raw(&zero, id), raw);
            s.execute("select.deselect", json!({})).unwrap();
            assert!(s.active().unwrap().doc.selection.is_none());
            assert!(s.undo_with_control(&f::ctl()).unwrap());
            f::same_selection(&zero, &s.active().unwrap().doc);
            let current = s.active().unwrap().doc.clone();
            let revision = s.active().unwrap().revision;
            let history = s.active().unwrap().history.clone();
            // Installed all-zero selection is a true guarded field, not the absent/full
            // selection sentinel. Existing selected kernels leave native pixels exact.
            s.execute("edit.clear", json!({"layer":id.0})).unwrap();
            assert!(current.shares_state(&s.active().unwrap().doc));
            assert_eq!(s.active().unwrap().revision, revision);
            assert!(history.shares_accounted_history(&s.active().unwrap().history));
            assert!(s.active().unwrap().history.can_redo());
            s.execute("paint.stroke", f::patch(&current, id)).unwrap();
            assert!(current.shares_state(&s.active().unwrap().doc));
            assert_eq!(f::raw(&s.active().unwrap().doc, id), raw);
            if depth == 0 {
                s.execute("edit.fill", json!({"layer":id.0,"contents":"black","opacity":100,"mode":"normal"})).unwrap();
                assert!(current.shares_state(&s.active().unwrap().doc));
            }
            install(&mut s, 0.5 + 2f64.powi(-24));
            let selected = s.active().unwrap().doc.clone();
            let revision = s.active().unwrap().revision;
            let past = s.active().unwrap().history.past_len();
            s.execute("edit.clear", json!({"layer":id.0})).unwrap();
            let cleared = f::raw(&s.active().unwrap().doc, id);
            assert_ne!(cleared, raw);
            if depth==1{
                // Fractional native Clear preserves RGB while computed alpha
                // is nonzero. Actual zero-alpha/results canonicalize the whole
                // pixel, before encoding; this is not an encoded-zero test.
                for (before,after) in raw.chunks_exact(32).zip(cleared.chunks_exact(32)){
                    let alpha=f64::from_ne_bytes(before[24..32].try_into().unwrap());
                    let expected=alpha*(1.0-(0.5+2f64.powi(-24)));
                    if expected==0.0{assert!(after.iter().all(|v|*v==0));}
                    else{assert_eq!(&before[..24],&after[..24]);assert_eq!(&after[24..32],&expected.to_ne_bytes());}
                }
            }
            assert_eq!(s.active().unwrap().revision, revision + 1);
            assert_eq!(s.active().unwrap().history.past_len(), past + 1);
            f::same_selection(&selected, &s.active().unwrap().doc);
            witness.preserved(&s.active().unwrap().doc, id, &f::ctl());
            assert!(s.undo_with_control(&f::ctl()).unwrap());
            assert_eq!(f::raw(&s.active().unwrap().doc, id), raw);
            if depth == 0 {
                s.execute("edit.fill", json!({"layer":id.0,"contents":"black","opacity":100,"mode":"normal"})).unwrap();
                assert_ne!(f::raw(&s.active().unwrap().doc, id), raw);
                f::same_selection(&selected, &s.active().unwrap().doc);
            } else {
                let before = s.active().unwrap().doc.clone();
                assert!(s.execute("edit.fill", json!({"layer":id.0,"contents":"black"})).is_err());
                assert!(before.shares_state(&s.active().unwrap().doc));
            }
            let before = s.active().unwrap().doc.clone();
            let native = f::raw(&before, id);
            let rev = s.active().unwrap().revision;
            s.execute("select.all", json!({})).unwrap();
            let all = s.active().unwrap().doc.clone();
            let field = all.selection.as_ref().unwrap();
            assert_eq!(field.format().sample, if depth == 1 { SampleType::F64 } else { SampleType::U8 });
            assert_eq!(f::raw(&all, id), native);
            assert_eq!(s.active().unwrap().revision, rev + 1);
            let revision = s.active().unwrap().revision;
            let history = s.active().unwrap().history.clone();
            s.execute("select.all", json!({})).unwrap();
            assert!(all.shares_state(&s.active().unwrap().doc));
            assert_eq!(s.active().unwrap().revision, revision);
            assert!(history.shares_accounted_history(&s.active().unwrap().history));
            s.execute("select.deselect", json!({})).unwrap();
            assert!(s.active().unwrap().doc.selection.is_none());
            assert_eq!(f::raw(&s.active().unwrap().doc, id), native);
            witness.preserved(&s.active().unwrap().doc, id, &f::ctl());
            assert_eq!(s.active().unwrap().source, source);
            assert!(s.active().unwrap().source_read_only);
        }
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: registered selection captured source/path/readOnly/view/channel/history/preferences fences, real guard and same-ledger rejection, uncontrolled capture closure, early Work/cancel; no late-cancellation claim or broader Color Range command grant"]
fn programme_selection_refuses_unpaid_or_stale_context_atomically() {
    for case in f::CASES {
        for variant in 0..7 {
            let mut s = session(case, 0);
            let before = s.active().unwrap().doc.clone();
            let journal = s.journal.clone();
            let p = json!({});
            let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "select.all", &p, &f::ctl()).unwrap();
            match variant {
                0 => s.active_mut().unwrap().source.as_mut().unwrap().label.push_str(" drift"),
                1 => s.active_mut().unwrap().path = Some("changed.pcraft".into()),
                2 => {
                    let v = s.active().unwrap().source_read_only;
                    s.active_mut().unwrap().source_read_only = !v;
                }
                3 => s.active_mut().unwrap().view_revision += 1,
                4 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
                5 => s.active_mut().unwrap().history.set_current_label("metadata drift", &f::ctl()).unwrap(),
                _ => s.prefs.edit(|p| p.performance.history_states = 1),
            }
            assert!(prepared.execute(&mut s, &f::ctl()).is_err());
            assert!(before.shares_state(&s.active().unwrap().doc));
            assert_eq!(s.journal, journal);
        }
    }
    let mut s = session(f::Case::MaskedFilteredFlat, 1);
    let before = s.active().unwrap().doc.clone();
    let field = selection(&s, 0.5);
    let uncontrolled = AccountedSelectionTarget::capture(&s).unwrap();
    assert!(s.replace_accounted_selection(uncontrolled, "not authorized", Some(&field), &f::ctl()).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    let foreign = session(f::Case::Point, 1);
    let wrong = selection(&foreign, 0.5);
    let target = AccountedSelectionTarget::capture_with_control(&s, &f::ctl()).unwrap();
    assert!(s.replace_accounted_selection(target, "wrong ledger", Some(&wrong), &f::ctl()).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    let deny = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
    assert!(AccountedSelectionTarget::capture_with_control(&s, &c).is_err());
    let p = json!({});
    assert!(crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "select.all", &p, &c).is_err());
    // Original cumulative ledger survives the captured target; independently
    // lowering the remaining allowance cannot receive a fresh capture budget.
    use std::sync::atomic::{AtomicU64, Ordering};
    let spent = AtomicU64::new(0);
    let ceiling = AtomicU64::new(u64::MAX);
    let meter = |units: u64| {
        let old = spent.fetch_add(units, Ordering::Relaxed);
        if old.checked_add(units).is_none_or(|v| v > ceiling.load(Ordering::Relaxed)) { Err(BackingError::Work) } else { Ok(()) }
    };
    let cumulative = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &meter };
    let captured = AccountedSelectionTarget::capture_with_control(&s, &cumulative).unwrap();
    ceiling.store(spent.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
    assert!(captured.validate_with_control(&s, &cumulative).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    let yes = || true;
    let interrupt = photocraft_raster::Interrupt::cancel_only(&yes);
    assert!(AccountedSelectionTarget::capture_with_control(&s, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    install(&mut s, 1.0);
    let all = s.active().unwrap().doc.clone();
    let target = AccountedSelectionTarget::capture_with_control(&s, &f::ctl()).unwrap();
    s.active_mut().unwrap().source.as_mut().unwrap().label.push_str(" stale noop");
    let installed = selection(&s, 1.0);
    assert!(s.replace_accounted_selection(target, "equal field", Some(&installed), &f::ctl()).is_err());
    assert!(all.shares_state(&s.active().unwrap().doc));
    // Inverse is now admitted only with its existing closed parameter grammar;
    // Color Range retains its prior independent raw-sample/Legacy policy.
    assert!(s.execute("select.inverse", json!({"unexpected":true})).is_err());
    assert!(s.execute("select.colorRange", json!({})).is_err());
}
