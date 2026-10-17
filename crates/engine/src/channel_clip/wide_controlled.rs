//! Closed unmanaged Gray clipboard storage. No luminosity, CMS, color
//! conversion or imported mask authority is inferred. Fractional Paste
//! uses only the explicitly defined finite-unit native arithmetic below.
use crate::channel_cmds::ChannelTarget;
use crate::edit_cmds::{Clip, ClipColorContext};
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::{Document, LayerId, WorkingProfileInterpretation, item_identity::ItemRef};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
use std::sync::Arc;

fn bad(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("native Gray clipboard: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn map(control: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(error) }
}
#[derive(Clone, Copy)]
enum Target {
    Channel(usize),
    Mask(LayerId),
}
fn decimal(value: &Value) -> Result<u64> {
    let s = value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad("invalid captured identity"))?;
    s.parse().map_err(|_| bad("identity overflow"))
}
fn context(session: &Session, params: &Value, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if params.as_object().is_some_and(|o| o.len() > 16 || o.keys().any(|k| k.len() > 128)) {
        return Err(bad("parameter count/key limit"));
    }
    if !params.is_null() && !params.is_object() {
        return Err(bad("expected parameters object"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if decimal(id)? == state.doc.id.0 && decimal(revision)? == state.revision => {}
        _ => return Err(bad("captured target changed")),
    }
    if state.revision == u64::MAX
        || state.floating.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || state.coalesce.is_some()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.command_has_job_on(state.doc.id)
        || session.command_has_pending_job()
    {
        return Err(bad("target is not idle"));
    }

    if state.doc.channels.len() > 10000 {
        return Err(bad("channel count limit"));
    }
    photocraft_doc::resident_structure::visit_layers_with(&state.doc, control, |_| Ok(true)).map_err(|e| map(control, e))?;
    poll(control)
}
fn target(session: &Session, params: &Value, control: &Interrupt<'_>) -> Result<Option<Target>> {
    let Some(state) = session.active() else { return Ok(None) };
    let selected = match params.get("target") {
        Some(Value::String(value)) if value == "mask" => Some(Target::Mask(state.active_layer.ok_or_else(|| bad("no active mask owner"))?)),
        Some(Value::Object(object)) if object.len() == 1 && object.contains_key("channel") => Some(Target::Channel(
            object.get("channel").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad("invalid channel target"))?,
        )),
        Some(_) => None,
        None => match state.channel_view.target {
            ChannelTarget::Alpha(index) => Some(Target::Channel(index)),
            ChannelTarget::LayerMask => Some(Target::Mask(state.active_layer.ok_or_else(|| bad("no active mask owner"))?)),
            _ => None,
        },
    };
    if matches!(selected, Some(Target::Mask(_))) {
        photocraft_doc::resident_structure::visit_layers_with(&state.doc, control, |_| Ok(true)).map_err(|e| map(control, e))?;
    }
    poll(control)?;
    Ok(selected)
}

fn surface(doc: &Document, target: Target) -> Result<&Surface> {
    match target {
        Target::Channel(i) => doc.channels.get(i).map(|c| &c.surface).ok_or_else(|| bad("missing saved channel")),
        Target::Mask(id) => doc.layer(id).and_then(|l| l.mask.as_ref()).map(|m| m.surface.as_ref()).ok_or_else(|| bad("missing layer mask")),
    }
}
fn locked(doc: &Document, target: Target) -> Result<()> {
    match target {
        Target::Channel(i) => crate::xcf_item_locks::require_channel_content(doc, doc.channels.get(i).ok_or_else(|| bad("missing channel"))?.id),
        Target::Mask(id) => crate::xcf_item_locks::require_layer_mask_content(doc, id),
    }
}
fn install(doc: &mut Document, target: Target, output: Surface) -> Result<()> {
    match target {
        Target::Channel(i) => doc.channels.get_mut(i).ok_or_else(|| bad("missing channel"))?.surface = output,
        Target::Mask(id) => doc.layer_mut(id).and_then(|l| l.mask.as_mut()).ok_or_else(|| bad("missing mask"))?.surface = Arc::new(output),
    }
    Ok(())
}
// Typed metadata establishes the current storage extent, not original rendered
// source equivalence. Reading retained raw storage never grants write authority.
fn declared_bounds(doc: &Document, target: Target, control: &Interrupt<'_>) -> Result<Rect> {
    poll(control)?;
    let Some(state) = &doc.xcf_state else { return Ok(doc.bounds()) };
    let count = state
        .items
        .len()
        .checked_add(state.source_items.len())
        .and_then(|n| n.checked_add(state.filter_stacks.len()))
        .and_then(|n| n.checked_add(state.filter_masks.len()))
        .and_then(|n| n.checked_add(state.opaque.len()))
        .filter(|n| *n <= 100000)
        .ok_or_else(|| bad("canonical owner count limit"))?;
    let _ = count;
    let (reference, owner) = match target {
        Target::Channel(index) => {
            let id = doc.channels.get(index).ok_or_else(|| bad("missing channel"))?.id;
            if id.0 == 0 || doc.channels.iter().filter(|channel| channel.id == id).count() != 1 {
                return Err(bad("ambiguous channel identity"));
            }
            (ItemRef::Channel(id), None)
        }
        Target::Mask(owner) => {
            let mut reference = None;
            for item in &state.items {
                poll(control)?;
                if item.mask.as_ref().is_some_and(|mask| mask.owner == ItemRef::Layer(owner))
                    && (!matches!(item.item, ItemRef::Mask(_)) || reference.replace(item.item).is_some())
                {
                    return Err(bad("ambiguous current mask owner"));
                }
            }
            (reference.ok_or_else(|| bad("missing current mask metadata"))?, Some(owner))
        }
    };
    let mut metadata = None;
    for item in &state.items {
        poll(control)?;
        if item.item == reference && metadata.replace(item).is_some() {
            return Err(bad("duplicate current item identity"));
        }
    }
    let item = metadata.ok_or_else(|| bad("missing canonical storage owner"))?;
    if item.source_flags != 0 || item.composition.is_some() {
        return Err(bad("unqualified current storage flags/composition"));
    }
    let bounds = item.raster_bounds.filter(|bounds| !bounds.is_empty()).ok_or_else(|| bad("missing current storage bounds"))?;
    let native = surface(doc, target)?;
    gray(native)?;
    if native.format().sample != doc.depth {
        return Err(bad("current native depth disagrees with canonical document"));
    }
    photocraft_raster::checked_region_byte_len(bounds, native.format()).map_err(|e| map(control, e))?;
    if let Some(owner) = owner {
        let ItemRef::Mask(id) = reference else { return Err(bad("mask identity")) };
        if id.0 == 0 || state.filter_masks.iter().any(|filter| filter.id == id) {
            return Err(bad("filter-owned mask is not a layer-mask clipboard target"));
        }
        let metadata = item.mask.as_ref().ok_or_else(|| bad("missing mask state"))?;
        let native_mask = doc.layer(owner).and_then(|layer| layer.mask.as_ref()).ok_or_else(|| bad("missing native mask"))?;
        let outside = try_read_sample_f64(native.default_bytes(), native.format().sample, 0).map_err(bad)?;
        if metadata.bounds != bounds
            || metadata.offset != [bounds.x0, bounds.y0]
            || metadata.enabled != native_mask.enabled
            || metadata.linked != native_mask.linked
            || metadata.outside_f64_bits != outside.to_bits()
        {
            return Err(bad("canonical/native mask metadata mismatch"));
        }
    } else if item.mask.is_some() || item.parent.is_some() {
        return Err(bad("unsupported saved-channel relationship"));
    }
    poll(control)?;
    Ok(bounds)
}
fn read_guard(doc: &Document, target: Target, control: &Interrupt<'_>) -> Result<Rect> {
    let bounds = declared_bounds(doc, target, control)?;
    if doc.xcf_state.is_some() {
        doc.validate_xcf_state_with(control).map_err(|e| map(control, e))?;
    }
    poll(control)?;
    Ok(bounds)
}
fn unit_storage(native: &Surface, control: &Interrupt<'_>) -> Result<()> {
    let default = try_read_sample_f64(native.default_bytes(), native.format().sample, 0).map_err(bad)?;
    if !default.is_finite() || !(0.0..=1.0).contains(&default) {
        return Err(bad("write requires finite unit native default"));
    }
    let work = (native.tile_count() as u64)
        .checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64) * 4)
        .filter(|n| *n <= 1 << 30)
        .ok_or_else(|| bad("native validation work limit"))?;
    let _ = work;
    let samples = (TILE_SIZE as usize) * (TILE_SIZE as usize);
    let expected = samples.checked_mul(native.format().bytes_per_pixel()).ok_or_else(|| bad("tile length overflow"))?;
    for (_, tile) in native.tiles() {
        poll(control)?;
        if tile.bytes().len() != expected {
            return Err(bad("invalid native tile length"));
        }
        for index in 0..samples {
            if index.is_multiple_of(1024) {
                poll(control)?;
            }
            let value = try_read_sample_f64(tile.bytes(), native.format().sample, index).map_err(bad)?;
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(bad("write requires finite unit native scalar storage"));
            }
        }
    }
    poll(control)
}
fn write_guard(doc: &Document, target: Target, control: &Interrupt<'_>) -> Result<Rect> {
    let bounds = read_guard(doc, target, control)?;
    locked(doc, target)?;
    let Some(state) = &doc.xcf_state else { return Ok(bounds) };
    let reference = match target {
        Target::Channel(index) => ItemRef::Channel(doc.channels.get(index).ok_or_else(|| bad("missing channel"))?.id),
        Target::Mask(owner) => state
            .items
            .iter()
            .find(|item| item.mask.as_ref().is_some_and(|mask| mask.owner == ItemRef::Layer(owner)))
            .map(|item| item.item)
            .ok_or_else(|| bad("missing mask owner"))?,
    };
    let affected = |item: ItemRef| item == reference || matches!(target,Target::Mask(owner) if item==ItemRef::Layer(owner));
    for item in &state.source_items {
        poll(control)?;
        if affected(item.owner) {
            return Err(bad("immutable source owner cannot be rewritten"));
        }
    }
    for stack in &state.filter_stacks {
        poll(control)?;
        if affected(stack.owner) {
            return Err(bad("filtered owner raw write is unqualified"));
        }
    }
    for record in &state.opaque {
        poll(control)?;
        if record.rendering_required && record.owner.is_some_and(affected) {
            return Err(bad("opaque rendering owner raw write is unqualified"));
        }
    }
    if let Target::Mask(owner) = target {
        let mask = doc.layer(owner).and_then(|layer| layer.mask.as_ref()).ok_or_else(|| bad("missing mask"))?;
        if mask.density != 1.0 || mask.feather != 0.0 {
            return Err(bad("mask effects are not a raw write context"));
        }
    }
    unit_storage(surface(doc, target)?, control)?;
    photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, control).map_err(|e| map(control, e))?;
    poll(control)?;
    Ok(bounds)
}

