use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use photocraft_genai::{Capabilities, EditRequest, GenError, GeneratedImage, ProviderInfo};

use super::*;

/// Answers every edit with a solid colour at the request's size (or `size` when set), and keeps
/// what it was sent. With `vary` each call's colour differs; with `flaky` every second call fails.
struct Fake {
    color: [u8; 3],
    vary: bool,
    flaky: bool,
    /// How long each call takes.
    delay: std::time::Duration,
    /// Answer with the image sent, every channel moved by this much (a model's colour drift).
    echo: Option<i16>,
    calls: AtomicUsize,
    size: Option<(u32, u32)>,
    fail: Option<GenError>,
    aspect: Option<(u32, u32)>,
    jpeg: bool,
    sent: Mutex<Vec<EditRequest>>,
}

impl Fake {
    fn new(color: [u8; 3]) -> Arc<Fake> {
        Arc::new(Fake {
            color,
            vary: false,
            flaky: false,
            delay: std::time::Duration::ZERO,
            echo: None,
            calls: AtomicUsize::new(0),
            size: None,
            fail: None,
            aspect: None,
            jpeg: false,
            sent: Mutex::new(Vec::new()),
        })
    }
}

impl ImageProvider for Fake {
    fn info(&self) -> ProviderInfo {
        ProviderInfo { id: "fake", name: "Fake", model: "fake-1".into(), host: "example.invalid" }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { generate: false, edit: true, variations: false }
    }
    fn edit_aspect(&self, w: u32, h: u32) -> (u32, u32) {
        self.aspect.unwrap_or((w, h))
    }
    fn edit(&self, r: &EditRequest) -> photocraft_genai::Result<GeneratedImage> {
        self.sent.lock().unwrap().push(r.clone());
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        if self.flaky && n % 2 == 1 {
            return Err(GenError::Quota);
        }
        if let Some(d) = self.echo {
            let img = codecs::decode(&r.image).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
            let px: Vec<u8> = img.data().iter().enumerate().map(|(i, &v)| if i % 4 == 3 { v } else { (i16::from(v) + d).clamp(0, 255) as u8 }).collect();
            let out = Image::from_u8(img.width(), img.height(), ChannelLayout::Rgba, px).unwrap();
            return Ok(GeneratedImage { bytes: codecs::encode(&out, Format::Png, &Default::default()).unwrap(), mime: "image/png".into() });
        }
        let color = if self.vary { [self.color[0].wrapping_add(n as u8 * 60), self.color[1], self.color[2]] } else { self.color };
        let (w, h) = self.size.or_else(|| photocraft_genai::png_size(&r.image)).unwrap();
        let px: Vec<u8> = (0..w * h).flat_map(|_| [color[0], color[1], color[2], 255]).collect();
        let img = Image::from_u8(w, h, ChannelLayout::Rgba, px).unwrap();
        if self.jpeg {
            let rgb = img.convert(ChannelLayout::Rgb, CodecSample::U8);
            return Ok(GeneratedImage { bytes: codecs::encode(&rgb, Format::Jpeg, &Default::default()).unwrap(), mime: "image/jpeg".into() });
        }
        Ok(GeneratedImage { bytes: codecs::encode(&img, Format::Png, &Default::default()).unwrap(), mime: "image/png".into() })
    }
}

fn session(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "#ffffff"})).unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn rgb_at(s: &Session, x: i32, y: i32) -> [u8; 3] {
    let d = doc(s);
    let p = photocraft_compose::flatten(d).get(x, y);
    [0, 1, 2].map(|c| (p[c] * 255.0).round() as u8)
}

#[test]
fn needs_a_provider_and_a_selection() {
    let mut s = session(64, 64);
    assert!(!s.is_enabled(FILL), "no provider: off");
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    assert!(!s.is_enabled(FILL), "still no provider");
    assert!(s.execute(FILL, json!({"prompt": "a cat"})).is_err());
    let fake = Fake::new([255, 0, 0]);
    s.set_image_provider(Some(fake.clone()));
    assert!(s.is_enabled(FILL));
    s.execute("select.deselect", json!({})).unwrap();
    assert!(!s.is_enabled(FILL), "no selection: off");
    assert!(fake.sent.lock().unwrap().is_empty());
}

