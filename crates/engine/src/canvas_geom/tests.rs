use super::*;
use crate::Session;
use photocraft_doc::vector::LiveShape;
use serde_json::json;

const W: u32 = 64;
const H: u32 = 40;

#[test]
fn quarter_turn_swaps_current_physical_axes_with_native_undo_and_unit_intact() {
    for command in ["image.imageRotation.90cw", "image.imageRotation.90ccw"] {
        let mut document = Document::new("Own physical turn", photocraft_doc::Size::new(4, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let unit = photocraft_doc::PhysicalUnit::Custom(photocraft_doc::CustomUnit {
            name: "Own unit".into(),
            factor: 2.5,
            digits: 3,
            symbol: "ou".into(),
            abbreviation: "ou".into(),
        });
        document.set_physical_resolution(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: unit.clone() }).unwrap();
        document.layers.push(photocraft_doc::Layer::raster("Own raster", document.pixel_format()));
        let mut session = Session::new();
        session.open_document(document, None).unwrap();
        let before = session.active().unwrap().doc.clone();
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        session.execute(command, json!({})).unwrap();
        let after = session.active().unwrap().doc.clone();
        assert_eq!(after.size, photocraft_doc::Size::new(2, 4));
        assert_eq!(after.effective_resolution(), photocraft_doc::PhysicalResolution { x_dpi: 288.0, y_dpi: 144.0, unit });
        assert_eq!(after.resolution_dpi, 288.0);
        assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&after, &Default::default()).unwrap()).unwrap(), *after);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), native);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, after);
    }
}

#[test]
fn half_turn_flips_and_legacy_only_turn_do_not_replace_physical_metadata() {
    for command in ["image.imageRotation.180", "image.imageRotation.flipCanvasHorizontal", "image.imageRotation.flipCanvasVertical"] {
        let mut document = Document::new("Own axes", photocraft_doc::Size::new(4, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        document
            .set_physical_resolution(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: photocraft_doc::PhysicalUnit::Millimeter })
            .unwrap();
        let expected = document.physical_resolution.clone();
        let mut session = Session::new();
        session.open_document(document, None).unwrap();
        session.execute(command, json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.physical_resolution, expected);
        assert_eq!(session.active().unwrap().doc.resolution_dpi, 144.0);
    }
    let mut document = Document::new("Own legacy", photocraft_doc::Size::new(4, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    document.resolution_dpi = 96.0;
    turn_canvas(&mut document, Turn::Cw90).unwrap();
    assert!(document.physical_resolution.is_none());
    assert_eq!(document.resolution_dpi, 96.0);
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

/// Paints the active raster layer with `f(x, y)` (straight RGBA).
fn paint(s: &mut Session, f: impl Fn(i32, i32) -> [f32; 4]) {
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                data.extend(photocraft_raster::from_rgba(&fmt, f(x, y)));
            }
        }
        surf.write_region(b, &data);
        surf.prune();
        Ok(())
    })
    .unwrap();
}

/// An asymmetric document: a gradient Background, a type layer, a shape layer, a raster layer
/// with a vector mask and a smart object, plus a guide, a saved path and a slice.
fn mixed(depth: u64, mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": depth, "mode": mode})).unwrap();
    paint(&mut s, |x, y| [x as f32 / W as f32, y as f32 / H as f32, 0.3, 1.0]);
    s.execute("type.create", json!({"x": 4, "y": 16, "text": "Fj", "size": 14, "color": "#102080"})).unwrap();
    s.execute("shape.create", json!({"kind": "rect", "rect": [40, 4, 18, 9], "fill": "#ff8800"})).unwrap();
    s.execute("shape.create", json!({"kind": "ellipse", "rect": [6, 24, 12, 10], "fill": "#00aa44"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    paint(&mut s, |x, y| if (28..60).contains(&x) && (18..38).contains(&y) { [0.9, 0.1, 0.6, 1.0] } else { [0.0; 4] });
    s.execute("path.set", json!({"path": {"subpaths": [{"knots": [[30, 20], [58, 22], [36, 37]]}]}})).unwrap();
    s.execute("layer.vectorMask.currentPath", json!({})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    paint(&mut s, |x, y| if (20..27).contains(&x) && (2..12).contains(&y) { [0.1, 0.8, 0.9, 1.0] } else { [0.0; 4] });
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "horizontal", "position": 5})).unwrap();
    s.execute("path.set", json!({"name": "Saved", "path": {"subpaths": [{"knots": [[2, 2], [12, 2], [12, 8]]}]}})).unwrap();
    s.execute("slice.new", json!({"rect": [1, 2, 10, 6]})).unwrap();
    s
}

fn command(t: Turn) -> &'static str {
    match t {
        Turn::FlipHorizontal => "image.imageRotation.flipCanvasHorizontal",
        Turn::FlipVertical => "image.imageRotation.flipCanvasVertical",
        Turn::Rotate180 => "image.imageRotation.180",
        Turn::Cw90 => "image.imageRotation.90cw",
        Turn::Ccw90 => "image.imageRotation.90ccw",
    }
}

