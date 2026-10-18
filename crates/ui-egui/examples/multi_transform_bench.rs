//! Release comparison of two single-layer transforms and one shared transform on a 24 MP canvas.
//! Run with `cargo run --release -p photocraft-ui-egui --example multi_transform_bench`.

use std::io::Write;
use std::time::Instant;

use photocraft_doc::{Document, LayerId};
use photocraft_engine::{EngineError, Session};
use photocraft_geom::Rect;
use photocraft_ui_egui::{PhotocraftApp, Services, transform_tool};
use serde_json::{Value, json};

fn command(s: &mut Session, id: &str, params: Value) -> Result<Value, String> {
    s.execute(id, params).map_err(|e| e.to_string())
}

fn source() -> Result<(Document, [LayerId; 2]), String> {
    let mut s = Session::new();
    command(&mut s, "file.new", json!({"width": 6000, "height": 4000, "background": "transparent"}))?;
    let mut add = || -> Result<LayerId, String> {
        command(&mut s, "layer.new.layer", json!({}))?.get("layer").and_then(Value::as_u64).map(LayerId).ok_or_else(|| "no new layer".into())
    };
    let ids = [add()?, add()?];
    s.edit("paint", |doc, _| {
        for (id, rect, color) in [(ids[0], Rect::new(0, 0, 3000, 4000), [0.8, 0.2, 0.1, 1.0]), (ids[1], Rect::new(3000, 0, 6000, 4000), [0.1, 0.3, 0.8, 1.0])] {
            let surface = doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or(EngineError::NoLayer(id))?;
            surface.fill_rect(rect, &color);
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    Ok((s.active().ok_or("no document")?.doc.as_ref().clone(), ids))
}

fn app(doc: &Document) -> PhotocraftApp {
    let mut s = Session::new();
    s.add_document(doc.clone(), None);
    PhotocraftApp::new(s, Services::default())
}

fn move_box(app: &mut PhotocraftApp) -> Result<(), String> {
    let t = app.ui.transform.as_mut().ok_or("no transform")?;
    t.interpolation = "nearest".into();
    t.quad = t.quad.map(|[x, y]| [x + 16.0, y + 8.0]);
    Ok(())
}

fn main() -> Result<(), String> {
    let (doc, ids) = source()?;
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(egui::RawInput { max_texture_side: Some(16384), ..Default::default() }, |_| {});
    let mut output = std::io::stdout().lock();
    for multi in [false, true] {
        let mut elapsed = Vec::new();
        for _ in 0..3 {
            let mut app = app(&doc);
            let before = app.session.active().ok_or("no document")?.history.past_len();
            let start = Instant::now();
            if multi {
                for (i, id) in ids.iter().enumerate() {
                    command(&mut app.session, "layer.select", json!({"layer": id.0, "mode": if i == 0 { "replace" } else { "add" }}))?;
                }
                transform_tool::begin(&mut app, &ctx)?;
                move_box(&mut app)?;
                transform_tool::commit(&mut app);
            } else {
                for id in ids {
                    command(&mut app.session, "layer.select", json!({"layer": id.0}))?;
                    transform_tool::begin(&mut app, &ctx)?;
                    move_box(&mut app)?;
                    transform_tool::commit(&mut app);
                }
            }
            elapsed.push(start.elapsed().as_secs_f64() * 1000.0);
            let st = app.session.active().ok_or("no document")?;
            if app.ui.transform.is_some() || st.history.past_len() != before + if multi { 1 } else { 2 } {
                return Err("transform did not commit the expected history steps".into());
            }
            for (id, expected) in [(ids[0], Rect::new(16, 8, 3016, 4008)), (ids[1], Rect::new(3016, 8, 6016, 4008))] {
                if st.doc.layer(id).and_then(|l| l.surface()).map(|s| s.content_bounds()) != Some(expected) {
                    return Err("transformed pixels differ from the expected result".into());
                }
            }
        }
        elapsed.sort_by(f64::total_cmp);
        writeln!(
            output,
            "{}",
            json!({"workflow": if multi { "shared_frame" } else { "two_single_layer_frames" }, "canvas": [6000, 4000], "samples_ms": elapsed})
        )
        .map_err(|e| e.to_string())?;
    }
    let mut app = app(&doc);
    for (i, id) in ids.iter().enumerate() {
        command(&mut app.session, "layer.select", json!({"layer": id.0, "mode": if i == 0 { "replace" } else { "add" }}))?;
    }
    let start = Instant::now();
    transform_tool::begin(&mut app, &ctx)?;
    let begin_ms = start.elapsed().as_secs_f64() * 1000.0;
    let xf = photocraft_ui_egui::canvas::ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0)),
        zoom: 0.25,
        center: [3000.0, 2000.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    let mut frames = Vec::new();
    for _ in 0..51 {
        move_box(&mut app)?;
        app.ui.transform.as_mut().ok_or("no transform")?.interpolation = "bicubic".into();
        let start = Instant::now();
        let _ = ctx.run_ui(egui::RawInput { max_texture_side: Some(16384), ..Default::default() }, |ui| transform_tool::draw_content(&app, ui.painter(), &xf));
        frames.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let first_ms = frames.remove(0);
    frames.sort_by(f64::total_cmp);
    writeln!(output, "{}", json!({"begin_ms": begin_ms, "first_frame_ms": first_ms, "steady_p50_ms": frames.get(25), "steady_p95_ms": frames.get(47)}))
        .map_err(|e| e.to_string())?;
    Ok(())
}
