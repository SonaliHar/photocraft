//! Deferred native continuation of shared editing consumers. No producer oracle.
//! Encoded/comparison buffers are external harness allocations, not receipts.
use super::*;
#[path = "accounted_native_stroke_test_support.rs"]
mod narrow;
#[path = "accounted_programme_paint_test_support.rs"]
mod programme;
#[path = "accounted_wide_plain_xcf_test_support.rs"]
mod wide;
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::accounted_selection::{AccountedSelection, CoverageDomain};
use serde_json::json;

#[test]
#[ignore = "UNRUN/uncompiled: genuine programme Sources solid Fill/Clear, exact programme/mask/source/sibling custody through native continuation and repeated edit/undo/redo; existing recipe domains, no numerical oracle"]
fn programme_fill_clear_native_continuation_preserves_programmes() {
    for case in [programme::Case::Point, programme::Case::FilteredFlat, programme::Case::MaskedFilteredFlat, programme::Case::Hierarchy] {
        let input = programme::load(case);
        let witness = programme::Witness::capture(input.owner(), &programme::ctl());
        let mut s = Session::new();
        programme::adopt(&mut s, case, &input);
        assert_eq!(s.active().unwrap().doc.depth, SampleType::F32);
        let target = programme::target(&s.active().unwrap().doc);
        let original = programme::raw(&s.active().unwrap().doc, target);
        let identity = s.active().unwrap().source.clone();
        s.execute("edit.fill", json!({"layer":target.0,"contents":"color","color":[0.2,0.7,0.4,0.6]})).unwrap();
        let filled = programme::raw(&s.active().unwrap().doc, target);
        assert_ne!(filled, original);
        witness.preserved(&s.active().unwrap().doc, target, &programme::ctl());
        assert_eq!(s.active().unwrap().source, identity);
        assert!(s.active().unwrap().source_read_only);
        assert!(s.undo_with_control(&programme::ctl()).unwrap());
        assert_eq!(programme::raw(&s.active().unwrap().doc, target), original);
        assert!(s.redo_with_control(&programme::ctl()).unwrap());
        assert_eq!(programme::raw(&s.active().unwrap().doc, target), filled);
        let mut native = Session::new();
        programme::reopen(&mut native, case, programme::archive(s.active().unwrap().doc.accounted().unwrap()));
        witness.preserved(&native.active().unwrap().doc, target, &programme::ctl());
        assert_eq!(programme::raw(&native.active().unwrap().doc, target), filled);
        let native_identity = native.active().unwrap().source.clone();
        native.execute("edit.clear", json!({"layer":target.0})).unwrap();
        let cleared = programme::raw(&native.active().unwrap().doc, target);
        assert_ne!(cleared, filled);
        witness.preserved(&native.active().unwrap().doc, target, &programme::ctl());
        assert_eq!(native.active().unwrap().source, native_identity);
        assert!(native.undo_with_control(&programme::ctl()).unwrap());
        assert_eq!(programme::raw(&native.active().unwrap().doc, target), filled);
        assert!(native.redo_with_control(&programme::ctl()).unwrap());
        assert_eq!(programme::raw(&native.active().unwrap().doc, target), cleared);
        let mut reopened = Session::new();
        programme::reopen(&mut reopened, case, programme::archive(native.active().unwrap().doc.accounted().unwrap()));
        witness.preserved(&reopened.active().unwrap().doc, target, &programme::ctl());
        assert_eq!(programme::raw(&reopened.active().unwrap().doc, target), cleared);
        assert_eq!(programme::raw(witness.before(), target), original);
    }
}