const TURNS: [Turn; 5] = [Turn::FlipHorizontal, Turn::FlipVertical, Turn::Rotate180, Turn::Cw90, Turn::Ccw90];

/// The flattened original, remapped by the turn's pixel map.
fn expected(before: &photocraft_compose::Buffer, t: Turn, nw: u32) -> Vec<[f32; 4]> {
    let map = t.pixel_map(W as i32, H as i32);
    let mut out = vec![[0.0f32; 4]; before.px.len()];
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            let (nx, ny) = map(x, y);
            out[ny as usize * nw as usize + nx as usize] = before.px[y as usize * W as usize + x as usize];
        }
    }
    out
}

/// (mean absolute difference, fraction of pixels differing by more than 0.2).
fn diff(a: &[[f32; 4]], b: &[[f32; 4]]) -> (f32, f32) {
    assert_eq!(a.len(), b.len());
    let mut sum = 0.0;
    let mut big = 0usize;
    for (p, q) in a.iter().zip(b) {
        let d = (0..4).map(|c| (p[c] * p[3] - q[c] * q[3]).abs()).fold(0.0f32, f32::max);
        sum += d;
        big += usize::from(d > 0.2);
    }
    (sum / a.len() as f32, big as f32 / a.len() as f32)
}

fn check_turn(depth: u64, mode: &str, t: Turn) {
    let mut s = mixed(depth, mode);
    let before = photocraft_compose::flatten(doc(&s)).unwrap();
    s.execute(command(t), json!({})).unwrap();
    let nw = doc(&s).size.width;
    let want = expected(&before, t, nw);
    // Caches are remapped exactly; vector masks re-rasterize from their mapped path.
    let got = photocraft_compose::flatten(doc(&s)).unwrap();
    let (mean, big) = diff(&got.px, &want);
    assert!(mean < 1.0 / 255.0 && big < 0.01, "{t:?} {depth} {mode}: mean {mean} big {big}");
    // Re-rendering every type, shape and smart object from its *geometry* gives the same image,
    // so the transforms (not only the cached pixels) moved.
    let mut d = doc(&s).clone();
    refresh_with_interrupt(&mut d, Refresh::All, &photocraft_raster::Interrupt::NONE).unwrap();
    let (mean, big) = diff(&photocraft_compose::flatten(&d).unwrap().px, &want);
    assert!(mean < 2.0 / 255.0 && big < 0.01, "{t:?} {depth} {mode} re-rendered: mean {mean} big {big}");
    // Undo restores the original, bit-exactly.
    s.execute("edit.undo", json!({})).unwrap();
    let undone = photocraft_compose::flatten(doc(&s)).unwrap();
    assert_eq!(undone.px.len(), before.px.len(), "{t:?} {depth} {mode}: undo changed the canvas size");
    let (mut n, mut max, mut first) = (0usize, 0.0f32, None);
    for (i, (p, q)) in undone.px.iter().zip(&before.px).enumerate() {
        if p != q {
            n += 1;
            max = (0..4).map(|c| (p[c] - q[c]).abs()).fold(max, f32::max);
            first.get_or_insert((i % W as usize, i / W as usize, *p, *q));
        }
    }
    assert!(n == 0, "{t:?} {depth} {mode}: undo left {n} pixels different (max {max}); first (x, y, undone, before): {first:?}");
}

#[test]
fn right_angle_turns_move_vectors_type_and_smart_objects() {
    for t in TURNS {
        check_turn(8, "rgb", t);
    }
}

#[test]
fn turns_at_every_depth_and_mode() {
    for (depth, mode) in [(16, "rgb"), (32, "rgb"), (8, "cmyk"), (16, "cmyk"), (8, "gray"), (16, "gray")] {
        check_turn(depth, mode, Turn::Cw90);
    }
    check_turn(32, "rgb", Turn::FlipHorizontal);
    check_turn(8, "cmyk", Turn::Ccw90);
}

