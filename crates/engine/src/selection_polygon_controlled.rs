//! Controlled upstream polygon selection, including band/default preservation.
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_algo::gimp::multi_input_coverage::{self as coverage, Operation, Params};
use photocraft_algo::selection::{self as sel, SelectionMode};
use photocraft_algo::selection_polygon_controlled as polygon;
use photocraft_color::checked_arithmetic::{Domain, Policy, Rounding, UnitOverflow};
use photocraft_color::{ColorMode, PixelFormat, SampleType, read_sample, write_sample};
use photocraft_doc::Document;
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
fn bad(cmd: &str, message: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: message.into() }
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn failure(cmd: &str, ctl: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if ctl.cancelled() { EngineError::Cancelled } else { bad(cmd, &error.to_string()) }
}
fn decimal(cmd: &str, v: &Value) -> Result<u64> {
    let s = v
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad(cmd, "canonical captured identity required"))?;
    s.parse().map_err(|_| bad(cmd, "captured identity range"))
}
#[derive(Clone, Copy)]
struct Options {
    mode: SelectionMode,
    aa: bool,
    feather: f32,
}
fn options(session: &Session, cmd: &str, p: &Value) -> Result<Options> {
    if cmd.len() > 128 || !p.is_null() && !p.as_object().is_some_and(|o| o.len() <= 64 && o.keys().all(|k| k.len() <= 256)) {
        return Err(bad(cmd, "bounded polygon parameters required"));
    }
    let mode = match p.get("mode") {
        None => SelectionMode::Replace,
        Some(Value::String(s)) if s.len() <= 64 => SelectionMode::parse(s),
        _ => return Err(bad(cmd, "bounded mode string required")),
    };
    let aa = match p.get("antiAlias") {
        None => true,
        Some(v) => v.as_bool().ok_or_else(|| bad(cmd, "antiAlias must be boolean"))?,
    };
    let feather = match p.get("feather") {
        None => 0.0,
        Some(v) => {
            let f = v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, "finite feather required"))? as f32;
            if !(0.0..=1000.0).contains(&f) {
                return Err(bad(cmd, "feather must be in 0..1000"));
            }
            f
        }
    };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if decimal(cmd, id)? == state.doc.id.0 && decimal(cmd, rev)? == state.revision => {}
        _ => return Err(bad(cmd, "captured polygon target changed")),
    }
    if state.revision == u64::MAX
        || state.floating.is_some()
        || state.coalesce.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.command_has_job_on(state.doc.id)
        || session.command_has_pending_job()
    {
        return Err(bad(cmd, "selection target must be idle"));
    }
    Ok(Options { mode, aa, feather })
}
fn json_points(cmd: &str, p: &Value, mut visitor: impl FnMut((f32, f32)) -> Result<()>) -> Result<usize> {
    if !p.as_object().is_some_and(|o| {
        o.len() <= 6 && o.keys().all(|k| matches!(k.as_str(), "points" | "mode" | "antiAlias" | "feather" | "documentId" | "expectedRevision"))
    }) {
        return Err(bad(cmd, "bounded Lasso object required"));
    }
    let values =
        p.get("points").and_then(Value::as_array).filter(|a| a.len() >= 3 && a.len() <= 4096).ok_or_else(|| bad(cmd, "Lasso requires 3..4096 points"))?;
    for value in values {
        let a = value.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad(cmd, "point must have two coordinates"))?;
        let coordinate = |v: &Value| {
            v.as_f64().filter(|n| n.is_finite() && n.abs() <= (1 << 29) as f64).map(|n| n as f32).ok_or_else(|| bad(cmd, "finite bounded point required"))
        };
        visitor((coordinate(a.first().ok_or_else(|| bad(cmd, "point x"))?)?, coordinate(a.get(1).ok_or_else(|| bad(cmd, "point y"))?)?))?;
    }
    Ok(values.len())
}
pub(crate) fn dispatch_preflight(session: &Session, p: &Value, _background: bool) -> Result<()> {
    options(session, "select.lasso", p)?;
    json_points("select.lasso", p, |_| Ok(()))?;
    Ok(())
}
fn rows(points: &[(f32, f32)], area: Rect, ctl: &Interrupt<'_>) -> Result<Rect> {
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for (i, (_, y)) in points.iter().enumerate() {
        if i.is_multiple_of(64) {
            poll(ctl)?;
        }
        lo = lo.min(*y);
        hi = hi.max(*y);
    }
    let clamp = |v: f32| v.clamp(area.y0 as f32, area.y1 as f32) as i32;
    let (y0, y1) = (clamp(lo.floor()), clamp(hi.ceil() + 1.0));
    Ok(if y0 >= y1 { Rect::EMPTY } else { Rect::new(area.x0, y0, area.x1, y1) })
}
struct Plan {
    area: Rect,
    format: PixelFormat,
    retain: bool,
    req: polygon::Requirements,
}
fn admission(doc: &Document, points: &[(f32, f32)], o: Options, cmd: &str, ctl: &Interrupt<'_>) -> Result<Plan> {
    poll(ctl)?;
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 300000 || doc.size.height > 300000 {
        return Err(bad(cmd, "canvas size limit"));
    }
    crate::selection_inverse_controlled::selection_write_authority(doc, ctl)?;
    if doc.selection.as_ref().is_some_and(|s| s.format().mode != ColorMode::Grayscale || s.format().alpha) {
        return Err(bad(cmd, "selection must be Gray without alpha"));
    }
    let area = if o.feather > 0.0 { doc.bounds() } else { rows(points, doc.bounds(), ctl)? };
    let retain = o.feather == 0.0 && matches!(o.mode, SelectionMode::Add | SelectionMode::Subtract) && doc.selection.is_some();
    let format = if let Some(old) = &doc.selection {
        if retain || matches!(old.format().sample, SampleType::U32 | SampleType::F64) { old.format() } else { PixelFormat::GRAY8 }
    } else if doc.selection_channel_id.is_some() && matches!(doc.depth, SampleType::U32 | SampleType::F64) {
        PixelFormat::new(ColorMode::Grayscale, doc.depth, false)
    } else {
        PixelFormat::GRAY8
    };
    let req = polygon::requirements_with_interrupt(points, area, o.aa, ctl).map_err(|e| failure(cmd, ctl, e))?;
    let tiles = if area.is_empty() {
        0
    } else {
        u64::try_from(
            (i64::from(area.x1 - 1).div_euclid(i64::from(TILE_SIZE)) - i64::from(area.x0).div_euclid(i64::from(TILE_SIZE)) + 1)
                * (i64::from(area.y1 - 1).div_euclid(i64::from(TILE_SIZE)) - i64::from(area.y0).div_euclid(i64::from(TILE_SIZE)) + 1),
        )
        .map_err(|_| bad(cmd, "tile count"))?
    };
    let tile_bytes = (TILE_SIZE as u64) * (TILE_SIZE as u64) * format.bytes_per_pixel() as u64;
    let pixels = area.size().area();
    let radius = (o.feather * 3.0).ceil() as u64;
    let blur = if o.feather > 0.0 {
        pixels
            .checked_mul(56)
            .and_then(|n| n.checked_add(radius.checked_mul(256)?))
            .and_then(|n| n.checked_add(u64::from(area.width().max(area.height())).checked_mul(64)?))
            .ok_or_else(|| bad(cmd, "feather storage"))?
    } else {
        0
    };
    let blur_work = if o.feather > 0.0 {
        pixels
            .checked_mul(radius.checked_mul(4).and_then(|n| n.checked_add(128)).ok_or_else(|| bad(cmd, "feather work"))?)
            .ok_or_else(|| bad(cmd, "feather work"))?
    } else {
        0
    };
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, ctl).map_err(|e| failure(cmd, ctl, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, ctl).map_err(|e| failure(cmd, ctl, e))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, ctl)?;
    raster
        .retained_bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|n| n.checked_add(model.checked_mul(5)?))
        .and_then(|n| n.checked_add(payload.checked_mul(2)?))
        .and_then(|n| n.checked_add(req.peak_resident_bytes))
        .and_then(|n| n.checked_add(blur))
        .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes.checked_add(1024)?)?))
        .and_then(|n| n.checked_add(tile_bytes.checked_mul(3)?))
        .and_then(|n| n.checked_add(65536))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad(cmd, "aggregate polygon resident limit"))?;
    req.work_units
        .checked_add(blur_work)
        .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes.checked_mul(8)?)?))
        .and_then(|n| n.checked_add(model.checked_add(payload)?.checked_mul(8)?))
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .filter(|n| *n <= 1 << 30)
        .ok_or_else(|| bad(cmd, "aggregate polygon work limit"))?;
    poll(ctl)?;
    Ok(Plan { area, format, retain, req })
}
fn q8(value: f32) -> u8 {
    (value.clamp(0., 1.) * 255. + 0.5) as u8
}
fn combine(old: &[u8], weight: f32, input: Option<SampleType>, output: PixelFormat, mode: SelectionMode, dst: &mut [u8], cmd: &str) -> Result<()> {
    let q = q8(weight);
    if matches!(output.sample, SampleType::U32 | SampleType::F64) {
        let mut new = [0u8; 8];
        let new = new.get_mut(..output.bytes_per_pixel()).ok_or_else(|| bad(cmd, "native shape width"))?;
        if output.sample == SampleType::U32 {
            new.copy_from_slice(&(u32::from(q) * 0x01010101).to_ne_bytes());
        } else {
            new.copy_from_slice(&(f64::from(q) / 255.).to_ne_bytes());
        }
        if mode == SelectionMode::Replace {
            dst.copy_from_slice(new);
            return Ok(());
        }
        let operation = match mode {
            SelectionMode::Add => Operation::UnionMax,
            SelectionMode::Subtract => Operation::SubtractSaturating,
            SelectionMode::Intersect => Operation::IntersectionMin,
            _ => return Err(bad(cmd, "native combine mode")),
        };
        coverage::evaluate_sample_into(
            output,
            old,
            new,
            dst,
            Params { operation, policy: Policy { domain: Domain::Unit, unit_overflow: UnitOverflow::Reject, rounding: Rounding::Nearest } },
        )
        .map_err(|e| bad(cmd, &e.to_string()))?;
    } else {
        let prev = if mode == SelectionMode::Replace { 0.0 } else { input.map_or(0., |sample| read_sample(old, sample, 0)) };
        if !prev.is_finite() {
            return Err(bad(cmd, "nonfinite previous coverage"));
        }
        let shape = f32::from(q) / 255.;
        let value = match mode {
            SelectionMode::Replace => weight,
            SelectionMode::Add => prev.max(shape),
            SelectionMode::Subtract => (prev - shape).max(0.),
            SelectionMode::Intersect => prev.min(shape),
        };
        // Upstream combine quantizes GRAY8 first, then band writes widen those
        // exact stored values back to the retained old ordinary storage depth.
        write_sample(dst, output.sample, 0, f32::from(q8(value)) / 255.);
    }
    Ok(())
}
fn prepare(doc: &Document, points: &[(f32, f32)], o: Options, cmd: &str, ctl: &Interrupt<'_>) -> Result<Option<Surface>> {
    let plan = admission(doc, points, o, cmd, ctl)?;
    if plan.area.is_empty() {
        poll(ctl)?;
        return Ok(if plan.retain { doc.selection.clone() } else { None });
    }
    let mask = polygon::polygon_with_interrupt(
        points,
        plan.area,
        o.aa,
        polygon::Limits { resident_bytes: plan.req.peak_resident_bytes, work_units: plan.req.work_units },
        ctl,
    )
    .map_err(|e| failure(cmd, ctl, e))?;
    let mask = if o.feather > 0.0 {
        sel::try_feather_with_interrupt(&mask, plan.area.width() as usize, plan.area.height() as usize, o.feather, ctl).map_err(|e| failure(cmd, ctl, e))?
    } else {
        mask
    };
    poll(ctl)?;
    let mut output = if plan.retain { doc.selection.clone().ok_or_else(|| bad(cmd, "missing retained selection"))? } else { Surface::new(plan.format) };
    let tile_len = (TILE_SIZE as usize) * (TILE_SIZE as usize) * plan.format.bytes_per_pixel();
    let mut raw = Vec::new();
    raw.try_reserve_exact(tile_len).map_err(|_| bad(cmd, "tile scratch allocation"))?;
    if raw.capacity() > tile_len.saturating_add(64) {
        return Err(bad(cmd, "tile scratch capacity"));
    }
    let mut previous = [0u8; 8];
    for coordinate in plan.area.tiles() {
        poll(ctl)?;
        let part = coordinate.rect().intersect(&plan.area);
        raw.clear();
        for y in part.y0..part.y1 {
            poll(ctl)?;
            for x in part.x0..part.x1 {
                poll(ctl)?;
                let offset = ((y - plan.area.y0) as usize)
                    .checked_mul(plan.area.width() as usize)
                    .and_then(|n| n.checked_add((x - plan.area.x0) as usize))
                    .ok_or_else(|| bad(cmd, "mask offset"))?;
                let weight = *mask.get(offset).ok_or_else(|| bad(cmd, "mask length"))?;
                let input = doc.selection.as_ref().map(|s| s.format().sample);
                let old_width = input.map_or(plan.format.bytes_per_pixel(), SampleType::bytes);
                let old = previous.get_mut(..old_width).ok_or_else(|| bad(cmd, "old word width"))?;
                old.fill(0);
                if let Some(source) = &doc.selection {
                    source.try_read_pixel_bytes(x, y, old).map_err(|e| failure(cmd, ctl, e))?;
                }
                let mut word = [0u8; 8];
                let dst = word.get_mut(..plan.format.bytes_per_pixel()).ok_or_else(|| bad(cmd, "output word width"))?;
                combine(old, weight, input, plan.format, o.mode, dst, cmd)?;
                raw.extend_from_slice(dst);
            }
        }
        // Only one private tile is cloned; output's growing map is never restaged.
        let mut replacement = Surface::try_with_default_bytes(plan.format, output.default_bytes()).map_err(|e| failure(cmd, ctl, e))?;
        if plan.retain
            && let Some(tile) = output.tile(coordinate)
        {
            replacement.put_tiles(std::iter::once((coordinate, std::sync::Arc::clone(tile))));
        }
        replacement.try_write_interleaved_with(part, &raw, ctl).map_err(|e| failure(cmd, ctl, e))?;
        poll(ctl)?;
        output.put_tiles(replacement.tiles().map(|(pos, tile)| (*pos, std::sync::Arc::clone(tile))));
    }
    output.try_prune_with(ctl).map_err(|e| failure(cmd, ctl, e))?;
    poll(ctl)?;
    Ok((output.tile_count() > 0).then_some(output))
}
pub(crate) fn run(session: &mut Session, p: &Value) -> Result<Value> {
    dispatch_preflight(session, p, false)?;
    let count = p.get("points").and_then(Value::as_array).map_or(0, Vec::len);
    let mut points = Vec::new();
    points.try_reserve_exact(count).map_err(|_| bad("select.lasso", "point allocation"))?;
    if points.capacity() > count.saturating_add(16) {
        return Err(bad("select.lasso", "point capacity"));
    }
    json_points("select.lasso", p, |point| {
        points.push(point);
        Ok(())
    })?;
    run_points(session, "select.lasso", "Lasso", &points, p)
}
pub(crate) fn run_points(session: &mut Session, cmd: &str, label: &str, points: &[(f32, f32)], p: &Value) -> Result<Value> {
    let o = options(session, cmd, p)?;
    if label.len() > 128 {
        return Err(bad(cmd, "bounded label required"));
    }
    polygon::requirements_with_interrupt(points, Rect::EMPTY, o.aa, &Interrupt::NONE).map_err(|e| failure(cmd, &Interrupt::NONE, e))?;
    let mut owned = Vec::new();
    owned.try_reserve_exact(points.len()).map_err(|_| bad(cmd, "point copy allocation"))?;
    if owned.capacity() > points.len().saturating_add(16) {
        return Err(bad(cmd, "point copy capacity"));
    }
    owned.extend_from_slice(points);
    let mut command = String::new();
    command.try_reserve_exact(cmd.len()).map_err(|_| bad(cmd, "command allocation"))?;
    if command.capacity() > cmd.len().saturating_add(16) {
        return Err(bad(cmd, "command capacity"));
    }
    command.push_str(cmd);
    let mut title = String::new();
    title.try_reserve_exact(label.len()).map_err(|_| bad(cmd, "label allocation"))?;
    if title.capacity() > label.len().saturating_add(16) {
        return Err(bad(cmd, "label capacity"));
    }
    title.push_str(label);
    let state = session.active().ok_or(EngineError::NoDocument)?;
    state.doc.validate_resident()?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let worker = baseline.clone();
    let apply_title = title.clone();
    let apply_command = command.clone();
    crate::jobs::run(
        session,
        label,
        true,
        move |ctx| ctx.stage(0., 0.95, &title, |ctl| Ok((prepare(&worker, &owned, o, &command, ctl)?, ctx.clone()))),
        move |session, (selection, ctx)| {
            let state = session.active().ok_or(EngineError::NoDocument)?;
            if state.doc.id != baseline.id
                || state.revision != revision
                || !baseline.downgrade().is_current(&state.doc)
                || state.floating.is_some()
                || state.coalesce.is_some()
                || session.command_has_job_on(state.doc.id)
                || session.command_has_pending_job()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
            {
                return Err(bad(&apply_command, "polygon target changed during preparation"));
            }
            ctx.stage(0.95, 1., &apply_title, |ctl| {
                poll(ctl)?;
                let selected = selection.is_some();
                let response = json!({"selected":selected});
                session.edit_with_interrupt(&apply_title, ctl, move |doc, _| {
                    crate::selection_inverse_controlled::selection_write_authority(doc, ctl)?;
                    doc.selection = selection;
                    crate::selection_inverse_controlled::selection_write_authority(doc, ctl)?;
                    poll(ctl)
                })?;
                Ok(response)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn oracle(doc: &Document, points: &[(f32, f32)], o: Options) -> Option<Surface> {
        let area = if o.feather > 0. { doc.bounds() } else { rows(points, doc.bounds(), &Interrupt::NONE).unwrap() };
        let mask = sel::polygon(points, area, o.aa);
        let mask = if o.feather > 0. { sel::feather(&mask, area.width() as usize, area.height() as usize, o.feather) } else { mask };
        let inside = sel::combine(doc.selection.as_ref(), &mask, area, o.mode);
        if o.feather > 0. || !matches!(o.mode, SelectionMode::Add | SelectionMode::Subtract) {
            return inside;
        }
        let Some(old) = &doc.selection else { return inside };
        if area.is_empty() {
            return Some(old.clone());
        }
        let values = inside.map_or_else(|| vec![0.; area.width() as usize * area.height() as usize], |s| s.read_region(area));
        let mut result = old.clone();
        result.write_region(area, &values);
        result.prune();
        (result.tile_count() > 0).then_some(result)
    }
    #[test]
    fn ordinary_polygon_and_feather_match_upstream_band_and_full_canvas_routes() {
        let points = [(-3.25, 4.5), (35.75, 6.25), (17.5, 134.25)];
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = Document::new("ordinary", photocraft_geom::Size::new(40, 140), ColorMode::Rgb, sample);
            let mut old = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, sample, false), &[0.2]);
            old.write_region(Rect::new(300, 160, 301, 161), &[0.75]);
            old.write_region(Rect::new(5, 5, 6, 6), &[0.5]);
            doc.selection = Some(old);
            let before = doc.clone();
            for mode in [SelectionMode::Replace, SelectionMode::Add, SelectionMode::Subtract, SelectionMode::Intersect] {
                for feather in [0., 1.25] {
                    let options = Options { mode, aa: true, feather };
                    let actual = prepare(&doc, &points, options, "select.lasso", &Interrupt::NONE).unwrap();
                    assert_eq!(actual, oracle(&doc, &points, options));
                }
            }
            assert_eq!(doc, before);
        }
    }
    #[test]
    fn native_band_preserves_default_offcanvas_lowbits_and_uses_saturating_subtraction() {
        let points = [(0., 2.), (2., 2.), (2., 3.), (0., 3.)];
        for sample in [SampleType::U32, SampleType::F64] {
            let mut doc = Document::new("wide", photocraft_geom::Size::new(4, 8), ColorMode::Rgb, sample);
            let value = if sample == SampleType::U32 { 0x80000001u32.to_ne_bytes().to_vec() } else { (0.25 + 2f64.powi(-45)).to_ne_bytes().to_vec() };
            let mut old = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, sample, false), &value).unwrap();
            old.try_write_interleaved(Rect::new(-2, -2, -1, -1), &value).unwrap();
            doc.selection = Some(old);
            let before = doc.clone();
            let output =
                prepare(&doc, &points, Options { mode: SelectionMode::Subtract, aa: false, feather: 0. }, "select.lasso", &Interrupt::NONE).unwrap().unwrap();
            assert_eq!(output.default_bytes(), value);
            let mut word = vec![0; sample.bytes()];
            output.try_read_pixel_bytes(3, 7, &mut word).unwrap();
            assert_eq!(word, value);
            output.try_read_pixel_bytes(0, 2, &mut word).unwrap();
            assert_eq!(word, vec![0; sample.bytes()]);
            output.try_read_pixel_bytes(-2, -2, &mut word).unwrap();
            assert_eq!(word, value);
            assert_eq!(doc, before);
        }
    }
    #[test]
    fn captured_stale_and_cancelled_polygon_leave_snapshot_and_history_intact() {
        let mut session = Session::new();
        session.add_document(Document::new("polygon", photocraft_geom::Size::new(20, 20), ColorMode::Rgb, SampleType::U32), None).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let p = json!({"points":[[0,0],[19,0],[0,19]],"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()});
        assert!(dispatch_preflight(&session, &p, false).is_err());
        let points = [(0., 0.), (19., 0.), (0., 19.)];
        assert!(matches!(
            prepare(&before, &points, Options { mode: SelectionMode::Replace, aa: true, feather: 0. }, "select.lasso", &Interrupt::cancel_only(&|| true)),
            Err(EngineError::Cancelled)
        ));
        assert!(before.shares_state(&session.active().unwrap().doc));
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(json_points("select.lasso", &json!({"points":[[0,0],[1,"bad"],[0,1]]}), |_| Ok(())).is_err());
    }
}