#[test]
fn the_results_land_in_a_new_group_masked_by_the_selection_and_undo_in_one_step() {
    let mut s = session(200, 100);
    s.execute("select.rect", json!({"x": 80, "y": 30, "width": 40, "height": 40})).unwrap();
    let fake = Fake::new([255, 0, 0]);
    s.set_image_provider(Some(fake.clone()));
    let before = doc(&s).layers.len();
    let history = s.active().unwrap().history.past_len();
    let r = s.execute(FILL, json!({"prompt": "  a red   balloon "})).unwrap();

    let d = doc(&s);
    assert_eq!(d.layers.len(), before + 1);
    let layer = d.layer(photocraft_doc::LayerId(r["layer"].as_u64().unwrap())).unwrap();
    assert_eq!(layer.name, "a red balloon");
    assert!(layer.is_group());
    assert!(layer.mask.is_some(), "masked by the selection, like Photoshop's generative layer");
    assert_eq!(s.active().unwrap().active_layer, Some(layer.id));
    assert_eq!(rgb_at(&s, 100, 50), [255, 0, 0], "inside the selection: the result");
    assert_eq!(rgb_at(&s, 70, 50), [255, 255, 255], "outside it: untouched, though the result covers it");
    assert_eq!(rgb_at(&s, 5, 5), [255, 255, 255]);
    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
    assert!(doc(&s).selection.is_some(), "the selection stays");

    // What was sent: the prompt, the image and a mask that is white only over the selection.
    let sent = fake.sent.lock().unwrap();
    assert_eq!(sent.len(), 3, "three variations by default");
    assert_eq!(sent[0].prompt, "a red   balloon");
    let mask = codecs::decode(&sent[0].mask).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
    let area = Rect::from_xywh(r["bounds"][0].as_i64().unwrap() as i32, r["bounds"][1].as_i64().unwrap() as i32, 1, 1);
    let at = |x: i32, y: i32| mask.data()[((y - area.y0) as usize * mask.width() as usize + (x - area.x0) as usize) * 4];
    assert_eq!(at(100, 50), 255);
    assert_eq!(at(78, 50), 0);
    drop(sent);

    assert!(s.undo());
    assert_eq!(doc(&s).layers.len(), before);
    assert_eq!(rgb_at(&s, 100, 50), [255, 255, 255]);
}

#[test]
fn an_empty_prompt_is_allowed_and_names_the_layer_generative_fill() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    s.set_image_provider(Some(Fake::new([0, 0, 255])));
    let r = s.execute(FILL, json!({})).unwrap();
    let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(doc(&s).layer(id).unwrap().name, "Generative Fill");
    let long = "word ".repeat(30);
    s.execute(FILL, json!({"prompt": long})).unwrap();
    assert!(doc(&s).layers.last().unwrap().name.ends_with('…'));
}

#[test]
fn a_result_of_another_size_is_scaled_to_the_area() {
    let mut s = session(300, 300);
    s.execute("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    let fake = Arc::new(Fake { size: Some((1024, 1024)), ..Arc::into_inner(Fake::new([0, 255, 0])).unwrap() });
    s.set_image_provider(Some(fake));
    s.execute(FILL, json!({"prompt": "grass"})).unwrap();
    assert_eq!(rgb_at(&s, 150, 150), [0, 255, 0]);
    // Just inside the corner, clear of the softened edge (a 2 px band on a 100 px selection).
    assert_eq!(rgb_at(&s, 104, 104), [0, 255, 0]);
}

#[test]
fn a_jpeg_result_is_read_too() {
    let mut s = session(120, 120);
    s.execute("select.rect", json!({"x": 30, "y": 30, "width": 60, "height": 60})).unwrap();
    let fake = Arc::new(Fake { jpeg: true, ..Arc::into_inner(Fake::new([0, 0, 255])).unwrap() });
    s.set_image_provider(Some(fake));
    s.execute(FILL, json!({"prompt": "blue"})).unwrap();
    let [r, g, b] = rgb_at(&s, 60, 60);
    assert!(r < 10 && g < 10 && b > 245, "{r} {g} {b}");
    assert_eq!(rgb_at(&s, 10, 10), [255, 255, 255]);
}

#[test]
fn failures_leave_no_history_and_say_why() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let history = s.active().unwrap().history.past_len();
    let fake = Arc::new(Fake { fail: Some(GenError::Auth), ..Arc::into_inner(Fake::new([0, 0, 0])).unwrap() });
    s.set_image_provider(Some(fake));
    let e = s.execute(FILL, json!({"prompt": "x"})).unwrap_err().to_string();
    assert!(e.contains("API key"), "{e}");
    assert_eq!(s.active().unwrap().history.past_len(), history);
    assert!(s.execute(FILL, json!({"prompt": 5})).is_err());
    assert!(s.execute(FILL, json!({"prompt": "x".repeat(photocraft_genai::MAX_PROMPT + 1)})).is_err());
}

#[test]
fn the_document_keeps_its_mode_and_depth() {
    for (mode, depth) in [("rgb", 16), ("grayscale", 8), ("cmyk", 8), ("rgb", 32)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64, "mode": mode, "depth": depth, "background": "#ffffff"})).unwrap();
        s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
        s.set_image_provider(Some(Fake::new([128, 128, 128])));
        let r = s.execute(FILL, json!({"prompt": "x"})).unwrap();
        let d = doc(&s);
        let l = d.layer(photocraft_doc::LayerId(r["variations"][0].as_u64().unwrap())).unwrap();
        let f = l.surface().unwrap().format();
        assert_eq!((f.mode, f.sample), (d.mode, d.depth), "{mode} {depth}");
    }
}