#[test]
fn turns_map_guides_paths_slices_and_live_shapes() {
    let mut s = mixed(8, "rgb");
    s.execute("image.imageRotation.90cw", json!({})).unwrap();
    let d = doc(&s);
    assert_eq!((d.size.width, d.size.height), (H, W));
    // A vertical guide at x=10 becomes a horizontal one at y=10; a horizontal one at y=5 becomes
    // a vertical one at x = H − 5.
    assert_eq!(d.guides.horizontal, vec![10.0]);
    assert_eq!(d.guides.vertical, vec![f64::from(H - 5)]);
    // Slice [1,2 10×6] → x' = H − y: [H−8, 1, 6×10].
    let sl = &d.slices.list[0];
    assert_eq!(sl.rect, Rect::new(H as i32 - 8, 1, H as i32 - 2, 11));
    let saved = d.paths.iter().find(|p| p.name == "Saved").unwrap();
    let k = saved.path.subpaths[0].knots[1].anchor;
    assert!((k.x - (f64::from(H) - 2.0)).abs() < 1e-9 && (k.y - 12.0).abs() < 1e-9, "{k:?}");
    // The rectangle stays a live rectangle with the rotated box.
    let live: Vec<&LiveShape> = d
        .walk()
        .into_iter()
        .filter_map(|(_, _, l)| match &l.content {
            LayerContent::Shape(sh) => sh.live.as_ref(),
            _ => None,
        })
        .collect();
    assert!(
        live.iter()
            .any(|l| matches!(l, LiveShape::Rect { rect, .. } if rect.iter().zip([f64::from(H) - 13.0, 40.0, 9.0, 18.0]).all(|(a, b)| (a - b).abs() < 1e-9))),
        "{live:?}"
    );
    assert!(live.iter().any(|l| matches!(l, LiveShape::Ellipse { .. })));
    // Flip twice is the identity for the geometry.
    let before = doc(&s).clone();
    s.execute("image.imageRotation.flipCanvasVertical", json!({})).unwrap();
    s.execute("image.imageRotation.flipCanvasVertical", json!({})).unwrap();
    let after = doc(&s);
    assert_eq!(after.guides, before.guides);
    assert_eq!(after.slices.list[0].rect, before.slices.list[0].rect);
    assert_eq!(photocraft_compose::flatten(after).unwrap().px, photocraft_compose::flatten(&before).unwrap().px);
}

#[test]
fn gradient_fill_angle_follows_the_canvas() {
    let a = Turn::Cw90.affine(10.0, 20.0);
    // Up (90°) turns clockwise to the right (0°); flips mirror.
    assert_eq!(map_angle(&a, 90.0), 0.0);
    assert_eq!(map_angle(&Turn::FlipHorizontal.affine(10.0, 20.0), 0.0), 180.0);
    assert_eq!(map_angle(&Turn::FlipVertical.affine(10.0, 20.0), 30.0), -30.0);
    assert_eq!(map_angle(&Turn::Ccw90.affine(10.0, 20.0), 0.0), 90.0);
}

fn type_and_shape_origin(d: &Document) -> ([f64; 6], [f64; 2]) {
    let mut t = None;
    let mut k = None;
    for (_, _, l) in d.walk() {
        match &l.content {
            LayerContent::Text(x) => t = Some(x.transform.m),
            LayerContent::Shape(sh) if k.is_none() => k = Some([sh.path.subpaths[0].knots[0].anchor.x, sh.path.subpaths[0].knots[0].anchor.y]),
            _ => {}
        }
    }
    (t.unwrap(), k.unwrap())
}

