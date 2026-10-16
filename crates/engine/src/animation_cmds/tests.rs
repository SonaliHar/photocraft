use super::*;
use photocraft_doc::LayerId;
use serde_json::json;

/// A 16×12 document with `names` as layers, bottom to top (the new-document Background is first).
fn session(names: &[&str]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    for n in names {
        s.execute("layer.new.layer", json!({"name": n})).unwrap();
    }
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn shown(s: &Session) -> Vec<(String, bool)> {
    doc(s).layers.iter().map(|l| (l.name.clone(), l.visible)).collect()
}

fn delays(s: &Session) -> Vec<u32> {
    doc(s).frame_animation.as_ref().unwrap().frames.iter().map(|f| f.delay_ms).collect()
}

fn current(s: &Session) -> usize {
    doc(s).frame_animation.as_ref().unwrap().current
}

fn id_of(s: &Session, name: &str) -> LayerId {
    doc(s).layers.iter().find(|l| l.name == name).unwrap().id
}

#[test]
fn make_frames_from_layers_shows_one_layer_per_frame() {
    let mut s = session(&["A", "B"]);
    let r = s.execute("animation.makeFramesFromLayers", json!({"seconds": 0.2})).unwrap();
    assert_eq!(r["animation"]["frames"].as_array().unwrap().len(), 3);
    assert_eq!(delays(&s), [200, 200, 200]);
    assert_eq!(shown(&s), [("Background".into(), true), ("A".into(), false), ("B".into(), false)]);
    s.execute("animation.selectFrame", json!({"frame": 2})).unwrap();
    assert_eq!(shown(&s), [("Background".into(), false), ("A".into(), false), ("B".into(), true)]);
    assert_eq!(current(&s), 2);
}

#[test]
fn visibility_edits_change_only_the_selected_frame() {
    let mut s = session(&["A"]);
    s.execute("animation.create", json!({})).unwrap();
    s.execute("animation.newFrame", json!({})).unwrap();
    assert_eq!(current(&s), 1);
    let a = id_of(&s, "A");
    s.execute("layer.hideLayers", json!({"layer": a.0})).unwrap();
    let anim = doc(&s).frame_animation.as_ref().unwrap();
    assert!(anim.frames[0].hidden.is_empty());
    assert_eq!(anim.frames[1].hidden, [a]);
    s.execute("animation.selectFrame", json!({"frame": 0})).unwrap();
    assert!(doc(&s).layer(a).unwrap().visible);
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    assert!(!doc(&s).layer(a).unwrap().visible);
}

#[test]
fn a_new_layer_shows_in_every_frame() {
    let mut s = session(&[]);
    s.execute("animation.create", json!({})).unwrap();
    s.execute("animation.newFrame", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Late"})).unwrap();
    for f in [0, 1] {
        s.execute("animation.selectFrame", json!({"frame": f})).unwrap();
        assert!(shown(&s).iter().any(|(n, v)| n == "Late" && *v), "frame {f}");
    }
}

#[test]
fn delays_apply_to_the_selected_frame_a_list_or_all() {
    let mut s = session(&["A", "B"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    s.execute("animation.setDelay", json!({"seconds": 0.5})).unwrap();
    assert_eq!(delays(&s), [500, 0, 0]);
    s.execute("animation.setDelay", json!({"delayMs": 40, "frames": [1, 2]})).unwrap();
    assert_eq!(delays(&s), [500, 40, 40]);
    s.execute("animation.setDelay", json!({"seconds": 0.1, "frames": "all"})).unwrap();
    assert_eq!(delays(&s), [100, 100, 100]);
}

#[test]
fn frame_changes_undo_and_redo() {
    let mut s = session(&["A"]);
    s.execute("animation.create", json!({})).unwrap();
    s.execute("animation.newFrame", json!({})).unwrap();
    s.execute("animation.setDelay", json!({"seconds": 1})).unwrap();
    assert_eq!(delays(&s), [0, 1000]);
    assert!(s.undo());
    assert_eq!(delays(&s), [0, 0]);
    assert!(s.undo());
    assert_eq!(delays(&s), [0]);
    assert!(s.redo());
    assert!(s.redo());
    assert_eq!(delays(&s), [0, 1000]);
}

#[test]
fn selecting_a_frame_is_not_a_history_step_and_keeps_a_clean_document_clean() {
    let mut s = session(&["A"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    let steps = s.active().unwrap().history.can_undo();
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    s.execute("animation.nextFrame", json!({})).unwrap();
    s.execute("animation.nextFrame", json!({})).unwrap();
    assert_eq!(current(&s), 1, "next stops at the last frame");
    s.execute("animation.previousFrame", json!({})).unwrap();
    s.execute("animation.previousFrame", json!({})).unwrap();
    assert_eq!(current(&s), 0);
    assert_eq!(s.active().unwrap().history.can_undo(), steps);
}

#[test]
fn loop_counts() {
    let mut s = session(&[]);
    s.execute("animation.create", json!({})).unwrap();
    for (v, plays) in [(json!("once"), 1), (json!(5), 5), (json!("forever"), 0), (json!(1), 1), (json!(0), 0)] {
        let r = s.execute("animation.setLoop", json!({"loops": v})).unwrap();
        assert_eq!(doc(&s).frame_animation.as_ref().unwrap().loops.plays(), plays, "{r}");
    }
    assert!(s.execute("animation.setLoop", json!({"loops": "sometimes"})).is_err());
    assert!(s.execute("animation.setLoop", json!({})).is_err());
}

#[test]
fn move_reverse_and_delete_keep_the_selection_on_its_frame() {
    let mut s = session(&["A", "B"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    for (i, ms) in [(0, 10), (1, 20), (2, 30)] {
        s.execute("animation.setDelay", json!({"delayMs": ms, "frames": [i]})).unwrap();
    }
    s.execute("animation.selectFrame", json!({"frame": 0})).unwrap();
    s.execute("animation.moveFrame", json!({"from": 0, "to": 2})).unwrap();
    assert_eq!(delays(&s), [20, 30, 10]);
    assert_eq!(current(&s), 2, "the selected frame moved with it");
    s.execute("animation.reverseFrames", json!({})).unwrap();
    assert_eq!(delays(&s), [10, 30, 20]);
    assert_eq!(current(&s), 0);
    s.execute("animation.deleteFrame", json!({"frame": 1})).unwrap();
    assert_eq!(delays(&s), [10, 20]);
    assert_eq!(current(&s), 0);
    s.execute("animation.deleteFrame", json!({})).unwrap();
    assert_eq!(delays(&s), [20]);
    assert!(s.execute("animation.deleteFrame", json!({})).is_err(), "the last frame stays");
}

#[test]
fn deleting_the_animation_keeps_the_layers_as_they_show() {
    let mut s = session(&["A"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    let before = shown(&s);
    s.execute("animation.delete", json!({})).unwrap();
    assert!(doc(&s).frame_animation.is_none());
    assert_eq!(shown(&s), before);
    assert!(s.execute("animation.newFrame", json!({})).is_err());
}

#[test]
fn bad_params_are_errors_not_panics() {
    let mut s = session(&["A"]);
    assert!(s.execute("animation.selectFrame", json!({"frame": 0})).is_err(), "no animation yet");
    s.execute("animation.create", json!({})).unwrap();
    assert!(s.execute("animation.create", json!({})).is_err(), "only one");
    for (cmd, p) in [
        ("animation.selectFrame", json!({"frame": 9})),
        ("animation.selectFrame", json!({})),
        ("animation.selectFrame", json!({"frame": -1})),
        ("animation.setDelay", json!({})),
        ("animation.setDelay", json!({"seconds": -1})),
        ("animation.setDelay", json!({"seconds": 1e12})),
        ("animation.setDelay", json!({"seconds": "soon"})),
        ("animation.setDelay", json!({"seconds": 1, "frames": [7]})),
        ("animation.setDelay", json!({"seconds": 1, "frames": 3})),
        ("animation.moveFrame", json!({"from": 0, "to": 5})),
        ("animation.deleteFrame", json!({"frame": 4})),
    ] {
        assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
    }
}

#[test]
fn pasted_frames_go_after_the_selection_and_undo() {
    let mut s = session(&["A", "B"]);
    s.execute("animation.makeFramesFromLayers", json!({"delayMs": 100})).unwrap();
    let b = id_of(&s, "B");
    s.execute("animation.pasteFrames", json!({"frames": [{"delayMs": 250, "hidden": [b.0]}, {"delayMs": 300}], "after": 0})).unwrap();
    assert_eq!(delays(&s), [100, 250, 300, 100, 100]);
    assert_eq!(current(&s), 2, "the last pasted frame is selected");
    assert!(doc(&s).layers.iter().all(|l| l.visible), "the selected pasted frame hides nothing");
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    assert!(!doc(&s).layers.iter().find(|l| l.id == b).unwrap().visible);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(delays(&s), [100, 100, 100]);
}

#[test]
fn pasting_drops_layers_the_document_lacks_and_rejects_bad_params() {
    let mut s = session(&["A"]);
    s.execute("animation.create", json!({})).unwrap();
    s.execute("animation.pasteFrames", json!({"frames": [{"delayMs": 50, "hidden": [999_999]}]})).unwrap();
    let a = doc(&s).frame_animation.as_ref().unwrap();
    assert!(a.frames[1].hidden.is_empty());
    for bad in
        [json!({}), json!({"frames": []}), json!({"frames": [{"hidden": "x"}]}), json!({"frames": [{"delayMs": -1}]}), json!({"frames": [{}], "after": 9})]
    {
        assert!(s.execute("animation.pasteFrames", bad).is_err());
    }
    assert_eq!(delays(&s).len(), 2);
}

fn four(s: &mut Session) {
    for (i, ms) in [10, 20, 30, 40].iter().enumerate() {
        s.execute("animation.setDelay", json!({"delayMs": ms, "frames": [i]})).unwrap();
    }
}

fn four_frames() -> Session {
    let mut s = session(&["A", "B", "C"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    four(&mut s);
    s
}

#[test]
fn several_frames_delete_and_duplicate_together() {
    let mut s = four_frames();
    let r = s.execute("animation.newFrame", json!({"frames": [1, 2]})).unwrap();
    assert_eq!(delays(&s), [10, 20, 30, 20, 30, 40]);
    assert_eq!(r["selection"], json!([3, 4]));
    assert_eq!(current(&s), 4);
    let r = s.execute("animation.deleteFrame", json!({"frames": [0, 3, 4]})).unwrap();
    assert_eq!(delays(&s), [20, 30, 40]);
    assert_eq!(r["selection"], json!([2]), "the selected frame was deleted: the one after it takes over");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(delays(&s).len(), 6);
    assert!(s.execute("animation.deleteFrame", json!({"frames": [0, 1, 2, 3, 4, 5]})).is_err(), "one frame stays");
}

#[test]
fn a_group_of_frames_moves_into_a_gap_in_order() {
    let mut s = four_frames();
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    let r = s.execute("animation.moveFrames", json!({"frames": [0, 1], "slot": 4})).unwrap();
    assert_eq!(delays(&s), [30, 40, 10, 20]);
    assert_eq!(r["selection"], json!([2, 3]));
    assert_eq!(current(&s), 3, "the selected frame went with its group");
    let r = s.execute("animation.moveFrames", json!({"frames": [1, 3], "slot": 0})).unwrap();
    assert_eq!(delays(&s), [40, 20, 30, 10]);
    assert_eq!(r["selection"], json!([0, 1]));
    s.execute("animation.moveFrames", json!({"frames": [2], "slot": 2})).unwrap();
    assert_eq!(delays(&s), [40, 20, 30, 10], "dropping a frame into its own gap changes nothing");
    for bad in [json!({"frames": [0]}), json!({"frames": [0], "slot": 5}), json!({"frames": [9], "slot": 0}), json!({"frames": [], "slot": 0})] {
        assert!(s.execute("animation.moveFrames", bad).is_err());
    }
}

fn opacity(s: &Session, frame: usize, name: &str) -> f32 {
    doc(s).frame_animation.as_ref().unwrap().frames[frame].opacity_of(id_of(s, name)).unwrap()
}

fn hidden(s: &Session, frame: usize, name: &str) -> bool {
    doc(s).frame_animation.as_ref().unwrap().frames[frame].hidden.contains(&id_of(s, name))
}

#[test]
fn tween_fades_between_two_frames() {
    let mut s = session(&["A"]);
    s.execute("animation.makeFramesFromLayers", json!({"delayMs": 100})).unwrap();
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    let r = s.execute("animation.tween", json!({"frames": 3})).unwrap();
    assert_eq!(delays(&s), [100; 5]);
    assert_eq!(r["selection"], json!([1, 2, 3]));
    assert_eq!(current(&s), 3);
    // Frame 0 shows only the Background, frame 4 only A: the new frames cross-fade.
    for (f, bg, a) in [(1, 0.75, 0.25), (2, 0.5, 0.5), (3, 0.25, 0.75)] {
        assert!((opacity(&s, f, "Background") - bg).abs() < 1e-5, "frame {f}");
        assert!((opacity(&s, f, "A") - a).abs() < 1e-5, "frame {f}");
        assert!(!hidden(&s, f, "Background") && !hidden(&s, f, "A"), "both show while fading");
    }
    // The selected frame's look is on the layers.
    assert!((doc(&s).layers.iter().find(|l| l.name == "A").unwrap().opacity - 0.75).abs() < 1e-5);
    // The ends are untouched.
    assert!(hidden(&s, 0, "A") && hidden(&s, 4, "Background"));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(delays(&s).len(), 2);
}

#[test]
fn tween_can_go_the_other_way_and_pick_layers() {
    let mut s = session(&["A"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    s.execute("animation.selectFrame", json!({"frame": 0})).unwrap();
    let a = id_of(&s, "A");
    s.execute("animation.tween", json!({"with": "next", "frames": 1, "layers": [a.0]})).unwrap();
    assert_eq!(delays(&s).len(), 3);
    assert!((opacity(&s, 1, "A") - 0.5).abs() < 1e-5);
    assert!(!hidden(&s, 1, "Background") && opacity(&s, 1, "Background") == 1.0, "a layer that isn't tweened keeps the selected frame's look");
    assert_eq!(current(&s), 1);
}

#[test]
fn tween_refuses_what_it_cannot_do() {
    let mut s = session(&["A"]);
    s.execute("animation.makeFramesFromLayers", json!({})).unwrap();
    s.execute("animation.selectFrame", json!({"frame": 0})).unwrap();
    for bad in [
        json!({}),
        json!({"with": "first"}),
        json!({"with": "sideways"}),
        json!({"with": "next", "frames": 0}),
        json!({"with": "next", "frames": 501}),
        json!({"with": "next", "layers": "x"}),
    ] {
        assert!(s.execute("animation.tween", bad).is_err());
    }
    assert_eq!(delays(&s).len(), 2);
}

#[test]
fn opacity_edits_change_only_the_selected_frame() {
    let mut s = session(&["A"]);
    s.execute("animation.create", json!({})).unwrap();
    s.execute("animation.newFrame", json!({})).unwrap();
    let a = id_of(&s, "A");
    s.execute("layer.setProps", json!({"layer": a.0, "opacity": 0.3})).unwrap();
    assert!((opacity(&s, 1, "A") - 0.3).abs() < 1e-5);
    assert_eq!(opacity(&s, 0, "A"), 1.0);
    s.execute("animation.selectFrame", json!({"frame": 0})).unwrap();
    assert_eq!(doc(&s).layer(a).unwrap().opacity, 1.0);
    s.execute("animation.selectFrame", json!({"frame": 1})).unwrap();
    assert!((doc(&s).layer(a).unwrap().opacity - 0.3).abs() < 1e-5);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc(&s).layer(a).unwrap().opacity, 1.0);
}

#[test]
fn pasted_frames_carry_their_opacities() {
    let mut s = session(&["A"]);
    s.execute("animation.create", json!({})).unwrap();
    let a = id_of(&s, "A");
    s.execute("animation.pasteFrames", json!({"frames": [{"delayMs": 10, "opacities": [[a.0, 0.4], [999_999, 0.1]]}]})).unwrap();
    assert!((opacity(&s, 1, "A") - 0.4).abs() < 1e-5);
    assert_eq!(doc(&s).frame_animation.as_ref().unwrap().frames[1].opacities.len(), 2, "unknown layers dropped, known ones all recorded");
    assert!(s.execute("animation.pasteFrames", json!({"frames": [{"opacities": [[1]]}]})).is_err());
}
