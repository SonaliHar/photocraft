//! Select › Sky, Select › Isolate Layers and Select › Transform Selection.
//!
//! * `select.sky` runs the classical sky segmentation of `photocraft_algo::segment::sky` on the
//!   composite (or the active layer) and combines it with the selection.
//! * `select.isolateLayers` toggles the Layers panel's isolation filter: only the layers selected
//!   when it was turned on are listed ([`crate::DocState::isolated_layers`], view state).
//! * `select.transformSelection` transforms the selection outline/mask only (not pixels): a
//!   quad (distort / perspective), an affine matrix, scale / rotate / skew / move about a
//!   reference point, or a warp (same params as `edit.transform.warp`).

use photocraft_doc::{Document, LayerId};
#[cfg(test)]
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn has_selection(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.doc.selection.is_some()).map(|_| ()).ok_or_else(|| "no selection".into())
}
fn has_layer(s: &Session) -> std::result::Result<(), String> {
    s.active().and_then(|d| d.active_layer).map(|_| ()).ok_or_else(|| "no active layer".into())
}
// ---------------------------------------------------------------- Sky

fn select_sky(s: &mut Session, p: &Value) -> Result<Value> {
    crate::selection_sky_controlled::run(s, p)
}

// ---------------------------------------------------------------- Isolate Layers

fn isolate_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(st.isolated_layers.is_empty());
    st.isolated_layers = if on { st.selected_layers()? } else { Vec::new() };
    st.revision += 1;
    Ok(json!({ "isolated": !st.isolated_layers.is_empty(), "layers": st.isolated_layers.iter().map(|l| l.0).collect::<Vec<_>>() }))
}

/// Whether a Layers panel row for `id` is listed under isolation (ancestors of an isolated layer
/// stay visible so it can be shown in place).
pub fn isolation_shows(doc: &Document, isolated: &[LayerId], id: LayerId) -> bool {
    if isolated.is_empty() || isolated.contains(&id) {
        return true;
    }
    fn contains(l: &photocraft_doc::Layer, set: &[LayerId]) -> bool {
        match &l.content {
            photocraft_doc::LayerContent::Group(g) => g.children.iter().any(|c| set.contains(&c.id) || contains(c, set)),
            _ => false,
        }
    }
    doc.layer(id).is_some_and(|l| contains(l, isolated))
}

// ---------------------------------------------------------------- Transform Selection

