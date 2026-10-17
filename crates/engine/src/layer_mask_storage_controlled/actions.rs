//! Multi-target metadata storage and the explicitly bounded Apply subset.
use super::{decimal, poll};
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::Other(format!("layer.layerMask: {}", message.into()))
}
fn error(control: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(error.to_string()) }
}
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_doc::{ColorMode, Document, LayerContent, LayerId, LayerMask, SampleType, item_identity::ItemRef};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::Value;
use std::sync::Arc;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Reveal,
    Hide,
    Delete,
    Apply,
    HideSelection,
    FromTransparency,
}
impl Action {
    fn adds(self) -> bool {
        matches!(self, Self::Reveal | Self::Hide | Self::HideSelection)
    }
    fn label(self) -> &'static str {
        match self {
            Self::Reveal | Self::Hide | Self::HideSelection => "Add Layer Mask",
            Self::Delete => "Delete Layer Mask",
            Self::Apply => "Apply Layer Mask",
            Self::FromTransparency => "From Transparency",
        }
    }
}
fn transient(session: &Session) -> Result<()> {
    let st = session.active().ok_or(EngineError::NoDocument)?;
    if st.revision == u64::MAX
        || st.doc.accounted().is_some()
        || st.floating.is_some()
        || st.coalesce.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || st.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
    {
        return Err(bad("finish transient work before changing masks"));
    }
    Ok(())
}
fn mask_lock(doc: &Document, id: LayerId) -> Result<()> {
    let locks = crate::xcf_item_locks::effective_layer_locks(doc, id)?;
    if locks.native.all || locks.imported.content {
        return Err(bad("layer mask content is locked"));
    }
    // Newly authored ordinary masks do not invent a source MaskId. Existing typed
    // associations keep their additional inherited mask permissions.
    if doc.xcf_state.as_ref().is_some_and(|s| s.items.iter().any(|i| i.mask.as_ref().is_some_and(|m| m.owner == ItemRef::Layer(id)))) {
        crate::xcf_item_locks::require_layer_mask_content(doc, id)?;
    }
    Ok(())
}
fn request(session: &Session, params: &Value, action: Action) -> Result<Vec<LayerId>> {
    if !params.is_null() {
        let o = params.as_object().filter(|o| o.len() <= 3).ok_or_else(|| bad("bounded mask parameters required"))?;
        if o.keys().any(|k| !matches!(k.as_str(), "layer" | "documentId" | "expectedRevision")) {
            return Err(bad("unknown mask parameter"));
        }
    }
    let st = session.active().ok_or(EngineError::NoDocument)?;
    if st.selected_layers.len() > 10_000 {
        return Err(bad("mask target count limit"));
    }
    if let Some(id) = params.get("layer")
        && id.as_u64().is_none_or(|id| id == 0)
    {
        return Err(bad("layer must be a nonzero unsigned integer"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if decimal(id) == Some(st.doc.id.0) && st.doc.id.0 != 0 && decimal(rev) == Some(st.revision) => {}
        _ => return Err(bad("captured mask target changed or incomplete")),
    }
    transient(session)?;
    let mut ids = if action.adds() {
        crate::layer_multi_cmds::mask_targets(session, params)?
    } else if action == Action::FromTransparency {
        let mut ids = Vec::new();
        ids.try_reserve_exact(1).map_err(|_| bad("mask target allocation"))?;
        ids.push(crate::commands::layer_param(session, params)?);
        ids
    } else {
        crate::layer_multi_cmds::targets(session, params)?
    };
    if ids.is_empty() || ids.len() > 10_000 {
        return Err(bad("mask target count limit"));
    }
    // The order remains the upstream bottom-to-top order. Removing an absent mask
    // is still a legitimate no-op for a single explicit owner.
    if ids.len() > 1 && matches!(action, Action::Apply | Action::Delete) {
        ids.retain(|id| st.doc.layer(*id).is_some_and(|l| l.mask.is_some()));
    }
    if action == Action::Apply && ids.is_empty() {
        return Err(bad("no selected raster layer has a mask"));
    }
    let mut sorted = Vec::new();
    sorted.try_reserve_exact(ids.len()).map_err(|_| bad("mask target allocation"))?;
    sorted.extend_from_slice(&ids);
    sorted.sort_unstable();
    if sorted.windows(2).any(|p| p.first() == p.get(1)) {
        return Err(bad("ambiguous mask owner"));
    }
    let mut counts = Vec::new();
    counts.try_reserve_exact(ids.len()).map_err(|_| bad("mask census allocation"))?;
    counts.resize(ids.len(), 0usize);
    let mut visits = 0u64;
    photocraft_doc::resident_structure::visit_layers_with(&st.doc, &Interrupt::NONE, |layer| {
        visits = visits.checked_add(1).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        if let Ok(i) = sorted.binary_search(&layer.id) {
            let count = counts.get_mut(i).ok_or(photocraft_raster::RasterInputError::InvalidBounds)?;
            *count = count.checked_add(1).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        }
        Ok(true)
    })?;
    if counts.iter().any(|n| *n != 1) {
        return Err(bad("missing or ambiguous mask owner"));
    }
    let mut metadata = visits;
    if let Some(state) = &st.doc.xcf_state {
        for count in [state.items.len(), state.source_items.len(), state.filter_stacks.len()] {
            if count > 10_000 {
                return Err(bad("mask relationship count limit"));
            }
            metadata = metadata.checked_add(count as u64).ok_or_else(|| bad("mask metadata work overflow"))?;
        }
        for stack in &state.filter_stacks {
            if stack.nodes.len() > 10_000 {
                return Err(bad("mask filter node count limit"));
            }
            metadata = metadata.checked_add(stack.nodes.len() as u64).ok_or_else(|| bad("mask metadata work overflow"))?;
            for node in &stack.nodes {
                if node.auxiliary.len() > 64 {
                    return Err(bad("mask auxiliary count limit"));
                }
                metadata = metadata.checked_add(node.auxiliary.len() as u64).ok_or_else(|| bad("mask metadata work overflow"))?;
            }
        }
    }
    if metadata.checked_mul(ids.len() as u64).is_none_or(|n| n > 2_000_000) {
        return Err(bad("mask target/relationship work limit"));
    }
    st.doc.validate_xcf_state_with(&Interrupt::NONE).map_err(|e| bad(e.to_string()))?;
    if matches!(action, Action::Delete | Action::Apply | Action::FromTransparency | Action::HideSelection)
        && let Some(state) = &st.doc.xcf_state
    {
        // Build a bounded, sorted set of existing mask identities just once;
        // references are checked by binary search, not a target×node×item loop.
        let mut removed = Vec::new();
        removed.try_reserve_exact(state.items.len()).map_err(|_| bad("mask relationship allocation"))?;
        for item in &state.items {
            if item.mask.as_ref().is_some_and(|mask| matches!(mask.owner, ItemRef::Layer(id) if sorted.binary_search(&id).is_ok())) {
                removed.push(item.item);
            }
        }
        removed.sort_unstable();
        if state.source_items.iter().any(|source| removed.binary_search(&source.owner).is_ok())
            || state.filter_stacks.iter().any(|stack| {
                stack.nodes.iter().any(|node| {
                    node.mask.is_some_and(|mask| removed.binary_search(&mask).is_ok()) || node.auxiliary.iter().any(|aux| removed.binary_search(&aux.1).is_ok())
                })
            })
        {
            return Err(bad("retained source/filter mask relationship removal is not qualified"));
        }
    }
    for id in &ids {
        mask_lock(&st.doc, *id)?;
        if action == Action::Apply {
            apply_domain(&st.doc, *id)?;
        }
        if action == Action::FromTransparency {
            raster_domain(&st.doc, *id)?;
        }
    }
    if action == Action::HideSelection {
        let selection = st.doc.selection.as_ref().ok_or_else(|| bad("no selection"))?;
        if selection.format().mode != ColorMode::Grayscale || selection.format().alpha {
            return Err(bad("Hide Selection requires native Gray coverage"));
        }
    }
    Ok(ids)
}
fn raster_domain(doc: &Document, id: LayerId) -> Result<&Surface> {
    let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Raster(surface) = &layer.content else {
        return Err(bad("Apply requires a plain resident raster"));
    };
    crate::xcf_item_locks::effective_layer_locks(doc, id)?.require(crate::xcf_item_locks::EditKind::Content)?;
    if layer.indexed_source.is_some()
        || layer.video.is_some()
        || layer.import_compositing.is_some()
        || doc.color_table.is_some()
        || doc.mode == ColorMode::Indexed
    {
        return Err(bad("source/indexed mask application is not qualified"));
    }
    if doc
        .xcf_state
        .as_ref()
        .is_some_and(|s| s.source_items.iter().any(|i| i.owner == ItemRef::Layer(id)) || s.filter_stacks.iter().any(|f| f.owner == ItemRef::Layer(id)))
    {
        return Err(bad("source-backed or filtered mask application is not qualified"));
    }
    Ok(surface)
}
fn apply_domain(doc: &Document, id: LayerId) -> Result<()> {
    let surface = raster_domain(doc, id)?;
    let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let mask = layer.mask.as_ref().ok_or_else(|| bad("the layer has no layer mask"))?;
    if mask.surface.format().mode != ColorMode::Grayscale
        || mask.surface.format().alpha
        || !mask.density.is_finite()
        || !(0.0..=1.0).contains(&mask.density)
        || !mask.feather.is_finite()
    {
        return Err(bad("Apply requires a finite Gray mask"));
    }
    let wide =
        matches!(surface.format().sample, SampleType::U32 | SampleType::F64) || matches!(mask.surface.format().sample, SampleType::U32 | SampleType::F64);
    if wide && mask.enabled {
        if !surface.format().alpha || photocraft_compose::masks::feather_sigma(mask.feather) > 0.0 {
            return Err(bad("wide Apply requires native alpha and no feather arithmetic"));
        }
        if mask.density != 1.0 && !(surface.format().sample == SampleType::F64 && mask.surface.format().sample == SampleType::F64) {
            return Err(bad("wide density arithmetic requires same-depth F64; U32 and cross-depth coverage require density one"));
        }
    }
    // Ordinary layer effects and an independent vector mask are presentation
    // metadata, not part of the applied pixel mask. Preserve them unchanged.
    if doc.xcf_state.is_some() && mask.enabled && photocraft_compose::masks::feather_sigma(mask.feather) > 0.0 {
        return Err(bad("source-state feathered mask arithmetic is not qualified"));
    }
    Ok(())
}
fn tiles(r: Rect) -> Result<u64> {
    if r.is_empty() {
        return Ok(0);
    }
    let first = photocraft_geom::TileCoord::containing(r.x0, r.y0);
    let last =
        photocraft_geom::TileCoord::containing(r.x1.checked_sub(1).ok_or_else(|| bad("mask extent"))?, r.y1.checked_sub(1).ok_or_else(|| bad("mask extent"))?);
    u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1)
        .ok()
        .and_then(|w| u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).ok().and_then(|h| w.checked_mul(h)))
        .ok_or_else(|| bad("mask tile span overflow"))
}
fn admission(doc: &Document, ids: &[LayerId], action: Action, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let mut peak = payload
        .checked_add(model.checked_mul(6).ok_or_else(|| bad("mask model overflow"))?)
        .and_then(|n| n.checked_add(raster.retained_bytes))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(6)?))
        .and_then(|n| n.checked_add(2 << 20))
        .ok_or_else(|| bad("mask staging overflow"))?;
    let mut work = payload.checked_add(model).and_then(|n| n.checked_mul(16)).ok_or_else(|| bad("mask work overflow"))?;
    for id in ids {
        poll(control)?;
        mask_lock(doc, *id)?;
        let layer = doc.layer(*id).ok_or(EngineError::NoLayer(*id))?;
        if let Some(mask) = &layer.mask {
            mask.surface.try_validate_tile_origins_with(control).map_err(|e| error(control, e))?;
        }
        if matches!(action, Action::Apply | Action::FromTransparency) {
            if action == Action::Apply {
                apply_domain(doc, *id)?;
            } else {
                raster_domain(doc, *id)?;
            }
            let surface = layer.surface().ok_or_else(|| bad("not a resident raster"))?;
            let area = surface.try_content_bounds_with_interrupt(control).map_err(|e| error(control, e))?;
            let count = tiles(area)?;
            let bpp = surface.format().bytes_per_pixel() as u64;
            let pixels = count.checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(TILE_SIZE as u64)).ok_or_else(|| bad("mask pixel count overflow"))?;
            peak = peak
                .checked_add(pixels.checked_mul(bpp).and_then(|n| n.checked_add(count.checked_mul(2048)?)).ok_or_else(|| bad("mask output overflow"))?)
                .ok_or_else(|| bad("mask output overflow"))?;
            if action == Action::FromTransparency {
                let mask_bpp = if matches!(surface.format().sample, SampleType::U32 | SampleType::F64) { surface.format().sample.bytes() as u64 } else { 1 };
                peak = peak
                    .checked_add(
                        pixels.checked_mul(mask_bpp).and_then(|n| n.checked_add(count.checked_mul(2048)?)).ok_or_else(|| bad("alpha mask output overflow"))?,
                    )
                    .ok_or_else(|| bad("alpha mask output overflow"))?;
                work = work
                    .checked_add(pixels.checked_mul(128).ok_or_else(|| bad("alpha extraction work overflow"))?)
                    .ok_or_else(|| bad("alpha extraction work overflow"))?;
                continue;
            }
            let mask = layer.mask.as_ref().ok_or_else(|| bad("mask disappeared"))?;
            if mask.enabled && photocraft_compose::masks::feather_sigma(mask.feather) > 0.0 {
                let clip = doc.bounds().union(&area);
                // Upstream's combined-mask far probe subtracts one million.
                clip.x0.checked_sub(1_000_000).ok_or_else(|| bad("feather probe coordinate overflow"))?;
                clip.y0.checked_sub(1_000_000).ok_or_else(|| bad("feather probe coordinate overflow"))?;
                let mask_bounds = mask.surface.try_content_bounds_with_interrupt(control).map_err(|e| error(control, e))?;
                let pad = (photocraft_compose::masks::feather_sigma(mask.feather) * 3.0).ceil() as i32 + 2;
                let grown = if mask_bounds.is_empty() {
                    mask_bounds
                } else {
                    Rect::new(
                        mask_bounds.x0.saturating_sub(pad),
                        mask_bounds.y0.saturating_sub(pad),
                        mask_bounds.x1.saturating_add(pad),
                        mask_bounds.y1.saturating_add(pad),
                    )
                };
                let region = grown.intersect(&clip);
                let count = u64::from(region.width()).checked_mul(u64::from(region.height())).ok_or_else(|| bad("feather plane overflow"))?;
                // Actual upstream fallback: vector plane, mask plane, transpose,
                // up to 16 row scratches and U16 output/cache copies. This is
                // bounded admission, not a claim of deep compositor cancellation.
                let rows = u64::from(region.width().max(region.height()));
                let scratch = count
                    .checked_mul(32)
                    .and_then(|n| n.checked_add(rows.checked_mul(128)?))
                    .and_then(|n| n.checked_add(tiles(region).ok()?.checked_mul(2048)?))
                    .ok_or_else(|| bad("feather staging overflow"))?;
                peak = peak.checked_add(scratch).ok_or_else(|| bad("feather staging overflow"))?;
                work = work
                    .checked_add(count.checked_mul(256).and_then(|n| n.checked_add(rows.checked_mul(100_000)?)).ok_or_else(|| bad("feather work overflow"))?)
                    .ok_or_else(|| bad("feather work overflow"))?;
            }
            let mask_pixels = (mask.surface.tile_count() as u64)
                .checked_mul(TILE_SIZE as u64)
                .and_then(|n| n.checked_mul(TILE_SIZE as u64))
                .ok_or_else(|| bad("mask sample count overflow"))?;
            work = work
                .checked_add(pixels.checked_add(mask_pixels).and_then(|n| n.checked_mul(128)).ok_or_else(|| bad("mask work overflow"))?)
                .ok_or_else(|| bad("mask work overflow"))?;
        }
    }
    if action == Action::HideSelection {
        let selection = doc.selection.as_ref().ok_or_else(|| bad("no selection"))?;
        selection.try_validate_tile_origins_with(control).map_err(|e| error(control, e))?;
        let count = tiles(doc.bounds())?;
        let pixels = count.checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(TILE_SIZE as u64)).ok_or_else(|| bad("selection mask output overflow"))?;
        let bpp = if matches!(selection.format().sample, SampleType::U32 | SampleType::F64) { selection.format().sample.bytes() as u64 } else { 1 };
        peak = peak
            .checked_add(pixels.checked_mul(bpp).and_then(|n| n.checked_add(count.checked_mul(2048)?)).ok_or_else(|| bad("selection mask output overflow"))?)
            .ok_or_else(|| bad("selection mask output overflow"))?;
        work =
            work.checked_add(pixels.checked_mul(128).ok_or_else(|| bad("selection mask work overflow"))?).ok_or_else(|| bad("selection mask work overflow"))?;
    }
    if peak > MAX_CHECKED_RASTER_BYTES || work > 8_000_000_000 {
        return Err(bad("mask retained/output/work limit"));
    }
    poll(control)
}
fn constant_mask(doc: &Document, value: f32) -> Result<LayerMask> {
    let fmt = photocraft_color::PixelFormat::new(ColorMode::Grayscale, doc.depth, false);
    let mut raw = [0u8; 8];
    photocraft_color::write_sample(&mut raw, doc.depth, 0, value);
    let bytes = raw.get(..fmt.bytes_per_pixel()).ok_or_else(|| bad("mask sample width"))?;
    Ok(LayerMask {
        surface: Arc::new(Surface::try_with_default_bytes(fmt, bytes).map_err(|e| bad(e.to_string()))?),
        enabled: true,
        linked: true,
        density: 1.0,
        feather: 0.0,
    })
}
fn coverage(mask: &LayerMask, x: i32, y: i32, wide: bool) -> Result<f32> {
    if !mask.enabled {
        return Ok(1.0);
    }
    if wide {
        let mut sample = [0.0f64];
        mask.surface.try_read_pixel_f64(x, y, &mut sample).map_err(|e| bad(e.to_string()))?;
        match sample[0] {
            0.0 => Ok(0.0),
            1.0 => Ok(1.0),
            _ => Err(bad("fractional/nonfinite wide Apply is not qualified")),
        }
    } else {
        let mut raw = [0u8; 8];
        let bytes = raw.get_mut(..mask.surface.format().bytes_per_pixel()).ok_or_else(|| bad("mask sample width"))?;
        mask.surface.try_read_pixel_bytes(x, y, bytes).map_err(|e| bad(e.to_string()))?;
        let value = photocraft_color::read_sample(bytes, mask.surface.format().sample, 0);
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(bad("mask coverage is not finite unit"));
        }
        Ok(if mask.density < 1.0 { 1.0 - mask.density * (1.0 - value) } else { value })
    }
}
#[derive(Clone, Copy)]
enum NativeWeight {
    Identity,
    Zero,
    U32(u32),
    F64(f64),
}
fn native_weight(sample: SampleType, mask: &LayerMask, x: i32, y: i32) -> Result<NativeWeight> {
    if !mask.enabled {
        return Ok(NativeWeight::Identity);
    }
    let mask_sample = mask.surface.format().sample;
    let mut raw = [0u8; 8];
    let encoded = raw.get_mut(..mask_sample.bytes()).ok_or_else(|| bad("mask width"))?;
    mask.surface.try_read_pixel_bytes(x, y, encoded).map_err(|e| bad(e.to_string()))?;
    if sample == SampleType::U32 && mask_sample == SampleType::U32 {
        if mask.density != 1.0 {
            return Err(bad("U32 rational Apply requires density one"));
        }
        let code = u32::from_ne_bytes((&*encoded).try_into().map_err(|_| bad("U32 mask width"))?);
        return Ok(match code {
            0 => NativeWeight::Zero,
            u32::MAX => NativeWeight::Identity,
            _ => NativeWeight::U32(code),
        });
    }
    let coverage = photocraft_color::try_read_sample_f64(encoded, mask_sample, 0).map_err(|e| bad(e.to_string()))?;
    if !coverage.is_finite() || !(0.0..=1.0).contains(&coverage) {
        return Err(bad("mask coverage is not finite unit"));
    }
    let effective = if sample == SampleType::F64 && mask_sample == SampleType::F64 {
        // Preserve the existing density interpolation. Each binary64 operation
        // is explicit and unfused; color components never enter arithmetic.
        let inverse = 1.0f64 - coverage;
        let attenuated = f64::from(mask.density) * inverse;
        1.0f64 - attenuated
    } else {
        if mask.density != 1.0 {
            return Err(bad("cross-depth native coverage requires density one"));
        }
        if coverage != 0.0 && coverage != 1.0 {
            return Err(bad("cross-depth fractional native coverage is unsupported"));
        }
        coverage
    };
    Ok(if effective == 0.0 {
        NativeWeight::Zero
    } else if effective == 1.0 {
        NativeWeight::Identity
    } else {
        NativeWeight::F64(effective)
    })
}
fn apply_native_alpha(pixel: &mut [u8], format: photocraft_color::PixelFormat, mask: &LayerMask, x: i32, y: i32) -> Result<()> {
    let weight = native_weight(format.sample, mask, x, y)?;
    if matches!(weight, NativeWeight::Identity) {
        return Ok(());
    }
    if !format.alpha {
        return Err(bad("native Apply requires alpha"));
    }
    let start = format.channels().checked_sub(1).and_then(|n| n.checked_mul(format.sample.bytes())).ok_or_else(|| bad("alpha offset"))?;
    let alpha = pixel.get_mut(start..).ok_or_else(|| bad("alpha range"))?;
    match weight {
        NativeWeight::Identity => {}
        NativeWeight::Zero => alpha.fill(0),
        NativeWeight::U32(coverage) => {
            let original = u32::from_ne_bytes((&*alpha).try_into().map_err(|_| bad("U32 alpha width"))?);
            let numerator = u64::from(original)
                .checked_mul(u64::from(coverage))
                .and_then(|n| n.checked_add(u64::from(u32::MAX) / 2))
                .ok_or_else(|| bad("U32 alpha product overflow"))?;
            let result = u32::try_from(numerator / u64::from(u32::MAX)).map_err(|_| bad("U32 alpha result overflow"))?;
            alpha.copy_from_slice(&result.to_ne_bytes());
        }
        NativeWeight::F64(coverage) => {
            if format.sample != SampleType::F64 {
                return Err(bad("fractional native Apply requires same-depth F64"));
            }
            let original = f64::from_ne_bytes((&*alpha).try_into().map_err(|_| bad("F64 alpha width"))?);
            if !original.is_finite() || !(0.0..=1.0).contains(&original) {
                return Err(bad("fractional F64 Apply requires finite unit alpha"));
            }
            let result = original * coverage;
            alpha.copy_from_slice(&result.to_ne_bytes());
        }
    }
    Ok(())
}
fn apply_surface(surface: &Surface, mask: &LayerMask, softened: Option<&Surface>, control: &Interrupt<'_>) -> Result<Surface> {
    poll(control)?;
    if !mask.enabled && matches!(surface.format().sample, SampleType::U32 | SampleType::F64) {
        return super::copy_storage(surface, control);
    }
    let format = surface.format();
    let channels = format.channels();
    let bpp = format.bytes_per_pixel();
    if channels == 0 || channels > 8 || bpp == 0 || !bpp.is_multiple_of(channels) {
        return Err(bad("invalid native pixel shape"));
    }
    let wide = matches!(format.sample, SampleType::U32 | SampleType::F64) || matches!(mask.surface.format().sample, SampleType::U32 | SampleType::F64);
    let area = surface.try_content_bounds_with_interrupt(control).map_err(|e| error(control, e))?;
    let mut output = Surface::try_with_default_bytes(format, surface.default_bytes()).map_err(|e| error(control, e))?;
    for cell in area.tiles() {
        poll(control)?;
        let part = cell.rect().intersect(&area);
        if part.is_empty() {
            continue;
        }
        let mut raw = surface.try_read_interleaved_with(part, control).map_err(|e| error(control, e))?;
        let width = usize::try_from(part.width()).map_err(|_| bad("mask row width"))?;
        for (index, pixel) in raw.chunks_exact_mut(bpp).enumerate() {
            if index % 256 == 0 {
                poll(control)?;
            }
            let x = i32::try_from(i64::from(part.x0) + (index % width) as i64).map_err(|_| bad("mask x overflow"))?;
            let y = i32::try_from(i64::from(part.y0) + (index / width) as i64).map_err(|_| bad("mask y overflow"))?;
            if wide {
                apply_native_alpha(pixel, format, mask, x, y)?;
            } else {
                let k = if let Some(softened) = softened {
                    let mut raw = [0u8; 8];
                    let encoded = raw.get_mut(..softened.format().bytes_per_pixel()).ok_or_else(|| bad("softened mask sample width"))?;
                    softened.try_read_pixel_bytes(x, y, encoded).map_err(|e| error(control, e))?;
                    let value = photocraft_color::read_sample(encoded, softened.format().sample, 0);
                    if !value.is_finite() {
                        return Err(bad("softened mask coverage is not finite"));
                    }
                    value
                } else {
                    coverage(mask, x, y, false)?
                };
                let mut values = [0.0f32; 8];
                let values = values.get_mut(..channels).ok_or_else(|| bad("pixel channels"))?;
                for (i, v) in values.iter_mut().enumerate() {
                    *v = photocraft_color::read_sample(pixel, format.sample, i);
                    if !v.is_finite() {
                        return Err(bad("ordinary nonfinite Apply is unsupported"));
                    }
                }
                let mut rgba = photocraft_raster::to_rgba(&format, values);
                rgba[3] *= k;
                let mut encoded = [0.0f32; 8];
                let n = photocraft_raster::from_rgba_into(&format, rgba, &mut encoded);
                if n != channels {
                    return Err(bad("pixel encoding mismatch"));
                }
                for (i, value) in encoded.get(..n).ok_or_else(|| bad("pixel encoding"))?.iter().enumerate() {
                    photocraft_color::write_sample(pixel, format.sample, i, *value);
                }
            }
        }
        output.try_write_interleaved_with(part, &raw, control).map_err(|e| error(control, e))?;
    }
    output.try_prune_with(control).map_err(|e| error(control, e))?;
    poll(control)?;
    Ok(output)
}
fn feathered_mask(doc: &Document, id: LayerId, control: &Interrupt<'_>) -> Result<Option<Surface>> {
    let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if !layer.mask.as_ref().is_some_and(|mask| mask.enabled && photocraft_compose::masks::feather_sigma(mask.feather) > 0.0) {
        return Ok(None);
    }
    poll(control)?;
    // This is the precise upstream ordinary fallback, after aggregate admission.
    // The compositor still has a boundary-only Interrupt seam here. No independent
    // vector mask is baked, and the original off-canvas union is preserved.
    let mut mask_only = layer.clone();
    mask_only.vector_mask = None;
    let source = layer.surface().ok_or_else(|| bad("not a resident raster"))?;
    let area = doc.bounds().union(&source.try_content_bounds_with_interrupt(control).map_err(|e| error(control, e))?);
    let result = photocraft_compose::masks::combined_mask(&mask_only, area);
    poll(control)?;
    Ok(result)
}
fn validate_apply_coverage(doc: &Document, ids: &[LayerId], control: &Interrupt<'_>) -> Result<()> {
    for id in ids {
        poll(control)?;
        let layer = doc.layer(*id).ok_or(EngineError::NoLayer(*id))?;
        let mask = layer.mask.as_ref().ok_or_else(|| bad("mask disappeared"))?;
        if !mask.enabled {
            continue;
        }
        let source = layer.surface().ok_or_else(|| bad("not a resident raster"))?;
        let wide =
            matches!(source.format().sample, SampleType::U32 | SampleType::F64) || matches!(mask.surface.format().sample, SampleType::U32 | SampleType::F64);
        let fractional = source.format().sample == mask.surface.format().sample && matches!(source.format().sample, SampleType::U32 | SampleType::F64);
        let default = photocraft_color::try_read_sample_f64(mask.surface.default_bytes(), mask.surface.format().sample, 0).map_err(|e| bad(e.to_string()))?;
        if !default.is_finite() || !(0.0..=1.0).contains(&default) || (wide && !fractional && default != 0.0 && default != 1.0) {
            return Err(bad("mask default coverage is not admitted"));
        }
        // Validate every allocated raw mask sample before candidate cloning. No
        // stored fractional wide coverage is silently narrowed, even off-canvas.
        for (cell, _) in mask.surface.tiles() {
            poll(control)?;
            let rect = cell.rect();
            let bytes = mask.surface.try_read_interleaved_with(rect, control).map_err(|e| error(control, e))?;
            for (index, raw) in bytes.chunks_exact(mask.surface.format().bytes_per_pixel()).enumerate() {
                if index % 256 == 0 {
                    poll(control)?;
                }
                let value = photocraft_color::try_read_sample_f64(raw, mask.surface.format().sample, 0).map_err(|e| bad(e.to_string()))?;
                if !value.is_finite() || !(0.0..=1.0).contains(&value) || (wide && !fractional && value != 0.0 && value != 1.0) {
                    return Err(bad("cross-depth fractional Apply or invalid unit coverage"));
                }
            }
        }
    }
    poll(control)
}
fn inverse_selection(doc: &Document, control: &Interrupt<'_>) -> Result<LayerMask> {
    let selection = doc.selection.as_ref().ok_or_else(|| bad("no selection"))?;
    let sample = selection.format().sample;
    let wide = matches!(sample, SampleType::U32 | SampleType::F64);
    let target = photocraft_color::PixelFormat::new(ColorMode::Grayscale, if wide { sample } else { SampleType::U8 }, false);
    let mut default = [0u8; 8];
    photocraft_color::write_sample(&mut default, target.sample, 0, 1.0);
    let mut output =
        Surface::try_with_default_bytes(target, default.get(..target.bytes_per_pixel()).ok_or_else(|| bad("mask width"))?).map_err(|e| error(control, e))?;
    let bounds = doc.bounds();
    for cell in bounds.tiles() {
        poll(control)?;
        let part = cell.rect().intersect(&bounds);
        if part.is_empty() {
            continue;
        }
        let source = selection.try_read_interleaved_with(part, control).map_err(|e| error(control, e))?;
        let pixels = source.len().checked_div(sample.bytes()).ok_or_else(|| bad("selection sample width"))?;
        let length = pixels.checked_mul(target.bytes_per_pixel()).ok_or_else(|| bad("selection mask length"))?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(length).map_err(|_| bad("selection mask allocation"))?;
        raw.resize(length, 0);
        for (i, (input, output)) in source.chunks_exact(sample.bytes()).zip(raw.chunks_exact_mut(target.bytes_per_pixel())).enumerate() {
            if i % 256 == 0 {
                poll(control)?;
            }
            match sample {
                // The storage code is already normalized to [0, MAX]. Invert
                // that code directly; neither f32 nor f64 quantization is used.
                SampleType::U32 => {
                    let code = u32::from_ne_bytes(input.try_into().map_err(|_| bad("U32 selection sample width"))?);
                    output.copy_from_slice(&(u32::MAX - code).to_ne_bytes());
                }
                // Caller-visible native arithmetic: ordinary, unfused binary64
                // subtraction, not an imported-operation or transfer assumption.
                SampleType::F64 => {
                    let value = f64::from_ne_bytes(input.try_into().map_err(|_| bad("F64 selection sample width"))?);
                    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                        return Err(bad("selection coverage is not finite unit"));
                    }
                    output.copy_from_slice(&(1.0f64 - value).to_ne_bytes());
                }
                _ => {
                    let value = photocraft_color::read_sample(input, sample, 0);
                    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                        return Err(bad("selection coverage is not finite unit"));
                    }
                    photocraft_color::write_sample(output, target.sample, 0, 1.0 - value);
                }
            }
        }
        output.try_write_interleaved_with(part, &raw, control).map_err(|e| error(control, e))?;
    }
    output.try_prune_with(control).map_err(|e| error(control, e))?;
    poll(control)?;
    Ok(LayerMask { surface: Arc::new(output), enabled: true, linked: true, density: 1.0, feather: 0.0 })
}
fn extract_alpha(source: &Surface, control: &Interrupt<'_>) -> Result<(Surface, LayerMask)> {
    poll(control)?;
    let format = source.format();
    let channels = format.channels();
    let sample = format.sample;
    let bpp = format.bytes_per_pixel();
    if channels == 0 || channels > 8 || bpp == 0 || sample.bytes() == 0 {
        return Err(bad("invalid alpha pixel shape"));
    }
    let wide = matches!(sample, SampleType::U32 | SampleType::F64);
    let mask_format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, if wide { sample } else { SampleType::U8 }, false);
    let zero = [0u8; 8];
    let mut mask = Surface::try_with_default_bytes(mask_format, zero.get(..mask_format.bytes_per_pixel()).ok_or_else(|| bad("alpha mask width"))?)
        .map_err(|e| error(control, e))?;
    let mut output = Surface::try_with_default_bytes(format, source.default_bytes()).map_err(|e| error(control, e))?;
    let area = source.try_content_bounds_with_interrupt(control).map_err(|e| error(control, e))?;
    for cell in area.tiles() {
        poll(control)?;
        let part = cell.rect().intersect(&area);
        if part.is_empty() {
            continue;
        }
        let mut raw = source.try_read_interleaved_with(part, control).map_err(|e| error(control, e))?;
        let count = raw.len() / bpp;
        let length = count.checked_mul(mask_format.bytes_per_pixel()).ok_or_else(|| bad("alpha mask length"))?;
        let mut coverage = Vec::new();
        coverage.try_reserve_exact(length).map_err(|_| bad("alpha mask allocation"))?;
        coverage.resize(length, 0);
        for (index, (pixel, alpha)) in raw.chunks_exact_mut(bpp).zip(coverage.chunks_exact_mut(mask_format.bytes_per_pixel())).enumerate() {
            if index % 256 == 0 {
                poll(control)?;
            }
            if wide {
                if format.alpha {
                    let offset = channels.checked_sub(1).and_then(|n| n.checked_mul(sample.bytes())).ok_or_else(|| bad("alpha offset"))?;
                    let input = pixel.get(offset..).ok_or_else(|| bad("alpha range"))?;
                    let value = photocraft_color::try_read_sample_f64(input, sample, 0).map_err(|e| bad(e.to_string()))?;
                    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                        return Err(bad("native alpha is not finite unit"));
                    }
                    if input.len() != alpha.len() {
                        return Err(bad("alpha width mismatch"));
                    }
                    alpha.copy_from_slice(input);
                    photocraft_color::try_write_sample_f64(pixel, sample, channels - 1, 1.0).map_err(|e| bad(e.to_string()))?;
                } else {
                    photocraft_color::try_write_sample_f64(alpha, sample, 0, 1.0).map_err(|e| bad(e.to_string()))?;
                }
            } else {
                let mut values = [0f32; 8];
                let values = values.get_mut(..channels).ok_or_else(|| bad("alpha channels"))?;
                for (i, value) in values.iter_mut().enumerate() {
                    *value = photocraft_color::read_sample(pixel, sample, i);
                    if !value.is_finite() {
                        return Err(bad("nonfinite ordinary alpha extraction"));
                    }
                }
                let mut rgba = photocraft_raster::to_rgba(&format, values);
                if !(0.0..=1.0).contains(&rgba[3]) {
                    return Err(bad("alpha is not unit coverage"));
                }
                photocraft_color::write_sample(alpha, SampleType::U8, 0, rgba[3]);
                rgba[3] = 1.0;
                let mut encoded = [0f32; 8];
                let n = photocraft_raster::from_rgba_into(&format, rgba, &mut encoded);
                if n != channels {
                    return Err(bad("alpha encoding mismatch"));
                }
                for (i, value) in encoded.get(..n).ok_or_else(|| bad("alpha encoding"))?.iter().enumerate() {
                    photocraft_color::write_sample(pixel, sample, i, *value);
                }
            }
        }
        output.try_write_interleaved_with(part, &raw, control).map_err(|e| error(control, e))?;
        mask.try_write_interleaved_with(part, &coverage, control).map_err(|e| error(control, e))?;
    }
    output.try_prune_with(control).map_err(|e| error(control, e))?;
    mask.try_prune_with(control).map_err(|e| error(control, e))?;
    poll(control)?;
    Ok((output, LayerMask { surface: Arc::new(mask), enabled: true, linked: true, density: 1.0, feather: 0.0 }))
}
fn stage(doc: &Document, ids: &[LayerId], action: Action, control: &Interrupt<'_>) -> Result<Option<Document>> {
    admission(doc, ids, action, control)?;
    if action == Action::Apply {
        validate_apply_coverage(doc, ids, control)?;
    }
    let constant = if action == Action::HideSelection {
        Some(inverse_selection(doc, control)?)
    } else if action.adds() {
        Some(constant_mask(doc, if action == Action::Reveal { 1.0 } else { 0.0 })?)
    } else {
        None
    };
    let changed = ids.iter().any(|id| {
        doc.layer(*id).is_some_and(|l| match action {
            Action::Reveal | Action::Hide | Action::HideSelection => l.mask.as_ref() != constant.as_ref() || action == Action::HideSelection,
            Action::Delete | Action::Apply => l.mask.is_some(),
            Action::FromTransparency => true,
        })
    });
    if !changed {
        return Ok(None);
    }
    poll(control)?;
    let mut next = doc.clone();
    for id in ids {
        poll(control)?;
        if action == Action::FromTransparency {
            let source = raster_domain(doc, *id)?;
            let (surface, mask) = extract_alpha(source, control)?;
            let target = next.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?;
            target.content = LayerContent::Raster(surface);
            target.mask = Some(mask);
        } else if action == Action::Apply {
            let source = doc.layer(*id).ok_or(EngineError::NoLayer(*id))?;
            let mask = source.mask.as_ref().ok_or_else(|| bad("mask disappeared"))?;
            let surface = source.surface().ok_or_else(|| bad("raster disappeared"))?;
            let softened = feathered_mask(doc, *id, control)?;
            let result = apply_surface(surface, mask, softened.as_ref(), control)?;
            let target = next.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?;
            target.content = LayerContent::Raster(result);
            target.mask = None;
        } else {
            if action.adds() {
                crate::extra_cmds::background_to_layer_for_mask(&mut next, *id);
            }
            next.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?.mask = constant.clone();
            if let Some(state) = &mut next.xcf_state {
                for item in &mut state.items {
                    poll(control)?;
                    if let Some(mask) = &mut item.mask
                        && mask.owner == ItemRef::Layer(*id)
                        && action.adds()
                    {
                        mask.outside_f64_bits = (if matches!(action, Action::Reveal | Action::HideSelection) { 1.0f64 } else { 0.0 }).to_bits();
                    }
                }
            }
        }
    }
    if action == Action::HideSelection {
        next.selection = None;
    }
    if action == Action::FromTransparency {
        if let Some(state) = &mut next.xcf_state {
            for item in &mut state.items {
                poll(control)?;
                if let Some(mask) = &mut item.mask
                    && matches!(mask.owner,ItemRef::Layer(id) if ids.contains(&id))
                {
                    mask.outside_f64_bits = 0.0f64.to_bits();
                    mask.enabled = true;
                    mask.linked = true;
                }
            }
        }
        if next == *doc {
            return Ok(None);
        }
    }
    // Session reconciliation removes only current typed mask relationships and
    // their dangling selections/sets; immutable retained source objects survive.
    poll(control)?;
    Ok(Some(next))
}
pub(super) fn dispatch_preflight(session: &Session, params: &Value, action: Action) -> Result<()> {
    request(session, params, action).map(|_| ())
}
pub(super) fn run(session: &mut Session, params: &Value, action: Action) -> Result<Value> {
    let ids = request(session, params, action)?;
    let st = session.active().ok_or(EngineError::NoDocument)?;
    let snapshot = st.doc.clone();
    let worker = snapshot.clone();
    let revision = st.revision;
    let active = st.active_layer;
    let mut selected = Vec::new();
    selected.try_reserve_exact(st.selected_layers.len()).map_err(|_| bad("selection capture allocation"))?;
    selected.extend_from_slice(&st.selected_layers);
    let weak = snapshot.downgrade();
    let label = action.label();
    crate::jobs::run(
        session,
        label,
        true,
        move |ctx| ctx.stage(0.0, 0.95, label, |ctl| Ok((stage(&worker, &ids, action, ctl)?, ctx.clone()))),
        move |session, (candidate, ctx)| {
            ctx.stage(0.95, 1.0, label, |control| {
                poll(control)?;
                let st = session.active().ok_or(EngineError::NoDocument)?;
                if st.doc.id != snapshot.id
                    || st.revision != revision
                    || !st.doc.shares_state(&snapshot)
                    || !weak.is_current(&st.doc)
                    || st.active_layer != active
                    || st.selected_layers != selected
                    || st.floating.is_some()
                    || st.coalesce.is_some()
                    || session.command_has_job_on(snapshot.id)
                    || session.command_has_pending_job()
                    || session.coalesce_request.is_some()
                    || session.color_restrict.is_some()
                {
                    return Err(bad("captured mask state changed"));
                }
                let Some(candidate) = candidate else { return Ok(Value::Null) };
                session.edit_with_interrupt(label, control, move |doc, _| {
                    poll(control)?;
                    *doc = candidate;
                    Ok(Value::Null)
                })
            })
        },
    )
}

