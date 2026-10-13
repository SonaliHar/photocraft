//! View extras: rulers (⌘R), grid (⌘'), guides (⌘;). Guides are dragged out of the rulers and
//! moved with the Move tool (drag one off the canvas to delete it), all through undoable engine
//! commands (`view.newGuide` / `view.moveGuide` / `view.deleteGuide`).

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use photocraft_doc::Document;
use photocraft_engine::prefs::Unit;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::theme::Tokens;

pub const RULER: f32 = 16.0;

/// A `#rrggbb` preference colour.
fn pref_color(s: &str, fallback: Color32) -> Color32 {
    photocraft_engine::prefs::parse_hex(s).map_or(fallback, |c| Color32::from_rgb(c[0], c[1], c[2]))
}

/// Draw a line in a Guides/Grid preference style (lines, dashed lines, dots).
fn styled_line(painter: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke, style: photocraft_engine::prefs::LineStyle) {
    use photocraft_engine::prefs::LineStyle;
    match style {
        LineStyle::Lines => {
            painter.line_segment([a, b], stroke);
        }
        LineStyle::Dashed => {
            painter.extend(egui::Shape::dashed_line(&[a, b], stroke, 4.0, 3.0));
        }
        LineStyle::Dots => {
            painter.extend(egui::Shape::dotted_line(&[a, b], stroke.color, 3.0, 0.6));
        }
    }
}

/// Ruler step in units so labelled ticks are at least `min` units apart: …0.1, 0.25, 0.5, 1, 2, 5,
/// 10… (whole steps only for pixels).
fn nice_step(min: f64, whole: bool) -> f64 {
    let mut step = if whole { 1.0 } else { 0.001 };
    loop {
        for m in [1.0, 2.0, 2.5, 5.0] {
            if (m != 2.5 || !whole) && step * m >= min {
                return step * m;
            }
        }
        step *= 10.0;
    }
}

/// English display name for a length unit, as shown in Preferences › Units & Rulers.
fn unit_label(u: Unit) -> &'static str {
    match u {
        Unit::Pixels => "Pixels",
        Unit::Inches => "Inches",
        Unit::Centimeters => "Centimeters",
        Unit::Millimeters => "Millimeters",
        Unit::Points => "Points",
        Unit::Picas => "Picas",
        Unit::Percent => "Percent",
    }
}

/// Every length unit, in the Preferences › Units & Rulers order.
const UNITS: [Unit; 7] = [Unit::Pixels, Unit::Inches, Unit::Centimeters, Unit::Millimeters, Unit::Points, Unit::Picas, Unit::Percent];

/// The ruler right-click menu: every length unit with its (translated) label and whether it's the
/// one in effect. Picking one changes the ruler unit exactly as the preference pane does.
fn unit_menu_items(current: Unit) -> Vec<(String, &'static str, bool)> {
    UNITS.iter().map(|&u| (tl!(unit_label(u)).to_string(), u.name(), u == current)).collect()
}

/// Right-clicking a ruler offers the length units; the chosen one's name is returned (`None` if the
/// menu is dismissed without a pick).
fn ruler_unit_menu(ui: &mut egui::Ui, current: Unit) -> Option<&'static str> {
    crate::widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(150.0);
        let mut chosen = None;
        for (label, name, checked) in unit_menu_items(current) {
            let label = if checked { format!("✓ {label}") } else { label };
            if ui.button(label).clicked() {
                ui.close();
                chosen = Some(name);
            }
        }
        chosen
    })
}

/// A guide being dragged: from a ruler (new) or an existing one (index).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideDrag {
    pub vertical: bool,
    pub index: Option<usize>,
    pub pos: f64,
}

/// Canvas area left after the rulers.
pub fn content_rect(app: &PhotocraftApp, rect: Rect) -> Rect {
    if app.ui.extras.rulers { Rect::from_min_max(rect.min + vec2(RULER, RULER), rect.max) } else { rect }
}

fn vertical_label(painter: &egui::Painter, center_x: f32, mut y: f32, text: &str, font: &egui::FontId, color: Color32) {
    for ch in text.chars() {
        let galley = painter.layout_no_wrap(ch.to_string(), font.clone(), color);
        let height = galley.size().y;
        painter.galley(pos2(center_x - galley.size().x * 0.5, y), galley, color);
        y += height;
    }
}