#[test]
fn the_context_area_grows_toward_the_providers_shape_inside_the_canvas() {
    let canvas = Rect::from_xywh(0, 0, 1000, 600);
    let same = |w, h| (w, h);
    // A 100² selection gets a 64 px margin on every side.
    assert_eq!(context_area(Rect::from_xywh(400, 200, 100, 100), canvas, same), Rect::new(336, 136, 564, 364));
    // At an edge the margin is cut off by the canvas.
    assert_eq!(context_area(Rect::from_xywh(0, 0, 100, 100), canvas, same), Rect::new(0, 0, 164, 164));
    // A provider that only makes 16:9 widens the area around its centre.
    let r = context_area(Rect::from_xywh(400, 200, 100, 100), canvas, |_, _| (16, 9));
    assert_eq!(r.height(), 228);
    assert!((f64::from(r.width()) / f64::from(r.height()) - 16.0 / 9.0).abs() < 0.01, "{r:?}");
    assert_eq!(r.x0 + r.x1, 336 + 564, "centred");
    // Against the canvas edge it shifts inward instead of leaving the canvas.
    let r = context_area(Rect::from_xywh(0, 0, 100, 100), canvas, |_, _| (16, 9));
    assert_eq!(r.x0, 0);
    assert!(r.x1 <= 1000);
    // It can't grow past the canvas: the remaining mismatch is scaled away.
    let r = context_area(Rect::from_xywh(0, 0, 1000, 600), canvas, |_, _| (21, 9));
    assert_eq!(r, canvas);
}

fn group(s: &Session, id: u64) -> &photocraft_doc::Layer {
    doc(s).layer(photocraft_doc::LayerId(id)).unwrap()
}

fn shown(s: &Session, id: u64) -> Vec<bool> {
    group(s, id).children().unwrap().iter().map(|l| l.visible).collect()
}

/// The colour variation `i` of group `id` holds at (`x`, `y`).
fn variation_rgb(s: &Session, id: u64, i: usize, x: i32, y: i32) -> [u8; 3] {
    let l = &group(s, id).children().unwrap()[i];
    let surf = l.surface().unwrap();
    let mut px = [0.0f32; 4];
    surf.read_pixel(x, y, &mut px[..surf.channels()]);
    let p = photocraft_raster::to_rgba(&surf.format(), &px[..surf.channels()]);
    [0, 1, 2].map(|c| (p[c] * 255.0).round() as u8)
}

fn filled(variations: Option<u64>) -> (Session, Arc<Fake>, Value) {
    let mut s = session(100, 100);
    s.execute("select.rect", json!({"x": 30, "y": 30, "width": 40, "height": 40})).unwrap();
    let fake = Arc::new(Fake { vary: true, ..Arc::into_inner(Fake::new([0, 0, 255])).unwrap() });
    s.set_image_provider(Some(fake.clone()));
    let r = s.execute(FILL, json!({"prompt": "a boat", "variations": variations})).unwrap();
    (s, fake, r)
}

#[test]
fn a_fill_makes_three_variations_with_the_first_showing() {
    let (s, fake, r) = filled(None);
    let id = r["layer"].as_u64().unwrap();
    assert_eq!(r["variations"].as_array().unwrap().len(), 3);
    let names: Vec<_> = group(&s, id).children().unwrap().iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, ["Variation 1", "Variation 2", "Variation 3"]);
    assert_eq!(shown(&s, id), [true, false, false]);
    assert_eq!(rgb_at(&s, 50, 50), variation_rgb(&s, id, 0, 50, 50));
    assert_eq!(fake.sent.lock().unwrap().len(), 3);
    assert_eq!(s.generative_layer(), Some(photocraft_doc::LayerId(id)));
    assert_eq!(s.generative_prompt(photocraft_doc::LayerId(id)), Some("a boat"));

    let (_, fake, r) = filled(Some(1));
    assert_eq!(r["variations"].as_array().unwrap().len(), 1);
    assert_eq!(fake.sent.lock().unwrap().len(), 1);
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    s.set_image_provider(Some(Fake::new([0, 0, 0])));
    for bad in [json!(0), json!(5), json!("3"), json!(-1)] {
        assert!(s.execute(FILL, json!({"variations": bad})).is_err(), "{bad}");
    }
}

