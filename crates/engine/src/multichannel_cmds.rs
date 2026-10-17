//! Image › Mode › Multichannel, and conversions out of it.
//!
//! A Multichannel document has no layers: its image is a set of ink channels, kept as spot
//! channels in [`Document::channels`] (value = ink density, 1 = solid; the compositor prints them
//! over white, see `photocraft_compose::multichannel`). Like Photoshop the conversion flattens and
//! turns each colour channel into an ink:
//!
//! * RGB → Cyan, Magenta, Yellow (the red, green and blue data read as ink: density = 1 − value);
//! * CMYK → Cyan, Magenta, Yellow, Black (the plates as they are);
//! * Lab → Alpha 1, Alpha 2, Alpha 3 (lightness, a, b data, printed in black);
//! * Grayscale → Black; Duotone → one channel per ink (its curve applied).
//!
//! Existing alpha channels stay alpha channels. Converting back reinterprets the channel data when
//! the count matches the target (3 → RGB / Lab, 4 → CMYK, 1 → Grayscale), the inverse of the
//! above; otherwise the printed look is converted.

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType, read_sample, write_sample};
use photocraft_doc::{AlphaChannel, Document, Layer, LayerContent};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
const CYAN: Color = Color::rgb(0., 0.682, 0.937);
const MAGENTA: Color = Color::rgb(0.925, 0., 0.549);
const YELLOW: Color = Color::rgb(1., 0.949, 0.);
const BLACK: Color = Color::rgb(0.137, 0.122, 0.125);
const SOURCES: [ColorMode; 5] = [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Grayscale, ColorMode::Duotone];
const WORK_LIMIT: u64 = 8_000_000_000;
fn error(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("Multichannel conversion: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled(control: &Interrupt<'_>, message: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { error(message) }
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    let state = session.active().ok_or("no document open")?;
    if SOURCES.contains(&state.doc.mode) {
        Ok(())
    } else {
        Err(format!("Multichannel needs an RGB, CMYK, Lab, Grayscale or Duotone image (the document is {:?})", state.doc.mode))
    }
}
/// Cheap dispatch validation before float normalization/parameter cloning. The
/// conventional profile/intent/bpc fields retain the old ignored-parameter
/// behavior here: none is resolved or used as path/profile authority.
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, target: ColorMode, _background: bool) -> Result<()> {
    if !params.is_null()
        && !params.as_object().is_some_and(|object| {
            object.len() <= 5 && object.keys().all(|key| matches!(key.as_str(), "documentId" | "expectedRevision" | "profile" | "intent" | "bpc"))
        })
    {
        return Err(error("expected bounded optional profile/intent/bpc and captured target"));
    }
    if params.get("profile").is_some_and(|value| !value.as_str().is_some_and(|value| !value.is_empty() && value.len() <= 4096 && !value.contains('\0'))) {
        return Err(error("profile must be a bounded nonempty string"));
    }
    if params.get("intent").is_some_and(|value| !value.as_str().is_some_and(|value| value.len() <= 32 && photocraft_cms::Intent::parse(value).is_some())) {
        return Err(error("invalid intent"));
    }
    if params.get("bpc").is_some_and(|value| !value.is_boolean()) {
        return Err(error("bpc must be boolean"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if target == ColorMode::Multichannel {
        if !SOURCES.contains(&state.doc.mode) {
            return Err(error("unsupported source mode"));
        }
    } else if state.doc.mode != ColorMode::Multichannel || !matches!(target, ColorMode::Rgb | ColorMode::Cmyk | ColorMode::Lab | ColorMode::Grayscale) {
        return Err(error("unsupported Multichannel destination"));
    }
    let captured = match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => false,
        (Some(id), Some(revision)) => {
            let exact = |value: &Value| -> Result<u64> {
                let text = value
                    .as_str()
                    .filter(|text| {
                        !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0'))
                    })
                    .ok_or_else(|| error("captured IDs must be canonical decimal strings"))?;
                text.parse().map_err(|_| error("captured ID exceeds range"))
            };
            let id = exact(id)?;
            if id == 0 || id != state.doc.id.0 || exact(revision)? != state.revision {
                return Err(error("captured target changed"));
            }
            true
        }
        _ => return Err(error("captured target pair is incomplete")),
    };
    if state.revision == u64::MAX || session.command_has_pending_job() || session.command_has_job_on(state.doc.id) {
        return Err(error("target revision/pending-work context refused"));
    }
    if captured
        && (state.floating.is_some()
            || state.coalesce.is_some()
            || session.has_jobs()
            || session.coalesce_request.is_some()
            || session.color_restrict.is_some())
    {
        return Err(error("captured target has floating or transient work"));
    }
    // These transitions have only the existing legacy numerical-plate contract.
    // No source graph, canonical indexed carrier or explicit TRC is reinterpreted.
    if !matches!(state.doc.depth, SampleType::U8 | SampleType::U16 | SampleType::F32)
        || state.doc.xcf_state.is_some()
        || state.doc.sample_encoding.is_some()
        || state.doc.working_profile_interpretation != photocraft_doc::WorkingProfileInterpretation::Icc
    {
        return Err(error("wide/indexed/current source interpretation is not qualified for numerical plates"));
    }
    if state.doc.mode == ColorMode::Multichannel && !state.doc.layers.is_empty() {
        return Err(error("Multichannel layers are not qualified for plate conversion"));
    }
    Ok(())
}
fn surface_admission(surface: &Surface, tiles: &mut u64, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    if !matches!(surface.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32) {
        return Err(error("wide samples cannot enter legacy f32 plate arithmetic"));
    }
    surface.try_validate_tile_origins_with(control).map_err(|e| controlled(control, e))?;
    *tiles = tiles.checked_add(surface.tile_count() as u64).ok_or_else(|| error("tile census overflow"))?;
    Ok(())
}
fn layer_admission(layers: &[Layer], depth: usize, count: &mut u64, tiles: &mut u64, control: &Interrupt<'_>) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(error("layer depth limit"));
    }
    for layer in layers {
        poll(control)?;
        *count = count.checked_add(1).filter(|count| *count <= 100_000).ok_or_else(|| error("layer count limit"))?;
        if layer.video.is_some() || layer.indexed_source.is_some() || layer.import_compositing.is_some() {
            return Err(error("imported/video/current layer composition is not qualified"));
        }
        if let Some(mask) = &layer.mask {
            surface_admission(&mask.surface, tiles, control)?;
        }
        if let Some(cache) = &layer.fill_cache {
            surface_admission(&cache.surface, tiles, control)?;
        }
        if let Some(surface) = layer.surface() {
            surface_admission(surface, tiles, control)?;
        }
        if let LayerContent::Smart(smart) = &layer.content
            && let Some(mask) = &smart.filter_mask
        {
            surface_admission(&mask.surface, tiles, control)?;
        }
        if matches!(&layer.content, LayerContent::BackedRaster(_)) {
            return Err(error("backed plate conversion unsupported"));
        }
        if let LayerContent::Group(group) = &layer.content {
            layer_admission(&group.children, depth + 1, count, tiles, control)?;
        }
    }
    Ok(())
}
fn ink_count(doc: &Document) -> Result<usize> {
    let count = match doc.mode {
        ColorMode::Rgb | ColorMode::Lab => 3,
        ColorMode::Cmyk => 4,
        ColorMode::Duotone if doc.duotone.as_ref().is_some_and(|duotone| !duotone.inks.is_empty()) => {
            doc.duotone.as_ref().map_or(0, |duotone| duotone.inks.len())
        }
        _ => 1,
    };
    if count > 4 {
        return Err(error("Duotone supports one to four inks"));
    }
    Ok(count)
}
fn admission(snapshot: &crate::DocumentSnapshot, target: ColorMode, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    snapshot.validate_resident()?;
    let canvas = snapshot.bounds();
    if canvas.is_empty() || snapshot.size.width > 300_000 || snapshot.size.height > 300_000 {
        return Err(error("invalid conversion canvas"));
    }
    let (mut layers, mut tiles) = (0, 0);
    layer_admission(&snapshot.layers, 0, &mut layers, &mut tiles, control)?;
    if snapshot.channels.len() > 100_000 {
        return Err(error("channel count limit"));
    }
    for channel in snapshot.channels.iter().chain(snapshot.quick_mask.iter()) {
        surface_admission(&channel.surface, &mut tiles, control)?;
    }
    if let Some(selection) = &snapshot.selection {
        surface_admission(selection, &mut tiles, control)?;
    }
    if let Some(duotone) = &snapshot.duotone {
        if duotone.inks.len() > 4 {
            return Err(error("Duotone ink count limit"));
        }
        for ink in &duotone.inks {
            poll(control)?;
            if ink.name.len() > 1 << 20 || ink.curve.len() > 4096 || ink.curve.iter().any(|point| !point.input.is_finite() || !point.output.is_finite()) {
                return Err(error("bounded finite Duotone ink definition required"));
            }
        }
    }
    let count = if target == ColorMode::Multichannel { ink_count(snapshot)? as u64 } else { 1 };
    let format = if target == ColorMode::Multichannel {
        PixelFormat::new(ColorMode::Grayscale, snapshot.depth, false)
    } else {
        PixelFormat::new(target, snapshot.depth, true)
    };
    let output_tiles = u64::from(canvas.width().div_ceil(TILE_SIZE as u32))
        .checked_mul(u64::from(canvas.height().div_ceil(TILE_SIZE as u32)))
        .ok_or_else(|| error("output tile span overflow"))?;
    let tile_bytes = (TILE_SIZE as u64).pow(2).checked_mul(format.bytes_per_pixel() as u64).ok_or_else(|| error("output tile size overflow"))?;
    let output =
        output_tiles.checked_mul(count).and_then(|n| n.checked_mul(tile_bytes.checked_add(1024)?)).ok_or_else(|| error("output plane ledger overflow"))?;
    let preparation = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(snapshot, preparation, control).map_err(|e| controlled(control, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, preparation, control).map_err(|e| controlled(control, e))? as u64;
    let raster = snapshot.resident_raster_footprint_with(control)?;
    let peak = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|n| n.checked_add(output))
        .and_then(|n| n.checked_add(384 << 20))
        .ok_or_else(|| error("conversion residency overflow"))?;
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(error("aggregate source/plane/model/scratch limit"));
    }
    let curve_work = snapshot.duotone.as_ref().map_or(1, |duotone| duotone.inks.iter().map(|ink| ink.curve.len() as u64).sum::<u64>().max(1));
    let samples = u64::from(canvas.width()).checked_mul(u64::from(canvas.height())).ok_or_else(|| error("canvas work overflow"))?;
    let work = samples
        .checked_mul((layers + snapshot.channels.len() as u64 + count).max(1))
        .and_then(|n| n.checked_mul(curve_work))
        .and_then(|n| n.checked_mul(64))
        .and_then(|n| n.checked_add(tiles.checked_mul(tiles.checked_add(output_tiles)?)?.checked_mul(1024)?))
        .ok_or_else(|| error("conversion work overflow"))?;
    if work > WORK_LIMIT {
        return Err(error("aggregate plate read/render/write work limit"));
    }
    poll(control)
}
fn bytes(length: usize, control: &Interrupt<'_>) -> Result<Vec<u8>> {
    poll(control)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| error("tile buffer allocation failed"))?;
    if bytes.capacity() > length.saturating_add(4096) {
        return Err(error("tile buffer capacity exceeds reservation"));
    }
    while bytes.len() < length {
        poll(control)?;
        bytes.resize(bytes.len().saturating_add(8192).min(length), 0);
    }
    Ok(bytes)
}
fn read_f32(surface: &Surface, part: Rect, control: &Interrupt<'_>) -> Result<Vec<f32>> {
    let raw = surface.try_read_interleaved_with(part, control).map_err(|e| controlled(control, e))?;
    let samples = raw.len().checked_div(surface.format().sample.bytes()).ok_or_else(|| error("sample size"))?;
    let mut values = Vec::new();
    values.try_reserve_exact(samples).map_err(|_| error("sample tile allocation failed"))?;
    if values.capacity() > samples.saturating_add(1024) {
        return Err(error("sample tile capacity exceeds reservation"));
    }
    for index in 0..samples {
        if index.is_multiple_of(256) {
            poll(control)?;
        }
        values.push(read_sample(&raw, surface.format().sample, index));
    }
    Ok(values)
}
fn encode(values: &[f32], format: PixelFormat, control: &Interrupt<'_>) -> Result<Vec<u8>> {
    let length = values.len().checked_mul(format.sample.bytes()).ok_or_else(|| error("encoded tile length overflow"))?;
    let mut output = bytes(length, control)?;
    for (index, value) in values.iter().enumerate() {
        if index.is_multiple_of(256) {
            poll(control)?;
        }
        write_sample(&mut output, format.sample, index, *value);
    }
    Ok(output)
}
fn plain(doc: &Document) -> Option<&Surface> {
    match doc.layers.as_slice() {
        [layer]
            if layer.visible
                && layer.opacity >= 1.
                && layer.fill_opacity >= 1.
                && layer.mask.is_none()
                && layer.vector_mask.is_none()
                && !photocraft_compose::effects::has_effects(layer) =>
        {
            match &layer.content {
                LayerContent::Raster(surface) if surface.format().mode == doc.pixel_format().mode => Some(surface),
                _ => None,
            }
        }
        _ => None,
    }
}
fn color_values(doc: &Document, part: Rect, control: &Interrupt<'_>) -> Result<Vec<f32>> {
    let format = doc.pixel_format();
    let cc = format.mode.color_channels();
    let pixels = (part.width() as usize).checked_mul(part.height() as usize).ok_or_else(|| error("tile area overflow"))?;
    let mut output = Vec::new();
    output.try_reserve_exact(pixels.checked_mul(cc).ok_or_else(|| error("tile channels overflow"))?).map_err(|_| error("color tile allocation failed"))?;
    if output.capacity() > pixels.checked_mul(cc).and_then(|n| n.checked_add(1024)).ok_or_else(|| error("color tile capacity overflow"))? {
        return Err(error("color tile capacity exceeded"));
    }
    if let Some(surface) = plain(doc) {
        let white = photocraft_raster::from_rgba(&format, [1., 1., 1., 1.]);
        let values = read_f32(surface, part, control)?;
        let channels = surface.channels();
        for (index, pixel) in values.chunks_exact(channels).enumerate() {
            if index.is_multiple_of(part.width() as usize) {
                poll(control)?;
            }
            let alpha = if surface.format().alpha { *pixel.last().ok_or_else(|| error("source alpha missing"))? } else { 1. };
            for channel in 0..cc {
                output.push(
                    *pixel.get(channel).ok_or_else(|| error("source color missing"))? * alpha
                        + *white.get(channel).ok_or_else(|| error("white color missing"))? * (1. - alpha),
                );
            }
        }
    } else {
        let buffer = photocraft_compose::render_with_interrupt(doc, part, control).map_err(|e| controlled(control, e))?;
        for (index, pixel) in buffer.px.iter().enumerate() {
            if index.is_multiple_of(part.width() as usize) {
                poll(control)?;
            }
            let [r, g, b, a] = *pixel;
            let converted = photocraft_raster::from_rgba(&format, [r * a + 1. * (1. - a), g * a + 1. * (1. - a), b * a + 1. * (1. - a), 1.]);
            output.extend_from_slice(converted.get(..cc).ok_or_else(|| error("converted color length"))?);
        }
    }
    Ok(output)
}
fn definitions(doc: &Document) -> Result<Vec<(String, Color)>> {
    let mut definitions = Vec::new();
    definitions.try_reserve_exact(ink_count(doc)?).map_err(|_| error("ink list allocation failed"))?;
    match doc.mode {
        ColorMode::Rgb | ColorMode::Cmyk => {
            for (name, color) in
                [("Cyan", CYAN), ("Magenta", MAGENTA), ("Yellow", YELLOW)].into_iter().chain((doc.mode == ColorMode::Cmyk).then_some(("Black", BLACK)))
            {
                definitions.push((name.to_owned(), color));
            }
        }
        ColorMode::Lab => {
            for index in 1..=3 {
                definitions.push((format!("Alpha {index}"), Color::BLACK));
            }
        }
        ColorMode::Duotone if doc.duotone.as_ref().is_some_and(|duotone| !duotone.inks.is_empty()) => {
            for ink in &doc.duotone.as_ref().ok_or_else(|| error("duotone missing"))?.inks {
                definitions.push((ink.name.clone(), Color::rgb(ink.color[0], ink.color[1], ink.color[2])));
            }
        }
        _ => definitions.push(("Black".to_owned(), Color::BLACK)),
    }
    Ok(definitions)
}
fn to_plates(source: &Document, control: &Interrupt<'_>) -> Result<Vec<AlphaChannel>> {
    let definitions = definitions(source)?;
    let gray = PixelFormat::new(ColorMode::Grayscale, source.depth, false);
    let mut channels = Vec::new();
    channels
        .try_reserve_exact(definitions.len().checked_add(source.channels.len()).ok_or_else(|| error("channel count overflow"))?)
        .map_err(|_| error("channel allocation failed"))?;
    for (name, color) in definitions {
        channels.push(AlphaChannel { spot: Some((color, 0.)), ..AlphaChannel::new(name, Surface::new(gray)) });
    }
    let cc = source.pixel_format().mode.color_channels();
    for coordinate in source.bounds().tiles() {
        poll(control)?;
        let part = coordinate.rect().intersect(&source.bounds());
        let colors = color_values(source, part, control)?;
        let pixels = (part.width() as usize).checked_mul(part.height() as usize).ok_or_else(|| error("plate tile area overflow"))?;
        for (channel_index, channel) in channels.iter_mut().enumerate() {
            poll(control)?;
            let mut values = Vec::new();
            values.try_reserve_exact(pixels).map_err(|_| error("plate tile allocation failed"))?;
            if values.capacity() > pixels.saturating_add(1024) {
                return Err(error("plate tile capacity exceeded"));
            }
            for (index, pixel) in colors.chunks_exact(cc).enumerate() {
                if index.is_multiple_of(part.width() as usize) {
                    poll(control)?;
                }
                let density = match source.mode {
                    ColorMode::Rgb | ColorMode::Lab => 1. - *pixel.get(channel_index).ok_or_else(|| error("plate color missing"))?,
                    ColorMode::Cmyk => *pixel.get(channel_index).ok_or_else(|| error("plate color missing"))?,
                    ColorMode::Duotone if source.duotone.as_ref().is_some_and(|duotone| !duotone.inks.is_empty()) => source
                        .duotone
                        .as_ref()
                        .and_then(|duotone| duotone.inks.get(channel_index))
                        .ok_or_else(|| error("ink curve missing"))?
                        .density(1. - *pixel.first().ok_or_else(|| error("gray plane missing"))?),
                    _ => 1. - *pixel.first().ok_or_else(|| error("gray plane missing"))?,
                };
                values.push(density.clamp(0., 1.));
            }
            let raw = encode(&values, gray, control)?;
            channel.surface.try_write_interleaved_with(part, &raw, control).map_err(|e| controlled(control, e))?;
        }
    }
    for channel in &mut channels {
        channel.surface.try_prune_with(control).map_err(|e| controlled(control, e))?;
    }
    for channel in &source.channels {
        poll(control)?;
        let mut channel = channel.clone();
        if channel.surface.format() != gray {
            channel.surface = channel.surface.try_convert_with(gray, control).map_err(|e| controlled(control, e))?;
        }
        channels.push(channel);
    }
    poll(control)?;
    Ok(channels)
}
/// Printed fallback uses the exact existing ink order, inversion and solidity
/// arithmetic, one tile at a time; no new CMYK/TRC/profile interpretation.
struct PrintedInk<'a> {
    channel: &'a AlphaChannel,
    bounds: Rect,
}
fn printed(inks: &[PrintedInk<'_>], part: Rect, control: &Interrupt<'_>) -> Result<Vec<[f32; 4]>> {
    let pixels = (part.width() as usize).checked_mul(part.height() as usize).ok_or_else(|| error("print tile area overflow"))?;
    let mut output = Vec::new();
    output.try_reserve_exact(pixels).map_err(|_| error("print tile allocation failed"))?;
    if output.capacity() > pixels.saturating_add(1024) {
        return Err(error("print tile capacity exceeded"));
    }
    while output.len() < pixels {
        poll(control)?;
        output.resize(output.len().saturating_add(256).min(pixels), [1., 1., 1., 1.]);
    }
    for prepared in inks {
        poll(control)?;
        let channel = prepared.channel;
        let Some((ink, solidity)) = channel.spot else { continue };
        if prepared.bounds.intersect(&part).is_empty() {
            continue;
        }
        let color = ink.to_rgb();
        let solidity = solidity.clamp(0., 1.);
        let samples = read_f32(&channel.surface, part, control)?;
        for (index, (pixel, values)) in output.iter_mut().zip(samples.chunks_exact(channel.surface.channels())).enumerate() {
            if index.is_multiple_of(part.width() as usize) {
                poll(control)?;
            }
            let value = values.first().copied().ok_or_else(|| error("ink sample missing"))?.clamp(0., 1.);
            if value <= 0. {
                continue;
            }
            let keep = 1. - solidity * value;
            for (component, ink) in pixel.iter_mut().take(3).zip(color) {
                *component = keep * *component * (1. - value + value * ink) + solidity * value * ink;
            }
        }
    }
    Ok(output)
}
fn from_plates(source: &Document, target: ColorMode, control: &Interrupt<'_>) -> Result<(Surface, &'static str)> {
    let mut inks = Vec::new();
    inks.try_reserve_exact(source.channels.len()).map_err(|_| error("ink reference allocation failed"))?;
    for channel in &source.channels {
        poll(control)?;
        if channel.spot.is_some() {
            inks.push(channel);
        }
    }
    let reinterpret = matches!((target, inks.len()), (ColorMode::Rgb | ColorMode::Lab, 3) | (ColorMode::Cmyk, 4) | (ColorMode::Grayscale, 1));
    let mut prepared = Vec::new();
    if !reinterpret {
        prepared.try_reserve_exact(inks.len()).map_err(|_| error("print bounds allocation failed"))?;
        for channel in &inks {
            poll(control)?;
            prepared.push(PrintedInk { channel, bounds: channel.surface.try_content_bounds_with_interrupt(control).map_err(|e| controlled(control, e))? });
        }
    }
    let format = PixelFormat::new(target, source.depth, true);
    let cc = format.mode.color_channels();
    let mut output = Surface::new(format);
    for coordinate in source.bounds().tiles() {
        poll(control)?;
        let part = coordinate.rect().intersect(&source.bounds());
        let pixels = (part.width() as usize).checked_mul(part.height() as usize).ok_or_else(|| error("tile area overflow"))?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(pixels.checked_mul(cc + 1).ok_or_else(|| error("output channels overflow"))?)
            .map_err(|_| error("output tile allocation failed"))?;
        let length = pixels.checked_mul(cc + 1).ok_or_else(|| error("output sample count overflow"))?;
        if values.capacity() > length.saturating_add(1024) {
            return Err(error("output tile capacity exceeded"));
        }
        while values.len() < length {
            poll(control)?;
            values.resize(values.len().saturating_add(1024).min(length), 0.);
        }
        if reinterpret {
            for (index, ink) in inks.iter().enumerate() {
                poll(control)?;
                let samples = read_f32(&ink.surface, part, control)?;
                for (ordinal, (pixel, density)) in values.chunks_exact_mut(cc + 1).zip(samples.chunks_exact(ink.surface.channels())).enumerate() {
                    if ordinal.is_multiple_of(part.width() as usize) {
                        poll(control)?;
                    }
                    let value = *density.first().ok_or_else(|| error("ink density missing"))?;
                    *pixel.get_mut(index).ok_or_else(|| error("target plate missing"))? = if target == ColorMode::Cmyk { value } else { 1. - value };
                }
            }
            for (ordinal, pixel) in values.chunks_exact_mut(cc + 1).enumerate() {
                if ordinal.is_multiple_of(part.width() as usize) {
                    poll(control)?;
                }
                *pixel.get_mut(cc).ok_or_else(|| error("target alpha missing"))? = 1.;
            }
        } else {
            for (ordinal, (pixel, shown)) in values.chunks_exact_mut(cc + 1).zip(printed(&prepared, part, control)?).enumerate() {
                if ordinal.is_multiple_of(part.width() as usize) {
                    poll(control)?;
                }
                let [r, g, b, _] = shown;
                let converted = photocraft_raster::from_rgba(&format, [r, g, b, 1.]);
                pixel.copy_from_slice(converted.get(..cc + 1).ok_or_else(|| error("converted target channels missing"))?);
            }
        }
        let raw = encode(&values, format, control)?;
        output.try_write_interleaved_with(part, &raw, control).map_err(|e| controlled(control, e))?;
    }
    output.try_prune_with(control).map_err(|e| controlled(control, e))?;
    Ok((output, if reinterpret { "channels" } else { "appearance" }))
}
fn convert(session: &mut Session, target: ColorMode, params: &Value) -> Result<Value> {
    dispatch_preflight(session, params, target, session.command_is_background())?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    state.doc.validate_resident()?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let work_base = baseline.clone();
    let label = if target == ColorMode::Multichannel { "Multichannel" } else { "Mode Change" };
    crate::jobs::run(
        session,
        label,
        true,
        move |ctx| {
            ctx.stage(0., 0.95, label, |control| {
                admission(&work_base, target, control)?;
                let (document, active, response) = if target == ColorMode::Multichannel {
                    let channels = to_plates(&work_base, control)?;
                    poll(control)?;
                    let names = channels.iter().take(ink_count(&work_base)?).map(|channel| channel.name.clone()).collect::<Vec<_>>();
                    let mut document = (*work_base).clone();
                    document.channels = channels;
                    document.layers.clear();
                    document.mode = ColorMode::Multichannel;
                    document.quick_mask = None;
                    document.color_table = None;
                    document.duotone = None;
                    document.icc_profile = None;
                    (document, None, json!({"mode":"multichannel","channels":names}))
                } else {
                    let (surface, how) = from_plates(&work_base, target, control)?;
                    poll(control)?;
                    let mut document = (*work_base).clone();
                    let mut others = Vec::new();
                    others.try_reserve_exact(document.channels.len()).map_err(|_| error("retained alpha channel allocation failed"))?;
                    for channel in std::mem::take(&mut document.channels) {
                        poll(control)?;
                        if channel.spot.is_none() {
                            others.push(channel);
                        }
                    }
                    let mut background = Layer::new("Background", LayerContent::Raster(surface));
                    background.locks.transparency = true;
                    background.locks.position = true;
                    let active = Some(background.id);
                    document.layers.clear();
                    document.layers.try_reserve(1).map_err(|_| error("background layer allocation failed"))?;
                    document.layers.push(background);
                    document.channels = others;
                    document.mode = target;
                    (document, active, json!({"mode":format!("{target:?}").to_lowercase(),"from":"multichannel","converted":how}))
                };
                poll(control)?;
                Ok((document, active, response, ctx.clone()))
            })
        },
        move |session, (document, active, response, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id || current.revision != revision || !current.doc.shares_state(&baseline) {
                return Err(error("captured target changed during preparation"));
            }
            ctx.stage(0.95, 1., label, |control| {
                poll(control)?;
                session.edit_with_interrupt(label, control, move |current, selected| {
                    *current = document;
                    *selected = active;
                    Ok(())
                })?;
                Ok(response)
            })
        },
    )
}
fn to_multichannel(session: &mut Session, params: &Value) -> Result<Value> {
    convert(session, ColorMode::Multichannel, params)
}
pub(crate) fn convert_from(session: &mut Session, target: ColorMode, params: &Value) -> Result<Value> {
    convert(session, target, params)
}

/// Names of the ink (spot) channels that make up a Multichannel image.
pub fn ink_names(doc: &Document) -> Vec<String> {
    doc.channels.iter().filter(|c| c.spot.is_some()).map(|c| c.name.clone()).collect()
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "image.mode.multichannel",
        label: "Multichannel",
        menu: &["Image", "Mode"],
        shortcut: None,
        params: r##"{documentId:canonical decimal string,expectedRevision:canonical decimal string} (optional paired captured target; flattens; RGB → Cyan/Magenta/Yellow, CMYK → Cyan/Magenta/Yellow/Black, Lab → Alpha 1–3, Grayscale → Black, Duotone → its inks)"##,
        enabled,
        run: to_multichannel,
        journal: true,
    }]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod controlled_tests;
