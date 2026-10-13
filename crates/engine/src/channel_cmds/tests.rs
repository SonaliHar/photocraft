use super::*;
use photocraft_color::SampleType;

fn session_depth(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 20, "depth": depth})).unwrap();
    s
}

fn session() -> Session {
    session_depth(8)
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn chan(s: &Session, i: usize, x: i32, y: i32) -> f32 {
    doc(s).channels[i].surface.sample_channel(x, y, 0)
}

fn sel_at(s: &Session, x: i32, y: i32) -> f32 {
    doc(s).selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
}

fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
}

fn rect(s: &mut Session, x: i32, y: i32, w: u32, h: u32, mode: &str) {
    s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h, "mode": mode})).unwrap();
}

#[test]
fn specs_are_documented_and_registered() {
    for sp in specs() {
        assert!(crate::commands::find(sp.id).is_some(), "{} not registered", sp.id);
        assert!(sp.params.starts_with('{'), "{}", sp.id);
    }
}

#[test]
fn save_selection_new_and_operations() {
    for depth in [8, 16, 32] {
        let mut s = session_depth(depth);
        assert!(s.execute("select.saveSelection", json!({})).is_err(), "needs a selection");
        rect(&mut s, 0, 0, 10, 20, "replace");
        let r = s.execute("select.saveSelection", json!({})).unwrap();
        assert_eq!(r["channel"], 0);
        assert_eq!(r["name"], "Alpha 1");
        let d = doc(&s);
        assert_eq!(d.channels[0].surface.format(), channel_format(d));
        assert_eq!(d.channels[0].surface.format().sample, SampleType::ALL[[8, 16, 32].iter().position(|v| *v == depth).unwrap()]);
        assert_eq!(chan(&s, 0, 5, 5), 1.0);
        assert_eq!(chan(&s, 0, 15, 5), 0.0);

        // add / subtract / intersect / replace into the existing channel
        rect(&mut s, 5, 0, 10, 20, "replace");
        s.execute("select.saveSelection", json!({"channel": 0, "operation": "add"})).unwrap();
        assert_eq!((chan(&s, 0, 2, 5), chan(&s, 0, 12, 5), chan(&s, 0, 20, 5)), (1.0, 1.0, 0.0));
        rect(&mut s, 0, 0, 3, 20, "replace");
        s.execute("select.saveSelection", json!({"channel": 0, "operation": "subtract"})).unwrap();
        assert_eq!((chan(&s, 0, 1, 5), chan(&s, 0, 5, 5)), (0.0, 1.0));
        rect(&mut s, 10, 0, 30, 20, "replace");
        s.execute("select.saveSelection", json!({"channel": "Alpha 1", "operation": "intersect"})).unwrap();
        assert_eq!((chan(&s, 0, 5, 5), chan(&s, 0, 12, 5)), (0.0, 1.0));
        s.execute("select.saveSelection", json!({"channel": 0, "operation": "replace", "name": "Mine"})).unwrap();
        assert_eq!((chan(&s, 0, 5, 5), chan(&s, 0, 30, 5)), (0.0, 1.0));
        assert_eq!(doc(&s).channels[0].name, "Mine");
        assert!(s.execute("select.saveSelection", json!({"operation": "bogus"})).is_err());
        assert!(s.execute("select.saveSelection", json!({"channel": 7, "operation": "add"})).is_err());
    }
}

#[test]
fn load_selection_operations_and_invert() {
    let mut s = session();
    rect(&mut s, 0, 0, 20, 20, "replace");
    s.execute("select.saveSelection", json!({})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("select.loadSelection", json!({"channel": 0})).unwrap();
    assert_eq!((sel_at(&s, 5, 5), sel_at(&s, 25, 5)), (1.0, 0.0));
    s.execute("select.loadSelection", json!({"channel": 0, "invert": true})).unwrap();
    assert_eq!((sel_at(&s, 5, 5), sel_at(&s, 25, 5)), (0.0, 1.0));
    rect(&mut s, 10, 0, 20, 20, "replace");
    s.execute("select.loadSelection", json!({"channel": 0, "operation": "add"})).unwrap();
    assert_eq!((sel_at(&s, 5, 5), sel_at(&s, 25, 5), sel_at(&s, 35, 5)), (1.0, 1.0, 0.0));
    rect(&mut s, 10, 0, 20, 20, "replace");
    s.execute("select.loadSelection", json!({"channel": 0, "operation": "subtract"})).unwrap();
    assert_eq!((sel_at(&s, 15, 5), sel_at(&s, 25, 5)), (0.0, 1.0));
    rect(&mut s, 10, 0, 20, 20, "replace");
    s.execute("select.loadSelection", json!({"channel": "Alpha 1", "operation": "intersect"})).unwrap();
    assert_eq!((sel_at(&s, 15, 5), sel_at(&s, 25, 5)), (1.0, 0.0));
    assert!(s.execute("select.loadSelection", json!({})).is_err());
    assert!(s.execute("select.loadSelection", json!({"channel": 0, "operation": "xor"})).is_err());
    // undo restores the previous selection
    s.undo();
    assert_eq!(sel_at(&s, 25, 5), 1.0);
}

#[test]
fn load_transparency_mask_and_composite() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10, "background": "transparent"})).unwrap();
    rect(&mut s, 0, 0, 5, 10, "replace");
    s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("select.loadSelection", json!({"channel": "transparency"})).unwrap();
    assert_eq!((sel_at(&s, 2, 2), sel_at(&s, 10, 2)), (1.0, 0.0));
    // Composite luminosity: white = selected; transparent over nothing = black.
    s.execute("select.loadSelection", json!({"channel": "composite"})).unwrap();
    assert!(sel_at(&s, 2, 2) > 0.99);
    s.execute("select.loadSelection", json!({"channel": "red"})).unwrap();
    assert!(sel_at(&s, 2, 2) > 0.99);
    // Layer mask
    assert!(s.execute("select.loadSelection", json!({"channel": "mask"})).is_err());
    s.execute("layer.layerMask.hideAll", json!({})).unwrap();
    s.execute("select.loadSelection", json!({"channel": "mask", "invert": true})).unwrap();
    assert_eq!(sel_at(&s, 10, 5), 1.0);
}

