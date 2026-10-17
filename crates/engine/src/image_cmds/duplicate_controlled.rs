//! Controlled duplicate preparation. Merged output uses the existing ordinary
//! compositor only; wide/indexed/imported merged arithmetic is not qualified.
//! Known public model/payload/map capacity is admitted, not process RSS or every
//! private legacy renderer allocation. Compositor internal polling limits remain.
use super::{bad, duplicate_refs};
use crate::{EngineError, Result, Session};
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType};
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use serde_json::{Value, json};
const CMD: &str = "image.duplicate";
const NAME_BYTES: usize = 2048;
const PLAN_BYTES: u64 = 16 << 20;
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn error(control: &Interrupt<'_>, value: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(CMD, value.to_string()) }
}
fn merged(params: &Value) -> Result<bool> {
    match params.get("mergedOnly") {
        None => Ok(false),
        Some(value) => value.as_bool().ok_or_else(|| bad(CMD, "mergedOnly must be boolean")),
    }
}
/// Borrowed validation before generic floating normalization/parameter cloning.
pub(super) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    if !params.is_null()
        && !params.as_object().is_some_and(|object| {
            object.len() <= 4 && object.keys().all(|key| matches!(key.as_str(), "name" | "mergedOnly" | "documentId" | "expectedRevision"))
        })
    {
        return Err(bad(CMD, "expected bounded Duplicate parameters"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if session.command_has_pending_job() || session.command_has_job_on(state.doc.id) {
        return Err(bad(CMD, "source document has pending work"));
    }
    if let Some(name) = params.get("name") {
        if !name.as_str().is_some_and(|name| name.len() <= NAME_BYTES && !name.contains('\0')) {
            return Err(bad(CMD, "name must be bounded UTF-8 without NUL"));
        }
    } else if state.doc.name.len() > NAME_BYTES - 5 || state.doc.name.contains('\0') {
        return Err(bad(CMD, "default duplicate name exceeds limit"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(document), Some(revision)) => {
            let decimal = |value: &Value| -> Result<u64> {
                let text = value
                    .as_str()
                    .filter(|text| {
                        !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0'))
                    })
                    .ok_or_else(|| bad(CMD, "captured IDs must be canonical decimal strings"))?;
                text.parse().map_err(|_| bad(CMD, "captured ID exceeds range"))
            };
            if decimal(document)? == 0 || decimal(document)? != state.doc.id.0 || decimal(revision)? != state.revision {
                return Err(bad(CMD, "captured target changed"));
            }
            if state.floating.is_some()
                || state.coalesce.is_some()
                || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
            {
                return Err(bad(CMD, "captured source has floating or transient work"));
            }
        }
        _ => return Err(bad(CMD, "documentId and expectedRevision must be supplied together")),
    }
    if merged(params)?
        && (state.doc.xcf_state.is_some() || state.doc.mode == ColorMode::Indexed || matches!(state.doc.depth, SampleType::U32 | SampleType::F64))
    {
        return Err(bad(CMD, "merged imported/indexed/wide arithmetic is not qualified"));
    }
    Ok(())
}
fn copy_name(name: &str, default: bool, control: &Interrupt<'_>) -> Result<String> {
    poll(control)?;
    let length = name.len().checked_add(if default { 5 } else { 0 }).filter(|length| *length <= NAME_BYTES).ok_or_else(|| bad(CMD, "duplicate name limit"))?;
    let mut result = String::new();
    result.try_reserve_exact(length).map_err(|_| bad(CMD, "name allocation"))?;
    if result.capacity() > NAME_BYTES + 64 {
        return Err(bad(CMD, "name capacity limit"));
    }
    result.push_str(name);
    if default {
        result.push_str(" copy");
    }
    poll(control)?;
    Ok(result)
}
fn admit(doc: &Document, merged: bool, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let raster = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let mut resident = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|n| n.checked_add(PLAN_BYTES))
        .ok_or_else(|| bad(CMD, "clone admission overflow"))?;
    let mut layers = 0u64;
    photocraft_doc::resident_structure::visit_layers_with(doc, control, |_| {
        layers = layers.checked_add(1).filter(|n| *n <= 10_000).ok_or(RasterInputError::LimitExceeded)?;
        Ok(true)
    })
    .map_err(|e| error(control, e))?;
    let validation_work = payload
        .checked_add(model)
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| n.checked_add(layers.checked_mul(4096)?))
        .filter(|n| *n <= 8_000_000_000)
        .ok_or_else(|| bad(CMD, "duplicate census/hash work limit"))?;
    if merged {
        if doc.xcf_state.is_some() || doc.mode == ColorMode::Indexed || matches!(doc.depth, SampleType::U32 | SampleType::F64) {
            return Err(bad(CMD, "merged imported/indexed/wide arithmetic is not qualified"));
        }
        if duplicate_refs::has_associations_with_interrupt(doc, control)? {
            return Err(bad(CMD, "unsupported merged current layer associations"));
        }
        let bounds = doc.bounds();
        if bounds.is_empty() || doc.size.width > 300_000 || doc.size.height > 300_000 {
            return Err(bad(CMD, "merged canvas extent unsupported"));
        }
        let tiles = super::canvas_tiles(bounds).map_err(|e| error(control, e))?;
        let tile_bytes = (photocraft_geom::TILE_SIZE as u64)
            .checked_mul(photocraft_geom::TILE_SIZE as u64)
            .and_then(|n| n.checked_mul(doc.pixel_format().bytes_per_pixel() as u64))
            .ok_or_else(|| bad(CMD, "merged tile bytes"))?;
        let band_rows = u64::from(doc.size.height).min(photocraft_geom::TILE_SIZE as u64);
        let scratch = u64::from(doc.size.width)
            .checked_mul(band_rows)
            .and_then(|n| n.checked_mul(16 + doc.pixel_format().bytes_per_pixel() as u64))
            .and_then(|n| n.checked_add(4096))
            .ok_or_else(|| bad(CMD, "merged scratch overflow"))?;
        resident = resident
            .checked_add(
                tiles.checked_mul(tile_bytes.checked_add(1024).ok_or_else(|| bad(CMD, "merged ledger"))?).ok_or_else(|| bad(CMD, "merged output overflow"))?,
            )
            .and_then(|n| n.checked_add(scratch))
            .ok_or_else(|| bad(CMD, "merged peak overflow"))?;
        let bands = u64::from(doc.size.height).div_ceil(photocraft_geom::TILE_SIZE as u64);
        let pixels = u64::from(doc.size.width).checked_mul(u64::from(doc.size.height)).ok_or_else(|| bad(CMD, "merged pixel count"))?;
        let work = pixels
            .checked_mul(layers.max(1))
            .and_then(|n| n.checked_mul(64))
            .and_then(|n| n.checked_add(bands.checked_mul(tiles)?.checked_mul(256)?))
            .and_then(|n| n.checked_add(validation_work))
            .filter(|n| *n <= 8_000_000_000)
            .ok_or_else(|| bad(CMD, "merged work limit"))?;
        let _ = work;
    }
    if resident > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(bad(CMD, "duplicate retained/model/output admission limit"));
    }
    poll(control)
}
fn merge_surface(doc: &Document, control: &Interrupt<'_>) -> Result<Surface> {
    let format = doc.pixel_format();
    let channels = format.channels();
    let bpp = format.bytes_per_pixel();
    let maximum = usize::try_from(doc.size.width)
        .ok()
        .and_then(|n| n.checked_mul((doc.size.height as usize).min(photocraft_geom::TILE_SIZE as usize)))
        .and_then(|n| n.checked_mul(bpp))
        .ok_or_else(|| bad(CMD, "merged band size"))?;
    let mut encoded = Vec::new();
    poll(control)?;
    encoded.try_reserve_exact(maximum).map_err(|_| bad(CMD, "merged band allocation"))?;
    if encoded.capacity() > maximum.saturating_add(4096) || encoded.capacity() as u64 > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(bad(CMD, "merged band capacity"));
    }
    while encoded.len() < maximum {
        poll(control)?;
        encoded.resize(encoded.len().saturating_add(32 << 10).min(maximum), 0);
    }
    let default = [0u8; 40];
    let mut output =
        Surface::try_with_default_bytes(format, default.get(..bpp).ok_or_else(|| bad(CMD, "merged pixel format"))?).map_err(|e| error(control, e))?;
    photocraft_compose::render_bands_with_interrupt(doc, doc.bounds(), photocraft_geom::TILE_SIZE, control, |buffer| {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let length = photocraft_raster::checked_region_byte_len(buffer.rect, format)?;
        let target = encoded.get_mut(..length).ok_or(RasterInputError::LengthMismatch)?;
        if bpp == 0 || target.len() / bpp != buffer.px.len() {
            return Err(RasterInputError::LengthMismatch);
        }
        for (index, (rgba, native)) in buffer.px.iter().zip(target.chunks_exact_mut(bpp)).enumerate() {
            if index.is_multiple_of(4096) {
                control.check().map_err(|_| RasterInputError::Cancelled)?;
            }
            if rgba.iter().any(|v| !v.is_finite()) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let mut values = [0f32; 8];
            let count = photocraft_raster::from_rgba_into(&format, *rgba, &mut values);
            if count != channels {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let values = values.get(..count).ok_or(RasterInputError::LengthMismatch)?;
            if values.iter().any(|value| !value.is_finite()) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            photocraft_raster::encode_pixel(&format, values, native);
        }
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        output.try_write_interleaved_with(buffer.rect, target, control)
    })
    .map_err(|e| error(control, e))?;
    output.try_prune_with(control).map_err(|e| error(control, e))?;
    poll(control)?;
    Ok(output)
}
pub(super) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    dispatch_preflight(session, params, session.command_is_background())?;
    let merge = merged(params)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    let baseline = active.doc.clone();
    let revision = active.revision;
    let name = copy_name(params.get("name").and_then(Value::as_str).unwrap_or(&baseline.name), params.get("name").is_none(), &Interrupt::NONE)?;
    let work_base = baseline.clone();
    crate::jobs::run(
        session,
        "Duplicate",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Duplicate", |control| {
                admit(&work_base, merge, control)?;
                let plan = if merge { None } else { Some(duplicate_refs::Plan::new_with_interrupt(&work_base, control)?) };
                let merged_surface = if merge { Some(merge_surface(&work_base, control)?) } else { None };
                poll(control)?;
                let mut document = (*work_base).clone();
                document.id = photocraft_doc::DocId::fresh();
                document.name = name;
                if let Some(surface) = merged_surface {
                    let mut layers = Vec::new();
                    layers.try_reserve_exact(1).map_err(|_| bad(CMD, "merged layer allocation"))?;
                    layers.push(Layer::new("Background", LayerContent::Raster(surface)));
                    document.layers = layers;
                    // These layouts addressed discarded source layers; the source keeps them.
                    document.layer_comps.clear();
                    document.last_applied_comp = None;
                    document.last_document_state = None;
                    // Variable bindings address layers discarded by the merged-only copy.
                    document.variables = photocraft_doc::Variables::default();
                }
                if let Some(plan) = plan {
                    plan.apply_with_interrupt(&mut document, control)?;
                }
                poll(control)?;
                Ok((document, ctx.clone()))
            })
        },
        move |session, (document, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id || current.revision != revision || !current.doc.shares_state(&baseline) {
                return Err(bad(CMD, "source changed while preparing duplicate"));
            }
            ctx.stage(0.95, 1.0, "Duplicate", |control| {
                poll(control)?;
                let response_name = copy_name(&document.name, false, control)?;
                let index = session.add_document_with_interrupt(document, None, control)?;
                // Publication is complete: do not turn a later cancellation into failure.
                Ok(json!({"document":index,"name":response_name}))
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{LayerId, Size};
    fn fixture(sample: SampleType) -> Session {
        let mut doc = Document::new("original", Size::new(2, 1), ColorMode::Rgb, sample);
        let mut layer = Layer::raster("raw", doc.pixel_format());
        layer.id = LayerId(9);
        let raw = match sample {
            SampleType::U32 => [1u32, 2, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>(),
            SampleType::F64 => [1f64.to_bits() + 1, (-0f64).to_bits(), 0x7ff8_0000_0000_1234, 1f64.to_bits()].into_iter().flat_map(u64::to_le_bytes).collect(),
            _ => vec![1, 2, 3, 255],
        };
        layer.surface_mut().unwrap().try_write_interleaved_with(photocraft_geom::Rect::new(0, 0, 1, 1), &raw, &Interrupt::NONE).unwrap();
        doc.layers.push(layer);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
    }
    #[test]
    fn raw_wide_duplicate_remaps_current_association_without_original_undo() {
        for sample in [SampleType::U32, SampleType::F64] {
            let mut session = fixture(sample);
            session
                .edit("variable", |doc, _| {
                    doc.variables.defs.push(photocraft_doc::variables::VariableDef {
                        name: "visible".into(),
                        layer: LayerId(9),
                        kind: photocraft_doc::variables::VarKind::Visibility,
                    });
                    Ok(())
                })
                .unwrap();
            let state = session.active().unwrap();
            let source = state.doc.clone();
            let revision = state.revision;
            let labels = state.history.entry_labels().map(str::to_owned).collect::<Vec<_>>();
            run(&mut session, &json!({"documentId":source.id.0.to_string(),"expectedRevision":revision.to_string()})).unwrap();
            assert_eq!(session.docs.len(), 2);
            let copied = &session.active().unwrap().doc;
            assert_ne!(copied.id, source.id);
            assert_ne!(copied.layers[0].id, source.layers[0].id);
            assert_eq!(copied.variables.defs[0].layer, copied.layers[0].id);
            assert_eq!(copied.layers[0].surface(), source.layers[0].surface());
            assert_eq!(session.docs[0].revision, revision);
            assert!(session.docs[0].doc.shares_state(&source));
            assert_eq!(session.docs[0].history.entry_labels().map(str::to_owned).collect::<Vec<_>>(), labels);
        }
    }
    #[test]
    fn merged_wide_stale_partial_and_bad_metadata_refuse_without_new_tab() {
        let mut session = fixture(SampleType::U32);
        let source = session.active().unwrap().doc.clone();
        for params in [
            json!({"mergedOnly":true}),
            json!({"mergedOnly":0}),
            json!({"name":"x".repeat(NAME_BYTES+1)}),
            json!({"documentId":source.id.0.to_string()}),
            json!({"documentId":source.id.0.to_string(),"expectedRevision":"99"}),
        ] {
            assert!(run(&mut session, &params).is_err());
            assert_eq!(session.docs.len(), 1);
            assert!(session.active().unwrap().doc.shares_state(&source));
        }
    }
    #[test]
    fn cancellation_during_admission_and_remap_never_publishes_or_mutates_source() {
        let session = fixture(SampleType::U32);
        let source = session.active().unwrap().doc.clone();
        let cancel = Interrupt::cancel_only(&|| true);
        assert!(matches!(admit(&source, false, &cancel), Err(EngineError::Cancelled)));
        assert!(matches!(duplicate_refs::Plan::new_with_interrupt(&source, &cancel), Err(EngineError::Cancelled)));
        let plan = duplicate_refs::Plan::new_with_interrupt(&source, &Interrupt::NONE).unwrap();
        let mut private = (*source).clone();
        assert!(matches!(plan.apply_with_interrupt(&mut private, &cancel), Err(EngineError::Cancelled)));
        assert_eq!(private, *source);
        assert!(session.active().unwrap().doc.shares_state(&source));
    }
    #[test]
    fn bounded_band_merge_matches_the_legacy_ordinary_native_values() {
        let session = fixture(SampleType::U8);
        let source = &session.active().unwrap().doc;
        admit(source, true, &Interrupt::NONE).unwrap();
        let merged = merge_surface(source, &Interrupt::NONE).unwrap();
        let buffer = photocraft_compose::flatten(source).unwrap();
        let format = source.pixel_format();
        let values: Vec<_> = buffer.px.iter().flat_map(|rgba| photocraft_raster::from_rgba(&format, *rgba)).collect();
        let mut old = Surface::new(format);
        old.write_region(source.bounds(), &values);
        old.prune();
        assert_eq!(merged, old);
    }
}
