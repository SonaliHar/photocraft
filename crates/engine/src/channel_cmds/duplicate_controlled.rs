//! Frozen channel duplication; raw same-format copies are not display composites.
//! Ordinary extraction reuses controlled source primitives and existing f32
//! luminosity/conversion order. Wide composite/color extraction is unsupported.
use super::{
    ChanRef, bad, channel_format, color_names, composite_name, edit_mode, parameter_ref, read_plane_with_interrupt, source_dimensions, source_reference,
};
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::item_identity::{ChannelId, FilterId, ItemRef};
use photocraft_doc::{AlphaChannel, Document, Layer, LayerContent, LayerId};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
use std::collections::BTreeMap;
const CMD: &str = "channel.duplicate";
const NAME_BYTES: usize = 2048;
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn error(ctl: &Interrupt<'_>, e: impl std::fmt::Display) -> EngineError {
    if ctl.cancelled() { EngineError::Cancelled } else { bad(CMD, e.to_string()) }
}
#[derive(Clone, Copy)]
enum Destination {
    Existing(usize),
    New,
}
#[derive(Clone, Copy)]
struct Request {
    reference: ChanRef,
    destination: Destination,
    invert: bool,
    source_id: Option<ChannelId>,
    metadata: bool,
}
fn metadata_shape(doc: &Document) -> Result<()> {
    if let Some(state) = &doc.xcf_state {
        let count = [state.items.len(), state.source_items.len(), state.filter_stacks.len(), state.filter_masks.len(), state.opaque.len()]
            .into_iter()
            .try_fold(0usize, |count, length| count.checked_add(length))
            .filter(|count| *count <= 100000)
            .ok_or_else(|| bad(CMD, "current channel metadata count limit"))?;
        let _ = count;
    }
    Ok(())
}
fn decimal(value: &Value, nonzero: bool) -> Result<u64> {
    let text = value
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= 20 && text.bytes().all(|b| b.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0')))
        .ok_or_else(|| bad(CMD, "captured IDs must be canonical decimal strings"))?;
    let n = text.parse().map_err(|_| bad(CMD, "captured ID exceeds range"))?;
    if nonzero && n == 0 {
        return Err(bad(CMD, "documentId must be nonzero"));
    }
    Ok(n)
}
fn request(session: &Session, params: &Value) -> Result<Request> {
    let object = params
        .as_object()
        .filter(|object| {
            object.len() <= 7
                && object.keys().all(|key| matches!(key.as_str(), "channel" | "channelId" | "name" | "invert" | "document" | "documentId" | "expectedRevision"))
        })
        .ok_or_else(|| bad(CMD, "expected bounded channel duplicate parameters"))?;
    let _ = object;
    let source = session.active().ok_or(EngineError::NoDocument)?;
    if source.doc.channels.len() > 10000 || session.documents().len() > 4096 {
        return Err(bad(CMD, "channel/document count limit"));
    }
    metadata_shape(&source.doc)?;
    if params.get("channelId").is_some() && params.get("channel").is_some() {
        return Err(bad(CMD, "supply exactly one channel selector"));
    }
    let reference = if let Some(id) = params.get("channelId") {
        if super::channel_identity(id).is_none() {
            return Err(bad(CMD, "invalid channel identity"));
        }
        super::parse_ref(params, &source.doc).ok_or_else(|| bad(CMD, "unknown channel identity"))?
    } else {
        source_reference(parameter_ref(params, "channel").ok_or_else(|| bad(CMD, "missing channel"))?, &source.doc, CMD)?
    };
    let invert = match params.get("invert") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(|| bad(CMD, "invert must be boolean"))?,
    };
    if let Some(name) = params.get("name")
        && !name.as_str().is_some_and(|name| name.len() <= NAME_BYTES && !name.contains('\0'))
    {
        return Err(bad(CMD, "name must be bounded UTF-8 without NUL"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(doc), Some(rev)) if decimal(doc, true)? == source.doc.id.0 && decimal(rev, false)? == source.revision => {}
        _ => return Err(bad(CMD, "captured source changed or incomplete")),
    }
    let destination = match params.get("document") {
        None | Some(Value::Null) => Destination::Existing(session.active_index().ok_or(EngineError::NoDocument)?),
        Some(Value::String(value)) if value == "new" => Destination::New,
        Some(value) => {
            let number = value.as_f64().filter(|n| n.is_finite() && *n >= 0.).ok_or_else(|| bad(CMD, "destination must be a document index or new"))?.round();
            if number >= session.documents().len() as f64 {
                return Err(bad(CMD, "destination document index out of range"));
            }
            Destination::Existing(number as usize)
        }
    };
    let source_id = match reference {
        ChanRef::Alpha(index) => Some(source.doc.channels.get(index).ok_or_else(|| bad(CMD, "channel index out of range"))?.id),
        _ => None,
    };
    if let Some(id) = source_id
        && (id.0 == 0 || source.doc.channels.iter().filter(|channel| channel.id == id).count() != 1)
    {
        return Err(bad(CMD, "ambiguous source channel identity"));
    }
    let metadata =
        source_id.is_some_and(|id| source.doc.xcf_state.as_ref().is_some_and(|state| state.items.iter().any(|item| item.item == ItemRef::Channel(id))));
    if metadata && !matches!(destination,Destination::Existing(index) if Some(index)==session.active_index()) {
        return Err(bad(CMD, "imported channel metadata requires same-image duplication"));
    }
    let selected = match reference {
        ChanRef::Alpha(index) => Some(&source.doc.channels.get(index).ok_or_else(|| bad(CMD, "channel index"))?.surface),
        ChanRef::QuickMask => Some(&source.doc.quick_mask.as_ref().ok_or_else(|| bad(CMD, "not in Quick Mask mode"))?.surface),
        _ => None,
    };
    if let Some(surface) = selected {
        if surface.format().mode != ColorMode::Grayscale || surface.format().alpha {
            return Err(bad(CMD, "saved native channel requires Gray without alpha"));
        }
    } else if matches!(source.doc.depth, SampleType::U32 | SampleType::F64) || source.doc.xcf_state.is_some() {
        return Err(bad(CMD, "wide/imported composite extraction is not qualified"));
    }
    for state in std::iter::once(source).chain(match destination {
        Destination::Existing(index) => session.documents().get(index),
        Destination::New => None,
    }) {
        if state.floating.is_some()
            || state.coalesce.is_some()
            || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
            || state.doc.accounted().is_some()
            || session.command_has_job_on(state.doc.id)
        {
            return Err(bad(CMD, "selected document has transient or backed state"));
        }
    }
    if session.has_jobs() || session.command_has_pending_job() || session.coalesce_request.is_some() || session.color_restrict.is_some() {
        return Err(bad(CMD, "finish jobs or gestures before channel duplication"));
    }
    if let Destination::Existing(index) = destination {
        let target = session.documents().get(index).ok_or_else(|| bad(CMD, "destination disappeared"))?;
        metadata_shape(&target.doc)?;
        if target.doc.size != source.doc.size || target.revision == u64::MAX || target.doc.channels.len() >= 10000 {
            return Err(bad(CMD, "destination dimensions/revision/channel limit"));
        }
        if let Some(surface) = selected
            && surface.format() != channel_format(&target.doc)
            && (matches!(surface.format().sample, SampleType::U32 | SampleType::F64) || matches!(target.doc.depth, SampleType::U32 | SampleType::F64))
        {
            return Err(bad(CMD, "wide cross-depth channel conversion is not qualified"));
        }
    } else if selected.is_some_and(|surface| {
        surface.format().sample != source.doc.depth
            && (matches!(surface.format().sample, SampleType::U32 | SampleType::F64) || matches!(source.doc.depth, SampleType::U32 | SampleType::F64))
    }) {
        return Err(bad(CMD, "wide new-image channel depth conversion is not qualified"));
    }
    if invert {
        invert_policy(&source.doc, source_id, &Interrupt::NONE)?;
    }
    Ok(Request { reference, destination, invert, source_id, metadata })
}
pub(super) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    request(session, params).map(|_| ())
}
fn copy_name(name: &str, suffix: bool, ctl: &Interrupt<'_>) -> Result<String> {
    poll(ctl)?;
    if name.contains('\0') {
        return Err(bad(CMD, "name contains NUL"));
    }
    let count = name.len().checked_add(if suffix { 5 } else { 0 }).filter(|n| *n <= NAME_BYTES).ok_or_else(|| bad(CMD, "channel name limit"))?;
    let mut result = String::new();
    result.try_reserve_exact(count).map_err(|_| bad(CMD, "name allocation"))?;
    if result.capacity() > NAME_BYTES + 64 {
        return Err(bad(CMD, "name capacity"));
    }
    result.push_str(name);
    if suffix {
        result.push_str(" copy");
    }
    poll(ctl)?;
    Ok(result)
}
fn selected_channel(doc: &Document, reference: ChanRef) -> Result<Option<&AlphaChannel>> {
    match reference {
        ChanRef::Alpha(index) => Ok(Some(doc.channels.get(index).ok_or_else(|| bad(CMD, "source channel disappeared"))?)),
        ChanRef::QuickMask => Ok(Some(doc.quick_mask.as_ref().ok_or_else(|| bad(CMD, "Quick Mask disappeared"))?)),
        _ => Ok(None),
    }
}
fn tiles(bounds: Rect) -> Result<u64> {
    if bounds.is_empty() {
        return Ok(0);
    }
    let w = (i64::from(bounds.x1) - i64::from(bounds.x0)) as u64;
    let h = (i64::from(bounds.y1) - i64::from(bounds.y0)) as u64;
    w.div_ceil(TILE_SIZE as u64).checked_mul(h.div_ceil(TILE_SIZE as u64)).ok_or_else(|| bad(CMD, "tile count overflow"))
}
#[cfg(test)]
fn admission(source: &Document, target: Option<&Document>, reference: ChanRef, ctl: &Interrupt<'_>) -> Result<()> {
    let prep = super::SourcePreparation::new(CMD, ctl)?;
    admission_prepared(source, target, reference, false, ctl, &prep)
}
fn admission_prepared(
    source: &Document,
    target: Option<&Document>,
    reference: ChanRef,
    inverted: bool,
    ctl: &Interrupt<'_>,
    prep: &super::SourcePreparation<'_, '_>,
) -> Result<()> {
    poll(ctl)?;
    let (area, pixels) = source_dimensions(source, CMD)?;
    let mut retained = 16u64 << 20;
    let mut work = 0u64;
    for doc in std::iter::once(source).chain(target) {
        poll(ctl)?;
        let (model, payload) = prep.census(doc)?;
        let raster = crate::DocumentSnapshot::resident_raster_footprint_of(doc, ctl)?;
        retained = retained
            .checked_add(payload)
            .and_then(|n| n.checked_add(model.checked_mul(5)?))
            .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
            .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(2)?))
            .ok_or_else(|| bad(CMD, "duplicate model/payload/COW overflow"))?;
        work = work
            .checked_add(payload.checked_add(model).and_then(|n| n.checked_mul(8)).ok_or_else(|| bad(CMD, "validation work overflow"))?)
            .ok_or_else(|| bad(CMD, "validation work overflow"))?;
    }
    if let Some(target) = target {
        let ids = target
            .channels
            .len()
            .checked_add(target.xcf_state.as_ref().map_or(0, |state| state.items.len()))
            .and_then(|n| n.checked_add(4))
            .ok_or_else(|| bad(CMD, "identity scan count"))? as u64;
        work = work
            .checked_add(ids.checked_mul(ids).and_then(|n| n.checked_mul(16)).ok_or_else(|| bad(CMD, "identity scan work"))?)
            .ok_or_else(|| bad(CMD, "identity work overflow"))?;
    }
    let canvas_tiles = tiles(area)?;
    let source_tiles = selected_channel(source, reference)?.map_or(canvas_tiles, |channel| channel.surface.tile_count() as u64);
    let output_tiles = source_tiles.checked_add(canvas_tiles).ok_or_else(|| bad(CMD, "output tile count"))?;
    let bpp = target.map_or(source.depth.bytes() * 2, |doc| channel_format(doc).bytes_per_pixel()) as u64;
    let native = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(bpp.max(16))).ok_or_else(|| bad(CMD, "tile scratch size"))?;
    retained = retained
        .checked_add(
            output_tiles
                .checked_mul(native.checked_add(1024).ok_or_else(|| bad(CMD, "map bytes"))?)
                .and_then(|n| n.checked_mul(3))
                .ok_or_else(|| bad(CMD, "native staging bytes"))?,
        )
        .and_then(|n| n.checked_add((pixels as u64).checked_mul(40)?))
        .and_then(|n| n.checked_add(native.checked_mul(8)?))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad(CMD, "source/target/channel staging limit"))?;
    work = work
        .checked_add(
            canvas_tiles
                .checked_mul(source_tiles.checked_add(output_tiles).ok_or_else(|| bad(CMD, "scheduler visits"))?)
                .and_then(|n| n.checked_mul(256))
                .ok_or_else(|| bad(CMD, "scheduler work"))?,
        )
        .and_then(|n| n.checked_add((pixels as u64).checked_mul(128)?))
        .filter(|n| *n <= 8_000_000_000)
        .ok_or_else(|| bad(CMD, "channel duplicate work limit"))?;
    prep.debit(work)?;
    let selected = selected_channel(source, reference)?;
    let source_format = selected.map_or(channel_format(source), |channel| channel.surface.format());
    // Distinct actual phases: extraction, optional inversion, then destination
    // conversion/new image. Do not hide the third output behind a two-tile sum.
    if selected.is_none() {
        prep.physical_output(canvas_tiles, source_format.bytes_per_pixel() as u64)?;
    }
    if inverted {
        prep.physical_output(canvas_tiles, source_format.bytes_per_pixel() as u64)?;
    }
    match target {
        None => prep.physical_output(canvas_tiles, (source.depth.bytes() * 2) as u64)?,
        Some(target) if channel_format(target) != source_format => {
            prep.physical_output(source_tiles.max(canvas_tiles), channel_format(target).bytes_per_pixel() as u64)?
        }
        Some(_) => {}
    }
    prep.admit_peak(retained)?;
    poll(ctl)
}