fn wide(surface: &Surface) -> bool {
    matches!(surface.format().sample, SampleType::U32 | SampleType::F64)
}
fn gray(surface: &Surface) -> Result<()> {
    if surface.format().mode != ColorMode::Grayscale || surface.format().alpha {
        return Err(bad("requires native Gray without alpha"));
    }
    Ok(())
}
fn boolean(surface: &Surface, x: i32, y: i32, control: &Interrupt<'_>) -> Result<bool> {
    gray(surface)?;
    let mut raw = [0u8; 8];
    let bytes = raw.get_mut(..surface.format().bytes_per_pixel()).ok_or_else(|| bad("coverage width"))?;
    surface.try_read_pixel_bytes(x, y, bytes).map_err(|e| map(control, e))?;
    bit(bytes, surface.format().sample, 0)
}
fn bit(raw: &[u8], sample: SampleType, index: usize) -> Result<bool> {
    let value = try_read_sample_f64(raw, sample, index).map_err(bad)?;
    if value == 0.0 {
        Ok(false)
    } else if value == 1.0 {
        Ok(true)
    } else {
        Err(bad("fractional/nonfinite coverage is unsupported"))
    }
}
fn unit_value(raw: &[u8], sample: SampleType, index: usize) -> Result<f64> {
    let value = try_read_sample_f64(raw, sample, index).map_err(bad)?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(bad("fractional composition requires finite unit samples"));
    }
    Ok(value)
}
fn u32_code(raw: &[u8]) -> Result<u32> {
    Ok(u32::from_ne_bytes(raw.try_into().map_err(|_| bad("U32 word length"))?))
}
/// Unfused dst*(1-alpha*limit)+gray*(alpha*limit). U32 uses an exact
/// rational numerator in u128, with one final nearest positive-half-up round;
/// there is no quantized alpha*limit intermediate and no f32 conversion.
/// Zero coverage preserves dst bits; full coverage preserves incoming bits,
/// including signed zero and NaN payloads, without interpreting gray values.
fn compose_native(destination: &mut [u8], incoming: &[u8], sample: SampleType, limit: Option<(&[u8], SampleType)>) -> Result<()> {
    let width = sample.bytes();
    if destination.len() != width || incoming.len() != width.checked_mul(2).ok_or_else(|| bad("composition width overflow"))? {
        return Err(bad("composition sample width"));
    }
    let gray = incoming.get(..width).ok_or_else(|| bad("incoming gray width"))?;
    match sample {
        SampleType::U32 => {
            let max = u128::from(u32::MAX);
            let alpha = u128::from(u32_code(incoming.get(width..width * 2).ok_or_else(|| bad("incoming alpha width"))?)?);
            let coverage = match limit {
                None => max,
                Some((bytes, SampleType::U32)) => u128::from(u32_code(bytes)?),
                Some((bytes, storage)) => {
                    let value = unit_value(bytes, storage, 0)?;
                    if value == 0.0 {
                        0
                    } else if value == 1.0 {
                        max
                    } else {
                        return Err(bad("fractional cross-depth limit is not qualified"));
                    }
                }
            };
            let weight = alpha.checked_mul(coverage).ok_or_else(|| bad("coverage overflow"))?;
            if weight == 0 {
                return Ok(());
            }
            let denominator = max.checked_mul(max).ok_or_else(|| bad("coverage denominator"))?;
            if weight == denominator {
                destination.copy_from_slice(gray);
                return Ok(());
            }
            let old = u128::from(u32_code(destination)?);
            let source = u128::from(u32_code(gray)?);
            let numerator = old
                .checked_mul(denominator.checked_sub(weight).ok_or_else(|| bad("coverage domain"))?)
                .and_then(|n| source.checked_mul(weight).and_then(|m| n.checked_add(m)))
                .ok_or_else(|| bad("composition numerator overflow"))?;
            let remainder = numerator % denominator;
            let result = (numerator / denominator)
                .checked_add(u128::from(remainder.checked_mul(2).ok_or_else(|| bad("rounding overflow"))? >= denominator))
                .ok_or_else(|| bad("rounding overflow"))?;
            destination.copy_from_slice(&u32::try_from(result).map_err(|_| bad("quantization overflow"))?.to_ne_bytes());
        }
        SampleType::F64 => {
            let alpha = unit_value(incoming, sample, 1)?;
            let limit = match limit {
                None => 1.0,
                Some((bytes, SampleType::F64)) => unit_value(bytes, SampleType::F64, 0)?,
                Some((bytes, storage)) => {
                    let value = unit_value(bytes, storage, 0)?;
                    if value != 0.0 && value != 1.0 {
                        return Err(bad("fractional cross-depth limit is not qualified"));
                    }
                    value
                }
            };
            let coverage = alpha * limit;
            if coverage == 0.0 {
                return Ok(());
            }
            if coverage == 1.0 {
                destination.copy_from_slice(gray);
                return Ok(());
            }
            let previous = unit_value(destination, sample, 0)?;
            let source = unit_value(gray, sample, 0)?;
            let background = previous * (1.0 - coverage);
            let foreground = source * coverage;
            let result = background + foreground;
            if !result.is_finite() || !(0.0..=1.0).contains(&result) {
                return Err(bad("composition result outside finite unit domain"));
            }
            destination.copy_from_slice(&result.to_ne_bytes());
        }
        _ => return Err(bad("composition requires U32/F64")),
    }
    Ok(())
}
fn inverse_unit(raw: &[u8], sample: SampleType, output: &mut [u8]) -> Result<()> {
    if raw.len() != sample.bytes() || output.len() != sample.bytes() {
        return Err(bad("inverse sample width"));
    }
    if sample == SampleType::U32 {
        output.copy_from_slice(&u32::MAX.checked_sub(u32_code(raw)?).ok_or_else(|| bad("inverse coverage"))?.to_ne_bytes());
    } else {
        let value = unit_value(raw, sample, 0)?;
        try_write_sample_f64(output, sample, 0, 1.0 - value).map_err(bad)?;
    }
    Ok(())
}

