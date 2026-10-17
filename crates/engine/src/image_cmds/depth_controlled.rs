//! Private, cancellable same-model bit-depth preparation. No CMS/indexed promotion.
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_color::SampleType;
use photocraft_doc::{Document, FilterOutputPrecision, Layer, LayerContent};
use photocraft_geom::TILE_SIZE;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface};
use serde_json::Value;

const MAX_ITEMS: usize = 100_000;
fn error(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("image.mode bit depth: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn raster(error_value: RasterInputError) -> EngineError {
    if error_value == RasterInputError::Cancelled { EngineError::Cancelled } else { error(error_value) }
}
fn target(id: &str) -> Option<SampleType> {
    match id {
        "image.mode.bits8" => Some(SampleType::U8),
        "image.mode.bits16" => Some(SampleType::U16),
        "image.mode.bits32" => Some(SampleType::F32),
        _ => None,
    }
}
fn exact(value: &Value, nonzero: bool) -> Result<u64> {
    let text = value
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= 20 && text.bytes().all(|b| b.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0')))
        .ok_or_else(|| error("captured target must use canonical decimal strings"))?;
    let value = text.parse::<u64>().map_err(|_| error("captured target exceeds range"))?;
    if nonzero && value == 0 {
        return Err(error("documentId must be nonzero"));
    }
    Ok(value)
}
fn context(session: &Session, params: &Value, allow_legacy_normalization: bool) -> Result<()> {
    if !params.is_null() && !params.as_object().is_some_and(|p| p.len() <= 2 && p.keys().all(|k| matches!(k.as_str(), "documentId" | "expectedRevision"))) {
        return Err(error("expected only optional documentId/expectedRevision"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if exact(id, true)? == state.doc.id.0 && exact(revision, false)? == state.revision => {}
        _ => return Err(error("captured target changed or incomplete")),
    }
    if state.revision == u64::MAX
        || (state.floating.is_some() && (!allow_legacy_normalization || params.get("documentId").is_some()))
        || state.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
        || state.coalesce.is_some()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.command_has_job_on(state.doc.id)
    {
        return Err(error("finish transient edits or pending work before changing depth"));
    }
    if state.doc.accounted().is_some() {
        return Err(error("accounted document depth conversion is unsupported"));
    }
    Ok(())
}
/// Root dispatches this borrowed guard before normalization or parameter cloning.
pub(super) fn dispatch_preflight(session: &Session, id: &str, params: &Value, _background: bool) -> Result<bool> {
    if target(id).is_none() {
        return Ok(false);
    }
    context(session, params, true)?;
    Ok(true)
}
fn no_op(doc: &Document, depth: SampleType) -> bool {
    doc.depth == depth && !doc.xcf_state.as_ref().is_some_and(|state| state.current_filter_output_precision == FilterOutputPrecision::Half)
}
fn visit_layers(layers: &[Layer], level: usize, count: &mut usize, control: &Interrupt<'_>, visit: &mut dyn FnMut(&Surface) -> Result<()>) -> Result<()> {
    if level > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(error("layer depth limit"));
    }
    for layer in layers {
        poll(control)?;
        *count = count.checked_add(1).filter(|n| *n <= MAX_ITEMS).ok_or_else(|| error("layer count limit"))?;
        if let Some(mask) = &layer.mask {
            visit(&mask.surface)?;
        }
        if let Some(cache) = &layer.fill_cache {
            visit(&cache.surface)?;
        }
        match &layer.content {
            LayerContent::Raster(surface) => visit(surface)?,
            LayerContent::Text(text) => {
                if let Some(surface) = &text.cache {
                    visit(surface)?
                }
            }
            LayerContent::Shape(shape) => {
                if let Some(surface) = &shape.cache {
                    visit(surface)?
                }
            }
            LayerContent::Smart(smart) => {
                if let Some(surface) = &smart.cache {
                    visit(surface)?;
                }
                if let Some(mask) = &smart.filter_mask {
                    visit(&mask.surface)?;
                }
            }
            LayerContent::BackedRaster(_) => return Err(error("backed raster depth conversion is unsupported")),
            LayerContent::Group(group) => visit_layers(&group.children, level + 1, count, control, visit)?,
            _ => {}
        }
    }
    Ok(())
}
/// All shared payload/model/maps and simultaneous output owners are admitted
/// before the first private Document clone. This is not an allocator/RSS claim.
fn prepare(snapshot: &DocumentSnapshot, depth: SampleType, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    snapshot.validate_resident()?;
    snapshot.validate_xcf_state_with(control).map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(e) })?;
    if snapshot.mode == photocraft_color::ColorMode::Indexed || snapshot.xcf_state.as_ref().is_some_and(|s| !s.editable_indexed.is_empty()) {
        return Err(error("indexed depth conversion requires an explicitly qualified ordinal conversion"));
    }
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, limits, control)
        .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(e) })? as u64;
    let model = photocraft_io::retained_native_model_bytes(snapshot, limits, control)
        .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(e) })? as u64;
    let retained = snapshot.resident_raster_footprint_with(control)?;
    let mut resident = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(retained.retained_bytes))
        .and_then(|n| n.checked_add(retained.clone_metadata_bytes.checked_mul(4)?))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| error("model/payload/COW headroom limit"))?;
    let mut work = 0u64;
    let mut visit = |surface: &Surface| -> Result<()> {
        poll(control)?;
        surface.try_validate_tile_origins_with(control).map_err(raster)?;
        let format = surface.format().with_sample(depth);
        let tiles = surface.tile_count() as u64;
        let tile_bytes = (TILE_SIZE as u64)
            .checked_mul(TILE_SIZE as u64)
            .and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64))
            .ok_or_else(|| error("tile output length overflow"))?;
        let output = tiles
            .checked_mul(tile_bytes.checked_add(512).ok_or_else(|| error("tile COW overflow"))?)
            .and_then(|n| n.checked_add(format.bytes_per_pixel() as u64))
            .ok_or_else(|| error("output budget overflow"))?;
        resident = resident
            .checked_add(output.checked_mul(2).ok_or_else(|| error("staging peak overflow"))?)
            .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
            .ok_or_else(|| error("aggregate conversion/output limit"))?;
        let crosses = (snapshot.depth == SampleType::F32) != (depth == SampleType::F32)
            && matches!(snapshot.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale);
        if crosses {
            // Conservatively price every surface, including mask/channel surfaces
            // which do not execute colour transfer, before cloning any document.
            let float_format = surface.format().with_sample(SampleType::F32);
            let mapping = tiles
                .checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64))
                .and_then(|n| n.checked_mul(float_format.bytes_per_pixel() as u64))
                .and_then(|n| n.checked_add(tiles.checked_mul(512)?))
                .and_then(|n| n.checked_add(1024))
                .ok_or_else(|| error("colour transfer output budget"))?;
            resident = resident
                .checked_add(mapping.checked_mul(2).ok_or_else(|| error("colour transfer staging peak"))?)
                .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
                .ok_or_else(|| error("aggregate colour transfer/output limit"))?;
            let extra_work = tiles
                .checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64))
                .and_then(|n| n.checked_add(1))
                .and_then(|n| n.checked_mul(float_format.channels() as u64))
                .and_then(|n| n.checked_mul(16))
                .ok_or_else(|| error("colour transfer work overflow"))?;
            work = work.checked_add(extra_work).filter(|n| *n <= 1 << 32).ok_or_else(|| error("colour transfer work limit"))?;
        }
        work = work
            .checked_add(
                tiles
                    .checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64))
                    .and_then(|n| n.checked_mul(format.channels() as u64))
                    .ok_or_else(|| error("sample work overflow"))?,
            )
            .filter(|n| *n <= 1 << 32)
            .ok_or_else(|| error("sample work limit"))?;
        Ok(())
    };
    visit_layers(&snapshot.layers, 0, &mut 0, control, &mut visit)?;
    if snapshot.channels.len() > MAX_ITEMS {
        return Err(error("channel count limit"));
    }
    for channel in snapshot.channels.iter().chain(snapshot.quick_mask.as_ref()) {
        poll(control)?;
        visit(&channel.surface)?;
    }
    poll(control)
}
fn convert(surface: &mut Surface, depth: SampleType, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let proposed = surface.try_convert_with(surface.format().with_sample(depth), control).map_err(raster)?;
    poll(control)?;
    *surface = proposed;
    Ok(())
}