fn invert_policy(doc: &Document, id: Option<ChannelId>, ctl: &Interrupt<'_>) -> Result<()> {
    let Some(state) = &doc.xcf_state else { return Ok(()) };
    poll(ctl)?;
    let owner = ItemRef::Channel(id.ok_or_else(|| bad(CMD, "imported Quick Mask inversion lacks qualified current channel metadata"))?);
    let mut found = None;
    for item in &state.items {
        poll(ctl)?;
        if item.item == owner {
            if found.is_some() {
                return Err(bad(CMD, "duplicate current channel metadata"));
            }
            found = Some(item);
        }
    }
    let item = found.ok_or_else(|| bad(CMD, "missing current channel inversion metadata"))?;
    if item.source_flags != 0 || item.composition.is_some() || item.mask.is_some() || item.parent.is_some() || item.raster_bounds.is_none() {
        return Err(bad(CMD, "channel inversion source flags/geometry are not qualified"));
    }
    for source in &state.source_items {
        poll(ctl)?;
        if source.owner == owner {
            return Err(bad(CMD, "source-backed channel inversion is not qualified"));
        }
    }
    for stack in &state.filter_stacks {
        poll(ctl)?;
        if stack.owner == owner {
            return Err(bad(CMD, "retained channel filter inversion is not qualified"));
        }
    }
    for opaque in &state.opaque {
        poll(ctl)?;
        if opaque.owner == Some(owner) && opaque.rendering_required {
            return Err(bad(CMD, "opaque channel inversion is not qualified"));
        }
    }
    poll(ctl)
}
fn unit_surface(surface: &Surface, area: Rect, ctl: &Interrupt<'_>) -> Result<()> {
    for coordinate in area.tiles() {
        poll(ctl)?;
        let part = coordinate.rect().intersect(&area);
        let raw = surface.try_read_interleaved_with(part, ctl).map_err(|e| error(ctl, e))?;
        let width = surface.format().bytes_per_pixel();
        if width == 0 {
            return Err(bad(CMD, "channel sample width"));
        }
        for (index, pixel) in raw.chunks_exact(width).enumerate() {
            if index.is_multiple_of(4096) {
                poll(ctl)?;
            }
            let value = photocraft_color::try_read_sample_f64(pixel, surface.format().sample, 0).map_err(|e| error(ctl, e))?;
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(bad(CMD, "wide channel inversion requires finite unit coverage in the canvas"));
            }
        }
    }
    poll(ctl)
}
fn plane_surface_controlled(plane: &[f32], area: Rect, format: PixelFormat, ctl: &Interrupt<'_>) -> Result<Surface> {
    let count = usize::try_from(i64::from(area.x1) - i64::from(area.x0))
        .ok()
        .and_then(|w| w.checked_mul(usize::try_from(i64::from(area.y1) - i64::from(area.y0)).ok()?))
        .ok_or_else(|| bad(CMD, "channel plane count"))?;
    if plane.len() != count {
        return Err(bad(CMD, "channel plane dimensions"));
    }
    let length = count.checked_mul(format.bytes_per_pixel()).ok_or_else(|| bad(CMD, "channel bytes"))?;
    let mut raw = Vec::new();
    poll(ctl)?;
    raw.try_reserve_exact(length).map_err(|_| bad(CMD, "channel plane allocation"))?;
    if raw.capacity() as u64 > MAX_CHECKED_RASTER_BYTES {
        return Err(bad(CMD, "channel plane capacity"));
    }
    for (index, value) in plane.iter().enumerate() {
        if index.is_multiple_of(4096) {
            poll(ctl)?;
        }
        if !value.is_finite() {
            return Err(bad(CMD, "nonfinite channel plane"));
        }
        let mut encoded = [0u8; 40];
        let native = encoded.get_mut(..format.bytes_per_pixel()).ok_or_else(|| bad(CMD, "channel pixel size"))?;
        photocraft_raster::encode_pixel(&format, &[value.clamp(0.0, 1.0)], native);
        raw.extend_from_slice(native);
    }
    let mut surface = Surface::try_from_interleaved_with(format, area, &raw, ctl).map_err(|e| error(ctl, e))?;
    surface.try_prune_with(ctl).map_err(|e| error(ctl, e))?;
    poll(ctl)?;
    Ok(surface)
}
#[cfg(test)]
fn prepare_channel(source: &Document, active_layer: Option<LayerId>, request: Request, name: String, ctl: &Interrupt<'_>) -> Result<AlphaChannel> {
    let prep = super::SourcePreparation::new(CMD, ctl)?;
    prepare_channel_prepared(source, active_layer, request, name, ctl, &prep)
}
fn prepare_channel_prepared(
    source: &Document,
    active_layer: Option<LayerId>,
    request: Request,
    name: String,
    ctl: &Interrupt<'_>,
    prep: &super::SourcePreparation<'_, '_>,
) -> Result<AlphaChannel> {
    poll(ctl)?;
    let area = source.bounds();
    let mut channel = match selected_channel(source, request.reference)? {
        Some(channel) => AlphaChannel {
            id: channel.id,
            name,
            surface: channel.surface.clone(),
            spot: channel.spot,
            color: channel.color,
            opacity: channel.opacity,
            indicates: channel.indicates,
        },
        None => {
            let planes = super::source_from_snapshot_prepared(
                source,
                active_layer,
                super::SnapshotSource { layer: None, channel: request.reference, gray: true, invert: false },
                CMD,
                ctl,
                prep,
            )?;
            let plane = planes.first().filter(|_| planes.len() == 1).ok_or_else(|| bad(CMD, "expected one source plane"))?;
            AlphaChannel::new(name, plane_surface_controlled(plane, area, channel_format(source), ctl)?)
        }
    };
    if request.invert {
        invert_policy(source, request.source_id, ctl)?;
        if matches!(channel.surface.format().sample, SampleType::U32 | SampleType::F64) {
            unit_surface(&channel.surface, area, ctl)?;
            channel.surface = photocraft_algo::wide::try_invert(&channel.surface, area, None, ctl).map_err(|e| error(ctl, e))?;
        } else {
            let mut plane = read_plane_with_interrupt(&channel.surface, area, ctl)?;
            for chunk in plane.chunks_mut(4096) {
                poll(ctl)?;
                for value in chunk {
                    *value = 1.0 - *value;
                }
            }
            channel.surface = plane_surface_controlled(&plane, area, channel.surface.format(), ctl)?;
        }
    }
    poll(ctl)?;
    Ok(channel)
}
fn fresh_channel(doc: &Document, ctl: &Interrupt<'_>) -> Result<ChannelId> {
    let attempts = doc
        .channels
        .len()
        .checked_add(doc.xcf_state.as_ref().map_or(0, |state| state.items.len()))
        .and_then(|n| n.checked_add(4))
        .ok_or_else(|| bad(CMD, "channel identity attempts"))?;
    for _ in 0..attempts {
        poll(ctl)?;
        let id = ChannelId::fresh();
        if id.0 == 0 {
            continue;
        }
        let mut collision = doc.selection_channel_id == Some(id) || doc.quick_mask.as_ref().is_some_and(|channel| channel.id == id);
        for channel in &doc.channels {
            poll(ctl)?;
            if channel.id == id {
                collision = true;
                break;
            }
        }
        if let Some(state) = &doc.xcf_state {
            for item in &state.items {
                poll(ctl)?;
                if item.item == ItemRef::Channel(id) {
                    collision = true;
                    break;
                }
            }
        }
        if !collision {
            return Ok(id);
        }
    }
    Err(bad(CMD, "channel identities exhausted"))
}
fn metadata(doc: &mut Document, source: ChannelId, target: ChannelId, ctl: &Interrupt<'_>) -> Result<()> {
    let Some(mut state) = doc.xcf_state.take() else { return Err(bad(CMD, "current channel metadata disappeared")) };
    let result = (|| {
        let mut next = 0u64;
        let mut count = 0usize;
        for stack in &state.filter_stacks {
            poll(ctl)?;
            for node in &stack.nodes {
                poll(ctl)?;
                count = count.checked_add(1).filter(|n| *n <= 100000).ok_or_else(|| bad(CMD, "filter count limit"))?;
                next = next.max(node.id.0);
            }
        }
        let mut ids = BTreeMap::new();
        for stack in &state.filter_stacks {
            poll(ctl)?;
            if stack.owner != ItemRef::Channel(source) {
                continue;
            }
            for node in &stack.nodes {
                poll(ctl)?;
                next = next.checked_add(1).ok_or_else(|| bad(CMD, "filter identities exhausted"))?;
                if ids.insert(node.id, FilterId(next)).is_some() {
                    return Err(bad(CMD, "duplicate filter identities"));
                }
            }
        }
        state.clone_channel_metadata_with_interrupt(source, target, doc, &ids, ctl).map_err(|e| error(ctl, e))
    })();
    doc.xcf_state = Some(state);
    result?;
    poll(ctl)
}

