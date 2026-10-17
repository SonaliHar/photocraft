//! #2260: copying an irregular selection retains RGB under zero alpha. The Magic Wand
//! must select the visible transparent area, independently of those hidden colours.

use photocraft_doc::LayerId;
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;

#[test]
fn wand_after_irregular_layer_copy_ignores_hidden_rgb() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 24, "depth": depth, "background": "#ff0000"})).unwrap();
        let original = s.active().unwrap().active_layer.unwrap();
        s.edit("Two-colour source", |doc, _| {
            doc.layer_mut(original).unwrap().surface_mut().unwrap().fill_rect(Rect::new(16, 0, 32, 24), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let area = s.active().unwrap().doc.bounds();
        let source = s.active().unwrap().doc.layer(original).unwrap().surface().unwrap().read_region(area);
        s.execute("select.lasso", json!({"points": [[4, 4], [28, 4], [4, 20]], "antiAlias": false})).unwrap();
        let before_copy = s.active().unwrap().doc.clone();
        let copied = LayerId(s.execute("layer.new.layerViaCopy", json!({})).unwrap()["layer"].as_u64().unwrap());
        let after_copy = s.active().unwrap().doc.clone();
        assert!(after_copy.selection.is_none());
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before_copy, "undo restores exact source pixels and coverage");
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc, after_copy, "redo restores exact copy and deselection");
        s.execute("layer.setProps", json!({"layer": original.0, "visible": false})).unwrap();
        let copied_pixels = s.active().unwrap().doc.layer(copied).unwrap().surface().unwrap().clone();
        assert!(s.active().unwrap().doc.selection.is_none(), "Layer via Copy deselects");
        // The copied triangle does not divide the canvas: every transparent pixel connects
        // to (0, 0), including differently coloured zero-alpha pixels inside its bounding box.
        assert_eq!(copied_pixels.rgba(24, 16)[3], 0.0);
        assert_eq!(copied_pixels.rgba(6, 6)[3], 1.0);
        for (tolerance, contiguous, all_layers) in [(0, true, false), (32, false, false), (32, true, true)] {
            let steps = s.active().unwrap().history.past_len();
            s.execute(
                "select.magicWand",
                json!({"x": 0, "y": 0, "tolerance": tolerance, "contiguous": contiguous, "antiAlias": false, "sampleAllLayers": all_layers}),
            )
            .unwrap();
            let st = s.active().unwrap();
            let mask = st.doc.selection.as_ref().unwrap();
            for y in area.y0..area.y1 {
                for x in area.x0..area.x1 {
                    let expected = if copied_pixels.rgba(x, y)[3] == 0.0 { 1.0 } else { 0.0 };
                    assert_eq!(
                        mask.sample_channel(x, y, 0),
                        expected,
                        "depth={depth}, tolerance={tolerance}, contiguous={contiguous}, allLayers={all_layers}, at ({x}, {y})"
                    );
                }
            }
            assert_eq!(st.doc.layer(original).unwrap().surface().unwrap().read_region(area), source, "copy and selection preserve the original pixels");
            assert_eq!(st.doc.layer(copied).unwrap().surface().unwrap().read_region(area), copied_pixels.read_region(area), "the wand only changes selection");
            assert_eq!(st.history.past_len(), steps + 1);
            assert!(s.undo());
            let st = s.active().unwrap();
            assert!(st.doc.selection.is_none(), "one undo clears the wand selection");
            assert!(st.doc.layer(copied).is_some(), "undo preserves the preceding copy");
            assert!(!st.doc.layer(original).unwrap().visible);
        }
    }
}

#[test]
fn layer_via_copy_and_cut_use_existing_inline_cancellation_before_publication() {
    for command in ["layer.new.layerViaCopy", "layer.new.layerViaCut"] {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":16,"height":12,"depth":16,"background":"#336699"})).unwrap();
        session.execute("select.rect", json!({"x":2,"y":2,"width":8,"height":6})).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let history = session.active().unwrap().history.past_len();
        let journal = session.journal.clone();
        let context = photocraft_engine::jobs::JobCtx::new();
        context.cancel();
        session.set_inline_job_ctx(Some(context.clone()));
        assert!(matches!(session.execute(command, json!({})), Err(photocraft_engine::EngineError::Cancelled)), "{command}");
        assert!(context.cancelled(), "the original context is reused, not replaced");
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().revision, revision);
        assert_eq!(session.active().unwrap().history.past_len(), history);
        assert_eq!(session.journal, journal);
        session.set_inline_job_ctx(None);
        session.execute(command, json!({})).unwrap();
        let after = session.active().unwrap().doc.clone();
        assert!(after.selection.is_none());
        assert_eq!(session.active().unwrap().history.past_len(), history + 1);
        assert!(session.undo());
        assert_eq!(session.active().unwrap().doc, before);
        assert!(session.redo());
        assert_eq!(session.active().unwrap().doc, after);
    }
}