#[test]
fn choosing_a_variation_shows_it_alone_in_one_undo_step() {
    let (mut s, _, r) = filled(None);
    let id = r["layer"].as_u64().unwrap();
    let colors: Vec<_> = (0..3).map(|i| variation_rgb(&s, id, i, 50, 50)).collect();
    assert_ne!(colors[0], colors[1]);
    let history = s.active().unwrap().history.past_len();
    s.execute(SELECT_VARIATION, json!({"index": 2})).unwrap();
    assert_eq!(shown(&s, id), [false, false, true]);
    assert_eq!(rgb_at(&s, 50, 50), colors[2]);
    assert_eq!(rgb_at(&s, 10, 10), [255, 255, 255], "still masked by the selection");
    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
    // With a variation itself active, the command still finds its group.
    let child = group(&s, id).children().unwrap()[0].id;
    s.execute("layer.select", json!({"layer": child.0})).unwrap();
    s.execute(SELECT_VARIATION, json!({"index": 1})).unwrap();
    assert_eq!(shown(&s, id), [false, true, false]);
    assert!(s.execute(SELECT_VARIATION, json!({"index": 3})).is_err());
    assert!(s.execute(SELECT_VARIATION, json!({})).is_err());
    assert!(s.undo());
    assert_eq!(shown(&s, id), [false, false, true]);
}

#[test]
fn variation_commands_need_a_generative_layer() {
    let mut s = session(64, 64);
    s.set_image_provider(Some(Fake::new([0, 0, 0])));
    assert!(!s.is_enabled(SELECT_VARIATION));
    assert!(!s.is_enabled(MORE));
    // A group that wasn't made by Generative Fill isn't one.
    s.execute("type.create", json!({"text": "Hi", "size": 20, "x": 5, "y": 30})).unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    assert_eq!(s.generative_layer(), None);
    assert!(s.execute(SELECT_VARIATION, json!({"index": 0})).is_err());
    assert!(s.execute(MORE, json!({})).is_err());
}

#[test]
fn generate_more_resends_the_same_request_and_adds_variations() {
    let (mut s, fake, r) = filled(None);
    let id = r["layer"].as_u64().unwrap();
    s.execute(SELECT_VARIATION, json!({"index": 1})).unwrap();
    // A later edit to the canvas doesn't change what is sent: the same image, mask and prompt.
    s.execute("select.deselect", json!({})).unwrap();
    let history = s.active().unwrap().history.past_len();
    let more = s.execute(MORE, json!({"count": 2})).unwrap();
    assert_eq!(more["variations"].as_array().unwrap().len(), 2);
    let names: Vec<_> = group(&s, id).children().unwrap().iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, ["Variation 1", "Variation 2", "Variation 3", "Variation 4", "Variation 5"]);
    assert_eq!(shown(&s, id), [false, false, false, true, false], "the first new one shows");
    let sent = fake.sent.lock().unwrap();
    assert_eq!(sent.len(), 5);
    assert_eq!(sent[4].image, sent[0].image);
    assert_eq!(sent[4].mask, sent[0].mask);
    assert_eq!(sent[4].prompt, "a boat");
    drop(sent);
    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
    assert!(s.undo());
    assert_eq!(shown(&s, id), [false, true, false]);
    assert!(s.execute(MORE, json!({"count": 9})).is_err());
    // Without a provider there is nothing to ask.
    s.set_image_provider(None);
    assert!(!s.is_enabled(MORE));
    assert!(s.is_enabled(SELECT_VARIATION), "flipping between results needs no provider");
}

#[test]
fn a_failed_variation_is_dropped_when_others_succeed() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let fake = Arc::new(Fake { flaky: true, ..Arc::into_inner(Fake::new([0, 200, 0])).unwrap() });
    s.set_image_provider(Some(fake));
    let r = s.execute(FILL, json!({"prompt": "x", "variations": 3})).unwrap();
    assert_eq!(r["variations"].as_array().unwrap().len(), 2);
    assert_eq!(rgb_at(&s, 16, 16), [0, 200, 0]);
}

#[test]
fn the_selected_pixels_are_never_sent() {
    // A red square under the selection on white: the provider sees the white around it carried
    // in (no red, and no flat grey it might read as an object), and the surroundings as they are.
    let mut s = session(120, 120);
    s.execute("select.rect", json!({"x": 40, "y": 40, "width": 40, "height": 40})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let fake = Fake::new([0, 0, 255]);
    s.set_image_provider(Some(fake.clone()));
    let r = s.execute(FILL, json!({"variations": 1})).unwrap();
    let sent = fake.sent.lock().unwrap();
    let img = codecs::decode(&sent[0].image).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
    let (x0, y0) = (r["bounds"][0].as_i64().unwrap() as i32, r["bounds"][1].as_i64().unwrap() as i32);
    let at = |x: i32, y: i32| {
        let i = ((y - y0) as usize * img.width() as usize + (x - x0) as usize) * 4;
        [img.data()[i], img.data()[i + 1], img.data()[i + 2]]
    };
    for (x, y) in [(60, 60), (41, 41), (79, 60)] {
        assert!(at(x, y).iter().all(|&c| c >= 250), "({x},{y}) {:?}: filled from the white around it", at(x, y));
    }
    assert_eq!(at(30, 30), [255, 255, 255], "the context is sent as it is");
}

#[test]
fn the_fill_takes_the_colours_beside_it() {
    // Left half black, right half white, a hole across the middle: each side of the hole takes
    // the colour next to it.
    let (w, h) = (64usize, 32usize);
    let original: Vec<f32> = (0..w * h).flat_map(|i| if i % w < 32 { [0.0; 3] } else { [1.0; 3] }).collect();
    let coverage: Vec<f32> = (0..w * h).map(|i| if (16..48).contains(&(i % w)) && (8..24).contains(&(i / w)) { 1.0 } else { 0.0 }).collect();
    let fill = surroundings_fill(&original, &coverage, w, h);
    assert!(fill[(16 * w + 17) * 3] < 0.2, "{}", fill[(16 * w + 17) * 3]);
    assert!(fill[(16 * w + 46) * 3] > 0.8, "{}", fill[(16 * w + 46) * 3]);
}

#[test]
fn variations_are_requested_at_the_same_time() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let delay = std::time::Duration::from_millis(400);
    let fake = Arc::new(Fake { delay, ..Arc::into_inner(Fake::new([0, 0, 0])).unwrap() });
    s.set_image_provider(Some(fake));
    let start = std::time::Instant::now();
    let r = s.execute(FILL, json!({"variations": 4})).unwrap();
    assert_eq!(r["variations"].as_array().unwrap().len(), 4);
    // One after another would take 1.6 s.
    assert!(start.elapsed() < delay * 2, "{:?}", start.elapsed());
}

