//! Private controlled Save Selection; raw same-depth coverage is not color math.
use super::load_selection_controlled::canonical_selection_storage;
use super::{ChanRef, channel_format, parameter_ref, parse_ref};
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_color::SampleType;
use photocraft_doc::{AlphaChannel, Document};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
use serde_json::{Value, json};
const ID: &str = "select.saveSelection";
fn error(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("{ID}: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn exact(v: &Value, nonzero: bool) -> Result<u64> {
    let s = v
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| error("captured IDs must be canonical decimal strings"))?;
    let n = s.parse::<u64>().map_err(|_| error("captured ID exceeds range"))?;
    if nonzero && n == 0 {
        return Err(error("documentId must be nonzero"));
    }
    Ok(n)
}
struct Request<'a> {
    destination: usize,
    existing: Option<usize>,
    operation: &'a str,
    name: Option<&'a str>,
    raw_storage: bool,
}
fn request<'a>(session: &Session, p: &'a Value, early: bool) -> Result<Request<'a>> {
    if !p.is_null()
        && !p.as_object().is_some_and(|p| {
            p.len() <= 8
                && p.keys().all(|k| {
                    matches!(k.as_str(), "name" | "channel" | "channelId" | "document" | "operation" | "documentId" | "expectedRevision" | "rawStorage")
                })
        })
    {
        return Err(error("expected bounded Save Selection parameters"));
    }
    let op = match p.get("operation") {
        None => "new",
        Some(v) => v
            .as_str()
            .filter(|s| matches!(*s, "new" | "replace" | "add" | "subtract" | "intersect"))
            .ok_or_else(|| error("unknown selection save operation"))?,
    };
    let name = match p.get("name") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let s = v.as_str().filter(|s| s.len() <= 4096 && !s.as_bytes().contains(&0)).ok_or_else(|| error("name must be bounded UTF8 without NUL"))?.trim();
            (!s.is_empty()).then_some(s)
        }
    };
    let raw_storage = match p.get("rawStorage") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(|| error("rawStorage must be boolean"))?,
    };
    if raw_storage && !matches!(op, "new" | "replace") {
        return Err(error("rawStorage does not support combining selections"));
    }
    let source = session.active().ok_or(EngineError::NoDocument)?;
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if exact(id, true)? == source.doc.id.0 && exact(rev, false)? == source.revision => {}
        _ => return Err(error("captured selection source changed or incomplete")),
    }
    let destination = match p.get("document") {
        None | Some(Value::Null) => session.active_index().ok_or(EngineError::NoDocument)?,
        Some(v) => {
            let n = v
                .as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0 && v.round() < (session.documents().len() as f64))
                .ok_or_else(|| error("invalid destination document index"))?
                .round();
            n as usize
        }
    };
    let target = session.documents().get(destination).ok_or_else(|| error("destination document missing"))?;
    let legacy_source_float = early && p.get("documentId").is_none() && !raw_storage;
    for state in [source, target] {
        let allow_float = legacy_source_float && state.doc.id == source.doc.id;
        if (state.floating.is_some() && !allow_float)
            || state.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
            || state.coalesce.is_some()
            || state.revision == u64::MAX
            || state.doc.accounted().is_some()
            || session.command_has_job_on(state.doc.id)
        {
            return Err(error("finish source/destination transient work before saving selection"));
        }
    }
    if session.has_jobs() || session.command_has_pending_job() || session.coalesce_request.is_some() || session.color_restrict.is_some() {
        return Err(error("pending work or restricted edit context"));
    }
    if source.doc.size != target.doc.size {
        return Err(error("the destination document must have the same size"));
    }
    if target.doc.channels.len() > 100_000 {
        return Err(error("destination channel count limit"));
    }
    let existing = match parameter_ref(p, "channel") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s == "new" => None,
        Some(v) => {
            match v {
                Value::String(s) if s.len() <= 4096 && !s.as_bytes().contains(&0) => {}
                Value::Number(n) if n.as_u64().and_then(|n| usize::try_from(n).ok()).is_some() => {}
                Value::Object(o) if std::ptr::eq(v, p) && o.get("channelId").is_some_and(|v| super::channel_identity(v).is_some()) => {}
                Value::Object(o)
                    if o.len() == 1
                        && (o.get("channelId").is_some_and(|v| super::channel_identity(v).is_some())
                            || o.get("alpha").is_some_and(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()).is_some())) => {}
                _ => return Err(error("invalid alpha channel selector")),
            }
            match parse_ref(v, &target.doc) {
                Some(ChanRef::Alpha(i)) if target.doc.channels.get(i).is_some() => Some(i),
                _ => return Err(error("channel must be new or an existing alpha channel")),
            }
        }
    };
    if raw_storage {
        let selection = source.doc.selection.as_ref().ok_or_else(|| error("rawStorage requires a stored selection"))?;
        if selection.format() != channel_format(&target.doc) {
            return Err(error("rawStorage source/destination depth mismatch"));
        }
        if source.doc.xcf_state.is_some() || target.doc.xcf_state.is_some() {
            if op != "replace" {
                return Err(error("canonical rawStorage Save supports existing channel replacement only"));
            }
            let index = existing.ok_or_else(|| error("canonical rawStorage Save cannot create imported channels"))?;
            let channel = target.doc.channels.get(index).ok_or_else(|| error("canonical target disappeared"))?;
            if name.is_some_and(|name| name != channel.name) {
                return Err(error("canonical rawStorage replacement preserves the target name"));
            }
            canonical_selection_storage::prepare_save(&source.doc, &target.doc, channel.id, &Interrupt::NONE)?;
        } else if source.doc.source_color_context.is_some() || target.doc.source_color_context.is_some() {
            return Err(error("rawStorage source-carrier context is not qualified"));
        }
        if existing.and_then(|index| target.doc.channels.get(index)).is_some_and(|channel| channel.spot.is_some()) {
            return Err(error("rawStorage target cannot be spot ink"));
        }
    }
    Ok(Request { destination, existing, operation: op, name, raw_storage })
}
pub(super) fn dispatch_preflight(session: &Session, id: &str, p: &Value, _background: bool) -> Result<bool> {
    if id != ID {
        return Ok(false);
    }
    let _ = request(session, p, true)?;
    Ok(true)
}
fn raw_lane(source: &Document, target: &Document, existing: Option<usize>, op: &str) -> bool {
    (existing.is_none() || matches!(op, "new" | "replace")) && source.selection.as_ref().is_none_or(|s| s.format() == channel_format(target))
}
fn prepare(source: &DocumentSnapshot, target: &DocumentSnapshot, existing: Option<usize>, op: &str, control: &Interrupt<'_>) -> Result<bool> {
    poll(control)?;
    source.validate_resident()?;
    target.validate_resident()?;
    source.validate_xcf_state_with(control).map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(e) })?;
    target.validate_xcf_state_with(control).map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(e) })?;
    let (area, _) = super::source_dimensions(source, ID)?;
    let _ = super::source_dimensions(target, ID)?;
    let raw = raw_lane(source, target, existing, op);
    if let Some(index) = existing.filter(|_| op != "new") {
        let channel = target.channels.get(index).ok_or_else(|| error("destination channel disappeared"))?;
        crate::xcf_item_locks::require_channel_content(target, channel.id)?;
        if target.xcf_state.as_ref().is_some_and(|s| s.items.iter().any(|item| item.item == photocraft_doc::item_identity::ItemRef::Channel(channel.id))) {
            return Err(error("canonical imported channel replacement needs an independently qualified current-state edit"));
        }
    }
    if !raw
        && (source.selection.as_ref().is_some_and(|s| matches!(s.format().sample, SampleType::U32 | SampleType::F64))
            || matches!(target.depth, SampleType::U32 | SampleType::F64)
            || source.xcf_state.is_some()
            || target.xcf_state.is_some())
    {
        return Err(error("wide or imported selection arithmetic is not qualified for legacy f32 math"));
    }
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let mut peak = 0u64;
    for snapshot in [source, target] {
        poll(control)?;
        let payload = photocraft_io::retained_native_payload_bytes(snapshot, limits, control)
            .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(e) })? as u64;
        let model = photocraft_io::retained_native_model_bytes(snapshot, limits, control)
            .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(e) })? as u64;
        let raster = snapshot.resident_raster_footprint_with(control)?;
        peak = peak
            .checked_add(payload)
            .and_then(|n| n.checked_add(model.checked_mul(4)?))
            .and_then(|n| n.checked_add(raster.retained_bytes))
            .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
            .ok_or_else(|| error("snapshot/model budget overflow"))?;
    }
    let bytes = checked_region_byte_len(area, channel_format(target)).map_err(error)? as u64;
    let pixels = area.size().area();
    let work = pixels
        .checked_mul(32)
        .and_then(|n| n.checked_add((target.channels.len() as u64).checked_pow(2)?.checked_mul(24)?))
        .ok_or_else(|| error("selection work overflow"))?;
    // Full canvas plane/output plus conversion/sparse COW capacity, conservative
    // logical bound only; it is not a global allocator/RSS guarantee.
    let span = |first: i32, last: i32| -> Result<u64> {
        u64::try_from(i64::from((last - 1).div_euclid(photocraft_geom::TILE_SIZE)) - i64::from(first.div_euclid(photocraft_geom::TILE_SIZE)) + 1)
            .map_err(|_| error("tile span overflow"))
    };
    let tiles = span(area.x0, area.x1)?.checked_mul(span(area.y0, area.y1)?).ok_or_else(|| error("tile count overflow"))?;
    let tile_payload = (photocraft_geom::TILE_SIZE as u64) * (photocraft_geom::TILE_SIZE as u64) * (channel_format(target).bytes_per_pixel() as u64);
    let output =
        tiles.checked_mul(tile_payload.checked_add(512).ok_or_else(|| error("tile metadata overflow"))?).ok_or_else(|| error("output tiles overflow"))?;
    let work = work
        .checked_add(
            tiles
                .checked_mul(tile_payload.checked_mul(4).and_then(|n| n.checked_add(256)).ok_or_else(|| error("tile work overflow"))?)
                .ok_or_else(|| error("tile work overflow"))?,
        )
        .ok_or_else(|| error("aggregate work overflow"))?;
    peak = peak
        .checked_add(bytes.checked_mul(4).ok_or_else(|| error("output size overflow"))?)
        .and_then(|n| n.checked_add(output.checked_mul(3)?))
        .and_then(|n| n.checked_add(pixels.checked_mul(32)?))
        .ok_or_else(|| error("selection peak overflow"))?;
    if peak > MAX_CHECKED_RASTER_BYTES || work > 1 << 32 {
        return Err(error("selection staging exceeds standard operation budget"));
    }
    poll(control)?;
    Ok(raw)
}
fn string_copy(text: &str) -> Result<String> {
    let mut name = String::new();
    name.try_reserve_exact(text.len()).map_err(|_| error("name allocation"))?;
    name.push_str(text);
    Ok(name)
}
fn zero_surface(format: photocraft_color::PixelFormat) -> Result<Surface> {
    Surface::try_with_default_bytes(format, [0u8; 8].get(..format.bytes_per_pixel()).ok_or_else(|| error("invalid channel format"))?).map_err(error)
}
fn plane(source: &Document, target: &Document, existing: Option<usize>, op: &str, raw: bool, control: &Interrupt<'_>) -> Result<Surface> {
    let area = target.bounds();
    let fmt = channel_format(target);
    let mut output = zero_surface(fmt)?;
    let Some(selection) = source.selection.as_ref() else {
        if raw {
            return Ok(output);
        }
        return ordinary_plane(source, target, existing, op, control);
    };
    if !raw {
        return ordinary_plane(source, target, existing, op, control);
    }
    for tile in area.tiles() {
        poll(control)?;
        let r = tile.rect().intersect(&area);
        let bytes =
            selection.try_read_interleaved_with(r, control).map_err(|e| if e == RasterInputError::Cancelled { EngineError::Cancelled } else { error(e) })?;
        if matches!(fmt.sample, SampleType::F32 | SampleType::F64) {
            for chunk in bytes.chunks_exact(fmt.sample.bytes()) {
                poll(control)?;
                let v = photocraft_color::try_read_sample_f64(chunk, fmt.sample, 0).map_err(error)?;
                if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                    return Err(error("invalid native coverage sample"));
                }
            }
        }
        output.try_write_interleaved_with(r, &bytes, control).map_err(|e| if e == RasterInputError::Cancelled { EngineError::Cancelled } else { error(e) })?;
    }
    output.try_prune_with(control).map_err(|e| if e == RasterInputError::Cancelled { EngineError::Cancelled } else { error(e) })?;
    Ok(output)
}
fn ordinary_plane(source: &Document, target: &Document, existing: Option<usize>, op: &str, control: &Interrupt<'_>) -> Result<Surface> {
    let area = target.bounds();
    let fmt = channel_format(target);
    let pixels = usize::try_from(area.size().area()).map_err(|_| error("canvas size exceeds address range"))?;
    let selection = match &source.selection {
        Some(s) => super::read_plane_with_interrupt(s, area, control)?,
        None => super::source_buffer(pixels, control)?,
    };
    let old = match existing.filter(|_| !matches!(op, "new" | "replace")) {
        Some(i) => Some(super::read_plane_with_interrupt(&target.channels.get(i).ok_or_else(|| error("channel disappeared"))?.surface, area, control)?),
        None => None,
    };
    let mut output = zero_surface(fmt)?;
    let mut index = 0usize;
    for tile in area.tiles() {
        poll(control)?;
        let r = tile.rect().intersect(&area);
        let length = checked_region_byte_len(r, fmt).map_err(error)?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(length).map_err(|_| error("output allocation"))?;
        if raw.capacity() != length {
            return Err(error("output capacity"));
        }
        while raw.len() < length {
            poll(control)?;
            raw.resize(raw.len().saturating_add(8192).min(length), 0);
        }
        for y in r.y0..r.y1 {
            poll(control)?;
            for x in r.x0..r.x1 {
                let i = usize::try_from(i64::from(y) - i64::from(area.y0))
                    .ok()
                    .and_then(|row| row.checked_mul(area.width() as usize))
                    .and_then(|i| i.checked_add(usize::try_from(i64::from(x) - i64::from(area.x0)).ok()?))
                    .ok_or_else(|| error("coverage index overflow"))?;
                let b = *selection.get(i).ok_or_else(|| error("selection length mismatch"))?;
                let a = old.as_ref().and_then(|v| v.get(i)).copied().unwrap_or(0.0);
                let value = match op {
                    "add" if old.is_some() => a.max(b),
                    "subtract" if old.is_some() => (a - b).max(0.0),
                    "intersect" if old.is_some() => a.min(b),
                    _ => b,
                }
                .clamp(0.0, 1.0);
                photocraft_color::write_sample(&mut raw, fmt.sample, index, value);
                index = index.checked_add(1).ok_or_else(|| error("output index overflow"))?;
            }
        }
        output.try_write_interleaved_with(r, &raw, control).map_err(|e| if e == RasterInputError::Cancelled { EngineError::Cancelled } else { error(e) })?;
        index = 0;
    }
    output.try_prune_with(control).map_err(|e| if e == RasterInputError::Cancelled { EngineError::Cancelled } else { error(e) })?;
    Ok(output)
}
pub(super) fn save_selection(session: &mut Session, p: &Value) -> Result<Value> {
    let parsed = request(session, p, false)?;
    let raw_storage = parsed.raw_storage;
    let di = parsed.destination;
    let existing = parsed.existing;
    let operation = string_copy(parsed.operation)?;
    let name = parsed.name.map(string_copy).transpose()?;
    let source = session.active().ok_or(EngineError::NoDocument)?;
    let source_snapshot = source.doc.clone();
    let source_revision = source.revision;
    let source_weak = source_snapshot.downgrade();
    let target = session.documents().get(di).ok_or(EngineError::NoDocument)?;
    let target_snapshot = target.doc.clone();
    let target_revision = target.revision;
    let target_weak = target_snapshot.downgrade();
    let work_source = source_snapshot.clone();
    let work_target = target_snapshot.clone();
    crate::jobs::run(
        session,
        "Save Selection",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Save Selection", |control| {
                let proof = if raw_storage && (work_source.xcf_state.is_some() || work_target.xcf_state.is_some()) {
                    if operation != "replace" {
                        return Err(error("canonical rawStorage Save supports replacement only"));
                    }
                    let index = existing.ok_or_else(|| error("canonical saved target missing"))?;
                    let channel = work_target.channels.get(index).ok_or_else(|| error("canonical saved target disappeared"))?;
                    Some(canonical_selection_storage::prepare_save(&work_source, &work_target, channel.id, control)?)
                } else {
                    None
                };
                let surface = if let Some(proof) = &proof {
                    let selection = work_source.selection.as_ref().ok_or_else(|| error("canonical source selection disappeared"))?;
                    super::selection_storage_controlled::copy_canonical_save(
                        &work_source,
                        &work_target,
                        selection,
                        channel_format(&work_target),
                        proof,
                        ID,
                        control,
                    )?
                } else {
                    let raw = prepare(&work_source, &work_target, existing, &operation, control)?;
                    if raw_storage {
                        let selection = work_source.selection.as_ref().ok_or_else(|| error("rawStorage selection disappeared"))?;
                        super::selection_storage_controlled::copy(&work_source, &work_target, selection, channel_format(&work_target), ID, control)?
                    } else {
                        plane(&work_source, &work_target, existing, &operation, raw, control)?
                    }
                };
                let canonical_noop = if proof.as_ref().is_some_and(|proof| !proof.changes_bounds()) {
                    let channel = existing.and_then(|index| work_target.channels.get(index)).ok_or_else(|| error("canonical no-op target disappeared"))?;
                    super::selection_storage_controlled::equal_storage(&channel.surface, &surface, control)?
                } else {
                    false
                };
                poll(control)?;
                let mut document = (*work_target).clone();
                let index = if existing.is_none() || operation == "new" {
                    document.channels.try_reserve(1).map_err(|_| error("channel allocation"))?;
                    let name = match name {
                        Some(ref name) => string_copy(name)?,
                        None => super::next_alpha_name(&document, "Alpha"),
                    };
                    document.channels.push(AlphaChannel::new(name, surface));
                    document.channels.len() - 1
                } else {
                    let index = existing.ok_or_else(|| error("destination missing"))?;
                    let channel = document.channels.get_mut(index).ok_or_else(|| error("channel disappeared"))?;
                    channel.surface = surface;
                    if let Some(ref name) = name {
                        channel.name = string_copy(name)?;
                    }
                    index
                };
                let result = json!({"channel":index,"name":document.channels.get(index).ok_or_else(||error("channel missing"))?.name,"document":di});
                poll(control)?;
                Ok((document, result, proof, canonical_noop, ctx.clone()))
            })
        },
        move |session, (document, result, proof, canonical_noop, ctx)| {
            let source = session.active().ok_or(EngineError::NoDocument)?;
            let target = session.documents().get(di).ok_or(EngineError::NoDocument)?;
            for (state, expected, revision, weak) in
                [(source, &source_snapshot, source_revision, &source_weak), (target, &target_snapshot, target_revision, &target_weak)]
            {
                if state.doc.id != expected.id
                    || state.revision != revision
                    || !weak.is_current(&state.doc)
                    || !state.doc.shares_state(expected)
                    || state.floating.is_some()
                    || state.coalesce.is_some()
                    || state.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
                    || session.command_has_job_on(state.doc.id)
                {
                    return Err(error("source or destination changed during preparation"));
                }
            }
            if session.command_has_job_on(target_snapshot.id) {
                return Err(error("destination has pending work"));
            }
            ctx.stage(0.95, 1.0, "Save Selection", |control| {
                poll(control)?;
                if canonical_noop {
                    return Ok(result);
                }
                session.edit_document_with_interrupt(di, "Save Selection", control, move |current, _| {
                    poll(control)?;
                    *current = document;
                    if let Some(proof) = proof {
                        proof.apply_bounds(current, control)?;
                    }
                    Ok(())
                })?;
                Ok(result)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::ColorMode;
    #[test]
    fn same_depth_wide_raw_copy_cross_document_keeps_focus_source_and_native_bits() {
        for sample in [SampleType::U32, SampleType::F64] {
            let mut source = Document::new("Source", photocraft_doc::Size::new(2, 1), ColorMode::Rgb, sample);
            let raw = if sample == SampleType::U32 {
                [0x80000001u32, u32::MAX - 1].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()
            } else {
                [(-0.0f64).to_bits(), 0.5000000000000001f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect()
            };
            let mut selection = zero_surface(channel_format(&source)).unwrap();
            selection.try_write_interleaved(source.bounds(), &raw).unwrap();
            source.selection = Some(selection);
            let target = Document::new("Target", source.size, ColorMode::Rgb, sample);
            let mut session = Session::new();
            session.add_document(source, None).unwrap();
            session.add_document(target, None).unwrap();
            session.set_active(0);
            let original = session.active().unwrap().doc.clone();
            let rev = session.active().unwrap().revision;
            let reply = save_selection(
                &mut session,
                &json!({"document":1,"name":"  Own native  ","documentId":original.id.0.to_string(),"expectedRevision":rev.to_string()}),
            )
            .unwrap();
            assert_eq!(session.active_index(), Some(0));
            assert_eq!(session.active().unwrap().revision, rev);
            assert!(original.shares_state(&session.active().unwrap().doc));
            assert_eq!(reply["name"], "Own native");
            assert_eq!(session.documents()[1].doc.channels[0].surface.try_read_interleaved(original.bounds()).unwrap(), raw);
            session.set_active(1);
            assert!(session.undo());
            assert!(session.active().unwrap().doc.channels.is_empty());
            assert!(session.redo());
            assert_eq!(session.active().unwrap().doc.channels[0].surface.try_read_interleaved(original.bounds()).unwrap(), raw);
        }
    }
    #[test]
    fn absent_selection_preserves_actual_legacy_zero_default() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32, SampleType::U32, SampleType::F64] {
            let doc = Document::new("No selection", photocraft_doc::Size::new(2, 1), ColorMode::Rgb, sample);
            assert_eq!(photocraft_algo::selection::mask_from_surface(None, doc.bounds()), [0.0, 0.0]);
            let output = plane(&doc, &doc, None, "new", true, &Interrupt::NONE).unwrap();
            assert_eq!(output.try_read_interleaved(doc.bounds()).unwrap(), vec![0u8; 2 * sample.bytes()]);
            assert_eq!(output.default_bytes(), vec![0u8; sample.bytes()]);
        }
    }
    #[test]
    fn ordinary_combination_clamps_legacy_math_and_wide_arithmetic_refuses() {
        let mut doc = Document::new("Coverage", photocraft_doc::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        let mut selection = zero_surface(channel_format(&doc)).unwrap();
        selection.try_write_interleaved(doc.bounds(), &[128]).unwrap();
        doc.selection = Some(selection);
        let mut old = zero_surface(channel_format(&doc)).unwrap();
        old.try_write_interleaved(doc.bounds(), &[64]).unwrap();
        doc.channels.push(AlphaChannel::new("Old", old));
        let result = ordinary_plane(&doc, &doc, Some(0), "subtract", &Interrupt::NONE).unwrap();
        assert_eq!(result.try_read_interleaved(doc.bounds()).unwrap(), [0]);
        let snapshot = DocumentSnapshot::from_resident(std::sync::Arc::new(doc)).unwrap();
        let cancel = || true;
        assert!(matches!(prepare(&snapshot, &snapshot, Some(0), "add", &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
        let mut wide = (*snapshot).clone();
        wide.depth = SampleType::U32;
        let wide = DocumentSnapshot::from_resident(std::sync::Arc::new(wide)).unwrap();
        assert!(prepare(&snapshot, &wide, Some(0), "add", &Interrupt::NONE).is_err());
    }
}
