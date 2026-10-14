//! Generative Fill: the provider result lands on a new layer, the document is untouched on every
//! failure, and the errors are plain sentences.

use std::sync::Arc;

use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::json;

use super::*;
use crate::ai::testing::FakeProvider;
use crate::jobs::Started;

/// A 64×48 document of `mode` at `depth` with a selection around the middle.
fn doc_session(depth: u32, mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth, "mode": mode, "background": "white"})).unwrap();
    s.execute("select.rect", json!({"x": 26, "y": 16, "width": 12, "height": 14})).unwrap();
    s
}

/// The Content-Aware Fill fixture: white canvas, vertical black stripes, a red blob to remove.
fn blob_session(depth: u32) -> Session {
    let mut s = doc_session(depth, "rgb");
    s.edit("stripes", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        for x in (0..64).step_by(8) {
            surf.fill_rect(Rect::new(x, 0, x + 3, 48), &[0.0, 0.0, 0.0, 1.0]);
        }
        surf.fill_rect(Rect::new(28, 18, 36, 28), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

fn use_provider(s: &mut Session, provider: FakeProvider) {
    s.set_ai_provider(Some(Arc::new(provider)));
}

fn active_id(s: &Session) -> LayerId {
    s.active().unwrap().active_layer.unwrap()
}

fn name_of(s: &Session, id: LayerId) -> String {
    s.active().unwrap().doc.layer(id).unwrap().name.clone()
}

fn named(s: &Session, name: &str) -> Option<LayerId> {
    s.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.name == name).map(|(_, _, l)| l.id)
}

/// Every sample of a layer over the canvas, in row order.
fn snapshot(s: &Session, id: LayerId) -> Vec<f32> {
    let st = s.active().unwrap();
    let doc = &st.doc;
    let surf = doc.layer(id).unwrap().surface().unwrap().clone();
    let b = doc.bounds();
    let mut out = Vec::new();
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            out.extend(surf.pixel(x, y));
        }
    }
    out
}

/// Alpha of one pixel of a layer (0.0 where it has no pixels).
fn alpha_at(s: &Session, id: LayerId, x: i32, y: i32) -> f32 {
    let st = s.active().unwrap();
    match st.doc.layer(id).unwrap().surface() {
        Some(surf) => surf.pixel(x, y).last().copied().unwrap_or(0.0),
        None => 0.0,
    }
}

#[test]
fn the_result_lands_on_a_new_layer_named_generative_fill_and_the_source_is_untouched() {
    let mut s = blob_session(8);
    let base = active_id(&s);
    let before = snapshot(&s, base);
    let (layers, steps) = (s.active().unwrap().doc.layer_count(), s.active().unwrap().history.past_len());
    use_provider(&mut s, FakeProvider { polls: 1, ..Default::default() });
    let r = s.execute(CMD, json!({})).unwrap();
    assert!(r["filled"].as_u64().unwrap() > 0, "{r}");
    let new = active_id(&s);
    assert_ne!(new, base, "a new layer, not the source");
    assert_eq!(r["layer"].as_u64().unwrap(), new.0, "the result names the new layer");
    assert_eq!(name_of(&s, new), "Generative Fill", "named exactly as D1 decided");
    assert_eq!(named(&s, "Generative Fill"), Some(new));
    // Directly above the layer that was active.
    let order: Vec<LayerId> = s.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.id).collect();
    let (at, above) = (order.iter().position(|i| *i == new).unwrap(), order.iter().position(|i| *i == base).unwrap());
    assert_eq!(at, above + 1, "inserted above the active layer");
    // B01: the source pixels are byte-identical, and only one layer and one history step appeared.
    assert_eq!(snapshot(&s, base), before, "the source layer's pixels never change");
    assert_eq!(s.active().unwrap().doc.layer_count(), layers + 1);
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one undo step");
    // The fill really did something inside the selection.
    let filled = s.active().unwrap().doc.layer(new).unwrap().surface().unwrap().pixel(32, 23);
    assert!((filled[0] - 0.75).abs() < 0.02, "the provider's grey landed: {filled:?}");
}

#[test]
fn one_undo_removes_the_generative_layer() {
    let mut s = blob_session(8);
    let base = active_id(&s);
    let (layers, steps) = (s.active().unwrap().doc.layer_count(), s.active().unwrap().history.past_len());
    use_provider(&mut s, FakeProvider::default());
    s.execute(CMD, json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer_count(), layers + 1);
    assert!(s.undo(), "one undo step");
    let st = s.active().unwrap();
    assert_eq!(st.doc.layer_count(), layers);
    assert_eq!(st.active_layer, Some(base), "the source layer is active again");
    assert_eq!(st.history.past_len(), steps);
    assert!(named(&s, "Generative Fill").is_none(), "the new layer is gone");
}