fn tile_count(area: Rect) -> Result<u64> {
    if area.is_empty() {
        return Ok(0);
    }
    let x = i64::from(area.x1 - 1).div_euclid(i64::from(TILE_SIZE)) - i64::from(area.x0).div_euclid(i64::from(TILE_SIZE)) + 1;
    let y = i64::from(area.y1 - 1).div_euclid(i64::from(TILE_SIZE)) - i64::from(area.y0).div_euclid(i64::from(TILE_SIZE)) + 1;
    u64::try_from(x).ok().and_then(|x| u64::try_from(y).ok().and_then(|y| x.checked_mul(y))).ok_or_else(|| bad("tile count"))
}
fn admission(doc: &Document, source: &Surface, area: Rect, extra: u64, control: &Interrupt<'_>) -> Result<()> {
    admission_repeated(doc, source, area, extra, 1, control)
}
fn admission_repeated(doc: &Document, source: &Surface, area: Rect, extra: u64, passes: u64, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    photocraft_raster::checked_region_byte_len(area, source.format()).map_err(|e| map(control, e))?;
    let count = tile_count(area)?;
    let tiles = source.tile_count() as u64;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, control).map_err(|e| map(control, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, control).map_err(|e| map(control, e))? as u64;
    let footprint = DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let output = count.checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64) * 16 + 1024).ok_or_else(|| bad("output limit"))?;
    let peak = footprint
        .retained_bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(footprint.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|n| n.checked_add(model.checked_mul(5)?))
        .and_then(|n| n.checked_add(payload.checked_mul(2)?))
        .and_then(|n| n.checked_add(output.checked_mul(2)?))
        .and_then(|n| n.checked_add(extra))
        .and_then(|n| n.checked_add(4 << 20))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad("aggregate resident limit"))?;
    let _ = peak;
    // The current checked writer stages its map for each tile; charge that actual repeated work.
    let work = count
        .checked_mul(tiles.checked_add(count).and_then(|n| n.checked_add((TILE_SIZE as u64) * (TILE_SIZE as u64) * 32)).ok_or_else(|| bad("work overflow"))?)
        .and_then(|n| n.checked_add(tiles.checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64) * 16)?))
        .and_then(|n| n.checked_add(footprint.retained_bytes.checked_mul(8)?))
        .and_then(|n| n.checked_add(model.checked_add(payload)?.checked_mul(8)?))
        .and_then(|n| n.checked_mul(passes))
        .filter(|n| *n <= 1 << 30)
        .ok_or_else(|| bad("work limit"))?;
    let _ = work;
    source.try_validate_tile_origins_with(control).map_err(|e| map(control, e))?;
    poll(control)
}
fn fence(session: &Session, snapshot: &DocumentSnapshot, revision: u64, target: Target, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let current = session.active().ok_or(EngineError::NoDocument)?;
    if matches!(target,Target::Mask(owner) if current.active_layer!=Some(owner)) {
        return Err(bad("mask owner target changed"));
    }
    if current.revision != revision
        || !snapshot.downgrade().is_current(&current.doc)
        || session.command_has_job_on(current.doc.id)
        || session.command_has_pending_job()
        || current.floating.is_some()
        || current.coalesce.is_some()
    {
        return Err(bad("clipboard target changed"));
    }
    poll(control)
}
fn selection_area(doc: &Document, control: &Interrupt<'_>) -> Result<Rect> {
    let area = match &doc.selection {
        Some(s) => {
            gray(s)?;
            if s.tile_count() as u64 > (1 << 30) / (TILE_SIZE as u64 * TILE_SIZE as u64) {
                return Err(bad("selection work limit"));
            }
            s.try_validate_tile_origins_with(control).map_err(|e| map(control, e))?;
            s.try_content_bounds_with_interrupt(control).map_err(|e| map(control, e))?.intersect(&doc.bounds())
        }
        None => doc.bounds(),
    };
    if area.is_empty() {
        return Err(bad("empty selection"));
    }
    Ok(area)
}
fn make_clip(doc: &Document, source: &Surface, area: Rect, control: &Interrupt<'_>) -> Result<Clip> {
    gray(source)?;
    let sample = source.format().sample;
    let format = PixelFormat::new(ColorMode::Grayscale, sample, true);
    let width = sample.bytes();
    let mut default = [0u8; 16];
    default.get_mut(..width).ok_or_else(|| bad("default width"))?.copy_from_slice(source.default_bytes());
    let mut output = Surface::try_with_default_bytes(format, default.get(..width * 2).ok_or_else(|| bad("default width"))?).map_err(|e| map(control, e))?;
    let mut any = false;
    for tc in area.tiles() {
        poll(control)?;
        let rect = tc.rect().intersect(&area);
        let len = photocraft_raster::checked_region_byte_len(rect, format).map_err(|e| map(control, e))?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(len).map_err(|_| bad("scratch allocation"))?;
        if raw.capacity() > 4 << 20 {
            return Err(bad("scratch capacity limit"));
        }
        raw.resize(len, 0);
        for (i, (x, y)) in (rect.y0..rect.y1).flat_map(|y| (rect.x0..rect.x1).map(move |x| (x, y))).enumerate() {
            poll(control)?;
            let pixel = raw.get_mut(i * width * 2..(i + 1) * width * 2).ok_or_else(|| bad("scratch offset"))?;
            source.try_read_pixel_bytes(x, y, pixel.get_mut(..width).ok_or_else(|| bad("sample width"))?).map_err(|e| map(control, e))?;
            let selected = match &doc.selection {
                Some(s) => boolean(s, x, y, control)?,
                None => true,
            };
            if selected {
                any = true;
                let alpha = pixel.get_mut(width..).ok_or_else(|| bad("alpha width"))?;
                match sample {
                    SampleType::U32 => alpha.copy_from_slice(&u32::MAX.to_ne_bytes()),
                    SampleType::F64 => alpha.copy_from_slice(&1f64.to_ne_bytes()),
                    _ => return Err(bad("not wide")),
                }
            }
        }
        output.try_write_interleaved_with(rect, &raw, control).map_err(|e| map(control, e))?;
    }
    if !any {
        return Err(bad("empty Boolean selection"));
    }
    poll(control)?;
    Ok(Clip {
        surface: output,
        bounds: area,
        color: ClipColorContext { mode: ColorMode::Grayscale, icc_profile: None, sample_encoding: None, interpretation: WorkingProfileInterpretation::Icc },
        composition: Vec::new(),
        layers: None,
        resolution: Some(doc.resolution_dpi),
    })
}
/// None is reserved for ordinary or other targets; wide unqualified contexts are errors.
pub(super) fn copy(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Option<Value>> {
    let Some(target) = target(session, params, control)? else { return Ok(None) };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if !wide(surface(&state.doc, target)?) {
        return Ok(None);
    }
    context(session, params, control)?;
    let snapshot = state.doc.clone();
    let revision = state.revision;
    admission(
        &snapshot,
        surface(&snapshot, target)?,
        snapshot.bounds(),
        session.clipboard.as_ref().map(|c| c.surface.resident_accounted_bytes()).transpose().map_err(|e| map(control, e))?.unwrap_or(0),
        control,
    )?;
    let area = selection_area(&snapshot, control)?.intersect(&read_guard(&snapshot, target, control)?);
    if area.is_empty() {
        return Err(bad("selection does not intersect current storage extent"));
    }
    let source = surface(&snapshot, target)?;
    let previous_clip = session.clipboard.as_ref().map(|c| c.surface.resident_accounted_bytes()).transpose().map_err(|e| map(control, e))?.unwrap_or(0);
    admission(&snapshot, source, area, previous_clip, control)?;
    let clip = make_clip(&snapshot, source, area, control)?;
    fence(session, &snapshot, revision, target, control)?;
    session.clipboard = Some(clip);
    Ok(Some(json!({"bounds":[area.x0,area.y0,area.width(),area.height()],"nativeGray":true})))
}
/// Zeroing is storage-only Boolean Cut, not a luminosity or coverage blend.
pub(super) fn cut(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Option<Value>> {
    let Some(target) = target(session, params, control)? else { return Ok(None) };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if !wide(surface(&state.doc, target)?) {
        return Ok(None);
    }
    context(session, params, control)?;
    let snapshot = state.doc.clone();
    let revision = state.revision;
    admission(
        &snapshot,
        surface(&snapshot, target)?,
        snapshot.bounds(),
        session.clipboard.as_ref().map(|c| c.surface.resident_accounted_bytes()).transpose().map_err(|e| map(control, e))?.unwrap_or(0),
        control,
    )?;
    let area = selection_area(&snapshot, control)?.intersect(&read_guard(&snapshot, target, control)?);
    if area.is_empty() {
        return Err(bad("selection does not intersect current storage extent"));
    }
    let source = surface(&snapshot, target)?;
    write_guard(&snapshot, target, control)?;
    let previous_clip = session.clipboard.as_ref().map(|c| c.surface.resident_accounted_bytes()).transpose().map_err(|e| map(control, e))?.unwrap_or(0);
    admission(&snapshot, source, area, previous_clip, control)?;
    let clip = make_clip(&snapshot, source, area, control)?;
    let mut output = source.clone();
    for tc in area.tiles() {
        poll(control)?;
        let rect = tc.rect().intersect(&area);
        let mut raw = output.try_read_interleaved_with(rect, control).map_err(|e| map(control, e))?;
        let bpp = source.format().bytes_per_pixel();
        for (i, (x, y)) in (rect.y0..rect.y1).flat_map(|y| (rect.x0..rect.x1).map(move |x| (x, y))).enumerate() {
            poll(control)?;
            if match &snapshot.selection {
                Some(s) => boolean(s, x, y, control)?,
                None => true,
            } {
                raw.get_mut(i * bpp..(i + 1) * bpp).ok_or_else(|| bad("cut offset"))?.fill(0)
            }
        }
        output.try_write_interleaved_with(rect, &raw, control).map_err(|e| map(control, e))?;
    }
    fence(session, &snapshot, revision, target, control)?;
    session.edit_with_interrupt("Cut", control, move |doc, _| {
        locked(doc, target)?;
        write_guard(doc, target, control)?;
        install(doc, target, output)?;
        write_guard(doc, target, control)?;
        Ok(())
    })?;
    session.clipboard = Some(clip);
    Ok(Some(json!({"nativeGray":true})))
}
/// Same-native replacement or declared finite-unit fractional composition.
pub(super) fn paste(
    session: &mut Session,
    params: &Value,
    in_place: bool,
    limit: Option<&Surface>,
    label: &str,
    control: &Interrupt<'_>,
) -> Result<Option<Value>> {
    let Some(target) = target(session, params, control)? else { return Ok(None) };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let destination = surface(&state.doc, target)?;
    if !wide(destination) {
        return Ok(None);
    }
    context(session, params, control)?;
    gray(destination)?;
    locked(&state.doc, target)?;
    let clip = session.clipboard.as_ref().ok_or_else(|| bad("empty clipboard"))?;
    let format = PixelFormat::new(ColorMode::Grayscale, destination.format().sample, true);
    if clip.surface.format() != format
        || clip.color.mode != ColorMode::Grayscale
        || clip.color.icc_profile.is_some()
        || clip.color.sample_encoding.is_some()
        || clip.color.interpretation != WorkingProfileInterpretation::Icc
        || !clip.composition.is_empty()
        || clip.layers.is_some()
    {
        return Err(bad("requires unmanaged same-native Gray clip"));
    }
    if let Some(center) = params.get("center")
        && !center.as_array().is_some_and(|a| a.len() == 2 && a.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)))
    {
        return Err(bad("invalid center"));
    }
    photocraft_raster::checked_region_byte_len(clip.bounds, format).map_err(|e| map(control, e))?;
    let (dx, dy) = crate::edit_cmds::paste_offset(clip, state.doc.bounds(), params, in_place)?;
    let shifted = Rect::new(
        clip.bounds.x0.checked_add(dx).ok_or_else(|| bad("offset overflow"))?,
        clip.bounds.y0.checked_add(dy).ok_or_else(|| bad("offset overflow"))?,
        clip.bounds.x1.checked_add(dx).ok_or_else(|| bad("offset overflow"))?,
        clip.bounds.y1.checked_add(dy).ok_or_else(|| bad("offset overflow"))?,
    );
    let declared = declared_bounds(&state.doc, target, control)?;
    let area = shifted.intersect(&state.doc.bounds()).intersect(&declared);
    let mut extra = clip.surface.resident_accounted_bytes().map_err(|e| map(control, e))?;
    if let Some(limit) = limit {
        gray(limit)?;
        extra = extra.checked_add(limit.resident_accounted_bytes().map_err(|e| map(control, e))?).ok_or_else(|| bad("limit bytes"))?;
        limit.try_validate_tile_origins_with(control).map_err(|e| map(control, e))?;
    }
    admission(&state.doc, destination, area, extra, control)?;
    write_guard(&state.doc, target, control)?;
    clip.surface.try_validate_tile_origins_with(control).map_err(|e| map(control, e))?;
    let snapshot = state.doc.clone();
    let revision = state.revision;
    let mut output = destination.clone();
    let width = destination.format().bytes_per_pixel();
    let mut incoming = [0u8; 16];
    for tc in area.tiles() {
        poll(control)?;
        let rect = tc.rect().intersect(&area);
        let mut raw = output.try_read_interleaved_with(rect, control).map_err(|e| map(control, e))?;
        for (i, (x, y)) in (rect.y0..rect.y1).flat_map(|y| (rect.x0..rect.x1).map(move |x| (x, y))).enumerate() {
            poll(control)?;
            let pixel = incoming.get_mut(..width * 2).ok_or_else(|| bad("clip width"))?;
            clip.surface
                .try_read_pixel_bytes(x.checked_sub(dx).ok_or_else(|| bad("source offset"))?, y.checked_sub(dy).ok_or_else(|| bad("source offset"))?, pixel)
                .map_err(|e| map(control, e))?;
            let mut limit_word = [0u8; 8];
            let limit_sample = if let Some(limit) = limit {
                let bytes = limit_word.get_mut(..limit.format().bytes_per_pixel()).ok_or_else(|| bad("limit word width"))?;
                limit.try_read_pixel_bytes(x, y, bytes).map_err(|e| map(control, e))?;
                Some((bytes as &[u8], limit.format().sample))
            } else {
                None
            };
            compose_native(raw.get_mut(i * width..(i + 1) * width).ok_or_else(|| bad("paste offset"))?, pixel, format.sample, limit_sample)?;
        }
        output.try_write_interleaved_with(rect, &raw, control).map_err(|e| map(control, e))?;
    }
    fence(session, &snapshot, revision, target, control)?;
    session.edit_with_interrupt(label, control, move |doc, _| {
        locked(doc, target)?;
        write_guard(doc, target, control)?;
        install(doc, target, output)?;
        write_guard(doc, target, control)?;
        doc.selection = None;
        Ok(())
    })?;
    Ok(Some(json!({"offset":[dx,dy],"nativeGray":true})))
}

