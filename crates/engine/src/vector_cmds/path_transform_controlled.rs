//! Controlled existing affine path transform; no new geometry/source interpretation.
use super::{bad, is_work, layer_id, refresh_shape, saved_path_id, transform_shape};
use crate::{EngineError, Result, Session};
use photocraft_doc::item_identity::{ItemRef, PathId};
use photocraft_doc::{Document, LayerContent, LayerId, Path, Subpath};
use photocraft_geom::Affine;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};
const CMD: &str = "path.transform";
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn error(control: &Interrupt<'_>, e: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(CMD, e.to_string()) }
}
fn finite_live(live: Option<&photocraft_doc::LiveShape>) -> Result<()> {
    let valid = match live {
        None => true,
        Some(photocraft_doc::LiveShape::Rect { rect, radii }) => rect.iter().chain(radii).all(|v| v.is_finite()),
        Some(photocraft_doc::LiveShape::Ellipse { rect }) => rect.iter().all(|v| v.is_finite()),
        Some(photocraft_doc::LiveShape::Polygon { rect, star_ratio, .. }) => rect.iter().all(|v| v.is_finite()) && star_ratio.is_finite(),
        Some(photocraft_doc::LiveShape::Line { from, to, weight }) => from.iter().chain(to).all(|v| v.is_finite()) && weight.is_finite(),
    };
    if valid { Ok(()) } else { Err(bad(CMD, "nonfinite live shape geometry")) }
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse().ok()
}
#[derive(Clone, Copy)]
enum Target {
    Saved(PathId),
    Work,
    Layer(LayerId),
}
struct Request<'a> {
    target: Target,
    name: &'a str,
    matrix: Affine,
    shape: bool,
}
fn layer_path(doc: &Document, id: LayerId) -> Result<&Path> {
    let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    match &layer.content {
        LayerContent::Shape(shape) => Ok(&shape.path),
        _ => layer.vector_mask.as_ref().map(|mask| &mask.path).ok_or_else(|| bad(CMD, "layer has no shape path or vector mask")),
    }
}
fn path(doc: &Document, target: Target) -> Result<&Path> {
    match target {
        Target::Saved(id) => doc.paths.iter().find(|saved| saved.id == id).map(|saved| &saved.path).ok_or_else(|| bad(CMD, "saved path disappeared")),
        Target::Work => doc.work_path.as_ref().ok_or_else(|| bad(CMD, "no work path")),
        Target::Layer(id) => layer_path(doc, id),
    }
}
fn retained_path(path: &Path, control: &Interrupt<'_>) -> Result<(usize, u64)> {
    poll(control)?;
    if path.subpaths.len() > 10000 {
        return Err(bad(CMD, "subpath limit"));
    }
    let mut knots = 0usize;
    let mut bytes = (path.subpaths.capacity() as u64).checked_mul(std::mem::size_of::<Subpath>() as u64).ok_or_else(|| bad(CMD, "path capacity overflow"))?;
    for sub in &path.subpaths {
        poll(control)?;
        knots = knots.checked_add(sub.knots.len()).filter(|n| *n <= 100000).ok_or_else(|| bad(CMD, "knot limit"))?;
        bytes = bytes
            .checked_add(
                (sub.knots.capacity() as u64)
                    .checked_mul(std::mem::size_of::<photocraft_doc::Knot>() as u64)
                    .ok_or_else(|| bad(CMD, "knot capacity overflow"))?,
            )
            .ok_or_else(|| bad(CMD, "path capacity overflow"))?;
        for knot in &sub.knots {
            poll(control)?;
            for p in [knot.anchor, knot.in_ctrl, knot.out_ctrl] {
                if !p.x.is_finite() || !p.y.is_finite() {
                    return Err(bad(CMD, "nonfinite retained path geometry"));
                }
            }
        }
    }
    poll(control)?;
    Ok((knots, bytes))
}
fn number(params: &Value, key: &str, default: f64) -> Result<f64> {
    match params.get(key) {
        None => Ok(default),
        Some(value) => {
            value.as_f64().filter(|n| n.is_finite() && n.abs() <= 1000000.).ok_or_else(|| bad(CMD, format!("{key} must be finite and within +/-1000000")))
        }
    }
}
fn request<'a>(session: &Session, params: &'a Value) -> Result<Request<'a>> {
    let object = params.as_object().filter(|map| map.len() <= 11).ok_or_else(|| bad(CMD, "bounded transform parameters required"))?;
    if !object.keys().all(|key| {
        matches!(
            key.as_str(),
            "pathId" | "name" | "layer" | "matrix" | "translateX" | "translateY" | "scaleX" | "scaleY" | "angle" | "documentId" | "expectedRevision"
        )
    }) {
        return Err(bad(CMD, "unknown transform parameter"));
    }
    let name = match params.get("name") {
        None => "work",
        Some(value) => value.as_str().filter(|text| text.len() <= 4096 && !text.contains('\0')).ok_or_else(|| bad(CMD, "bounded UTF-8 name required"))?,
    };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.revision == u64::MAX || state.doc.paths.len() > 10000 || state.doc.accounted().is_some() {
        return Err(bad(CMD, "path revision/count/resident limit"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if state.doc.id.0 != 0 && decimal(id) == Some(state.doc.id.0) && decimal(revision) == Some(state.revision) => {}
        _ => return Err(bad(CMD, "captured transform target changed or incomplete")),
    }
    if state.floating.is_some()
        || state.coalesce.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
    {
        return Err(bad(CMD, "finish jobs or gestures before path transform"));
    }
    photocraft_doc::resident_structure::visit_layers_with(&state.doc, &Interrupt::NONE, |_| Ok(true))?;
    if let Some(imported) = &state.doc.xcf_state
        && (imported.items.len() > 10000 || imported.source_items.len() > 10000)
    {
        return Err(bad(CMD, "source path metadata limit"));
    }
    let id = saved_path_id(params)?;
    if params.get("layer").is_some() {
        layer_id(session, params)?;
    }
    let target = if let Some(id) = id {
        if state.doc.paths.iter().filter(|path| path.id == id).count() != 1 {
            return Err(bad(CMD, "missing or ambiguous pathId"));
        }
        Target::Saved(id)
    } else if name == "layer" {
        let id = layer_id(session, params)?;
        let mut count = 0usize;
        photocraft_doc::resident_structure::visit_layers_with(&state.doc, &Interrupt::NONE, |layer| {
            if layer.id == id {
                count += 1;
            }
            Ok(true)
        })?;
        if count != 1 {
            return Err(bad(CMD, "missing or ambiguous layer identity"));
        }
        Target::Layer(id)
    } else if is_work(Some(name)) {
        Target::Work
    } else {
        Target::Saved(state.doc.paths.iter().find(|path| path.name == name).ok_or_else(|| bad(CMD, "no named path"))?.id)
    };
    let source = path(&state.doc, target)?;
    retained_path(source, &Interrupt::NONE)?;
    let shape = matches!(target,Target::Layer(id) if state.doc.layer(id).is_some_and(|layer|matches!(layer.content,LayerContent::Shape(_))));
    let wide_cache=match target {
        Target::Layer(id)=>state.doc.layer(id).is_some_and(|layer|matches!(&layer.content,LayerContent::Shape(shape) if shape.cache.as_ref().is_some_and(|surface|matches!(surface.format().sample,photocraft_doc::SampleType::U32|photocraft_doc::SampleType::F64)))),_=>false,
    };
    if shape && (wide_cache || matches!(state.doc.depth, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64)) {
        return Err(bad(CMD, "wide shape cache generation is not qualified"));
    }
    if let Target::Layer(id) = target
        && let LayerContent::Shape(shape) = &state.doc.layer(id).ok_or(EngineError::NoLayer(id))?.content
    {
        finite_live(shape.live.as_ref())?;
    }
    if let Target::Layer(id) = target {
        if let Some(imported) = &state.doc.xcf_state
            && imported.source_items.iter().any(|source| source.owner == ItemRef::Layer(id))
        {
            return Err(bad(CMD, "source-owned layer path transform is not qualified"));
        }
        crate::xcf_item_locks::effective_layer_locks(&state.doc, id)?.require(crate::xcf_item_locks::EditKind::Content)?;
        crate::xcf_item_locks::effective_layer_locks(&state.doc, id)?.require(crate::xcf_item_locks::EditKind::Position)?;
    }
    crate::xcf_vector_guard::pre_dispatch(session, CMD, params)?;
    // Validate every supplied scalar even when the explicit matrix takes precedence.
    for key in ["translateX", "translateY", "scaleX", "scaleY", "angle"] {
        number(params, key, 0.)?;
    }
    let matrix = if let Some(value) = params.get("matrix") {
        let values = value.as_array().filter(|values| values.len() == 6).ok_or_else(|| bad(CMD, "matrix must contain six numbers"))?;
        let mut m = [0f64; 6];
        for (slot, value) in m.iter_mut().zip(values) {
            *slot = value.as_f64().filter(|n| n.is_finite() && n.abs() <= 1000000.).ok_or_else(|| bad(CMD, "finite bounded matrix coefficients required"))?;
        }
        Affine { m }
    } else {
        if !["translateX", "translateY", "scaleX", "scaleY", "angle"].iter().any(|key| params.get(*key).is_some()) {
            return Err(bad(CMD, "pass matrix or translate/scale/angle parameters"));
        }
        // Numeric components pivot on the same resolved path that will be mutated.
        // An explicit layer target must not borrow another active layer's center.
        let center = source;
        retained_path(center, &Interrupt::NONE)?;
        let (x0, y0, x1, y1) = center.control_bounds().ok_or_else(|| bad(CMD, "path has no points to transform"))?;
        let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        Affine::translate(-cx, -cy)
            .then(&Affine { m: [number(params, "scaleX", 1.)?, 0., 0., number(params, "scaleY", 1.)?, 0., 0.] })
            .then(&Affine::rotate(number(params, "angle", 0.)?.to_radians()))
            .then(&Affine::translate(cx + number(params, "translateX", 0.)?, cy + number(params, "translateY", 0.)?))
    };
    if !matrix.m.iter().all(|v| v.is_finite()) {
        return Err(bad(CMD, "derived matrix overflow"));
    }
    Ok(Request { target, name, matrix, shape })
}
pub(super) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    request(session, params).map(|_| ())
}
fn transformed(source: &Path, matrix: &Affine, control: &Interrupt<'_>) -> Result<Path> {
    poll(control)?;
    let mut subpaths = Vec::new();
    subpaths.try_reserve_exact(source.subpaths.len()).map_err(|_| bad(CMD, "subpath allocation"))?;
    for old in &source.subpaths {
        poll(control)?;
        let mut knots = Vec::new();
        knots.try_reserve_exact(old.knots.len()).map_err(|_| bad(CMD, "knot allocation"))?;
        for knot in &old.knots {
            poll(control)?;
            let knot = knot.transform(matrix);
            for point in [knot.anchor, knot.in_ctrl, knot.out_ctrl] {
                if !point.x.is_finite() || !point.y.is_finite() {
                    return Err(bad(CMD, "transformed point overflow"));
                }
            }
            knots.push(knot);
        }
        subpaths.push(Subpath { closed: old.closed, op: old.op, knots });
    }
    poll(control)?;
    Ok(Path { subpaths, fill_rule: source.fill_rule, inverted: source.inverted })
}
fn admission(doc: &Document, target: Target, shape: bool, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let (count, bytes) = retained_path(path(doc, target)?, control)?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let raster = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let pixels = if shape { u64::from(doc.size.width).checked_mul(u64::from(doc.size.height)).ok_or_else(|| bad(CMD, "render area overflow"))? } else { 0 };
    let peak = payload
        .checked_add(model.checked_mul(8).ok_or_else(|| bad(CMD, "model capacity overflow"))?)
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(3)?))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(8)?))
        .and_then(|n| n.checked_add(bytes.checked_mul(4)?))
        .and_then(|n| n.checked_add(pixels.checked_mul(64)?))
        .and_then(|n| n.checked_add(128 << 20));
    if peak.is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES) {
        return Err(bad(CMD, "transform aggregate model/cache/geometry admission"));
    }
    let work = payload
        .checked_add(model)
        .and_then(|n| n.checked_mul(16))
        .and_then(|n| n.checked_add((count as u64).checked_mul(2048)?))
        .and_then(|n| n.checked_add(pixels.checked_mul((count as u64).checked_add(1)?)?.checked_mul(64)?));
    if work.is_none_or(|n| n > 8_000_000_000) {
        return Err(bad(CMD, "transform work admission"));
    }
    poll(control)
}
pub(super) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    let request = request(session, params)?;
    let target = request.target;
    let matrix = request.matrix;
    let shape = request.shape;
    let mut name = String::new();
    name.try_reserve_exact(request.name.len()).map_err(|_| bad(CMD, "response name allocation"))?;
    name.push_str(request.name);
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let snapshot = state.doc.clone();
    let revision = state.revision;
    let active = state.active_layer;
    let weak = snapshot.downgrade();
    let worker = snapshot.clone();
    crate::jobs::run(
        session,
        "Transform Path",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Transform Path", |control| {
                admission(&worker, target, shape, control)?;
                let replacement = transformed(path(&worker, target)?, &matrix, control)?;
                poll(control)?;
                let mut document = (*worker).clone();
                match target {
                    Target::Saved(id) => {
                        document.paths.iter_mut().find(|path| path.id == id).ok_or_else(|| bad(CMD, "saved target disappeared"))?.path = replacement
                    }
                    Target::Work => document.work_path = Some(replacement),
                    Target::Layer(id) => {
                        let layer = document.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
                        if let LayerContent::Shape(shape) = &mut layer.content {
                            // The existing helper supplies the exact live-shape policy;
                            // the separately polled path transform supplies its knots.
                            shape.path = Path::default();
                            poll(control)?;
                            transform_shape(shape, &matrix);
                            shape.path = replacement;
                            shape.psd_raw = None;
                            finite_live(shape.live.as_ref())?;
                            poll(control)?;
                            refresh_shape(&worker, shape);
                            poll(control)?;
                        } else {
                            layer.vector_mask.as_mut().ok_or_else(|| bad(CMD, "vector mask disappeared"))?.path = replacement;
                        }
                    }
                }
                poll(control)?;
                Ok((document, json!({"name":name,"matrix":matrix.m}), ctx.clone()))
            })
        },
        move |session, (document, response, ctx)| {
            ctx.stage(0.95, 1., "Transform Path", |control| {
                poll(control)?;
                let state = session.active().ok_or(EngineError::NoDocument)?;
                if state.doc.id != snapshot.id
                    || state.revision != revision
                    || !state.doc.shares_state(&snapshot)
                    || !weak.is_current(&state.doc)
                    || state.active_layer != active
                    || state.floating.is_some()
                    || state.coalesce.is_some()
                    || session.command_has_job_on(snapshot.id)
                    || session.command_has_pending_job()
                    || session.coalesce_request.is_some()
                    || session.color_restrict.is_some()
                {
                    return Err(bad(CMD, "captured path transform state changed"));
                }
                session.edit_with_interrupt("Transform Path", control, move |doc, _| {
                    poll(control)?;
                    *doc = document;
                    Ok(())
                })?;
                Ok(response)
            })
        },
    )
}

