//! Cut Out tool (C): drag across the image to mark a vertical or horizontal band; on release the
//! band is removed and the two sides are joined (`image.cutOut`), like Snagit's Cut Out. The drag
//! shades the band across the whole canvas and shows its width. In Auto (the default) the drag
//! picks the direction: left/right removes a vertical band, up/down a horizontal one. The options
//! bar can fix the direction and picks how the seam is marked (straight, sawtooth or wave).

use egui::{Pos2, Stroke};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;
use crate::theme::Tokens;

/// Cut Out options bar state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CutOutOptions {
    /// "auto" (the drag decides), "vertical" (remove columns) or "horizontal" (remove rows).
    pub direction: String,
    /// "straight" | "sawtooth" | "wave".
    pub edge: String,
    /// Seam colour, straight RGBA.
    pub edge_color: [f32; 4],
}

impl Default for CutOutOptions {
    fn default() -> Self {
        CutOutOptions { direction: "auto".into(), edge: "straight".into(), edge_color: [1.0; 4] }
    }
}

/// Does a drag from `start` to `end` remove a vertical band (columns)? Auto follows the drag:
/// mostly sideways cuts columns, mostly up or down cuts rows.
pub fn vertical(app: &PhotocraftApp, start: [f64; 2], end: [f64; 2]) -> bool {
    match app.ui.tool_options.cut_out.direction.as_str() {
        "vertical" => true,
        "horizontal" => false,
        _ => (end[0] - start[0]).abs() >= (end[1] - start[1]).abs(),
    }
}

/// The band `[from, to)` a drag from `start` to `end` marks, clamped to the canvas (None when
/// under a pixel or covering everything).
pub fn band(app: &PhotocraftApp, start: [f64; 2], end: [f64; 2]) -> Option<(i32, i32)> {
    let size = app.session.active()?.doc.size;
    let (a, b, extent) = if vertical(app, start, end) { (start[0], end[0], size.width) } else { (start[1], end[1], size.height) };
    let clamp = |v: f64| v.round().clamp(0.0, f64::from(extent)) as i32;
    let (from, to) = (clamp(a.min(b)), clamp(a.max(b)));
    (to - from >= 1 && to - from < extent as i32).then_some((from, to))
}

/// The shaded band while dragging, across the whole canvas, with its width beside the pointer.
pub fn draw_preview(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, start: [f64; 2], end: [f64; 2]) {
    let Some((from, to)) = band(app, start, end) else { return };
    let Some(size) = app.session.active().map(|st| st.doc.size) else { return };
    let (w, h) = (size.width as f32, size.height as f32);
    let (from, to) = (from as f32, to as f32);
    let vertical = vertical(app, start, end);
    let corners = if vertical { [(from, 0.0), (to, 0.0), (to, h), (from, h)] } else { [(0.0, from), (w, from), (w, to), (0.0, to)] };
    let pts: Vec<Pos2> = corners.iter().map(|(x, y)| xf.to_screen(*x, *y)).collect();
    let t = Tokens::get(painter.ctx());
    painter.add(egui::Shape::convex_polygon(pts.clone(), t.danger.gamma_multiply(0.28), Stroke::NONE));
    // The two cut lines.
    let (a, b) = if vertical { ([pts[0], pts[3]], [pts[1], pts[2]]) } else { ([pts[0], pts[1]], [pts[3], pts[2]]) };
    for l in [a, b] {
        painter.extend(crate::tool_feedback::contrast_path(l.to_vec(), false, t.danger));
    }
    if let Some(at) = painter.ctx().input(|i| i.pointer.hover_pos()) {
        crate::canvas::draw_value_readout(app, painter.ctx(), "cut-out-readout", at, ["Cut:"], [format!("{} px", (to - from) as i32)]);
    }
}