#[test]
fn canvas_size_and_crop_move_vector_geometry() {
    let mut s = mixed(8, "rgb");
    let before = photocraft_compose::flatten(doc(&s)).unwrap();
    let (t0, k0) = type_and_shape_origin(doc(&s));
    s.execute("image.canvasSize", json!({"width": 20, "height": 10, "relative": true, "anchor": "bottomRight", "extensionColor": "transparent"})).unwrap();
    let d = doc(&s);
    let (t1, k1) = type_and_shape_origin(d);
    assert_eq!((t1[4] - t0[4], t1[5] - t0[5]), (20.0, 10.0));
    assert_eq!((k1[0] - k0[0], k1[1] - k0[1]), (20.0, 10.0));
    assert_eq!(d.guides.vertical, vec![30.0]);
    assert_eq!(d.slices.list[0].rect, Rect::new(21, 12, 31, 18));
    // The old canvas shows unchanged at its new offset, also after re-rendering from geometry.
    let mut r = d.clone();
    refresh_with_interrupt(&mut r, Refresh::All, &photocraft_raster::Interrupt::NONE).unwrap();
    for img in [photocraft_compose::flatten(d).unwrap(), photocraft_compose::flatten(&r).unwrap()] {
        let nw = img.rect.width() as usize;
        let shifted: Vec<[f32; 4]> = (0..H as usize).flat_map(|y| img.px[(y + 10) * nw + 20..(y + 10) * nw + 20 + W as usize].to_vec()).collect();
        let (mean, big) = diff(&shifted, &before.px);
        assert!(mean < 2.0 / 255.0 && big < 0.01, "mean {mean} big {big}");
    }
    // Crop back: everything returns.
    s.execute("image.crop", json!({"x": 20, "y": 10, "width": W, "height": H})).unwrap();
    let (t2, k2) = type_and_shape_origin(doc(&s));
    assert_eq!((t2, k2), (t0, k0));
    let (mean, _) = diff(&photocraft_compose::flatten(doc(&s)).unwrap().px, &before.px);
    assert!(mean < 1.0 / 255.0, "{mean}");
}

#[test]
fn image_size_scales_vector_geometry() {
    let mut s = mixed(8, "rgb");
    let (t0, k0) = type_and_shape_origin(doc(&s));
    s.execute("image.imageSize", json!({"width": W * 2, "height": H * 2})).unwrap();
    let d = doc(&s);
    let (t1, k1) = type_and_shape_origin(d);
    assert!((t1[0] - 2.0 * t0[0]).abs() < 1e-9 && (t1[4] - 2.0 * t0[4]).abs() < 1e-9, "{t0:?} {t1:?}");
    assert_eq!((k1[0], k1[1]), (2.0 * k0[0], 2.0 * k0[1]));
    assert_eq!(d.guides.vertical, vec![20.0]);
}

#[test]
fn arbitrary_rotation_moves_marks_and_unlinked_vector_masks() {
    let mut s = mixed(8, "rgb");
    // Unlink the vector mask: a canvas rotation still moves it.
    s.edit("unlink", |doc, _| {
        for l in doc.layers.iter_mut() {
            if let Some(vm) = &mut l.vector_mask {
                vm.linked = false;
            }
        }
        Ok(())
    })
    .unwrap();
    let before = doc(&s).clone();
    s.execute("image.rotation.arbitrary", json!({"angle": 90, "direction": "cw"})).unwrap();
    let d = doc(&s);
    let a = Turn::Cw90.affine(f64::from(W), f64::from(H));
    let vm = |d: &Document| d.walk().into_iter().find_map(|(_, _, l)| l.vector_mask.as_ref().map(|v| v.path.subpaths[0].knots[0].anchor)).unwrap();
    let (p, q) = (a.apply(vm(&before)), vm(d));
    assert!((p.x - q.x).abs() < 1e-6 && (p.y - q.y).abs() < 1e-6, "{p:?} {q:?}");
    assert_eq!(d.slices.list[0].rect, map_rect(&a, before.slices.list[0].rect));
    // 90° arbitrary == 90° clockwise for the composite.
    let mut s2 = mixed(8, "rgb");
    s2.execute("image.imageRotation.90cw", json!({})).unwrap();
    let (mean, big) = diff(&photocraft_compose::flatten(d).unwrap().px, &photocraft_compose::flatten(doc(&s2)).unwrap().px);
    assert!(mean < 3.0 / 255.0 && big < 0.02, "mean {mean} big {big}");
}

#[test]
fn turns_on_empty_and_tiny_documents_never_panic() {
    for (w, h) in [(1, 1), (1, 7), (5, 1)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": w, "height": h})).unwrap();
        for t in TURNS {
            s.execute(command(t), json!({"junk": [1, 2]})).unwrap();
        }
        s.execute("image.rotation.arbitrary", json!({"angle": 33})).unwrap();
    }
    let mut s = Session::new();
    for t in TURNS {
        assert!(s.execute(command(t), json!({})).is_err(), "no document");
    }
}

