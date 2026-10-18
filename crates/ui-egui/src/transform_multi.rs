//! Cached stack fragments for layers sharing one Free Transform frame.

use super::*;
use photocraft_engine::EngineError;
use photocraft_geom::Rect;

/// Keep the aggregate CPU images and GPU uploads bounded even for many disjoint targets.
const PREVIEW_TEXELS: usize = 64 * 1024 * 1024;

struct Part {
    ids: Vec<LayerId>,
    moving: bool,
}

struct Piece {
    texture: crate::transform_tex::PreviewTextures,
    rect: Rect,
    moving: bool,
}

pub(super) struct Preview {
    ctx: egui::Context,
    max_side: usize,
    session: u64,
    revision: u64,
    base: Vec<LayerId>,
    pieces: Vec<Piece>,
}

/// Keep whole groups intact unless a selected descendant requires splitting their stack.
fn parts(doc: &Document, ids: &[LayerId]) -> Vec<Part> {
    let walk = doc.walk();
    let targets: Vec<_> = walk.iter().filter(|(_, _, l)| ids.contains(&l.id)).map(|(p, _, _)| p).collect();
    let mut skip: Option<Vec<usize>> = None;
    let mut parts: Vec<Part> = Vec::new();
    for (path, _, layer) in &walk {
        if skip.as_ref().is_some_and(|p| path.starts_with(p)) {
            continue;
        }
        if layer.is_group() && targets.iter().any(|p| p.len() > path.len() && p.starts_with(path)) {
            continue;
        }
        skip = Some(path.clone());
        let moving = ids.contains(&layer.id);
        if let Some(part) = parts.last_mut().filter(|p| p.moving == moving) {
            part.ids.push(layer.id);
        } else {
            parts.push(Part { ids: vec![layer.id], moving });
        }
    }
    parts
}

impl Preview {
    fn new(ctx: &egui::Context, doc: &Document, ids: &[LayerId], frame: Rect, session: u64, revision: u64, max_side: usize) -> Result<Self, String> {
        for id in ids {
            if doc.layer(*id).is_none() {
                return Err(EngineError::NoLayer(*id).to_string());
            }
        }
        let mut parts = parts(doc, ids);
        // The canvas already draws the stack below the lowest moving layer. Keep that
        // fragment in its ordinary compositor instead of making another full-canvas texture.
        let base = if parts.first().is_some_and(|p| !p.moving) { parts.remove(0).ids } else { Vec::new() };
        // A selected raster fragment can leave a second, stationary fragment behind.
        let count = if doc.selection.is_some() { parts.len().saturating_mul(2) } else { parts.len() }.max(1);
        let side = max_side.max(1).min(((PREVIEW_TEXELS / count).max(1) as f64).sqrt().floor() as usize).max(1);
        let mut out = Self { ctx: ctx.clone(), max_side, session, revision, base, pieces: Vec::new() };
        for part in parts {
            let mut solo = doc.clone();
            show_only(&mut solo.layers, &part.ids);
            if part.moving
                && let Some(selection) = &doc.selection
            {
                let mut rest = solo.clone();
                for id in &part.ids {
                    let layer = solo.layer_mut(*id).ok_or_else(|| EngineError::NoLayer(*id).to_string())?;
                    let leftover = rest.layer_mut(*id).ok_or_else(|| EngineError::NoLayer(*id).to_string())?;
                    if matches!(layer.content, LayerContent::Raster(_))
                        && let Some(surface) = layer.surface_mut()
                    {
                        let (lifted, remaining) = photocraft_engine::transform_cmds::split_selected(surface, selection).map_err(|e| e.to_string())?;
                        *surface = lifted;
                        if let Some(surface) = leftover.surface_mut() {
                            *surface = remaining;
                        }
                    } else {
                        leftover.visible = false;
                    }
                }
                out.push(&rest, frame, false, side);
            }
            out.push(&solo, frame, part.moving, side);
        }
        Ok(out)
    }

