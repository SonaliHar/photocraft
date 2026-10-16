//! Deferred actual programme-selection custody through native continuation.
use super::*;
#[path = "accounted_programme_selection_native_test_support.rs"]
mod f;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::accounted_selection::{AccountedSelection, CoverageDomain};
use serde_json::json;

#[test]
#[ignore = "UNRUN/uncompiled: genuine four-class F32/F64 programme Sources retain guarded fractional native selection/programmes/masks/source through paint/native reopen/second edit/undo/redo and Deselect; no new Source selection or numerical oracle"]
fn programme_selection_native_reopen_and_continued_edit_keep_real_custody() {
    for case in f::CASES {
        for depth in 0..2 {
            let input = f::load(case, depth);
            let witness = f::Witness::capture(input.owner(), &f::ctl());
            let mut s = Session::new();
            f::adopt(&mut s, case, &input);
            s.tools.brush.pressure_size = false;
            let doc = &s.active().unwrap().doc;
            let bounds = doc.bounds();
            let sample = doc.depth;
            let word = match sample {
                SampleType::F32 => 0.50000006f32.to_ne_bytes().to_vec(),
                SampleType::F64 => (0.5 + 2f64.powi(-45)).to_ne_bytes().to_vec(),
                _ => panic!("two existing native floating Source depths"),
            };
            let bytes = word.repeat(bounds.width() as usize * bounds.height() as usize);
            let selection = AccountedSelection::try_from_native(
                PixelFormat::new(ColorMode::Grayscale, sample, false),
                bounds,
                &vec![0; word.len()],
                &bytes,
                CoverageDomain::FiniteUnit,
                doc.quota().unwrap(),
                &f::ctl(),
            )
            .unwrap();
            let target = crate::accounted_selection_session::AccountedSelectionTarget::capture_with_control(&s, &f::ctl()).unwrap();
            s.replace_accounted_selection(target, "fractional programme selection", Some(&selection), &f::ctl()).unwrap();
            let selected = s.active().unwrap().doc.clone();
            let layer = f::base::target(&selected);
            let original = f::raw(&selected, layer);
            let identity = s.active().unwrap().source.clone();
            witness.preserved(&selected, layer, &f::ctl());
            selected.accounted().unwrap().checked_programme_selection(&f::ctl()).unwrap();
            s.execute("paint.stroke", f::patch(&selected, layer)).unwrap();
            let painted = f::raw(&s.active().unwrap().doc, layer);
            assert_ne!(painted, original);
            witness.preserved(&s.active().unwrap().doc, layer, &f::ctl());
            f::same_selection(&selected, &s.active().unwrap().doc);
            assert_eq!(s.active().unwrap().source, identity);
            assert!(s.active().unwrap().source_read_only);

            let mut native = Session::new();
            f::reopen(&mut native, case, f::archive(s.active().unwrap().doc.accounted().unwrap()));
            let reopened = native.active().unwrap().doc.clone();
            reopened.accounted().unwrap().checked_programme_selection(&f::ctl()).unwrap();
            witness.preserved(&reopened, layer, &f::ctl());
            f::same_selection(&selected, &reopened);
            assert_eq!(f::raw(&reopened, layer), painted);
            assert!(!native.active().unwrap().source_read_only);
            native.tools.brush.pressure_size = false;
            let mut patch = f::patch(&reopened, layer);
            patch["color"] = json!([0.12500000000000003, 0.6250000000000001, 0.8750000000000001, 1.0]);
            native.execute("paint.stroke", patch).unwrap();
            let second = f::raw(&native.active().unwrap().doc, layer);
            assert_ne!(second, painted);
            witness.preserved(&native.active().unwrap().doc, layer, &f::ctl());
            f::same_selection(&selected, &native.active().unwrap().doc);
            assert!(native.undo_with_control(&f::ctl()).unwrap());
            assert_eq!(f::raw(&native.active().unwrap().doc, layer), painted);
            f::same_selection(&selected, &native.active().unwrap().doc);
            assert!(native.redo_with_control(&f::ctl()).unwrap());
            assert_eq!(f::raw(&native.active().unwrap().doc, layer), second);
            native.execute("select.deselect", json!({})).unwrap();
            assert!(native.active().unwrap().doc.selection.is_none());
            assert_eq!(f::raw(&native.active().unwrap().doc, layer), second);
            witness.preserved(&native.active().unwrap().doc, layer, &f::ctl());
            let mut deselected = Session::new();
            f::reopen(&mut deselected, case, f::archive(native.active().unwrap().doc.accounted().unwrap()));
            assert!(deselected.active().unwrap().doc.selection.is_none());
            assert_eq!(f::raw(&deselected.active().unwrap().doc, layer), second);
            witness.preserved(&deselected.active().unwrap().doc, layer, &f::ctl());
            assert_eq!(f::raw(witness.before(), layer), original);
        }
    }
}
