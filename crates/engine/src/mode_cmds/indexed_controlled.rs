//! Controlled existing Indexed Color quantization. DEFAULT keeps the quantizer's
//! legacy expanded RGBA bytes. canonicalIndexed is an explicit lossy conversion
//! policy: returned ordinals select palette RGB even under alpha zero. Neither
//! route infers GIMP source admission or a new source/profile interpretation.
use super::{bad, parse_dither, parse_forced};
use crate::{EngineError, Result, Session};
use photocraft_algo::quantize::{self, Dither, Forced, PaletteKind};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{ColorTable, Document, EditableIndexedLayer, EditableIndexedRaster, Layer, LayerContent};
use photocraft_geom::TILE_SIZE;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface};
use serde_json::{Value, json};
use std::sync::Arc;
const CMD: &str = "image.mode.indexedColor";
#[derive(Clone, Copy)]
struct Settings {
    kind: PaletteKind,
    forced: Forced,
    dither: Dither,
    colors: usize,
    amount: f32,
    transparency: bool,
    canonical: bool,
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn error(control: &Interrupt<'_>, e: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(CMD, e.to_string()) }
}
fn settings(params: &Value) -> Result<Settings> {
    if !params.is_null()
        && !params.as_object().is_some_and(|object| {
            object.len() <= 9
                && object.keys().all(|key| {
                    matches!(
                        key.as_str(),
                        "palette" | "forced" | "dither" | "colors" | "amount" | "transparency" | "canonicalIndexed" | "documentId" | "expectedRevision"
                    )
                })
        })
    {
        return Err(bad(CMD, "expected bounded Indexed Color parameters"));
    }
    let text = |key, default| -> Result<&str> {
        match params.get(key) {
            None => Ok(default),
            Some(value) => value.as_str().filter(|s| s.len() <= 32 && !s.contains('\0')).ok_or_else(|| bad(CMD, "palette/forced/dither must be bounded names")),
        }
    };
    let number = |key, default: f64| -> Result<f64> {
        match params.get(key) {
            None => Ok(default),
            Some(value) => value.as_f64().filter(|n| n.is_finite()).ok_or_else(|| bad(CMD, "colors/amount must be finite numbers")),
        }
    };
    let boolean = |key, default| -> Result<bool> {
        match params.get(key) {
            None => Ok(default),
            Some(value) => value.as_bool().ok_or_else(|| bad(CMD, "transparency/canonicalIndexed must be boolean")),
        }
    };
    let colors = number("colors", 256.)?;
    if !(2.0..=256.0).contains(&colors) || colors.fract() != 0.0 {
        return Err(bad(CMD, "colors must be an integer between 2 and 256"));
    }
    Ok(Settings {
        kind: PaletteKind::from_id(text("palette", "selective")?).ok_or_else(|| bad(CMD, "unknown palette"))?,
        forced: parse_forced(text("forced", "blackWhite")?).ok_or_else(|| bad(CMD, "unknown forced colors"))?,
        dither: parse_dither(text("dither", "diffusion")?).ok_or_else(|| bad(CMD, "unknown dither"))?,
        colors: colors as usize,
        amount: (number("amount", 75.)?.clamp(0., 100.) as f32) / 100.,
        transparency: boolean("transparency", true)?,
        canonical: boolean("canonicalIndexed", false)?,
    })
}
fn decimal(value: &Value, nonzero: bool) -> Result<u64> {
    let text = value
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= 20 && text.bytes().all(|b| b.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0')))
        .ok_or_else(|| bad(CMD, "captured IDs must be canonical decimal strings"))?;
    let number = text.parse().map_err(|_| bad(CMD, "captured ID exceeds range"))?;
    if nonzero && number == 0 {
        return Err(bad(CMD, "documentId must be nonzero"));
    }
    Ok(number)
}
/// Cheap envelope before floating normalization/params cloning. Ordinary legacy
/// no-pair requests may auto-anchor; persisted imported attachments never do.
pub(super) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    let config = settings(params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let captured = match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => false,
        (Some(doc), Some(rev)) if decimal(doc, true)? == state.doc.id.0 && decimal(rev, false)? == state.revision => true,
        _ => return Err(bad(CMD, "captured target changed or incomplete")),
    };
    if state.doc.xcf_state.is_some() || matches!(state.doc.depth, SampleType::U32 | SampleType::F64) || state.doc.accounted().is_some() {
        return Err(bad(CMD, "imported/current/wide indexed conversion is not qualified"));
    }
    if config.canonical && state.doc.mode != ColorMode::Rgb {
        return Err(bad(CMD, "canonicalIndexed requires ordinary RGB input; other profile policies are not qualified"));
    }
    if state.revision == u64::MAX
        || (captured && state.floating.is_some())
        || state.coalesce.is_some()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.command_has_job_on(state.doc.id)
    {
        return Err(bad(CMD, "finish transient edits or pending work before indexed conversion"));
    }
    source_envelope(&state.doc, &Interrupt::NONE)?;
    Ok(())
}
fn source_envelope(doc: &Document, control: &Interrupt<'_>) -> Result<()> {
    let mut count = 0usize;
    photocraft_doc::resident_structure::visit_layers_with(doc, control, |layer| {
        count = count.checked_add(1).filter(|n| *n <= 10000).ok_or(RasterInputError::LimitExceeded)?;
        if layer.indexed_source.is_some() || layer.video.is_some() || matches!(layer.content, LayerContent::BackedRaster(_)) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let smart_mask = match &layer.content {
            LayerContent::Smart(smart) => smart.filter_mask.as_ref().map(|mask| mask.surface.as_ref()),
            _ => None,
        };
        for surface in
            [layer.surface(), layer.fill_cache.as_ref().map(|cache| &cache.surface), layer.mask.as_ref().map(|mask| mask.surface.as_ref()), smart_mask]
                .into_iter()
                .flatten()
        {
            if matches!(surface.format().sample, SampleType::U32 | SampleType::F64) {
                return Err(RasterInputError::UnsupportedConversion);
            }
        }
        Ok(true)
    })
    .map_err(|e| error(control, e))?;
    if doc.channels.len() > 10000
        || doc.paths.len() > 10000
        || doc.layer_comps.len() > 100000
        || doc.slices.list.len() > 100000
        || doc.variables.defs.len() > 100000
    {
        return Err(bad(CMD, "source metadata count limit"));
    }
    for channel in &doc.channels {
        poll(control)?;
        let format = channel.surface.format();
        // Ordinary Gray F64 auxiliary channels use the native f64-to-U8 converter
        // below. Source/current documents and wide layer pixels remain excluded.
        if format.sample == SampleType::U32 || format.sample == SampleType::F64 && format.mode != ColorMode::Grayscale {
            return Err(bad(CMD, "wide saved channel conversion is not qualified"));
        }
    }
    // Flattening must not retain current references to discarded layer identities.
    if !doc.variables.defs.is_empty() {
        return Err(bad(CMD, "current layer variable associations are unsupported for flattening"));
    }
    for comp in doc.layer_comps.iter().chain(doc.last_document_state.iter()) {
        poll(control)?;
        if !comp.states.is_empty() {
            return Err(bad(CMD, "current layer comp associations are unsupported for flattening"));
        }
    }
    for slice in &doc.slices.list {
        poll(control)?;
        if slice.layer.is_some() {
            return Err(bad(CMD, "current layer slice associations are unsupported for flattening"));
        }
    }
    poll(control)
}
fn count(doc: &Document, canonical: bool) -> Result<usize> {
    let w = doc.size.width;
    let h = doc.size.height;
    if w == 0 || h == 0 || w > i32::MAX as u32 || h > i32::MAX as u32 || canonical && (w > 262144 || h > 262144) {
        return Err(bad(CMD, "indexed canvas extent"));
    }
    let n = (w as u64).checked_mul(h as u64).filter(|n| !canonical || *n <= 64 << 20).ok_or_else(|| bad(CMD, "indexed plane limit"))?;
    usize::try_from(n).map_err(|_| bad(CMD, "indexed pixel count"))
}
fn admit(doc: &Document, config: Settings, control: &Interrupt<'_>) -> Result<u64> {
    poll(control)?;
    let pixels = count(doc, config.canonical)?;
    if doc.xcf_state.is_some() || matches!(doc.depth, SampleType::U32 | SampleType::F64) {
        return Err(bad(CMD, "imported/current/wide indexed conversion is not qualified"));
    }
    source_envelope(doc, control)?;
    let mut layer_count = 0u64;
    photocraft_doc::resident_structure::visit_layers_with(doc, control, |layer| {
        layer_count = layer_count.checked_add(1).filter(|n| *n <= 10000).ok_or(RasterInputError::LimitExceeded)?;
        if layer.indexed_source.is_some() || matches!(layer.content, LayerContent::BackedRaster(_)) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        for source in [layer.surface(), layer.fill_cache.as_ref().map(|cache| &cache.surface)].into_iter().flatten() {
            if matches!(source.format().sample, SampleType::U32 | SampleType::F64) {
                return Err(RasterInputError::UnsupportedConversion);
            }
        }
        Ok(true)
    })
    .map_err(|e| error(control, e))?;
    for channel in &doc.channels {
        poll(control)?;
        let format = channel.surface.format();
        // Ordinary Gray F64 auxiliary channels use the native f64-to-U8 converter
        // below. Source/current documents and wide layer pixels remain excluded.
        if format.sample == SampleType::U32 || format.sample == SampleType::F64 && format.mode != ColorMode::Grayscale {
            return Err(bad(CMD, "wide saved channel conversion is not qualified"));
        }
    }
    let preparation = photocraft_io::NativeSaveLimits::default().preparation;
    // Runtime native bytes include each saved channel's actual sample width.
    // Gray-U8 output is no larger than admitted Gray-F64 input; retained/clone
    // envelopes below cover both planes, and work prices all physical input bytes.
    let payload = photocraft_io::retained_native_payload_bytes(doc, preparation, control).map_err(|e| error(control, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, preparation, control).map_err(|e| error(control, e))? as u64;
    let raster = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let palette = quantize::build_palette_requirements(pixels, config.kind, config.colors, config.forced).map_err(|e| error(control, e))?;
    let mapping = quantize::quantize_requirements(pixels, doc.size.width as usize, 256, config.dither).map_err(|e| error(control, e))?;
    let tiles = (doc.size.width as u64)
        .div_ceil(TILE_SIZE as u64)
        .checked_mul((doc.size.height as u64).div_ceil(TILE_SIZE as u64))
        .ok_or_else(|| bad(CMD, "indexed output tiles"))?;
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(4)).ok_or_else(|| bad(CMD, "indexed tile bytes"))?;
    let output =
        tiles.checked_mul(tile_bytes.checked_add(1024).ok_or_else(|| bad(CMD, "indexed output map"))?).ok_or_else(|| bad(CMD, "indexed output storage"))?;
    let band = (doc.size.width as u64)
        .checked_mul((doc.size.height as u64).min(TILE_SIZE as u64))
        .and_then(|n| n.checked_mul(16))
        .ok_or_else(|| bad(CMD, "compositor band scratch"))?;
    let requirements = palette
        .retained_bytes
        .checked_add(palette.scratch_bytes)
        .and_then(|n| n.checked_add(palette.output_bytes))
        .and_then(|n| n.checked_add(mapping.retained_bytes))
        .and_then(|n| n.checked_add(mapping.scratch_bytes))
        .and_then(|n| n.checked_add(mapping.output_bytes))
        .ok_or_else(|| bad(CMD, "quantizer requirements overflow"))?;
    let resident = model
        .checked_mul(5)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add((pixels as u64).checked_mul(24)?))
        .and_then(|n| n.checked_add(requirements))
        .and_then(|n| n.checked_add(output.checked_mul(3)?))
        .and_then(|n| n.checked_add(band))
        .and_then(|n| n.checked_add(16 << 20))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad(CMD, "aggregate composite/palette/planes/model/output admission limit"))?;
    let sync = crate::xcf_editing::direct_indexed_sync_work(pixels, usize::try_from(tiles).map_err(|_| bad(CMD, "indexed tiles"))?)?;
    let work = palette
        .work_units
        .checked_add(mapping.work_units)
        .and_then(|n| n.checked_add((pixels as u64).checked_mul(layer_count.max(1))?.checked_mul(64)?))
        .and_then(|n| n.checked_add(sync.checked_mul(2)?))
        .and_then(|n| n.checked_add(payload.checked_add(model)?.checked_mul(4)?))
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(8)?))
        .filter(|n| *n <= quantize::ControlledLimits::default().work_units)
        .ok_or_else(|| bad(CMD, "aggregate quantization work limit"))?;
    let _ = (resident, work);
    poll(control)?;
    Ok(MAX_CHECKED_RASTER_BYTES)
}
fn composite(doc: &Document, opaque: bool, control: &Interrupt<'_>) -> Result<Vec<[f32; 4]>> {
    let pixels = count(doc, false)?;
    poll(control)?;
    let mut output = Vec::new();
    output.try_reserve_exact(pixels).map_err(|_| bad(CMD, "composite allocation"))?;
    if output.capacity().checked_mul(16).is_none_or(|n| n as u64 > MAX_CHECKED_RASTER_BYTES) {
        return Err(bad(CMD, "composite capacity"));
    }
    photocraft_compose::render_bands_with_interrupt(doc, doc.bounds(), TILE_SIZE, control, |mut band| {
        for (index, pixel) in band.px.iter_mut().enumerate() {
            if index.is_multiple_of(4096) {
                control.check().map_err(|_| RasterInputError::Cancelled)?;
            }
            if pixel.iter().any(|n| !n.is_finite()) {
                return Err(RasterInputError::InvalidSample);
            }
            if opaque {
                let a = pixel[3];
                for color in pixel.get_mut(..3).ok_or(RasterInputError::LengthMismatch)? {
                    *color = *color * a + 1.0 * (1.0 - a);
                }
                pixel[3] = 1.;
            }
        }
        if output.len().checked_add(band.px.len()).is_none_or(|n| n > pixels) {
            return Err(RasterInputError::LengthMismatch);
        }
        output.extend_from_slice(&band.px);
        control.check().map_err(|_| RasterInputError::Cancelled)
    })
    .map_err(|e| error(control, e))?;
    if output.len() != pixels {
        return Err(bad(CMD, "composite extent length mismatch"));
    }
    poll(control)?;
    Ok(output)
}