    fn push(&mut self, doc: &Document, frame: Rect, moving: bool, max_side: usize) {
        // Include off-canvas source pixels so dragging them onto the canvas previews them too.
        let canvas = if moving { doc.bounds().union(&frame) } else { doc.bounds() };
        let rect =
            doc.layers.iter().filter(|l| l.visible).fold(Rect::EMPTY, |a, l| a.union(&photocraft_compose::composite_bounds(l, canvas).unwrap_or(canvas)));
        if rect.is_empty() {
            return;
        }
        let (image, uv) = composite_image(doc, rect, max_side);
        let name = format!("transform-{}-part-{}", self.session, self.pieces.len());
        let texture = crate::transform_tex::PreviewTextures::new(&self.ctx, name, image, uv);
        self.pieces.push(Piece { texture, rect, moving });
    }

    pub(super) fn draw(&self, painter: &egui::Painter, xf: &ViewXform, t: &TransformSession, canvas: [f32; 2]) {
        let Some(h) = Homography::rect_to_quad(t.rect, t.quad) else { return };
        let zoom = xf.zoom * painter.ctx().pixels_per_point();
        let front = h.front_side(t.rect);
        for piece in &self.pieces {
            let scale = if piece.moving { quad_scale(t) } else { 1.0 };
            let (texture, uv) = piece.texture.pick(painter.ctx(), zoom, scale, piece.moving && t.interpolation == "nearest");
            let mut mesh = egui::Mesh::with_texture(texture);
            let n = if piece.moving && (h.0[6].abs() > 1e-12 || h.0[7].abs() > 1e-12) { 24 } else { 1 };
            let mut ahead = Vec::with_capacity((n + 1) * (n + 1));
            for y in 0..=n {
                for x in 0..=n {
                    let (u, v) = (x as f64 / n as f64, y as f64 / n as f64);
                    let source = (f64::from(piece.rect.x0) + u * f64::from(piece.rect.width()), f64::from(piece.rect.y0) + v * f64::from(piece.rect.height()));
                    let (x, y) = if piece.moving { h.apply(source.0, source.1) } else { source };
                    let valid = (!piece.moving || front * h.w(source.0, source.1) > 1e-9) && (x as f32).is_finite() && (y as f32).is_finite();
                    ahead.push(valid);
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: if valid { pos2(x as f32, y as f32) } else { Pos2::ZERO },
                        uv: pos2(u as f32 * uv[0], v as f32 * uv[1]),
                        color: Color32::WHITE,
                    });
                }
            }
            for y in 0..n {
                for x in 0..n {
                    let a = (y * (n + 1) + x) as u32;
                    for triangle in [[a, a + 1, a + n as u32 + 2], [a, a + n as u32 + 2, a + n as u32 + 1]] {
                        if triangle.iter().all(|k| ahead.get(*k as usize).copied().unwrap_or(false)) {
                            mesh.add_triangle(triangle[0], triangle[1], triangle[2]);
                        }
                    }
                }
            }
            painter.add(clip_to_canvas(mesh, canvas, |p| xf.to_screen(p.x, p.y)));
        }
    }
}

fn background(doc: &Document, keep: &[LayerId]) -> Arc<Document> {
    let mut doc = doc.clone();
    show_only(&mut doc.layers, keep);
    Arc::new(doc)
}

pub(super) fn begin(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    doc: Arc<Document>,
    ids: Vec<LayerId>,
    active: LayerId,
    b: Rect,
    revision: u64,
) -> Result<(), String> {
    crate::type_tool::commit(app);
    let session = app.ui.alloc_id();
    let max_side = ctx.input(|i| i.max_texture_side).max(app.gpu.as_ref().map_or(1, |g| g.max_texture_side()));
    let multi = Preview::new(ctx, &doc, &ids, b, session, revision, max_side)?;
    let background = background(&doc, &multi.base);
    let texture =
        crate::transform_tex::PreviewTextures::new(ctx, format!("transform-{session}"), egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]), [1.0, 1.0]);
    app.transform_preview = Some(TransformPreview {
        session,
        doc: background,
        texture,
        opacity: 1.0,
        multi: Some(multi),
        above: None,
        styled: None,
        gesture: None,
        warp_drag: None,
        split_tool: None,
        split_pointer: None,
        split_placing: false,
        split_quick: false,
        steps: Steps::default(),
        tool: app.ui.tool,
        path: None,
        follow: None,
        committing: None,
        content_aware: None,
    });
    let rect = [f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)];
    app.ui.transform = Some(TransformSession {
        session,
        layer: active.0,
        layers: ids.iter().map(|id| id.0).collect(),
        rect,
        quad: corners(rect),
        pivot: [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0],
        interpolation: "bicubic".into(),
        warp: None,
        selection: false,
        target: None,
        path: None,
        made: None,
        mode: Default::default(),
        content_aware: None,
        skew_v: 0.0,
    });
    start_steps(app);
    Ok(())
}