// Meaningful existing-route oracles; not executed in the implementation-only wave.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, NamedPath, SampleType, Size};
    fn geometry() -> Path {
        super::super::parse_path(&json!({"subpaths":[{"closed":false,"knots":[{"anchor":[-4.,2.],"in":[-5.,1.],"out":[-3.,3.],"smooth":true},[8.,10.]]}],"fillRule":"evenodd","inverted":true})).unwrap()
    }
    fn fixture() -> Session {
        let mut doc = Document::new("Own transform", Size::new(16, 16), ColorMode::Rgb, SampleType::U32);
        let mut path = NamedPath::new("Saved", geometry());
        path.id = PathId(u64::MAX);
        doc.paths.push(path);
        doc.work_path = Some(geometry());
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
    }
    #[test]
    fn explicit_matrix_maximum_saved_id_matches_old_point_math_and_native_history() {
        let mut session = fixture();
        let before = session.active().unwrap().doc.clone();
        let matrix = Affine { m: [-2., 0.25, 0.5, 3., -7., 9.] };
        let expected = before.paths[0].path.transform(&matrix);
        run(&mut session, &json!({"pathId":u64::MAX.to_string(),"name":"work","matrix":matrix.m})).unwrap();
        let current = &session.active().unwrap().doc;
        assert_eq!(current.paths[0].path, expected);
        assert_eq!(current.work_path, before.work_path);
        assert_eq!(current.paths[0].id, PathId(u64::MAX));
        let reopened = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(current, &Default::default()).unwrap()).unwrap();
        // MAX cannot reserve a future runtime identity; native loading rekeys the
        // complete identity set. Compare every persisted path field under its bijection.
        let current_paths = current.paths.clone();
        assert_eq!(reopened.paths.len(), current_paths.len());
        assert_eq!(reopened.work_path, current.work_path);
        let mut remapped_paths = reopened.paths.clone();
        let mut loaded_ids = std::collections::BTreeSet::new();
        for (loaded, original) in remapped_paths.iter_mut().zip(&current_paths) {
            assert_ne!(loaded.id.0, 0);
            assert_ne!(loaded.id, PathId(u64::MAX));
            assert!(loaded_ids.insert(loaded.id.0));
            loaded.id = original.id;
        }
        assert_eq!(remapped_paths, current_paths);
        session.execute("edit.undo", Value::Null).unwrap();
        assert_eq!(session.active().unwrap().doc.paths, before.paths);
        session.execute("edit.redo", Value::Null).unwrap();
        assert_eq!(session.active().unwrap().doc.paths, current_paths);
    }
    #[test]
    fn centered_components_preserve_existing_order_and_closed_handle_metadata() {
        let mut session = fixture();
        let before = session.active().unwrap().doc.work_path.clone().unwrap();
        let (x0, y0, x1, y1) = before.control_bounds().unwrap();
        let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let expected = Affine::translate(-cx, -cy)
            .then(&Affine { m: [-2., 0., 0., 3., 0., 0.] })
            .then(&Affine::rotate(37f64.to_radians()))
            .then(&Affine::translate(cx - 5., cy + 7.));
        let result = run(&mut session, &json!({"translateX":-5.,"translateY":7.,"scaleX":-2.,"scaleY":3.,"angle":37.})).unwrap();
        assert_eq!(result["matrix"], json!(expected.m));
        assert_eq!(session.active().unwrap().doc.work_path.as_ref().unwrap(), &before.transform(&expected));
        let current = session.active().unwrap().doc.work_path.as_ref().unwrap();
        assert_eq!(current.fill_rule, before.fill_rule);
        assert_eq!(current.inverted, before.inverted);
        assert_eq!(current.subpaths[0].closed, before.subpaths[0].closed);
        assert_eq!(current.subpaths[0].knots[0].smooth, before.subpaths[0].knots[0].smooth);
    }
    #[test]
    fn shape_route_matches_existing_live_shape_transform_and_cache_rebuild() {
        let mut doc = Document::new("Own shape", Size::new(16, 16), ColorMode::Rgb, SampleType::U8);
        let shape = photocraft_doc::ShapeLayer {
            path: Path::new(vec![Subpath::polygon(&[(1., 2.), (5., 2.), (5., 6.), (1., 6.)])]),
            fill: Some(photocraft_doc::Fill::Solid(photocraft_color::Color::WHITE)),
            live: Some(photocraft_doc::LiveShape::Rect { rect: [1., 2., 4., 4.], radii: [0.; 4] }),
            psd_raw: Some(vec![1, 2, 3].into()),
            ..Default::default()
        };
        let matrix = Affine { m: [2., 0., 0., 1., -1., 3.] };
        let mut expected = shape.clone();
        transform_shape(&mut expected, &matrix);
        refresh_shape(&doc, &mut expected);
        let layer = photocraft_doc::Layer::new("Own shape", LayerContent::Shape(shape));
        let id = layer.id;
        doc.layers.push(layer);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        run(&mut session, &json!({"name":"layer","layer":id.0.to_string(),"matrix":matrix.m})).unwrap();
        let LayerContent::Shape(actual) = &session.active().unwrap().doc.layer(id).unwrap().content else {
            panic!("shape fixture");
        };
        assert_eq!(actual, &expected);
        assert!(actual.psd_raw.is_none());
        assert!(actual.cache.is_some());
    }
    #[test]
    fn malformed_stale_or_cancelled_transform_keeps_original_and_real_redo() {
        let mut session = fixture();
        session
            .edit("Seed", |doc, _| {
                doc.name = "Seed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", Value::Null).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let redo = session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
        for params in [
            json!({"matrix":[1,0,0,1,0]}),
            json!({"matrix":[1,0,0,1,0,1000001]}),
            json!({"angle":"37"}),
            json!({"matrix":Affine::IDENTITY.m,"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()}),
        ] {
            assert!(run(&mut session, &params).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().revision, revision);
            assert_eq!(session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
        }
        let original = geometry();
        let mut many = original.clone();
        many.subpaths[0].knots = vec![original.subpaths[0].knots[0]; 2048];
        let preserved = many.clone();
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 8;
        assert!(matches!(transformed(&many, &Affine::translate(-1., 2.), &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
        assert_eq!(many, preserved);
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
}