#[test]
fn checked_turn_charges_all_surfaces_before_authoring() {
    let mut d = Document::new("aggregate", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F64);
    for _ in 0..2 {
        let mut layer = photocraft_doc::Layer::raster("wide", d.pixel_format());
        let pixel: Vec<_> = [0.25f64, 0.5, 0.75, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect();
        layer.surface_mut().unwrap().try_write_interleaved(photocraft_geom::Rect::new(0, 0, 1, 1), &pixel).unwrap();
        d.layers.push(layer);
    }
    let a = Turn::Cw90.affine(1.0, 1.0);
    let limits = photocraft_algo::wide_geometry::GeometryLimits { resident_bytes: 18 << 20, work_units: 100 };
    let mut one = d.clone();
    one.layers.pop();
    assert!(super::preflight_checked_turn(&one, &a, limits).is_ok());
    assert!(super::preflight_checked_turn(&d, &a, limits).is_err());
    let limits = photocraft_algo::wide_geometry::GeometryLimits { resident_bytes: 64 << 20, work_units: 63 };
    assert!(super::preflight_checked_turn(&d, &a, limits).is_err());
}

#[test]
fn checked_turn_rejects_unrepresentable_declared_geometry_atomically() {
    let mut d = Document::new("bounds", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F64);
    let layer = photocraft_doc::Layer::raster("wide", d.pixel_format());
    let mut state = photocraft_doc::XcfState::default();
    state.items.push(photocraft_doc::xcf_state::ItemState {
        item: photocraft_doc::item_identity::ItemRef::Layer(layer.id),
        raster_bounds: Some(photocraft_geom::Rect::new(i32::MIN, 0, i32::MIN + 1, 1)),
        name: layer.name.clone(),
        custom_name: false,
        visible: true,
        locks: Default::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        composition: None,
        mask: None,
    });
    d.layers.push(layer);
    d.xcf_state = Some(state);
    let before = d.clone();
    assert!(super::turn_canvas(&mut d, Turn::Cw90).is_err());
    assert_eq!(d, before);
}

// Synthetic command/native regressions: no corpus or rotation-axis reference claim.
type PhysicalTestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn physical_text(doc: &Document, id: photocraft_doc::LayerId) -> PhysicalTestResult<&TextLayer> {
    match &doc.layer(id).ok_or("missing text layer")?.content {
        photocraft_doc::LayerContent::Text(t) => Ok(t),
        _ => Err("not text".into()),
    }
}
fn physical_size(t: &TextLayer) -> PhysicalTestResult<f32> {
    Ok(t.char_runs().first().ok_or("missing style")?.style.size_pt)
}
fn parsed_size(t: &TextLayer) -> PhysicalTestResult<f32> {
    let parsed = photocraft_text::psd::text_layer_from_tysh(t.psd_raw.as_deref().ok_or("missing TySh")?, 144.0).ok_or("TySh parser refused")?;
    physical_size(&parsed)
}
fn physical_native(doc: &Document, id: Option<photocraft_doc::LayerId>) -> PhysicalTestResult<serde_json::Value> {
    let bytes = photocraft_format::save_to_bytes(doc, &Default::default())?;
    let restored = photocraft_format::load_from_bytes(&bytes)?;
    let (stream, _) = photocraft_format::PcraftWriter::new().save_zip_stream(
        doc,
        &Default::default(),
        std::io::Cursor::new(Vec::new()),
        Default::default(),
        64 << 20,
        &photocraft_raster::Interrupt::NONE,
    )?;
    let stream_bytes = stream.into_inner();
    let streamed =
        photocraft_format::load_from_stream(std::io::Cursor::new(&stream_bytes), &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE)?;
    if restored != *doc || streamed != *doc {
        return Err("native roundtrip changed document".into());
    }
    Ok(json!({"legacy_bytes":bytes.len(),"zip64_bytes":stream_bytes.len(),"exact_document":true,
        "parsed_tysh_size_pt":match id {Some(id)=>Some(parsed_size(physical_text(&restored,id)?)?),None=>None}}))
}
fn physical_unit() -> photocraft_doc::PhysicalUnit {
    photocraft_doc::PhysicalUnit::Custom(photocraft_doc::CustomUnit {
        name: "Own retag unit".into(),
        factor: 7.25,
        digits: 4,
        symbol: "ru".into(),
        abbreviation: "retag".into(),
    })
}
fn physical_canvas_one(depth: u32) -> PhysicalTestResult<serde_json::Value> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":200,"height":100,"depth":depth,"background":"transparent"}))?;
    s.edit("Physical source", |doc, _| {
        doc.physical_resolution = Some(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 144.0, unit: physical_unit() });
        doc.resolution_dpi = 72.0;
        Ok(())
    })?;
    let dpi = crate::type_cmds::checked_layout_dpi(&s.active().ok_or("no document")?.doc)?;
    if dpi != 144.0 {
        return Err("linked engine lacks authoritative square DPI guard".into());
    }
    let result = s.execute("type.create", json!({"text":"AV","size":24,"x":30,"y":60}))?;
    let id = photocraft_doc::LayerId(result.get("layer").and_then(serde_json::Value::as_u64).ok_or("missing layer ID")?);
    s.edit("Redo witness", |doc, _| {
        doc.name.push_str(" redo");
        Ok(())
    })?;
    s.execute("edit.undo", json!({}))?;
    let before = (*s.active().ok_or("before missing")?.doc).clone();
    let t = physical_text(&before, id)?;
    let cache = t.cache.as_ref().ok_or("source cache absent")?;
    let bounds = cache.content_bounds();
    let source_pixels = cache.try_read_interleaved(bounds)?;
    let before_pt = parsed_size(t)?;
    if before_pt != 24.0 {
        return Err("source TySh not qualified at physical144".into());
    }
    let revision_before = s.active().ok_or("state")?.revision;
    let history_before = s.active().ok_or("state")?.history.entry_labels().map(str::to_owned).collect::<Vec<_>>();
    let redo_before = s.active().ok_or("state")?.history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
    s.execute("image.canvasSize", json!({"width":220,"height":110,"anchor":"center","extensionColor":"transparent"}))?;
    let after = (*s.active().ok_or("after missing")?.doc).clone();
    let at = physical_text(&after, id)?;
    let ac = at.cache.as_ref().ok_or("after cache absent")?;
    let translated = bounds.translate(10, 5);
    if ac.content_bounds() != translated || ac.try_read_interleaved(translated)? != source_pixels {
        return Err("canvas cache translation lost native bits".into());
    }
    if physical_size(at)? != 24.0
        || after.physical_resolution != before.physical_resolution
        || after.resolution_dpi.to_bits() != before.resolution_dpi.to_bits()
    {
        return Err("typed size or physical metadata changed".into());
    }
    let after_pt = parsed_size(at)?;
    let archived = physical_native(&after, Some(id))?;
    if after_pt != 24.0 {
        return Err(format!("Canvas Size retagged 24pt as {after_pt}pt at depth {depth}").into());
    }
    let revision_after = s.active().ok_or("state")?.revision;
    let history_after = s.active().ok_or("state")?.history.entry_labels().map(str::to_owned).collect::<Vec<_>>();
    s.execute("edit.undo", json!({}))?;
    if *s.active().ok_or("undo missing")?.doc != before {
        return Err("undo did not restore source".into());
    }
    s.execute("edit.redo", json!({}))?;
    if *s.active().ok_or("redo missing")?.doc != after {
        return Err("redo did not restore result".into());
    }
    Ok(json!({"depth":depth,"physical_dpi":[144.0,144.0],"legacy_dpi":72,"checked_dpi":dpi,
        "typed_size_pt":24,"tysh_before_pt_at144":before_pt,"tysh_after_pt_at144":after_pt,
        "regression_observed":after_pt!=24.0,"cache_shift_native_exact":true,"physical_metadata_exact":true,
        "history_before":history_before,"history_after":history_after,"redo_before":redo_before,
        "revision_before":revision_before,"revision_after":revision_after,"undo_redo_exact":true,"native":archived}))
}
fn physical_canvas_raster(depth: u32) -> PhysicalTestResult<serde_json::Value> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":200,"height":100,"depth":depth,"background":"transparent"}))?;
    s.edit("Raster unequal physical source", |doc, _| {
        doc.physical_resolution = Some(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 72.0, unit: physical_unit() });
        let layer = doc.layers.first_mut().ok_or_else(|| crate::EngineError::Other("raster layer".into()))?;
        let surface = layer.surface_mut().ok_or_else(|| crate::EngineError::Other("raster surface".into()))?;
        surface.fill_rect(Rect::new(10, 8, 20, 18), &[0.25, 0.5, 0.75, 1.0]);
        Ok(())
    })?;
    let before = (*s.active().ok_or("before")?.doc).clone();
    let pixels = before.layers.first().and_then(|l| l.surface()).ok_or("before raster")?.try_read_interleaved(Rect::new(10, 8, 20, 18))?;
    s.execute("image.canvasSize", json!({"width":220,"height":110,"anchor":"center","extensionColor":"transparent"}))?;
    let after = (*s.active().ok_or("after")?.doc).clone();
    if after.physical_resolution != before.physical_resolution
        || after.layers.first().and_then(|l| l.surface()).ok_or("after raster")?.try_read_interleaved(Rect::new(20, 13, 30, 23))? != pixels
    {
        return Err("raster unequal control changed metadata/native pixels".into());
    }
    Ok(
        json!({"depth":depth,"physical_dpi":[144,72],"registered_canvas_size_succeeded":true,"shifted_native_exact":true,"metadata_exact":true,"native":physical_native(&after,None)?}),
    )
}