fn raw_legacy_surface(doc: &Document, px: &[[f32; 4]], control: &Interrupt<'_>) -> Result<Surface> {
    let length = px.len().checked_mul(4).ok_or_else(|| bad(CMD, "legacy carrier byte count"))?;
    let mut bytes = Vec::new();
    poll(control)?;
    bytes.try_reserve_exact(length).map_err(|_| bad(CMD, "legacy carrier allocation"))?;
    if bytes.capacity() as u64 > MAX_CHECKED_RASTER_BYTES {
        return Err(bad(CMD, "legacy carrier capacity"));
    }
    for (index, pixel) in px.iter().enumerate() {
        if index.is_multiple_of(4096) {
            poll(control)?;
        }
        let mut raw = [0u8; 4];
        let mut normalized = [0f32; 8];
        let channels = photocraft_raster::from_rgba_into(&PixelFormat::RGBA8, *pixel, &mut normalized);
        if channels != 4 {
            return Err(bad(CMD, "legacy Indexed carrier channels"));
        }
        photocraft_raster::encode_pixel(&PixelFormat::RGBA8, normalized.get(..channels).ok_or_else(|| bad(CMD, "carrier conversion length"))?, &mut raw);
        bytes.extend_from_slice(&raw);
    }
    let mut surface = Surface::try_from_interleaved_with(PixelFormat::RGBA8, doc.bounds(), &bytes, control).map_err(|e| error(control, e))?;
    surface.try_prune_with(control).map_err(|e| error(control, e))?;
    poll(control)?;
    Ok(surface)
}
fn canonical_plane(doc: &Document, indices: Vec<u8>, px: &[[f32; 4]], palette: &[[u8; 3]], control: &Interrupt<'_>) -> Result<EditableIndexedRaster> {
    if indices.len() != px.len() {
        return Err(bad(CMD, "quantizer ordinal length"));
    }
    let mut alpha = Vec::new();
    poll(control)?;
    alpha.try_reserve_exact(px.len()).map_err(|_| bad(CMD, "native alpha allocation"))?;
    if alpha.capacity() as u64 > MAX_CHECKED_RASTER_BYTES {
        return Err(bad(CMD, "native alpha capacity"));
    }
    for (index, pixel) in px.iter().enumerate() {
        if index.is_multiple_of(4096) {
            poll(control)?;
        }
        let mut raw = [0u8; 1];
        photocraft_color::write_sample(&mut raw, SampleType::U8, 0, pixel[3]);
        alpha.push(raw[0]);
    }
    let mut colors = Vec::new();
    colors.try_reserve_exact(palette.len()).map_err(|_| bad(CMD, "canonical palette allocation"))?;
    if colors.capacity() > 256 {
        return Err(bad(CMD, "canonical palette capacity"));
    }
    colors.extend_from_slice(palette);
    poll(control)?;
    EditableIndexedRaster::try_new_with_interrupt(doc.bounds(), Arc::new(indices), Some(Arc::new(alpha)), colors, control).map_err(|e| error(control, e))
}
fn canonical_state(
    doc: &Document,
    owner: photocraft_doc::LayerId,
    raster: EditableIndexedRaster,
    control: &Interrupt<'_>,
) -> Result<photocraft_doc::xcf_state::XcfState> {
    poll(control)?;
    let mut state = photocraft_doc::xcf_state::XcfState::default();
    // Source-free native current metadata, not an imported schema/provenance epoch.
    // Reconcile's existing bounded metadata routine has boundary cancellation here.
    state.reconcile_layers(doc).map_err(|e| error(control, e))?;
    poll(control)?;
    let item = state
        .items
        .iter_mut()
        .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(owner))
        .ok_or_else(|| bad(CMD, "new Indexed owner metadata"))?;
    item.raster_bounds = Some(raster.bounds());
    state.editable_indexed.try_reserve_exact(1).map_err(|_| bad(CMD, "canonical indexed state allocation"))?;
    state.editable_indexed.push(EditableIndexedLayer { owner, raster });
    poll(control)?;
    Ok(state)
}
pub(super) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    dispatch_preflight(session, params, session.command_is_background())?;
    let config = settings(params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let weak = baseline.downgrade();
    let work_base = baseline.clone();
    crate::jobs::run(
        session,
        "Indexed Color",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Indexed Color", |control| {
                let limit = admit(&work_base, config, control)?;
                let limits = quantize::ControlledLimits { resident_bytes: limit, work_units: quantize::ControlledLimits::default().work_units };
                let mut pixels = composite(&work_base, !config.transparency, control)?;
                let mut clear = false;
                for chunk in pixels.chunks(4096) {
                    poll(control)?;
                    if config.transparency && chunk.iter().any(|pixel| pixel[3] < 0.5) {
                        clear = true;
                    }
                }
                let colors = if clear { config.colors.saturating_sub(1).max(2) } else { config.colors };
                let mut palette =
                    quantize::build_palette_with_interrupt(&pixels, config.kind, colors, config.forced, limits, control).map_err(|e| error(control, e))?;
                let transparent = if clear && palette.len() < 256 {
                    palette.try_reserve_exact(1).map_err(|_| bad(CMD, "transparent palette slot allocation"))?;
                    palette.push([255, 255, 255]);
                    Some(palette.len() - 1)
                } else {
                    None
                };
                if palette.is_empty() || palette.len() > 256 || palette.capacity() > 512 {
                    return Err(bad(CMD, "quantizer palette length/capacity"));
                }
                let indices = quantize::quantize_with_interrupt(
                    &mut pixels,
                    work_base.size.width as usize,
                    &palette,
                    config.dither,
                    config.amount,
                    transparent,
                    limits,
                    control,
                )
                .map_err(|e| error(control, e))?;
                let current = if config.canonical {
                    Some(canonical_plane(&work_base, indices, &pixels, &palette, control)?)
                } else {
                    drop(indices);
                    None
                };
                let carrier = match &current {
                    Some(raster) => crate::xcf_editing::expanded_with(raster, PixelFormat::RGBA8, transparent.map(|slot| slot as u8), control)?,
                    None => raw_legacy_surface(&work_base, &pixels, control)?,
                };
                drop(pixels);
                poll(control)?;
                let mut doc = (*work_base).clone();
                doc.mode = ColorMode::Indexed;
                doc.depth = SampleType::U8;
                let mut layer = Layer::new(if transparent.is_some() { "Index" } else { "Background" }, LayerContent::Raster(carrier));
                if transparent.is_none() {
                    layer.locks.transparency = true;
                    layer.locks.position = true;
                }
                let owner = layer.id;
                let mut layers = Vec::new();
                layers.try_reserve_exact(1).map_err(|_| bad(CMD, "indexed layer allocation"))?;
                layers.push(layer);
                doc.layers = layers;
                for channel in &mut doc.channels {
                    poll(control)?;
                    let format = PixelFormat { alpha: channel.surface.format().alpha, ..PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false) };
                    channel.surface = channel.surface.try_convert_with(format, control).map_err(|e| error(control, e))?;
                }
                doc.quick_mask = None;
                doc.duotone = None;
                let mut table = Vec::new();
                table.try_reserve_exact(palette.len()).map_err(|_| bad(CMD, "color table allocation"))?;
                if table.capacity() > 256 {
                    return Err(bad(CMD, "color table capacity"));
                }
                table.extend_from_slice(&palette);
                doc.color_table = Some(ColorTable { colors: table, transparent: transparent.map(|slot| slot as u8) });
                if let Some(raster) = current {
                    doc.xcf_state = Some(canonical_state(&doc, owner, raster, control)?);
                    crate::xcf_indexed_geometry::validate_current_with_interrupt(&doc, control)?;
                }
                poll(control)?;
                Ok((doc, owner, palette.len(), transparent, ctx.clone()))
            })
        },
        move |session, (doc, owner, colors, transparent, ctx)| {
            let state = session.active().ok_or(EngineError::NoDocument)?;
            if state.doc.id != baseline.id
                || state.revision != revision
                || !weak.is_current(&state.doc)
                || !state.doc.shares_state(&baseline)
                || state.floating.is_some()
                || state.coalesce.is_some()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
            {
                return Err(bad(CMD, "target changed while preparing Indexed Color"));
            }
            ctx.stage(0.95, 1.0, "Indexed Color", |control| {
                poll(control)?;
                session.edit_with_interrupt("Indexed Color", control, move |current, active| {
                    poll(control)?;
                    *current = doc;
                    *active = Some(owner);
                    Ok(())
                })?;
                Ok(json!({"colors":colors,"transparentIndex":transparent,"canonicalIndexed":config.canonical}))
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::Size;
    fn fixture() -> Session {
        let mut doc = Document::new("quantize", Size::new(3, 1), ColorMode::Rgb, SampleType::U8);
        let pixels = [255, 0, 0, 255, 0, 255, 0, 128, 7, 8, 9, 0];
        let surface = Surface::try_from_interleaved_with(PixelFormat::RGBA8, doc.bounds(), &pixels, &Interrupt::NONE).unwrap();
        doc.layers.push(Layer::new("source", LayerContent::Raster(surface)));
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
    }
    #[test]
    fn default_legacy_carrier_is_exact_and_canonical_opt_in_retains_returned_ordinals() {
        let params = json!({"palette":"exact","forced":"none","dither":"none","transparency":true});
        let mut legacy = fixture();
        let original = legacy.active().unwrap().doc.clone();
        let mut expected = super::composite(&original, false, &Interrupt::NONE).unwrap();
        let mut palette = quantize::build_palette(&expected, PaletteKind::Exact, 255, Forced::None).unwrap();
        palette.push([255, 255, 255]);
        let transparent = palette.len() - 1;
        let ordinals = quantize::quantize(&mut expected, 3, &palette, Dither::None, 0.75, Some(transparent));
        let expected_surface = raw_legacy_surface(&original, &expected, &Interrupt::NONE).unwrap();
        run(&mut legacy, &params).unwrap();
        let legacy_doc = &legacy.active().unwrap().doc;
        assert_eq!(legacy_doc.layers[0].surface(), Some(&expected_surface));
        assert!(legacy_doc.xcf_state.is_none());
        assert_eq!(legacy_doc.color_table.as_ref().unwrap().colors, palette);
        let mut canonical = fixture();
        let mut opt = params;
        opt["canonicalIndexed"] = Value::Bool(true);
        run(&mut canonical, &opt).unwrap();
        let current = &canonical.active().unwrap().doc;
        let raster = &current.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
        assert_eq!(raster.indices().as_slice(), ordinals);
        assert_eq!(raster.palette(), palette);
        assert_eq!(raster.alpha().unwrap().as_slice(), &[255, 255, 0]);
        assert_eq!(current.color_table.as_ref().unwrap().transparent, Some(transparent as u8));
        assert_eq!(current.layers[0].surface().unwrap().to_interleaved(photocraft_geom::Rect::new(2, 0, 3, 1)), vec![255, 255, 255, 0]);
        assert_eq!(expected_surface.to_interleaved(photocraft_geom::Rect::new(2, 0, 3, 1)), vec![0, 0, 0, 0]);
        crate::xcf_indexed_geometry::validate_current_with_interrupt(current, &Interrupt::NONE).unwrap();
        let saved = photocraft_format::save_to_bytes(current, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened.xcf_state, current.xcf_state);
        assert_eq!(reopened.layers[0].surface(), current.layers[0].surface());
        canonical.execute("edit.undo", Value::Null).unwrap();
        assert!(canonical.active().unwrap().doc.xcf_state.is_none());
        assert_eq!(canonical.active().unwrap().doc.layers[0].surface(), original.layers[0].surface());
        canonical.execute("edit.redo", Value::Null).unwrap();
        assert_eq!(canonical.active().unwrap().doc.xcf_state, reopened.xcf_state);
    }
    #[test]
    fn canonical_representation_preserves_duplicate_palette_order_and_zero_alpha_policy() {
        let session = fixture();
        let doc = &session.active().unwrap().doc;
        let palette = [[8, 9, 10], [8, 9, 10], [255, 255, 255]];
        let pixels = [[0., 0., 0., 0.], [8. / 255., 9. / 255., 10. / 255., 0.25], [1., 1., 1., 1.]];
        let raster = canonical_plane(doc, vec![1, 0, 2], &pixels, &palette, &Interrupt::NONE).unwrap();
        assert_eq!(raster.indices().as_slice(), &[1, 0, 2]);
        assert_eq!(raster.palette(), palette);
        assert_eq!(raster.alpha().unwrap().as_slice(), &[0, 64, 255]);
        let carrier = crate::xcf_editing::expanded_with(&raster, PixelFormat::RGBA8, None, &Interrupt::NONE).unwrap();
        assert_eq!(carrier.to_interleaved(doc.bounds()), vec![8, 9, 10, 0, 8, 9, 10, 64, 255, 255, 255, 255]);
    }
    #[test]
    fn malformed_stale_source_or_cancel_preserves_original_and_real_redo() {
        let mut session = fixture();
        session
            .edit("seed", |doc, _| {
                doc.name = "seed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", Value::Null).unwrap();
        let state = session.active().unwrap();
        let before = state.doc.clone();
        let revision = state.revision;
        let redo = state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
        for params in [
            json!({"amount":false}),
            json!({"palette":"unknown"}),
            json!({"transparency":0}),
            json!({"canonicalIndexed":"true"}),
            json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()}),
            json!({"documentId":before.id.0.to_string()}),
        ] {
            assert!(run(&mut session, &params).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().revision, revision);
            assert_eq!(session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
        }
        assert!(matches!(admit(&before, settings(&Value::Null).unwrap(), &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
}