#[test]
fn a_models_colour_drift_is_taken_out() {
    // The fake redraws the image 40 levels darker everywhere. Outside the selection that shows
    // it drifted, so the fill (white, from around the selection) comes back white, not 215.
    let mut s = session(160, 160);
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60})).unwrap();
    let fake = Arc::new(Fake { echo: Some(-40), ..Arc::into_inner(Fake::new([0, 0, 0])).unwrap() });
    s.set_image_provider(Some(fake));
    s.execute(FILL, json!({"variations": 1})).unwrap();
    for (x, y) in [(80, 80), (52, 52), (108, 108)] {
        let [r, g, b] = rgb_at(&s, x, y);
        assert!([r, g, b].iter().all(|&c| c >= 252), "({x},{y}): {r} {g} {b}");
    }
}

#[test]
fn drift_that_varies_across_the_image_leaves_no_seam() {
    // A flat mid-grey photo; the model's result drifts from -0.1 on the left to +0.1 on the right.
    let (w, h) = (100u32, 80u32);
    let n = (w * h) as usize;
    let inside = |i: usize| {
        let (x, y) = (i % w as usize, i / w as usize);
        (30..70).contains(&x) && (20..60).contains(&y)
    };
    let reference = Reference { original: vec![0.5; n * 3], coverage: (0..n).map(|i| if inside(i) { 1.0 } else { 0.0 }).collect(), w, h };
    let mut px: Vec<f32> = (0..n)
        .flat_map(|i| {
            let d = 0.2 * (i % w as usize) as f32 / w as f32 - 0.1;
            [0.5 + d, 0.5 + d, 0.5 + d, 1.0]
        })
        .collect();
    let stats = match_surroundings(&mut px, &reference);
    assert!(stats.starts_with("mean difference"), "{stats}");
    let worst = (0..n).filter(|&i| inside(i)).map(|i| (px[i * 4] - 0.5).abs()).fold(0.0f32, f32::max);
    assert!(worst < 0.02, "worst {worst}");
}

/// Spins until its job is cancelled (or 5 s pass) and records whether it saw the cancel.
struct Waits {
    started: std::sync::atomic::AtomicBool,
    saw_cancel: std::sync::atomic::AtomicBool,
}

