//! Controlled ordinary Merge Spot Channel. Imported/source-defined composition
//! and wide numerics are not admitted by this legacy f32 ink equation.
use crate::{EngineError, Result, Session};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, item_identity::ChannelId};
use photocraft_geom::TILE_SIZE;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError};
use serde_json::Value;
const CMD: &str = "channel.mergeSpot";
const LABEL: &str = "Merge Spot Channel";
fn bad(message: impl std::fmt::Display) -> EngineError {
    super::bad(CMD, message.to_string())
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn raster(error: RasterInputError) -> EngineError {
    if error == RasterInputError::Cancelled { EngineError::Cancelled } else { bad(error) }
}
fn io(error: photocraft_io::IoError) -> EngineError {
    if matches!(error, photocraft_io::IoError::Cancelled) { EngineError::Cancelled } else { bad(error) }
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || (text.len() > 1 && text.starts_with('0')) || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
fn request(session: &Session, params: &Value) -> Result<(usize, ChannelId)> {
    if !params.is_null()
        && !params
            .as_object()
            .is_some_and(|o| o.len() <= 4 && o.keys().all(|k| matches!(k.as_str(), "channel" | "channelId" | "documentId" | "expectedRevision")))
    {
        return Err(bad("expected bounded Merge Spot Channel parameters"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.channels.len() > 10_000 || state.channel_view.alpha_visible.len() > 10_000 || state.channel_view.color_hidden.len() > 16 {
        return Err(bad("channel/view count limit"));
    }
    if params.get("channel").is_some() && params.get("channelId").is_some() {
        return Err(bad("supply one channel selector"));
    }
    if let Some(value) = params.get("channel") {
        let valid = match value {
            Value::String(s) => s.len() <= 4096 && !s.contains('\0'),
            Value::Number(n) => n.as_u64().is_some(),
            Value::Object(o) => {
                o.len() == 1
                    && o.iter().all(|(k, v)| match k.as_str() {
                        "alpha" => v.as_u64().is_some(),
                        "channelId" => super::channel_identity(v).is_some(),
                        _ => false,
                    })
            }
            _ => false,
        };
        if !valid {
            return Err(bad("invalid bounded channel selector"));
        }
    }
    if params.get("channelId").is_some_and(|v| super::channel_identity(v).is_none()) {
        return Err(bad("invalid stable channelId"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if decimal(id) == Some(state.doc.id.0) && state.doc.id.0 != 0 && decimal(rev) == Some(state.revision) => {}
        _ => return Err(bad("captured document/revision changed or incomplete")),
    }
    let index = if super::parameter_ref(params, "channel").is_some() {
        super::alpha_index(&state.doc, params, "channel", CMD)?
    } else if let super::ChannelTarget::Alpha(i) = state.channel_view.target {
        i
    } else {
        state.doc.channels.iter().position(|c| c.spot.is_some()).ok_or_else(|| bad("no spot channel"))?
    };
    let channel = state.doc.channels.get(index).ok_or_else(|| bad("channel index out of range"))?;
    if channel.id.0 == 0 || state.doc.channels.iter().filter(|c| c.id == channel.id).count() != 1 {
        return Err(bad("zero/ambiguous channel identity"));
    }
    let (ink, solidity) = channel.spot.ok_or_else(|| bad("not a spot channel"))?;
    if !matches!(ink.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !ink.c.iter().all(|v| v.is_finite())
        || !ink.alpha.is_finite()
        || !solidity.is_finite()
        || !(0.0..=1.0).contains(&solidity)
    {
        return Err(bad("finite RGB/Gray spot ink and unit solidity required"));
    }
    Ok((index, channel.id))
}
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    request(session, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.source_read_only
        || state.revision >= u64::MAX - 1
        || session.documents().iter().any(|state| state.floating.is_some())
        || state.coalesce.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || state.doc.accounted().is_some()
        || session.accounted_slots.is_some()
    {
        return Err(bad("idle uncoalesced resident merge context required"));
    }
    if session.smart_links.iter().any(|l| l.parent == state.doc.id || l.child == state.doc.id) {
        return Err(bad("smart contents merge unsupported"));
    }
    domain(&state.doc)
}
fn domain(document: &Document) -> Result<()> {
    if document.xcf_state.is_some()
        || document.source_color_context.is_some()
        || document.color_table.is_some()
        || !document.patterns.is_empty()
        || !matches!(document.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(document.depth, SampleType::U8 | SampleType::U16 | SampleType::F32)
    {
        return Err(bad("Merge Spot Channel supports ordinary source-free RGB/Gray U8/U16/F32 without pattern preparation only"));
    }
    Ok(())
}
fn admission(document: &Document, index: usize, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    domain(document)?;
    super::source_dimensions(document, CMD)?;
    let format = document.pixel_format();
    let channel = document.channels.get(index).ok_or_else(|| bad("missing channel"))?;
    if channel.surface.format() != PixelFormat::new(ColorMode::Grayscale, document.depth, false) || document.selection_channel_id == Some(channel.id) {
        return Err(bad("plain same-depth saved Gray spot channel required"));
    }
    let mut layers = 0u64;
    let mut source_tiles = channel.surface.tile_count() as u64;
    photocraft_doc::resident_structure::visit_layers_with(document, control, |layer| {
        layers = layers.checked_add(1).ok_or(RasterInputError::LimitExceeded)?;
        if layer.locks.all
            || layer.locks.pixels
            || !layer.opacity.is_finite()
            || !layer.fill_opacity.is_finite()
            || layer.mask.is_some()
            || layer.vector_mask.is_some()
            || layer.indexed_source.is_some()
            || layer.import_compositing.is_some()
            || layer.video.is_some()
            || layer.fill_cache.is_some()
            || !layer.psd_blocks.is_empty()
            || layer.effects != Default::default()
            || layer.advanced != Default::default()
            || layer.blend_if != Default::default()
            || layer.excluded_channels != 0
        {
            return Err(RasterInputError::UnsupportedConversion);
        }
        match &layer.content {
            LayerContent::Raster(surface) if surface.format() == format => {
                surface.try_validate_tile_origins_with(control)?;
                source_tiles = source_tiles.checked_add(surface.tile_count() as u64).ok_or(RasterInputError::LimitExceeded)?;
            }
            LayerContent::Group(_) => {}
            _ => return Err(RasterInputError::UnsupportedConversion),
        }
        Ok(true)
    })
    .map_err(raster)?;
    channel.surface.try_validate_tile_origins_with(control).map_err(raster)?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(document, limits, control).map_err(io)? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(document, limits, control).map_err(io)? as u64;
    let held = crate::DocumentSnapshot::resident_raster_footprint_of(document, control)?;
    let tiles = u64::from(document.size.width)
        .div_ceil(TILE_SIZE as u64)
        .checked_mul(u64::from(document.size.height).div_ceil(TILE_SIZE as u64))
        .ok_or_else(|| bad("output tile count"))?;
    let tile_bytes = (TILE_SIZE as u64) * (TILE_SIZE as u64) * (format.bytes_per_pixel() as u64);
    let replacement_row = u64::from(document.size.width).div_ceil(TILE_SIZE as u64).checked_mul(tile_bytes).ok_or_else(|| bad("staged row tile residency"))?;
    let row = u64::from(document.size.width).checked_mul(64).ok_or_else(|| bad("row scratch size"))?;
    let peak = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(held.retained_bytes))
        .and_then(|n| n.checked_add(held.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes.checked_add(2048)?)?))
        .and_then(|n| n.checked_add(row))
        .and_then(|n| n.checked_add(replacement_row))
        .and_then(|n| n.checked_add(1 << 20))
        .ok_or_else(|| bad("merge residency overflow"))?;
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(bad("aggregate merge model/raster/output residency limit"));
    }
    // Row writes revisit growing sparse maps. This includes repeated scheduler/map visits.
    document
        .bounds()
        .size()
        .area()
        .checked_mul(layers.checked_add(1).and_then(|n| n.checked_mul(256)).ok_or_else(|| bad("merge work"))?)
        .and_then(|n| n.checked_add(u64::from(document.size.height).checked_mul(source_tiles.checked_add(tiles)?)?.checked_mul(256)?))
        .and_then(|n| n.checked_add(model.checked_add(payload)?))
        .filter(|n| *n <= 8_000_000_000)
        .ok_or_else(|| bad("merge work limit"))?;
    poll(control)
}
fn prepare(document: &Document, index: usize, control: &Interrupt<'_>) -> Result<(Document, photocraft_doc::LayerId)> {
    admission(document, index, control)?;
    let format = document.pixel_format();
    let channel = document.channels.get(index).ok_or_else(|| bad("missing spot channel"))?;
    let (ink, solidity) = channel.spot.ok_or_else(|| bad("missing spot ink"))?;
    let ink = ink.to_rgb();
    let mut background = Layer::raster("Background", format);
    background.locks.transparency = true;
    background.locks.position = true;
    let surface = background.surface_mut().ok_or_else(|| bad("missing prepared background surface"))?;
    let mut failed = None;
    let rendered = photocraft_compose::render_bands_with_interrupt(document, document.bounds(), 1, control, |band| {
        let result = (|| -> Result<()> {
            poll(control)?;
            let plane = super::read_plane_with_interrupt(&channel.surface, band.rect, control)?;
            if plane.len() != band.px.len() {
                return Err(bad("spot/composite band shape mismatch"));
            }
            let length = photocraft_raster::checked_region_byte_len(band.rect, format).map_err(raster)?;
            let mut encoded = Vec::new();
            encoded.try_reserve_exact(length).map_err(|_| bad("merge row allocation failed"))?;
            encoded.resize(length, 0);
            for (i, (c, &v)) in band.px.iter().zip(&plane).enumerate() {
                if i % 256 == 0 {
                    poll(control)?;
                }
                if !c.iter().all(|v| v.is_finite()) || !v.is_finite() {
                    return Err(bad("nonfinite merge samples"));
                }
                let rgb: [f32; 3] = std::array::from_fn(|j| {
                    let over = solidity * ink[j] + (1.0 - solidity) * c[j] * ink[j];
                    c[j] + (over - c[j]) * v.clamp(0.0, 1.0)
                });
                if !rgb.iter().all(|v| v.is_finite()) {
                    return Err(bad("nonfinite ink composite"));
                }
                let mut native = [0.; 4];
                let count = photocraft_raster::from_rgba_into(&format, [rgb[0], rgb[1], rgb[2], 1.], &mut native);
                let offset = i.checked_mul(format.bytes_per_pixel()).ok_or_else(|| bad("row offset"))?;
                let end = offset.checked_add(format.bytes_per_pixel()).ok_or_else(|| bad("row end"))?;
                let pixel = encoded.get_mut(offset..end).ok_or_else(|| bad("row shape"))?;
                for (j, &value) in native.get(..count).ok_or_else(|| bad("native channel count"))?.iter().enumerate() {
                    photocraft_color::write_sample(pixel, format.sample, j, value);
                }
            }
            surface.try_write_interleaved_with(band.rect, &encoded, control).map_err(raster)?;
            poll(control)
        })();
        result.map_err(|error| {
            let cancelled = matches!(error, EngineError::Cancelled);
            failed = Some(error);
            if cancelled { RasterInputError::Cancelled } else { RasterInputError::UnsupportedConversion }
        })
    });
    if let Some(error) = failed {
        return Err(error);
    }
    rendered.map_err(raster)?;
    surface.try_prune_with(control).map_err(raster)?;
    poll(control)?;
    let id = background.id;
    let mut layers = Vec::new();
    layers.try_reserve_exact(1).map_err(|_| bad("background layer allocation failed"))?;
    layers.push(background);
    let mut candidate = document.clone();
    candidate.layers = layers;
    candidate.channels.remove(index);
    poll(control)?;
    Ok((candidate, id))
}
pub(crate) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    let (index, id) = request(session, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let before = state.doc.clone();
    let revision = state.revision;
    let active = state.active_layer;
    let copy = |values: &[bool]| -> Result<Vec<bool>> {
        let mut out = Vec::new();
        out.try_reserve_exact(values.len()).map_err(|_| bad("view witness allocation failed"))?;
        out.extend_from_slice(values);
        Ok(out)
    };
    let view = super::ChannelView {
        target: state.channel_view.target,
        color_hidden: copy(&state.channel_view.color_hidden)?,
        alpha_visible: copy(&state.channel_view.alpha_visible)?,
        quick_mask_hidden: state.channel_view.quick_mask_hidden,
        layer_mask: state.channel_view.layer_mask.clone(),
        editing_mask: state.channel_view.editing_mask,
    };
    let baseline = before.clone();
    crate::jobs::run(
        session,
        LABEL,
        true,
        move |ctx| {
            ctx.stage(0., 0.95, LABEL, |ctl| {
                if before.channels.get(index).is_none_or(|channel| channel.id != id) {
                    return Err(bad("stale channel identity"));
                }
                let (candidate, layer) = prepare(&before, index, ctl)?;
                Ok((candidate, layer, ctx.clone()))
            })
        },
        move |session, (candidate, layer, ctx)| {
            let state = session.active().ok_or(EngineError::NoDocument)?;
            if state.doc.id != baseline.id
                || state.revision != revision
                || !state.doc.shares_state(&baseline)
                || state.active_layer != active
                || state.channel_view != view
                || state.source_read_only
                || session.documents().iter().any(|state| state.floating.is_some())
                || state.coalesce.is_some()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
                || session.command_has_pending_job()
            {
                return Err(bad("merge source/target/view changed during preparation"));
            }
            let mut selected = Vec::new();
            selected.try_reserve_exact(1).map_err(|_| bad("layer target allocation failed"))?;
            selected.push(layer);
            ctx.stage(0.95, 1., LABEL, |ctl| {
                poll(ctl)?;
                session.edit_with_layer_target_and_channel_reset_with_interrupt(
                    LABEL,
                    ctl,
                    photocraft_ops::LayerTarget { active: Some(layer), selected },
                    Some(layer),
                    move |document, active| {
                        *document = candidate;
                        *active = Some(layer);
                        Ok(())
                    },
                )?;
                Ok(Value::Null)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::Color;
    use photocraft_doc::{AlphaChannel, Size};
    use photocraft_raster::Surface;
    use serde_json::json;
    fn fixture(sample: SampleType) -> Document {
        let mut doc = Document::new("Spot", Size::new(2, 1), ColorMode::Rgb, sample);
        let mut layer = Layer::raster("Pixels", doc.pixel_format());
        layer.surface_mut().unwrap().write_region(doc.bounds(), &[0.2, 0.4, 0.8, 1., 0.7, 0.3, 0.1, 0.5]);
        doc.layers.push(layer);
        let mut plane = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
        plane.write_region(doc.bounds(), &[0.25, 1.]);
        let mut channel = AlphaChannel::new("Ink", plane);
        channel.spot = Some((Color::rgb(0.8, 0.3, 0.6), 0.4));
        doc.channels.push(channel);
        doc
    }
    // Genuine old compositor/plane/formula oracle; authored but not executed.
    #[test]
    fn banded_output_matches_legacy_ink_and_native_quantization() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let doc = fixture(sample);
            let comp = photocraft_compose::render(&doc, doc.bounds()).unwrap();
            let k = super::super::read_plane(&doc.channels[0].surface, doc.bounds());
            let (ink, solidity) = doc.channels[0].spot.unwrap();
            let ink = ink.to_rgb();
            let values: Vec<f32> = comp
                .px
                .iter()
                .zip(k)
                .flat_map(|(c, v)| {
                    let rgb: [f32; 3] = std::array::from_fn(|j| {
                        let over = solidity * ink[j] + (1. - solidity) * c[j] * ink[j];
                        c[j] + (over - c[j]) * v.clamp(0., 1.)
                    });
                    photocraft_raster::from_rgba(&doc.pixel_format(), [rgb[0], rgb[1], rgb[2], 1.])
                })
                .collect();
            let mut expected = Surface::new(doc.pixel_format());
            expected.write_region(doc.bounds(), &values);
            let (candidate, _) = prepare(&doc, 0, &Interrupt::NONE).unwrap();
            assert_eq!(
                candidate.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(),
                expected.try_read_interleaved(doc.bounds()).unwrap()
            );
            assert!(candidate.channels.is_empty());
            assert_eq!(doc.channels.len(), 1);
            assert_eq!(candidate.physical_resolution, doc.physical_resolution);
        }
    }
    #[test]
    fn wide_stale_locked_and_cancel_refuse_without_changing_source() {
        let mut session = Session::new();
        session.add_document(fixture(SampleType::U8), None).unwrap();
        let before = session.active().unwrap().doc.clone();
        assert!(dispatch_preflight(&session, &json!({"documentId":before.id.0.to_string(),"expectedRevision":"999"}), false).is_err());
        assert!(matches!(prepare(&before, 0, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        let mut locked = (*before).clone();
        locked.layers[0].locks.pixels = true;
        assert!(prepare(&locked, 0, &Interrupt::NONE).is_err());
        let mut wide = (*before).clone();
        wide.depth = SampleType::U32;
        assert!(domain(&wide).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
    #[test]
    fn registered_merge_one_undo_redo_and_channel_reset() {
        let mut session = Session::new();
        session.add_document(fixture(SampleType::U16), None).unwrap();
        session.active_mut().unwrap().channel_view.target = super::super::ChannelTarget::Alpha(0);
        let before = session.active().unwrap().doc.clone();
        let params = json!({"channelId":before.channels[0].id.0.to_string(),"documentId":before.id.0.to_string(),"expectedRevision":session.active().unwrap().revision.to_string()});
        session.execute(CMD, params).unwrap();
        let after = session.active().unwrap().doc.clone();
        assert_eq!(session.active().unwrap().channel_view, super::super::ChannelView::default());
        assert!(after.channels.is_empty());
        assert!(session.undo());
        assert_eq!(*session.active().unwrap().doc, *before);
        assert!(session.redo());
        assert_eq!(*session.active().unwrap().doc, *after);
    }
}