#[test]
fn channel_management_and_view_state() {
    let mut s = session();
    let r = s.execute("channel.new", json!({"fill": "white", "name": "W"})).unwrap();
    assert_eq!(r["channel"], 0);
    assert_eq!(chan(&s, 0, 3, 3), 1.0);
    // A new channel is targeted and shown alone.
    let st = s.active().unwrap();
    assert_eq!(st.channel_view.target, ChannelTarget::Alpha(0));
    assert!(st.channel_view.alpha_shown(0) && st.channel_view.visible_colors(3) == 0);
    s.execute("channel.new", json!({})).unwrap();
    assert_eq!(doc(&s).channels[1].name, "Alpha 1");
    s.execute("channel.duplicate", json!({"channel": 0})).unwrap();
    assert_eq!(doc(&s).channels[2].name, "W copy");
    s.execute("channel.duplicate", json!({"channel": "red", "invert": true})).unwrap();
    assert_eq!(doc(&s).channels[3].name, "Red copy");
    assert_eq!(chan(&s, 3, 1, 1), 0.0, "white image, inverted");
    s.execute("channel.rename", json!({"channel": 1, "name": "Renamed"})).unwrap();
    assert!(s.execute("channel.rename", json!({"channel": 1, "name": " "})).is_err());
    s.execute("channel.move", json!({"channel": "Renamed", "to": 0})).unwrap();
    let names: Vec<_> = doc(&s).channels.iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, ["Renamed", "W", "W copy", "Red copy"]);
    s.execute("channel.target", json!({"channel": 1})).unwrap();
    s.execute("channel.delete", json!({"channel": 0})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Alpha(0), "target follows its channel");
    s.execute("channel.delete", json!({})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Composite);
    assert_eq!(doc(&s).channels.len(), 2);
    // Visibility toggles (eye) for colour, composite, alpha.
    s.execute("channel.setVisible", json!({"channel": "blue", "visible": false})).unwrap();
    assert!(!s.active().unwrap().channel_view.color_visible(2));
    s.execute("channel.setVisible", json!({"channel": "composite"})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.visible_colors(3), 3);
    s.execute("channel.setVisible", json!({"channel": 1})).unwrap();
    assert!(s.active().unwrap().channel_view.alpha_shown(1));
    assert!(!s.active().unwrap().channel_view.is_plain(doc(&s)));
    // Viewing state doesn't dirty or add history.
    let hist = s.active().unwrap().history.entries(&crate::read_control_none()).unwrap().values().to_vec().len();
    s.execute("channel.target", json!({"channel": "green"})).unwrap();
    assert_eq!(s.active().unwrap().history.entries(&crate::read_control_none()).unwrap().values().to_vec().len(), hist);
    let j = s.execute("channel.list", json!({})).unwrap();
    assert_eq!(j["target"]["kind"], "color");
    assert_eq!(j["alpha"].as_array().unwrap().len(), 2);
    // Undo of the delete brings the channel back; the view stays valid.
    s.undo();
    assert_eq!(doc(&s).channels.len(), 3);
}

#[test]
fn shortcut_slots_target_channels() {
    let mut s = session();
    s.execute("channel.new", json!({})).unwrap();
    s.execute("channel.target.slot3", json!({})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Color(0));
    s.execute("channel.target.slot6", json!({})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Alpha(0));
    assert!(!s.is_enabled("channel.target.slot7"));
    // The reason names the shortcut in the platform's notation (⌘7 on the Mac, Ctrl+7 elsewhere).
    let err = s.execute("channel.target.slot7", json!({})).unwrap_err().to_string();
    assert!(err.contains(if cfg!(target_os = "macos") { "no channel for ⌘7" } else { "no channel for Ctrl+7" }), "{err}");
    s.execute("channel.target.composite", json!({})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Composite);
    // Grayscale: ⌘3 is the first alpha channel.
    let mut g = Session::new();
    g.execute("file.new", json!({"width": 4, "height": 4, "mode": "gray"})).unwrap();
    g.execute("channel.new", json!({})).unwrap();
    g.execute("channel.target.slot3", json!({})).unwrap();
    assert_eq!(g.active().unwrap().channel_view.target, ChannelTarget::Alpha(0));
}

