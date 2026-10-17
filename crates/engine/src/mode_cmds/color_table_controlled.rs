//! Atomic bounded Color Table replacement. Canonical ordinals never reverse-map RGB.
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_color::{ColorMode, SampleType, read_sample};
use photocraft_doc::{ColorTable, Document, Layer, LayerContent};
use photocraft_geom::TILE_SIZE;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
const CMD: &str = "image.mode.colorTable";
fn bad(message: impl std::fmt::Display) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: message.to_string() }
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn error(failure: impl std::fmt::Display, control: &Interrupt<'_>) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(failure) }
}
fn exact(value: &Value, nonzero: bool) -> Result<u64> {
    let text = value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad("captured target requires canonical decimal strings"))?;
    let number = text.parse::<u64>().map_err(|_| bad("captured target exceeds range"))?;
    if nonzero && number == 0 {
        return Err(bad("documentId must be nonzero"));
    }
    Ok(number)
}
fn color(value: &Value) -> Result<[u8; 3]> {
    match value {
        Value::String(text) if text.len() == 6 || (text.len() == 7 && text.starts_with('#')) => {}
        Value::Array(values) if (3..=4).contains(&values.len()) && values.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)) => {}
        _ => return Err(bad("color must be #rrggbb or three finite RGB components")),
    }
    super::parse_color(value).ok_or_else(|| bad("invalid RGB color"))
}
fn preset(value: &Value) -> Result<&str> {
    match value.get("table") {
        None => Ok("custom"),
        Some(value) => value
            .as_str()
            .filter(|s| matches!(*s, "custom" | "grayscale" | "blackBody" | "spectrum" | "systemMac" | "system" | "systemWindows" | "web"))
            .ok_or_else(|| bad("unknown Color Table preset")),
    }
}
fn palette_length(current: &ColorTable, value: &Value) -> Result<usize> {
    let name = preset(value)?;
    let mut length = match name {
        "custom" => current.colors.len(),
        "web" => 216,
        _ => 256,
    };
    if let Some(colors) = value.get("colors") {
        let values = colors.as_array().filter(|v| !v.is_empty() && v.len() <= 256).ok_or_else(|| bad("colors requires 1 to 256 entries"))?;
        for value in values {
            color(value)?;
        }
        length = values.len();
    }
    if let Some(entries) = value.get("entries") {
        let entries = entries.as_object().filter(|v| v.len() <= 256).ok_or_else(|| bad("entries must be a bounded object"))?;
        for (key, value) in entries {
            let index = key
                .parse::<usize>()
                .ok()
                .filter(|_| !key.is_empty() && key.len() <= 20 && key.bytes().all(|b| b.is_ascii_digit()))
                .ok_or_else(|| bad("invalid table entry index"))?;
            if index >= length {
                return Err(bad("table entry is past the palette"));
            }
            color(value)?;
        }
    }
    let transparent = match value.get("transparent") {
        None => current.transparent.filter(|&index| usize::from(index) < length).map(u64::from),
        Some(Value::Null) => None,
        Some(value) => Some(value.as_u64().ok_or_else(|| bad("transparent must be an entry index or null"))?),
    };
    if transparent.is_some_and(|index| index >= length as u64) {
        return Err(bad("transparent entry is past the palette"));
    }
    Ok(length)
}
fn context(session: &Session, value: &Value, allow_legacy: bool) -> Result<()> {
    if !value.is_null()
        && !value.as_object().is_some_and(|p| {
            p.len() <= 6 && p.keys().all(|k| matches!(k.as_str(), "table" | "colors" | "entries" | "transparent" | "documentId" | "expectedRevision"))
        })
    {
        return Err(bad("expected bounded Color Table parameters"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let current =
        state.doc.color_table.as_ref().filter(|t| !t.colors.is_empty() && t.colors.len() <= 256).ok_or_else(|| bad("document has no valid Color Table"))?;
    palette_length(current, value)?;
    match (value.get("documentId"), value.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if exact(id, true)? == state.doc.id.0 && exact(revision, false)? == state.revision => {}
        _ => return Err(bad("captured document/revision changed or incomplete")),
    }
    let legacy = state.doc.xcf_state.is_none() && matches!(state.doc.depth, SampleType::U8 | SampleType::U16 | SampleType::F32);
    if state.doc.mode != ColorMode::Indexed
        || state.revision == u64::MAX
        || state.coalesce.is_some()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || (state.floating.is_some() && (!allow_legacy || value.get("documentId").is_some() || !legacy))
        || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.command_has_job_on(state.doc.id)
    {
        return Err(bad("finish preview/coalescing/pending work before changing Color Table"));
    }
    if state.doc.accounted().is_some() {
        return Err(bad("accounted Color Table replacement is unsupported"));
    }
    Ok(())
}
pub(super) fn dispatch_preflight(session: &Session, value: &Value, _background: bool) -> Result<()> {
    context(session, value, true)
}
fn copy_colors(colors: &[[u8; 3]]) -> Result<Vec<[u8; 3]>> {
    let mut out = Vec::new();
    out.try_reserve_exact(colors.len()).map_err(bad)?;
    if out.capacity() > 512 {
        return Err(bad("palette allocation capacity limit"));
    }
    out.extend_from_slice(colors);
    Ok(out)
}
fn configuration(current: &ColorTable, value: &Value) -> Result<ColorTable> {
    let name = preset(value)?;
    let mut colors = if name == "custom" { copy_colors(&current.colors)? } else { super::preset_table(name).ok_or_else(|| bad("unknown preset"))? };
    if let Some(values) = value.get("colors").and_then(Value::as_array) {
        colors = Vec::new();
        colors.try_reserve_exact(values.len()).map_err(bad)?;
        if colors.capacity() > 512 {
            return Err(bad("palette capacity limit"));
        }
        for value in values {
            colors.push(color(value)?);
        }
    }
    if let Some(entries) = value.get("entries").and_then(Value::as_object) {
        for (key, value) in entries {
            let index = key.parse::<usize>().map_err(bad)?;
            *colors.get_mut(index).ok_or_else(|| bad("table entry missing"))? = color(value)?;
        }
    }
    let transparent = match value.get("transparent") {
        None => current.transparent.filter(|&index| usize::from(index) < colors.len()),
        Some(Value::Null) => None,
        Some(value) => Some(u8::try_from(value.as_u64().ok_or_else(|| bad("transparent entry"))?).map_err(bad)?),
    };
    Ok(ColorTable { colors, transparent })
}
fn report(table: &ColorTable) -> Result<Value> {
    let mut colors = Vec::new();
    colors.try_reserve_exact(table.colors.len()).map_err(bad)?;
    for color in &table.colors {
        colors.push(super::hex(*color));
    }
    Ok(json!({"colors":colors,"transparent":table.transparent}))
}
fn ordinary(surface: &Surface) -> Result<()> {
    if !matches!(surface.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32)
        || surface.format().channels() == 0
        || surface.format().channels() > 5
        || surface.format().bytes_per_pixel() > 40
    {
        return Err(bad("legacy palette arithmetic requires ordinary U8/U16/F32 storage"));
    }
    Ok(())
}
fn source_context(layers: &[Layer], depth: usize, count: &mut usize, canonical: bool, control: &Interrupt<'_>) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("palette hierarchy depth limit"));
    }
    for layer in layers {
        poll(control)?;
        *count = count.checked_add(1).filter(|n| *n <= 10000).ok_or_else(|| bad("palette layer count limit"))?;
        if layer.video.is_some() || (!canonical && layer.indexed_source.is_some()) || matches!(layer.content, LayerContent::BackedRaster(_)) {
            return Err(bad("unqualified source/backed/video palette replacement"));
        }
        if let LayerContent::Group(group) = &layer.content {
            source_context(&group.children, depth + 1, count, canonical, control)?;
        }
    }
    Ok(())
}
fn prepare(snapshot: &DocumentSnapshot, table: &ColorTable, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    snapshot.validate_resident()?;
    let canonical = snapshot.xcf_state.as_ref().is_some_and(|state| !state.editable_indexed.is_empty());
    if snapshot.xcf_state.as_ref().is_some_and(|state| {
        state.editable_indexed.len() > 10000
            || state.items.len() > 10000
            || !state.source_items.is_empty()
            || !state.filter_stacks.is_empty()
            || state.floating.is_some()
            || !canonical
    }) {
        return Err(bad("source/filter palette replacement is not qualified"));
    }
    if matches!(snapshot.depth, SampleType::U32 | SampleType::F64) {
        return Err(bad("wide palette replacement is not qualified"));
    }
    let mut layer_count = 0;
    source_context(&snapshot.layers, 0, &mut layer_count, canonical, control)?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(snapshot, limits, control).map_err(|e| error(e, control))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, limits, control).map_err(|e| error(e, control))? as u64;
    let footprint = snapshot.resident_raster_footprint_with(control)?;
    let mut peak = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(footprint.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|n| n.checked_add((layer_count as u64).checked_mul(512)?))
        .ok_or_else(|| bad("palette source/model budget overflow"))?;
    let mut work = (layer_count as u64)
        .checked_mul(layer_count as u64)
        .and_then(|n| n.checked_mul(8))
        .filter(|n| *n <= 1_000_000_000)
        .ok_or_else(|| bad("palette owner/lock lookup work limit"))?;
    let tile_pixels = (TILE_SIZE as u64) * (TILE_SIZE as u64);
    if canonical {
        let required = crate::xcf_editing::palette_replacement_requirements(snapshot, table, control)?;
        peak = required
            .retained_bytes
            .checked_add(required.staged_bytes)
            .and_then(|n| n.checked_add(required.scratch_bytes))
            .and_then(|n| n.checked_add(model.checked_mul(3)?))
            .and_then(|n| n.checked_add(footprint.clone_metadata_bytes.checked_mul(3)?))
            .and_then(|n| n.checked_add((layer_count as u64).checked_mul(512)?))
            .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
            .ok_or_else(|| bad("canonical palette/clone headroom limit"))?;
        work = work.checked_add(required.work_units).filter(|n| *n <= 8_000_000_000).ok_or_else(|| bad("canonical palette aggregate work limit"))?;
    } else {
        let mut charge = |surface: &Surface, count: u64, tiles: u64, palette: u64| -> Result<()> {
            ordinary(surface)?;
            let staged = tile_pixels
                .checked_mul(surface.format().bytes_per_pixel() as u64)
                .and_then(|n| n.checked_add(1024))
                .and_then(|n| n.checked_mul(tiles))
                .and_then(|n| n.checked_mul(3))
                .and_then(|n| n.checked_add(count.checked_mul(10)?))
                .and_then(|n| n.checked_add(tile_pixels.checked_mul(160)?))
                .and_then(|n| n.checked_add(palette.checked_mul(6)?))
                .ok_or_else(|| bad("palette output headroom overflow"))?;
            peak = peak.checked_add(staged).ok_or_else(|| bad("palette output headroom overflow"))?;
            work = work
                .checked_add(
                    count.checked_mul(palette.checked_add(40).ok_or_else(|| bad("palette work overflow"))?).ok_or_else(|| bad("palette work overflow"))?,
                )
                .and_then(|n| n.checked_add(tiles.checked_mul(tile_pixels)?.checked_mul(16)?))
                .filter(|n| *n <= 1_000_000_000)
                .ok_or_else(|| bad("palette aggregate work limit"))?;
            if peak > MAX_CHECKED_RASTER_BYTES {
                return Err(bad("palette aggregate resident limit"));
            }
            Ok(())
        };
        for layer in &snapshot.layers {
            poll(control)?;
            if let LayerContent::Raster(surface) = &layer.content {
                let bounds = surface.tile_bounds();
                let count = u64::from(bounds.width()).checked_mul(u64::from(bounds.height())).ok_or_else(|| bad("legacy palette extent overflow"))?;
                photocraft_raster::checked_region_byte_len(bounds, surface.format()).map_err(bad)?;
                charge(
                    surface,
                    count,
                    if bounds.is_empty() { 0 } else { crate::xcf_indexed_pixel_cmds::tile_count(bounds)? },
                    snapshot.color_table.as_ref().map_or(0, |table| table.colors.len()) as u64,
                )?;
            }
        }
    }
    if canonical {
        let verified = crate::xcf_indexed_geometry::validate_current_with_limits(
            snapshot,
            photocraft_algo::wide_geometry::GeometryLimits {
                resident_bytes: MAX_CHECKED_RASTER_BYTES.checked_sub(peak).ok_or_else(|| bad("remaining palette residency"))?,
                work_units: 8_000_000_000u64
                    .checked_sub(work)
                    .ok_or_else(|| bad("remaining palette work"))?
                    .min(photocraft_algo::wide_geometry::GeometryLimits::default().work_units),
            },
            control,
        )?;
        peak = peak.checked_add(verified.scratch_bytes).filter(|n| *n <= MAX_CHECKED_RASTER_BYTES).ok_or_else(|| bad("palette verification scratch limit"))?;
        work = work.checked_add(verified.work_units).filter(|n| *n <= 8_000_000_000).ok_or_else(|| bad("palette verification work limit"))?;
        let old = snapshot.color_table.as_ref().ok_or_else(|| bad("missing old palette"))?;
        for item in &snapshot.xcf_state.as_ref().ok_or_else(|| bad("missing canonical state"))?.editable_indexed {
            poll(control)?;
            let locks = crate::xcf_item_locks::effective_layer_locks(snapshot, item.owner)?;
            locks.require(crate::xcf_item_locks::EditKind::Content)?;
            if old.transparent != table.transparent {
                locks.require(crate::xcf_item_locks::EditKind::Alpha)?;
            }
        }
    }
    if peak > MAX_CHECKED_RASTER_BYTES || work > 8_000_000_000 {
        return Err(bad("aggregate palette admission limit"));
    }
    poll(control)
}
fn legacy(surface: &mut Surface, old: &ColorTable, new: &ColorTable, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    ordinary(surface)?;
    surface.try_validate_tile_origins_with(control).map_err(|e| error(e, control))?;
    let bounds = surface.try_content_bounds_with_interrupt(control).map_err(|e| error(e, control))?;
    if bounds.is_empty() {
        return Ok(());
    }
    let format = surface.format();
    let sample = format.sample;
    let bytes_per_pixel = format.bytes_per_pixel();
    let channels = format.channels();
    for coordinate in bounds.tiles() {
        poll(control)?;
        let area = coordinate.rect().intersect(&bounds);
        let length = photocraft_raster::checked_region_byte_len(area, format).map_err(bad)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(bad)?;
        if bytes.capacity() > length.checked_mul(2).ok_or_else(|| bad("palette tile capacity"))? {
            return Err(bad("palette tile scratch limit"));
        }
        bytes.resize(length, 0);
        surface.try_read_interleaved_into_with(area, &mut bytes, control).map_err(|e| error(e, control))?;
        for chunk in bytes.chunks_mut(bytes_per_pixel * 1024) {
            poll(control)?;
            for pixel in chunk.chunks_exact_mut(bytes_per_pixel) {
                let mut values = [0.0; 5];
                for (component, native) in values.iter_mut().zip(pixel.chunks_exact(sample.bytes())) {
                    *component = read_sample(native, sample, 0);
                    if !component.is_finite() {
                        return Err(bad("nonfinite legacy palette input"));
                    }
                }
                let rgba = photocraft_raster::to_rgba(&format, values.get(..channels).ok_or_else(|| bad("palette channel count"))?);
                if rgba.iter().any(|n| !n.is_finite()) {
                    return Err(bad("nonfinite legacy palette conversion"));
                }
                // Preserve upstream's legacy palette rule: only a zero-alpha pixel
                // with the old transparent entry's exact RGB belongs to that entry.
                // Other hidden colours retain their independent zero alpha.
                let rgb = [rgba[0], rgba[1], rgba[2]];
                let index = if rgba[3] <= 0.0 {
                    let Some(index) = old.transparent.map(usize::from) else { continue };
                    if index >= old.colors.len() || old.rgb(index) != rgb {
                        continue;
                    }
                    index
                } else {
                    old.nearest(rgb)
                };
                let alpha = if new.transparent == Some(index as u8) {
                    0.0
                } else if rgba[3] <= 0.0 {
                    1.0
                } else {
                    rgba[3]
                };
                let rgb = if index < new.colors.len() { new.rgb(index) } else { new.rgb(new.nearest([rgba[0], rgba[1], rgba[2]])) };
                let mut result = [0.0; 5];
                photocraft_raster::from_rgba_into(&format, [rgb[0], rgb[1], rgb[2], alpha], &mut result);
                if result.iter().take(channels).any(|n| !n.is_finite()) {
                    return Err(bad("nonfinite legacy palette output"));
                }
                photocraft_raster::encode_pixel(&format, result.get(..channels).ok_or_else(|| bad("palette output channels"))?, pixel);
            }
        }
        surface.try_write_interleaved_with(area, &bytes, control).map_err(|e| error(e, control))?;
    }
    surface.try_prune_with(control).map_err(|e| error(e, control))?;
    poll(control)
}
fn stage(snapshot: &DocumentSnapshot, table: ColorTable, control: &Interrupt<'_>) -> Result<Document> {
    prepare(snapshot, &table, control)?;
    poll(control)?;
    let old = snapshot.color_table.as_ref().ok_or_else(|| bad("missing Color Table"))?;
    let mut doc = (**snapshot).clone();
    let handled = crate::xcf_editing::replace_palette_with_interrupt(&mut doc, &table, control)?;
    for layer in &mut doc.layers {
        poll(control)?;
        if !handled.contains(&layer.id)
            && let LayerContent::Raster(surface) = &mut layer.content
        {
            legacy(surface, old, &table, control)?;
        }
    }
    doc.color_table = Some(table);
    poll(control)?;
    Ok(doc)
}
pub(super) fn run(session: &mut Session, value: &Value) -> Result<Value> {
    context(session, value, false)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let current = state.doc.color_table.as_ref().ok_or_else(|| bad("missing Color Table"))?;
    let table = configuration(current, value)?;
    if &table == current {
        return report(current);
    }
    let output = report(&table)?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let weak = baseline.downgrade();
    let work = baseline.clone();
    crate::jobs::run(
        session,
        "Color Table",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Color Table", |control| {
                let document = stage(&work, table, control)?;
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
                || current.coalesce.is_some()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
            {
                return Err(bad("target changed while preparing Color Table"));
            }
            ctx.stage(0.95, 1.0, "Color Table", |control| {
                poll(control)?;
                session.edit_with_interrupt("Color Table", control, move |current, _| {
                    poll(control)?;
                    *current = document;
                    Ok(())
                })?;
                Ok(output)
            })
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::Document;
    fn canonical() -> Document {
        use photocraft_doc::{EditableIndexedLayer, EditableIndexedRaster, LayerId, item_identity::ItemRef};
        let bounds = photocraft_geom::Rect::new(-1, 0, 1, 1);
        let palette = vec![[255, 0, 0], [255, 0, 0]];
        let raster = EditableIndexedRaster::try_new(bounds, std::sync::Arc::new(vec![0, 1]), Some(std::sync::Arc::new(vec![255, 0])), palette.clone()).unwrap();
        let surface = crate::xcf_editing::expanded(&raster, photocraft_doc::PixelFormat::RGBA8, None).unwrap();
        let mut doc = Document::new("Own duplicate ordinals", photocraft_doc::Size::new(2, 1), ColorMode::Indexed, SampleType::U8);
        doc.color_table = Some(ColorTable { colors: palette, transparent: None });
        let layer = Layer::new("Own raster", LayerContent::Raster(surface));
        let owner: LayerId = layer.id;
        doc.layers.push(layer);
        let mut state = photocraft_doc::xcf_state::XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        state.items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).unwrap().raster_bounds = Some(bounds);
        state.editable_indexed.push(EditableIndexedLayer { owner, raster });
        doc.xcf_state = Some(state);
        doc
    }
    #[test]
    fn canonical_palette_edit_keeps_exact_duplicate_ordinals_independent_alpha_and_snapshot() {
        let snapshot = DocumentSnapshot::from_resident(std::sync::Arc::new(canonical())).unwrap();
        let before = (*snapshot).clone();
        let old = &snapshot.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
        let table = ColorTable { colors: vec![[0, 255, 0], [255, 255, 0]], transparent: None };
        let edited = stage(&snapshot, table, &Interrupt::NONE).unwrap();
        let new = &edited.xcf_state.as_ref().unwrap().editable_indexed[0].raster;
        assert!(std::sync::Arc::ptr_eq(old.indices(), new.indices()));
        assert!(std::sync::Arc::ptr_eq(old.alpha().unwrap(), new.alpha().unwrap()));
        assert_eq!(new.indices().as_slice(), &[0, 1]);
        assert_eq!(*snapshot, before);
        assert_eq!(edited.layers[0].surface().unwrap().try_read_interleaved(new.bounds()).unwrap(), [0, 255, 0, 255, 255, 255, 0, 0]);
        assert!(matches!(
            stage(&snapshot, ColorTable { colors: vec![[1, 2, 3]], transparent: None }, &Interrupt::cancel_only(&|| true)),
            Err(EngineError::Cancelled)
        ));
        assert_eq!(*snapshot, before);
    }
    #[test]
    fn identical_table_report_keeps_real_redo_and_revision() {
        let mut session = Session::new();
        session.add_document(canonical(), None).unwrap();
        session
            .edit("Owned history seed", |doc, _| {
                doc.name.push_str(" changed");
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let original = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let output = run(&mut session, &Value::Null).unwrap();
        assert_eq!(output["colors"].as_array().unwrap().len(), 2);
        assert!(original.shares_state(&session.active().unwrap().doc));
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.redo());
    }
    #[test]
    fn borrowed_params_refuse_oversized_and_wrong_typed_palette_before_clone() {
        let mut session = Session::new();
        let mut doc = Document::new("Legacy palette", photocraft_doc::Size::new(1, 1), ColorMode::Indexed, SampleType::U8);
        doc.color_table = Some(ColorTable { colors: vec![[0, 0, 0], [255, 255, 255]], transparent: None });
        session.add_document(doc, None).unwrap();
        for value in [
            json!({"colors":[]}),
            json!({"colors":vec!["#000000";257]}),
            json!({"entries":[]}),
            json!({"transparent":"0"}),
            json!({"entries":{"2":"#ffffff"}}),
            json!({"colors":[[1,2,3,4,5]]}),
        ] {
            assert!(dispatch_preflight(&session, &value, false).is_err());
        }
        assert!(dispatch_preflight(&session, &json!({"entries":{"0":"#123456"},"transparent":null}), false).is_ok());
    }
    #[test]
    fn legacy_tiled_replacement_keeps_hidden_zero_alpha_and_old_nearest_rule() {
        let old = ColorTable { colors: vec![[255, 0, 0], [255, 0, 0], [0, 0, 255]], transparent: None };
        let new = ColorTable { colors: vec![[0, 255, 0], [255, 255, 0], [0, 0, 255]], transparent: Some(0) };
        let bounds = photocraft_geom::Rect::new(-1, 0, 2, 1);
        let raw = [255, 0, 0, 255, 255, 0, 0, 0, 0, 0, 255, 128];
        let mut surface = Surface::try_from_interleaved(photocraft_doc::PixelFormat::RGBA8, bounds, &raw).unwrap();
        legacy(&mut surface, &old, &new, &Interrupt::NONE).unwrap();
        assert_eq!(surface.try_read_interleaved(bounds).unwrap(), [0, 255, 0, 0, 255, 0, 0, 0, 0, 0, 255, 128]);
        let before = surface.clone();
        assert!(matches!(legacy(&mut surface, &old, &new, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert_eq!(surface, before);
    }
    #[test]
    fn legacy_cleared_palette_alpha_restores_only_the_old_transparent_entry() {
        let old = ColorTable { colors: vec![[255, 0, 0], [0, 0, 255]], transparent: Some(0) };
        let new = ColorTable { colors: old.colors.clone(), transparent: None };
        let bounds = photocraft_geom::Rect::new(-1, 0, 2, 1);
        let mut surface = Surface::try_from_interleaved(photocraft_doc::PixelFormat::RGBA8, bounds, &[255, 0, 0, 0, 0, 0, 255, 0, 0, 0, 255, 128]).unwrap();
        legacy(&mut surface, &old, &new, &Interrupt::NONE).unwrap();
        assert_eq!(surface.try_read_interleaved(bounds).unwrap(), [255, 0, 0, 255, 0, 0, 255, 0, 0, 0, 255, 128]);
    }
}
