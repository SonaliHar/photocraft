//! Explicit native channel assembly into fresh unassigned RGB; not a CMS/source equivalence.
use crate::{DocState, DocumentSnapshot, EngineError, Result, Session};
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document, Layer, LayerContent, WorkingProfileInterpretation};
use photocraft_geom::TILE_SIZE;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::Value;
const CMD: &str = "channel.merge";
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: message.into() }
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn decimal(v: &Value) -> Result<u64> {
    let s = v
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad("canonical captured identity required"))?;
    s.parse().map_err(|_| bad("captured identity range"))
}
fn source_surface(doc: &Document) -> Result<&Surface> {
    if doc.mode != ColorMode::Grayscale
        || !matches!(doc.depth, SampleType::U32 | SampleType::F64)
        || doc.xcf_state.is_some()
        || doc.source_color_context.is_some()
        || doc.color_table.is_some()
        || doc.icc_profile.is_some()
        || doc.working_profile_interpretation != WorkingProfileInterpretation::Icc
        || doc.layers.len() != 1
    {
        return Err(bad("native RGB assembly requires source-free unassigned same-depth Gray documents"));
    }
    let layer = doc.layers.first().ok_or_else(|| bad("missing source raster"))?;
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
        return Err(bad("native Gray assembly raster must be neutral"));
    }
    let LayerContent::Raster(surface) = &layer.content else { return Err(bad("native Gray assembly requires resident raster")) };
    if surface.format() != doc.pixel_format() {
        return Err(bad("source raster format mismatch"));
    }
    Ok(surface)
}
struct Request {
    indices: Vec<usize>,
    name: String,
}
fn request(session: &Session, params: &Value) -> Result<Option<Request>> {
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if session.documents().len() > 4096 {
        return Err(bad("open document count limit"));
    }
    let mode = params.get("mode").and_then(Value::as_str).unwrap_or("rgb");
    let color_count = match mode {
        "rgb" => 3,
        "cmyk" => 4,
        "lab" => 3,
        _ => 3,
    };
    let mut indices = Vec::new();
    let requested = params.get("documents").and_then(Value::as_array);
    let count = requested.map_or(color_count, Vec::len);
    if count > 4096 {
        return Err(bad("source document count limit"));
    }
    indices.try_reserve_exact(count).map_err(|_| bad("source index allocation"))?;
    if indices.capacity() > count.saturating_add(64) {
        return Err(bad("source index capacity"));
    }
    if let Some(values) = requested {
        for v in values {
            let index = v.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n < session.documents().len()).ok_or_else(|| bad("bad document index"))?;
            indices.push(index);
        }
    } else {
        let index = session.active_index().ok_or(EngineError::NoDocument)?;
        if active.doc.mode == ColorMode::Grayscale {
            indices.push(index);
        }
        for (i, state) in session.documents().iter().enumerate() {
            if indices.len() == color_count {
                break;
            }
            if i != index && state.doc.mode == ColorMode::Grayscale && state.doc.size == active.doc.size {
                indices.push(i);
            }
        }
    }
    let wide = indices.iter().any(|i| session.documents().get(*i).is_some_and(|state| matches!(state.doc.depth, SampleType::U32 | SampleType::F64)));
    if !wide {
        return Ok(None);
    }
    if mode != "rgb" {
        return Err(bad("native channel assembly currently supports RGB output only"));
    }
    if !params.is_null()
        && !params
            .as_object()
            .is_some_and(|o| o.len() <= 5 && o.keys().all(|k| matches!(k.as_str(), "mode" | "name" | "documents" | "documentId" | "expectedRevision")))
    {
        return Err(bad("bounded Merge parameters required"));
    }
    if params.get("mode").is_some_and(|v| v.as_str() != Some("rgb")) || params.get("documents").is_some_and(|v| !v.is_array()) {
        return Err(bad("invalid native Merge parameter type"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if decimal(id)? == active.doc.id.0 && decimal(revision)? == active.revision => {}
        _ => return Err(bad("captured active document changed")),
    }
    if indices.len() < 3
        || session.accounted_slots.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || active.doc.accounted().is_some()
        || active.floating.is_some()
        || active.coalesce.is_some()
    {
        return Err(bad("finish transient/accounted work before native Merge"));
    }
    let first = indices.first().and_then(|i| session.documents().get(*i)).ok_or_else(|| bad("missing first source"))?;
    let mut seen = std::collections::HashSet::new();
    seen.try_reserve(indices.len()).map_err(|_| bad("source identity allocation"))?;
    if seen.capacity() > indices.len().saturating_mul(4).saturating_add(64) {
        return Err(bad("source identity capacity"));
    }
    for index in &indices {
        if !seen.insert(*index) {
            return Err(bad("duplicate source document index"));
        }
        let state = session.documents().get(*index).ok_or_else(|| bad("missing source"))?;
        if state.doc.size != first.doc.size
            || state.doc.depth != first.doc.depth
            || state.doc.accounted().is_some()
            || state.floating.is_some()
            || state.coalesce.is_some()
        {
            return Err(bad("sources must have identical size/depth and idle resident state"));
        }
        if session.smart_links.iter().any(|link| link.parent == state.doc.id || link.child == state.doc.id) {
            return Err(bad("smart contents assembly unsupported"));
        }
        source_surface(&state.doc)?;
    }
    let name = match params.get("name") {
        None => "Untitled",
        Some(v) => v.as_str().ok_or_else(|| bad("name must be UTF-8 string"))?,
    };
    if name.len() > 4096 || name.contains('\0') {
        return Err(bad("bounded name without NUL required"));
    }
    let mut owned = String::new();
    owned.try_reserve_exact(name.len()).map_err(|_| bad("name allocation"))?;
    if owned.capacity() > name.len().saturating_add(64) {
        return Err(bad("name capacity"));
    }
    owned.push_str(name);
    Ok(Some(Request { indices, name: owned }))
}
pub(super) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<bool> {
    request(session, params).map(|r| r.is_some())
}
fn io_error(error: photocraft_io::IoError) -> EngineError {
    match error {
        photocraft_io::IoError::Cancelled => EngineError::Cancelled,
        e => EngineError::Other(e.to_string()),
    }
}
fn admission(sources: &[(DocumentSnapshot, u64)], ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    let first = &sources.first().ok_or_else(|| bad("missing first source"))?.0;
    super::source_dimensions(first, CMD)?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let mut retained = 0u64;
    let mut work = 0u64;
    for (source, _) in sources {
        poll(ctl)?;
        let surface = source_surface(source)?;
        surface.try_validate_tile_origins_with(ctl)?;
        let raster = DocumentSnapshot::resident_raster_footprint_of(source, ctl)?;
        let model = photocraft_io::retained_native_model_bytes(source, limits, ctl).map_err(io_error)? as u64;
        let payload = photocraft_io::retained_native_payload_bytes(source, limits, ctl).map_err(io_error)? as u64;
        retained = retained
            .checked_add(raster.retained_bytes)
            .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
            .and_then(|n| n.checked_add(model.checked_mul(4)?))
            .and_then(|n| n.checked_add(payload))
            .ok_or_else(|| bad("source resident overflow"))?;
        work = work.checked_add(model).and_then(|n| n.checked_add(payload)).ok_or_else(|| bad("source work overflow"))?;
    }
    if let Some(resolution) = &first.physical_resolution {
        resolution.validate().map_err(|e| bad(&e.to_string()))?;
    } else {
        photocraft_doc::PhysicalResolution::square(f64::from(first.resolution_dpi)).validate().map_err(|e| bad(&e.to_string()))?;
    }
    let tiles = u64::from(first.size.width)
        .div_ceil(TILE_SIZE as u64)
        .checked_mul(u64::from(first.size.height).div_ceil(TILE_SIZE as u64))
        .ok_or_else(|| bad("output tile count"))?;
    // RGB+alpha plus every extra source's Gray sample. Includes all output tiles/maps,
    // two maximum scratch/replacement tiles, DocState/slot metadata, and names.
    let sample_bytes = first.depth.bytes() as u64;
    let components = (sources.len() as u64).checked_add(1).ok_or_else(|| bad("output component count"))?;
    let tile_pixels = (TILE_SIZE as u64) * (TILE_SIZE as u64);
    let output = tiles
        .checked_mul(tile_pixels)
        .and_then(|n| n.checked_mul(sample_bytes.checked_mul(components)?))
        .and_then(|n| n.checked_add(tiles.checked_mul((sources.len() as u64).checked_mul(1024)?)?))
        .ok_or_else(|| bad("output resident overflow"))?;
    retained
        .checked_add(output)
        .and_then(|n| n.checked_add(tile_pixels.checked_mul(sample_bytes.checked_mul(12)?)?))
        .and_then(|n| n.checked_add(4 << 20))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad("aggregate native Merge resident limit"))?;
    first
        .size
        .area()
        .checked_mul((sources.len() as u64).checked_mul(128).ok_or_else(|| bad("output work"))?)
        .and_then(|n| n.checked_add(work))
        .filter(|n| *n <= 1 << 32)
        .ok_or_else(|| bad("native Merge work limit"))?;
    poll(ctl)
}
fn read_gray(source: &Surface, x: i32, y: i32, sample: SampleType, raw: &mut [u8; 16]) -> Result<()> {
    let width = sample.bytes();
    let bytes = raw.get_mut(..width * 2).ok_or_else(|| bad("Gray pixel width"))?;
    source.try_read_pixel_bytes(x, y, bytes)?;
    let alpha = bytes.get(width..).ok_or_else(|| bad("Gray alpha width"))?;
    let opaque = match sample {
        SampleType::U32 => alpha == u32::MAX.to_ne_bytes(),
        SampleType::F64 => alpha == 1f64.to_ne_bytes(),
        _ => false,
    };
    if !opaque {
        return Err(bad("native Merge requires opaque Gray canvas pixels"));
    }
    if sample == SampleType::F64 {
        let word: [u8; 8] = bytes.get(..width).ok_or_else(|| bad("Gray word"))?.try_into().map_err(|_| bad("Gray word width"))?;
        let value = f64::from_ne_bytes(word);
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(bad("native Merge supports finite-unit scalar values only"));
        }
    }
    Ok(())
}
fn assemble(sources: &[(DocumentSnapshot, u64)], name: String, ctl: &Interrupt<'_>) -> Result<DocState> {
    admission(sources, ctl)?;
    let first = &sources.first().ok_or_else(|| bad("first source"))?.0;
    let mut surfaces = Vec::new();
    surfaces.try_reserve_exact(sources.len()).map_err(|_| bad("source surface reference allocation"))?;
    if surfaces.capacity() > sources.len().saturating_add(64) {
        return Err(bad("source reference capacity"));
    }
    for (source, _) in sources {
        poll(ctl)?;
        surfaces.push(source_surface(source)?);
    }
    let sample = first.depth;
    let width = sample.bytes();
    let format = PixelFormat::new(ColorMode::Rgb, sample, true);
    let mut doc = Document::new(name, first.size, ColorMode::Rgb, sample);
    doc.set_physical_resolution(first.effective_resolution()).map_err(|e| bad(&e.to_string()))?;
    let mut output = Surface::new(format);
    let mut raw = Vec::new();
    let capacity = (TILE_SIZE as usize) * (TILE_SIZE as usize) * width * 4;
    raw.try_reserve_exact(capacity).map_err(|_| bad("tile buffer allocation"))?;
    if raw.capacity() > capacity.saturating_add(64) {
        return Err(bad("tile buffer capacity"));
    }
    let mut pixel = [0u8; 16];
    let mut one = [0u8; 8];
    if sample == SampleType::U32 {
        one.get_mut(..4).ok_or_else(|| bad("unit width"))?.copy_from_slice(&u32::MAX.to_ne_bytes());
    } else {
        one.copy_from_slice(&1f64.to_ne_bytes());
    }
    for tile in doc.bounds().tiles() {
        poll(ctl)?;
        let area = tile.rect().intersect(&doc.bounds());
        raw.clear();
        for y in area.y0..area.y1 {
            poll(ctl)?;
            for x in area.x0..area.x1 {
                poll(ctl)?;
                for surface in surfaces.get(..3).ok_or_else(|| bad("RGB source count"))? {
                    read_gray(surface, x, y, sample, &mut pixel)?;
                    raw.extend_from_slice(pixel.get(..width).ok_or_else(|| bad("component width"))?);
                }
                raw.extend_from_slice(one.get(..width).ok_or_else(|| bad("opaque width"))?);
            }
        }
        let mut replacement = Surface::new(format);
        replacement.try_write_interleaved_with(area, &raw, ctl)?;
        poll(ctl)?;
        output.put_tiles(replacement.tiles().map(|(position, tile)| (*position, std::sync::Arc::clone(tile))));
    }
    output.try_prune_with(ctl)?;
    let mut background = Layer::raster("Background", format);
    background.locks.transparency = true;
    background.locks.position = true;
    background.content = LayerContent::Raster(output);
    doc.layers.try_reserve_exact(1).map_err(|_| bad("background allocation"))?;
    doc.layers.push(background);
    let extras = sources.len().checked_sub(3).ok_or_else(|| bad("source count"))?;
    doc.channels.try_reserve_exact(extras).map_err(|_| bad("saved channel allocation"))?;
    if doc.channels.capacity() > extras.saturating_add(64) {
        return Err(bad("saved channel capacity"));
    }
    let gray = PixelFormat::new(ColorMode::Grayscale, sample, false);
    for (index, source) in surfaces.iter().skip(3).enumerate() {
        poll(ctl)?;
        let mut channel = Surface::new(gray);
        for tile in doc.bounds().tiles() {
            poll(ctl)?;
            let area = tile.rect().intersect(&doc.bounds());
            raw.clear();
            for y in area.y0..area.y1 {
                poll(ctl)?;
                for x in area.x0..area.x1 {
                    poll(ctl)?;
                    read_gray(source, x, y, sample, &mut pixel)?;
                    raw.extend_from_slice(pixel.get(..width).ok_or_else(|| bad("saved word width"))?);
                }
            }
            let mut replacement = Surface::new(gray);
            replacement.try_write_interleaved_with(area, &raw, ctl)?;
            poll(ctl)?;
            channel.put_tiles(replacement.tiles().map(|(position, tile)| (*position, std::sync::Arc::clone(tile))));
        }
        channel.try_prune_with(ctl)?;
        // The ordinal is bounded to 4096; this tiny name is prepared before publication.
        doc.channels.push(AlphaChannel::new(format!("Alpha {}", index + 1), channel));
    }
    poll(ctl)?;
    DocState::new_with_interrupt(doc, None, ctl)
}
pub(super) fn try_run(session: &mut Session, params: &Value) -> Result<Option<Value>> {
    let Some(request) = request(session, params)? else { return Ok(None) };
    let anchor = session.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let anchor_revision = session.active().ok_or(EngineError::NoDocument)?.revision;
    let mut sources = Vec::new();
    sources.try_reserve_exact(request.indices.len()).map_err(|_| bad("source snapshot allocation"))?;
    if sources.capacity() > request.indices.len().saturating_add(64) {
        return Err(bad("source snapshot capacity"));
    }
    for index in request.indices {
        let state = session.documents().get(index).ok_or_else(|| bad("missing source"))?;
        sources.push((state.doc.clone(), state.revision));
    }
    let result = crate::jobs::run(
        session,
        "Merge Channels",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Merge Channels", |ctl| {
                let output = assemble(&sources, request.name, ctl)?;
                Ok((output, sources, ctx.clone()))
            })
        },
        move |session, (output, sources, ctx)| {
            ctx.stage(0.95, 1., "Merge Channels", |ctl| session.publish_merge_batch_with_interrupt(&anchor, anchor_revision, &sources, output, ctl))
        },
    )?;
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_geom::Rect;
    fn gray(value: &[u8], sample: SampleType) -> Document {
        let mut doc = Document::new("Gray", photocraft_geom::Size::new(257, 1), ColorMode::Grayscale, sample);
        let mut pixel = value.to_vec();
        if sample == SampleType::U32 {
            pixel.extend_from_slice(&u32::MAX.to_ne_bytes());
        } else {
            pixel.extend_from_slice(&1f64.to_ne_bytes());
        }
        let mut layer = Layer::raster("Background", doc.pixel_format());
        layer.content = LayerContent::Raster(Surface::try_with_default_bytes(doc.pixel_format(), &pixel).unwrap());
        doc.layers.push(layer);
        doc
    }
    #[test]
    fn raw_rgb_assembly_retains_low_bits_and_extra_channel_words() {
        for sample in [SampleType::U32, SampleType::F64] {
            let words: Vec<Vec<u8>> = if sample == SampleType::U32 {
                [0x80000001u32, 7, u32::MAX - 1, 19].into_iter().map(|v| v.to_ne_bytes().to_vec()).collect()
            } else {
                [0.25 + 2f64.powi(-45), -0.0, 0.75, 0.5 + 2f64.powi(-48)].into_iter().map(|v| v.to_ne_bytes().to_vec()).collect()
            };
            let mut sources = Vec::new();
            for word in &words {
                let state = DocState::new(gray(word, sample), None).unwrap();
                sources.push((state.doc, state.revision));
            }
            let before = sources.iter().map(|(doc, _)| doc.clone()).collect::<Vec<_>>();
            let output = assemble(&sources, "Merged".into(), &Interrupt::NONE).unwrap();
            let surface = output.doc.layers[0].surface().unwrap();
            let mut actual = vec![0; sample.bytes() * 4];
            surface.try_read_pixel_bytes(256, 0, &mut actual).unwrap();
            assert_eq!(&actual[..sample.bytes() * 3], words[..3].iter().flatten().copied().collect::<Vec<_>>());
            surface.try_read_pixel_bytes(257, 0, &mut actual).unwrap();
            assert_eq!(actual, vec![0; sample.bytes() * 4]);
            let mut saved = vec![0; sample.bytes()];
            output.doc.channels[0].surface.try_read_pixel_bytes(256, 0, &mut saved).unwrap();
            assert_eq!(saved, words[3]);
            assert_eq!(output.doc.channels[0].name, "Alpha 1");
            assert!(output.doc.icc_profile.is_none());
            assert_eq!(output.doc.working_profile_interpretation, WorkingProfileInterpretation::Icc);
            for ((source, _), original) in sources.iter().zip(before) {
                assert!(source.shares_state(&original));
            }
            assert!(matches!(assemble(&sources, "cancelled".into(), &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        }
    }
    #[test]
    fn source_order_first_axes_and_late_invalid_scalar_are_explicit() {
        let mut session = Session::new();
        for value in [0.125f64, 0.25, 0.5] {
            session.add_document(gray(&value.to_ne_bytes(), SampleType::F64), None).unwrap();
        }
        let first = session.documents()[1].doc.clone();
        let sources = [1usize, 2, 0].map(|index| (session.documents()[index].doc.clone(), session.documents()[index].revision));
        let output = assemble(&sources, "ordered".into(), &Interrupt::NONE).unwrap();
        assert_eq!(output.doc.effective_resolution(), first.effective_resolution());
        let mut raw = [0; 32];
        output.doc.layers[0].surface().unwrap().try_read_pixel_bytes(0, 0, &mut raw).unwrap();
        assert_eq!(&raw[..8], &0.25f64.to_ne_bytes());
        let mut invalid = gray(&0.5f64.to_ne_bytes(), SampleType::F64);
        invalid.layers[0]
            .surface_mut()
            .unwrap()
            .try_write_interleaved(Rect::new(256, 0, 257, 1), &[f64::NAN.to_ne_bytes(), 1f64.to_ne_bytes()].concat())
            .unwrap();
        let invalid = DocState::new(invalid, None).unwrap();
        let inputs = vec![sources[0].clone(), sources[1].clone(), (invalid.doc, invalid.revision)];
        assert!(assemble(&inputs, "invalid".into(), &Interrupt::NONE).is_err());
        assert_eq!(session.documents().len(), 3);
        assert!(first.shares_state(&session.documents()[1].doc));
        assert!(request(&session, &serde_json::json!({"documents":[0,0,1]})).is_err());
    }
}
