//! Frozen point edits. Shape regeneration retains its legacy boundary-only cancellation seam.
use crate::vector_cmds::{bad, layer_id, saved_path_id};
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_doc::{
    Document, LayerContent, LayerId, Path, Subpath,
    item_identity::{ItemRef, PathId},
    xcf_items::EditableSource,
};
use photocraft_geom::Point;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use photocraft_vector::edit::{self, Handle};
use serde_json::{Value, json};
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| EngineError::Cancelled)
}
fn error(cmd: &str, c: &Interrupt<'_>, e: impl std::fmt::Display) -> EngineError {
    if c.cancelled() { EngineError::Cancelled } else { bad(cmd, e.to_string()) }
}
#[derive(Clone, Copy)]
enum Target {
    Work,
    Saved(PathId),
    Layer(LayerId, bool),
}
#[derive(Clone, Copy)]
enum Operation<'a> {
    Anchors(&'a [Value], [f64; 2]),
    CurvatureAnchors(&'a [Value], [f64; 2]),
    CurvatureConvert([usize; 2]),
    Handle([usize; 2], Handle, Point, bool),
    Bend([usize; 2], f64, [f64; 2]),
    Convert([usize; 2], Option<Point>),
}
enum OwnedOperation {
    Anchors(Vec<[usize; 2]>, [f64; 2]),
    CurvatureAnchors(Vec<[usize; 2]>, [f64; 2]),
    CurvatureConvert([usize; 2]),
    Handle([usize; 2], Handle, Point, bool),
    Bend([usize; 2], f64, [f64; 2]),
    Convert([usize; 2], Option<Point>),
}
fn decimal(v: &Value, nonzero: bool, cmd: &str) -> Result<u64> {
    let s = v
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad(cmd, "canonical captured IDs required"))?;
    let n = s.parse::<u64>().map_err(|_| bad(cmd, "ID overflow"))?;
    if nonzero && n == 0 {
        return Err(bad(cmd, "zero ID"));
    }
    Ok(n)
}
fn pair(v: &Value, cmd: &str) -> Result<[usize; 2]> {
    let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad(cmd, "anchor requires exactly two indices"))?;
    let n = |i| {
        a.get(i).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).filter(|n| *n < 100000).ok_or_else(|| bad(cmd, "bounded integer index required"))
    };
    Ok([n(0)?, n(1)?])
}
fn point(v: &Value, cmd: &str) -> Result<Point> {
    let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad(cmd, "point requires exactly two coordinates"))?;
    let n =
        |i| a.get(i).and_then(Value::as_f64).filter(|n| n.is_finite() && n.abs() <= 1e6).ok_or_else(|| bad(cmd, "finite coordinate within 1000000 required"));
    Ok(Point::new(n(0)?, n(1)?))
}
fn knot(p: &Value, cmd: &str) -> Result<[usize; 2]> {
    let n = |key| {
        p.get(key)
            .and_then(Value::as_u64)
            .and_then(|v| usize::try_from(v).ok())
            .filter(|v| *v < 100000)
            .ok_or_else(|| bad(cmd, "bounded knot indices required"))
    };
    Ok([n("subpath")?, n("knot")?])
}
fn operation<'a>(cmd: &str, p: &'a Value) -> Result<Operation<'a>> {
    let movement = || point(p.get("move").ok_or_else(|| bad(cmd, "missing move"))?, cmd).map(|v| [v.x, v.y]);
    Ok(match cmd {
        "path.moveAnchors" => {
            let a = p
                .get("anchors")
                .and_then(Value::as_array)
                .filter(|a| !a.is_empty() && a.len() <= 100000)
                .ok_or_else(|| bad(cmd, "bounded nonempty anchors required"))?;
            for v in a {
                pair(v, cmd)?;
            }
            if p.get("curvature").and_then(Value::as_bool).unwrap_or(false) {
                Operation::CurvatureAnchors(a, movement()?)
            } else {
                Operation::Anchors(a, movement()?)
            }
        }
        "path.moveHandle" => Operation::Handle(
            knot(p, cmd)?,
            p.get("handle").and_then(Value::as_str).and_then(Handle::parse).ok_or_else(|| bad(cmd, "handle must be in or out"))?,
            point(p.get("to").ok_or_else(|| bad(cmd, "missing to"))?, cmd)?,
            p.get("independent").map(|v| v.as_bool().ok_or_else(|| bad(cmd, "independent must be boolean"))).transpose()?.unwrap_or(false),
        ),
        "path.bendSegment" => Operation::Bend(
            knot(p, cmd)?,
            p.get("t")
                .map(|v| v.as_f64().filter(|v| v.is_finite() && (0.0..=1.0).contains(v)).ok_or_else(|| bad(cmd, "t must be finite in 0..1")))
                .transpose()?
                .unwrap_or(0.5),
            movement()?,
        ),
        "path.convertPoint" => {
            if p.get("curvature").and_then(Value::as_bool).unwrap_or(false) {
                Operation::CurvatureConvert(knot(p, cmd)?)
            } else {
                Operation::Convert(knot(p, cmd)?, p.get("out").map(|v| point(v, cmd)).transpose()?)
            }
        }
        _ => return Err(bad(cmd, "unknown point edit")),
    })
}
fn context(s: &Session, cmd: &str, p: &Value, normalize: bool, background: bool) -> Result<()> {
    let map = p.as_object().filter(|m| m.len() <= 12).ok_or_else(|| bad(cmd, "bounded point-edit object required"))?;
    if map.keys().any(|k| {
        !matches!(k.as_str(), "name" | "pathId" | "layer" | "documentId" | "expectedRevision" | "coalesce")
            && !match cmd {
                "path.moveAnchors" => matches!(k.as_str(), "anchors" | "move" | "curvature"),
                "path.moveHandle" => matches!(k.as_str(), "subpath" | "knot" | "handle" | "to" | "independent"),
                "path.bendSegment" => matches!(k.as_str(), "subpath" | "knot" | "t" | "move"),
                "path.convertPoint" => matches!(k.as_str(), "subpath" | "knot" | "out" | "curvature"),
                _ => false,
            }
    }) {
        return Err(bad(cmd, "unknown point-edit field"));
    }
    for key in ["name", "coalesce"] {
        if let Some(v) = p.get(key)
            && !v.as_str().is_some_and(|v| v.len() <= 2048 && !v.contains('\0'))
        {
            return Err(bad(cmd, "bounded UTF8 selector/gesture required"));
        }
    }
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let captured = match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => false,
        (Some(id), Some(rev)) if decimal(id, true, cmd)? == state.doc.id.0 && decimal(rev, false, cmd)? == state.revision => true,
        _ => return Err(bad(cmd, "captured point target changed or incomplete")),
    };
    if state.revision == u64::MAX
        || state.doc.accounted().is_some()
        || s.has_jobs()
        || s.command_has_pending_job()
        || s.command_has_job_on(state.doc.id)
        || s.color_restrict.is_some()
        || (state.floating.is_some() && (!normalize || captured || state.doc.xcf_state.is_some()))
        || state.doc.xcf_state.as_ref().is_some_and(|v| v.floating.is_some())
        || (background && (p.get("coalesce").is_some() || s.coalesce_request.is_some()))
    {
        return Err(bad(cmd, "finish target preview/job before point edit"));
    }
    if state.doc.paths.len() > 10000 {
        return Err(bad(cmd, "saved path count"));
    }
    if p.get("layer").is_some() {
        layer_id(s, p)?;
    }
    operation(cmd, p)?;
    crate::xcf_vector_guard::pre_dispatch(s, cmd, p)
}
fn target(s: &Session, p: &Value, cmd: &str, ctl: &Interrupt<'_>) -> Result<Target> {
    let state = s.active().ok_or(EngineError::NoDocument)?;
    if let Some(id) = saved_path_id(p)? {
        let mut count = 0;
        for path in &state.doc.paths {
            poll(ctl)?;
            if path.id == id {
                count += 1;
            }
        }
        if count != 1 {
            return Err(bad(cmd, "missing/ambiguous saved pathId"));
        }
        return Ok(Target::Saved(id));
    }
    let name = p.get("name").and_then(Value::as_str).unwrap_or("work");
    if name == "layer" {
        let id = layer_id(s, p)?;
        let layer = state.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        return Ok(Target::Layer(id, matches!(layer.content, LayerContent::Shape(_))));
    }
    if crate::vector_cmds::is_work(Some(name)) {
        return Ok(Target::Work);
    }
    let mut found = None;
    for saved in &state.doc.paths {
        poll(ctl)?;
        if saved.name == name && found.replace(saved.id).is_some() {
            return Err(bad(cmd, "ambiguous saved path name"));
        }
    }
    found.map(Target::Saved).ok_or_else(|| bad(cmd, "missing saved path"))
}
fn borrowed<'a>(doc: &'a Document, t: Target, cmd: &str) -> Result<&'a Path> {
    match t {
        Target::Work => doc.work_path.as_ref().ok_or_else(|| bad(cmd, "no work path")),
        Target::Saved(id) => doc.paths.iter().find(|p| p.id == id).map(|p| &p.path).ok_or_else(|| bad(cmd, "saved path disappeared")),
        Target::Layer(id, shape) => {
            let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            if shape && let LayerContent::Shape(sh) = &l.content {
                return Ok(&sh.path);
            }
            l.vector_mask.as_ref().map(|v| &v.path).ok_or_else(|| bad(cmd, "layer has no vector path"))
        }
    }
}
fn census(path: &Path, cmd: &str, ctl: &Interrupt<'_>) -> Result<u64> {
    if path.subpaths.len() > 10000 {
        return Err(bad(cmd, "subpath count"));
    }
    let mut count = 0usize;
    let mut bytes = (path.subpaths.capacity() as u64).checked_mul(std::mem::size_of::<Subpath>() as u64).ok_or_else(|| bad(cmd, "path capacity"))?;
    for sub in &path.subpaths {
        poll(ctl)?;
        count = count.checked_add(sub.knots.len()).filter(|n| *n <= 100000).ok_or_else(|| bad(cmd, "aggregate knot count"))?;
        bytes = bytes
            .checked_add(
                (sub.knots.capacity() as u64).checked_mul(std::mem::size_of::<photocraft_doc::Knot>() as u64).ok_or_else(|| bad(cmd, "knot capacity"))?,
            )
            .filter(|n| *n <= 64 << 20)
            .ok_or_else(|| bad(cmd, "path capacity bound"))?;
        for knot in &sub.knots {
            poll(ctl)?;
            for v in [knot.anchor, knot.in_ctrl, knot.out_ctrl] {
                if !v.x.is_finite() || !v.y.is_finite() || v.x.abs() > 1e6 || v.y.abs() > 1e6 {
                    return Err(bad(cmd, "path coordinate domain"));
                }
            }
        }
    }
    Ok(bytes)
}
fn policy(doc: &Document, t: Target, cmd: &str, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    match t {
        Target::Layer(id, shape) => {
            fn visit(layers: &[photocraft_doc::Layer], id: LayerId, depth: usize, n: &mut usize, m: &mut usize, cmd: &str, c: &Interrupt<'_>) -> Result<()> {
                if depth > photocraft_doc::MAX_GROUP_DEPTH {
                    return Err(bad(cmd, "owner depth"));
                }
                for l in layers {
                    poll(c)?;
                    *n = n.checked_add(1).filter(|v| *v <= 10000).ok_or_else(|| bad(cmd, "owner count"))?;
                    if l.id == id {
                        *m += 1;
                    }
                    if let LayerContent::Group(g) = &l.content {
                        visit(&g.children, id, depth + 1, n, m, cmd, c)?;
                    }
                }
                Ok(())
            }
            let (mut n, mut m) = (0, 0);
            visit(&doc.layers, id, 0, &mut n, &mut m, cmd, ctl)?;
            if id.0 == 0 || m != 1 {
                return Err(bad(cmd, "missing/ambiguous layer identity"));
            }
            let locks = doc.effective_locks(id);
            if locks.all || locks.pixels || locks.position {
                return Err(bad(cmd, "vector owner is locked"));
            }
            let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            if layer.indexed_source.is_some() || layer.video.is_some() || matches!(layer.content, LayerContent::BackedRaster(_)) {
                return Err(bad(cmd, "source geometry edit is unsupported"));
            }
            if shape && matches!(doc.depth, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {
                return Err(bad(cmd, "wide shape regeneration is unsupported"));
            }
            if let LayerContent::Shape(sh) = &layer.content {
                photocraft_vector::flatten::validate_shape(&sh.path, sh.stroke.as_ref()).map_err(|e| bad(cmd, e))?;
            }
            if doc.xcf_state.as_ref().is_some_and(|state| state.source_items.iter().any(|v| v.owner == ItemRef::Layer(id))) {
                return Err(bad(cmd, "source-bound layer path interpretation is unsupported"));
            }
        }
        Target::Saved(id) => {
            if id.0 == 0 {
                return Err(bad(cmd, "zero path identity"));
            }
            let mut matches = 0;
            for p in &doc.paths {
                poll(ctl)?;
                if p.id == id {
                    matches += 1;
                }
            }
            if matches != 1 {
                return Err(bad(cmd, "ambiguous saved path identity"));
            }
            if let Some(state) = &doc.xcf_state {
                if state.items.len() > 10000 || state.source_items.len() > 10000 {
                    return Err(bad(cmd, "typed path metadata count"));
                }
                doc.validate_xcf_state_with(ctl).map_err(|e| error(cmd, ctl, e))?;
                for item in &state.items {
                    poll(ctl)?;
                    if item.item == ItemRef::Path(id) {
                        let locks = crate::xcf_item_locks::effective_item_locks(doc, item.item)?;
                        locks.require(crate::xcf_item_locks::EditKind::Content)?;
                        locks.require(crate::xcf_item_locks::EditKind::Position)?;
                    }
                }
                for source in &state.source_items {
                    poll(ctl)?;
                    if matches!(source.source,EditableSource::Vector{path,..}
if path==id)
                    {
                        let locks = crate::xcf_item_locks::effective_item_locks(doc, source.owner)?;
                        locks.require(crate::xcf_item_locks::EditKind::Content)?;
                        locks.require(crate::xcf_item_locks::EditKind::Position)?;
                    }
                }
            }
        }
        Target::Work => {}
    }
    poll(ctl)
}
fn admit(doc: &DocumentSnapshot, t: Target, cmd: &str, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    doc.validate_resident()?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, ctl).map_err(|e| error(cmd, ctl, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, ctl).map_err(|e| error(cmd, ctl, e))? as u64;
    let raster = doc.resident_raster_footprint_with(ctl)?;
    policy(doc, t, cmd, ctl)?;
    let selected = borrowed(doc, t, cmd)?;
    let path = census(selected, cmd, ctl)?;
    let knots = selected.subpaths.iter().try_fold(0u64, |n, s| n.checked_add(s.knots.len() as u64)).ok_or_else(|| bad(cmd, "response knot count"))?;
    let mut extra = path
        .checked_mul(4)
        .and_then(|n| n.checked_add(knots.checked_mul(2048)?))
        .and_then(|n| n.checked_add(16 << 20))
        .ok_or_else(|| bad(cmd, "point staging/response capacity"))?;
    if matches!(t, Target::Layer(_, true)) {
        let area = doc.bounds();
        let count = u64::from(area.width()).checked_mul(u64::from(area.height())).ok_or_else(|| bad(cmd, "shape count"))?;
        let bpp = doc.pixel_format().bytes_per_pixel() as u64;
        extra = extra
            .checked_add(
                count.checked_mul(bpp.checked_add(64).ok_or_else(|| bad(cmd, "shape pixel capacity"))?).ok_or_else(|| bad(cmd, "shape render capacity"))?,
            )
            .and_then(|n| n.checked_add(128 << 20))
            .ok_or_else(|| bad(cmd, "shape staging capacity"))?;
        if count.checked_mul(2048).is_none_or(|n| n > 4_000_000_000) {
            return Err(bad(cmd, "legacy shape-render work bound"));
        }
    }
    if model.checked_mul(8).is_none_or(|n| n > 1_000_000_000)
        || model
            .checked_mul(4)
            .and_then(|n| n.checked_add(payload))
            .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
            .and_then(|n| n.checked_add(extra))
            .is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES)
    {
        return Err(bad(cmd, "point-edit retained/COW/work limit"));
    }
    poll(ctl)
}
fn text(v: &str, cmd: &str) -> Result<String> {
    if v.len() > 2048 {
        return Err(bad(cmd, "response name bound"));
    }
    let mut s = String::new();
    s.try_reserve_exact(v.len()).map_err(|_| bad(cmd, "name allocation"))?;
    if s.capacity() > 4096 {
        return Err(bad(cmd, "name capacity"));
    }
    s.push_str(v);
    Ok(s)
}
fn copy(path: &Path, cmd: &str, ctl: &Interrupt<'_>) -> Result<Path> {
    let mut subs = Vec::new();
    subs.try_reserve_exact(path.subpaths.len()).map_err(|_| bad(cmd, "subpath allocation"))?;
    for sub in &path.subpaths {
        poll(ctl)?;
        let mut knots = Vec::new();
        knots.try_reserve_exact(sub.knots.len()).map_err(|_| bad(cmd, "knot allocation"))?;
        for batch in sub.knots.chunks(256) {
            poll(ctl)?;
            knots.extend_from_slice(batch);
        }
        subs.push(Subpath { closed: sub.closed, op: sub.op, knots });
    }
    let p = Path { subpaths: subs, fill_rule: path.fill_rule, inverted: path.inverted };
    census(&p, cmd, ctl)?;
    Ok(p)
}
fn owned(op: Operation<'_>, cmd: &str, ctl: &Interrupt<'_>) -> Result<OwnedOperation> {
    Ok(match op {
        Operation::Anchors(values, d) | Operation::CurvatureAnchors(values, d) => {
            let mut out = Vec::new();
            out.try_reserve_exact(values.len()).map_err(|_| bad(cmd, "anchor allocation"))?;
            if out.capacity() > 200000 {
                return Err(bad(cmd, "anchor capacity"));
            }
            for v in values {
                poll(ctl)?;
                out.push(pair(v, cmd)?);
            }
            out.sort_unstable();
            out.dedup();
            poll(ctl)?;
            if matches!(op, Operation::CurvatureAnchors(..)) { OwnedOperation::CurvatureAnchors(out, d) } else { OwnedOperation::Anchors(out, d) }
        }
        Operation::Handle(k, h, to, i) => OwnedOperation::Handle(k, h, to, i),
        Operation::Bend(k, t, d) => OwnedOperation::Bend(k, t, d),
        Operation::Convert(k, to) => OwnedOperation::Convert(k, to),
        Operation::CurvatureConvert(k) => OwnedOperation::CurvatureConvert(k),
    })
}
fn apply(path: &mut Path, op: &OwnedOperation, cmd: &str, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    match op {
        OwnedOperation::Anchors(anchors, d) => {
            for [s, k] in anchors {
                poll(ctl)?;
                let knot = path.subpaths.get_mut(*s).and_then(|v| v.knots.get_mut(*k)).ok_or_else(|| bad(cmd, "missing anchor"))?;
                let add = |p: Point| Point::new(p.x + d[0], p.y + d[1]);
                knot.anchor = add(knot.anchor);
                knot.in_ctrl = add(knot.in_ctrl);
                knot.out_ctrl = add(knot.out_ctrl);
            }
        }
        // Upstream curvature recomputes only touched subpaths. The existing
        // complete path census and knots*2048 staging envelope precede this
        // bounded legacy helper, with the same interruption fence on both sides.
        OwnedOperation::CurvatureAnchors(anchors, d) => edit::move_anchors_curvature(path, anchors, *d).map_err(|e| bad(cmd, e))?,
        OwnedOperation::CurvatureConvert(k) => edit::toggle_curvature_point(path, *k).map_err(|e| bad(cmd, e))?,
        OwnedOperation::Handle(k, h, to, i) => edit::move_handle(path, *k, *h, *to, *i).map_err(|e| bad(cmd, e))?,
        OwnedOperation::Bend(k, t, d) => edit::bend_segment(path, *k, *t, *d).map_err(|e| bad(cmd, e))?,
        OwnedOperation::Convert(k, to) => edit::convert_point(path, *k, *to).map_err(|e| bad(cmd, e))?,
    }
    census(path, cmd, ctl)?;
    poll(ctl)
}
fn indices(path: &Path, op: Operation<'_>, cmd: &str, ctl: &Interrupt<'_>) -> Result<()> {
    let check = |[s, k]: [usize; 2]| path.subpaths.get(s).and_then(|v| v.knots.get(k)).ok_or_else(|| bad(cmd, "missing indexed knot")).map(|_| ());
    match op {
        Operation::Anchors(values, _) | Operation::CurvatureAnchors(values, _) => {
            for value in values {
                poll(ctl)?;
                check(pair(value, cmd)?)?;
            }
        }
        Operation::Bend([s, k], _, _) => {
            check([s, k])?;
            let sub = path.subpaths.get(s).ok_or_else(|| bad(cmd, "missing subpath"))?;
            let next = k.checked_add(1).ok_or_else(|| bad(cmd, "segment index overflow"))?;
            if next >= sub.knots.len() && !(sub.closed && sub.knots.len() > 1) {
                return Err(bad(cmd, "no segment leaves this knot"));
            }
        }
        Operation::Handle(k, _, _, _) | Operation::Convert(k, _) | Operation::CurvatureConvert(k) => check(k)?,
    }
    poll(ctl)
}
fn install(doc: &mut Document, t: Target, path: Path, cmd: &str, ctl: &Interrupt<'_>) -> Result<()> {
    match t {
        Target::Work => doc.work_path = Some(path),
        Target::Saved(id) => doc.paths.iter_mut().find(|v| v.id == id).ok_or_else(|| bad(cmd, "saved path changed"))?.path = path,
        Target::Layer(id, shape) => {
            let format = doc.pixel_format();
            let bounds = doc.bounds();
            let layer = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            if shape {
                let LayerContent::Shape(sh) = &mut layer.content else {
                    return Err(bad(cmd, "shape owner changed"));
                };
                sh.path = path;
                sh.live = None;
                sh.psd_raw = None;
                // Legacy shape fill/stroke math has no inner interrupt API.
                // Admitted work/storage is bounded; cancellation is polled on
                // both sides and only this private candidate can be changed.
                photocraft_vector::flatten::validate_shape(&sh.path, sh.stroke.as_ref()).map_err(|e| bad(cmd, e))?;
                poll(ctl)?;
                sh.cache = Some(photocraft_vector::render_shape(sh, format, bounds));
                poll(ctl)?;
            } else {
                layer.vector_mask.as_mut().ok_or_else(|| bad(cmd, "vector mask disappeared"))?.path = path;
            }
        }
    }
    poll(ctl)
}
fn same_path(a: &Path, b: &Path, ctl: &Interrupt<'_>) -> Result<bool> {
    if a.fill_rule != b.fill_rule || a.inverted != b.inverted || a.subpaths.len() != b.subpaths.len() {
        return Ok(false);
    }
    for (a, b) in a.subpaths.iter().zip(&b.subpaths) {
        poll(ctl)?;
        if a.closed != b.closed || a.op != b.op || a.knots.len() != b.knots.len() {
            return Ok(false);
        }
        for (a, b) in a.knots.iter().zip(&b.knots) {
            poll(ctl)?;
            if a.smooth != b.smooth {
                return Ok(false);
            }
            for (a, b) in [a.anchor, a.in_ctrl, a.out_ctrl].into_iter().zip([b.anchor, b.in_ctrl, b.out_ctrl]) {
                if a.x.to_bits() != b.x.to_bits() || a.y.to_bits() != b.y.to_bits() {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}
pub(super) fn dispatch_preflight(s: &Session, cmd: &str, p: &Value, background: bool) -> Result<()> {
    context(s, cmd, p, true, background)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let t = target(s, p, cmd, &Interrupt::NONE)?;
    admit(&st.doc, t, cmd, &Interrupt::NONE)?;
    indices(borrowed(&st.doc, t, cmd)?, operation(cmd, p)?, cmd, &Interrupt::NONE)
}
pub(super) fn run(s: &mut Session, cmd: &str, p: &Value) -> Result<Value> {
    context(s, cmd, p, false, false)?;
    let t = target(s, p, cmd, &Interrupt::NONE)?;
    let state = s.active().ok_or(EngineError::NoDocument)?;
    admit(&state.doc, t, cmd, &Interrupt::NONE)?;
    indices(borrowed(&state.doc, t, cmd)?, operation(cmd, p)?, cmd, &Interrupt::NONE)?;
    let op = owned(operation(cmd, p)?, cmd, &Interrupt::NONE)?;
    let name = match t {
        Target::Saved(id) => state.doc.paths.iter().find(|v| v.id == id).map(|v| v.name.as_str()).ok_or_else(|| bad(cmd, "saved path disappeared"))?,
        _ => p.get("name").and_then(Value::as_str).unwrap_or("work"),
    };
    let name = text(name, cmd)?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let weak = baseline.downgrade();
    let active = state.active_layer;
    let coalesce = s.coalesce_request.as_deref().map(|v| text(v, cmd)).transpose()?;
    let prior = state.coalesce.as_deref().map(|v| text(v, cmd)).transpose()?;
    let label = match cmd {
        "path.moveAnchors" => "Drag Anchor Points",
        "path.moveHandle" => "Drag Direction Point",
        "path.bendSegment" => "Drag Segment",
        _ => "Convert Point",
    };
    let work = baseline.clone();
    let command = cmd.to_owned();
    crate::jobs::run(
        s,
        label,
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, label, |ctl| {
                admit(&work, t, &command, ctl)?;
                let old = borrowed(&work, t, &command)?;
                let mut path = copy(old, &command, ctl)?;
                apply(&mut path, &op, &command, ctl)?;
                let response =
                    json!({"name":name,"pathId":match t{Target::Saved(id)=>Some(id.0.to_string()),_=>None},"path":crate::vector_cmds::path_json(&path)});
                if same_path(&path, old, ctl)? {
                    return Ok((None, response, ctx.clone()));
                }
                poll(ctl)?;
                let mut document = (*work).clone();
                install(&mut document, t, path, &command, ctl)?;
                crate::xcf_vector_guard::validate_path_edits_with_interrupt(&work, &document, ctl)?;
                poll(ctl)?;
                Ok((Some(document), response, ctx.clone()))
            })
        },
        move |s, (document, response, ctx)| {
            let current = s.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id
                || current.revision != revision
                || !weak.is_current(&current.doc)
                || !current.doc.shares_state(&baseline)
                || current.active_layer != active
                || current.floating.is_some()
                || current.coalesce != prior
                || s.coalesce_request != coalesce
                || s.color_restrict.is_some()
                || s.command_has_job_on(current.doc.id)
            {
                return Err(bad("path.pointEdit", "point-edit target/gesture changed during preparation"));
            }
            ctx.stage(0.95, 1.0, label, |ctl| {
                policy(&s.active().ok_or(EngineError::NoDocument)?.doc, t, "path.pointEdit", ctl)?;
                poll(ctl)?;
                if let Some(document) = document {
                    s.edit_with_interrupt(label, ctl, move |doc, _| {
                        poll(ctl)?;
                        *doc = document;
                        Ok(())
                    })?;
                }
                Ok(response)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Knot, Size};
    fn session() -> Session {
        let mut d = Document::new("Points", Size::new(16, 12), ColorMode::Rgb, SampleType::U16);
        let mut sub = Subpath::polyline(&[(1.0, 3.0), (5.0, 1.0), (10.0, 4.0)]);
        sub.knots[1] = Knot::smooth(Point::new(5.0, 1.0), Point::new(3.0, 1.0), Point::new(7.0, 1.0));
        d.work_path = Some(Path::new(vec![sub]));
        let mut s = Session::new();
        s.add_document(d, None).unwrap();
        s
    }
    #[test]
    fn controlled_point_equations_match_original_geometry() {
        for (cmd, p) in [
            ("path.moveAnchors", json!({"anchors":[[0,1],[0,1]],"move":[2,-1]})),
            ("path.moveHandle", json!({"subpath":0,"knot":1,"handle":"out","to":[8,3]})),
            ("path.bendSegment", json!({"subpath":0,"knot":1,"t":0.5,"move":[1,2]})),
            ("path.convertPoint", json!({"subpath":0,"knot":1,"out":[8,2]})),
        ] {
            let mut s = session();
            let mut expected = s.active().unwrap().doc.work_path.clone().unwrap();
            match operation(cmd, &p).unwrap() {
                Operation::Anchors(_, d) => edit::move_anchors(&mut expected, &[[0, 1], [0, 1]], d).unwrap(),
                Operation::CurvatureAnchors(_, d) => edit::move_anchors_curvature(&mut expected, &[[0, 1], [0, 1]], d).unwrap(),
                Operation::CurvatureConvert(k) => edit::toggle_curvature_point(&mut expected, k).unwrap(),
                Operation::Handle(k, h, to, i) => edit::move_handle(&mut expected, k, h, to, i).unwrap(),
                Operation::Bend(k, t, d) => edit::bend_segment(&mut expected, k, t, d).unwrap(),
                Operation::Convert(k, to) => edit::convert_point(&mut expected, k, to).unwrap(),
            };
            let before = s.active().unwrap().doc.clone();
            run(&mut s, cmd, &p).unwrap();
            assert_eq!(s.active().unwrap().doc.work_path.as_ref(), Some(&expected));
            assert_eq!(s.active().unwrap().doc.layers, before.layers);
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc.work_path, before.work_path);
        }
    }
    #[test]
    fn late_invalid_cancel_noop_and_sync_coalesce_preserve_history() {
        let mut s = session();
        let original = s.active().unwrap().doc.clone();
        let p = json!({"anchors":[[0,1],[0,99999]],"move":[1,0]});
        assert!(dispatch_preflight(&s, "path.moveAnchors", &p, false).is_err());
        assert!(s.active().unwrap().doc.shares_state(&original));
        assert!(matches!(copy(original.work_path.as_ref().unwrap(), "path.moveAnchors", &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        s.edit("Redo seed", |d, _| {
            d.name.push_str(" changed");
            Ok(())
        })
        .unwrap();
        assert!(s.undo());
        let revision = s.active().unwrap().revision;
        run(&mut s, "path.moveAnchors", &json!({"anchors":[[0,1]],"move":[0,0]})).unwrap();
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(s.redo());
        let p = json!({"anchors":[[0,0]],"move":[1,0],"coalesce":"Owned point drag"});
        assert!(dispatch_preflight(&s, "path.moveAnchors", &p, true).is_err());
        s.coalesce_request = Some("Owned point drag".into());
        run(&mut s, "path.moveAnchors", &p).unwrap();
        run(&mut s, "path.moveAnchors", &p).unwrap();
        s.coalesce_request = None;
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots[0].anchor, Point::new(1.0, 3.0));
    }
}