impl ImageProvider for Waits {
    fn info(&self) -> ProviderInfo {
        ProviderInfo { id: "waits", name: "Waits", model: "w".into(), host: "example.invalid" }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { generate: false, edit: true, variations: false }
    }
    fn edit(&self, r: &EditRequest) -> photocraft_genai::Result<GeneratedImage> {
        self.started.store(true, Ordering::SeqCst);
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            if r.cancel.is_cancelled() {
                self.saw_cancel.store(true, Ordering::SeqCst);
                return Err(GenError::Cancelled);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Err(GenError::Timeout)
    }
}

#[test]
fn cancelling_the_job_reaches_the_provider() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let waits = Arc::new(Waits { started: false.into(), saw_cancel: false.into() });
    s.set_image_provider(Some(waits.clone()));
    let crate::jobs::Started::Job(id) = s.start(FILL, json!({"prompt": "x", "variations": 1})).unwrap() else { panic!("expected a job") };
    // Cancel only once the provider is waiting, or the job stops before ever calling it.
    let start = std::time::Instant::now();
    while !waits.started.load(Ordering::SeqCst) && start.elapsed() < std::time::Duration::from_secs(4) {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(s.cancel_job(id));
    let start = std::time::Instant::now();
    while !waits.saw_cancel.load(Ordering::SeqCst) && start.elapsed() < std::time::Duration::from_secs(4) {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(waits.saw_cancel.load(Ordering::SeqCst), "the provider's token turned cancelled");
}

/// A provider with size limits, answering like `Fake`.
struct Sized {
    inner: Arc<Fake>,
    max: u32,
    min: u32,
}

impl ImageProvider for Sized {
    fn info(&self) -> ProviderInfo {
        self.inner.info()
    }
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn max_side(&self) -> u32 {
        self.max
    }
    fn min_side(&self) -> u32 {
        self.min
    }
    fn edit(&self, r: &EditRequest) -> photocraft_genai::Result<GeneratedImage> {
        self.inner.edit(r)
    }
}

#[test]
fn a_tiny_selection_is_upscaled_to_the_providers_minimum() {
    let mut s = session(200, 200);
    s.execute("select.rect", json!({"x": 95, "y": 95, "width": 10, "height": 10})).unwrap();
    let fake = Fake::new([255, 0, 0]);
    s.set_image_provider(Some(Arc::new(Sized { inner: fake.clone(), max: 2048, min: 512 })));
    s.execute(FILL, json!({"prompt": "x", "variations": 1})).unwrap();
    let sent = fake.sent.lock().unwrap();
    let (w, h) = photocraft_genai::png_size(&sent[0].image).unwrap();
    assert_eq!(w.max(h), 512, "the upload's long side reaches min_side");
    assert_eq!(rgb_at(&s, 100, 100), [255, 0, 0], "and the result still lands in the selection");
}

#[test]
fn a_providers_max_side_caps_the_upload() {
    let mut s = session(3000, 1000);
    s.execute("select.rect", json!({"x": 100, "y": 100, "width": 2600, "height": 700})).unwrap();
    let fake = Fake::new([0, 0, 255]);
    s.set_image_provider(Some(Arc::new(Sized { inner: fake.clone(), max: 1536, min: 0 })));
    s.execute(FILL, json!({"prompt": "x", "variations": 1})).unwrap();
    let (w, h) = photocraft_genai::png_size(&fake.sent.lock().unwrap()[0].image).unwrap();
    assert!(w.max(h) <= 1536, "{w}x{h}");
}

#[test]
fn upload_scale_shrinks_grows_and_never_exceeds_the_upload_side() {
    assert!((upload_scale(4096.0, 2048, 0) - 0.5).abs() < 1e-9);
    assert!((upload_scale(1000.0, 2048, 0) - 1.0).abs() < 1e-9);
    assert!((upload_scale(100.0, 2048, 512) - 5.12).abs() < 1e-9);
    assert!((upload_scale(3072.0, 1536, 0) - 0.5).abs() < 1e-9);
    assert!(upload_scale(10.0, 2048, 100_000) <= 204.8 + 1e-9, "min_side is capped at UPLOAD_SIDE");
}

/// `Fake` under another id.
struct Named {
    id: &'static str,
    inner: Arc<Fake>,
    needs_prompt: bool,
    can_edit: bool,
}

impl ImageProvider for Named {
    fn info(&self) -> ProviderInfo {
        ProviderInfo { id: self.id, name: self.id, model: "m".into(), host: "example.invalid" }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { edit: self.can_edit, ..self.inner.capabilities() }
    }
    fn needs_prompt(&self) -> bool {
        self.needs_prompt
    }
    fn edit(&self, r: &EditRequest) -> photocraft_genai::Result<GeneratedImage> {
        self.inner.edit(r)
    }
}

fn two_providers(s: &mut Session) -> (Arc<Fake>, Arc<Fake>) {
    let (red, blue) = (Fake::new([255, 0, 0]), Fake::new([0, 0, 255]));
    let mut map: std::collections::BTreeMap<String, SharedProvider> = Default::default();
    map.insert("red".into(), Arc::new(Named { id: "red", inner: red.clone(), needs_prompt: false, can_edit: true }));
    map.insert("blue".into(), Arc::new(Named { id: "blue", inner: blue.clone(), needs_prompt: true, can_edit: true }));
    s.set_image_providers(map, Some("red".into()));
    (red, blue)
}

#[test]
fn the_provider_param_picks_one_and_the_default_is_used_without_it() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let (red, blue) = two_providers(&mut s);
    assert_eq!(s.image_provider_ids(), vec!["blue".to_owned(), "red".to_owned()]);
    assert_eq!(s.image_provider().unwrap().info().id, "red");
    s.execute(FILL, json!({"prompt": "x", "variations": 1})).unwrap();
    assert_eq!((red.sent.lock().unwrap().len(), blue.sent.lock().unwrap().len()), (1, 0));
    s.execute(FILL, json!({"prompt": "x", "variations": 1, "provider": "blue"})).unwrap();
    assert_eq!(blue.sent.lock().unwrap().len(), 1);
    let e = s.execute(FILL, json!({"prompt": "x", "provider": "green"})).unwrap_err().to_string();
    assert!(e.contains("green"), "{e}");
    assert!(s.execute(FILL, json!({"prompt": "x", "provider": 3})).is_err());
}

#[test]
fn an_empty_prompt_is_refused_before_sending_for_providers_that_need_one() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let (_, blue) = two_providers(&mut s);
    let history = s.active().unwrap().history.past_len();
    let e = s.execute(FILL, json!({"prompt": "  ", "provider": "blue"})).unwrap_err().to_string();
    assert!(e.contains("needs a prompt"), "{e}");
    assert!(blue.sent.lock().unwrap().is_empty());
    assert_eq!(s.active().unwrap().history.past_len(), history);
    s.execute(FILL, json!({"prompt": "", "variations": 1})).unwrap();
}

