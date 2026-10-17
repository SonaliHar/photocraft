//! Controlled creation of ordinary saved coverage channels. No source admission.
use super::load_selection_controlled::canonical_selection_storage;
use super::{ChannelTarget, bad, channel_format, indicates_param, target, touch};
use crate::{EngineError, Result, Session};
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::item_identity::ChannelId;
use photocraft_doc::{AlphaChannel, ColorIndicates, Document};
use photocraft_geom::TILE_SIZE;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
const NAME_LIMIT: usize = 2048;
fn cmd(spot: bool) -> &'static str {
    if spot { "channel.newSpot" } else { "channel.new" }
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn raster_error(control: &Interrupt<'_>, spot: bool, error: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(cmd(spot), error.to_string()) }
}
#[derive(Clone, Copy)]
enum Fill {
    Black,
    White,
    Selection,
}
#[derive(Clone, Copy)]
struct Settings {
    fill: Fill,
    raw_storage: bool,
    color: Color,
    opacity: Option<f32>,
    solidity: f32,
    indicates: Option<ColorIndicates>,
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse().ok()
}
fn percentage(params: &Value, key: &str, default: f32, spot: bool) -> Result<f32> {
    match params.get(key) {
        None => Ok(default),
        Some(value) => {
            let number = value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(cmd(spot), "percentage must be finite numeric"))?;
            // Ordinary opacity historically casts before dividing; solidity clamps first.
            Ok(if key == "opacity" { (number as f32 / 100.).clamp(0., 1.) } else { number.clamp(0., 100.) as f32 / 100. })
        }
    }
}
fn settings(params: &Value, spot: bool) -> Result<Settings> {
    if !params.is_null() {
        let object = params.as_object().filter(|map| map.len() <= 9).ok_or_else(|| bad(cmd(spot), "expected bounded channel parameters"))?;
        if !object.keys().all(|key| {
            if spot {
                matches!(key.as_str(), "name" | "color" | "solidity" | "fromSelection" | "documentId" | "expectedRevision")
            } else {
                matches!(key.as_str(), "name" | "fill" | "color" | "opacity" | "indicates" | "documentId" | "expectedRevision" | "rawStorage")
            }
        }) {
            return Err(bad(cmd(spot), "unknown channel parameter"));
        }
    }
    if let Some(value) = params.get("name")
        && !value.as_str().is_some_and(|name| name.len() <= NAME_LIMIT && !name.contains('\0'))
    {
        return Err(bad(cmd(spot), "bounded UTF-8 name required"));
    }
    let color = match params.get("color") {
        None => AlphaChannel::DEFAULT_COLOR,
        Some(Value::String(text)) if text.len() <= 9 => {
            let rgb = crate::commands::parse_hex(text).ok_or_else(|| bad(cmd(spot), "invalid color"))?;
            Color::rgb(rgb[0], rgb[1], rgb[2])
        }
        Some(Value::Array(values)) if (3..=4).contains(&values.len()) => {
            let mut rgb = [0f32; 3];
            for (index, value) in values.iter().enumerate() {
                let number = value.as_f64().filter(|n| n.is_finite() && (*n as f32).is_finite()).ok_or_else(|| bad(cmd(spot), "finite color required"))?;
                if let Some(out) = rgb.get_mut(index) {
                    *out = number as f32;
                }
            }
            Color::rgb(rgb[0], rgb[1], rgb[2])
        }
        _ => return Err(bad(cmd(spot), "invalid bounded color")),
    };
    let fill = if spot {
        match params.get("fromSelection") {
            None => Fill::Selection,
            Some(value) => {
                if value.as_bool().ok_or_else(|| bad(cmd(spot), "fromSelection must be boolean"))? {
                    Fill::Selection
                } else {
                    Fill::Black
                }
            }
        }
    } else {
        match params.get("fill") {
            None => Fill::Black,
            Some(Value::String(text)) => match text.as_str() {
                "black" => Fill::Black,
                "white" => Fill::White,
                "selection" => Fill::Selection,
                _ => return Err(bad(cmd(spot), "unknown fill")),
            },
            _ => return Err(bad(cmd(spot), "fill must be string")),
        }
    };
    let indicates = if params.get("indicates").is_some() {
        Some(indicates_param(params.get("indicates")).ok_or_else(|| bad(cmd(spot), "unknown indicates mode"))?)
    } else {
        None
    };
    let raw_storage = match params.get("rawStorage") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(|| bad(cmd(spot), "rawStorage must be boolean"))?,
    };
    if raw_storage && (spot || !matches!(fill, Fill::Selection)) {
        return Err(bad(cmd(spot), "rawStorage requires ordinary channel.new fill selection"));
    }
    Ok(Settings {
        fill,
        raw_storage,
        color,
        opacity: if params.get("opacity").is_some() { Some(percentage(params, "opacity", 0.5, spot)?) } else { None },
        solidity: percentage(params, "solidity", 0., spot)?,
        indicates,
    })
}
pub(super) fn dispatch_preflight(session: &Session, params: &Value, spot: bool) -> Result<()> {
    let config = settings(params, spot)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.channel_view.color_hidden.capacity().checked_add(state.channel_view.alpha_visible.capacity()).is_none_or(|bytes| bytes > 1 << 20) {
        return Err(bad(cmd(spot), "channel view capacity limit"));
    }
    // The document edit and subsequent target/visibility update each advance revision.
    if state.revision.checked_add(2).is_none() {
        return Err(bad(cmd(spot), "channel creation requires two revisions of headroom"));
    }
    if state.doc.channels.len() >= 10000
        || state.doc.accounted().is_some()
        || (state.doc.xcf_state.is_some() && !config.raw_storage)
        || state.doc.mode == ColorMode::Indexed
        || state.doc.color_table.is_some()
        || (state.doc.source_color_context.is_some() && (!config.raw_storage || state.doc.xcf_state.is_none()))
    {
        return Err(bad(cmd(spot), "channel creation requires ordinary current resident context"));
    }
    photocraft_doc::resident_structure::visit_layers_with(&state.doc, &Interrupt::NONE, |layer| {
        if layer.indexed_source.is_some() {
            return Err(photocraft_raster::RasterInputError::UnsupportedConversion);
        }
        Ok(true)
    })
    .map_err(|_| bad(cmd(spot), "indexed source or unsupported layer structure is not qualified"))?;
    if state.floating.is_some()
        || state.coalesce.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
    {
        return Err(bad(cmd(spot), "finish jobs or gestures before channel creation"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if decimal(id) == Some(state.doc.id.0) && state.doc.id.0 != 0 && decimal(revision) == Some(state.revision) => {}
        _ => return Err(bad(cmd(spot), "captured target changed or incomplete")),
    }
    super::source_dimensions(&state.doc, cmd(spot))?;
    if matches!(config.fill, Fill::Selection)
        && let Some(selection) = &state.doc.selection
    {
        let format = selection.format();
        let target = channel_format(&state.doc);
        if format.mode != ColorMode::Grayscale
            || format.alpha
            || (format != target && (matches!(format.sample, SampleType::U32 | SampleType::F64) || matches!(target.sample, SampleType::U32 | SampleType::F64)))
        {
            return Err(bad(cmd(spot), "selection format conversion is not qualified"));
        }
    }
    if config.raw_storage {
        if state.doc.xcf_state.is_some() {
            canonical_selection_storage::prepare_new(&state.doc, &Interrupt::NONE)?;
        }
        let selection = state.doc.selection.as_ref().ok_or_else(|| bad(cmd(spot), "rawStorage requires a stored selection"))?;
        if selection.format() != channel_format(&state.doc) {
            return Err(bad(cmd(spot), "rawStorage requires identical Gray depth without alpha"));
        }
    }
    Ok(())
}
fn admission(doc: &Document, spot: bool, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let (area, pixels) = super::source_dimensions(doc, cmd(spot))?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, control).map_err(|e| raster_error(control, spot, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, control).map_err(|e| raster_error(control, spot, e))? as u64;
    let raster = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let tile_count = (u64::from(doc.size.width))
        .div_ceil(TILE_SIZE as u64)
        .checked_mul(u64::from(doc.size.height).div_ceil(TILE_SIZE as u64))
        .ok_or_else(|| bad(cmd(spot), "tile count overflow"))?;
    let per_tile = (TILE_SIZE as u64)
        .checked_mul(TILE_SIZE as u64)
        .and_then(|n| n.checked_mul(channel_format(doc).bytes_per_pixel() as u64))
        .and_then(|n| n.checked_add(1024))
        .ok_or_else(|| bad(cmd(spot), "tile bytes overflow"))?;
    let retained = payload
        .checked_add(model.checked_mul(5).ok_or_else(|| bad(cmd(spot), "model overflow"))?)
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|n| n.checked_add(tile_count.checked_mul(per_tile)?.checked_mul(3)?))
        .and_then(|n| n.checked_add(per_tile.checked_mul(8)?))
        .and_then(|n| n.checked_add(1 << 20));
    if retained.is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES) {
        return Err(bad(cmd(spot), "channel staging resident limit"));
    }
    let existing = doc.selection.as_ref().map_or(0, |s| s.tile_count() as u64);
    let work = tile_count
        .checked_mul(tile_count.checked_add(existing).and_then(|n| n.checked_add(1)).ok_or_else(|| bad(cmd(spot), "scheduler work"))?)
        .and_then(|n| n.checked_mul(256))
        .and_then(|n| n.checked_add((pixels as u64).checked_mul(64)?))
        .and_then(|n| n.checked_add((doc.channels.len() as u64).checked_add(1)?.checked_pow(2)?.checked_mul(64)?))
        .and_then(|n| n.checked_add(payload.checked_add(model)?.checked_mul(8)?));
    if work.is_none_or(|n| n > 8_000_000_000) {
        return Err(bad(cmd(spot), "channel preparation work limit"));
    }
    let _ = area;
    poll(control)
}
fn name(doc: &Document, explicit: Option<&str>, spot: bool, control: &Interrupt<'_>) -> Result<String> {
    if let Some(text) = explicit {
        let mut out = String::new();
        poll(control)?;
        out.try_reserve_exact(text.len()).map_err(|_| bad(cmd(spot), "name allocation"))?;
        out.push_str(text);
        poll(control)?;
        return Ok(out);
    }
    let base = if spot { "Spot Color" } else { "Alpha" };
    for number in 1..=doc.channels.len() + 1 {
        poll(control)?;
        let candidate = format!("{base} {number}");
        let mut used = false;
        for channel in &doc.channels {
            poll(control)?;
            if channel.name == candidate {
                used = true;
                break;
            }
        }
        if !used {
            return Ok(candidate);
        }
    }
    Err(bad(cmd(spot), "channel name exhausted"))
}
fn fresh(doc: &Document, spot: bool, control: &Interrupt<'_>) -> Result<ChannelId> {
    for _ in 0..doc.channels.len() + 4 {
        poll(control)?;
        let id = ChannelId::fresh();
        if id.0 == 0 {
            continue;
        }
        let mut used = doc.selection_channel_id == Some(id) || doc.quick_mask.as_ref().is_some_and(|c| c.id == id);
        for channel in &doc.channels {
            poll(control)?;
            if channel.id == id {
                used = true;
                break;
            }
        }
        if !used {
            return Ok(id);
        }
    }
    Err(bad(cmd(spot), "channel identities exhausted"))
}
fn surface(doc: &Document, config: Settings, spot: bool, control: &Interrupt<'_>) -> Result<Surface> {
    poll(control)?;
    let format = channel_format(doc);
    let area = doc.bounds();
    let zero = [0u8; 8];
    let native_zero = zero.get(..format.bytes_per_pixel()).ok_or_else(|| bad(cmd(spot), "channel sample width"))?;
    let mut out = Surface::try_with_default_bytes(format, native_zero).map_err(|e| raster_error(control, spot, e))?;
    let selection = if matches!(config.fill, Fill::Selection) { doc.selection.as_ref() } else { None };
    if matches!(config.fill, Fill::Black) || (matches!(config.fill, Fill::Selection) && selection.is_none()) {
        poll(control)?;
        return Ok(out);
    }
    for coordinate in area.tiles() {
        poll(control)?;
        let part = coordinate.rect().intersect(&area);
        let raw = if let Some(selection) = selection {
            if selection.format() == format {
                let mut raw = selection.try_read_interleaved_with(part, control).map_err(|e| raster_error(control, spot, e))?;
                if matches!(format.sample, SampleType::F32 | SampleType::F64) {
                    for (index, pixel) in raw.chunks_exact_mut(format.bytes_per_pixel()).enumerate() {
                        if index.is_multiple_of(4096) {
                            poll(control)?;
                        }
                        let value = photocraft_color::try_read_sample_f64(pixel, format.sample, 0).map_err(|e| raster_error(control, spot, e))?;
                        if !value.is_finite() {
                            return Err(bad(cmd(spot), "nonfinite selection coverage"));
                        }
                        if format.sample == SampleType::F64 && !(0.0..=1.0).contains(&value) {
                            return Err(bad(cmd(spot), "wide selection requires native unit coverage"));
                        }
                        if format.sample == SampleType::F32 && !(0.0..=1.0).contains(&value) {
                            photocraft_color::try_write_sample_f64(pixel, format.sample, 0, value.clamp(0., 1.)).map_err(|e| raster_error(control, spot, e))?;
                        }
                    }
                }
                raw
            } else {
                let plane = super::read_plane_with_interrupt(selection, part, control)?;
                let mut raw = Vec::new();
                raw.try_reserve_exact(plane.len().checked_mul(format.bytes_per_pixel()).ok_or_else(|| bad(cmd(spot), "tile bytes"))?)
                    .map_err(|_| bad(cmd(spot), "tile allocation"))?;
                for (index, value) in plane.iter().enumerate() {
                    if index.is_multiple_of(4096) {
                        poll(control)?;
                    }
                    if !value.is_finite() {
                        return Err(bad(cmd(spot), "nonfinite selection coverage"));
                    }
                    let mut sample = [0u8; 8];
                    let native = sample.get_mut(..format.bytes_per_pixel()).ok_or_else(|| bad(cmd(spot), "channel sample width"))?;
                    photocraft_raster::encode_pixel(&format, &[value.clamp(0., 1.)], native);
                    raw.extend_from_slice(native);
                }
                raw
            }
        } else {
            let count = usize::try_from(part.width())
                .ok()
                .and_then(|w| w.checked_mul(usize::try_from(part.height()).ok()?))
                .ok_or_else(|| bad(cmd(spot), "tile area"))?;
            let length = count.checked_mul(format.bytes_per_pixel()).ok_or_else(|| bad(cmd(spot), "tile bytes"))?;
            let mut raw = Vec::new();
            raw.try_reserve_exact(length).map_err(|_| bad(cmd(spot), "tile allocation"))?;
            let mut one = [0u8; 8];
            let one = one.get_mut(..format.bytes_per_pixel()).ok_or_else(|| bad(cmd(spot), "channel sample width"))?;
            photocraft_color::try_write_sample_f64(one, format.sample, 0, 1.).map_err(|e| raster_error(control, spot, e))?;
            for index in 0..count {
                if index.is_multiple_of(4096) {
                    poll(control)?;
                }
                raw.extend_from_slice(one);
            }
            raw
        };
        if raw.capacity() as u64 > MAX_CHECKED_RASTER_BYTES {
            return Err(bad(cmd(spot), "tile capacity"));
        }
        out.try_write_interleaved_with(part, &raw, control).map_err(|e| raster_error(control, spot, e))?;
    }
    out.try_prune_with(control).map_err(|e| raster_error(control, spot, e))?;
    poll(control)?;
    Ok(out)
}
pub(super) fn run(session: &mut Session, params: &Value, spot: bool) -> Result<Value> {
    dispatch_preflight(session, params, spot)?;
    let config = settings(params, spot)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let snapshot = state.doc.clone();
    let revision = state.revision;
    let weak = snapshot.downgrade();
    // Keep params transport small and caller-owned; no source locator is consulted.
    let explicit = match params.get("name").and_then(Value::as_str) {
        Some(text) => Some(name(&snapshot, Some(text), spot, &Interrupt::NONE)?),
        None => None,
    };
    let worker = snapshot.clone();
    let label = if spot { "New Spot Channel" } else { "New Channel" };
    crate::jobs::run(
        session,
        label,
        true,
        move |ctx| {
            ctx.stage(0., 0.95, label, |control| {
                admission(&worker, spot, control)?;
                let proof =
                    if config.raw_storage && worker.xcf_state.is_some() { Some(canonical_selection_storage::prepare_new(&worker, control)?) } else { None };
                let channel_name = name(&worker, explicit.as_deref(), spot, control)?;
                let channel_surface = if let Some(proof) = &proof {
                    let selection = worker.selection.as_ref().ok_or_else(|| bad(cmd(spot), "canonical New source selection disappeared"))?;
                    super::selection_storage_controlled::copy_canonical_new(&worker, selection, channel_format(&worker), proof, cmd(spot), control)?
                } else if config.raw_storage {
                    let selection = worker.selection.as_ref().ok_or_else(|| bad(cmd(spot), "rawStorage selection disappeared"))?;
                    super::selection_storage_controlled::copy(&worker, &worker, selection, channel_format(&worker), cmd(spot), control)?
                } else {
                    surface(&worker, config, spot, control)?
                };
                let mut channel = AlphaChannel::new(channel_name, channel_surface);
                channel.id = if let Some(proof) = &proof { proof.fresh_id(&worker, control)? } else { fresh(&worker, spot, control)? };
                if spot {
                    channel.spot = Some((config.color, config.solidity));
                } else {
                    channel.color = config.color;
                    if let Some(opacity) = config.opacity {
                        channel.opacity = opacity;
                    }
                    if let Some(indicates) = config.indicates {
                        channel.indicates = indicates;
                    }
                }
                poll(control)?;
                Ok((channel, proof, ctx.clone()))
            })
        },
        move |session, (channel, proof, ctx)| {
            ctx.stage(0.95, 1., label, |control| {
                poll(control)?;
                let state = session.active().ok_or(EngineError::NoDocument)?;
                if state.doc.id != snapshot.id
                    || state.revision != revision
                    || !weak.is_current(&state.doc)
                    || !state.doc.shares_state(&snapshot)
                    || state.floating.is_some()
                    || state.coalesce.is_some()
                    || session.coalesce_request.is_some()
                    || session.color_restrict.is_some()
                    || session.command_has_job_on(snapshot.id)
                    || session.command_has_pending_job()
                {
                    return Err(bad(cmd(spot), "target changed during channel preparation"));
                }
                let index = session.edit_with_interrupt(label, control, move |doc, _| {
                    poll(control)?;
                    if let Some(proof) = proof {
                        return proof.install(doc, channel, control);
                    }
                    doc.channels.try_reserve_exact(1).map_err(|_| bad(cmd(spot), "channel allocation"))?;
                    let index = doc.channels.len();
                    doc.channels.push(channel);
                    poll(control)?;
                    Ok(index)
                })?;
                if spot {
                    if let Some(state) = session.active_mut() {
                        state.channel_view.set_alpha_visible(index, true);
                        touch(state);
                    }
                } else {
                    target(session, ChannelTarget::Alpha(index));
                }
                Ok(json!({"channel":index}))
            })
        },
    )
}