// RGB/Gray colour samples cross the established document transfer only around
// F32 depth. Alpha, layer/filter masks and saved channels never enter this map.
fn map_color_transfer(surface: &Surface, transfer: photocraft_compose::adjust::Transfer, decode: bool, control: &Interrupt<'_>) -> Result<Surface> {
    poll(control)?;
    let format = surface.format();
    if format.sample != SampleType::F32 {
        return Err(error("transfer requires F32 colour samples"));
    }
    let colors = format.mode.color_channels();
    let stride = format.bytes_per_pixel();
    let map = |pixel: &mut [u8]| -> Result<()> {
        for word in pixel.get_mut(..colors * 4).ok_or_else(|| error("transfer colour layout"))?.as_chunks_mut::<4>().0 {
            let value = f32::from_ne_bytes(*word);
            if !value.is_finite() {
                return Err(error("nonfinite transfer colour"));
            }
            let value = if decode { transfer.decode(value) } else { transfer.encode(value).clamp(0.0, 1.0) };
            if !value.is_finite() {
                return Err(error("nonfinite transfer output"));
            }
            *word = value.to_ne_bytes();
        }
        Ok(())
    };
    let mut default_storage = [0u8; 20];
    let default = default_storage.get_mut(..stride).ok_or_else(|| error("transfer default layout"))?;
    default.copy_from_slice(surface.default_bytes());
    map(default)?;
    let mut out = Surface::try_with_default_bytes(format, default).map_err(raster)?;
    let capacity_limit = surface.tile_count().checked_mul(4).and_then(|n| n.checked_add(8)).ok_or_else(|| error("transfer identity capacity"))?;
    let mut converted: std::collections::HashMap<*const photocraft_raster::Tile, std::sync::Arc<photocraft_raster::Tile>> = std::collections::HashMap::new();
    converted.try_reserve(surface.tile_count()).map_err(|_| error("transfer identity allocation"))?;
    if converted.capacity() > capacity_limit {
        return Err(error("transfer identity capacity"));
    }
    for (coordinate, tile) in surface.tiles() {
        poll(control)?;
        let identity = std::sync::Arc::as_ptr(tile);
        if let Some(existing) = converted.get(&identity) {
            out.put_tiles([(*coordinate, existing.clone())]);
            continue;
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(tile.bytes().len()).map_err(|_| error("transfer tile allocation"))?;
        if bytes.capacity() > tile.bytes().len().checked_add(128).ok_or_else(|| error("transfer tile capacity overflow"))? {
            return Err(error("transfer tile allocator capacity"));
        }
        for chunk in tile.bytes().chunks(65_536) {
            poll(control)?;
            bytes.extend_from_slice(chunk);
        }
        for (index, pixel) in bytes.chunks_exact_mut(stride).enumerate() {
            if index.is_multiple_of(256) {
                poll(control)?;
            }
            map(pixel)?;
        }
        out.try_write_interleaved_with(coordinate.rect(), &bytes, control).map_err(raster)?;
        let result = out.tile(*coordinate).cloned().ok_or_else(|| error("transfer tile missing"))?;
        converted.insert(identity, result);
    }
    poll(control)?;
    Ok(out)
}
fn convert_color(surface: &mut Surface, depth: SampleType, color: Option<(photocraft_compose::adjust::Transfer, bool)>, control: &Interrupt<'_>) -> Result<()> {
    if let Some((transfer, false)) = color {
        *surface = map_color_transfer(surface, transfer, false, control)?;
    }
    convert(surface, depth, control)?;
    if let Some((transfer, true)) = color {
        *surface = map_color_transfer(surface, transfer, true, control)?;
    }
    Ok(())
}

fn convert_layers(
    layers: &mut [Layer],
    level: usize,
    count: &mut usize,
    depth: SampleType,
    color: Option<(photocraft_compose::adjust::Transfer, bool)>,
    control: &Interrupt<'_>,
) -> Result<()> {
    if level > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(error("layer depth limit"));
    }
    for layer in layers {
        poll(control)?;
        *count = count.checked_add(1).filter(|n| *n <= MAX_ITEMS).ok_or_else(|| error("layer count limit"))?;
        if let Some(mask) = &mut layer.mask {
            convert(std::sync::Arc::make_mut(&mut mask.surface), depth, control)?;
        }
        if let Some(cache) = &mut layer.fill_cache {
            convert_color(&mut cache.surface, depth, color, control)?;
        }
        match &mut layer.content {
            LayerContent::Raster(surface) => convert_color(surface, depth, color, control)?,
            LayerContent::Text(text) => {
                if let Some(surface) = &mut text.cache {
                    convert_color(surface, depth, color, control)?;
                }
            }
            LayerContent::Shape(shape) => {
                if let Some(surface) = &mut shape.cache {
                    convert_color(surface, depth, color, control)?;
                }
            }
            LayerContent::Smart(smart) => {
                if let Some(surface) = &mut smart.cache {
                    convert_color(surface, depth, color, control)?;
                }
                if let Some(mask) = &mut smart.filter_mask {
                    convert(std::sync::Arc::make_mut(&mut mask.surface), depth, control)?;
                }
            }
            LayerContent::BackedRaster(_) => return Err(error("backed raster depth conversion is unsupported")),
            LayerContent::Group(group) => convert_layers(&mut group.children, level + 1, count, depth, color, control)?,
            _ => {}
        }
    }
    Ok(())
}
fn stage(snapshot: &DocumentSnapshot, depth: SampleType, control: &Interrupt<'_>) -> Result<Document> {
    prepare(snapshot, depth, control)?;
    poll(control)?;
    let mut doc = (**snapshot).clone();
    let to_linear = depth == SampleType::F32;
    let crosses =
        (snapshot.depth == SampleType::F32) != to_linear && matches!(snapshot.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale);
    let transfer = photocraft_compose::adjust::Transfer::for_document(snapshot.mode, if to_linear { snapshot.depth } else { depth });
    let color = crosses.then_some((transfer, to_linear));
    convert_layers(&mut doc.layers, 0, &mut 0, depth, color, control)?;
    for channel in doc.channels.iter_mut().chain(doc.quick_mask.as_mut()) {
        poll(control)?;
        convert(&mut channel.surface, depth, control)?;
    }
    if let Some(state) = &mut doc.xcf_state {
        poll(control)?;
        state.current_filter_output_precision = FilterOutputPrecision::Canonical;
    }
    doc.depth = depth;
    poll(control)?;
    Ok(doc)
}
pub(super) fn convert_depth(session: &mut Session, depth: SampleType, params: &Value) -> Result<Value> {
    if !matches!(depth, SampleType::U8 | SampleType::U16 | SampleType::F32) {
        return Err(error("unregistered target depth"));
    }
    context(session, params, false)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if no_op(&state.doc, depth) {
        return Ok(Value::Null);
    }
    let baseline = state.doc.clone();
    let weak = baseline.downgrade();
    let revision = state.revision;
    let work_base = baseline.clone();
    crate::jobs::run(
        session,
        "Bit Depth",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Bit Depth", |control| {
                let document = stage(&work_base, depth, control)?;
                Ok((document, ctx.clone()))
            })
        },
        move |session, (document, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id
                || current.revision != revision
                || !weak.is_current(&current.doc)
                || !current.doc.shares_state(&baseline)
                || current.floating.is_some()
                || current.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
                || current.coalesce.is_some()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
            {
                return Err(error("target changed while preparing depth"));
            }
            ctx.stage(0.95, 1.0, "Bit Depth", |control| {
                poll(control)?;
                session.edit_with_interrupt("Bit Depth", control, move |current, _| {
                    poll(control)?;
                    *current = document;
                    Ok(())
                })?;
                Ok(Value::Null)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::ColorMode;
    use photocraft_doc::Size;
    #[test]
    fn early_legacy_float_can_normalize_but_captured_and_run_guards_cannot() {
        let mut session = Session::new();
        session.execute("file.new", serde_json::json!({"width":2,"height":2,"background":"#123456"})).unwrap();
        session.execute("select.all", Value::Null).unwrap();
        session.execute("select.float", serde_json::json!({"dx":1,"dy":0,"copy":true})).unwrap();
        let state = session.active().unwrap();
        assert!(state.floating.is_some());
        let captured = serde_json::json!({"documentId":state.doc.id.0.to_string(),"expectedRevision":state.revision.to_string()});
        assert!(dispatch_preflight(&session, "image.mode.bits16", &Value::Null, false).unwrap());
        assert!(dispatch_preflight(&session, "image.mode.bits16", &captured, false).is_err());
        assert!(context(&session, &Value::Null, false).is_err());
        assert!(session.active().unwrap().floating.is_some());
    }
    #[test]
    fn private_conversion_cancellation_nonfinite_and_raw_default_are_atomic() {
        let mut document = Document::new("Own depth", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
        document.layers.push(Layer::new(
            "Raw source",
            LayerContent::Raster(
                Surface::try_with_default_bytes(
                    photocraft_color::PixelFormat::RGBA64F,
                    &[0.125f64, 0.5, 0.75, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>(),
                )
                .unwrap(),
            ),
        ));
        let mut session = Session::new();
        session.add_document(document.clone(), None).unwrap();
        let original = session.active().unwrap().doc.clone();
        let cancelled = || true;
        assert!(matches!(stage(&original, SampleType::U16, &Interrupt::cancel_only(&cancelled)), Err(EngineError::Cancelled)));
        assert!(original.shares_state(&session.active().unwrap().doc));
        document.layers[0].content = LayerContent::Raster(
            Surface::try_with_default_bytes(
                photocraft_color::PixelFormat::RGBA64F,
                &[f64::NAN, 0.0, 0.0, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>(),
            )
            .unwrap(),
        );
        let bad = DocumentSnapshot::from_resident(std::sync::Arc::new(document)).unwrap();
        assert!(stage(&bad, SampleType::U8, &Interrupt::NONE).is_err());
        let good = stage(&original, SampleType::U16, &Interrupt::NONE).unwrap();
        assert_eq!(good.depth, SampleType::U16);
        assert_eq!(good.icc_profile, original.icc_profile);
        assert_eq!(good.physical_resolution, original.physical_resolution);
    }
    #[test]
    fn registered_identity_depth_keeps_real_redo_and_captured_target_refuses() {
        let mut session = Session::new();
        session.execute("file.new", serde_json::json!({"width":1,"height":1})).unwrap();
        session.execute("image.mode.bits16", Value::Null).unwrap();
        assert!(session.undo());
        let revision = session.active().unwrap().revision;
        session.execute("image.mode.bits8", Value::Null).unwrap();
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.redo());
        assert!(dispatch_preflight(&session, "image.mode.bits8", &serde_json::json!({"documentId":"0","expectedRevision":"0"}), false).is_err());
    }
    #[test]
    fn f32_crossing_linearizes_colour_but_not_alpha_masks_or_channels() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let format = photocraft_color::PixelFormat::new(mode, SampleType::U8, true);
            let colors = mode.color_channels();
            let mut values = vec![128.0 / 255.0; format.channels()];
            values[colors] = 96.0 / 255.0;
            let source = Surface::with_default(format, &values);
            let mut layer = Layer::new("colour default", LayerContent::Raster(source));
            let mut mask = photocraft_doc::LayerMask::reveal_all();
            mask.surface = std::sync::Arc::new(Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[128.0 / 255.0]));
            layer.mask = Some(mask);
            let mut document = Document::new("transfer", Size::new(1, 1), mode, SampleType::U8);
            document.layers.push(layer);
            document
                .channels
                .push(photocraft_doc::AlphaChannel::new("saved coverage", Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[64.0 / 255.0])));
            let original = DocumentSnapshot::from_resident(std::sync::Arc::new(document.clone())).unwrap();
            let widened = stage(&original, SampleType::F32, &Interrupt::NONE).unwrap();
            let pixel = widened.layers[0].surface().unwrap().pixel(0, 0);
            let encoded = 128.0f32 / 255.0;
            let expected = if mode == ColorMode::Rgb { ((encoded + 0.055) / 1.055).powf(2.4) } else { encoded.powf(1.732) };
            for value in pixel.iter().take(colors) {
                assert!((value - expected).abs() < 1e-6);
            }
            assert_eq!(pixel[colors], 96.0 / 255.0);
            assert_eq!(widened.layers[0].mask.as_ref().unwrap().surface.pixel(0, 0)[0], 128.0 / 255.0);
            assert_eq!(widened.channels[0].surface.pixel(0, 0)[0], 64.0 / 255.0);
            let widened = DocumentSnapshot::from_resident(std::sync::Arc::new(widened)).unwrap();
            let back = stage(&widened, SampleType::U8, &Interrupt::NONE).unwrap();
            assert_eq!(back, document, "full ordinary document roundtrip including coverage/defaults");
            assert!(matches!(stage(&original, SampleType::F32, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
            assert_eq!(&*original, &document);
        }
    }
}