fn transform_selection(s: &mut Session, p: &Value) -> Result<Value> {
    crate::selection_transform_controlled::run(s, p)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "select.sky",
            label: "Sky",
            menu: &["Select"],
            shortcut: None,
            params: r##"{"mode":"replace|add|subtract|intersect"="replace","sampleAllLayers":bool=true,"threshold":0..100=50 (higher = stricter),"softness":0..100=50 (edge softness),"documentId":string?,"expectedRevision":string?} → {selected, changed, coverage (0–1 of the canvas), bounds [x,y,w,h]}"##,
            enabled: has_doc,
            run: select_sky,
            journal: true,
        },
        CommandSpec {
            id: "select.isolateLayers",
            label: "Isolate Layers",
            menu: &["Select"],
            shortcut: None,
            params: r##"{"on":bool? (default: toggle)} — Layers panel lists only the selected layers → {isolated, layers}"##,
            enabled: has_layer,
            run: isolate_layers,
            journal: false,
        },
        CommandSpec {
            id: "select.transformSelection",
            label: "Transform Selection",
            menu: &["Select"],
            shortcut: None,
            params: r##"{"rect":[x0,y0,x1,y1]? (frame; default = selection bounds),"quad"|"corners":[[x,y]×4]? (where the frame's corners go: distort/perspective),"matrix":[a,b,c,d,e,f]?,"scaleX":%=100,"scaleY":%=100,"rotate":deg=0,"skewX":deg=0,"skewY":deg=0,"dx":px=0,"dy":px=0,"reference":"center|topLeft|top|topRight|left|right|bottomLeft|bottom|bottomRight"|[x,y]="center","style"|"mesh"|"grid"|"warp":… (warp, as edit.transform.warp),"interpolation":"bilinear|bicubic|nearest"="bilinear","documentId":decimal-string?,"expectedRevision":decimal-string? (captured pair)}"##,
            enabled: has_selection,
            run: transform_selection,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel_bounds(s: &Session) -> Option<Rect> {
        s.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds())
    }

    fn landscape(depth: u64) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 160, "height": 100, "depth": depth})).unwrap();
        s.edit("paint", |doc, active| {
            let l = doc.layer_mut(active.unwrap()).unwrap();
            let surf = l.surface_mut().unwrap();
            let n = surf.channels();
            for y in 0..100 {
                for x in 0..160 {
                    let (cx, cy) = (112.0f32, 35.0f32);
                    let crown = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() < 18.0;
                    let nz = (((x * 7919 + y * 104_729) % 97) as f32 / 97.0 - 0.5) * 0.25;
                    let c = if crown {
                        [0.1 + nz * 0.5, 0.35 + nz, 0.1]
                    } else if y < 60 {
                        let t = y as f32 / 60.0;
                        [0.25 + 0.4 * t, 0.5 + 0.3 * t, 0.9 + 0.05 * t]
                    } else {
                        [0.35 + nz, 0.3 + nz, 0.15]
                    };
                    let mut px = vec![1.0f32; n];
                    px[..3].copy_from_slice(&c);
                    surf.write_pixel(x, y, &px);
                }
            }
            Ok(())
        })
        .unwrap();
        s
    }

    #[test]
    fn sky_selects_the_sky_at_every_depth() {
        for depth in [8, 16, 32] {
            let mut s = landscape(depth);
            let r = s.execute("select.sky", json!({})).unwrap();
            assert_eq!(r["changed"], json!(true), "{depth}");
            let cov = r["coverage"].as_f64().unwrap();
            assert!(cov > 0.45 && cov < 0.62, "depth {depth}: coverage {cov}");
            let sel = s.active().unwrap().doc.selection.clone().unwrap();
            assert!(sel.sample_channel(10, 10, 0) > 0.9, "{depth}");
            assert!(sel.sample_channel(10, 90, 0) < 0.1, "{depth}");
            assert!(sel.sample_channel(112, 35, 0) < 0.1, "{depth}: tree");
            s.undo();
            assert!(s.active().unwrap().doc.selection.is_none());
        }
    }

    #[test]
    fn sky_without_sky_changes_nothing() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
        s.execute("edit.fill", json!({"color": "#504020"})).ok();
        let r = s.execute("select.sky", json!({})).unwrap();
        assert_eq!(r["changed"], json!(false));
        assert!(s.active().unwrap().doc.selection.is_none());
    }

    #[test]
    fn isolate_layers_toggles() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 20})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let active = s.active().unwrap().active_layer.unwrap();
        let r = s.execute("select.isolateLayers", json!({})).unwrap();
        assert_eq!(r["isolated"], json!(true));
        assert_eq!(r["layers"], json!([active.0]));
        let d = s.active().unwrap();
        let bg = d.doc.layers[0].id;
        assert!(isolation_shows(&d.doc, &d.isolated_layers, active));
        assert!(!isolation_shows(&d.doc, &d.isolated_layers, bg));
        assert_eq!(crate::inspect::document(d).unwrap()["isolatedLayers"], json!([active.0]));
        let r = s.execute("select.isolateLayers", json!({})).unwrap();
        assert_eq!(r["isolated"], json!(false));
        assert!(s.active().unwrap().isolated_layers.is_empty());
    }

    fn rect_sel() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
        s.execute("select.rect", json!({"x": 20, "y": 30, "width": 20, "height": 10})).unwrap();
        s
    }

    #[test]
    fn transform_selection_scales_rotates_and_moves() {
        let mut s = rect_sel();
        assert_eq!(sel_bounds(&s), Some(Rect::new(20, 30, 40, 40)));
        let px0 = s.active().unwrap().doc.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 100, 100));
        s.execute("select.transformSelection", json!({"scaleX": 200, "scaleY": 200})).unwrap();
        let b = sel_bounds(&s).unwrap();
        assert!((b.x0 - 10).abs() <= 1 && (b.x1 - 50).abs() <= 1 && (b.y0 - 25).abs() <= 1 && (b.y1 - 45).abs() <= 1, "{b:?}");
        // Pixels untouched.
        assert_eq!(s.active().unwrap().doc.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 100, 100)), px0);
        s.undo();
        assert_eq!(sel_bounds(&s), Some(Rect::new(20, 30, 40, 40)));
        s.execute("select.transformSelection", json!({"rotate": 90})).unwrap();
        let b = sel_bounds(&s).unwrap();
        assert!(b.width().abs_diff(10) <= 2 && b.height().abs_diff(20) <= 2, "{b:?}");
        s.undo();
        s.execute("select.transformSelection", json!({"dx": 30, "dy": -10, "reference": "topLeft"})).unwrap();
        assert_eq!(sel_bounds(&s), Some(Rect::new(50, 20, 70, 30)));
    }

    #[test]
    fn transform_selection_perspective_and_warp() {
        let mut s = rect_sel();
        s.execute("select.transformSelection", json!({"corners": [[25, 30], [35, 30], [45, 40], [15, 40]]})).unwrap();
        let sel = s.active().unwrap().doc.selection.clone().unwrap();
        assert!(sel.sample_channel(17, 39, 0) > 0.5 && sel.sample_channel(21, 31, 0) < 0.5);
        s.undo();
        let r = s.execute("select.transformSelection", json!({"style": "arc", "bend": 50})).unwrap();
        assert_eq!(r["changed"], json!(true));
        assert_ne!(sel_bounds(&s), Some(Rect::new(20, 30, 40, 40)));
        assert!(s.execute("select.transformSelection", json!({"scaleX": 0})).is_err());
    }

    #[test]
    fn transform_selection_needs_a_selection() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
        assert!(!s.is_enabled("select.transformSelection"));
    }
}

#[cfg(test)]
mod hang_tests {
    use super::*;

    #[test]
    fn transform_selection_with_extreme_params_terminates() {
        // Regression (panic_hunt): a huge off-canvas transform used to allocate a buffer spanning
        // the old + new bounds and hang. It must return quickly without crashing.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 24, "height": 16})).unwrap();
        s.execute("select.rect", json!({"x": 1, "y": 1, "width": 6, "height": 5})).unwrap();
        // dx/dy move the selection far off-canvas; scaleX enlarges it — the old code allocated a
        // buffer spanning the original + moved bounds (~1e5 × 1e5) and hung.
        let r = s.execute("select.transformSelection", json!({"dx": 1e5, "dy": -1e5, "scaleX": 300.0}));
        assert!(r.is_ok(), "far transform returns without hanging: {r:?}");
        // The document is still usable.
        assert!(s.execute("select.all", json!({})).is_ok());
    }
}