#[test]
fn a_second_run_numbers_the_layer_instead_of_reusing_the_name() {
    let mut s = blob_session(8);
    use_provider(&mut s, FakeProvider::default());
    s.execute(CMD, json!({})).unwrap();
    s.execute(CMD, json!({})).unwrap();
    assert!(named(&s, "Generative Fill").is_some());
    assert!(named(&s, "Generative Fill 2").is_some(), "no two layers share a name");
}

#[test]
fn pixels_outside_the_selection_stay_transparent_and_a_feather_blends_by_coverage() {
    // Hard-edged selection: the new layer is transparent outside it, opaque inside.
    let mut s = blob_session(8);
    use_provider(&mut s, FakeProvider::default());
    s.execute(CMD, json!({})).unwrap();
    let new = active_id(&s);
    for (x, y) in [(2, 2), (25, 23), (40, 23), (32, 8), (32, 35)] {
        assert_eq!(alpha_at(&s, new, x, y), 0.0, "outside the selection at ({x},{y})");
    }
    for (x, y) in [(27, 17), (37, 29), (32, 23)] {
        assert_eq!(alpha_at(&s, new, x, y), 1.0, "inside the selection at ({x},{y})");
    }
    // Feathered selection: the edge is partial coverage, so the fill blends in rather than cutting.
    let mut s = blob_session(8);
    s.execute("select.modify.feather", json!({"radius": 4})).unwrap();
    use_provider(&mut s, FakeProvider::default());
    s.execute(CMD, json!({})).unwrap();
    let new = active_id(&s);
    let edge = alpha_at(&s, new, 25, 23);
    assert!(edge > 0.0 && edge < 1.0, "the feathered edge blends by coverage: {edge}");
    assert_eq!(alpha_at(&s, new, 2, 2), 0.0, "still nothing far outside");
    assert!(alpha_at(&s, new, 32, 23) > 0.7, "still mostly filled in the middle");
    let partial = (16..30).map(|y| alpha_at(&s, new, 25, y)).filter(|a| *a > 0.0 && *a < 1.0).count();
    assert!(partial > 0, "a band of partial coverage, not one row");
}

#[test]
fn a_missing_or_off_canvas_selection_is_refused_in_content_aware_fill_s_words() {
    for rect in [None, Some(json!({"x": 200, "y": 200, "width": 8, "height": 8})), Some(json!({"x": -60, "y": -60, "width": 20, "height": 20}))] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48, "background": "white"})).unwrap();
        if let Some(rect) = &rect {
            s.execute("select.rect", rect.clone()).ok();
        }
        use_provider(&mut s, FakeProvider::default());
        let generative = s.execute(CMD, json!({}));
        let content_aware = s.execute("edit.contentAwareFill", json!({}));
        assert!(generative.is_err(), "{rect:?}: {generative:?}");
        assert!(content_aware.is_err(), "{rect:?}: {content_aware:?}");
        // Same reason; the command id in the message is the only difference.
        let reason = |result: &Result<serde_json::Value>| match result {
            Err(EngineError::Disabled(_, why)) => why.clone(),
            other => format!("{other:?}"),
        };
        assert_eq!(reason(&generative), reason(&content_aware), "{rect:?}");
    }
}

#[test]
fn without_a_provider_the_message_names_the_manager() {
    let mut s = blob_session(8);
    assert_eq!(s.execute(CMD, json!({})).unwrap_err().to_string(), "Craft AI Manager isn't running");
    // B04: the menu item stays enabled without a provider, so the user gets the message.
    assert!(s.is_enabled(CMD), "not greyed out");
    s.execute("select.deselect", json!({})).unwrap();
    assert!(!s.is_enabled(CMD), "but greyed out with no selection");
}

#[test]
fn a_prompt_is_refused_until_the_provider_supports_one() {
    for prompt in [json!("a cat"), json!(" "), json!("   \n")] {
        let mut s = blob_session(8);
        use_provider(&mut s, FakeProvider::default());
        let want = if prompt.as_str().unwrap_or_default().trim().is_empty() { Ok(()) } else { Err(()) };
        let got = s.execute(CMD, json!({"prompt": prompt})).map(|_| ());
        match want {
            Ok(()) => assert!(got.is_ok(), "blank prompt {prompt} runs: {got:?}"),
            Err(()) => assert_eq!(got.unwrap_err().to_string(), "Prompts aren't supported yet; leave the prompt empty to remove the selection", "{prompt}"),
        }
    }
    // A prompt of the wrong type names the parameter.
    let mut s = blob_session(8);
    use_provider(&mut s, FakeProvider::default());
    assert!(s.execute(CMD, json!({"prompt": 7})).unwrap_err().to_string().contains("`prompt`"));
    // Null and absent are the same as no prompt at all.
    for p in [json!({}), json!({"prompt": null}), json!({"prompt": ""})] {
        let mut s = blob_session(8);
        use_provider(&mut s, FakeProvider::default());
        assert!(s.execute(CMD, p.clone()).is_ok(), "{p}");
    }
}