/// Removes the band a finished drag marked.
pub fn finish(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    let Some((from, to)) = band(app, start, end) else { return };
    let axis = if vertical(app, start, end) { "vertical" } else { "horizontal" };
    let o = &app.ui.tool_options.cut_out;
    let c = o.edge_color;
    let hex = format!(
        "#{:02x}{:02x}{:02x}",
        (c[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[2].clamp(0.0, 1.0) * 255.0).round() as u8
    );
    let p = json!({"axis": axis, "from": from, "to": to, "edge": o.edge, "edgeColor": hex});
    if let Err(e) = app.run("image.cutOut", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Options bar for the Cut Out tool; false for other tools.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if tool != Tool::CutOut {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    let lbl = |ui: &mut egui::Ui, s: &str| {
        ui.label(egui::RichText::new(s).color(t.text_dim).size(12.0));
    };
    let o = &mut app.ui.tool_options.cut_out;
    lbl(ui, tl!("Direction:"));
    let dirs = [("auto".to_string(), "Auto (follow the drag)"), ("vertical".to_string(), "Vertical band"), ("horizontal".to_string(), "Horizontal band")];
    crate::widgets::dropdown(ui, "cut-out-axis", &mut o.direction, &dirs, 170.0);
    crate::widgets::vline(ui, 22.0);
    lbl(ui, tl!("Edge:"));
    let mut edge = o.edge.clone();
    let edges = [("straight".to_string(), "Straight"), ("sawtooth".to_string(), "Sawtooth"), ("wave".to_string(), "Wave")];
    if crate::widgets::dropdown(ui, "cut-out-edge", &mut edge, &edges, 96.0) {
        o.edge = edge;
    }
    let styled = o.edge != "straight";
    let color = o.edge_color;
    ui.add_enabled_ui(styled, |ui| {
        let paint = photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(color[0], color[1], color[2], color[3]));
        if let Some(crate::vector_ui::PaintChange::Color(c)) = crate::vector_ui::swatch(ui, Some(&paint), color, tl!("Edge color")) {
            app.ui.tool_options.cut_out.edge_color = c;
        }
    });
    crate::widgets::vline(ui, 22.0);
    let hint = match app.ui.tool_options.cut_out.direction.as_str() {
        "vertical" => tl!("Drag left or right across the part to remove; the sides join"),
        "horizontal" => tl!("Drag up or down across the part to remove; the sides join"),
        _ => tl!("Drag across the part to remove: sideways cuts a vertical band, up or down a horizontal one"),
    };
    lbl(ui, hint);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 60, "background": "white"})).unwrap();
        app
    }

    #[test]
    fn the_band_follows_the_direction_and_stays_on_the_canvas() {
        let mut app = app();
        app.ui.tool_options.cut_out.direction = "vertical".into();
        assert_eq!(band(&app, [70.4, 5.0], [20.0, 50.0]), Some((20, 70)));
        assert_eq!(band(&app, [-30.0, 5.0], [10.0, 5.0]), Some((0, 10)));
        assert_eq!(band(&app, [0.0, 0.0], [100.0, 0.0]), None, "the whole width");
        assert_eq!(band(&app, [10.0, 0.0], [10.2, 40.0]), None, "under a pixel");
        app.ui.tool_options.cut_out.direction = "horizontal".into();
        assert_eq!(band(&app, [70.0, 5.0], [20.0, 50.0]), Some((5, 50)));
    }

    #[test]
    fn auto_direction_follows_the_drag() {
        let mut app = app();
        assert_eq!(app.ui.tool_options.cut_out.direction, "auto", "the default");
        // Up or down removes rows.
        assert!(!vertical(&app, [50.0, 10.0], [52.0, 40.0]));
        finish(&mut app, [50.0, 10.0], [52.0, 40.0]);
        let size = app.session.active().unwrap().doc.size;
        assert_eq!((size.width, size.height), (100, 30), "a horizontal band of 30 rows");
        // Sideways removes columns.
        assert!(vertical(&app, [10.0, 5.0], [60.0, 9.0]));
        finish(&mut app, [10.0, 5.0], [60.0, 9.0]);
        let size = app.session.active().unwrap().doc.size;
        assert_eq!((size.width, size.height), (50, 30));
    }

    #[test]
    fn a_drag_cuts_the_band_out_with_the_chosen_edge() {
        let mut app = app();
        app.ui.tool_options.cut_out.edge = "sawtooth".into();
        finish(&mut app, [30.0, 10.0], [50.0, 20.0]);
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(doc.size.width, 80);
        assert_eq!(doc.layers.last().unwrap().name, "Cut Out Edge");
    }
}

#[cfg(test)]
mod undo_tests {
    use super::*;

    /// A tool cut over arrow layers is one history step: undo brings the full canvas back.
    #[test]
    fn undo_after_a_tool_cut_restores_the_canvas() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 900, "height": 520, "background": "white"})).unwrap();
        for x in [40, 330, 620] {
            app.run("shape.create", json!({"rect": [x, 60, 240, 380], "fill": "#d8e8f8"})).unwrap();
        }
        app.run("layer.mergeVisible", json!({})).unwrap();
        let g = crate::arrow_ui::geometry([360.0, 120.0], [560.0, 120.0], egui::Modifiers::NONE).unwrap();
        crate::arrow_ui::create(&mut app, &g);
        app.run("shape.edit", json!({"stroke": {"dashes": [0, 2], "cap": "round", "arrowStart": true}})).unwrap();
        let w = |app: &PhotocraftApp| app.session.active().unwrap().doc.size.width;
        finish(&mut app, [300.0, 260.0], [610.0, 270.0]);
        assert_eq!(w(&app), 590);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(w(&app), 900);
    }
}
