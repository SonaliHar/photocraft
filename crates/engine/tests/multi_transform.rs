use photocraft_doc::{LayerContent, LayerId};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;

fn scene(depth: u32) -> (Session, LayerId, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 96, "depth": depth})).unwrap();
    let mut layer = |name| LayerId(s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap());
    let a = layer("A");
    let middle = layer("Unselected");
    let b = layer("B");
    s.edit("paint", |doc, _| {
        for (id, rect, color) in [
            (a, Rect::new(10, 20, 20, 30), [1.0, 0.0, 0.0, 1.0]),
            (middle, Rect::new(30, 20, 40, 30), [0.0, 1.0, 0.0, 1.0]),
            (b, Rect::new(60, 20, 70, 30), [0.0, 0.0, 1.0, 1.0]),
        ] {
            doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(rect, &color);
        }
        Ok(())
    })
    .unwrap();
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    (s, a, middle, b)
}

fn bounds(s: &Session, id: LayerId) -> Rect {
    s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds()
}

#[test]
fn selected_layers_scale_about_the_shared_center_and_undo_together_at_every_depth() {
    for depth in [8, 16, 32] {
        let (mut s, a, middle, b) = scene(depth);
        let before = s.active().unwrap().history.past_len();
        let result = s.execute("edit.transform", json!({"scaleX": 200, "scaleY": 200, "interpolation": "nearest"})).unwrap();
        assert_eq!(result["rect"], json!([10.0, 20.0, 70.0, 30.0]));
        assert_eq!(bounds(&s, a), Rect::new(-20, 15, 0, 35));
        assert_eq!(bounds(&s, b), Rect::new(80, 15, 100, 35));
        assert_eq!(bounds(&s, middle), Rect::new(30, 20, 40, 30));
        assert_eq!(s.active().unwrap().history.past_len(), before + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(bounds(&s, a), Rect::new(10, 20, 20, 30));
        assert_eq!(bounds(&s, b), Rect::new(60, 20, 70, 30));
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(bounds(&s, a), Rect::new(-20, 15, 0, 35));
        assert_eq!(bounds(&s, b), Rect::new(80, 15, 100, 35));
        assert_eq!(s.active().unwrap().selected_layers(), vec![a, b]);
    }
}

#[test]
fn explicit_targets_override_the_current_selection() {
    let (mut s, a, middle, b) = scene(8);
    s.execute("layer.select", json!({"layer": middle.0})).unwrap();
    s.execute("edit.transform", json!({"layers": [a.0, b.0], "dx": 5, "dy": -3, "interpolation": "nearest"})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(15, 17, 25, 27));
    assert_eq!(bounds(&s, b), Rect::new(65, 17, 75, 27));
    assert_eq!(bounds(&s, middle), Rect::new(30, 20, 40, 30));
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    s.execute("edit.transform", json!({"layer": a.0, "dx": 5, "interpolation": "nearest"})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(15, 20, 25, 30));
    assert_eq!(bounds(&s, b), Rect::new(60, 20, 70, 30));
}

#[test]
fn selecting_a_group_and_its_child_does_not_transform_the_child_twice() {
    let (mut s, a, middle, b) = scene(8);
    let group = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    s.execute("edit.transform", json!({"dx": 5, "interpolation": "nearest"})).unwrap();
    assert!(s.active().unwrap().doc.layer(group).unwrap().is_group());
    assert_eq!(bounds(&s, a), Rect::new(15, 20, 25, 30));
    assert_eq!(bounds(&s, b), Rect::new(65, 20, 75, 30));
    assert_eq!(bounds(&s, middle), Rect::new(30, 20, 40, 30));
}

#[test]
fn one_locked_target_refuses_the_whole_transform_without_history() {
    let (mut s, a, _, b) = scene(8);
    s.edit("lock", |doc, _| {
        doc.layer_mut(b).unwrap().locks.position = true;
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().history.past_len();
    assert!(s.execute("edit.transform", json!({"dx": 5})).is_err());
    assert_eq!(bounds(&s, a), Rect::new(10, 20, 20, 30));
    assert_eq!(bounds(&s, b), Rect::new(60, 20, 70, 30));
    assert_eq!(s.active().unwrap().history.past_len(), before);
}

#[test]
fn invalid_target_lists_and_excessive_results_fail_without_changing_any_layer() {
    let (mut s, a, _, b) = scene(8);
    let before = s.active().unwrap().history.past_len();
    for params in [
        json!({"layers": [], "dx": 1}),
        json!({"layers": "all", "dx": 1}),
        json!({"layers": [null], "dx": 1}),
        json!({"layers": [-1], "dx": 1}),
        json!({"layers": [u64::MAX], "dx": 1}),
        json!({"layers": [a.0, b.0], "layer": a.0, "dx": 1}),
        json!({"layers": [a.0, a.0, a.0, a.0, a.0], "dx": 1}),
        json!({"layers": [a.0, b.0], "scaleX": 1e12, "scaleY": 1e12}),
    ] {
        assert!(s.execute("edit.transform", params.clone()).is_err(), "{params}");
        assert_eq!(bounds(&s, a), Rect::new(10, 20, 20, 30));
        assert_eq!(bounds(&s, b), Rect::new(60, 20, 70, 30));
        assert_eq!(s.active().unwrap().history.past_len(), before);
    }
    assert!(Session::new().execute("edit.transform", json!({"layers": [a.0, b.0], "dx": 1})).is_err());
}

#[test]
fn type_stays_editable_when_transformed_with_a_pixel_layer() {
    let (mut s, a, _, _) = scene(8);
    let text = LayerId(s.execute("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap()["layer"].as_u64().unwrap());
    let before = match &s.active().unwrap().doc.layer(text).unwrap().content {
        LayerContent::Text(text) => text.transform,
        _ => panic!("type layer"),
    };
    s.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 4, 7], "interpolation": "nearest"})).unwrap();
    let LayerContent::Text(text) = &s.active().unwrap().doc.layer(text).unwrap().content else { panic!("type stays editable") };
    assert_eq!(text.text, "Sample");
    assert_eq!(text.transform.m[4], before.m[4] + 4.0);
    assert_eq!(text.transform.m[5], before.m[5] + 7.0);
    assert_eq!(bounds(&s, a), Rect::new(14, 27, 24, 37));
}