/// Tick step (document px) so major ticks are at least ~60 screen px apart: 1, 2, 5, 10, 20, 50…
#[cfg(test)]
fn tick_step(zoom: f32) -> f64 {
    nice_step(60.0 / zoom.max(1e-4) as f64, true)
}

pub(crate) fn physical_grid_steps(doc: &Document, g: &photocraft_engine::prefs::GuidesGridAndSlices, ppi: f64) -> Option<[f64; 2]> {
    let legacy = photocraft_doc::PhysicalResolution::square(f64::from(doc.resolution_dpi));
    let resolution = doc.physical_resolution.as_ref().unwrap_or(&legacy);
    resolution.validate().ok()?;
    Some([g.major_px(resolution.x_dpi, doc.size.width as f64, ppi), g.major_px(resolution.y_dpi, doc.size.height as f64, ppi)])
}

/// View › Show › Grid, from Preferences › Guides, Grid & Slices (spacing, subdivisions, colour,
/// style). Photoshop's default is a gridline every inch with 4 subdivisions.
pub fn draw_grid(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, doc: &Document) {
    let g = &app.session.prefs().guides_grid_and_slices;
    let ppi = app.session.prefs().units_and_rulers.point_size.per_inch();
    let Some(majors) = physical_grid_steps(doc, g, ppi) else {
        return;
    };
    let base = pref_color(&g.grid_color, Color32::from_gray(140));
    let (w, h) = (doc.size.width as f64, doc.size.height as f64);
    let clip = painter.clip_rect();
    let corners = [xf.to_doc(clip.left_top()), xf.to_doc(clip.right_top()), xf.to_doc(clip.left_bottom()), xf.to_doc(clip.right_bottom())];
    if corners.iter().flatten().any(|v| !v.is_finite()) {
        return;
    }
    let visible = [
        (
            corners.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min).clamp(0.0, w),
            corners.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max).clamp(0.0, w),
        ),
        (
            corners.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min).clamp(0.0, h),
            corners.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max).clamp(0.0, h),
        ),
    ];
    for (vertical, major, extent) in [(true, majors[0], w), (false, majors[1], h)] {
        for (step, alpha) in [(major / g.subdivisions.max(1) as f64, 0.45f32), (major, 1.0)] {
            if !step.is_finite() || step <= 0.0 || step * (xf.zoom as f64) < 6.0 || (alpha < 1.0 && g.subdivisions <= 1) {
                continue;
            }
            let st = Stroke::new(1.0, base.gamma_multiply(alpha));
            let (start, end) = if vertical { visible[0] } else { visible[1] };
            let mut at = (start / step).ceil().max(1.0) * step;
            let mut count = 0usize;
            while at < extent.min(end) && count < 4096 {
                if vertical {
                    styled_line(painter, xf.to_screen(at as f32, 0.0), xf.to_screen(at as f32, h as f32), st, g.grid_style);
                } else {
                    styled_line(painter, xf.to_screen(0.0, at as f32), xf.to_screen(w as f32, at as f32), st, g.grid_style);
                }
                let next = at + step;
                if next <= at {
                    break;
                }
                at = next;
                count += 1;
            }
        }
    }
}

pub fn draw_guides(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, doc: &Document) {
    let clip = painter.clip_rect();
    let drag = app.guide_drag;
    let g = &app.session.prefs().guides_grid_and_slices;
    let (guide, style) = (pref_color(&g.guide_color, Color32::from_rgb(74, 255, 255)), g.guide_style);
    let line = |vertical: bool, pos: f64, color: Color32| {
        if vertical {
            let x = xf.to_screen(pos as f32, 0.0).x;
            styled_line(painter, pos2(x, clip.top()), pos2(x, clip.bottom()), Stroke::new(1.0, color), style);
        } else {
            let y = xf.to_screen(0.0, pos as f32).y;
            styled_line(painter, pos2(clip.left(), y), pos2(clip.right(), y), Stroke::new(1.0, color), style);
        }
    };
    if app.ui.extras.guides {
        for (vertical, list) in [(true, &doc.guides.vertical), (false, &doc.guides.horizontal)] {
            for (i, p) in list.iter().enumerate() {
                if drag.is_some_and(|d| d.vertical == vertical && d.index == Some(i)) {
                    continue;
                }
                line(vertical, *p as f64, guide);
            }
        }
    }
    if let Some(d) = drag {
        line(d.vertical, d.pos, guide);
    }
}

