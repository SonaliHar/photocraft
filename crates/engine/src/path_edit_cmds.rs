//! Point-level path edits, what Photoshop's Direct Selection (A), Convert Point and Add / Delete
//! Anchor Point tools do on the canvas: move anchors, drag direction handles, bend segments,
//! convert points, and insert or remove anchors. The
//! geometry is `photocraft_vector::edit`; the canvas previews a drag with it and commits the
//! whole drag as one command (one history step).
//!
//! Every command targets a path like `path.transform`: `"name":"work"` (default), a saved path's
//! name, or `"layer"` with `"layer":id?` for a shape layer's path (re-rendered; it stops being a
//! live shape) or a layer's vector mask. Knots are addressed by `subpath` and `knot` index, as
//! listed by `path.info`, and the result is `{name, path}` in the compact JSON form.

use photocraft_doc::{LayerContent, Path};
use photocraft_vector::edit;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::vector_cmds::{bad, has_doc, layer_id, path_json, path_mut, with_shape};
use crate::{Result, Session};

mod controlled;

/// Called before normalization and parameter copying for the four controlled edits.
pub(crate) fn dispatch_preflight(s: &Session, command: &str, p: &Value, background: bool) -> Result<()> {
    controlled::dispatch_preflight(s, command, p, background)
}

fn index(cmd: &str, p: &Value, key: &str) -> Result<usize> {
    p.get(key).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(cmd, format!("missing `{key}` index")))
}

fn knot(cmd: &str, p: &Value) -> Result<[usize; 2]> {
    Ok([index(cmd, p, "subpath")?, index(cmd, p, "knot")?])
}

/// Applies `f` to the targeted path in one undoable step labelled `label`.
fn edit_path(s: &mut Session, cmd: &str, p: &Value, label: &str, f: impl FnOnce(&mut Path) -> std::result::Result<(), String>) -> Result<Value> {
    let saved_id = crate::vector_cmds::saved_path_id(p).map_err(|error| bad(cmd, error.to_string()))?;
    let name = if let Some(id) = saved_id {
        s.active()
            .ok_or(crate::EngineError::NoDocument)?
            .doc
            .paths
            .iter()
            .find(|path| path.id == id)
            .ok_or_else(|| bad(cmd, "no saved path with pathId"))?
            .name
            .clone()
    } else {
        p.get("name").and_then(Value::as_str).unwrap_or("work").to_owned()
    };
    let layer = if saved_id.is_none() && name == "layer" { Some(layer_id(s, p)?) } else { None };
    let shape = layer.filter(|id| s.active().and_then(|d| d.doc.layer(*id)).is_some_and(|l| matches!(l.content, LayerContent::Shape(_))));
    let path = match shape {
        Some(id) => with_shape(s, id, label, |sh, _| {
            f(&mut sh.path).map_err(|e| bad(cmd, e))?;
            sh.live = None;
            sh.psd_raw = None;
            Ok(path_json(&sh.path))
        })?,
        None => s.edit(label, |doc, _| {
            let path = if let Some(id) = saved_id {
                &mut doc.paths.iter_mut().find(|path| path.id == id).ok_or_else(|| bad(cmd, "no saved path with pathId"))?.path
            } else {
                path_mut(doc, &name, layer)?
            };
            f(path).map_err(|e| bad(cmd, e))?;
            Ok(path_json(path))
        })?,
    };
    Ok(json!({ "name": name, "pathId":saved_id.map(|id|id.0.to_string()), "path": path }))
}

fn move_anchors(s: &mut Session, p: &Value) -> Result<Value> {
    controlled::run(s, "path.moveAnchors", p)
}
fn move_handle(s: &mut Session, p: &Value) -> Result<Value> {
    controlled::run(s, "path.moveHandle", p)
}
fn bend_segment(s: &mut Session, p: &Value) -> Result<Value> {
    controlled::run(s, "path.bendSegment", p)
}
fn convert_point(s: &mut Session, p: &Value) -> Result<Value> {
    controlled::run(s, "path.convertPoint", p)
}