#[test]
fn physical_canvas_retag_preserves_authoritative_points_and_native_history() {
    for depth in [8, 16, 32] {
        physical_canvas_one(depth).unwrap();
    }
}
#[test]
fn physical_canvas_raster_unequal_axes_remain_supported() {
    for depth in [8, 16, 32] {
        physical_canvas_raster(depth).unwrap();
    }
}
#[test]
fn physical_canvas_retag_refuses_invalid_axes_atomically() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":200,"height":100,"depth":depth,"background":"transparent"})).unwrap();
        s.execute("type.create", json!({"text":"AV","size":24,"x":30,"y":60})).unwrap();
        for (x, y) in [(0.0, 144.0), (f64::INFINITY, 144.0)] {
            s.edit("physical hostile", |d, _| {
                d.physical_resolution = Some(photocraft_doc::PhysicalResolution { x_dpi: x, y_dpi: y, unit: physical_unit() });
                Ok(())
            })
            .unwrap();
            s.edit("redo witness", |d, _| {
                d.layers[0].name.push('x');
                Ok(())
            })
            .unwrap();
            s.execute("edit.undo", json!({})).unwrap();
            let before = doc(&s).clone();
            let rev = s.active().unwrap().revision;
            let history = s.active().unwrap().history.entry_labels().map(str::to_owned).collect::<Vec<_>>();
            let redo = s.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
            assert!(s.execute("image.canvasSize", json!({"width":220,"height":110,"anchor":"center","extensionColor":"transparent"})).is_err());
            assert_eq!(doc(&s), &before);
            assert_eq!(s.active().unwrap().revision, rev);
            assert_eq!(s.active().unwrap().history.entry_labels().map(str::to_owned).collect::<Vec<_>>(), history);
            assert_eq!(s.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
            let mut direct = before.clone();
            assert!(turn_canvas(&mut direct, Turn::Cw90).is_err());
            assert_eq!(direct, before);
        }
    }
}