#[test]
fn painting_filtering_and_adjusting_into_an_alpha_channel() {
    for depth in [8, 16, 32] {
        let mut s = session_depth(depth);
        s.execute("channel.new", json!({})).unwrap();
        let before = px(&s, 10, 10);
        // Targeted channel: the stroke goes into the channel, not the layer.
        s.execute("paint.stroke", json!({"points": [[10, 10]], "size": 8, "hardness": 1.0, "color": "#ffffff"})).unwrap();
        assert!(chan(&s, 0, 10, 10) > 0.99, "depth {depth}");
        assert_eq!(chan(&s, 0, 30, 10), 0.0);
        assert_eq!(px(&s, 10, 10), before);
        // Erasing paints the background colour (white) on channels; use black foreground fill.
        s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
        assert!((chan(&s, 0, 30, 10) - 0.5).abs() < 0.01);
        // Adjustments apply to the channel (also where it was untouched).
        s.execute("image.adjustments.invert", json!({})).unwrap();
        assert!((chan(&s, 0, 30, 10) - 0.5).abs() < 0.01);
        assert_eq!(px(&s, 10, 10), before, "layer untouched by the adjustment");
        // Filters too, limited by the selection.
        s.execute("channel.new", json!({})).unwrap();
        s.execute("paint.stroke", json!({"points": [[20, 10]], "size": 6, "hardness": 1.0, "color": "#ffffff"})).unwrap();
        rect(&mut s, 0, 0, 20, 20, "replace");
        s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
        assert!(chan(&s, 1, 16, 10) > 0.0, "blurred inside the selection");
        assert_eq!(chan(&s, 1, 26, 10), 0.0, "outside the selection untouched");
        // Explicit targets work without targeting the channel.
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("paint.bucket", json!({"x": 1, "y": 1, "color": "#ffffff", "target": {"channel": 1}, "tolerance": 0})).unwrap();
        assert_eq!(chan(&s, 1, 1, 1), 1.0);
        assert!(s.execute("paint.stroke", json!({"points": [[1, 1]], "target": {"channel": 9}})).is_err());
        // Undo restores the channel.
        s.undo();
        assert_eq!(chan(&s, 1, 1, 1), 0.0);
    }
}

