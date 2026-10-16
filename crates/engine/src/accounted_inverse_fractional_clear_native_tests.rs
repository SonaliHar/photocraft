//! Deferred native continuation for native Inverse and fractional wide Clear.
//! Encoded/comparison buffers belong to the external harness, not receipts.
use super::*;
#[path = "accounted_programme_selection_native_test_support.rs"]
mod f;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::accounted_selection::{AccountedSelection, CoverageDomain};
use serde_json::json;

#[test]
#[ignore = "UNRUN/uncompiled: genuine programme F32/F64 native-format Inverse preserves image/programmes/source, exact mask/native continuation and fractional Clear/undo/redo across typed saves; no old resident Gray8-equivalence or numerical oracle"]
fn native_inverse_fractional_clear_and_native_reopen_keep_custody() {
    for case in f::CASES {
        for depth in 0..2 {
            let input = f::load(case, depth);
            let witness = f::Witness::capture(input.owner(), &f::ctl());
            let mut s = Session::new();
            f::adopt(&mut s, case, &input);
            let doc = &s.active().unwrap().doc;
            let bounds = doc.bounds();
            let sample = doc.depth;
            let (word, inverse) = match sample {
                SampleType::F32 => {
                    let v = 0.25000003f32;
                    (v.to_ne_bytes().to_vec(), (1.0f32 - v).to_ne_bytes().to_vec())
                }
                SampleType::F64 => {
                    let v = 0.25 + 2f64.powi(-45);
                    (v.to_ne_bytes().to_vec(), (1.0f64 - v).to_ne_bytes().to_vec())
                }
                _ => panic!("two genuine native Source depths"),
            };
            let pixels = bounds.width() as usize * bounds.height() as usize;
            let selection = AccountedSelection::try_from_native(
                PixelFormat::new(ColorMode::Grayscale, sample, false),
                bounds,
                &vec![0; word.len()],
                &word.repeat(pixels),
                CoverageDomain::FiniteUnit,
                doc.quota().unwrap(),
                &f::ctl(),
            )
            .unwrap();
            let layer = f::base::target(doc);
            let original = f::raw(doc, layer);
            let source = s.active().unwrap().source.clone();
            let target = crate::accounted_selection_session::AccountedSelectionTarget::capture_with_control(&s, &f::ctl()).unwrap();
            s.replace_accounted_selection(target, "fractional selection", Some(&selection), &f::ctl()).unwrap();
            s.execute("select.inverse", json!({})).unwrap();
            let inverted = s.active().unwrap().doc.clone();
            assert_eq!(inverted.selection.as_ref().unwrap().format().sample, sample);
            assert_eq!(inverted.selection.as_ref().unwrap().try_read_interleaved(bounds).unwrap(), inverse.repeat(pixels));
            assert_eq!(f::raw(&inverted, layer), original);
            assert_eq!(s.active().unwrap().source, source);
            assert!(s.active().unwrap().source_read_only);
            witness.preserved(&inverted, layer, &f::ctl());

            let mut native = Session::new();
            f::reopen(&mut native, case, f::archive(inverted.accounted().unwrap()));
            f::same_selection(&inverted, &native.active().unwrap().doc);
            assert_eq!(f::raw(&native.active().unwrap().doc, layer), original);
            native.execute("edit.clear", json!({"layer":layer.0})).unwrap();
            let cleared = native.active().unwrap().doc.clone();
            let changed = f::raw(&cleared, layer);
            assert_ne!(changed, original);
            witness.preserved(&cleared, layer, &f::ctl());
            f::same_selection(&inverted, &cleared);
            assert!(native.undo_with_control(&f::ctl()).unwrap());
            assert_eq!(f::raw(&native.active().unwrap().doc, layer), original);
            f::same_selection(&inverted, &native.active().unwrap().doc);
            assert!(native.redo_with_control(&f::ctl()).unwrap());
            assert_eq!(f::raw(&native.active().unwrap().doc, layer), changed);
            let mut reopen = Session::new();
            f::reopen(&mut reopen, case, f::archive(native.active().unwrap().doc.accounted().unwrap()));
            witness.preserved(&reopen.active().unwrap().doc, layer, &f::ctl());
            f::same_selection(&inverted, &reopen.active().unwrap().doc);
            assert_eq!(f::raw(&reopen.active().unwrap().doc, layer), changed);
            assert_eq!(f::raw(witness.before(), layer), original);
        }
    }
}