/// Existing guide under a document point (within 4 screen px).
pub fn guide_at(app: &PhotocraftApp, x: f64, y: f64) -> Option<(bool, usize)> {
    // Hidden canvas guides are not interactive: the Move tool must reach the layer below.
    // Keep hit testing consistent with the visibility gate in canvas::draw.
    if !app.ui.extras.guides || !app.ui.view.shows(app.ui.view.show.canvas_guides) || app.ui.extras.lock_guides {
        return None;
    }
    let doc = &app.session.active()?.doc;
    let tol = 4.0 / app.current_zoom().max(0.01) as f64;
    for (i, g) in doc.guides.vertical.iter().enumerate() {
        if (*g as f64 - x).abs() <= tol {
            return Some((true, i));
        }
    }
    for (i, g) in doc.guides.horizontal.iter().enumerate() {
        if (*g as f64 - y).abs() <= tol {
            return Some((false, i));
        }
    }
    None
}

/// Finish a guide drag: create, move, or delete (dropped outside the canvas).
pub fn finish_drag(app: &mut PhotocraftApp, d: GuideDrag) {
    let Some(size) = app.session.active().map(|s| s.doc.size) else { return };
    let extent = if d.vertical { size.width } else { size.height } as f64;
    let orientation = if d.vertical { "vertical" } else { "horizontal" };
    let inside = d.pos >= 0.0 && d.pos <= extent;
    let pos = (d.pos * 1000.0).round() / 1000.0;
    let _ = match (d.index, inside) {
        (None, true) => app.run("view.newGuide", json!({"orientation": orientation, "position": pos})),
        (Some(i), true) => app.run("view.moveGuide", json!({"orientation": orientation, "index": i, "position": pos})),
        (Some(i), false) => app.run("view.deleteGuide", json!({"orientation": orientation, "index": i})),
        (None, false) => Ok(serde_json::Value::Null),
    };
    if d.index.is_none() && inside {
        app.ui.extras.guides = true;
    }
}

