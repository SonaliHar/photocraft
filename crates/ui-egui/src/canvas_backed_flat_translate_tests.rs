//! UNRUN Move over actual original flat native sources; shifted view/selection,
//! locks and effect references are explicitly synthetic CURRENT controls.
use super::*;
#[path = "canvas_flat_translate_fixture.rs"]
mod fixture;
use fixture::ctl;
use serde_json::{Value, json};
fn preview(original: &photocraft_doc::Document, current: &photocraft_engine::DocumentSnapshot) {
    let expected = display_image(None, &fixture::reference(original, current));
    let (actual, fee) = backed_snapshot_display_image(current, current.size.width, current.size.height, None, &Interrupt::NONE).unwrap();
    for (a, b) in actual.pixels.iter().zip(&expected.pixels) {
        for c in 0..4 {
            assert!((i16::from(a.to_array()[c]) - i16::from(b.to_array()[c])).abs() <= 1);
        }
    }
    drop((actual, fee));
}
#[test]
#[ignore = "UNRUN: six hash-pinned original flat inputs with genuine CURRENT native selected view, registered UI Move bg=true, inverse literal native kernel reference, exact raw/default/policy/profile/effectreference custody and native continuation; no GIMP translation claim"]
fn flat_current_ui_move_is_synchronous_atomic_and_keeps_native_sources() {
    for case in fixture::cases() {
        let source = fixture::import(&case);
        let mut source_app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut source_app, "Original.xcf", &source, &ctl()).unwrap();
        let authored = source_app.session.active().unwrap().doc.clone();
        fixture::observed(&case, &authored);
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut app, "Current selection.pcraft", fixture::seed(authored.accounted().unwrap(), false), &ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let bottom = before.layers[0].id;
        let top = before.layers.last().unwrap().id;
        assert_eq!(app.session.active().unwrap().selection_ids(), vec![top, bottom]);
        let past = app.session.active().unwrap().history.past_len();
        let journal = app.session.journal.clone();
        app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("flat move denied".into())));
        assert!(app.run("layer.translate", json!({"dx":1})).unwrap_err().contains("flat move denied"));
        app.session.authorize = None;
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(app.session.journal, journal);
        // Explicit name overrides selected multi-target; rounding is half away from zero.
        assert!(app.run("layer.translate", json!({"layer":before.layers[0].name,"dx":1.49,"dy":-0.5,"units":"px","coalesce":"flat\0move"})).unwrap().is_null());
        let first = app.session.active().unwrap().doc.clone();
        fixture::exact(&before, &first, &[(bottom, 1, -1)]);
        app.run("layer.setProps", json!({"layer":bottom.0,"name":"Moved CURRENT bottom","coalesce":"flat\0move"})).unwrap();
        app.run("layer.translate", json!({"dx":-2,"dy":1,"coalesce":"flat\0move"})).unwrap();
        let moved = app.session.active().unwrap().doc.clone();
        fixture::exact(&before, &moved, &[(bottom, -1, 0), (top, -2, 1)]);
        preview(&before, &moved);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(app.session.active().unwrap().history.entry_labels().last(), Some("Move"));
        assert!(!app.session.has_jobs());
        assert!(app.jobs.last_started.is_none());
        app.run("edit.undo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let redo = app.session.active().unwrap().history.clone();
        app.run("layer.translate", json!({"layer":"invalid target intentionally ignored","dx":0.49,"dy":-0.49,"coalesce":"flat\0move"})).unwrap();
        assert!(app.session.active().unwrap().history.shares_accounted_history(&redo));
        app.run("edit.redo", Value::Null).unwrap();
        // Exact same upstream point conversion, independent from source DPI assumptions.
        let dpi = f64::from(moved.resolution_dpi);
        app.run("layer.translate", json!({"layer":bottom.0,"dx":72.0/dpi,"dy":0,"units":"pt"})).unwrap();
        let point = app.session.active().unwrap().doc.clone();
        fixture::exact(&moved, &point, &[(bottom, 1, 0)]);
        app.run("edit.undo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&moved));
        let history = app.session.active().unwrap().history.clone();
        for params in [json!({"dx":1,"target":"mask"}), json!({"dx":1,"target":{"channel":1}}), json!({"dx":2147483647})] {
            assert!(app.run("layer.translate", params).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&moved));
            assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        }
        let named = moved.accounted().unwrap().try_with_document_name("Moved flat CURRENT", &ctl()).unwrap();
        let q = named.quota().clone();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "Other.pcraft", fixture::archive(&named), &ctl()).unwrap();
        let current = reopened.session.active().unwrap().doc.clone();
        assert_eq!(current.name, "Moved flat CURRENT");
        fixture::exact(&moved, &current, &[]);
        assert!(q.shares_ledger(current.accounted().unwrap().quota()));
        preview(&before, &current);
        // Any selected position lock refuses the whole transaction, even if the first target is free.
        let mut locked = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut locked, "Locked current.pcraft", fixture::seed(authored.accounted().unwrap(), true), &ctl()).unwrap();
        let old = locked.session.active().unwrap().doc.clone();
        assert!(locked.run("layer.translate", json!({"dx":1})).is_err());
        assert!(locked.session.active().unwrap().doc.shares_state(&old));
        assert_eq!(std::fs::read(&case.path).unwrap(), case.input);
    }
}