#[test]
fn single_colour_channel_target_limits_edits() {
    let mut s = session();
    s.execute("channel.target", json!({"channel": "red"})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    assert_eq!(px(&s, 5, 5), vec![0.0, 1.0, 1.0, 1.0]);
    s.execute("channel.target", json!({"channel": "blue"})).unwrap();
    s.execute("paint.stroke", json!({"points": [[20, 10]], "size": 6, "hardness": 1.0, "color": "#000000"})).unwrap();
    assert_eq!(px(&s, 20, 10), vec![0.0, 1.0, 0.0, 1.0]);
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    assert_eq!(px(&s, 5, 5), vec![0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn quick_mask_enter_paint_exit() {
    let mut s = session();
    rect(&mut s, 0, 0, 20, 20, "replace");
    s.execute("select.editInQuickMaskMode", json!({})).unwrap();
    let d = doc(&s);
    let q = d.quick_mask.as_ref().unwrap();
    assert!(d.selection.is_none());
    assert_eq!((q.surface.sample_channel(5, 5, 0), q.surface.sample_channel(30, 5, 0)), (1.0, 0.0));
    assert_eq!((q.opacity, q.indicates), (0.5, ColorIndicates::MaskedAreas));
    // Painting black masks; white unmasks. The layer is untouched.
    let before = px(&s, 5, 5);
    s.execute("paint.stroke", json!({"points": [[5, 5]], "size": 4, "hardness": 1.0, "color": "#000000"})).unwrap();
    s.execute("paint.stroke", json!({"points": [[30, 10]], "size": 4, "hardness": 1.0, "color": "#ffffff"})).unwrap();
    assert_eq!(px(&s, 5, 5), before);
    s.execute("select.editInQuickMaskMode", json!({})).unwrap();
    assert!(doc(&s).quick_mask.is_none());
    assert_eq!(sel_at(&s, 5, 5), 0.0, "painted black: masked");
    assert_eq!(sel_at(&s, 12, 12), 1.0);
    assert_eq!(sel_at(&s, 30, 10), 1.0, "painted white: selected");
    assert_eq!(sel_at(&s, 35, 2), 0.0);
    // History: enter and exit are steps; undo goes back into Quick Mask with the paint.
    let labels: Vec<String> =
        s.active().unwrap().history.entries(&crate::read_control_none()).unwrap().values().to_vec().iter().map(|e| e.to_string()).collect();
    assert!(labels.iter().any(|l| l.contains("Edit in Quick Mask")) && labels.iter().any(|l| l.contains("Exit Quick Mask")), "{labels:?}");
    s.undo();
    assert!(doc(&s).quick_mask.is_some());
    assert!(doc(&s).selection.is_none());
    s.redo();
    assert!(doc(&s).quick_mask.is_none());
    assert_eq!(sel_at(&s, 30, 10), 1.0);
}

#[test]
fn quick_mask_without_selection_and_options() {
    let mut s = session();
    s.execute("channel.options", json!({"channel": "quickMask", "color": "#0000ff", "opacity": 30})).unwrap();
    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    let q = doc(&s).quick_mask.clone().unwrap();
    assert_eq!(q.color, Color::rgb(0.0, 0.0, 1.0));
    assert!((q.opacity - 0.3).abs() < 1e-6);
    // Leaving an untouched Quick Mask leaves no selection.
    s.execute("select.editInQuickMaskMode", json!({"on": false})).unwrap();
    assert!(doc(&s).selection.is_none());
    // A filter in Quick Mask edits the mask.
    s.execute("select.editInQuickMaskMode", json!({})).unwrap();
    s.execute("paint.stroke", json!({"points": [[10, 10]], "size": 12, "hardness": 1.0, "color": "#000000"})).unwrap();
    s.execute("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    let v = doc(&s).quick_mask.as_ref().unwrap().surface.sample_channel(16, 10, 0);
    assert!(v > 0.0 && v < 1.0, "feathered edge {v}");
    s.execute("select.editInQuickMaskMode", json!({})).unwrap();
    assert!(sel_at(&s, 10, 10) < 0.05 && sel_at(&s, 30, 10) > 0.99, "{} {}", sel_at(&s, 10, 10), sel_at(&s, 30, 10));
}

#[test]
fn channel_options_and_spot_channels() {
    let mut s = session();
    rect(&mut s, 0, 0, 10, 10, "replace");
    s.execute("channel.new", json!({"fill": "selection"})).unwrap();
    s.execute("channel.options", json!({"channel": 0, "name": "Opt", "color": "#00ff00", "opacity": 25, "indicates": "selected"})).unwrap();
    let c = &doc(&s).channels[0];
    assert_eq!((c.name.as_str(), c.color, c.indicates), ("Opt", Color::rgb(0.0, 1.0, 0.0), ColorIndicates::SelectedAreas));
    assert!((c.opacity - 0.25).abs() < 1e-6);
    assert_eq!(c.overlay_coverage(1.0), 1.0);
    s.execute("channel.options", json!({"channel": 0, "indicates": "spot", "solidity": 40})).unwrap();
    assert_eq!(doc(&s).channels[0].spot.map(|x| x.1), Some(0.4));
    let r = s.execute("channel.newSpot", json!({"color": "#ff00ff", "solidity": 100})).unwrap();
    let i = r["channel"].as_u64().unwrap() as usize;
    assert_eq!(doc(&s).channels[i].name, "Spot Color 1");
    assert_eq!(chan(&s, i, 5, 5), 1.0, "from the selection");
    // Merge spot: flattens and prints the ink (solid magenta) into the image.
    s.execute("channel.mergeSpot", json!({"channel": i})).unwrap();
    let d = doc(&s);
    assert_eq!(d.layers.len(), 1);
    assert_eq!(d.channels.len(), 1);
    let p = px(&s, 5, 5);
    assert!((p[0] - 1.0).abs() < 0.01 && p[1] < 0.01 && (p[2] - 1.0).abs() < 0.01, "{p:?}");
    assert_eq!(px(&s, 20, 15)[..3], [1.0, 1.0, 1.0]);
}

#[test]
fn apply_image_hand_computed() {
    let mut s = session();
    // Target: 50% gray. Source: a second document, red #ff0000 / 50% gray.
    s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
    s.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff4000"})).unwrap();
    s.set_active(0);
    let g = 128.0 / 255.0;
    s.execute("image.applyImage", json!({"source": {"document": 1}, "blending": "multiply"})).unwrap();
    let p = px(&s, 3, 3);
    assert!((p[0] - g).abs() < 0.003 && (p[1] - g * 64.0 / 255.0).abs() < 0.003 && p[2].abs() < 0.003, "{p:?}");
    s.undo();
    // Single source channel applied to every target channel, screen at 50% opacity, inverted.
    s.execute("image.applyImage", json!({"source": {"document": 1, "channel": "green", "invert": true}, "blending": "screen", "opacity": 50})).unwrap();
    let top = 1.0 - 64.0 / 255.0;
    let screen = 1.0 - (1.0 - g) * (1.0 - top);
    let want = g + (screen - g) * 0.5;
    let p = px(&s, 3, 3);
    assert!(p[..3].iter().all(|v| (v - want).abs() < 0.003), "{p:?} vs {want}");
    s.undo();
    // Add with scale and offset; masked by an alpha channel; limited to the selection.
    s.execute("channel.new", json!({"fill": "white"})).unwrap();
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    rect(&mut s, 0, 0, 20, 20, "replace");
    s.execute("image.applyImage", json!({"source": {"document": 1, "channel": "red"}, "blending": "add", "scale": 2, "offset": -10, "mask": {"channel": 0}}))
        .unwrap();
    let want = ((g + 1.0) / 2.0 - 10.0 / 255.0).clamp(0.0, 1.0);
    assert!((px(&s, 3, 3)[1] - want).abs() < 0.003);
    assert!((px(&s, 30, 3)[1] - g).abs() < 0.003, "outside the selection");
    // Size mismatch is refused.
    s.execute("file.new", json!({"width": 5, "height": 5})).unwrap();
    s.set_active(0);
    assert!(s.execute("image.applyImage", json!({"source": {"document": 2}})).is_err());
    assert!(s.execute("image.applyImage", json!({"blending": "hue"})).is_err());
}

#[test]
fn apply_image_onto_transparent_layer() {
    for depth in [8, 16, 32] {
        let mut s = session_depth(depth);
        s.execute("edit.fill", json!({"color": "#c08040"})).unwrap();
        let (r, g, b) = (192.0 / 255.0, 128.0 / 255.0, 64.0 / 255.0);
        // An empty layer has no pixels to blend with: it takes the source, whatever the mode.
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("image.applyImage", json!({"blending": "multiply"})).unwrap();
        let p = px(&s, 3, 3);
        assert!((p[0] - r).abs() < 0.003 && (p[1] - g).abs() < 0.003 && (p[2] - b).abs() < 0.003 && p[3] > 0.999, "depth {depth}: {p:?}");
        s.undo();
        // At 50% opacity the empty layer becomes the source at 50% alpha.
        s.execute("image.applyImage", json!({"blending": "multiply", "opacity": 50})).unwrap();
        let p = px(&s, 3, 3);
        assert!((p[0] - r).abs() < 0.003 && (p[3] - 0.5).abs() < 0.003, "depth {depth}: {p:?}");
        s.undo();
        // Preserve Transparency leaves an empty layer empty.
        s.execute("image.applyImage", json!({"blending": "multiply", "preserveTransparency": true})).unwrap();
        assert!(px(&s, 3, 3)[3] < 0.001, "depth {depth}");
    }
}

#[test]
fn apply_image_into_alpha_channel() {
    let mut s = session();
    s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    s.execute("channel.new", json!({"fill": "white"})).unwrap();
    // Targeted channel: Normal blend of the inverted composite → black.
    s.execute("image.applyImage", json!({"source": {"channel": "composite", "invert": true}, "blending": "normal"})).unwrap();
    assert!(chan(&s, 0, 3, 3) < 0.01);
}

#[test]
fn calculations_results() {
    let mut s = session();
    rect(&mut s, 0, 0, 20, 20, "replace");
    s.execute("select.saveSelection", json!({})).unwrap();
    rect(&mut s, 10, 0, 20, 20, "replace");
    s.execute("select.saveSelection", json!({})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    // Multiply = intersection of the two masks.
    let r = s.execute("image.calculations", json!({"source1": {"channel": 0}, "source2": {"channel": 1}, "blending": "multiply", "name": "X"})).unwrap();
    let i = r["channel"].as_u64().unwrap() as usize;
    assert_eq!(doc(&s).channels[i].name, "X");
    assert_eq!((chan(&s, i, 5, 5), chan(&s, i, 15, 5), chan(&s, i, 25, 5)), (0.0, 1.0, 0.0));
    // Screen = union, at 50% opacity; source 1 inverted.
    s.execute(
        "image.calculations",
        json!({"source1": {"channel": 0, "invert": true}, "source2": {"channel": 1}, "blending": "difference", "result": "selection"}),
    )
    .unwrap();
    // |s2 - (1 - s1)|: x=5: |0-0|=0, x=15: |1-0|=1, x=35: |0-1|=1
    assert_eq!((sel_at(&s, 5, 5), sel_at(&s, 15, 5), sel_at(&s, 35, 5)), (0.0, 1.0, 1.0));
    let r = s
        .execute(
            "image.calculations",
            json!({"source1": {"channel": "composite"}, "source2": {"channel": 1}, "blending": "normal", "opacity": 50, "result": "newDocument"}),
        )
        .unwrap();
    assert_eq!(r["document"], 1);
    let d = doc(&s);
    assert_eq!(d.mode, ColorMode::Grayscale);
    let v = d.layers[0].surface().unwrap().pixel(5, 5)[0];
    assert!((v - 0.5).abs() < 0.01, "{v}");
    s.set_active(0);
    assert!(s.execute("image.calculations", json!({"source1": {"channel": 0}, "source2": {"channel": 1}, "result": "x"})).is_err());
}

#[test]
fn split_and_merge_channels() {
    let mut s = session();
    s.execute("edit.fill", json!({"color": "#ff8000"})).unwrap();
    s.execute("channel.new", json!({"fill": "white", "name": "A"})).unwrap();
    let r = s.execute("channel.split", json!({})).unwrap();
    assert_eq!(r["documents"].as_array().unwrap().len(), 4);
    assert_eq!(s.documents().len(), 4, "original closed");
    let names: Vec<_> = s.documents().iter().map(|d| d.doc.name.clone()).collect();
    assert_eq!(names, ["Untitled_R", "Untitled_G", "Untitled_B", "Untitled_A"]);
    let v = |s: &Session, i: usize| s.documents()[i].doc.layers[0].surface().unwrap().pixel(1, 1)[0];
    assert_eq!(v(&s, 0), 1.0);
    assert!((v(&s, 1) - 128.0 / 255.0).abs() < 0.003);
    assert_eq!(v(&s, 2), 0.0);
    // Merge the three colour docs back (order: active first, then the rest).
    s.set_active(0);
    let r = s.execute("channel.merge", json!({"mode": "rgb", "documents": [0, 1, 2]})).unwrap();
    assert_eq!(s.documents().len(), 2);
    assert_eq!(r["document"], 1);
    let p = px(&s, 1, 1);
    assert!((p[0] - 1.0).abs() < 0.003 && (p[1] - 128.0 / 255.0).abs() < 0.003 && p[2].abs() < 0.003, "{p:?}");
    assert!(s.execute("channel.merge", json!({"mode": "cmyk"})).is_err());
}

#[test]
fn merge_rejects_a_repeated_source_and_keeps_unrelated_documents() {
    // #934: `documents: [0, 0, 1]` passed validation, then the close loop ran `close(1)`,
    // `close(0)`, `close(0)` and the last call removed the unrelated RGB document at index 2.
    let mut s = session();
    s.execute("channel.split", json!({})).unwrap();
    assert_eq!(s.documents().len(), 3);
    s.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    let keep = s.documents()[3].doc.id;
    let before: Vec<_> = s.documents().iter().map(|d| d.doc.id).collect();
    let err = s.execute("channel.merge", json!({"mode": "rgb", "documents": [0, 0, 1]})).unwrap_err();
    assert!(err.to_string().contains("more than once"), "{err}");
    let after: Vec<_> = s.documents().iter().map(|d| d.doc.id).collect();
    assert_eq!(after, before, "a rejected merge closes nothing");
    assert!(s.documents().iter().any(|d| d.doc.id == keep));
    assert!(s.set_active(3) && s.undo(), "the unrelated document keeps its history");
    // Distinct indices still merge and close exactly the named sources.
    s.execute("channel.merge", json!({"mode": "rgb", "documents": [0, 1, 2]})).unwrap();
    assert_eq!(s.documents().len(), 2);
    assert!(s.documents().iter().any(|d| d.doc.id == keep));
}

#[test]
fn duplicate_to_other_document() {
    let mut s = session();
    rect(&mut s, 0, 0, 5, 5, "replace");
    s.execute("select.saveSelection", json!({"name": "S"})).unwrap();
    s.execute("file.new", json!({"width": 40, "height": 20, "depth": 16})).unwrap();
    s.set_active(0);
    s.execute("channel.duplicate", json!({"channel": "S", "document": 1, "name": "S2"})).unwrap();
    let d1 = &s.documents()[1].doc;
    assert_eq!(d1.channels[0].name, "S2");
    assert_eq!(d1.channels[0].surface.format().sample, SampleType::U16);
    assert_eq!(d1.channels[0].surface.sample_channel(2, 2, 0), 1.0);
    let r = s.execute("channel.duplicate", json!({"channel": 0, "document": "new"})).unwrap();
    assert_eq!(s.documents()[r["document"].as_u64().unwrap() as usize].doc.mode, ColorMode::Grayscale);
}

#[test]
fn image_size_and_rotate_carry_channels_and_quick_mask() {
    let mut s = session();
    rect(&mut s, 0, 0, 10, 20, "replace");
    s.execute("select.saveSelection", json!({})).unwrap();
    s.execute("select.editInQuickMaskMode", json!({})).unwrap();
    s.execute("image.imageRotation.180", json!({})).unwrap();
    s.execute("image.mode.bits16", json!({})).unwrap();
    let d = doc(&s);
    assert_eq!(d.channels[0].surface.format().sample, SampleType::U16);
    assert_eq!(d.quick_mask.as_ref().unwrap().surface.format().sample, SampleType::U16);
}

#[test]
fn apply_image_and_calculations_fail_gracefully_never_panic() {
    // Rule 9: adversarial params must return Err, not panic (guards the empty-source `src[0]` /
    // `.remove(0)` paths in apply_image/calculations).
    let mut s = session();
    for p in [
        json!({}),
        json!({"source": {"channel": 999}}),
        json!({"source": {"channel": "nope"}}),
        json!({"source": {"document": 999}}),
        json!({"blending": "bogus", "opacity": -50}),
    ] {
        // Must return a Result (Ok or Err) without panicking.
        let _ = s.execute("image.applyImage", p.clone());
        let _ = s.execute("image.calculations", p);
    }
    // A well-formed self-apply still works.
    assert!(s.execute("image.applyImage", json!({"source": {"channel": "composite"}})).is_ok());
}

#[test]
fn stable_channel_ids_survive_index_changes_duplication_native_and_undo() {
    let mut s = session();
    s.edit("Own channels", |doc, _| {
        doc.channels.push(AlphaChannel::new("duplicate name", Surface::new(PixelFormat::GRAY8)));
        doc.channels.push(AlphaChannel::new("duplicate name", Surface::new(PixelFormat::GRAY8)));
        doc.xcf_state = Some(photocraft_doc::XcfState::default());
        Ok(())
    })
    .unwrap();
    let first = doc(&s).channels[0].id;
    let second = doc(&s).channels[1].id;
    assert_eq!(parse_ref(&json!({"channelId":second.0}), doc(&s)), Some(ChanRef::Alpha(1)));
    s.execute("channel.delete", json!({"channelId":first.0})).unwrap();
    assert_eq!(parse_ref(&json!({"channelId":second.0}), doc(&s)), Some(ChanRef::Alpha(0)));
    s.execute("channel.rename", json!({"channelId":second.0,"name":"Own renamed"})).unwrap();
    assert_eq!(doc(&s).channels[0].name, "Own renamed");
    s.execute("channel.duplicate", json!({"channel":{"channelId":second.0}})).unwrap();
    assert_ne!(doc(&s).channels[1].id, second);
    let back = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(doc(&s), &Default::default()).unwrap()).unwrap();
    assert_eq!(back, *doc(&s));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc(&s).channels.len(), 1);
    assert_eq!(doc(&s).channels[0].id, second);
    let before = doc(&s).clone();
    assert!(s.execute("channel.delete", json!({"channelId":u64::MAX})).is_err());
    assert_eq!(doc(&s), &before);
}

#[test]
fn duplicated_imported_channel_keeps_ordered_filter_metadata_and_fresh_ids() {
    use photocraft_doc::item_identity::{FilterId, ItemRef};
    use photocraft_doc::xcf_filters::{FilterNode, FilterStack, OperationIdentity};
    let mut s = session();
    s.edit("Own channel", |doc, _| {
        doc.channels.push(AlphaChannel::new("Own", Surface::new(PixelFormat::GRAY8)));
        let id = doc.channels[0].id;
        let mut state = photocraft_doc::XcfState::default();
        state.reconcile_layers(doc).map_err(|e| bad("own", e.to_string()))?;
        state.filter_stacks.push(FilterStack {
            owner: ItemRef::Channel(id),
            revision: 2,
            nodes: vec![FilterNode {
                id: FilterId(1),
                operation: OperationIdentity { name: "own:filter".into(), stored_version: "1".into(), schema_epoch: "own".into() },
                arguments: vec![],
                enabled: false,
                opacity_f64_bits: 1.0f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                source_display_name: Some("Own".into()),
                source_icon: None,
                mask: Some(ItemRef::Mask(photocraft_doc::item_identity::MaskId(9))),
                auxiliary: vec![("self".into(), ItemRef::Channel(id))],
                composition: None,
            }],
        });
        let bounds = Rect::new(0, 0, 1, 1);
        let mut mask_item = state.items.iter().find(|item| item.item == ItemRef::Channel(id)).unwrap().clone();
        mask_item.item = ItemRef::Mask(photocraft_doc::item_identity::MaskId(9));
        mask_item.raster_bounds = Some(bounds);
        mask_item.mask = Some(photocraft_doc::xcf_state::MaskState {
            owner: ItemRef::Channel(id),
            bounds,
            offset: [0, 0],
            outside_f64_bits: 0,
            enabled: true,
            shown: false,
            editing: false,
            linked: true,
        });
        state.items.push(mask_item);
        state.filter_masks.push(
            photocraft_doc::FilterMask::try_new(photocraft_doc::item_identity::MaskId(9), FilterId(1), Surface::new(PixelFormat::GRAY8), bounds).unwrap(),
        );
        doc.xcf_state = Some(state);
        Ok(())
    })
    .unwrap();
    let original = doc(&s).channels[0].id;
    s.execute("channel.duplicate", json!({"channelId":original.0})).unwrap();
    let target = doc(&s).channels[1].id;
    let state = doc(&s).xcf_state.as_ref().unwrap();
    let copy = state.filter_stacks.iter().find(|stack| stack.owner == ItemRef::Channel(target)).unwrap();
    assert_ne!(copy.nodes[0].id, FilterId(1));
    assert_eq!(copy.nodes[0].auxiliary[0].1, ItemRef::Channel(target));
    assert_eq!(copy.nodes[0].source_clipping, Some(0));
    let Some(ItemRef::Mask(mask_id)) = copy.nodes[0].mask else { panic!("fresh filter mask") };
    assert_ne!(mask_id, photocraft_doc::item_identity::MaskId(9));
    let mask = state.filter_masks.iter().find(|mask| mask.id == mask_id).unwrap();
    assert_eq!(mask.owner, copy.nodes[0].id);
    assert_eq!(mask.surface, state.filter_masks[0].surface);
    assert_eq!(state.items.iter().find(|item| item.item == ItemRef::Mask(mask_id)).unwrap().mask.as_ref().unwrap().owner, ItemRef::Channel(target));
    let back = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(doc(&s), &Default::default()).unwrap()).unwrap();
    assert_eq!(back, *doc(&s));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc(&s).xcf_state.as_ref().unwrap().filter_stacks.len(), 1);
}

#[test]
fn imported_channel_view_uses_ids_visibility_and_single_active_target() {
    use photocraft_doc::item_identity::ItemRef;
    let mut d = Document::new("Own", photocraft_doc::Size::new(1, 1), ColorMode::Rgb, photocraft_doc::SampleType::U8);
    d.channels.push(AlphaChannel::new("Same", Surface::new(PixelFormat::GRAY8)));
    d.channels.push(AlphaChannel::new("Same", Surface::new(PixelFormat::GRAY8)));
    let first = d.channels[0].id;
    let second = d.channels[1].id;
    let mut state = photocraft_doc::XcfState::default();
    state.reconcile_layers(&d).unwrap();
    state.items.iter_mut().find(|item| item.item == ItemRef::Channel(first)).unwrap().visible = false;
    state.items.iter_mut().find(|item| item.item == ItemRef::Channel(second)).unwrap().visible = true;
    state.selection.active = Some(ItemRef::Channel(second));
    state.selection.selected = vec![ItemRef::Channel(second)];
    d.xcf_state = Some(state);
    let view = imported_view(&d);
    assert_eq!(view.target, ChannelTarget::Alpha(1));
    assert_eq!(view.alpha_visible, vec![false, true]);
    d.channels.swap(0, 1);
    assert_eq!(imported_view(&d).target, ChannelTarget::Alpha(0));
    d.xcf_state.as_mut().unwrap().selection.selected.push(ItemRef::Channel(first));
    assert_eq!(imported_view(&d).target, ChannelTarget::Composite);
    let selection = photocraft_doc::item_identity::ChannelId::fresh();
    d.selection_channel_id = Some(selection);
    d.xcf_state.as_mut().unwrap().selection.active = Some(ItemRef::Channel(selection));
    assert_eq!(imported_view(&d).target, ChannelTarget::Composite);
}

fn hidden_mask_document() -> Document {
    use photocraft_doc::item_identity::{ItemRef, MaskId};
    let mut document = Document::with_background("Own mask", photocraft_doc::Size::new(2, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let owner = document.layers[0].id;
    document.layers[0].locks = Default::default();
    document.layers[0].mask = Some(photocraft_doc::LayerMask::reveal_all());
    document.layers.push(Layer::new("Other", LayerContent::Raster(Surface::new(PixelFormat::RGBA8))));
    let mut selection = Surface::new(PixelFormat::GRAY8);
    selection.write_pixel(0, 0, &[1.0]);
    document.selection = Some(selection);
    let mut state = photocraft_doc::XcfState::default();
    state.reconcile_layers(&document).unwrap();
    let mut metadata = state.items.iter().find(|item| item.item == ItemRef::Layer(owner)).unwrap().clone();
    metadata.item = ItemRef::Mask(MaskId(71));
    metadata.mask = Some(photocraft_doc::xcf_state::MaskState {
        owner: ItemRef::Layer(owner),
        bounds: Rect::new(0, 0, 2, 1),
        offset: [0, 0],
        outside_f64_bits: 1.0_f64.to_bits(),
        enabled: true,
        linked: true,
        editing: true,
        shown: false,
    });
    state.items.push(metadata);
    state.selection.mask_target = Some(ItemRef::Mask(MaskId(71)));
    document.xcf_state = Some(state);
    document
}

#[test]
fn imported_hidden_mask_native_reopen_routes_edits_undo_and_selection_without_showing() {
    let source = hidden_mask_document();
    let owner = source.layers[0].id;
    let bytes = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
    let mut session = Session::new();
    session.add_document(reopened, None).unwrap();
    assert_eq!(session.active().unwrap().active_layer, Some(owner));
    assert_eq!(mask_editing_target(session.active().unwrap()), Some(owner));
    assert!(session.active().unwrap().channel_view.layer_mask.is_none());
    let before = doc(&session).clone();
    session.execute("image.adjustments.invert", json!({})).unwrap();
    let edited = doc(&session);
    assert_eq!(edited.layers[0].surface(), before.layers[0].surface());
    assert_eq!(edited.selection, before.selection);
    assert_eq!(edited.layers[0].mask.as_ref().unwrap().surface.sample_channel(0, 0, 0), 0.0);
    assert_eq!(edited.layers[0].mask.as_ref().unwrap().surface.sample_channel(1, 0, 0), 1.0);
    assert!(session.active().unwrap().channel_view.layer_mask.is_none());
    assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(edited, &Default::default()).unwrap()).unwrap(), *edited);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc(&session), &before);
    assert_eq!(mask_editing_target(session.active().unwrap()), Some(owner));
    session.execute("channel.target.composite", json!({})).unwrap();
    assert_eq!(mask_editing_target(session.active().unwrap()), None);
}

#[test]
fn imported_mask_flag_conflicts_and_deleted_masks_do_not_guess_a_target() {
    use photocraft_doc::item_identity::{ItemRef, MaskId};
    let mut document = hidden_mask_document();
    let mut state = document.xcf_state.take().unwrap();
    state.selection.mask_target = None;
    let mut second = state.items.iter().find(|item| item.item == ItemRef::Mask(MaskId(71))).unwrap().clone();
    second.item = ItemRef::Mask(MaskId(72));
    state.items.push(second);
    document.xcf_state = Some(state);
    assert_eq!(imported_editing_mask(&document), None);
    document.xcf_state.as_mut().unwrap().selection.mask_target = Some(ItemRef::Mask(MaskId(71)));
    assert_eq!(imported_editing_mask(&document), Some(document.layers[0].id));
    document.layers[0].mask = None;
    assert_eq!(imported_editing_mask(&document), None);
}