fn add_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.addAnchor";
    let k = knot(CMD, p)?;
    let t = match p.get("t") {
        Some(v) => v.as_f64().filter(|t| *t > 0.0 && *t < 1.0).ok_or_else(|| bad(CMD, "`t` must be strictly between 0 and 1"))?,
        None => 0.5,
    };
    let mut added = None;
    let mut r = edit_path(s, CMD, p, "Add Anchor Point", |path| {
        added = Some(edit::add_anchor(path, k, t)?);
        Ok(())
    })?;
    if let (Some(o), Some([sp, kn])) = (r.as_object_mut(), added) {
        o.insert("anchor".into(), json!({"subpath": sp, "knot": kn}));
    }
    Ok(r)
}

fn delete_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.deleteAnchor";
    let k = knot(CMD, p)?;
    let mut removed = false;
    let mut r = edit_path(s, CMD, p, "Delete Anchor Point", |path| {
        removed = edit::delete_anchor(path, k)?;
        Ok(())
    })?;
    if let Some(o) = r.as_object_mut() {
        o.insert("subpathRemoved".into(), json!(removed));
    }
    Ok(r)
}

pub fn specs() -> Vec<CommandSpec> {
    const TARGET: &str = r#""pathId":decimal string|u64? (saved path identity takes precedence),"name":str|"work"|"layer"="work","layer":id? (with "layer": a shape layer's path or a vector mask),"documentId":canonical decimal string?,"expectedRevision":canonical decimal string? (paired captured target for moveAnchors/moveHandle/bendSegment/convertPoint)"#;
    let spec = |id: &'static str, label: &'static str, params: String, run: fn(&mut Session, &Value) -> Result<Value>| CommandSpec {
        id,
        label,
        menu: &[],
        shortcut: None,
        params: crate::vector_cmds::leak(format!("{{{TARGET},{params}}} → {{name,pathId:decimal string|null,path}}")),
        enabled: has_doc,
        run,
        journal: true,
    };
    vec![
        spec("path.moveAnchors", "Drag Anchor Points", r#""anchors":[[subpath,knot],…],"move":[dx,dy],"curvature":bool=false (anchors move with their handles; Direct Selection drag. curvature = the Curvature Pen's drag: every smooth knot of the touched subpaths gets its handles recomputed so the curve flows through the anchors)"#.into(), move_anchors),
        spec(
            "path.moveHandle",
            "Drag Direction Point",
            r#""subpath":i,"knot":j,"handle":"in|out","to":[x,y],"independent":bool=false (a smooth point turns its other handle to stay collinear; independent = ⌥ / Convert Point, makes a corner)"#.into(),
            move_handle,
        ),
        spec(
            "path.bendSegment",
            "Drag Segment",
            r#""subpath":i,"knot":j (the segment leaving it),"t":0..1=0.5 (where it was grabbed),"move":[dx,dy] (a straight segment moves with its anchors; a curve reshapes through the dragged point)"#.into(),
            bend_segment,
        ),
        spec(
            "path.convertPoint",
            "Convert Point",
            r#""subpath":i,"knot":j,"out":[x,y]? (none: corner with retracted handles; given: smooth with this out handle and its mirror),"curvature":bool=false (the Curvature Pen's double-click: toggles smooth/corner and reflows the smooth knots of the subpath; out ignored)"#.into(),
            convert_point,
        ),
        spec(
            "path.addAnchor",
            "Add Anchor Point",
            r#""subpath":i,"knot":j (the segment leaving it),"t":0<t<1=0.5 (where on the segment) (a curve is split there without changing its shape and gets a smooth anchor; a straight segment gets a corner) (the result also has anchor:{subpath,knot})"#.into(),
            add_anchor,
        ),
        spec(
            "path.deleteAnchor",
            "Delete Anchor Point",
            r#""subpath":i,"knot":j (its neighbours are joined, keeping their handles; a subpath's last anchor removes the subpath; the path's only anchor is refused) (the result also has subpathRemoved:bool)"#.into(),
            delete_anchor,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use photocraft_doc::{LayerId, ShapeLayer};
    use serde_json::json;

    use crate::Session;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 120, "height": 100, "background": "white"})).unwrap();
        s
    }

    fn square() -> serde_json::Value {
        json!({"subpaths": [{"closed": true, "knots": [[10, 10], [60, 10], [60, 60], [10, 60]]}]})
    }

    fn anchor(v: &serde_json::Value, k: usize) -> [f64; 2] {
        serde_json::from_value(v["path"]["subpaths"][0]["knots"][k]["anchor"].clone()).unwrap()
    }

    fn work(s: &Session) -> photocraft_doc::Path {
        s.active().unwrap().doc.work_path.clone().unwrap()
    }

    #[test]
    fn anchors_move_in_one_undoable_step() {
        let mut s = session();
        s.execute("path.set", json!({"path": square()})).unwrap();
        let before = work(&s);
        let r = s.execute("path.moveAnchors", json!({"anchors": [[0, 1], [0, 2]], "move": [5, -3]})).unwrap();
        assert_eq!((anchor(&r, 1), anchor(&r, 2), anchor(&r, 0)), ([65.0, 7.0], [65.0, 57.0], [10.0, 10.0]));
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Drag Anchor Points"));
        assert!(s.undo());
        assert_eq!(work(&s), before);
    }

    #[test]
    fn a_curvature_anchor_drag_reflows_the_smooth_knots() {
        let mut s = session();
        let smooth = |a: [f64; 2]| json!({"anchor": a, "in": a, "out": a, "smooth": true});
        let path = json!({"subpaths": [{"closed": true, "knots": [smooth([0.0, 50.0]), smooth([100.0, 0.0]), smooth([200.0, 50.0]), smooth([100.0, 100.0])]}]});
        s.execute("path.set", json!({"path": path})).unwrap();
        s.execute("path.moveAnchors", json!({"anchors": [[0, 1]], "move": [0, 60], "curvature": true})).unwrap();
        let p = work(&s);
        let pts = [[0.0, 50.0], [100.0, 60.0], [200.0, 50.0], [100.0, 100.0]];
        let want = photocraft_vector::edit::curvature_knots(&pts, &[false; 4], true);
        for (k, w) in p.subpaths[0].knots.iter().zip(want) {
            assert_eq!([[k.anchor.x, k.anchor.y], [k.in_ctrl.x, k.in_ctrl.y], [k.out_ctrl.x, k.out_ctrl.y]], w);
        }
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Drag Anchor Points"));
        assert!(s.execute("path.moveAnchors", json!({"anchors": [[0, 9]], "move": [1, 0], "curvature": true})).is_err());
    }

    #[test]
    fn a_curvature_convert_toggles_the_point_and_reflows() {
        let mut s = session();
        s.execute("path.set", json!({"path": square()})).unwrap();
        let before = work(&s);
        s.execute("path.convertPoint", json!({"subpath": 0, "knot": 1, "curvature": true})).unwrap();
        let k = work(&s).subpaths[0].knots[1];
        assert!(k.smooth && k.in_ctrl != k.anchor && k.out_ctrl != k.anchor, "{k:?}");
        s.execute("path.convertPoint", json!({"subpath": 0, "knot": 1, "curvature": true})).unwrap();
        assert_eq!(work(&s), before);
        assert!(s.execute("path.convertPoint", json!({"subpath": 0, "knot": 9, "curvature": true})).is_err());
    }

    #[test]
    fn a_coalesced_drag_is_one_history_step() {
        let mut s = session();
        s.execute("path.set", json!({"path": square()})).unwrap();
        let steps = s.active().unwrap().history.entries(&crate::read_control_none()).unwrap().values().to_vec().len();
        for _ in 0..3 {
            s.execute("path.moveAnchors", json!({"anchors": [[0, 0]], "move": [1, 0], "coalesce": "drag-1"})).unwrap();
        }
        assert_eq!(s.active().unwrap().history.entries(&crate::read_control_none()).unwrap().values().to_vec().len(), steps + 1);
        assert_eq!(work(&s).subpaths[0].knots[0].anchor.x, 13.0);
    }

    #[test]
    fn handles_segments_and_convert_point() {
        let mut s = session();
        s.execute("path.set", json!({"path": square()})).unwrap();
        // Pull smooth handles out of the corner at (60, 10), then turn one: the other follows.
        s.execute("path.convertPoint", json!({"subpath": 0, "knot": 1, "out": [70, 10]})).unwrap();
        let r = s.execute("path.moveHandle", json!({"subpath": 0, "knot": 1, "handle": "out", "to": [60, 20]})).unwrap();
        let k = &r["path"]["subpaths"][0]["knots"][1];
        assert_eq!((k["in"].clone(), k["smooth"].clone()), (json!([60.0, 0.0]), json!(true)));
        let r = s.execute("path.moveHandle", json!({"subpath": 0, "knot": 1, "handle": "in", "to": [50, 0], "independent": true})).unwrap();
        let k = &r["path"]["subpaths"][0]["knots"][1];
        assert_eq!((k["out"].clone(), k["smooth"].clone()), (json!([60.0, 20.0]), json!(false)));
        // The straight bottom edge (2 → 3) drags with its anchors.
        let r = s.execute("path.bendSegment", json!({"subpath": 0, "knot": 2, "move": [0, 8]})).unwrap();
        assert_eq!((anchor(&r, 2), anchor(&r, 3)), ([60.0, 68.0], [10.0, 68.0]));
        let r = s.execute("path.convertPoint", json!({"subpath": 0, "knot": 1})).unwrap();
        assert_eq!(r["path"]["subpaths"][0]["knots"][1]["out"], json!([60.0, 10.0]));
    }

    #[test]
    fn shape_paths_and_vector_masks_are_edited_and_rerendered() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 120, "height": 100, "depth": depth})).unwrap();
            let id = s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 40], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
            let r = s.execute("path.moveAnchors", json!({"name": "layer", "layer": id, "anchors": [[0, 1], [0, 2]], "move": [30, 0]})).unwrap();
            assert_eq!(anchor(&r, 1), [80.0, 10.0]);
            let st = s.active().unwrap();
            let photocraft_doc::LayerContent::Shape(ShapeLayer { live, cache, .. }) = &st.doc.layer(LayerId(id)).unwrap().content else { panic!("shape") };
            assert!(live.is_none(), "an edited rectangle is a plain path now");
            assert_eq!(cache.as_ref().unwrap().content_bounds().x1, 80, "depth {depth}: re-rendered");
        }
        let mut s = session();
        // Background has a real position lock. Keep this positive vector-mask
        // geometry case on an actual ordinary unlocked pixel layer.
        s.execute("layer.new.layer", json!({"name":"Own unlocked mask owner"})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap().0;
        s.execute("layer.vectorMask.add", json!({"layer": id, "path": square()})).unwrap();
        s.execute("path.moveAnchors", json!({"name": "layer", "layer": id, "anchors": [[0, 0]], "move": [-5, -5]})).unwrap();
        let vm = s.active().unwrap().doc.layer(LayerId(id)).unwrap().vector_mask.clone().unwrap();
        assert_eq!(vm.path.subpaths[0].knots[0].anchor, photocraft_geom::Point::new(5.0, 5.0));
    }

    #[test]
    fn position_locked_background_vector_mask_point_edit_is_atomic() {
        let mut s = session();
        let id = s.active().unwrap().active_layer.unwrap();
        assert!(s.active().unwrap().doc.effective_locks(id).position);
        s.execute("layer.vectorMask.add", json!({"layer":id.0,"path":square()})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let past = s.active().unwrap().history.past_len();
        let result = s.execute("path.moveAnchors", json!({"name":"layer","layer":id.0,"anchors":[[0,0]],"move":[-5,-5]}));
        assert!(result.unwrap_err().to_string().contains("vector owner is locked"));
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.active().unwrap().history.past_len(), past);
    }

    fn curve() -> serde_json::Value {
        json!({"subpaths": [{"closed": false, "knots": [
            {"anchor": [10, 80], "in": [10, 80], "out": [10, 20]},
            {"anchor": [60, 50], "in": [40, 20], "out": [80, 80], "smooth": true},
            {"anchor": [110, 80], "in": [110, 20], "out": [110, 80]}
        ]}]})
    }

    fn knots(s: &Session) -> usize {
        work(s).subpaths.iter().map(|sp| sp.knots.len()).sum()
    }

    #[test]
    fn anchors_are_added_and_deleted_in_one_undoable_step_each() {
        let mut s = session();
        s.execute("path.set", json!({"path": curve()})).unwrap();
        let before = work(&s);
        let steps = s.active().unwrap().history.entry_labels().map(str::to_owned).collect::<Vec<_>>().len();
        let r = s.execute("path.addAnchor", json!({"subpath": 0, "knot": 0, "t": 0.5})).unwrap();
        assert_eq!(r["anchor"], json!({"subpath": 0, "knot": 1}));
        assert_eq!((knots(&s), s.active().unwrap().history.entry_labels().map(str::to_owned).collect::<Vec<_>>().len()), (4, steps + 1));
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
        assert!(work(&s).subpaths[0].knots[1].smooth);
        let added = work(&s);
        assert!(s.undo());
        assert_eq!(work(&s), before, "one undo restores the path exactly");
        assert!(s.redo());
        assert_eq!(work(&s), added);
        let r = s.execute("path.deleteAnchor", json!({"subpath": 0, "knot": 1})).unwrap();
        assert_eq!(r["subpathRemoved"], json!(false));
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
        assert_eq!(knots(&s), 3);
        // Deleting the middle anchor of the original curve joins the ends with their own handles.
        s.execute("path.set", json!({"path": curve()})).unwrap();
        s.execute("path.deleteAnchor", json!({"subpath": 0, "knot": 1})).unwrap();
        let k = &work(&s).subpaths[0].knots;
        assert_eq!((k.len(), k[0].out_ctrl, k[1].in_ctrl), (2, before.subpaths[0].knots[0].out_ctrl, before.subpaths[0].knots[2].in_ctrl));
    }

    #[test]
    fn anchor_tools_edit_saved_paths_shapes_and_vector_masks() {
        let mut s = session();
        s.execute("path.set", json!({"name": "Outline", "path": square()})).unwrap();
        let r = s.execute("path.deleteAnchor", json!({"name": "Outline", "subpath": 0, "knot": 0})).unwrap();
        assert_eq!((r["path"]["subpaths"][0]["knots"].as_array().unwrap().len(), r["path"]["subpaths"][0]["closed"].clone()), (3, json!(true)));
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 120, "height": 100, "depth": depth})).unwrap();
            let id = s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 40], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
            // Top edge, then pull the new midpoint up: the layer re-renders from the edited path.
            let r = s.execute("path.addAnchor", json!({"name": "layer", "layer": id, "subpath": 0, "knot": 0, "t": 0.5})).unwrap();
            assert_eq!(anchor(&r, 1), [30.0, 10.0]);
            s.execute("path.moveAnchors", json!({"name": "layer", "layer": id, "anchors": [[0, 1]], "move": [0, -8]})).unwrap();
            let st = s.active().unwrap();
            let photocraft_doc::LayerContent::Shape(ShapeLayer { live, cache, .. }) = &st.doc.layer(LayerId(id)).unwrap().content else { panic!("shape") };
            assert!(live.is_none());
            assert_eq!(cache.as_ref().unwrap().content_bounds().y0, 2, "depth {depth}: re-rendered");
            s.execute("path.deleteAnchor", json!({"name": "layer", "layer": id, "subpath": 0, "knot": 1})).unwrap();
            let st = s.active().unwrap();
            let photocraft_doc::LayerContent::Shape(ShapeLayer { cache, path, .. }) = &st.doc.layer(LayerId(id)).unwrap().content else { panic!("shape") };
            assert_eq!((path.subpaths[0].knots.len(), cache.as_ref().unwrap().content_bounds().y0), (4, 10), "depth {depth}");
        }
        let id = s.active().unwrap().active_layer.unwrap().0;
        s.execute("layer.vectorMask.add", json!({"layer": id, "path": square()})).unwrap();
        s.execute("path.addAnchor", json!({"name": "layer", "layer": id, "subpath": 0, "knot": 3, "t": 0.5})).unwrap();
        let vm = s.active().unwrap().doc.layer(LayerId(id)).unwrap().vector_mask.clone().unwrap();
        assert_eq!(vm.path.subpaths[0].knots[4].anchor, photocraft_geom::Point::new(10.0, 35.0), "the closing segment's midpoint");
    }

    #[test]
    fn the_last_anchor_of_a_shape_is_refused() {
        let mut s = session();
        let one = json!({"subpaths": [{"closed": false, "knots": [[10, 10], [50, 10]]}]});
        let id = s.execute("shape.create", json!({"kind": "path", "path": one, "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
        s.execute("path.deleteAnchor", json!({"name": "layer", "layer": id, "subpath": 0, "knot": 0})).unwrap();
        let steps = s.active().unwrap().history.entry_labels().map(str::to_owned).collect::<Vec<_>>().len();
        let e = s.execute("path.deleteAnchor", json!({"name": "layer", "layer": id, "subpath": 0, "knot": 0})).unwrap_err();
        assert!(e.to_string().contains("only anchor"), "{e}");
        assert_eq!(s.active().unwrap().history.entry_labels().map(str::to_owned).collect::<Vec<_>>().len(), steps);
    }

    #[test]
    fn bad_params_fail_without_an_edit() {
        let mut s = session();
        assert!(s.execute("path.moveAnchors", json!({"anchors": [[0, 0]], "move": [1, 1]})).is_err(), "no work path");
        s.execute("path.set", json!({"path": square()})).unwrap();
        let steps = s.active().unwrap().history.entries(&crate::read_control_none()).unwrap().values().to_vec().len();
        for (id, p) in [
            ("path.moveAnchors", json!({"anchors": [[0, 9]], "move": [1, 1]})),
            ("path.moveAnchors", json!({"anchors": [], "move": [1, 1]})),
            ("path.moveAnchors", json!({"anchors": [[0]], "move": [1, 1]})),
            ("path.moveAnchors", json!({"anchors": [[0, 0]], "move": [1e300, 1]})),
            ("path.moveAnchors", json!({"anchors": [[0, 0]]})),
            ("path.moveHandle", json!({"subpath": 0, "knot": 0, "handle": "up", "to": [1, 1]})),
            ("path.moveHandle", json!({"subpath": 0, "knot": 0, "handle": "in"})),
            ("path.moveHandle", json!({"subpath": -1, "knot": 0, "handle": "in", "to": [1, 1]})),
            ("path.bendSegment", json!({"subpath": 0, "knot": 0, "t": 2, "move": [1, 1]})),
            ("path.bendSegment", json!({"subpath": 5, "knot": 0, "move": [1, 1]})),
            ("path.convertPoint", json!({"subpath": 0, "knot": 4})),
            ("path.convertPoint", json!({"subpath": 0, "knot": 0, "out": "x"})),
            ("path.convertPoint", json!({"name": "nope", "subpath": 0, "knot": 0})),
            ("path.convertPoint", json!({"name": "layer", "subpath": 0, "knot": 0})),
            ("path.addAnchor", json!({"subpath": 0, "knot": 0, "t": 0})),
            ("path.addAnchor", json!({"subpath": 0, "knot": 0, "t": 1})),
            ("path.addAnchor", json!({"subpath": 0, "knot": 0, "t": -0.5})),
            ("path.addAnchor", json!({"subpath": 0, "knot": 0, "t": "half"})),
            ("path.addAnchor", json!({"subpath": 0, "knot": 4})),
            ("path.addAnchor", json!({"subpath": 1, "knot": 0})),
            ("path.addAnchor", json!({"knot": 0})),
            ("path.addAnchor", json!({"name": "nope", "subpath": 0, "knot": 0})),
            ("path.deleteAnchor", json!({"subpath": 0, "knot": 4})),
            ("path.deleteAnchor", json!({"subpath": 0, "knot": -1})),
            ("path.deleteAnchor", json!({"subpath": 0})),
            ("path.deleteAnchor", json!({"name": "layer", "subpath": 0, "knot": 0})),
        ] {
            assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
        }
        assert_eq!(s.active().unwrap().history.entries(&crate::read_control_none()).unwrap().values().to_vec().len(), steps);
    }
}
