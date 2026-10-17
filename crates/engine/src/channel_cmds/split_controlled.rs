//! Closed source-free native Split Channels; no compositor or source-format equivalence is inferred.
//! One neutral opaque RGB/Gray raster and plain compatible saved channels are decomposed
//! losslessly in the canvas. Legacy output cropping/zero defaults and Gray profile policy remain.
use crate::{DocState, DocumentSnapshot, EngineError, Result, Session};
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::Value;
const CMD: &str = "channel.split";
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: message.into() }
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn decimal(value: &Value) -> Result<u64> {
    let s = value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad("captured IDs must be canonical decimals"))?;
    s.parse().map_err(|_| bad("captured identity range"))
}
fn source_surface(doc: &Document) -> Result<&Surface> {
    if doc.xcf_state.is_some()
        || doc.source_color_context.is_some()
        || doc.color_table.is_some()
        || !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || doc.layers.len() != 1
    {
        return Err(bad("native Split requires one source-free RGB/Gray raster"));
    }
    let layer = doc.layers.first().ok_or_else(|| bad("missing raster"))?;
    if !layer.visible
        || layer.opacity != 1.0
        || layer.fill_opacity != 1.0
        || layer.blend != BlendMode::Normal
        || layer.clipped
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || layer.indexed_source.is_some()
        || layer.import_compositing.is_some()
        || layer.video.is_some()
        || layer.fill_cache.is_some()
        || !layer.psd_blocks.is_empty()
        || layer.effects != (photocraft_doc::Effects { enabled: layer.effects.enabled, ..Default::default() })
        || layer.blend_if != Default::default()
        || layer.advanced != Default::default()
        || layer.excluded_channels != 0
    {
        return Err(bad("native Split raster is not neutral"));
    }
    let LayerContent::Raster(surface) = &layer.content else { return Err(bad("native Split source must be resident raster")) };
    if surface.format() != doc.pixel_format() {
        return Err(bad("native Split raster format mismatch"));
    }
    Ok(surface)
}
fn request(session: &Session, params: &Value) -> Result<Option<bool>> {
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if !matches!(state.doc.depth, SampleType::U32 | SampleType::F64) {
        return Ok(None);
    }
    if !params.is_null()
        && !params.as_object().is_some_and(|o| o.len() <= 3 && o.keys().all(|k| matches!(k.as_str(), "closeOriginal" | "documentId" | "expectedRevision")))
    {
        return Err(bad("bounded Split parameters required"));
    }
    let close = match params.get("closeOriginal") {
        None => true,
        Some(v) => v.as_bool().ok_or_else(|| bad("closeOriginal must be boolean"))?,
    };
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if decimal(id)? == state.doc.id.0 && decimal(rev)? == state.revision => {}
        _ => return Err(bad("captured split source changed")),
    }
    if state.floating.is_some()
        || state.coalesce.is_some()
        || state.doc.accounted().is_some()
        || session.accounted_slots.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
    {
        return Err(bad("finish transient/accounted work before native Split"));
    }
    if session.smart_links.iter().any(|link| link.parent == state.doc.id || link.child == state.doc.id) {
        return Err(bad("smart contents Split is unsupported"));
    }
    source_surface(&state.doc)?;
    let count =
        state.doc.mode.color_channels().checked_add(state.doc.channels.len()).filter(|n| *n >= 2 && *n <= 4096).ok_or_else(|| bad("split output count"))?;
    session
        .documents()
        .len()
        .checked_sub(usize::from(close))
        .and_then(|n| n.checked_add(count))
        .filter(|n| *n <= 4096)
        .ok_or_else(|| bad("split tab count limit"))?;
    Ok(Some(close))
}
pub(super) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<bool> {
    request(session, params).map(|r| r.is_some())
}
fn unit(word: &[u8], sample: SampleType) -> Result<()> {
    if sample == SampleType::F64 {
        let raw: [u8; 8] = word.try_into().map_err(|_| bad("F64 word width"))?;
        let value = f64::from_ne_bytes(raw);
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(bad("native Split supports finite unit source values only"));
        }
    }
    Ok(())
}
fn io_error(error: photocraft_io::IoError) -> EngineError {
    match error {
        photocraft_io::IoError::Cancelled => EngineError::Cancelled,
        error => EngineError::Other(error.to_string()),
    }
}
fn admission(doc: &Document, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    source_surface(doc)?;
    super::source_dimensions(doc, CMD)?;
    if doc.name.len() > 4096 || doc.name.contains('\0') {
        return Err(bad("bounded source name required"));
    }
    let mut names = doc.name.len().checked_mul(doc.mode.color_channels()).ok_or_else(|| bad("name budget"))?;
    let gray = PixelFormat::new(ColorMode::Grayscale, doc.depth, false);
    for channel in &doc.channels {
        poll(ctl)?;
        if channel.spot.is_some() || channel.surface.format() != gray || channel.name.len() > 4096 || channel.name.contains('\0') {
            return Err(bad("saved channels must be plain same-depth Gray"));
        }
        names = names
            .checked_add(doc.name.len())
            .and_then(|n| n.checked_add(channel.name.len()))
            .filter(|n| *n <= 4 << 20)
            .ok_or_else(|| bad("aggregate output names"))?;
        channel.surface.try_validate_tile_origins_with(ctl)?;
    }
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, ctl).map_err(io_error)? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, ctl).map_err(io_error)? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, ctl)?;
    let outputs = (doc.mode.color_channels() + doc.channels.len()) as u64;
    let tiles = u64::from(doc.size.width)
        .div_ceil(TILE_SIZE as u64)
        .checked_mul(u64::from(doc.size.height).div_ceil(TILE_SIZE as u64))
        .ok_or_else(|| bad("tile count"))?;
    let tile_bytes = (TILE_SIZE as u64) * (TILE_SIZE as u64) * (doc.depth.bytes() as u64) * 2;
    let per_tile = tile_bytes + 1024;
    let axis_bytes = if let Some(photocraft_doc::PhysicalResolution { unit: photocraft_doc::PhysicalUnit::Custom(unit), .. }) = &doc.physical_resolution {
        unit.name
            .capacity()
            .checked_add(unit.symbol.capacity())
            .and_then(|n| n.checked_add(unit.abbreviation.capacity()))
            .ok_or_else(|| bad("axis metadata size"))? as u64
    } else {
        0
    };
    raster
        .retained_bytes
        .checked_add(raster.clone_metadata_bytes.checked_mul(4).ok_or_else(|| bad("source maps"))?)
        .and_then(|n| n.checked_add(model.checked_mul(4)?))
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(outputs.checked_mul(tiles)?.checked_mul(per_tile)?))
        .and_then(|n| n.checked_add(outputs.checked_mul(axis_bytes.checked_add(65536)?)?))
        .and_then(|n| n.checked_add(tile_bytes.checked_mul(3)?))
        .and_then(|n| n.checked_add(4 << 20))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad("aggregate split resident/output limit"))?;
    doc.bounds()
        .size()
        .area()
        .checked_mul(outputs.checked_mul(64).ok_or_else(|| bad("output work"))?)
        .and_then(|n| n.checked_add(model.checked_add(payload)?))
        .filter(|n| *n <= 1 << 32)
        .ok_or_else(|| bad("split work limit"))?;
    source_surface(doc)?.try_validate_tile_origins_with(ctl)?;
    // Axis validation occurs before output constructors/names, without creating a color-space alias.
    if let Some(resolution) = &doc.physical_resolution {
        resolution.validate().map_err(|e| EngineError::Other(e.to_string()))?;
    } else {
        photocraft_doc::PhysicalResolution::square(f64::from(doc.resolution_dpi)).validate().map_err(|e| EngineError::Other(e.to_string()))?;
    }
    poll(ctl)
}
fn validate_canvas(doc: &Document, ctl: &Interrupt<'_>) -> Result<()> {
    let surface = source_surface(doc)?;
    let sample = doc.depth.bytes();
    let channels = surface.channels();
    let mut raw = [0u8; 32];
    for y in 0..doc.size.height {
        poll(ctl)?;
        for x in 0..doc.size.width {
            if x.is_multiple_of(1024) {
                poll(ctl)?;
            }
            let bytes = raw.get_mut(..surface.format().bytes_per_pixel()).ok_or_else(|| bad("source pixel width"))?;
            surface.try_read_pixel_bytes(x as i32, y as i32, bytes)?;
            for word in bytes.chunks_exact(sample) {
                unit(word, doc.depth)?;
            }
            let alpha = bytes.get((channels - 1) * sample..channels * sample).ok_or_else(|| bad("source alpha width"))?;
            let opaque = if doc.depth == SampleType::U32 { alpha == u32::MAX.to_ne_bytes() } else { alpha == 1f64.to_ne_bytes() };
            if !opaque {
                return Err(bad("native Split requires opaque canvas pixels"));
            }
        }
    }
    for channel in &doc.channels {
        for y in 0..doc.size.height {
            poll(ctl)?;
            for x in 0..doc.size.width {
                if x.is_multiple_of(1024) {
                    poll(ctl)?;
                }
                let bytes = raw.get_mut(..sample).ok_or_else(|| bad("saved word width"))?;
                channel.surface.try_read_pixel_bytes(x as i32, y as i32, bytes)?;
                unit(bytes, doc.depth)?;
            }
        }
    }
    poll(ctl)
}
fn output_surface(source: &Surface, component: usize, bounds: Rect, sample: SampleType, ctl: &Interrupt<'_>) -> Result<Surface> {
    let format = PixelFormat::new(ColorMode::Grayscale, sample, true);
    let width = sample.bytes();
    let zero = [0u8; 16];
    let mut output = Surface::try_with_default_bytes(format, zero.get(..width * 2).ok_or_else(|| bad("output default"))?)?;
    let mut one = [0u8; 8];
    match sample {
        SampleType::U32 => one.get_mut(..4).ok_or_else(|| bad("unit width"))?.copy_from_slice(&u32::MAX.to_ne_bytes()),
        SampleType::F64 => one.copy_from_slice(&1f64.to_ne_bytes()),
        _ => return Err(bad("native Split sample type")),
    }
    let one = one.get(..width).ok_or_else(|| bad("unit width"))?;
    let capacity = (TILE_SIZE as usize) * (TILE_SIZE as usize) * width * 2;
    let mut raw = Vec::new();
    raw.try_reserve_exact(capacity).map_err(|_| bad("tile output allocation"))?;
    if raw.capacity() > capacity.saturating_add(64) {
        return Err(bad("tile output capacity"));
    }
    let mut pixel = [0u8; 32];
    for tile in bounds.tiles() {
        poll(ctl)?;
        let area = tile.rect().intersect(&bounds);
        raw.clear();
        for y in area.y0..area.y1 {
            poll(ctl)?;
            for x in area.x0..area.x1 {
                poll(ctl)?;
                let bytes = pixel.get_mut(..source.format().bytes_per_pixel()).ok_or_else(|| bad("source pixel width"))?;
                source.try_read_pixel_bytes(x, y, bytes)?;
                let begin = component.checked_mul(width).ok_or_else(|| bad("component offset"))?;
                let word = bytes.get(begin..begin + width).ok_or_else(|| bad("component word"))?;
                raw.extend_from_slice(word);
                raw.extend_from_slice(one);
            }
        }
        // Write into a private one-tile map, not a clone of the growing output map.
        let mut replacement = Surface::try_with_default_bytes(format, zero.get(..width * 2).ok_or_else(|| bad("output default"))?)?;
        replacement.try_write_interleaved_with(area, &raw, ctl)?;
        poll(ctl)?;
        output.put_tiles(replacement.tiles().map(|(position, tile)| (*position, std::sync::Arc::clone(tile))));
    }
    output.try_prune_with(ctl)?;
    poll(ctl)?;
    Ok(output)
}
fn output_name(prefix: &str, suffix: &str) -> Result<String> {
    let length = prefix.len().checked_add(suffix.len()).and_then(|n| n.checked_add(1)).ok_or_else(|| bad("output name length"))?;
    let mut name = String::new();
    name.try_reserve_exact(length).map_err(|_| bad("output name allocation"))?;
    name.push_str(prefix);
    name.push('_');
    name.push_str(suffix);
    Ok(name)
}
fn make_state(source: &Document, name: String, surface: Surface, ctl: &Interrupt<'_>) -> Result<DocState> {
    poll(ctl)?;
    let mut doc = Document::new(name, source.size, ColorMode::Grayscale, source.depth);
    doc.set_physical_resolution(source.effective_resolution()).map_err(|e| EngineError::Other(e.to_string()))?;
    let mut layer = Layer::raster("Background", doc.pixel_format());
    layer.locks.transparency = true;
    layer.locks.position = true;
    layer.content = LayerContent::Raster(surface);
    doc.layers.try_reserve_exact(1).map_err(|_| bad("output layer allocation"))?;
    doc.layers.push(layer);
    poll(ctl)?;
    DocState::new_with_interrupt(doc, None, ctl)
}
pub(super) fn try_run(session: &mut Session, params: &Value) -> Result<Option<Value>> {
    let Some(close_original) = request(session, params)? else { return Ok(None) };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let source = state.doc.clone();
    let revision = state.revision;
    let worker = source.clone();
    let count = worker.mode.color_channels() + worker.channels.len();
    let result = crate::jobs::run(
        session,
        "Split Channels",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Split Channels", |ctl| {
                admission(&worker, ctl)?;
                validate_canvas(&worker, ctl)?;
                let mut outputs = Vec::new();
                outputs.try_reserve_exact(count).map_err(|_| bad("output state allocation"))?;
                if outputs.capacity() > count.saturating_add(64) {
                    return Err(bad("output state capacity"));
                }
                for (component, label) in super::color_names(worker.mode).iter().enumerate() {
                    poll(ctl)?;
                    let mut suffix_bytes = [0u8; 4];
                    let first = label.chars().next().ok_or_else(|| bad("color component name"))?.to_ascii_uppercase();
                    let suffix = first.encode_utf8(&mut suffix_bytes);
                    let surface = output_surface(source_surface(&worker)?, component, worker.bounds(), worker.depth, ctl)?;
                    outputs.push(make_state(&worker, output_name(&worker.name, suffix)?, surface, ctl)?);
                }
                for channel in &worker.channels {
                    poll(ctl)?;
                    let surface = output_surface(&channel.surface, 0, worker.bounds(), worker.depth, ctl)?;
                    outputs.push(make_state(&worker, output_name(&worker.name, &channel.name)?, surface, ctl)?);
                }
                poll(ctl)?;
                Ok((outputs, ctx.clone()))
            })
        },
        move |session, (outputs, ctx)| {
            ctx.stage(0.95, 1., "Split Channels", |ctl| session.publish_split_batch_with_interrupt(&source, revision, outputs, close_original, ctl))
        },
    )?;
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_component_words_and_canvas_default_are_exact() {
        for sample in [SampleType::U32, SampleType::F64] {
            let format = PixelFormat::new(ColorMode::Rgb, sample, true);
            let words: Vec<Vec<u8>> = if sample == SampleType::U32 {
                [0x80000001u32, 7, u32::MAX - 1, u32::MAX].into_iter().map(|v| v.to_ne_bytes().to_vec()).collect()
            } else {
                [0.25 + 2f64.powi(-45), -0.0, 0.75, 1.0].into_iter().map(|v| v.to_ne_bytes().to_vec()).collect()
            };
            let default: Vec<u8> = words.iter().flatten().copied().collect();
            let source = Surface::try_with_default_bytes(format, &default).unwrap();
            let before = source.clone();
            let output = output_surface(&source, 0, Rect::new(0, 0, 257, 1), sample, &Interrupt::NONE).unwrap();
            let mut actual = vec![0; sample.bytes() * 2];
            output.try_read_pixel_bytes(256, 0, &mut actual).unwrap();
            assert_eq!(&actual[..sample.bytes()], &words[0]);
            assert_eq!(&actual[sample.bytes()..], &words[3]);
            output.try_read_pixel_bytes(257, 0, &mut actual).unwrap();
            assert_eq!(actual, vec![0; sample.bytes() * 2]);
            assert_eq!(output.default_bytes(), vec![0; sample.bytes() * 2]);
            assert_eq!(source, before);
            assert!(matches!(output_surface(&source, 0, Rect::new(0, 0, 257, 1), sample, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        }
    }
    #[test]
    fn derived_state_preserves_axes_without_rgb_profile_or_source_alias() {
        let mut source = Document::new("Original", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
        source
            .set_physical_resolution(photocraft_doc::PhysicalResolution { x_dpi: 144.123456789, y_dpi: 72.5, unit: photocraft_doc::PhysicalUnit::Inch })
            .unwrap();
        let gray = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true));
        let output = make_state(&source, output_name(&source.name, "R").unwrap(), gray, &Interrupt::NONE).unwrap();
        assert_eq!(output.doc.name, "Original_R");
        assert_eq!(output.doc.effective_resolution(), source.effective_resolution());
        assert!(output.doc.xcf_state.is_none());
        assert!(output.doc.source_color_context.is_none());
        assert!(output.doc.layers[0].locks.position && output.doc.layers[0].locks.transparency);
    }
}
