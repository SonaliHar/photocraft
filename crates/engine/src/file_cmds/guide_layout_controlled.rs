//! Controlled guide layout/current shape-bound metadata; no unit conversion.
use crate::{EngineError, Result, Session};
use photocraft_doc::{Document, Guides, LayerContent, LayerId};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};
fn error(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("guide layout: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled(control: &Interrupt<'_>, message: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { error(message) }
}
#[derive(Clone, Copy)]
struct Layout {
    columns: u32,
    rows: u32,
    margin: [f64; 4],
    width: Option<f64>,
    height: Option<f64>,
    gutter: f64,
    row_gutter: f64,
    center: bool,
    clear: bool,
}
#[derive(Clone, Copy)]
enum Plan {
    Layout(Layout),
    Shape(LayerId),
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || (text.len() > 1 && text.starts_with('0')) || !text.bytes().all(|value| value.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
fn number(params: &Value, key: &str) -> Result<Option<f64>> {
    params.get(key).map(|value| value.as_f64().filter(|value| value.is_finite()).ok_or_else(|| error(format!("{key} must be finite")))).transpose()
}
fn boolean(params: &Value, key: &str) -> Result<bool> {
    params.get(key).map(|value| value.as_bool().ok_or_else(|| error(format!("{key} must be boolean")))).transpose().map(|value| value.unwrap_or(false))
}
fn plan(session: &Session, params: &Value, shape: bool) -> Result<Plan> {
    if !params.is_null()
        && !params.as_object().is_some_and(|object| {
            object.len() <= 12
                && object.keys().all(|key| {
                    if shape {
                        matches!(key.as_str(), "layer" | "documentId" | "expectedRevision")
                    } else {
                        matches!(
                            key.as_str(),
                            "columns"
                                | "rows"
                                | "width"
                                | "height"
                                | "margin"
                                | "gutter"
                                | "rowGutter"
                                | "centerColumns"
                                | "clearExisting"
                                | "documentId"
                                | "expectedRevision"
                                | "target"
                        )
                    }
                })
        })
    {
        return Err(error("expected bounded layout or shape parameters"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.guides.horizontal.len() > 100_000 || state.doc.guides.vertical.len() > 100_000 {
        return Err(error("guide count limit"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if decimal(id) == Some(state.doc.id.0) && state.doc.id.0 != 0 && decimal(revision) == Some(state.revision) => {}
        _ => return Err(error("captured document/revision changed or incomplete")),
    }
    if shape {
        let id = match params.get("layer") {
            None => state.active_layer.ok_or_else(|| error("no active layer"))?,
            Some(value) => LayerId(value.as_u64().or_else(|| decimal(value)).filter(|id| *id != 0).ok_or_else(|| error("invalid shape layer identity"))?),
        };
        return Ok(Plan::Shape(id));
    }
    let count = |key: &str| -> Result<u32> {
        params
            .get(key)
            .map(|value| {
                value.as_u64().filter(|value| *value <= 1000).map(|value| value as u32).ok_or_else(|| error(format!("{key} must be an unsigned count <=1000")))
            })
            .transpose()
            .map(|value| value.unwrap_or(0))
    };
    let margin = match params.get("margin") {
        None => [0.; 4],
        Some(Value::Array(values)) if values.len() == 4 => {
            let mut output = [0.; 4];
            for (index, value) in values.iter().enumerate() {
                *output.get_mut(index).ok_or_else(|| error("margin index"))? =
                    value.as_f64().filter(|value| value.is_finite()).ok_or_else(|| error("margin values must be finite"))?;
            }
            output
        }
        Some(value) => [value.as_f64().filter(|value| value.is_finite()).ok_or_else(|| error("margin must be finite or four finite values"))?; 4],
    };
    match params.get("target").and_then(Value::as_str) {
        None | Some("document" | "selectedArtboards") => {}
        _ => return Err(error("target must be document or selectedArtboards")),
    };
    let gutter = number(params, "gutter")?.unwrap_or(0.);
    if margin.iter().any(|v| *v < 0.)
        || gutter < 0.
        || number(params, "rowGutter")?.is_some_and(|v| v < 0.)
        || number(params, "width")?.is_some_and(|v| v < 0.)
        || number(params, "height")?.is_some_and(|v| v < 0.)
    {
        return Err(error("layout sizes must be nonnegative"));
    }
    Ok(Plan::Layout(Layout {
        columns: count("columns")?,
        rows: count("rows")?,
        margin,
        width: number(params, "width")?.filter(|value| *value > 0.),
        height: number(params, "height")?.filter(|value| *value > 0.),
        gutter,
        row_gutter: number(params, "rowGutter")?.unwrap_or(gutter),
        center: boolean(params, "centerColumns")?,
        clear: boolean(params, "clearExisting")?,
    }))
}
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, shape: bool, _background: bool) -> Result<()> {
    plan(session, params, shape)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.revision == u64::MAX
        || state.floating.is_some()
        || state.coalesce.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(error("idle uncoalesced guide context required"));
    }
    Ok(())
}
fn unique(values: &mut Vec<f64>, value: f64, control: &Interrupt<'_>) -> Result<()> {
    if !value.is_finite() {
        return Err(error("coordinate exceeds finite native f64 storage"));
    }
    for (index, current) in values.iter().enumerate() {
        if index.is_multiple_of(1024) {
            poll(control)?;
        }
        if !current.is_finite() {
            return Err(error("nonfinite retained guide"));
        }
        if (*current - value).abs() < 0.01 {
            return Ok(());
        }
    }
    if values.len() >= 100_000 {
        return Err(error("result guide count limit"));
    }
    values.try_reserve(1).map_err(|_| error("guide allocation denied"))?;
    values.push(value);
    Ok(())
}
// Keep the explicit authority, borrowed payload and original-control proof inputs together.
#[allow(clippy::too_many_arguments)]
fn lines(start: f64, end: f64, count: u32, width: Option<f64>, gutter: f64, center: bool, output: &mut Vec<f64>, control: &Interrupt<'_>) -> Result<()> {
    let available = (end - start).max(0.);
    if count == 0 {
        return poll(control);
    }
    let fixed = width.is_some();
    let n = f64::from(count);
    let width = width.unwrap_or(((available - (n - 1.) * gutter) / n).max(0.));
    let total = n * width + (n - 1.) * gutter;
    // Match the shared preview's admitted geometry before producing lines.
    // A fixed width or gutter must fit, rather than silently escaping margins.
    if width <= 0. || !total.is_finite() || total > available + 0.001 {
        return Err(error("count, size and gutter must fit inside the target margins"));
    }
    let start = if center && fixed { start + (available - total) / 2. } else { start };
    for index in 0..count {
        poll(control)?;
        let left = start + f64::from(index) * (width + gutter);
        unique(output, left, control)?;
        unique(output, left + width, control)?;
    }
    Ok(())
}
fn copied(values: &[f64], control: &Interrupt<'_>) -> Result<Vec<f64>> {
    let mut output = Vec::new();
    output.try_reserve_exact(values.len()).map_err(|_| error("guide copy allocation denied"))?;
    for (index, value) in values.iter().enumerate() {
        if index.is_multiple_of(1024) {
            poll(control)?;
        }
        if !value.is_finite() {
            return Err(error("nonfinite retained guide"));
        }
        output.push(*value);
    }
    poll(control)?;
    Ok(output)
}
fn shape_bounds(document: &Document, id: LayerId, control: &Interrupt<'_>) -> Result<[f64; 4]> {
    poll(control)?;
    // Validate actual native/imported ancestry. Reading bounds does not edit the
    // layer, so locks are not interpreted as new permission to change its pixels.
    crate::xcf_item_locks::effective_layer_locks(document, id)?;
    if let Some(state) = &document.xcf_state {
        if state.items.len() > 10_000 || state.source_items.len() > 10_000 || state.filter_stacks.len() > 10_000 {
            return Err(error("shape relation count limit"));
        }
        for item in &state.items {
            poll(control)?;
            if item.item == photocraft_doc::item_identity::ItemRef::Layer(id) && (item.composition.is_some() || item.mask.is_some() || item.source_flags != 0) {
                return Err(error("imported shape mask/composition/flag extent interpretation is not qualified"));
            }
        }
        for source in &state.source_items {
            poll(control)?;
            if source.owner == photocraft_doc::item_identity::ItemRef::Layer(id) {
                return Err(error("source-owned shape extent interpretation is not qualified"));
            }
        }
        for stack in &state.filter_stacks {
            poll(control)?;
            if stack.owner == photocraft_doc::item_identity::ItemRef::Layer(id) {
                return Err(error("filtered shape extent interpretation is not qualified"));
            }
        }
    }
    let layer = document.layer(id).ok_or(EngineError::NoLayer(id))?;
    if layer.import_compositing.is_some() || layer.indexed_source.is_some() || layer.video.is_some() {
        return Err(error("imported semantic shape extent is not qualified"));
    }
    let LayerContent::Shape(shape) = &layer.content else {
        return Err(error("layer is not a shape"));
    };
    if shape.path.subpaths.len() > 10_000 {
        return Err(error("shape subpath limit"));
    }
    let mut knots = 0usize;
    let mut bounds: Option<[f64; 4]> = None;
    for subpath in &shape.path.subpaths {
        poll(control)?;
        knots = knots.checked_add(subpath.knots.len()).filter(|knots| *knots <= 100_000).ok_or_else(|| error("shape knot work limit"))?;
        for knot in &subpath.knots {
            poll(control)?;
            for point in [knot.anchor, knot.in_ctrl, knot.out_ctrl] {
                if !point.x.is_finite() || !point.y.is_finite() {
                    return Err(error("nonfinite shape control point"));
                }
                bounds = Some(match bounds {
                    None => [point.x, point.y, point.x, point.y],
                    Some([x0, y0, x1, y1]) => [x0.min(point.x), y0.min(point.y), x1.max(point.x), y1.max(point.y)],
                });
            }
        }
    }
    bounds.ok_or_else(|| error("shape has no path"))
}
fn prepare_targets(
    snapshot: &crate::DocumentSnapshot,
    plan: Plan,
    targets: &[photocraft_geom::Rect],
    selected: bool,
    control: &Interrupt<'_>,
) -> Result<(Guides, Value)> {
    poll(control)?;
    snapshot.validate_resident()?;
    let added = match plan {
        Plan::Layout(layout) => {
            (u64::from(layout.columns + layout.rows) * 2 + 4).checked_mul(targets.len() as u64).ok_or_else(|| error("guide target work overflow"))?
        }
        Plan::Shape(_) => 6,
    };
    let existing = snapshot.guides.horizontal.len().checked_add(snapshot.guides.vertical.len()).ok_or_else(|| error("guide count overflow"))? as u64;
    if added.checked_mul(existing.checked_add(added).ok_or_else(|| error("work overflow"))?).is_none_or(|work| work > 8_000_000_000) {
        return Err(error("guide duplicate-scan work limit"));
    }
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(snapshot, limits, control).map_err(|e| controlled(control, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, limits, control).map_err(|e| controlled(control, e))? as u64;
    let raster = snapshot.resident_raster_footprint_with(control)?;
    if model
        .checked_mul(4)
        .and_then(|bytes| bytes.checked_add(payload))
        .and_then(|bytes| bytes.checked_add(raster.retained_bytes))
        .and_then(|bytes| bytes.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|bytes| bytes.checked_add(2_000_000))
        .is_none_or(|bytes| bytes > MAX_CHECKED_RASTER_BYTES)
    {
        return Err(error("guide/source metadata residency limit"));
    }
    let (vertical, horizontal, clear, response) = match plan {
        Plan::Layout(layout) => {
            let [top, left, bottom, right] = layout.margin;
            let mut vertical = Vec::new();
            let mut horizontal = Vec::new();
            if targets.is_empty() {
                return Err(error("select at least one artboard"));
            }
            for rect in targets {
                poll(control)?;
                let (x0, y0, x1, y1) = (f64::from(rect.x0) + left, f64::from(rect.y0) + top, f64::from(rect.x1) - right, f64::from(rect.y1) - bottom);
                if x0 >= x1 || y0 >= y1 {
                    return Err(error("margins must leave space inside the target"));
                }
                if layout.margin.iter().any(|value| *value != 0.) {
                    unique(&mut vertical, x0, control)?;
                    unique(&mut vertical, x1, control)?;
                    unique(&mut horizontal, y0, control)?;
                    unique(&mut horizontal, y1, control)?;
                }
                lines(x0, x1, layout.columns, layout.width, layout.gutter, layout.center, &mut vertical, control)?;
                lines(y0, y1, layout.rows, layout.height, layout.row_gutter, layout.center, &mut horizontal, control)?;
            }
            if vertical.is_empty() && horizontal.is_empty() {
                return Err(error("give columns, rows or a margin"));
            }
            let response = json!({"vertical":vertical.len(),"horizontal":horizontal.len()});
            (vertical, horizontal, layout.clear, response)
        }
        Plan::Shape(id) => {
            let [x0, y0, x1, y1] = shape_bounds(snapshot, id, control)?;
            let mut vertical = Vec::new();
            let mut horizontal = Vec::new();
            for x in [x0, (x0 + x1) / 2., x1] {
                unique(&mut vertical, x, control)?;
            }
            for y in [y0, (y0 + y1) / 2., y1] {
                unique(&mut horizontal, y, control)?;
            }
            (vertical, horizontal, false, json!({"bounds":[x0,y0,x1,y1]}))
        }
    };
    let mut result = if clear && !selected {
        Guides::default()
    } else {
        Guides {
            horizontal: copied(&snapshot.guides.horizontal, control)?,
            vertical: copied(&snapshot.guides.vertical, control)?,
            colors: copied_colors(&snapshot.guides.colors, control)?,
        }
    };
    if clear && selected {
        result.vertical.retain(|v| !targets.iter().any(|r| *v >= f64::from(r.x0) && *v <= f64::from(r.x1)));
        result.horizontal.retain(|v| !targets.iter().any(|r| *v >= f64::from(r.y0) && *v <= f64::from(r.y1)));
        result.prune_colors();
    }
    for value in vertical {
        poll(control)?;
        unique(&mut result.vertical, value, control)?;
    }
    for value in horizontal {
        poll(control)?;
        unique(&mut result.horizontal, value, control)?;
    }
    poll(control)?;
    Ok((result, response))
}
pub(crate) fn run(session: &mut Session, params: &Value, shape: bool) -> Result<Value> {
    dispatch_preflight(session, params, shape, false)?;
    let plan = plan(session, params, shape)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let selected = !shape && params.get("target").and_then(Value::as_str) == Some("selectedArtboards");
    let mut targets = Vec::new();
    if selected {
        targets = crate::guide_layout::try_selected_artboards(session)?;
        if targets.is_empty() {
            return Err(error("select at least one artboard"));
        }
    } else {
        targets.try_reserve_exact(1).map_err(|_| error("target allocation denied"))?;
        targets.push(baseline.bounds());
    }
    let work = baseline.clone();
    let label = if shape { "New Guides From Shape" } else { "New Guide Layout" };
    crate::jobs::run(
        session,
        label,
        true,
        move |ctx| {
            ctx.stage(0., 0.95, label, |control| {
                let (guides, response) = prepare_targets(&work, plan, &targets, selected, control)?;
                Ok((guides, response, ctx.clone()))
            })
        },
        move |session, (guides, response, ctx)| {
            let state = session.active().ok_or(EngineError::NoDocument)?;
            if state.doc.id != baseline.id || state.revision != revision || !state.doc.shares_state(&baseline) {
                return Err(error("guide target changed during preparation"));
            }
            ctx.stage(0.95, 1., label, |control| {
                poll(control)?;
                if exact_guides(&baseline.guides, &guides, control)? {
                    return Ok(response);
                }
                session.edit_with_interrupt(label, control, move |document, _| {
                    document.guides = guides;
                    Ok(())
                })?;
                Ok(response)
            })
        },
    )
}
fn exact_guides(first: &Guides, second: &Guides, control: &Interrupt<'_>) -> Result<bool> {
    let same = |first: &[f64], second: &[f64]| -> Result<bool> {
        if first.len() != second.len() {
            return Ok(false);
        }
        for (index, (first, second)) in first.iter().zip(second).enumerate() {
            if index.is_multiple_of(1024) {
                poll(control)?;
            }
            if first.to_bits() != second.to_bits() {
                return Ok(false);
            }
        }
        poll(control)?;
        Ok(true)
    };
    Ok(same(&first.horizontal, &second.horizontal)? && same(&first.vertical, &second.vertical)? && first.colors == second.colors)
}

fn copied_colors(values: &[photocraft_doc::GuideColor], control: &Interrupt<'_>) -> Result<Vec<photocraft_doc::GuideColor>> {
    if values.len() > 100_000 {
        return Err(error("guide color metadata limit"));
    }
    let mut output = Vec::new();
    output.try_reserve_exact(values.len()).map_err(|_| error("guide color allocation denied"))?;
    for value in values {
        poll(control)?;
        if !value.position.is_finite() {
            return Err(error("nonfinite guide color position"));
        }
        output.push(*value);
    }
    Ok(output)
}

#[cfg(test)]
fn prepare(snapshot: &crate::DocumentSnapshot, plan: Plan, control: &Interrupt<'_>) -> Result<(Guides, Value)> {
    prepare_targets(snapshot, plan, &[snapshot.bounds()], false, control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{Color, ColorMode, SampleType};
    use photocraft_doc::{Layer, Path, PhysicalResolution, ShapeLayer, Size, Subpath};
    fn fixture() -> Session {
        let mut document = Document::with_background("guide layout", Size::new(100, 80), ColorMode::Rgb, SampleType::F64, Color::WHITE);
        document.physical_resolution = Some(PhysicalResolution { y_dpi: 288., ..PhysicalResolution::square(144.) });
        document.resolution_dpi = 72.;
        document.guides.vertical = vec![5.];
        let mut session = Session::new();
        session.add_document(document, None).unwrap();
        session
    }
    #[test]
    fn old_layout_arithmetic_and_duplicate_tolerance_are_exact_oracles() {
        let session = fixture();
        let snapshot = &session.active().unwrap().doc;
        for params in [
            json!({"columns":2,"rows":3}),
            json!({"columns":2,"width":20,"gutter":10,"centerColumns":true,"clearExisting":true}),
            json!({"columns":3,"rows":2,"margin":[5,5,10,15],"gutter":2.5,"rowGutter":3}),
        ] {
            let Plan::Layout(layout) = plan(&session, &params, false).unwrap() else { panic!("layout expected") };
            let [top, left, bottom, right] = layout.margin;
            let mut vertical = Vec::new();
            let mut horizontal = Vec::new();
            if layout.margin.iter().any(|value| *value != 0.) {
                super::super::push_unique(&mut vertical, left);
                super::super::push_unique(&mut vertical, 100. - right);
                super::super::push_unique(&mut horizontal, top);
                super::super::push_unique(&mut horizontal, 80. - bottom);
            }
            super::super::layout_lines(left, 100. - right, layout.columns, layout.width, layout.gutter, layout.center, &mut vertical);
            super::super::layout_lines(top, 80. - bottom, layout.rows, layout.height, layout.row_gutter, layout.center, &mut horizontal);
            let response = json!({"vertical":vertical.len(),"horizontal":horizontal.len()});
            let mut expected = if layout.clear { Guides::default() } else { snapshot.guides.clone() };
            for value in vertical {
                super::super::push_unique(&mut expected.vertical, value);
            }
            for value in horizontal {
                super::super::push_unique(&mut expected.horizontal, value);
            }
            let (actual, actual_response) = prepare(snapshot, Plan::Layout(layout), &Interrupt::NONE).unwrap();
            assert!(exact_guides(&actual, &expected, &Interrupt::NONE).unwrap());
            assert_eq!(actual_response, response);
        }
    }
    #[test]
    fn shape_bounds_match_current_path_and_preserve_other_fields_one_undo() {
        let mut session = fixture();
        let path = Path::new(vec![Subpath::polygon(&[(-10., 5.), (30., 5.), (30., 25.), (-10., 25.)])]);
        let layer =
            Layer::new("Shape", LayerContent::Shape(ShapeLayer { path, fill: Some(photocraft_doc::Fill::Solid(Color::BLACK)), ..ShapeLayer::default() }));
        let id = layer.id;
        session
            .edit("shape fixture", |document, active| {
                document.layers.push(layer);
                *active = Some(id);
                Ok(())
            })
            .unwrap();
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.past_len();
        let LayerContent::Shape(shape) = &before.layer(id).unwrap().content else { panic!("shape expected") };
        let expected = shape.path.control_bounds().unwrap();
        assert_eq!(shape_bounds(&before, id, &Interrupt::NONE).unwrap(), [expected.0, expected.1, expected.2, expected.3]);
        let response = run(&mut session, &json!({"layer":id.0}), true).unwrap();
        assert_eq!(response["bounds"], json!([-10., 5., 30., 25.]));
        let state = session.active().unwrap();
        assert_eq!(state.history.past_len(), history + 1);
        assert_eq!(state.doc.guides.vertical, vec![5., -10., 10., 30.]);
        assert_eq!(state.doc.guides.horizontal, vec![5., 15., 25.]);
        assert_eq!(state.doc.physical_resolution, before.physical_resolution);
        assert_eq!(state.doc.resolution_dpi, before.resolution_dpi);
        assert_eq!(state.doc.depth, before.depth);
        assert_eq!(state.doc.layer(id).unwrap().content, before.layer(id).unwrap().content);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.guides, before.guides);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.guides.horizontal, vec![5., 15., 25.]);
    }
    #[test]
    fn identical_layout_noop_keeps_revision_and_redo() {
        let mut session = fixture();
        let params = json!({"columns":2,"clearExisting":true});
        run(&mut session, &params, false).unwrap();
        session
            .edit("redo fixture", |document, _| {
                document.name = "renamed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", json!({})).unwrap();
        let state = session.active().unwrap();
        let before = state.doc.clone();
        let revision = state.revision;
        let redo = state.history.redo_label().unwrap().to_string();
        run(&mut session, &params, false).unwrap();
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().history.redo_label(), Some(redo.as_str()));
    }
    #[test]
    fn upstream_negative_layout_sizes_refuse_without_touching_native_history_or_redo() {
        let mut session = fixture();
        session
            .edit("Retained guide redo", |document, _| {
                document.layers[0].name.push_str(" redo");
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let state = session.active().unwrap();
        let before = state.doc.clone();
        let revision = state.revision;
        let past = state.history.entry_labels().map(str::to_string).collect::<Vec<_>>();
        let redo = state.history.redo_labels().map(str::to_string).collect::<Vec<_>>();
        assert!(!redo.is_empty());
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        for params in [json!({"margin":-2.25}), json!({"columns":2,"width":0,"gutter":-2})] {
            assert!(run(&mut session, &params, false).is_err());
            let state = session.active().unwrap();
            assert!(state.doc.shares_state(&before));
            assert_eq!(state.revision, revision);
            assert_eq!(state.history.entry_labels().collect::<Vec<_>>(), past.iter().map(String::as_str).collect::<Vec<_>>());
            assert_eq!(state.history.redo_labels().collect::<Vec<_>>(), redo.iter().map(String::as_str).collect::<Vec<_>>());
            assert_eq!(photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(), native);
        }
        assert!(session.redo());
        assert!(session.active().unwrap().doc.layers[0].name.ends_with(" redo"));
    }

    #[test]
    fn stale_hostile_and_cancelled_layouts_do_not_publish() {
        let session = fixture();
        let state = session.active().unwrap();
        let before = state.doc.clone();
        for params in [
            json!({"columns":1001}),
            json!({"margin":[1,2]}),
            json!({"columns":2,"width":"20"}),
            json!({"columns":2,"centerColumns":1}),
            json!({"columns":2,"documentId":before.id.0.to_string(),"expectedRevision":state.revision.saturating_add(1).to_string()}),
        ] {
            assert!(dispatch_preflight(&session, &params, false, false).is_err());
        }
        let params = json!({"columns":1000,"rows":1000});
        let planned = plan(&session, &params, false).unwrap();
        let cancel = || true;
        assert!(matches!(prepare(&before, planned, &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
        assert!(state.doc.shares_state(&before));
    }
}