// Meaningful implementation drafts; no test execution in this production wave.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::Size;
    fn fixture(sample: SampleType) -> (Session, Vec<u8>) {
        let mut doc = Document::new("Own selection", Size::new(2, 1), ColorMode::Rgb, sample);
        let raw = match sample {
            SampleType::U32 => [1u32, u32::MAX - 1].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>(),
            SampleType::F64 => [0.5f64.to_bits() + 1, (-0f64).to_bits()].into_iter().flat_map(u64::to_le_bytes).collect(),
            _ => vec![1, 254],
        };
        doc.selection = Some(Surface::try_from_interleaved_with(channel_format(&doc), doc.bounds(), &raw, &Interrupt::NONE).unwrap());
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        (session, raw)
    }
    #[test]
    fn native_selection_words_metadata_and_one_undo_survive_native_roundtrip() {
        for sample in [SampleType::U32, SampleType::F64] {
            let (mut session, raw) = fixture(sample);
            let before = session.active().unwrap().doc.clone();
            run(&mut session, &json!({"fill":"selection","name":"Own raw"}), false).unwrap();
            let current = &session.active().unwrap().doc;
            assert_eq!(current.channels[0].surface.to_interleaved(current.bounds()), raw);
            assert_eq!(current.channels[0].surface.default_bytes(), vec![0; sample.bytes()]);
            assert_eq!(current.selection, before.selection);
            assert_eq!(current.channels[0].name, "Own raw");
            let reopened = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(current, &Default::default()).unwrap()).unwrap();
            assert_eq!(reopened.channels, current.channels);
            assert_eq!(session.active().unwrap().channel_view.target, ChannelTarget::Alpha(0));
            session.execute("edit.undo", Value::Null).unwrap();
            assert!(session.active().unwrap().doc.channels.is_empty());
            assert_eq!(session.active().unwrap().doc.selection, before.selection);
            session.execute("edit.redo", Value::Null).unwrap();
            assert_eq!(session.active().unwrap().doc.channels, reopened.channels);
        }
    }
    #[test]
    fn original_defaults_fill_and_spot_overlay_are_preserved() {
        let (mut session, _) = fixture(SampleType::U8);
        run(&mut session, &Value::Null, false).unwrap();
        assert_eq!(session.active().unwrap().doc.channels[0].name, "Alpha 1");
        assert_eq!(session.active().unwrap().doc.channels[0].surface.to_interleaved(session.active().unwrap().doc.bounds()), vec![0, 0]);
        run(&mut session, &json!({"fill":"white","color":"#123456","opacity":25,"indicates":"selected"}), false).unwrap();
        let white = &session.active().unwrap().doc.channels[1];
        assert_eq!(white.name, "Alpha 2");
        assert_eq!(white.surface.to_interleaved(session.active().unwrap().doc.bounds()), vec![255, 255]);
        assert_eq!(white.opacity, 0.25);
        assert_eq!(white.indicates, ColorIndicates::SelectedAreas);
        assert_eq!(white.color, Color::rgb(18. / 255., 52. / 255., 86. / 255.));
        run(&mut session, &json!({"color":"#010203","solidity":75}), true).unwrap();
        let spot = &session.active().unwrap().doc.channels[2];
        assert_eq!(spot.name, "Spot Color 1");
        assert_eq!(spot.spot, Some((Color::rgb(1. / 255., 2. / 255., 3. / 255.), 0.75)));
        assert_eq!(spot.opacity, 0.5);
        assert_eq!(spot.color, AlphaChannel::DEFAULT_COLOR);
        assert_eq!(spot.indicates, ColorIndicates::MaskedAreas);
        assert_eq!(spot.surface.to_interleaved(session.active().unwrap().doc.bounds()), vec![1, 254]);
        assert_eq!(session.active().unwrap().channel_view.target, ChannelTarget::Alpha(1));
        assert!(session.active().unwrap().channel_view.alpha_visible[2]);
    }
    #[test]
    fn stale_malformed_and_unqualified_wide_conversion_preserve_real_redo() {
        let (mut session, _) = fixture(SampleType::U32);
        session
            .edit("Seed", |doc, _| {
                doc.name = "Seed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", Value::Null).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let redo = session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
        for params in [
            json!({"fill":0}),
            json!({"opacity":false}),
            json!({"color":[1.,2.]}),
            json!({"name":"x".repeat(NAME_LIMIT+1)}),
            json!({"documentId":before.id.0.to_string()}),
            json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()}),
        ] {
            assert!(run(&mut session, &params, false).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().revision, revision);
            assert_eq!(session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
        }
        let mut private = (*before).clone();
        private.selection = Some(Surface::new(photocraft_color::PixelFormat::GRAY8));
        let mut unsupported = Session::new();
        unsupported.add_document(private, None).unwrap();
        assert!(dispatch_preflight(&unsupported, &json!({"fill":"selection"}), false).is_err());
    }
    #[test]
    fn cancellation_during_preparation_never_publishes_or_mutates_selection() {
        let (session, _) = fixture(SampleType::F64);
        let before = session.active().unwrap().doc.clone();
        let cancel = Interrupt::cancel_only(&|| true);
        assert!(matches!(admission(&before, false, &cancel), Err(EngineError::Cancelled)));
        assert!(matches!(surface(&before, settings(&json!({"fill":"selection"}), false).unwrap(), false, &cancel), Err(EngineError::Cancelled)));
        assert!(matches!(fresh(&before, false, &cancel), Err(EngineError::Cancelled)));
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert!(session.active().unwrap().doc.channels.is_empty());
    }
    #[test]
    fn exhausted_second_revision_refuses_before_dispatch_callback() {
        for spot in [false, true] {
            for revision in [u64::MAX - 1, u64::MAX] {
                let (mut session, _) = fixture(SampleType::U32);
                session.active_mut().unwrap().revision = revision;
                let snapshot = session.active().unwrap().doc.clone();
                let target = session.active().unwrap().channel_view.target;
                let mut callback_called = false;
                let result = dispatch_preflight(&session, &Value::Null, spot).and_then(|()| {
                    callback_called = true;
                    run(&mut session, &Value::Null, spot)
                });
                assert!(result.unwrap_err().to_string().contains("two revisions of headroom"));
                assert!(!callback_called);
                assert!(run(&mut session, &Value::Null, spot).is_err());
                assert_eq!(session.active().unwrap().revision, revision);
                assert!(session.active().unwrap().doc.shares_state(&snapshot));
                assert!(session.active().unwrap().doc.channels.is_empty());
                assert_eq!(session.active().unwrap().channel_view.target, target);
            }
        }
    }

    #[test]
    fn explicit_raw_selection_creation_preserves_default_offcanvas_and_metadata() {
        let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let mut doc = Document::new("Raw new channel", photocraft_geom::Size::new(2, 2), ColorMode::Rgb, SampleType::F64);
        let mut selection = Surface::try_with_default_bytes(format, &(-0.0f64).to_ne_bytes()).unwrap();
        let nan = 0x7ff8000000000042u64.to_ne_bytes();
        selection.try_write_interleaved(photocraft_geom::Rect::new(-7, 9, -6, 10), &nan).unwrap();
        doc.selection = Some(selection.clone());
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        run(&mut session, &json!({"fill":"selection","rawStorage":true,"name":"Raw copy","opacity":25}), false).unwrap();
        let state = session.active().unwrap();
        let channel = &state.doc.channels[0];
        assert_eq!(channel.surface.default_bytes(), selection.default_bytes());
        assert_eq!(channel.surface.try_read_interleaved(photocraft_geom::Rect::new(-7, 9, -6, 10)).unwrap(), nan);
        assert!(selection.tiles().zip(channel.surface.tiles()).all(|((a, b), (c, d))| a == c && std::sync::Arc::ptr_eq(b, d)));
        assert_eq!(channel.name, "Raw copy");
        assert_eq!(channel.opacity, 0.25);
        assert!(session.undo());
        assert!(session.active().unwrap().doc.channels.is_empty());
        assert!(session.redo());
        for params in [json!({"fill":"white","rawStorage":true}), json!({"fill":"selection","rawStorage":1})] {
            assert!(dispatch_preflight(&session, &params, false).is_err());
        }
        assert!(dispatch_preflight(&session, &json!({"fromSelection":true,"rawStorage":true}), true).is_err());
    }
}