/// Rulers along the top and left of `full` (the canvas rect before `content_rect`), with the
/// pointer position marked; dragging out of a ruler creates a guide.
pub fn draw_rulers(app: &mut PhotocraftApp, ui: &mut egui::Ui, full: Rect, xf: &ViewXform) {
    let t = Tokens::get(ui.ctx());
    let top = Rect::from_min_max(pos2(full.left() + RULER, full.top()), pos2(full.right(), full.top() + RULER));
    let left = Rect::from_min_max(pos2(full.left(), full.top() + RULER), pos2(full.left() + RULER, full.bottom()));
    let corner = Rect::from_min_size(full.min, vec2(RULER, RULER));
    let p = ui.painter_at(full);
    let bg = t.chrome;
    for r in [top, left, corner] {
        p.rect_filled(r, 0.0, bg);
    }
    p.line_segment([top.left_bottom(), top.right_bottom()], Stroke::new(1.0, t.separator));
    p.line_segment([left.right_top(), left.right_bottom()], Stroke::new(1.0, t.separator));
    let font = egui::FontId::proportional(9.0);
    let tick = Stroke::new(1.0, t.text_faint);
    // Preferences › Units & Rulers: labels in the ruler unit (percent of the document's side).
    let ur = &app.session.prefs().units_and_rulers;
    let (unit, ppi) = (ur.rulers, ur.point_size.per_inch());
    let (dpi, size) = if let Some(d) = app.session.active() {
        let legacy = photocraft_doc::PhysicalResolution::square(f64::from(d.doc.resolution_dpi));
        let resolution = d.doc.physical_resolution.as_ref().unwrap_or(&legacy);
        if resolution.validate().is_err() {
            return;
        }
        ([resolution.x_dpi, resolution.y_dpi], [d.doc.size.width as f64, d.doc.size.height as f64])
    } else {
        ([72.0, 72.0], [1.0, 1.0])
    };
    let whole = unit == photocraft_engine::prefs::Unit::Pixels;
    let label = |v: f64, step: f64| -> String { if step >= 1.0 || whole { format!("{}", v.round() as i64) } else { crate::widgets::fmt_num2(v) } };
    for (vertical, extent, axis_dpi) in [(false, size[0], dpi[0]), (true, size[1], dpi[1])] {
        let px_per_unit = unit.to_px(1.0, axis_dpi, extent, ppi).max(1e-9);
        if !px_per_unit.is_finite() {
            continue;
        }
        let step = nice_step(60.0 / (xf.zoom as f64 * px_per_unit).max(1e-6), whole);
        let (d0, d1) = if vertical {
            (xf.to_doc(left.left_top())[1], xf.to_doc(left.left_bottom())[1])
        } else {
            (xf.to_doc(top.left_top())[0], xf.to_doc(top.right_top())[0])
        };
        let (u0, u1) = (d0 / px_per_unit, d1 / px_per_unit);
        let mut v = (u0 / step).floor() * step;
        let mut count = 0usize;
        while v <= u1 && count < 4096 {
            let at = |u: f64| -> f32 {
                let px = (u * px_per_unit) as f32;
                if vertical { xf.to_screen(0.0, px).y } else { xf.to_screen(px, 0.0).x }
            };
            let c = at(v);
            if vertical {
                p.line_segment([pos2(left.right() - RULER, c), pos2(left.right(), c)], tick);
                // Use the actual font height so decimal points and minus signs
                // do not overlap adjacent glyphs in the narrow vertical ruler.
                vertical_label(&p, left.center().x, c + 2.0, &label(v, step), &font, t.text_dim);
            } else {
                p.line_segment([pos2(c, top.bottom() - RULER), pos2(c, top.bottom())], tick);
                p.text(pos2(c + 2.0, top.top() + 1.0), Align2::LEFT_TOP, label(v, step), font.clone(), t.text_dim);
            }
            for k in 1..10 {
                let m = at(v + step * k as f64 / 10.0);
                let len = if k == 5 { 6.0 } else { 3.0 };
                if vertical {
                    p.line_segment([pos2(left.right() - len, m), pos2(left.right(), m)], tick);
                } else {
                    p.line_segment([pos2(m, top.bottom() - len), pos2(m, top.bottom())], tick);
                }
            }
            let next = v + step;
            if next <= v {
                break;
            }
            v = next;
            count += 1;
        }
    }
    // Pointer position markers.
    if let Some(h) = ui.ctx().pointer_hover_pos().filter(|h| full.contains(*h)) {
        let m = Stroke::new(1.0, t.text);
        p.line_segment([pos2(h.x, top.top()), pos2(h.x, top.bottom())], m);
        p.line_segment([pos2(left.left(), h.y), pos2(left.right(), h.y)], m);
    }
    // Drag a new guide out of a ruler; right-click to switch the ruler unit.
    let mut chosen_unit: Option<&'static str> = None;
    for (r, vertical, salt) in [(top, false, "ruler-top"), (left, true, "ruler-left"), (corner, false, "ruler-corner")] {
        // Guides drag out of the top/left rulers; all three sense clicks so the right-click unit
        // menu opens anywhere on the ruler (the corner only needs the menu).
        let sense = if salt == "ruler-corner" { Sense::click() } else { Sense::click_and_drag() };
        let resp = ui.interact(r, ui.id().with(salt), sense);
        if resp.hovered() && salt != "ruler-corner" {
            ui.ctx().set_cursor_icon(if vertical { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
        }
        if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.drag_started()) {
            let d = xf.to_doc(pos);
            let pos = crate::snap_ui::snap_guide(app, vertical, if vertical { d[0] } else { d[1] });
            app.guide_drag = Some(GuideDrag { vertical, index: None, pos });
        }
        if resp.drag_stopped()
            && let Some(d) = app.guide_drag.take()
        {
            finish_drag(app, d);
        }
        resp.context_menu(|ui| {
            if let Some(name) = ruler_unit_menu(ui, unit) {
                chosen_unit = Some(name);
            }
        });
    }
    if let Some(name) = chosen_unit {
        let _ = app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": name}));
    }
    let _ = Pos2::ZERO;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_decimal_glyphs_fit_ruler_and_do_not_overlap() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            vertical_label(ui.painter(), 8., 20., "0.25", &egui::FontId::proportional(9.), Color32::WHITE);
        });
        output.textures_delta.clear();
        let glyphs: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| if let egui::Shape::Text(t) = &s.shape { Some((t.pos, t.galley.size(), t.galley.text())) } else { None })
            .collect();
        assert_eq!(glyphs.iter().map(|v| v.2).collect::<Vec<_>>(), ["0", ".", "2", "5"]);
        for (pos, size, _) in &glyphs {
            assert!(pos.x >= 0. && pos.x + size.x <= RULER);
        }
        for pair in glyphs.windows(2) {
            assert!(pair[0].0.y + pair[0].1.y <= pair[1].0.y);
        }
    }

    #[test]
    fn unit_steps_allow_fractions() {
        assert_eq!(nice_step(0.3, false), 0.5);
        assert_eq!(nice_step(0.2, false), 0.2);
        assert_eq!(nice_step(0.22, false), 0.25);
        assert_eq!(nice_step(0.3, true), 1.0);
        assert_eq!(nice_step(23.0, true), 50.0);
    }

    #[test]
    fn ticks_follow_1_2_5_series() {
        assert_eq!(tick_step(1.0), 100.0);
        assert_eq!(tick_step(0.1), 1000.0);
        assert_eq!(tick_step(4.0), 20.0);
        assert_eq!(tick_step(64.0), 1.0);
        assert_eq!(tick_step(0.25), 500.0);
    }

    #[test]
    fn hidden_guides_are_not_hit_tested() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.sync_views();
        app.session.execute("view.newGuide", json!({"orientation": "vertical", "position": 50})).unwrap();
        app.session.execute("view.newGuide", json!({"orientation": "horizontal", "position": 25})).unwrap();
        let positions = app.session.active().unwrap().doc.guides.clone();

        // Visible, unlocked guides remain interactive in either orientation.
        assert_eq!(guide_at(&app, 50.0, 10.0), Some((true, 0)));
        assert_eq!(guide_at(&app, 10.0, 25.0), Some((false, 0)));

        // All ways to hide the canvas guides must also disable hit testing.
        app.ui.extras.guides = false;
        assert_eq!(guide_at(&app, 50.0, 10.0), None);
        assert_eq!(guide_at(&app, 10.0, 25.0), None);
        app.ui.extras.guides = true;

        app.ui.view.show.canvas_guides = false;
        assert_eq!(guide_at(&app, 50.0, 10.0), None);
        assert_eq!(guide_at(&app, 10.0, 25.0), None);
        app.ui.view.show.canvas_guides = true;

        app.ui.view.extras = false;
        assert_eq!(guide_at(&app, 50.0, 10.0), None);
        assert_eq!(guide_at(&app, 10.0, 25.0), None);
        app.ui.view.extras = true;

        // Hiding/showing must not change the stored guide positions.
        assert_eq!(app.session.active().unwrap().doc.guides, positions);
        assert_eq!(guide_at(&app, 50.0, 10.0), Some((true, 0)));
        assert_eq!(guide_at(&app, 10.0, 25.0), Some((false, 0)));

        app.ui.extras.lock_guides = true;
        assert_eq!(guide_at(&app, 50.0, 10.0), None);
        assert_eq!(guide_at(&app, 10.0, 25.0), None);
    }

    #[test]
    fn unit_menu_lists_every_unit_with_the_current_one_checked() {
        let items = unit_menu_items(Unit::Inches);
        assert_eq!(items.len(), Unit::NAMES.len());
        // Every unit is offered, labelled, and in the preference's order.
        assert_eq!(items[0], ("Pixels".to_string(), "pixels", false));
        assert_eq!(items.iter().find(|(_, name, _)| *name == "cm").unwrap().0, "Centimeters");
        // Exactly the current unit is checked.
        let checked: Vec<_> = items.iter().filter(|(_, _, c)| *c).map(|(_, name, _)| *name).collect();
        assert_eq!(checked, vec!["inches"]);
    }

    #[test]
    fn picking_a_ruler_unit_changes_the_preference() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert_eq!(app.session.prefs().units_and_rulers.rulers, Unit::Pixels);
        // The context menu applies its pick exactly as the preference pane does.
        app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "cm"})).unwrap();
        assert_eq!(app.session.prefs().units_and_rulers.rulers, Unit::Centimeters);
    }

    #[test]
    fn guide_drags_create_move_and_delete() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.sync_views();
        finish_drag(&mut app, GuideDrag { vertical: true, index: None, pos: 50.0 });
        finish_drag(&mut app, GuideDrag { vertical: false, index: None, pos: 500.0 }); // off canvas: ignored
        assert_eq!(app.session.active().unwrap().doc.guides.vertical, vec![50.0]);
        assert!(app.session.active().unwrap().doc.guides.horizontal.is_empty());
        assert_eq!(guide_at(&app, 51.0, 10.0), Some((true, 0)));
        finish_drag(&mut app, GuideDrag { vertical: true, index: Some(0), pos: 120.0 });
        assert_eq!(app.session.active().unwrap().doc.guides.vertical, vec![120.0]);
        finish_drag(&mut app, GuideDrag { vertical: true, index: Some(0), pos: -30.0 });
        assert!(app.session.active().unwrap().doc.guides.vertical.is_empty());
        app.ui.extras.lock_guides = true;
        assert_eq!(guide_at(&app, 0.0, 0.0), None);
    }
    #[test]
    fn physical_grid_and_ruler_units_measure_each_axis() {
        let mut doc = Document::new("axes", photocraft_doc::Size::new(144, 288), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        doc.set_physical_resolution(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: photocraft_doc::PhysicalUnit::Inch }).unwrap();
        doc.resolution_dpi = 999.0;
        let mut grid = photocraft_engine::prefs::GuidesGridAndSlices { gridline_every: 1.0, grid_unit: Unit::Inches, ..Default::default() };
        assert_eq!(physical_grid_steps(&doc, &grid, 72.0), Some([144.0, 288.0]));
        let r = doc.effective_resolution();
        assert_eq!(Unit::Inches.to_px(1.0, r.x_dpi, 144.0, 72.0), 144.0);
        assert_eq!(Unit::Inches.to_px(1.0, r.y_dpi, 288.0, 72.0), 288.0);
        grid.grid_unit = Unit::Percent;
        grid.gridline_every = 50.0;
        assert_eq!(physical_grid_steps(&doc, &grid, 72.0), Some([72.0, 144.0]));
        doc.physical_resolution.as_mut().unwrap().y_dpi = f64::NAN;
        assert_eq!(physical_grid_steps(&doc, &grid, 72.0), None);
    }
}