/// Paste Special must enter here before the ordinary f32 selection inversion.
pub(super) fn paste_special(session: &mut Session, params: &Value, outside: bool, control: &Interrupt<'_>) -> Result<Option<Value>> {
    let Some(target) = target(session, params, control)? else { return Ok(None) };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let destination = surface(&state.doc, target)?;
    if !wide(destination) {
        return Ok(None);
    }
    context(session, params, control)?;
    gray(destination)?;
    locked(&state.doc, target)?;
    let snapshot = state.doc.clone();
    let revision = state.revision;
    let selection = snapshot.selection.as_ref().ok_or_else(|| bad("Paste Into/Outside requires a selection"))?;
    gray(selection)?;
    let clip_bytes = session.clipboard.as_ref().map(|c| c.surface.resident_accounted_bytes()).transpose().map_err(|e| map(control, e))?.unwrap_or(0);
    admission_repeated(&snapshot, destination, snapshot.bounds(), clip_bytes, if outside { 2 } else { 1 }, control)?;
    selection.try_validate_tile_origins_with(control).map_err(|e| map(control, e))?;
    let bounds = selection.try_content_bounds_with_interrupt(control).map_err(|e| map(control, e))?;
    photocraft_raster::checked_region_byte_len(bounds, selection.format()).map_err(|e| map(control, e))?;
    let mut forwarded = json!({"center":[(i64::from(bounds.x0)+i64::from(bounds.x1)) as f64/2.0,(i64::from(bounds.y0)+i64::from(bounds.y1)) as f64/2.0]});
    // Only these already-bounded literal fields are forwarded; no nested caller JSON clone.
    for key in ["target", "documentId", "expectedRevision"] {
        if let Some(value) = params.get(key) {
            forwarded[key] = value.clone();
        }
    }
    if let Some(center) = params.get("center") {
        if !center.as_array().is_some_and(|a| a.len() == 2 && a.iter().all(|v| v.as_f64().is_some_and(f64::is_finite))) {
            return Err(bad("invalid center"));
        }
        forwarded["center"] = center.clone();
    }
    let label = if outside { "Paste Outside" } else { "Paste Into" };
    if !outside {
        fence(session, &snapshot, revision, target, control)?;
        return paste(session, &forwarded, false, Some(selection), label, control);
    }
    let area = bounds.intersect(&snapshot.bounds());
    let format = selection.format();
    let width = format.bytes_per_pixel();
    let mut default = [0u8; 8];
    inverse_unit(selection.default_bytes(), format.sample, default.get_mut(..width).ok_or_else(|| bad("inverse default width"))?)?;
    let mut inverse =
        Surface::try_with_default_bytes(format, default.get(..width).ok_or_else(|| bad("inverse default width"))?).map_err(|e| map(control, e))?;
    for tile in area.tiles() {
        poll(control)?;
        let rect = tile.rect().intersect(&area);
        let len = photocraft_raster::checked_region_byte_len(rect, format).map_err(|e| map(control, e))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).map_err(|_| bad("limit scratch allocation"))?;
        if bytes.capacity() > 4 << 20 {
            return Err(bad("limit scratch capacity"));
        }
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                poll(control)?;
                let mut input = [0u8; 8];
                let mut output = [0u8; 8];
                let input = input.get_mut(..width).ok_or_else(|| bad("inverse sample width"))?;
                let output = output.get_mut(..width).ok_or_else(|| bad("inverse sample width"))?;
                selection.try_read_pixel_bytes(x, y, input).map_err(|e| map(control, e))?;
                inverse_unit(input, format.sample, output)?;
                bytes.extend_from_slice(output);
            }
        }
        inverse.try_write_interleaved_with(rect, &bytes, control).map_err(|e| map(control, e))?;
    }
    fence(session, &snapshot, revision, target, control)?;
    paste(session, &forwarded, false, Some(&inverse), label, control)
}