#[test]
fn non_rgb_documents_are_refused_at_8_and_16_bit() {
    for mode in ["gray", "cmyk", "lab"] {
        for depth in [8, 16] {
            let mut s = doc_session(depth, mode);
            use_provider(&mut s, FakeProvider::default());
            let e = s.execute(CMD, json!({})).unwrap_err();
            assert_eq!(e.to_string(), "Generative Fill works on RGB documents for now", "{mode} at {depth}-bit");
            let st = s.active().unwrap();
            assert_eq!(st.doc.layer_count(), 1, "{mode} at {depth}-bit added a layer");
        }
    }
}

#[test]
fn rgb_works_at_8_16_and_32_bit() {
    for depth in [8, 16, 32] {
        let mut s = blob_session(depth);
        use_provider(&mut s, FakeProvider::default());
        let r = s.execute(CMD, json!({})).unwrap();
        assert!(r["filled"].as_u64().unwrap() > 0, "{depth}-bit: {r}");
        assert_eq!(name_of(&s, active_id(&s)), "Generative Fill", "{depth}-bit");
    }
}

#[test]
fn a_bad_provider_result_is_an_error_and_the_document_is_unchanged() {
    let cases = [
        (FakeProvider { fail: Some("The model ran out of memory".into()), ..Default::default() }, "The model ran out of memory"),
        (FakeProvider { wrong_size: true, ..Default::default() }, "does not match the requested window"),
        (FakeProvider { wrong_format: true, ..Default::default() }, "does not match the requested window"),
        (FakeProvider { bad_pixel: true, ..Default::default() }, "The AI result contains invalid pixel values; the document is unchanged"),
    ];
    for (provider, want) in cases {
        let mut s = blob_session(8);
        let base = active_id(&s);
        let before = snapshot(&s, base);
        let (layers, steps) = (s.active().unwrap().doc.layer_count(), s.active().unwrap().history.past_len());
        use_provider(&mut s, provider);
        let e = s.execute(CMD, json!({})).unwrap_err();
        assert!(e.to_string().contains(want), "{want}: {e}");
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer_count(), layers, "{want}: a layer appeared");
        assert_eq!(st.history.past_len(), steps, "{want}: history changed");
        assert_eq!(snapshot(&s, base), before, "{want}: the source layer changed");
        assert!(named(&s, "Generative Fill").is_none(), "{want}: the layer was named anyway");
    }
}

#[test]
fn cancelling_the_job_leaves_the_document_untouched() {
    let mut s = blob_session(8);
    let base = active_id(&s);
    let before = snapshot(&s, base);
    let (layers, steps) = (s.active().unwrap().doc.layer_count(), s.active().unwrap().history.past_len());
    use_provider(&mut s, FakeProvider { never_finish: true, ..Default::default() });
    let id = match s.start(CMD, json!({})).unwrap() {
        Started::Job(id) => id,
        Started::Done(v) => panic!("expected a background job, ran inline: {v}"),
    };
    assert!(s.cancel_job(id));
    assert!(matches!(s.wait_job(id), Err(EngineError::Cancelled)));
    s.join_cancelled_jobs();
    let st = s.active().unwrap();
    assert_eq!(st.doc.layer_count(), layers, "no layer appeared");
    assert_eq!(st.history.past_len(), steps, "no history step");
    assert_eq!(st.active_layer, Some(base), "the source layer is still active");
    assert_eq!(snapshot(&s, base), before, "the pixels never changed");
    assert!(named(&s, "Generative Fill").is_none());
}

#[test]
fn the_job_is_labelled_generative_fill() {
    let mut s = blob_session(8);
    use_provider(&mut s, FakeProvider { never_finish: true, ..Default::default() });
    let id = match s.start(CMD, json!({})).unwrap() {
        Started::Job(id) => id,
        Started::Done(v) => panic!("expected a background job: {v}"),
    };
    let info = s.job(id).unwrap();
    assert_eq!(info.label, "Generative Fill");
    assert_eq!(info.command, CMD);
    s.cancel_job(id);
    let _ = s.wait_job(id);
    s.join_cancelled_jobs();
}

#[test]
fn the_command_is_registered_once_under_edit() {
    let specs = specs();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].id, "edit.generativeFill");
    assert_eq!(specs[0].menu, &["Edit"]);
    assert_eq!(specs[0].label, "Generative Fill\u{2026}");
    assert!(specs[0].journal);
    let all = crate::command_specs();
    assert_eq!(all.iter().filter(|c| c.id == CMD).count(), 1, "registered exactly once");
}