fn new_image(source: &Document, channel: &AlphaChannel, ctl: &Interrupt<'_>) -> Result<Document> {
    poll(ctl)?;
    let mut document = Document::new(&channel.name, source.size, ColorMode::Grayscale, source.depth);
    document.set_physical_resolution(source.effective_resolution()).map_err(|e| error(ctl, e))?;
    let format = document.pixel_format();
    let area = source.bounds();
    let mut surface = Surface::try_with_default_bytes(format, [0u8; 40].get(..format.bytes_per_pixel()).ok_or_else(|| bad(CMD, "new channel pixel format"))?)
        .map_err(|e| error(ctl, e))?;
    if matches!(channel.surface.format().sample, SampleType::U32 | SampleType::F64) {
        if channel.surface.format().sample != source.depth {
            return Err(bad(CMD, "wide new-image channel depth conversion is not qualified"));
        }
        let sample = source.depth.bytes();
        let mut one = [0u8; 8];
        let one = one.get_mut(..sample).ok_or_else(|| bad(CMD, "wide sample width"))?;
        photocraft_color::try_write_sample_f64(one, source.depth, 0, 1.).map_err(|e| error(ctl, e))?;
        for coordinate in area.tiles() {
            poll(ctl)?;
            let part = coordinate.rect().intersect(&area);
            let raw = channel.surface.try_read_interleaved_with(part, ctl).map_err(|e| error(ctl, e))?;
            let length = raw.len().checked_mul(2).ok_or_else(|| bad(CMD, "new-image tile length"))?;
            let mut paired = Vec::new();
            paired.try_reserve_exact(length).map_err(|_| bad(CMD, "new-image native tile allocation"))?;
            if paired.capacity() as u64 > MAX_CHECKED_RASTER_BYTES {
                return Err(bad(CMD, "new-image native tile capacity"));
            }
            for (index, pixel) in raw.chunks_exact(sample).enumerate() {
                if index.is_multiple_of(4096) {
                    poll(ctl)?;
                }
                paired.extend_from_slice(pixel);
                paired.extend_from_slice(one);
            }
            surface.try_write_interleaved_with(part, &paired, ctl).map_err(|e| error(ctl, e))?;
        }
    } else {
        let values = read_plane_with_interrupt(&channel.surface, area, ctl)?;
        let length = values.len().checked_mul(format.bytes_per_pixel()).ok_or_else(|| bad(CMD, "new-image encoded length"))?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(length).map_err(|_| bad(CMD, "new-image plane allocation"))?;
        if raw.capacity() as u64 > MAX_CHECKED_RASTER_BYTES {
            return Err(bad(CMD, "new-image plane capacity"));
        }
        for (index, value) in values.iter().enumerate() {
            if index.is_multiple_of(4096) {
                poll(ctl)?;
            }
            if !value.is_finite() {
                return Err(bad(CMD, "nonfinite new-image channel value"));
            }
            let mut encoded = [0u8; 40];
            let pixel = encoded.get_mut(..format.bytes_per_pixel()).ok_or_else(|| bad(CMD, "new-image sample length"))?;
            photocraft_raster::encode_pixel(&format, &[value.clamp(0., 1.), 1.], pixel);
            raw.extend_from_slice(pixel);
        }
        surface.try_write_interleaved_with(area, &raw, ctl).map_err(|e| error(ctl, e))?;
    }
    surface.try_prune_with(ctl).map_err(|e| error(ctl, e))?;
    let mut background = Layer::new("Background", LayerContent::Raster(surface));
    background.locks.transparency = true;
    background.locks.position = true;
    document.layers.try_reserve_exact(1).map_err(|_| bad(CMD, "new-image layer allocation"))?;
    document.layers.push(background);
    poll(ctl)?;
    Ok(document)
}
fn current_index(session: &Session, id: photocraft_doc::DocId, revision: u64, snapshot: &DocumentSnapshot, ctl: &Interrupt<'_>) -> Result<usize> {
    poll(ctl)?;
    if session.documents().len() > 4096 {
        return Err(bad(CMD, "document count changed beyond limit"));
    }
    if session.command_has_job_on(id) || session.command_has_pending_job() {
        return Err(bad(CMD, "captured document acquired an in-flight job"));
    }
    let weak = snapshot.downgrade();
    let mut index = None;
    for (position, state) in session.documents().iter().enumerate() {
        poll(ctl)?;
        if state.doc.id == id {
            if index.is_some() {
                return Err(bad(CMD, "duplicate document identities"));
            }
            if state.revision != revision
                || !state.doc.shares_state(snapshot)
                || !weak.is_current(&state.doc)
                || state.floating.is_some()
                || state.coalesce.is_some()
                || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
            {
                return Err(bad(CMD, "document changed during channel duplication"));
            }
            index = Some(position);
        }
    }
    poll(ctl)?;
    index.ok_or_else(|| bad(CMD, "document closed during channel duplication"))
}
pub(super) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    let request = request(session, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let source = state.doc.clone();
    let source_revision = state.revision;
    let active_layer = state.active_layer;
    let base = match selected_channel(&source, request.reference)? {
        Some(channel) => channel.name.as_str(),
        None => match request.reference {
            ChanRef::Color(index) => *color_names(edit_mode(&source)).get(index).ok_or_else(|| bad(CMD, "color channel index"))?,
            _ => composite_name(edit_mode(&source)),
        },
    };
    let name = copy_name(params.get("name").and_then(Value::as_str).unwrap_or(base), params.get("name").is_none(), &Interrupt::NONE)?;
    let target = match request.destination {
        Destination::New => None,
        Destination::Existing(index) => {
            let state = session.documents().get(index).ok_or_else(|| bad(CMD, "destination closed"))?;
            Some((state.doc.clone(), state.revision))
        }
    };
    let source_work = source.clone();
    let target_work = target.clone();
    crate::jobs::run(
        session,
        "Duplicate Channel",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Duplicate Channel", |ctl| {
                let prep = super::SourcePreparation::new(CMD, ctl)?;
                admission_prepared(&source_work, target_work.as_ref().map(|(doc, _)| &**doc), request.reference, request.invert, ctl, &prep)?;
                if request.invert {
                    invert_policy(&source_work, request.source_id, ctl)?;
                }
                let mut channel = prepare_channel_prepared(&source_work, active_layer, request, name, ctl, &prep)?;
                let output = match &target_work {
                    None => new_image(&source_work, &channel, ctl)?,
                    Some((target, _)) => {
                        let format = channel_format(target);
                        if channel.surface.format() != format {
                            channel.surface = channel.surface.try_convert_with(format, ctl).map_err(|e| error(ctl, e))?;
                        }
                        channel.id = fresh_channel(target, ctl)?;
                        let new_id = channel.id;
                        poll(ctl)?;
                        let mut doc = (**target).clone();
                        doc.channels.try_reserve_exact(1).map_err(|_| bad(CMD, "destination channel allocation"))?;
                        doc.channels.push(channel);
                        if request.metadata {
                            metadata(&mut doc, request.source_id.ok_or_else(|| bad(CMD, "source channel identity"))?, new_id, ctl)?;
                        }
                        poll(ctl)?;
                        doc
                    }
                };
                poll(ctl)?;
                Ok((output, ctx.clone()))
            })
        },
        move |session, (document, ctx)| {
            ctx.stage(0.95, 1.0, "Duplicate Channel", |ctl| {
                current_index(session, source.id, source_revision, &source, ctl)?;
                if session.coalesce_request.is_some() || session.color_restrict.is_some() {
                    return Err(bad(CMD, "gesture changed during channel duplication"));
                }
                match &target {
                    None => {
                        let index = session.add_document_with_interrupt(document, None, ctl)?;
                        Ok(json!({"document":index}))
                    }
                    Some((target, revision)) => {
                        let index = current_index(session, target.id, *revision, target, ctl)?;
                        let channel = document.channels.len().checked_sub(1).ok_or_else(|| bad(CMD, "missing prepared channel"))?;
                        poll(ctl)?;
                        session.edit_document_with_interrupt(index, "Duplicate Channel", ctl, move |doc, _| {
                            poll(ctl)?;
                            *doc = document;
                            Ok(())
                        })?;
                        Ok(json!({"channel":channel,"document":index}))
                    }
                }
            })
        },
    )
}