#[test]
fn generate_more_uses_the_provider_that_made_the_layer() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let (red, blue) = two_providers(&mut s);
    let r = s.execute(FILL, json!({"prompt": "x", "variations": 1, "provider": "blue"})).unwrap();
    let layer = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(s.generative_provider(layer), Some("blue"), "the provider recorded on the layer, not the default");
    assert_eq!(s.generative_provider(photocraft_doc::LayerId(u64::MAX)), None);
    s.execute(MORE, json!({"layer": r["layer"], "count": 1})).unwrap();
    assert_eq!(blue.sent.lock().unwrap().len(), 2, "more went to blue, not the default");
    assert!(red.sent.lock().unwrap().is_empty());
    s.execute(MORE, json!({"layer": r["layer"], "count": 1, "provider": "red"})).unwrap();
    assert_eq!(red.sent.lock().unwrap().len(), 1);
}

#[test]
fn generate_more_needs_the_provider_that_made_the_layer() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    two_providers(&mut s);
    let r = s.execute(FILL, json!({"prompt": "x", "variations": 1, "provider": "blue"})).unwrap();
    // Blue is removed in the settings.
    let red: SharedProvider = s.image_provider_by_id("red").unwrap().clone();
    s.set_image_providers([("red".to_owned(), red)].into(), Some("red".into()));
    let history = s.active().unwrap().history.past_len();
    let children = group(&s, r["layer"].as_u64().unwrap()).children().unwrap().len();
    let e = s.execute(MORE, json!({"layer": r["layer"], "count": 1})).unwrap_err().to_string();
    assert!(e.contains("blue"), "{e}");
    assert_eq!(s.active().unwrap().history.past_len(), history, "no undo step");
    assert_eq!(group(&s, r["layer"].as_u64().unwrap()).children().unwrap().len(), children);
}

#[test]
fn set_image_provider_still_installs_one_default() {
    let mut s = session(16, 16);
    s.set_image_provider(Some(Fake::new([1, 2, 3])));
    assert_eq!(s.image_provider_ids(), vec!["fake".to_owned()]);
    s.set_image_provider(None);
    assert!(s.image_provider().is_none() && s.image_provider_ids().is_empty());
}

/// "red" (the default) and "blue", with the given edit abilities.
fn abilities(s: &mut Session, red_edits: bool, blue_edits: bool) -> (Arc<Fake>, Arc<Fake>) {
    let (red, blue) = (Fake::new([255, 0, 0]), Fake::new([0, 0, 255]));
    let mut map: std::collections::BTreeMap<String, SharedProvider> = Default::default();
    map.insert("red".into(), Arc::new(Named { id: "red", inner: red.clone(), needs_prompt: false, can_edit: red_edits }));
    map.insert("blue".into(), Arc::new(Named { id: "blue", inner: blue.clone(), needs_prompt: false, can_edit: blue_edits }));
    s.set_image_providers(map, Some("red".into()));
    (red, blue)
}

#[test]
fn a_chosen_provider_that_can_edit_works_when_the_default_cannot() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let (red, blue) = abilities(&mut s, false, true);
    assert!(s.is_enabled(FILL), "any provider that can edit enables the command");
    s.execute(FILL, json!({"prompt": "x", "variations": 1, "provider": "blue"})).unwrap();
    assert_eq!((red.sent.lock().unwrap().len(), blue.sent.lock().unwrap().len()), (0, 1));
}

#[test]
fn a_chosen_provider_that_cannot_edit_is_refused_before_sending() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let (_, blue) = abilities(&mut s, true, false);
    let history = s.active().unwrap().history.past_len();
    let e = s.execute(FILL, json!({"prompt": "x", "provider": "blue"})).unwrap_err().to_string();
    assert!(e.contains("can't fill selections"), "{e}");
    assert!(blue.sent.lock().unwrap().is_empty());
    assert_eq!(s.active().unwrap().history.past_len(), history);
    // Generate More checks the provider that made the layer too.
    let r = s.execute(FILL, json!({"prompt": "x", "variations": 1})).unwrap();
    abilities(&mut s, true, false);
    let e = s.execute(MORE, json!({"layer": r["layer"], "provider": "blue"})).unwrap_err().to_string();
    assert!(e.contains("can't fill selections"), "{e}");
    assert!(blue.sent.lock().unwrap().is_empty());
}

