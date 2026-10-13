//! Controlled canvas selection inversion. Narrow storage follows upstream f32
//! arithmetic and GRAY8 encoding; U32/F64 are explicit native unit coverage.
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_color::{ColorMode, PixelFormat, SampleType, read_sample, try_read_sample_f64, write_sample};
use photocraft_doc::{Document, item_identity::ItemRef};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{
    Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface,
    staged::{StagedSurfaceBuilder, StagingBudget},
};
use serde_json::Value;
const ID: &str = "select.inverse";
const MAX_WORK: u64 = 1 << 30;
fn bad(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("{ID}: {message}"))
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn error(ctl: &Interrupt<'_>, e: impl std::fmt::Display) -> EngineError {
    if ctl.cancelled() { EngineError::Cancelled } else { bad(e) }
}
fn decimal(value: &Value) -> Result<u64> {
    let text = value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad("captured IDs must be canonical decimal strings"))?;
    text.parse().map_err(|_| bad("identity overflow"))
}
fn parameters(session: &Session, p: &Value) -> Result<()> {
    if !p.is_null() {
        let object = p
            .as_object()
            .filter(|o| o.len() <= 2 && o.keys().all(|k| matches!(k.as_str(), "documentId" | "expectedRevision")))
            .ok_or_else(|| bad("expected captured pair or empty parameters"))?;
        let _ = object;
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if decimal(id)? == state.doc.id.0 && decimal(rev)? == state.revision => {}
        _ => return Err(bad("captured selection target changed")),
    }
    if state.revision == u64::MAX
        || state.floating.is_some()
        || state.coalesce.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.command_has_pending_job()
        || session.command_has_job_on(state.doc.id)
    {
        return Err(bad("selection target is not idle"));
    }
    if state.doc.selection.is_none() {
        return Err(bad("no selection"));
    }
    Ok(())
}
pub(crate) fn dispatch_preflight(session: &Session, p: &Value, _background: bool) -> Result<()> {
    parameters(session, p)
}
fn canvas(doc: &Document) -> Result<Rect> {
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 300000 || doc.size.height > 300000 {
        return Err(bad("invalid canvas dimensions"));
    }
    Ok(doc.bounds())
}
fn authority(doc: &Document, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    let Some(state) = &doc.xcf_state else {
        if doc.selection_channel_id.is_some() {
            return Err(bad("selection identity lacks current canonical metadata"));
        }
        return Ok(());
    };
    let count = state
        .items
        .len()
        .checked_add(state.source_items.len())
        .and_then(|n| n.checked_add(state.filter_stacks.len()))
        .and_then(|n| n.checked_add(state.opaque.len()))
        .filter(|n| *n <= 100000)
        .ok_or_else(|| bad("selection metadata count limit"))?;
    let _ = count;
    if let Some(id) = doc.selection_channel_id {
        let reference = ItemRef::Channel(id);
        let mut found = None;
        for item in &state.items {
            poll(ctl)?;
            if item.item == reference {
                if found.replace(item).is_some() {
                    return Err(bad("ambiguous selection owner"));
                }
            }
        }
        let item = found.ok_or_else(|| bad("missing selection owner"))?;
        if item.source_flags != 0 || item.mask.is_some() || item.composition.is_some() || item.parent.is_some() || item.raster_bounds != Some(canvas(doc)?) {
            return Err(bad("selection geometry/flags are not qualified"));
        }
        crate::xcf_item_locks::effective_item_locks(doc, reference)?.require(crate::xcf_item_locks::EditKind::Content)?;
        for source in &state.source_items {
            poll(ctl)?;
            if source.owner == reference {
                return Err(bad("immutable source selection cannot be rewritten"));
            }
        }
        for stack in &state.filter_stacks {
            poll(ctl)?;
            if stack.owner == reference {
                return Err(bad("filtered selection inversion is not qualified"));
            }
        }
        for opaque in &state.opaque {
            poll(ctl)?;
            if opaque.owner == Some(reference) && opaque.rendering_required {
                return Err(bad("opaque selection inversion is not qualified"));
            }
        }
        let source = doc.selection.as_ref().ok_or_else(|| bad("missing selection"))?;
        if matches!(source.format().sample, SampleType::U32 | SampleType::F64) && source.format().sample != doc.depth {
            return Err(bad("canonical selection depth mismatch"));
        }
    }
    doc.validate_xcf_state_with(ctl).map_err(|e| error(ctl, e))?;
    poll(ctl)
}
struct Plan {
    format: PixelFormat,
    tiles: u64,
    stage_bytes: u64,
    stage_work: u64,
}
fn admission(doc: &Document, ctl: &Interrupt<'_>) -> Result<Plan> {
    poll(ctl)?;
    let area = canvas(doc)?;
    let source = doc.selection.as_ref().ok_or_else(|| bad("no selection"))?;
    let input = source.format();
    if input.mode != ColorMode::Grayscale || input.alpha {
        return Err(bad("selection must be Gray without alpha"));
    }
    let format = if matches!(input.sample, SampleType::U32 | SampleType::F64) { input } else { PixelFormat::GRAY8 };
    let width = u64::from(doc.size.width);
    let height = u64::from(doc.size.height);
    let tiles = width.div_ceil(TILE_SIZE as u64).checked_mul(height.div_ceil(TILE_SIZE as u64)).ok_or_else(|| bad("tile count overflow"))?;
    let tile_bytes = (TILE_SIZE as u64) * (TILE_SIZE as u64) * format.bytes_per_pixel() as u64;
    let stage_bytes = tiles
        .checked_mul(tile_bytes.checked_mul(4).and_then(|n| n.checked_add(4096)).ok_or_else(|| bad("stage capacity"))?)
        .ok_or_else(|| bad("output capacity"))?;
    let stage_work = tiles
        .checked_mul(tile_bytes.checked_mul(8).and_then(|n| n.checked_add(8192)).ok_or_else(|| bad("staging work"))?)
        .ok_or_else(|| bad("staging work"))?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, ctl).map_err(|e| error(ctl, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, ctl).map_err(|e| error(ctl, e))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, ctl)?;
    let peak = raster
        .retained_bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|n| n.checked_add(model.checked_mul(5)?))
        .and_then(|n| n.checked_add(payload.checked_mul(2)?))
        .and_then(|n| n.checked_add(stage_bytes))
        .and_then(|n| n.checked_add(2 << 20))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad("aggregate resident limit"))?;
    let _ = peak;
    let work = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(16))
        .and_then(|n| n.checked_add(stage_work))
        .and_then(|n| n.checked_add(model.checked_add(payload)?.checked_mul(8)?))
        .filter(|n| *n <= MAX_WORK)
        .ok_or_else(|| bad("aggregate work limit"))?;
    let _ = work;
    let _ = area;
    source.try_validate_tile_origins_with(ctl).map_err(|e| error(ctl, e))?;
    authority(doc, ctl)?;
    poll(ctl)?;
    Ok(Plan { format, tiles, stage_bytes, stage_work })
}
struct Prepaid {
    bytes: u64,
    work: u64,
}
impl StagingBudget for Prepaid {
    type Lease = ();
    fn reserve(&mut self, bytes: u64) -> std::result::Result<(), RasterInputError> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn work(&mut self, units: u64) -> std::result::Result<(), RasterInputError> {
        self.work = self.work.checked_sub(units).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
fn invert_sample(source: &[u8], sample: SampleType, output: &mut [u8]) -> Result<()> {
    match sample {
        SampleType::U32 => {
            let code = u32::from_ne_bytes(source.try_into().map_err(|_| bad("U32 sample length"))?);
            if output.len() != 4 {
                return Err(bad("U32 output length"));
            }
            output.copy_from_slice(&(u32::MAX - code).to_ne_bytes());
        }
        SampleType::F64 => {
            let value = try_read_sample_f64(source, sample, 0).map_err(bad)?;
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(bad("native selection requires finite unit coverage"));
            }
            if output.len() != 8 {
                return Err(bad("F64 output length"));
            }
            output.copy_from_slice(&(1.0 - value).to_ne_bytes());
        }
        _ => {
            if output.len() != 1 {
                return Err(bad("legacy selection output length"));
            }
            let value = read_sample(source, sample, 0);
            write_sample(output, SampleType::U8, 0, 1.0 - value);
        }
    }
    Ok(())
}
fn prepare(doc: &Document, ctl: &Interrupt<'_>) -> Result<Surface> {
    let plan = admission(doc, ctl)?;
    let source = doc.selection.as_ref().ok_or_else(|| bad("no selection"))?;
    let bounds = canvas(doc)?;
    let bpp = plan.format.bytes_per_pixel();
    let input_width = source.format().bytes_per_pixel();
    let zero = [0u8; 8];
    let mut output = Surface::try_with_default_bytes(plan.format, zero.get(..bpp).ok_or_else(|| bad("default width"))?).map_err(|e| error(ctl, e))?;
    let mut budget = Prepaid { bytes: plan.stage_bytes, work: plan.stage_work };
    let mut visited = 0u64;
    let tile_len = (TILE_SIZE as usize) * (TILE_SIZE as usize) * bpp;
    let mut scratch = Vec::new();
    scratch.try_reserve_exact(tile_len).map_err(|_| bad("tile scratch allocation"))?;
    if scratch.capacity() > 2 << 20 {
        return Err(bad("scratch capacity limit"));
    }
    scratch.resize(tile_len, 0);
    let mut cell = Vec::new();
    cell.try_reserve_exact(64 * 64 * bpp).map_err(|_| bad("cell scratch allocation"))?;
    if cell.capacity() > 1 << 20 {
        return Err(bad("cell capacity limit"));
    }
    for coordinate in bounds.tiles() {
        poll(ctl)?;
        visited = visited.checked_add(1).filter(|n| *n <= plan.tiles).ok_or_else(|| bad("tile census changed"))?;
        scratch.fill(0);
        let tile = coordinate.rect();
        let part = tile.intersect(&bounds);
        let mut input = [0u8; 8];
        for y in part.y0..part.y1 {
            poll(ctl)?;
            for x in part.x0..part.x1 {
                poll(ctl)?;
                let input = input.get_mut(..input_width).ok_or_else(|| bad("input word width"))?;
                source.try_read_pixel_bytes(x, y, input).map_err(|e| error(ctl, e))?;
                let offset = usize::try_from(i64::from(y) - i64::from(tile.y0))
                    .ok()
                    .and_then(|row| row.checked_mul(TILE_SIZE as usize))
                    .and_then(|row| usize::try_from(i64::from(x) - i64::from(tile.x0)).ok().and_then(|col| row.checked_add(col)))
                    .and_then(|n| n.checked_mul(bpp))
                    .ok_or_else(|| bad("tile offset"))?;
                invert_sample(
                    input,
                    source.format().sample,
                    scratch.get_mut(offset..offset.checked_add(bpp).ok_or_else(|| bad("tile offset"))?).ok_or_else(|| bad("tile sample"))?,
                )?;
            }
        }
        let mut builder =
            StagedSurfaceBuilder::new(plan.format, zero.get(..bpp).ok_or_else(|| bad("default width"))?, tile, &mut budget, ctl).map_err(|e| error(ctl, e))?;
        for cy in (0..TILE_SIZE).step_by(64) {
            for cx in (0..TILE_SIZE).step_by(64) {
                poll(ctl)?;
                cell.clear();
                for row in cy..cy + 64 {
                    let offset = ((row as usize) * (TILE_SIZE as usize) + cx as usize) * bpp;
                    cell.extend_from_slice(scratch.get(offset..offset + 64 * bpp).ok_or_else(|| bad("cell row"))?);
                }
                builder
                    .push_region(Rect::new(tile.x0 + cx, tile.y0 + cy, tile.x0 + cx + 64, tile.y0 + cy + 64), &cell, &mut budget, ctl)
                    .map_err(|e| error(ctl, e))?;
            }
        }
        let (replacement, _lease) = builder.finalize(&mut budget, ctl).map_err(|e| error(ctl, e))?.into_parts();
        poll(ctl)?;
        output.put_tiles(replacement.tiles().map(|(position, tile)| (*position, std::sync::Arc::clone(tile))));
    }
    output.try_prune_with(ctl).map_err(|e| error(ctl, e))?;
    poll(ctl)?;
    Ok(output)
}
pub(crate) fn run(session: &mut Session, p: &Value) -> Result<Value> {
    parameters(session, p)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    state.doc.validate_resident()?;
    // These are shared snapshot handles, not deep Document/raster copies.
    // Worker admission is token-controlled and precedes every output allocation;
    // the same admission remains live through final history/model COW publication.
    let baseline = state.doc.clone();
    let revision = state.revision;
    let worker = baseline.clone();
    crate::jobs::run(
        session,
        "Inverse",
        true,
        move |ctx| ctx.stage(0.0, 0.95, "Invert Selection", |ctl| Ok((prepare(&worker, ctl)?, ctx.clone()))),
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
                return Err(bad("selection target changed during preparation"));
            }
            ctx.stage(0.95, 1.0, "Invert Selection", |ctl| {
                poll(ctl)?;
                session.edit_with_interrupt("Inverse", ctl, move |doc, _| {
                    authority(doc, ctl)?;
                    doc.selection = Some(selection);
                    authority(doc, ctl)?;
                    poll(ctl)
                })?;
                Ok(Value::Null)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_low_bits_and_f64_inverse_never_pass_through_f32() {
        let mut u32_output = [0; 4];
        invert_sample(&0x80000001u32.to_ne_bytes(), SampleType::U32, &mut u32_output).unwrap();
        assert_eq!(u32_output, 0x7ffffffeu32.to_ne_bytes());
        let value = 0.25 + 2f64.powi(-45);
        let mut output = [0; 8];
        invert_sample(&value.to_ne_bytes(), SampleType::F64, &mut output).unwrap();
        assert_eq!(output, (1.0 - value).to_ne_bytes());
        assert!(invert_sample(&f64::NAN.to_ne_bytes(), SampleType::F64, &mut output).is_err());
    }
    #[test]
    fn legacy_inverse_matches_upstream_f32_then_u8_encoding() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for value in [0., 0.25, 0.5, 1.] {
                let mut raw = [0; 4];
                let input = raw.get_mut(..sample.bytes()).unwrap();
                write_sample(input, sample, 0, value);
                let mut actual = [0];
                invert_sample(input, sample, &mut actual).unwrap();
                let mut expected = [0];
                write_sample(&mut expected, SampleType::U8, 0, 1.0 - read_sample(input, sample, 0));
                assert_eq!(actual, expected);
            }
        }
    }
    #[test]
    fn prepare_preserves_original_and_cancellation_publishes_nothing() {
        let mut doc = Document::new("Inverse draft", photocraft_geom::Size::new(2, 1), ColorMode::Rgb, SampleType::U32);
        let mut source = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false), &17u32.to_ne_bytes()).unwrap();
        source.try_write_interleaved(Rect::new(0, 0, 1, 1), &42u32.to_ne_bytes()).unwrap();
        doc.selection = Some(source);
        let before = doc.clone();
        assert!(matches!(prepare(&doc, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert_eq!(doc, before);
        let output = prepare(&doc, &Interrupt::NONE).unwrap();
        assert_eq!(output.default_bytes(), &[0; 4]);
        let mut word = [0; 4];
        output.try_read_pixel_bytes(0, 0, &mut word).unwrap();
        assert_eq!(word, (u32::MAX - 42).to_ne_bytes());
        assert_eq!(doc, before);
    }
}