pub(super) fn follow_edits(app: &mut PhotocraftApp) -> bool {
    let (Some(t), Some(pv)) = (&app.ui.transform, app.transform_preview.as_mut()) else { return false };
    let Some(multi) = pv.multi.as_mut() else { return false };
    let Some(st) = app.session.active().filter(|st| st.doc.id == pv.doc.id && st.revision != multi.revision) else { return true };
    let ids: Vec<LayerId> = t.layers.iter().copied().map(LayerId).collect();
    let frame = Rect::new(t.rect[0] as i32, t.rect[1] as i32, t.rect[2] as i32, t.rect[3] as i32);
    match Preview::new(&multi.ctx, &st.doc, &ids, frame, multi.session, st.revision, multi.max_side) {
        Ok(preview) => {
            pv.doc = background(&st.doc, &preview.base);
            *multi = preview;
        }
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scene() -> (PhotocraftApp, LayerId, LayerId, LayerId) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 128, "height": 96})).unwrap();
        let mut add = |name| LayerId(app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap());
        let a = add("A");
        let middle = add("Unselected");
        let b = add("B");
        app.session
            .edit("paint", |doc, _| {
                for (id, rect, color) in [
                    (a, Rect::new(10, 16, 40, 44), [1.0, 0.0, 0.0, 1.0]),
                    (middle, Rect::new(24, 16, 48, 44), [0.0, 1.0, 0.0, 1.0]),
                    (b, Rect::new(32, 16, 64, 44), [0.0, 0.0, 1.0, 1.0]),
                ] {
                    doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(rect, &color);
                }
                Ok(())
            })
            .unwrap();
        app.run("layer.select", json!({"layer": a.0})).unwrap();
        app.run("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
        app.sync_views();
        (app, a, middle, b)
    }

    fn bounds(app: &PhotocraftApp, id: LayerId) -> Rect {
        app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds()
    }

    fn translate(app: &mut PhotocraftApp, dx: f64) {
        let t = app.ui.transform.as_mut().unwrap();
        t.interpolation = "nearest".into();
        t.quad = t.quad.map(|[x, y]| [x + dx, y]);
    }

    #[test]
    fn multi_frame_commits_its_frozen_selection_in_one_undo_step() {
        let (mut app, a, middle, b) = scene();
        let ctx = egui::Context::default();
        let before = app.session.active().unwrap().history.past_len();
        crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
        let t = app.ui.transform.as_ref().unwrap();
        assert_eq!(t.rect, [10.0, 16.0, 64.0, 44.0]);
        assert_eq!(t.layers, vec![a.0, b.0]);
        translate(&mut app, 8.0);
        assert_eq!(bounds(&app, a), Rect::new(10, 16, 40, 44), "preview does not edit pixels");
        app.run("layer.select", json!({"layer": middle.0})).unwrap();
        super::super::follow_edits(&mut app);
        commit(&mut app);
        assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
        assert_eq!(bounds(&app, a), Rect::new(18, 16, 48, 44));
        assert_eq!(bounds(&app, b), Rect::new(40, 16, 72, 44));
        assert_eq!(bounds(&app, middle), Rect::new(24, 16, 48, 44));
        assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(bounds(&app, a), Rect::new(10, 16, 40, 44));
        assert_eq!(bounds(&app, b), Rect::new(32, 16, 64, 44));
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(bounds(&app, a), Rect::new(18, 16, 48, 44));
        assert_eq!(bounds(&app, b), Rect::new(40, 16, 72, 44));
    }

    #[test]
    fn cancel_and_locked_nonactive_targets_leave_the_document_untouched() {
        let (mut app, a, _, b) = scene();
        let ctx = egui::Context::default();
        let before = app.session.active().unwrap().history.past_len();
        super::super::begin(&mut app, &ctx).unwrap();
        translate(&mut app, 8.0);
        cancel(&mut app);
        assert_eq!(bounds(&app, a), Rect::new(10, 16, 40, 44));
        assert_eq!(bounds(&app, b), Rect::new(32, 16, 64, 44));
        assert_eq!(app.session.active().unwrap().history.past_len(), before);
        app.session
            .edit("lock", |doc, _| {
                doc.layer_mut(a).unwrap().locks.position = true;
                Ok(())
            })
            .unwrap();
        assert_eq!(super::super::begin(&mut app, &ctx), Err(LOCKED.into()));
        assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
    }

    #[test]
    fn a_selected_group_carries_its_active_child_once() {
        let (mut app, a, _, b) = scene();
        let group = LayerId(app.run("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
        app.run("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
        assert_ne!(is_enabled(&app, "edit.transform.warp"), Some(false), "a group and its child are one effective target");
        super::super::begin(&mut app, &egui::Context::default()).unwrap();
        let t = app.ui.transform.as_ref().unwrap();
        assert_eq!(t.layer, group.0);
        assert_eq!(t.layers, vec![group.0]);
        assert_eq!(t.rect, [10.0, 16.0, 64.0, 44.0]);
        translate(&mut app, 8.0);
        commit(&mut app);
        assert_eq!(bounds(&app, a), Rect::new(18, 16, 48, 44));
        assert_eq!(bounds(&app, b), Rect::new(40, 16, 72, 44));
    }

    #[test]
    fn multi_frame_does_not_silently_warp_only_the_active_layer() {
        let (mut app, _, _, _) = scene();
        let ctx = egui::Context::default();
        assert!(begin_warp(&mut app, &ctx).is_err());
        assert!(app.ui.transform.is_none());
        super::super::begin(&mut app, &ctx).unwrap();
        assert_eq!(is_enabled(&app, "edit.transform.warp"), Some(false));
        enter_warp(&mut app, &ctx);
        assert!(app.ui.transform.as_ref().unwrap().warp.is_none());
    }

    #[test]
    fn multi_preview_keeps_unselected_layers_between_the_moving_layers() {
        let (app, a, middle, b) = scene();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(128.0, 96.0)).wgpu().build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if app.ui.transform.is_none() {
                    super::super::begin(app, ui.ctx()).unwrap();
                }
                let xf = ViewXform {
                    rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(128.0, 96.0)),
                    zoom: 1.0,
                    center: [64.0, 48.0],
                    flip: false,
                    rotation: 0.0,
                    aspect: 1.0,
                };
                ui.painter().rect_filled(xf.rect, 0.0, Color32::WHITE);
                draw_content(app, ui.painter(), &xf);
            },
            app,
        );
        h.run_steps(4);
        translate(h.state_mut(), 8.0);
        h.run_steps(2);
        let image = h.render().unwrap();
        let mut oracle = photocraft_engine::Session::new();
        oracle.add_document(h.state().session.active().unwrap().doc.as_ref().clone(), None);
        oracle.execute("edit.transform", json!({"layers": [a.0, b.0], "dx": 8, "interpolation": "nearest"})).unwrap();
        let expected = photocraft_compose::render(&oracle.active().unwrap().doc, Rect::new(0, 0, 128, 96)).to_rgba8();
        for y in 20..40 {
            for x in 12..80 {
                let actual = image.get_pixel(x, y).0;
                let offset = (y as usize * 128 + x as usize) * 4;
                let expected = &expected.pixels[offset..offset + 4];
                assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(*b) <= 1), "at {x},{y}: {actual:?} != {expected:?}");
            }
        }
        assert_eq!(bounds(h.state(), middle), Rect::new(24, 16, 48, 44));
        assert_eq!(bounds(h.state(), a), Rect::new(10, 16, 40, 44));
    }
}