// Draft regressions: intentionally not executed in the implementation-only wave.
#[cfg(test)]
// Test assertions are permitted by repository policy.
#[allow(clippy::unreachable)]
mod tests {
    use super::*;
    use photocraft_doc::Size;
    fn fixture(sample: SampleType) -> (Session, Vec<u8>) {
        let mut doc = Document::new("Source", Size::new(2, 1), ColorMode::Rgb, sample);
        let raw = match sample {
            SampleType::U32 => [1u32, u32::MAX - 1].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>(),
            SampleType::F64 => [1f64.to_bits() + 1, (-0f64).to_bits()].into_iter().flat_map(u64::to_le_bytes).collect(),
            _ => vec![13, 247],
        };
        let surface = Surface::try_from_interleaved_with(channel_format(&doc), doc.bounds(), &raw, &Interrupt::NONE).unwrap();
        doc.channels.push(AlphaChannel::new("Own coverage", surface));
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        (session, raw)
    }
    #[test]
    fn wide_saved_copy_preserves_words_native_and_one_target_undo() {
        for sample in [SampleType::U32, SampleType::F64] {
            let (mut session, raw) = fixture(sample);
            let before = session.active().unwrap().doc.clone();
            let source_id = before.channels[0].id;
            run(&mut session, &json!({"channelId":source_id.0.to_string()})).unwrap();
            let current = &session.active().unwrap().doc;
            assert_eq!(current.channels.len(), 2);
            assert_ne!(current.channels[1].id, source_id);
            assert_eq!(current.channels[1].surface.to_interleaved(current.bounds()), raw);
            assert_eq!(current.channels[0].surface, before.channels[0].surface);
            let saved = photocraft_format::save_to_bytes(current, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_eq!(reopened.channels, current.channels);
            session.execute("edit.undo", Value::Null).unwrap();
            assert_eq!(session.active().unwrap().doc.channels, before.channels);
            session.execute("edit.redo", Value::Null).unwrap();
            assert_eq!(session.active().unwrap().doc.channels, reopened.channels);
        }
    }
    #[test]
    fn cross_document_copy_keeps_focus_source_and_target_history_separate() {
        let (mut session, raw) = fixture(SampleType::U32);
        let source = session.active().unwrap().doc.clone();
        let source_revision = session.active().unwrap().revision;
        let destination = Document::new("Destination", source.size, ColorMode::Rgb, SampleType::U32);
        session.add_document(destination, None).unwrap();
        session.set_active(0);
        let target_revision = session.documents()[1].revision;
        let result = run(&mut session, &json!({"channel":0,"document":1})).unwrap();
        assert_eq!(result["document"], 1);
        assert_eq!(session.active_index(), Some(0));
        assert!(session.documents()[0].doc.shares_state(&source));
        assert_eq!(session.documents()[0].revision, source_revision);
        assert_eq!(session.documents()[1].revision, target_revision + 1);
        assert_eq!(session.documents()[1].doc.channels[0].surface.to_interleaved(source.bounds()), raw);
        session.set_active(1);
        session.execute("edit.undo", Value::Null).unwrap();
        assert!(session.active().unwrap().doc.channels.is_empty());
        assert_eq!(session.documents()[0].revision, source_revision);
    }
    #[test]
    fn wide_inversion_checks_unit_domain_and_preserves_real_redo_on_refusal() {
        let (mut session, _) = fixture(SampleType::F64);
        session
            .edit("Seed", |doc, _| {
                doc.name = "Seed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", Value::Null).unwrap();
        let source = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let redo = session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
        // First sample is the exact F64 successor of one: outside unit coverage.
        assert!(run(&mut session, &json!({"channel":0,"invert":true})).is_err());
        assert!(session.active().unwrap().doc.shares_state(&source));
        assert_eq!(session.active().unwrap().revision, revision);
        assert_eq!(session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
        let (mut integer, _) = fixture(SampleType::U32);
        run(&mut integer, &json!({"channel":0,"invert":true})).unwrap();
        let doc = &integer.active().unwrap().doc;
        assert_eq!(doc.channels[1].surface.to_interleaved(doc.bounds()), [u32::MAX - 1, 1].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>());
    }
    #[test]
    fn cancelled_admission_and_stale_fence_leave_snapshots_unchanged() {
        let (mut session, _) = fixture(SampleType::U32);
        let source = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let cancel = Interrupt::cancel_only(&|| true);
        assert!(matches!(admission(&source, Some(&source), ChanRef::Alpha(0), &cancel), Err(EngineError::Cancelled)));
        let requested = request(&session, &json!({"channel":0})).unwrap();
        assert!(matches!(prepare_channel(&source, None, requested, "Own copy".into(), &cancel), Err(EngineError::Cancelled)));
        assert!(run(&mut session, &json!({"channel":0,"documentId":source.id.0.to_string(),"expectedRevision":(revision+1).to_string()})).is_err());
        assert!(session.active().unwrap().doc.shares_state(&source));
        session
            .edit("Changed", |doc, _| {
                doc.name = "Changed".into();
                Ok(())
            })
            .unwrap();
        assert!(current_index(&session, source.id, revision, &source, &Interrupt::NONE).is_err());
    }
    #[test]
    fn new_wide_image_copies_canvas_raw_gray_and_adds_exact_opaque_alpha() {
        for sample in [SampleType::U32, SampleType::F64] {
            let (session, raw) = fixture(sample);
            let source = &session.active().unwrap().doc;
            admission(source, None, ChanRef::Alpha(0), &Interrupt::NONE).unwrap();
            let doc = new_image(source, &source.channels[0], &Interrupt::NONE).unwrap();
            let one = match sample {
                SampleType::U32 => u32::MAX.to_le_bytes().to_vec(),
                SampleType::F64 => 1f64.to_le_bytes().to_vec(),
                _ => unreachable!(),
            };
            let expected = raw.chunks_exact(sample.bytes()).flat_map(|sample| sample.iter().chain(one.iter()).copied()).collect::<Vec<_>>();
            assert_eq!(doc.layers[0].surface().unwrap().to_interleaved(doc.bounds()), expected);
            assert_eq!(doc.effective_resolution(), source.effective_resolution());
            assert_eq!(source.channels[0].surface.to_interleaved(source.bounds()), raw);
        }
    }
}