// Meaningful transaction/raw-storage regressions drafted; intentionally unrun.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Layer, Size};
    use serde_json::json;
    fn fixture(sample: SampleType) -> (Document, Vec<LayerId>) {
        let mut doc = Document::new("Masks", Size::new(2, 1), ColorMode::Rgb, sample);
        let mut ids = Vec::new();
        for name in ["One", "Two"] {
            let layer = Layer::raster(name, doc.pixel_format());
            ids.push(layer.id);
            doc.layers.push(layer);
        }
        (doc, ids)
    }
    #[test]
    fn multi_target_constant_mask_native_depth_and_delete_do_not_touch_pixels_or_source() {
        for sample in [SampleType::U32, SampleType::F64] {
            let (original, ids) = fixture(sample);
            let hidden = stage(&original, &ids, Action::Hide, &Interrupt::NONE).unwrap().unwrap();
            for id in &ids {
                let old = original.layer(*id).unwrap();
                let current = hidden.layer(*id).unwrap();
                assert_eq!(current.content, old.content);
                let mask = current.mask.as_ref().unwrap();
                assert_eq!(mask.surface.format().sample, sample);
                assert_eq!(photocraft_color::try_read_sample_f64(mask.surface.default_bytes(), sample, 0).unwrap(), 0.0);
            }
            assert!(stage(&hidden, &ids, Action::Hide, &Interrupt::NONE).unwrap().is_none());
            let deleted = stage(&hidden, &ids, Action::Delete, &Interrupt::NONE).unwrap().unwrap();
            assert_eq!(deleted, original);
        }
    }
    #[test]
    fn wide_boolean_apply_copies_hidden_rgb_nan_and_negative_zero_and_only_clears_alpha() {
        let (mut doc, ids) = fixture(SampleType::F64);
        let id = ids[0];
        let raw =
            [0x7ff8_0000_0000_0042_u64, (-0.0f64).to_bits(), 1.25f64.to_bits(), 0.5f64.to_bits()].into_iter().flat_map(u64::to_le_bytes).collect::<Vec<_>>();
        let format = doc.pixel_format();
        let mask = constant_mask(&doc, 0.0).unwrap();
        let layer = doc.layer_mut(id).unwrap();
        layer.content = LayerContent::Raster(Surface::try_from_interleaved_with(format, Rect::new(-1, 0, 0, 1), &raw, &Interrupt::NONE).unwrap());
        layer.mask = Some(mask);
        let before = doc.clone();
        let applied = stage(&doc, &[id], Action::Apply, &Interrupt::NONE).unwrap().unwrap();
        let mut expected = raw.clone();
        expected[24..].fill(0);
        assert_eq!(applied.layer(id).unwrap().surface().unwrap().to_interleaved(Rect::new(-1, 0, 0, 1)), expected);
        assert!(applied.layer(id).unwrap().mask.is_none());
        assert_eq!(doc, before);
    }
    #[test]
    fn ordinary_feather_effect_vector_mask_and_offcanvas_match_upstream_apply_equations() {
        let (mut doc, ids) = fixture(SampleType::U16);
        let id = ids[0];
        let mut source = Surface::new(doc.pixel_format());
        source.fill_rect(Rect::new(-2, 0, 3, 1), &[0.2, 0.4, 0.7, 0.6]);
        let mut mask = constant_mask(&doc, 0.5).unwrap();
        mask.feather = 2.0;
        mask.density = 0.7;
        Arc::make_mut(&mut mask.surface).fill_rect(Rect::new(-1, 0, 1, 1), &[1.0]);
        let layer = doc.layer_mut(id).unwrap();
        layer.content = LayerContent::Raster(source.clone());
        layer.mask = Some(mask.clone());
        layer.effects.reference = Some((2.0, 3.0));
        layer.vector_mask = Some(photocraft_doc::VectorMask::new(photocraft_doc::Path::default()));
        let before = doc.clone();
        let layer = doc.layer(id).unwrap();
        let mut mask_only = layer.clone();
        mask_only.vector_mask = None;
        let region = source.content_bounds();
        let soft = photocraft_compose::masks::combined_mask(&mask_only, doc.bounds().union(&region)).unwrap();
        let fmt = source.format();
        let channels = fmt.channels();
        let mut expected = source.clone();
        let mut result = Vec::new();
        let mut encoded = [0.0f32; 8];
        for (pixel, k) in source.read_region(region).chunks_exact(channels).zip(soft.read_region(region)) {
            let mut rgba = photocraft_raster::to_rgba(&fmt, pixel);
            rgba[3] *= k;
            let count = photocraft_raster::from_rgba_into(&fmt, rgba, &mut encoded);
            result.extend_from_slice(&encoded[..count]);
        }
        expected.write_region(region, &result);
        expected.prune();
        let after = stage(&doc, &[id], Action::Apply, &Interrupt::NONE).unwrap().unwrap();
        let current = after.layer(id).unwrap();
        assert_eq!(current.surface().unwrap(), &expected);
        assert_eq!(current.vector_mask, layer.vector_mask);
        assert_eq!(current.effects, layer.effects);
        assert_eq!(after.size, doc.size);
        assert!(current.mask.is_none());
        assert_eq!(doc, before);
    }
    #[test]
    fn hide_selection_canvas_clipping_default_and_boolean_wide_storage() {
        for sample in [SampleType::U16, SampleType::U32, SampleType::F64] {
            let (mut doc, ids) = fixture(sample);
            let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut selection = Surface::with_default(format, &[0.0]);
            selection.fill_rect(Rect::new(-1, 0, 1, 1), &[1.0]);
            doc.selection = Some(selection);
            let before = doc.clone();
            let after = stage(&doc, &ids, Action::HideSelection, &Interrupt::NONE).unwrap().unwrap();
            assert!(after.selection.is_none());
            for id in &ids {
                let mask = after.layer(*id).unwrap().mask.as_ref().unwrap();
                assert_eq!(mask.surface.sample_channel(-1, 0, 0), 1.0);
                assert_eq!(mask.surface.sample_channel(0, 0, 0), 0.0);
                assert_eq!(mask.surface.sample_channel(1, 0, 0), 1.0);
                assert_eq!(mask.surface.format().sample, if sample == SampleType::U16 { SampleType::U8 } else { sample });
                assert_eq!(after.layer(*id).unwrap().content, doc.layer(*id).unwrap().content);
            }
            assert_eq!(doc, before);
        }
    }
    #[test]
    fn hide_selection_fractional_u32_codes_and_f64_words_are_not_narrowed() {
        let (mut doc, _) = fixture(SampleType::U32);
        let fmt = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let codes = [1u32, u32::MAX - 2];
        let raw = codes.into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
        doc.selection = Some(Surface::try_from_interleaved_with(fmt, doc.bounds(), &raw, &Interrupt::NONE).unwrap());
        let mask = inverse_selection(&doc, &Interrupt::NONE).unwrap();
        let expected = codes.into_iter().map(|v| u32::MAX - v).flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
        assert_eq!(mask.surface.to_interleaved(doc.bounds()), expected);
        let fmt = fmt.with_sample(SampleType::F64);
        let values = [f64::from_bits(0.5f64.to_bits() + 1), f64::from_bits(1.0f64.to_bits() - 1)];
        let raw = values.into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
        doc.selection = Some(Surface::try_from_interleaved_with(fmt, doc.bounds(), &raw, &Interrupt::NONE).unwrap());
        let mask = inverse_selection(&doc, &Interrupt::NONE).unwrap();
        assert_eq!(mask.surface.to_interleaved(doc.bounds()), values.into_iter().map(|v| 1.0f64 - v).flat_map(f64::to_ne_bytes).collect::<Vec<_>>());
        assert_eq!(mask.surface.sample_channel(-1, 0, 0), 1.0);
        for invalid in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            let raw = [invalid, 0.5].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
            doc.selection = Some(Surface::try_from_interleaved_with(fmt, doc.bounds(), &raw, &Interrupt::NONE).unwrap());
            let before = doc.clone();
            assert!(inverse_selection(&doc, &Interrupt::NONE).is_err());
            assert_eq!(doc, before);
        }
    }
    #[test]
    fn native_alpha_extraction_keeps_low_bits_negative_zero_rgb_and_offcanvas_storage() {
        let format = photocraft_color::PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
        let input = [
            0x7ff8_0000_0000_0042u64,
            (-0.0f64).to_bits(),
            1.25f64.to_bits(),
            (-0.0f64).to_bits(),
            0.2f64.to_bits(),
            0.3f64.to_bits(),
            0.4f64.to_bits(),
            0.5f64.to_bits() + 1,
        ]
        .into_iter()
        .flat_map(u64::to_ne_bytes)
        .collect::<Vec<_>>();
        let area = Rect::new(-1, 0, 1, 1);
        let surface = Surface::try_from_interleaved_with(format, area, &input, &Interrupt::NONE).unwrap();
        let (opaque, mask) = extract_alpha(&surface, &Interrupt::NONE).unwrap();
        let alpha = [(-0.0f64).to_bits(), 0.5f64.to_bits() + 1].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        assert_eq!(mask.surface.to_interleaved(area), alpha);
        assert_eq!(mask.surface.default_bytes(), 0.0f64.to_ne_bytes());
        let mut expected = input.clone();
        for pixel in expected.as_chunks_mut::<32>().0.iter_mut() {
            pixel[24..].copy_from_slice(&1.0f64.to_ne_bytes());
        }
        assert_eq!(opaque.to_interleaved(area), expected);
        assert_eq!(opaque.default_bytes(), surface.default_bytes());
        assert_eq!(surface.to_interleaved(area), input);
        let cancelled = || true;
        assert!(matches!(extract_alpha(&surface, &Interrupt::cancel_only(&cancelled)), Err(EngineError::Cancelled)));
    }
    #[test]
    fn fractional_native_apply_rounds_u32_nearest_and_preserves_f64_low_color_words() {
        let format = photocraft_color::PixelFormat::new(ColorMode::Rgb, SampleType::U32, true);
        let coverage = 333_333_333u32;
        let original = 123_456_789u32;
        let mask = LayerMask {
            surface: Arc::new(
                Surface::try_with_default_bytes(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false), &coverage.to_ne_bytes())
                    .unwrap(),
            ),
            enabled: true,
            linked: true,
            density: 1.0,
            feather: 0.0,
        };
        let mut pixel = [u32::MAX - 1, 1, 987_654_321, original].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
        let colors = pixel[..12].to_vec();
        apply_native_alpha(&mut pixel, format, &mask, 0, 0).unwrap();
        let expected = ((u128::from(original) * u128::from(coverage) + u128::from(u32::MAX) / 2) / u128::from(u32::MAX)) as u32;
        assert_eq!(&pixel[..12], colors);
        assert_eq!(&pixel[12..], expected.to_ne_bytes());
        let format = format.with_sample(SampleType::F64);
        let coverage = f64::from_bits(0.3f64.to_bits() + 1);
        let original = f64::from_bits(0.5f64.to_bits() + 7);
        let mask = LayerMask {
            surface: Arc::new(
                Surface::try_with_default_bytes(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &coverage.to_ne_bytes())
                    .unwrap(),
            ),
            enabled: true,
            linked: true,
            density: 0.7,
            feather: 0.0,
        };
        let mut pixel =
            [0x7ff8_0000_0000_0042u64, (-0.0f64).to_bits(), 1.25f64.to_bits(), original.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let colors = pixel[..24].to_vec();
        apply_native_alpha(&mut pixel, format, &mask, 0, 0).unwrap();
        let expected = original * (1.0f64 - f64::from(mask.density) * (1.0f64 - coverage));
        assert_eq!(&pixel[..24], colors);
        assert_eq!(&pixel[24..], expected.to_ne_bytes());
        let mut invalid = [0u64, 0, 0, f64::NAN.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let before = invalid.clone();
        assert!(apply_native_alpha(&mut invalid, format, &mask, 0, 0).is_err());
        assert_eq!(invalid, before);
    }
    #[test]
    fn fractional_wide_and_cancel_refuse_and_delete_noop_retains_real_redo() {
        let (mut doc, ids) = fixture(SampleType::U32);
        let id = ids[0];
        let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let fractional = LayerMask {
            surface: Arc::new(Surface::try_with_default_bytes(format, &0.5f64.to_ne_bytes()).unwrap()),
            enabled: true,
            linked: true,
            density: 1.0,
            feather: 0.0,
        };
        doc.layer_mut(id).unwrap().mask = Some(fractional);
        let before = doc.clone();
        assert!(stage(&doc, &[id], Action::Apply, &Interrupt::NONE).is_err());
        assert_eq!(doc, before);
        let cancelled = || true;
        assert!(matches!(stage(&doc, &ids, Action::Delete, &Interrupt::cancel_only(&cancelled)), Err(EngineError::Cancelled)));
        let (doc, ids) = fixture(SampleType::U32);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
            .edit("Seed", |doc, _| {
                doc.name = "Seed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", Value::Null).unwrap();
        let before = session.active().unwrap().doc.clone();
        run(&mut session, &json!({"layer":ids[0].0}), Action::Delete).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert!(session.active().unwrap().history.can_redo());
        let st = session.active().unwrap();
        assert!(
            dispatch_preflight(
                &session,
                &json!({"layer":ids[0].0,"documentId":st.doc.id.0.to_string(),"expectedRevision":(st.revision+1).to_string()}),
                Action::Reveal
            )
            .is_err()
        );
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
}