/// The generative group's mask value at (`x`, `y`).
fn mask_at(s: &Session, id: u64, x: i32, y: i32) -> f32 {
    group(s, id).mask.as_ref().unwrap().value(x, y)
}

#[test]
fn the_edge_band_grows_with_the_selection() {
    assert_eq!(edge_band(Rect::from_xywh(0, 0, 40, 40)), 0, "small selections keep a hard edge");
    assert_eq!(edge_band(Rect::from_xywh(0, 0, 63, 10)), 0);
    assert_eq!(edge_band(Rect::from_xywh(0, 0, 1000, 1000)), 15);
    assert_eq!(edge_band(Rect::from_xywh(0, 0, 4000, 4000)), 32, "capped");
    assert_eq!(edge_band(Rect::from_xywh(0, 0, 10, 2000)), 30, "the long side counts");
    assert_eq!(edge_band(Rect::EMPTY), 0);
}

#[test]
fn the_generative_layer_mask_has_a_soft_edge_past_the_selection() {
    let mut s = session(300, 300);
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    s.set_image_provider(Some(Fake::new([255, 0, 0])));
    let r = s.execute(FILL, json!({"prompt": "x", "variations": 1})).unwrap();
    let id = r["layer"].as_u64().unwrap();
    let b = edge_band(Rect::from_xywh(50, 50, 100, 100)) as i32;
    assert!(b > 0);
    assert!(mask_at(&s, id, 100, 100) > 0.99, "{}", mask_at(&s, id, 100, 100));
    assert!(mask_at(&s, id, 50 - b - 2, 100) < 0.01, "{}", mask_at(&s, id, 50 - b - 2, 100));
    let edge = mask_at(&s, id, 50, 100);
    assert!(edge > 0.0 && edge < 1.0, "the selection's edge is blended: {edge}");
    assert!(mask_at(&s, id, 49, 100) > 0.0, "the blend reaches past the selection");
    // The variation covers the band the mask now reveals.
    assert_eq!(variation_rgb(&s, id, 0, 50 - b, 100), variation_rgb(&s, id, 0, 100, 100));
    // The selection itself is unchanged.
    assert_eq!(doc(&s).selection.as_ref().unwrap().content_bounds(), Rect::from_xywh(50, 50, 100, 100));
}

#[test]
fn a_small_selection_keeps_its_hard_edge() {
    let mut s = session(100, 100);
    s.execute("select.rect", json!({"x": 30, "y": 30, "width": 40, "height": 40})).unwrap();
    s.set_image_provider(Some(Fake::new([255, 0, 0])));
    let r = s.execute(FILL, json!({"prompt": "x", "variations": 1})).unwrap();
    let id = r["layer"].as_u64().unwrap();
    for (x, y) in [(29, 50), (50, 29), (70, 50), (50, 70), (10, 10)] {
        assert_eq!(mask_at(&s, id, x, y), 0.0, "({x},{y})");
    }
    for (x, y) in [(30, 50), (69, 69), (50, 50)] {
        assert_eq!(mask_at(&s, id, x, y), 1.0, "({x},{y})");
    }
}

#[test]
fn the_soft_edge_does_not_fade_at_the_canvas_edge() {
    // A selection running off the canvas continues past it, so the fill isn't dimmed there.
    let mut s = session(300, 300);
    s.execute("select.rect", json!({"x": 0, "y": 50, "width": 150, "height": 150})).unwrap();
    s.set_image_provider(Some(Fake::new([255, 0, 0])));
    let r = s.execute(FILL, json!({"prompt": "x", "variations": 1})).unwrap();
    let id = r["layer"].as_u64().unwrap();
    assert!(mask_at(&s, id, 0, 120) > 0.99, "{}", mask_at(&s, id, 0, 120));
    let edge = mask_at(&s, id, 149, 120);
    assert!(edge > 0.0 && edge < 1.0, "{edge}");
}

#[test]
fn blended_mask_softens_partial_selections_too() {
    // A feathered (partial) selection still works: values stay in 0..1 and the middle is kept.
    let canvas = Rect::from_xywh(0, 0, 400, 400);
    let selected = Rect::from_xywh(100, 100, 200, 200);
    let m: Vec<f32> = (0..200 * 200).map(|i| if (i % 200) < 10 { 0.5 } else { 1.0 }).collect();
    let sel = photocraft_algo::selection::combine(None, &m, selected, photocraft_algo::selection::SelectionMode::Replace).unwrap();
    let out = blended_mask(&sel, selected, canvas);
    let v = |x: i32, y: i32| out.sample_channel(x, y, 0);
    assert!(v(200, 200) > 0.99);
    assert!(v(50, 200) == 0.0);
    for x in 90..110 {
        let a = v(x, 200);
        assert!((0.0..=1.0).contains(&a), "{x}: {a}");
    }
}