fn selection_word(sample: SampleType) -> Vec<u8> {
    match sample {
        SampleType::U8 => vec![128],
        SampleType::U16 => 0x8001u16.to_ne_bytes().to_vec(),
        SampleType::U32 => 0x80000001u32.to_ne_bytes().to_vec(),
        SampleType::F32 => 0.50000006f32.to_ne_bytes().to_vec(),
        SampleType::F64 => (0.5 + 2f64.powi(-45)).to_ne_bytes().to_vec(),
        _ => panic!("five native selection depths"),
    }
}
fn plain_session(sample: SampleType, locked: bool) -> Session {
    // The genuine source owner needs scratch for the independent guarded
    // selection constructor; earlier no-selection fixtures intentionally have 0.
    let bytes = if matches!(sample, SampleType::U32 | SampleType::F64) {
        wide::encoded(false, sample, locked)
    } else {
        let (precision, color, alpha) = match sample {
            SampleType::U8 => (150, vec![63], vec![191]),
            SampleType::U16 => (250, 0x4001u16.to_be_bytes().to_vec(), 0xc001u16.to_be_bytes().to_vec()),
            SampleType::F32 => (650, 0.25000003f32.to_be_bytes().to_vec(), 0.75000006f32.to_be_bytes().to_vec()),
            _ => panic!("narrow fixture format"),
        };
        let mut pixel = color.repeat(3);
        pixel.extend(alpha);
        let mut options = xcf::FixtureOptions::default();
        if locked {
            options.layer_properties.push((10, 1u32.to_be_bytes().to_vec()));
        }
        xcf::image_custom(13, precision, 0, 257, 259, &pixel.repeat(257 * 259), &options)
    };
    let quota = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 512 << 20,
        index: 64 << 20,
        scratch: 16 << 20,
    });
    let source = photocraft_io::import_owned_staged_plain_bytes_in_quota(
        "selected-source.xcf",
        std::sync::Arc::new(bytes),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        quota,
        &programme::ctl(),
    )
    .unwrap();
    let mut s = Session::new();
    s.add_owned_plain_xcf_import(&source, "selected-source.xcf", Some("selected-source.xcf"), &programme::ctl()).unwrap();
    s.tools.brush.pressure_size = false;
    s
}
fn selection_bytes(s: &Session) -> Vec<u8> {
    let doc = &s.active().unwrap().doc;
    doc.selection.as_ref().unwrap().try_read_interleaved(doc.bounds()).unwrap()
}

#[test]
#[ignore = "UNRUN/uncompiled: generated Legacy five-depth guarded fractional selection survives native brush COW/native reopen/second edit/history with exact native mask words and source state; no external numerical or UI acceptance"]
fn selected_native_brush_reopens_and_keeps_guarded_selection_words() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        for locked in [false, true] {
            let mut s = plain_session(sample, locked);
            let bounds = s.active().unwrap().doc.bounds();
            let word = selection_word(sample);
            let bytes = word.repeat(bounds.width() as usize * bounds.height() as usize);
            let selection = AccountedSelection::try_from_native(
                PixelFormat { mode: ColorMode::Grayscale, sample, alpha: false },
                bounds,
                &vec![0; word.len()],
                &bytes,
                CoverageDomain::FiniteUnit,
                s.active().unwrap().doc.quota().unwrap(),
                &programme::ctl(),
            )
            .unwrap();
            let target = crate::accounted_selection_session::AccountedSelectionTarget::capture(&s).unwrap();
            s.replace_accounted_selection(target, "fractional selection", Some(&selection), &programme::ctl()).unwrap();
            let before = s.active().unwrap().doc.clone();
            let original = narrow::payload(&before);
            let identity = s.active().unwrap().source.clone();
            s.execute("paint.stroke", narrow::patch()).unwrap();
            let painted = narrow::payload(&s.active().unwrap().doc);
            assert_ne!(painted, original);
            assert_eq!(selection_bytes(&s), bytes);
            assert_eq!(s.active().unwrap().source, identity);
            assert_eq!(s.active().unwrap().doc.xcf_state, before.xcf_state);
            let mut native = Session::new();
            native
                .add_owned_native_archive(
                    programme::archive(s.active().unwrap().doc.accounted().unwrap()),
                    AccountedNativeOpenPolicy::KeepCurrentInterpretation,
                    "selected-copy.pcraft",
                    None,
                    &programme::ctl(),
                )
                .unwrap();
            assert_eq!(selection_bytes(&native), bytes);
            assert_eq!(narrow::payload(&native.active().unwrap().doc), painted);
            native.tools.brush.pressure_size = false;
            let mut patch = narrow::patch();
            patch["color"] = json!([0.12500000000000003, 0.6250000000000001, 0.8750000000000001, 1.0]);
            native.execute("paint.stroke", patch).unwrap();
            let second = narrow::payload(&native.active().unwrap().doc);
            assert_ne!(second, painted);
            assert_eq!(selection_bytes(&native), bytes);
            assert!(native.undo_with_control(&programme::ctl()).unwrap());
            assert_eq!(narrow::payload(&native.active().unwrap().doc), painted);
            assert_eq!(selection_bytes(&native), bytes);
            assert!(native.redo_with_control(&programme::ctl()).unwrap());
            assert_eq!(narrow::payload(&native.active().unwrap().doc), second);
            assert_eq!(selection_bytes(&native), bytes);
            assert_eq!(narrow::payload(&before), original);
        }
    }
}