/// Metadata-only distinction for UI preparation. This is not lock, sample,
/// rendering, or source admission; dispatch rechecks the entire current owner.
pub(super) fn write_candidate(session: &Session) -> bool {
    if !target_candidate(session) {
        return false;
    }
    let Some(state) = session.active() else { return false };
    let target = match target(session, &Value::Null, &Interrupt::NONE) {
        Ok(Some(target)) => target,
        _ => return false,
    };
    let Some(metadata) = &state.doc.xcf_state else { return true };
    if declared_bounds(&state.doc, target, &Interrupt::NONE).is_err() {
        return false;
    }
    let reference = match target {
        Target::Channel(index) => match state.doc.channels.get(index) {
            Some(channel) => ItemRef::Channel(channel.id),
            None => return false,
        },
        Target::Mask(owner) => match metadata.items.iter().find(|item| item.mask.as_ref().is_some_and(|mask| mask.owner == ItemRef::Layer(owner))) {
            Some(item) => item.item,
            None => return false,
        },
    };
    let affected = |item: ItemRef| item == reference || matches!(target,Target::Mask(owner) if item==ItemRef::Layer(owner));
    !metadata.source_items.iter().any(|item| affected(item.owner))
        && !metadata.filter_stacks.iter().any(|stack| affected(stack.owner))
        && !metadata.opaque.iter().any(|record| record.rendering_required && record.owner.is_some_and(affected))
}