#[cfg(test)]
mod physical_resolution_borrow_tests {
    use super::*;
    fn unit() -> photocraft_doc::PhysicalUnit {
        photocraft_doc::PhysicalUnit::Custom(photocraft_doc::CustomUnit {
            name: "Own".into(),
            factor: 7.25,
            digits: 4,
            symbol: "u".into(),
            abbreviation: "own".into(),
        })
    }
    fn resolution(x: f64, y: f64) -> photocraft_doc::PhysicalResolution {
        photocraft_doc::PhysicalResolution { x_dpi: x, y_dpi: y, unit: unit() }
    }

    #[test]
    fn grid_custom_legacy_unequal_tiny_and_invalid_controls() {
        let mut d = Document::new("borrow", photocraft_doc::Size::new(144, 288), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let mut g = photocraft_engine::prefs::GuidesGridAndSlices { gridline_every: 1.0, grid_unit: Unit::Inches, ..Default::default() };
        d.resolution_dpi = 999.0;
        d.physical_resolution = Some(resolution(144.0, 288.0));
        let before = d.clone();
        assert_eq!(physical_grid_steps(&d, &g, 72.0), Some([144.0, 288.0]));
        assert_eq!(d, before);
        g.grid_unit = Unit::Percent;
        g.gridline_every = 50.0;
        assert_eq!(physical_grid_steps(&d, &g, 72.0), Some([72.0, 144.0]));
        g.grid_unit = Unit::Inches;
        g.gridline_every = 1.0;
        d.physical_resolution = Some(resolution(0.5, 0.25));
        assert_eq!(physical_grid_steps(&d, &g, 72.0), Some([0.5, 0.25]));
        for y in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            d.physical_resolution = Some(resolution(144.0, y));
            assert_eq!(physical_grid_steps(&d, &g, 72.0), None);
        }
        for name in [String::new(), "bad\0name".into(), "x".repeat((1 << 20) + 1)] {
            let mut r = resolution(144.0, 288.0);
            if let photocraft_doc::PhysicalUnit::Custom(u) = &mut r.unit {
                u.name = name;
            }
            d.physical_resolution = Some(r);
            assert_eq!(physical_grid_steps(&d, &g, 72.0), None);
        }
        d.physical_resolution = None;
        d.resolution_dpi = 72.0;
        assert_eq!(physical_grid_steps(&d, &g, 72.0), Some([72.0, 72.0]));
    }
}