#[test]
fn physical_canvas_typed_unequal_axes_keep_cache_transform_tysh_and_native_exact() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":200,"height":100,"depth":depth,"background":"transparent"})).unwrap();
        let id = photocraft_doc::LayerId(s.execute("type.create", json!({"text":"AV","size":24,"x":30,"y":60})).unwrap()["layer"].as_u64().unwrap());
        s.edit("Author unequal typed axes", |d, _| {
            d.physical_resolution = Some(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 72.0, unit: physical_unit() });
            let snapshot = d.clone();
            let LayerContent::Text(t) = &mut d.layer_mut(id).unwrap().content else {
                return Err(crate::EngineError::Other("Expected authored text layer".into()));
            };
            crate::type_cmds::try_refresh(&snapshot, t)?;
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().doc.clone();
        let resolution = before.physical_resolution.as_ref().unwrap();
        let t = physical_text(&before, id).unwrap();
        let cache = t.cache.as_ref().unwrap();
        let bounds = cache.content_bounds();
        let pixels = cache.try_read_interleaved(bounds).unwrap();
        let parsed = photocraft_text::psd::text_layer_from_tysh_with_resolution(t.psd_raw.as_deref().unwrap(), resolution).unwrap().unwrap();
        assert_eq!(physical_size(&parsed).unwrap(), 24.0);
        assert_eq!(parsed.transform, t.transform);
        s.execute("image.canvasSize", json!({"width":220,"height":110,"anchor":"center","extensionColor":"transparent"})).unwrap();
        let after = s.active().unwrap().doc.clone();
        let transformed = physical_text(&after, id).unwrap();
        let shifted = bounds.translate(10, 5);
        assert_eq!(transformed.cache.as_ref().unwrap().content_bounds(), shifted);
        assert_eq!(transformed.cache.as_ref().unwrap().try_read_interleaved(shifted).unwrap(), pixels);
        assert_eq!(transformed.transform, Affine::translate(10.0, 5.0).mul(&t.transform));
        assert_eq!(after.physical_resolution, before.physical_resolution);
        assert_eq!(after.resolution_dpi.to_bits(), before.resolution_dpi.to_bits());
        assert_eq!(physical_size(transformed).unwrap(), 24.0);
        let parsed = photocraft_text::psd::text_layer_from_tysh_with_resolution(transformed.psd_raw.as_deref().unwrap(), resolution).unwrap().unwrap();
        assert_eq!(physical_size(&parsed).unwrap(), 24.0);
        assert_eq!(parsed.transform, transformed.transform);
        physical_native(&after, None).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.shares_state(&before));
        s.execute("edit.redo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.shares_state(&after));
    }
}
#[test]
fn physical_canvas_cache_only_text_does_not_require_scalar_resolution() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":200,"height":100,"depth":depth,"background":"transparent"})).unwrap();
        let id = photocraft_doc::LayerId(s.execute("type.create", json!({"text":"AV","size":24,"x":30,"y":60})).unwrap()["layer"].as_u64().unwrap());
        s.edit("cache-only unequal source", |d, _| {
            d.physical_resolution = Some(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 72.0, unit: physical_unit() });
            let l = d.layer_mut(id).ok_or_else(|| crate::EngineError::Other("text absent".into()))?;
            if let LayerContent::Text(t) = &mut l.content {
                t.psd_raw = None;
            }
            Ok(())
        })
        .unwrap();
        let before = doc(&s).clone();
        let t = physical_text(&before, id).unwrap();
        let cache = t.cache.as_ref().unwrap();
        let bounds = cache.content_bounds();
        let bytes = cache.try_read_interleaved(bounds).unwrap();
        s.execute("image.canvasSize", json!({"width":220,"height":110,"anchor":"center","extensionColor":"transparent"})).unwrap();
        let after = doc(&s);
        let t = physical_text(after, id).unwrap();
        let cache = t.cache.as_ref().unwrap();
        assert!(t.psd_raw.is_none());
        assert_eq!(cache.content_bounds(), bounds.translate(10, 5));
        assert_eq!(cache.try_read_interleaved(bounds.translate(10, 5)).unwrap(), bytes);
        assert_eq!(after.physical_resolution, before.physical_resolution);
        physical_native(after, None).unwrap();
    }
}
#[test]
fn physical_canvas_nested_retag_and_depth_preflight() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":200,"height":100,"background":"transparent"})).unwrap();
    let id = photocraft_doc::LayerId(s.execute("type.create", json!({"text":"AV","size":24,"x":30,"y":60})).unwrap()["layer"].as_u64().unwrap());
    let mut d = doc(&s).clone();
    let layer = d.layers.pop().unwrap();
    d.layers.push(Layer::group("nested", vec![layer]));
    d.physical_resolution = Some(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 144.0, unit: physical_unit() });
    // Rebuild the authored 72dpi block at the authoritative square resolution first.
    let snapshot = d.clone();
    if let LayerContent::Text(t) = &mut d.layer_mut(id).unwrap().content {
        crate::type_cmds::try_refresh(&snapshot, t).unwrap();
    }
    transform_geometry(&mut d, &Affine::translate(10.0, 5.0)).unwrap();
    assert_eq!(parsed_size(physical_text(&d, id).unwrap()).unwrap(), 24.0);
    let mut deep = doc(&s).clone();
    let mut layer = deep.layers.pop().unwrap();
    for _ in 0..=photocraft_doc::MAX_GROUP_DEPTH {
        layer = Layer::group("depth", vec![layer]);
    }
    deep.layers = vec![layer];
    let before = deep.clone();
    assert!(transform_geometry(&mut deep, &Affine::translate(10.0, 5.0)).is_err());
    assert_eq!(deep, before);
    assert!(turn_canvas(&mut deep, Turn::Cw90).is_err());
    assert_eq!(deep, before);
}