/// Metadata-only menu eligibility, not authorization or sample qualification.
/// Saved-channel Cut does not require an active pixel layer. Mask Copy/Cut may
/// also target the independent native mask of a non-raster layer.
pub(super) fn target_candidate(session: &Session) -> bool {
    let Some(state) = session.active() else { return false };
    if state.doc.channels.len() > 10000 {
        return false;
    }
    let eligible = |surface: &Surface| wide(surface) && surface.format().mode == ColorMode::Grayscale && !surface.format().alpha;
    match state.channel_view.target {
        ChannelTarget::Alpha(index) => state.doc.channels.get(index).is_some_and(|channel| eligible(&channel.surface)),
        ChannelTarget::LayerMask => {
            let Some(owner) = state.active_layer else { return false };
            let mut found = false;
            let result = photocraft_doc::resident_structure::visit_layers_with(&state.doc, &Interrupt::NONE, |layer| {
                if layer.id == owner {
                    found = layer.mask.as_ref().is_some_and(|mask| eligible(&mask.surface));
                    Ok(false)
                } else {
                    Ok(true)
                }
            });
            result.is_ok() && found
        }
        _ => false,
    }
}

/// Cheap borrowed preflight for shared dispatch, before generic floating normalization.
pub(super) fn dispatch_preflight(session: &Session, params: &Value, control: &Interrupt<'_>) -> Result<()> {
    let Some(target) = target(session, params, control)? else { return Ok(()) };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if wide(surface(&state.doc, target)?) {
        context(session, params, control)?;
        gray(surface(&state.doc, target)?)?;
        declared_bounds(&state.doc, target, control)?;
    }
    poll(control)
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_geom::Size;
    fn fixture(sample: SampleType, bytes: &[u8]) -> Session {
        let mut doc = Document::new("native channel", Size::new(2, 1), ColorMode::Rgb, sample);
        let mut surface = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, sample, false), bytes).unwrap();
        surface.try_write_interleaved(Rect::new(0, 0, 1, 1), bytes).unwrap();
        doc.channels.push(photocraft_doc::AlphaChannel::new("scalar", surface));
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
    }
    #[test]
    fn encoded_low_bits_and_nan_payload_copy_paste_without_color_profile() {
        for (sample, bytes) in [(SampleType::U32, 0x80000001u32.to_ne_bytes().to_vec()), (SampleType::F64, 0x7ff8000000000042u64.to_ne_bytes().to_vec())] {
            let mut session = fixture(sample, &bytes);
            let params = json!({"target":{"channel":0}});
            assert!(copy(&mut session, &params, &Interrupt::NONE).unwrap().is_some());
            let clip = session.clipboard.as_ref().unwrap();
            assert!(clip.color.icc_profile.is_none());
            assert!(clip.color.sample_encoding.is_none());
            assert_eq!(clip.surface.default_bytes().get(..sample.bytes()).unwrap(), bytes.as_slice());
            assert!(paste(&mut session, &params, true, None, "Paste", &Interrupt::NONE).unwrap().is_some());
            let mut raw = vec![0; sample.bytes()];
            session.active().unwrap().doc.channels[0].surface.try_read_pixel_bytes(0, 0, &mut raw).unwrap();
            assert_eq!(raw, bytes);
        }
    }
    #[test]
    fn fractional_selection_refuses_without_clipboard_or_document_change() {
        let mut session = fixture(SampleType::F64, &0.31250000000000006f64.to_ne_bytes());
        session
            .edit("coverage", |doc, _| {
                let mut coverage = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
                coverage.try_write_interleaved(Rect::new(0, 0, 1, 1), &0.5f64.to_ne_bytes()).unwrap();
                doc.selection = Some(coverage);
                Ok(())
            })
            .unwrap();
        let before = session.active().unwrap().doc.clone();
        let rev = session.active().unwrap().revision;
        assert!(copy(&mut session, &json!({"target":{"channel":0}}), &Interrupt::NONE).is_err());
        assert!(session.clipboard.is_none());
        assert!(before.downgrade().is_current(&session.active().unwrap().doc));
        assert_eq!(rev, session.active().unwrap().revision);
    }
    #[test]
    fn paste_outside_uses_native_boolean_default_not_f32_inversion() {
        let word = 0x80000001u32.to_ne_bytes();
        let mut session = fixture(SampleType::U32, &word);
        let params = json!({"target":{"channel":0}});
        copy(&mut session, &params, &Interrupt::NONE).unwrap();
        session
            .edit("destination and outside coverage", |doc, _| {
                doc.channels[0].surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false));
                let mut selection =
                    Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &1f64.to_ne_bytes()).unwrap();
                selection.try_write_interleaved(Rect::new(0, 0, 1, 1), &(-0f64).to_ne_bytes()).unwrap();
                doc.selection = Some(selection);
                Ok(())
            })
            .unwrap();
        let old = session.active().unwrap().doc.clone();
        assert!(paste_special(&mut session, &params, true, &Interrupt::NONE).unwrap().is_some());
        let surface = &session.active().unwrap().doc.channels[0].surface;
        let mut raw = [0; 4];
        surface.try_read_pixel_bytes(0, 0, &mut raw).unwrap();
        assert_eq!(raw, word);
        surface.try_read_pixel_bytes(1, 0, &mut raw).unwrap();
        assert_eq!(raw, [0; 4]);
        assert_eq!(old.channels[0].surface.default_bytes(), &[0; 4]);
        assert!(session.active().unwrap().doc.selection.is_none());
    }
    #[test]
    fn canonical_channel_copy_keeps_declared_bounds_and_source_state() {
        let word = 0x80000001u32.to_ne_bytes();
        let mut session = fixture(SampleType::U32, &word);
        session
            .edit("canonical channel metadata", |doc, _| {
                let mut state = photocraft_doc::XcfState::default();
                state.reconcile_layers(doc).unwrap();
                let id = doc.channels[0].id;
                let item = state.items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap();
                item.raster_bounds = Some(Rect::new(1, 0, 2, 1));
                doc.xcf_state = Some(state);
                Ok(())
            })
            .unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        copy(&mut session, &json!({"target":{"channel":0}}), &Interrupt::NONE).unwrap();
        assert_eq!(session.clipboard.as_ref().unwrap().bounds, Rect::new(1, 0, 2, 1));
        assert!(before.downgrade().is_current(&session.active().unwrap().doc));
        assert_eq!(revision, session.active().unwrap().revision);
        assert_eq!(session.active().unwrap().doc.xcf_state, before.xcf_state);
    }
    #[test]
    fn canonical_write_refuses_nonunit_without_publishing_clipboard_or_document() {
        let mut session = fixture(SampleType::F64, &2f64.to_ne_bytes());
        session
            .edit("canonical channel metadata", |doc, _| {
                let mut state = photocraft_doc::XcfState::default();
                state.reconcile_layers(doc).unwrap();
                let id = doc.channels[0].id;
                let bounds = doc.bounds();
                state.items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap().raster_bounds = Some(bounds);
                doc.xcf_state = Some(state);
                Ok(())
            })
            .unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let params = json!({"target":{"channel":0}});
        assert!(copy(&mut session, &params, &Interrupt::NONE).is_ok());
        let old_clip = session.clipboard.as_ref().unwrap().surface.default_bytes().to_vec();
        assert!(cut(&mut session, &params, &Interrupt::NONE).is_err());
        assert_eq!(session.clipboard.as_ref().unwrap().surface.default_bytes(), old_clip);
        assert!(before.downgrade().is_current(&session.active().unwrap().doc));
        assert_eq!(revision, session.active().unwrap().revision);
    }
    #[test]
    fn fractional_u32_composition_keeps_low_codes_without_alpha_quantization() {
        let mut destination = 2u32.to_ne_bytes();
        let mut incoming = [0u8; 8];
        incoming[..4].copy_from_slice(&3u32.to_ne_bytes());
        incoming[4..].copy_from_slice(&0x80000000u32.to_ne_bytes());
        let limit = 0x80000000u32.to_ne_bytes();
        compose_native(&mut destination, &incoming, SampleType::U32, Some((&limit, SampleType::U32))).unwrap();
        // Effective coverage is just over 1/4. The once-rounded result is 2,
        // not the result of intermediate f32 sample or integer-alpha rounding.
        assert_eq!(u32::from_ne_bytes(destination), 2);
        let mut destination = 0x80000001u32.to_ne_bytes();
        incoming[..4].copy_from_slice(&0x80000005u32.to_ne_bytes());
        incoming[4..].copy_from_slice(&u32::MAX.to_ne_bytes());
        compose_native(&mut destination, &incoming, SampleType::U32, Some((&limit, SampleType::U32))).unwrap();
        assert_eq!(u32::from_ne_bytes(destination), 0x80000003);
    }
    #[test]
    fn fractional_f64_composition_uses_declared_unfused_equation() {
        let previous = 0.25 + 2f64.powi(-50);
        let source = 0.75 - 2f64.powi(-51);
        let alpha = 0.375f64;
        let limit = 0.625f64;
        let coverage = alpha * limit;
        let expected = previous * (1.0 - coverage) + source * coverage;
        let mut destination = previous.to_ne_bytes();
        let mut incoming = [0u8; 16];
        incoming[..8].copy_from_slice(&source.to_ne_bytes());
        incoming[8..].copy_from_slice(&alpha.to_ne_bytes());
        compose_native(&mut destination, &incoming, SampleType::F64, Some((&limit.to_ne_bytes(), SampleType::F64))).unwrap();
        assert_eq!(destination, expected.to_ne_bytes());
        incoming[..8].copy_from_slice(&0x7ff8000000000042u64.to_ne_bytes());
        incoming[8..].copy_from_slice(&1f64.to_ne_bytes());
        compose_native(&mut destination, &incoming, SampleType::F64, None).unwrap();
        assert_eq!(destination, 0x7ff8000000000042u64.to_ne_bytes());
        incoming[8..].copy_from_slice(&0f64.to_ne_bytes());
        compose_native(&mut destination, &incoming, SampleType::F64, None).unwrap();
        assert_eq!(destination, 0x7ff8000000000042u64.to_ne_bytes());
    }
    #[test]
    fn fractional_invalid_samples_and_cross_depth_limits_leave_destination_unchanged() {
        let initial = 0.5f64.to_ne_bytes();
        let mut destination = initial;
        let mut incoming = [0u8; 16];
        incoming[..8].copy_from_slice(&f64::NAN.to_ne_bytes());
        incoming[8..].copy_from_slice(&0.5f64.to_ne_bytes());
        assert!(compose_native(&mut destination, &incoming, SampleType::F64, None).is_err());
        assert_eq!(destination, initial);
        incoming[..8].copy_from_slice(&0.25f64.to_ne_bytes());
        assert!(compose_native(&mut destination, &incoming, SampleType::F64, Some((&[128], SampleType::U8))).is_err());
        assert_eq!(destination, initial);
        assert!(compose_native(&mut [0; 1], &incoming, SampleType::F64, None).is_err());
    }
    #[test]
    fn cancellation_never_publishes_a_clip() {
        let mut session = fixture(SampleType::U32, &42u32.to_ne_bytes());
        let cancelled = || true;
        let progress = |_: f32| {};
        let control = Interrupt::new(&cancelled, &progress);
        assert!(matches!(copy(&mut session, &json!({"target":{"channel":0}}), &control), Err(EngineError::Cancelled)));
        assert!(session.clipboard.is_none());
    }
}
