//! Controlled ordinary one-shot strokes and upstream preset/path coverage unions.
//! Live commits and wide math retain their existing parent routes.
use crate::{EngineError, Result, Session};
use photocraft_color::SampleType;
use photocraft_paint::stroke_controlled::{
    StrokeError, StrokeLimits, reflected_passes_with_interrupt, render_stroke_passes_with_interrupt, render_stroke_with_interrupt,
};
use photocraft_paint::{BrushSettings, StrokePoint};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::Value;

#[path = "live_batch_controlled.rs"]
pub(super) mod live_batch_controlled;
#[path = "live_transaction_controlled.rs"]
pub(super) mod live_transaction_controlled;
#[path = "path_symmetry_controlled.rs"]
mod path_symmetry_controlled;

fn bad(cmd: &str, message: impl Into<String>) -> EngineError {
    super::bad(cmd, message)
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn paint_error(cmd: &str, error: StrokeError) -> EngineError {
    match error {
        StrokeError::Cancelled => EngineError::Cancelled,
        error => bad(cmd, error.to_string()),
    }
}
/// Prepare every upstream mirror in its original order. Identity mirrors are
/// retained here: callers apply the existing primary-only distinctness filter,
/// so two different mirrors that coincide are never incorrectly deduplicated.
/// No stylus field is evaluated or normalized by this geometry-only boundary.
pub(super) fn prepare_reflected_passes(
    axis: &crate::symmetry_cmds::PaintingSymmetry,
    points: &[StrokePoint],
    limits: StrokeLimits,
    control: &Interrupt<'_>,
) -> Result<Vec<Vec<StrokePoint>>> {
    poll(control)?;
    let standard = StrokeLimits::default();
    if limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units || points.len() > 100_000 {
        return Err(EngineError::Other("painting symmetry resource limit".into()));
    }
    for point in points {
        poll(control)?;
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(EngineError::Other("painting symmetry requires finite coordinates".into()));
        }
    }
    let mirrors = match axis {
        crate::symmetry_cmds::PaintingSymmetry::Path(axis) => {
            let (captured, mirrors, _) = path_symmetry_controlled::PreparedAxis::prepare(axis, points, limits, control)?;
            drop(captured);
            mirrors
        }
        crate::symmetry_cmds::PaintingSymmetry::Preset(preset) => {
            // Reserve the scalar validation and caller's primary/mirror dedup
            // comparisons as well as the lower helper's exact reflection work.
            let outer_work = (points.len() as u64)
                .checked_mul((preset.mirror_count() as u64) + 1)
                .and_then(|work| work.checked_mul(16))
                .ok_or_else(|| EngineError::Other("painting symmetry work overflow".into()))?;
            let remaining = limits.work_units.checked_sub(outer_work).ok_or_else(|| EngineError::Other("painting symmetry work limit".into()))?;
            reflected_passes_with_interrupt(*preset, points, StrokeLimits { work_units: remaining, ..limits }, control).map_err(|error| match error {
                StrokeError::Cancelled => EngineError::Cancelled,
                error => EngineError::Other(format!("painting symmetry: {error}")),
            })?
        }
    };
    poll(control)?;
    Ok(mirrors)
}

fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || (text.len() > 1 && text.starts_with('0')) || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
// Charge every node, including numeric-only trees, before any JSON cloning or
// brush deserialization. This admits retained stored-tip payloads, not a grant.
fn shape(value: &Value, depth: usize, visits: &mut usize, bytes: &mut usize, cmd: &str) -> Result<()> {
    if depth > 16 {
        return Err(bad(cmd, "stroke parameter depth limit"));
    }
    *visits = visits.checked_add(1).filter(|count| *count <= 1_000_000).ok_or_else(|| bad(cmd, "stroke parameter visit limit"))?;
    *bytes = bytes.checked_add(std::mem::size_of::<Value>()).filter(|count| *count <= 64 << 20).ok_or_else(|| bad(cmd, "stroke parameter byte limit"))?;
    match value {
        Value::String(text) => {
            *bytes = bytes.checked_add(text.len()).filter(|count| *count <= 64 << 20).ok_or_else(|| bad(cmd, "stroke parameter byte limit"))?;
        }
        Value::Array(values) => {
            for value in values {
                shape(value, depth + 1, visits, bytes, cmd)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                *bytes = bytes.checked_add(key.len()).filter(|count| *count <= 64 << 20).ok_or_else(|| bad(cmd, "stroke parameter byte limit"))?;
                shape(value, depth + 1, visits, bytes, cmd)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Borrowed preflight, intended before the parent's point/brush preparation.
/// It does not render, load a tip, normalize floating pixels, or take a live commit.
pub(super) fn dispatch_preflight(session: &Session, cmd: &str, params: &Value, _background: bool) -> Result<()> {
    if !params.is_object() {
        return Err(bad(cmd, "stroke parameters must be an object"));
    }
    shape(params, 0, &mut 0, &mut 0, cmd)?;
    let points = params.get("points").and_then(Value::as_array).ok_or_else(|| bad(cmd, "missing points"))?;
    if points.is_empty() || points.len() > 100_000 {
        return Err(bad(cmd, "stroke point count limit"));
    }
    for key in ["size", "hardness", "opacity", "flow", "spacing", "smoothing", "zoom"] {
        if let Some(value) = params.get(key)
            && value.as_f64().is_none_or(|value| !value.is_finite() || !(value as f32).is_finite())
        {
            return Err(bad(cmd, format!("{key} must be a finite representable number")));
        }
    }
    if params.get("layer").is_some_and(|value| value.as_u64().filter(|id| *id != 0).is_none()) {
        return Err(bad(cmd, "layer must be a nonzero unsigned integer"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let captured = match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => false,
        (Some(id), Some(revision)) if state.doc.id.0 != 0 && decimal(id) == Some(state.doc.id.0) && decimal(revision) == Some(state.revision) => true,
        _ => return Err(bad(cmd, "captured stroke target changed or incomplete")),
    };
    if state.revision == u64::MAX || session.command_has_job_on(state.doc.id) || session.command_has_pending_job() {
        return Err(bad(cmd, "stroke target is busy or its revision is exhausted"));
    }
    if captured && (state.floating.is_some() || state.coalesce.is_some() || session.coalesce_request.is_some()) {
        return Err(bad(cmd, "finish transient work before a captured stroke"));
    }
    Ok(())
}

fn brush_assets(brush: &BrushSettings, cmd: &str) -> Result<u64> {
    let mut bytes = 0u64;
    for stored in [&brush.tip, &brush.dual_brush.tip]
        .into_iter()
        .filter_map(|tip| match tip {
            photocraft_paint::TipShape::Stored(stored) => Some(stored),
            _ => None,
        })
        .chain(std::iter::once(&brush.texture.pattern).filter_map(|pattern| match pattern {
            photocraft_paint::brush::Pattern::Stored(stored) => Some(stored),
            _ => None,
        }))
    {
        bytes = bytes
            .checked_add((stored.key.capacity() as u64).checked_mul(4).ok_or_else(|| bad(cmd, "brush asset key accounting overflow"))?)
            .filter(|bytes| *bytes <= MAX_CHECKED_RASTER_BYTES)
            .ok_or_else(|| bad(cmd, "brush asset key limit"))?;
    }
    for tile in [brush.tip.bitmap(), brush.dual_brush.tip.bitmap(), brush.texture.pattern.bitmap()].into_iter().flatten() {
        let count = u64::from(tile.width)
            .checked_mul(u64::from(tile.height))
            .filter(|count| *count > 0 && *count == tile.data.len() as u64)
            .ok_or_else(|| bad(cmd, "invalid retained brush asset"))?;
        bytes = bytes
            .checked_add(
                (tile.data.len() as u64).checked_mul(16).and_then(|bytes| bytes.checked_add(32)).ok_or_else(|| bad(cmd, "brush asset accounting overflow"))?,
            )
            .and_then(|bytes| bytes.checked_add(count.checked_mul(16)?))
            .filter(|bytes| *bytes <= MAX_CHECKED_RASTER_BYTES)
            .ok_or_else(|| bad(cmd, "brush asset limit"))?;
    }
    Ok(bytes)
}

/// Returns None without taking live pixels or editing for routes still owned by
/// the parent: live-commit reuse and existing qualified wide math.
#[allow(clippy::too_many_arguments)]
pub(super) fn try_run(
    session: &mut Session,
    cmd: &str,
    params: &Value,
    label: &str,
    brush: &BrushSettings,
    points: &[StrokePoint],
    auto_erase: bool,
) -> Result<Option<Value>> {
    try_run_with_interrupt(session, cmd, params, label, brush, points, auto_erase, &Interrupt::NONE)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn try_run_with_interrupt(
    session: &mut Session,
    cmd: &str,
    params: &Value,
    label: &str,
    brush: &BrushSettings,
    points: &[StrokePoint],
    auto_erase: bool,
    control: &Interrupt<'_>,
) -> Result<Option<Value>> {
    poll(control)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if session.live_commit.as_ref().is_some_and(|commit| commit.matches(session, cmd, params, points)) {
        return Ok(None);
    }
    let (preset, path_axis) = match state.symmetry.as_ref() {
        None => (None, None),
        Some(crate::symmetry_cmds::PaintingSymmetry::Preset(preset)) => (Some(*preset), None),
        Some(crate::symmetry_cmds::PaintingSymmetry::Path(axis)) => (None, Some(axis)),
    };
    let id = if crate::channel_cmds::is_channel_target(params) { None } else { Some(super::layer_id(session, params)?) };
    let source = crate::channel_cmds::read_target_surface(&state.doc, id, params)?;
    if !matches!(source.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32) {
        return Ok(None);
    }
    dispatch_preflight(session, cmd, params, false)?;
    super::check_coords(points, cmd)?;
    if points.len() > 100_000 {
        return Err(bad(cmd, "stroke point count limit"));
    }
    let snapshot = state.doc.clone();
    let weak = snapshot.downgrade();
    let revision = state.revision;
    let active = state.active_layer;
    let background = session.tools.background;
    let foreground = brush.color;
    let cmyk = super::doc_cmyk_space(session);
    // Admit the existing model clone plus lower private generation/output work.
    // This is conservative retained ownership accounting, not an allocator/RSS proof.
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(&snapshot, limits, control)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(cmd, error.to_string()) })? as u64;
    let model = photocraft_io::retained_native_model_bytes(&snapshot, limits, control)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(cmd, error.to_string()) })? as u64;
    let raster = crate::DocumentSnapshot::resident_raster_footprint_of(&snapshot, control)?;
    let assets = brush_assets(brush, cmd)?;
    let retained = payload
        .checked_add(assets)
        .and_then(|count| count.checked_add(model.checked_mul(4)?))
        .and_then(|count| count.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|count| count.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|count| count.checked_add(1 << 20))
        .ok_or_else(|| bad(cmd, "stroke ownership accounting overflow"))?;
    let mut available = MAX_CHECKED_RASTER_BYTES.checked_sub(retained).filter(|bytes| *bytes > 0).ok_or_else(|| bad(cmd, "stroke ownership limit"))?;
    let (captured_path, mirrors, reflection_work) = if let Some(axis) = path_axis {
        let (captured, mirrors, work) =
            path_symmetry_controlled::PreparedAxis::prepare(axis, points, StrokeLimits { resident_bytes: available, ..StrokeLimits::default() }, control)?;
        available = available.checked_sub(captured.retained_bytes).filter(|bytes| *bytes > 0).ok_or_else(|| bad(cmd, "path symmetry ownership limit"))?;
        (Some(captured), mirrors, work)
    } else {
        let work = (points.len() as u64)
            .checked_mul(preset.map_or(0, |preset| preset.mirror_count()) as u64)
            .and_then(|count| count.checked_mul(32))
            .ok_or_else(|| bad(cmd, "symmetry work overflow"))?;
        let mirrors = match preset {
            Some(preset) => reflected_passes_with_interrupt(preset, points, StrokeLimits { resident_bytes: available, ..StrokeLimits::default() }, control)
                .map_err(|error| paint_error(cmd, error))?,
            None => Vec::new(),
        };
        (None, mirrors, work)
    };
    let mut reflected_bytes =
        (mirrors.capacity() as u64).checked_mul(std::mem::size_of::<Vec<StrokePoint>>() as u64).ok_or_else(|| bad(cmd, "symmetry ownership overflow"))?;
    for pass in &mirrors {
        poll(control)?;
        reflected_bytes = reflected_bytes
            .checked_add(
                (pass.capacity() as u64).checked_mul(std::mem::size_of::<StrokePoint>() as u64).ok_or_else(|| bad(cmd, "symmetry ownership overflow"))?,
            )
            .ok_or_else(|| bad(cmd, "symmetry ownership overflow"))?;
    }
    available = available.checked_sub(reflected_bytes).filter(|bytes| *bytes > 0).ok_or_else(|| bad(cmd, "symmetry ownership limit"))?;
    let work_units = StrokeLimits::default().work_units.checked_sub(reflection_work).filter(|work| *work > 0).ok_or_else(|| bad(cmd, "symmetry work limit"))?;
    // Upstream compares every reflected pass with the primary, preserving order
    // and retaining distinct mirrors even when two mirrors agree with each other.
    let mut passes = [points; 4];
    let mut pass_count = 1usize;
    for pass in &mirrors {
        if pass.len() != points.len() {
            return Err(bad(cmd, "symmetry point count mismatch"));
        }
        let mut distinct = false;
        for (primary, reflected) in points.iter().zip(pass) {
            poll(control)?;
            if (primary.x - reflected.x).abs() > 1.0e-6 || (primary.y - reflected.y).abs() > 1.0e-6 {
                distinct = true;
                break;
            }
        }
        if distinct {
            *passes.get_mut(pass_count).ok_or_else(|| bad(cmd, "symmetry pass count limit"))? = pass.as_slice();
            pass_count = pass_count.checked_add(1).ok_or_else(|| bad(cmd, "symmetry pass count overflow"))?;
        }
    }
    poll(control)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let symmetry_matches = match (captured_path.as_ref(), state.symmetry.as_ref()) {
        (Some(captured), Some(crate::symmetry_cmds::PaintingSymmetry::Path(axis))) => captured.matches(axis, control)?,
        (None, Some(crate::symmetry_cmds::PaintingSymmetry::Preset(current))) => preset == Some(*current),
        (None, None) => preset.is_none(),
        _ => false,
    };
    if state.doc.id != snapshot.id
        || state.revision != revision
        || state.active_layer != active
        || !symmetry_matches
        || !weak.is_current(&state.doc)
        || !state.doc.shares_state(&snapshot)
    {
        return Err(bad(cmd, "captured stroke state changed"));
    }
    let (id, mut resolved, zoom) = super::stroke_target(session, params, brush.clone())?;
    let damage = photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
        session.edit_with_interrupt(label, control, |doc, _| {
            poll(control)?;
            // Same retained selection as the frozen document, with no map clone.
            let selection = snapshot.selection.as_ref();
            let (surface, lock) = crate::channel_cmds::target_surface(doc, id, params)?;
            super::erase_locked(&mut resolved, lock, background);
            if auto_erase {
                super::apply_auto_erase(&mut resolved, surface, points.first(), foreground, background);
            }
            let limits = StrokeLimits { resident_bytes: available, work_units };
            let damage = if pass_count == 1 {
                render_stroke_with_interrupt(surface, &resolved, points, selection, lock, zoom, limits, control)
            } else {
                let passes = passes.get(..pass_count).ok_or_else(|| bad(cmd, "symmetry pass bounds"))?;
                render_stroke_passes_with_interrupt(surface, &resolved, passes, selection, lock, zoom, limits, control)
            }
            .map_err(|error| paint_error(cmd, error))?;
            poll(control)?;
            Ok(damage)
        })
    })?;
    Ok(Some(super::damage_json(session, damage)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn captured_stale_and_parameter_tree_refuse_without_editing() {
        let mut session = Session::new();
        session
            .add_document(photocraft_doc::Document::new("Own stroke", photocraft_geom::Size::new(2, 2), photocraft_doc::ColorMode::Rgb, SampleType::U8), None)
            .unwrap();
        let before = session.active().unwrap().doc.clone();
        assert!(
            dispatch_preflight(&session, "paint.stroke", &json!({"points":[[0,0]],"documentId":"1","expectedRevision":"18446744073709551615"}), false).is_err()
        );
        assert!(dispatch_preflight(&session, "paint.stroke", &json!({"points":[[0,0]],"zoom":"wrong"}), false).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
    }

    #[test]
    fn ordinary_pixels_match_existing_renderer_and_cancel_preserves_source() {
        use photocraft_doc::{ColorMode, Document, Layer, SampleType, Size};
        use photocraft_geom::Rect;
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = Document::new("Own controlled stroke", Size::new(16, 16), ColorMode::Rgb, sample);
            let mut layer = Layer::raster("Own pixels", doc.pixel_format());
            let id = layer.id;
            layer.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 16, 16), &[0.5, 0.25, 0.75, 1.0]);
            let mut expected = layer.surface().unwrap().clone();
            doc.layers.push(layer);
            let mut session = Session::new();
            session.add_document(doc, None).unwrap();
            let brush = BrushSettings { size: 3.0, pressure_size: false, color: [0.2, 0.7, 0.3, 1.0], seed: 3, ..Default::default() };
            let points = [StrokePoint::new(3.0, 3.0, 1.0), StrokePoint::new(9.0, 8.0, 0.7)];
            let params = json!({"layer":id.0,"points":[[3,3,1],[9,8,0.7]],"seed":3});
            photocraft_paint::render_stroke(&mut expected, &brush, &points, None, false, 1.0);
            let before = session.active().unwrap().doc.clone();
            let cancelled = || true;
            assert!(matches!(
                try_run_with_interrupt(&mut session, "paint.stroke", &params, "Brush Tool", &brush, &points, false, &Interrupt::cancel_only(&cancelled)),
                Err(EngineError::Cancelled)
            ));
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert!(try_run(&mut session, "paint.stroke", &params, "Brush Tool", &brush, &points, false).unwrap().is_some());
            assert_eq!(session.active().unwrap().doc.layer(id).unwrap().surface().unwrap(), &expected);
            assert!(session.undo_with_control(&crate::read_control_none()).unwrap());
            assert_eq!(&*session.active().unwrap().doc, &*before);
        }
    }

    #[test]
    fn preset_consumer_matches_upstream_union_and_keeps_one_undo() {
        use photocraft_doc::{ColorMode, Document, Layer, Size};
        use photocraft_geom::Rect;
        use photocraft_paint::{
            StrokeRenderer,
            symmetry::{PresetSymmetry, SymmetryMode},
        };
        for mode in SymmetryMode::ALL {
            let mut doc = Document::new("Own preset stroke", Size::new(16, 16), ColorMode::Rgb, SampleType::F32);
            let mut layer = Layer::raster("Own pixels", doc.pixel_format());
            let id = layer.id;
            layer.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 16, 16), &[0.5, 0.25, 0.75, 1.0]);
            let pre = layer.surface().unwrap().clone();
            let mut expected = pre.clone();
            doc.layers.push(layer);
            let mut session = Session::new();
            session.add_document(doc, None).unwrap();
            let preset = PresetSymmetry::new(mode, [8.0, 8.0], 17.0).unwrap();
            session.active_mut().unwrap().symmetry = Some(crate::symmetry_cmds::PaintingSymmetry::Preset(preset));
            let brush = BrushSettings {
                size: 4.0,
                pressure_size: false,
                opacity: 0.6,
                flow: 0.3,
                wet_edges: true,
                color: [0.2, 0.7, 0.3, 1.0],
                seed: 3,
                ..Default::default()
            };
            let points = [StrokePoint::new(3.0, 3.0, 1.0), StrokePoint::new(9.0, 8.0, 0.7)];
            let params = json!({"layer":id.0,"points":[[3,3,1],[9,8,0.7]],"seed":3});
            let mut original = StrokeRenderer::new(&brush, Some(pre.format()), 1.0);
            original.push(&points);
            original.finish();
            let copies: Vec<_> = preset
                .reflected_passes(&points)
                .iter()
                .filter(|pass| crate::symmetry_cmds::SymmetryAxis::has_distinct_mirror(&points, pass))
                .map(|pass| {
                    let mut renderer = StrokeRenderer::new(&brush, Some(pre.format()), 1.0);
                    renderer.push(pass);
                    renderer.finish();
                    renderer
                })
                .collect();
            let expected_damage = original.composite_union_many(&copies, &pre, &mut expected, None, false);
            let before = session.active().unwrap().doc.clone();
            let revision = session.active().unwrap().revision;
            let result = try_run(&mut session, "paint.stroke", &params, "Brush Tool", &brush, &points, false).unwrap().unwrap();
            assert_eq!(result["damage"], json!([expected_damage.x0, expected_damage.y0, expected_damage.width(), expected_damage.height()]));
            assert_eq!(session.active().unwrap().doc.layer(id).unwrap().surface().unwrap(), &expected);
            assert_eq!(session.active().unwrap().revision, revision + 1);
            assert!(session.undo_with_control(&crate::read_control_none()).unwrap());
            assert_eq!(&*session.active().unwrap().doc, &*before);
        }
    }
    #[test]
    fn path_consumer_matches_upstream_union_and_one_undo() {
        use photocraft_doc::{ColorMode, Document, Layer, Size};
        use photocraft_geom::Rect;
        use photocraft_paint::StrokeRenderer;
        let mut doc = Document::new("Own path stroke", Size::new(16, 16), ColorMode::Rgb, SampleType::F32);
        let mut layer = Layer::raster("Own pixels", doc.pixel_format());
        let id = layer.id;
        layer.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 16, 16), &[0.5, 0.25, 0.75, 1.0]);
        let pre = layer.surface().unwrap().clone();
        let mut expected = pre.clone();
        doc.layers.push(layer);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        let axis =
            crate::symmetry_cmds::SymmetryAxis { source: "Own curved samples".into(), segments: vec![[[8.0, 0.0], [8.0, 8.0]], [[8.0, 8.0], [12.0, 16.0]]] };
        session.active_mut().unwrap().symmetry = Some(crate::symmetry_cmds::PaintingSymmetry::Path(axis.clone()));
        let brush = BrushSettings { size: 4.0, pressure_size: false, opacity: 0.6, flow: 0.3, wet_edges: true, seed: 3, ..Default::default() };
        let points = [StrokePoint::new(3.0, 3.0, 1.0), StrokePoint::new(9.0, 8.0, 0.7)];
        let mut original = StrokeRenderer::new(&brush, Some(pre.format()), 1.0);
        original.push(&points);
        original.finish();
        let reflected = axis.reflect_points(&points);
        let mut mirror = StrokeRenderer::new(&brush, Some(pre.format()), 1.0);
        mirror.push(&reflected);
        mirror.finish();
        original.composite_union_many(&[mirror], &pre, &mut expected, None, false);
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let params = serde_json::json!({"layer":id.0,"points":[[3,3,1],[9,8,0.7]],"seed":3});
        assert!(try_run(&mut session, "paint.stroke", &params, "Brush Tool", &brush, &points, false).unwrap().is_some());
        assert_eq!(session.active().unwrap().doc.layer(id).unwrap().surface().unwrap(), &expected);
        assert_eq!(session.active().unwrap().revision, revision + 1);
        assert!(session.undo_with_control(&crate::read_control_none()).unwrap());
        assert_eq!(&*session.active().unwrap().doc, &*before);
    }

    #[test]
    fn shared_reflection_preparer_matches_upstream_order_and_raw_stylus_bits() {
        use crate::symmetry_cmds::{PaintingSymmetry, SymmetryAxis};
        use photocraft_paint::symmetry::{PresetSymmetry, SymmetryMode};
        let points = [StrokePoint {
            x: 2.0,
            y: 3.0,
            pressure: f32::from_bits(0x7fc01234),
            tilt_x: -0.0,
            time: f64::from_bits(0x8000000000000000),
            ..Default::default()
        }];
        let axes = [
            PaintingSymmetry::Path(SymmetryAxis { source: "Own axis".into(), segments: vec![[[8.0, 0.0], [8.0, 16.0]]] }),
            PaintingSymmetry::Preset(PresetSymmetry::new(SymmetryMode::Dual, [8.0, 8.0], 17.0).unwrap()),
        ];
        for axis in axes {
            let expected = axis.reflected_passes(&points);
            let actual = prepare_reflected_passes(&axis, &points, StrokeLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(actual.len(), expected.len());
            for (left, right) in actual.iter().zip(&expected) {
                for (left, right) in left.iter().zip(right) {
                    assert_eq!(left.x.to_bits(), right.x.to_bits());
                    assert_eq!(left.y.to_bits(), right.y.to_bits());
                    assert_eq!(left.pressure.to_bits(), right.pressure.to_bits());
                    assert_eq!(left.tilt_x.to_bits(), right.tilt_x.to_bits());
                    assert_eq!(left.time.to_bits(), right.time.to_bits());
                }
            }
        }
    }

    #[test]
    fn unrelated_live_commit_does_not_bypass_controlled_stroke() {
        use photocraft_doc::{ColorMode, Document, Layer, SampleType, Size};
        use photocraft_geom::Rect;
        let mut doc = Document::new("Stale live", Size::new(16, 16), ColorMode::Rgb, SampleType::U8);
        let layer = Layer::raster("Pixels", doc.pixel_format());
        let id = layer.id;
        let source = layer.surface().unwrap().clone();
        doc.layers.push(layer);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session.active_mut().unwrap().active_layer = Some(id);
        let state = session.active().unwrap();
        session.live_commit = Some(super::super::LiveCommit {
            cmd: "paint.pencil".into(),
            base: (state.doc.id, state.revision),
            params: json!({}),
            seed: None,
            points: vec![],
            surface: source.clone(),
            bounds: Rect::EMPTY,
        });
        let brush = BrushSettings { size: 3.0, pressure_size: false, color: [0.8, 0.1, 0.3, 1.0], ..Default::default() };
        let points = [StrokePoint::new(3.0, 3.0, 1.0)];
        let history = session.active().unwrap().history.past_len();
        let result = try_run(&mut session, "paint.stroke", &json!({"points":[[3,3,1]]}), "Brush", &brush, &points, false).unwrap();
        assert!(result.is_some());
        assert_eq!(session.active().unwrap().history.past_len(), history + 1);
        assert_ne!(session.active().unwrap().doc.layer(id).unwrap().surface().unwrap(), &source);
    }
}
