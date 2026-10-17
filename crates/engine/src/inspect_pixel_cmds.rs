//! One-pixel precision inspection; no legacy f32 sampler or external reads.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_cms::WideWork;
use photocraft_compose::{gimp_composite::CompositionError, gimp_render::PreparedRenderer};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, MAX_GROUP_DEPTH};
use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
use std::{
    fmt::Write,
    sync::atomic::{AtomicU64, Ordering},
};

const ID: &str = "document.pixel.inspect";
const MAX_LAYERS: usize = 10_000;
const MAX_NATIVE_BYTES: usize = 2048;
const MAX_CHECKS: u64 = 4096;
const MAX_RENDER_BYTES: u64 = 16 << 20;
const MAX_SAMPLE_UNITS: u64 = 1_000_000;

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: message.into() }
}

#[derive(Clone, Copy)]
struct Params {
    x: i32,
    y: i32,
    layer: Option<LayerId>,
    native_only: bool,
}

fn params(value: &Value) -> Result<Params> {
    // Match the command dispatch convention without cloning caller-owned JSON.
    if value.is_null() {
        return Ok(Params { x: 0, y: 0, layer: None, native_only: false });
    }
    let object = value.as_object().ok_or_else(|| bad("expected an object"))?;
    if object.len() > 4 {
        return Err(bad("inspection accepts at most four fields"));
    }
    if object.keys().any(|key| !matches!(key.as_str(), "x" | "y" | "layerId" | "view")) {
        return Err(bad("unknown inspection parameter"));
    }
    let coordinate = |key: &str| -> Result<i32> {
        match object.get(key) {
            None => Ok(0),
            Some(value) => value.as_i64().and_then(|value| i32::try_from(value).ok()).ok_or_else(|| bad(format!("{key} must be an i32 integer"))),
        }
    };
    let layer = object
        .get("layerId")
        .map(|value| -> Result<LayerId> {
            let text = value.as_str().ok_or_else(|| bad("layerId must be a canonical decimal string"))?;
            if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(bad("layerId must be a nonzero canonical decimal U64"));
            }
            text.parse::<u64>().map(LayerId).map_err(|_| bad("layerId exceeds U64"))
        })
        .transpose()?;
    let native_only = match object.get("view") {
        None => false,
        Some(Value::String(view)) if view == "composite" => false,
        Some(Value::String(view)) if view == "native" => true,
        _ => return Err(bad("view must be composite or native")),
    };
    if native_only && layer.is_none() {
        return Err(bad("native view requires explicit layerId"));
    }
    Ok(Params { x: coordinate("x")?, y: coordinate("y")?, layer, native_only })
}

/// Cheap envelope admission before the dispatcher clones caller-owned JSON.
pub(crate) fn dispatch_preflight(value: &Value) -> Result<()> {
    let p = params(value)?;
    p.x.checked_add(1).ok_or_else(|| bad("x cannot form a half-open pixel rectangle"))?;
    p.y.checked_add(1).ok_or_else(|| bad("y cannot form a half-open pixel rectangle"))?;
    Ok(())
}

fn layer<'a>(doc: &'a Document, wanted: Option<LayerId>, control: &Interrupt<'_>) -> Result<Option<&'a Layer>> {
    if doc.layers.len() > MAX_LAYERS {
        return Err(bad("layer traversal limit"));
    }
    let mut pending = Vec::new();
    pending.try_reserve_exact(doc.layers.len()).map_err(|_| bad("layer traversal allocation"))?;
    pending.extend(doc.layers.iter().map(|layer| (layer, 0usize)));
    let mut total = 0usize;
    let mut found = None;
    while let Some((current, depth)) = pending.pop() {
        control.check().map_err(|_| EngineError::Cancelled)?;
        total = total.checked_add(1).ok_or_else(|| bad("layer traversal limit"))?;
        if total > MAX_LAYERS || depth > MAX_GROUP_DEPTH {
            return Err(bad("layer traversal limit"));
        }
        if wanted == Some(current.id) {
            if found.is_some() {
                return Err(bad("ambiguous layer identity"));
            }
            found = Some(current);
        }
        if let Some(children) = current.children() {
            let remaining = pending.len().checked_add(children.len()).and_then(|n| n.checked_add(total)).ok_or_else(|| bad("layer traversal limit"))?;
            if remaining > MAX_LAYERS {
                return Err(bad("layer traversal limit"));
            }
            let next = depth.checked_add(1).filter(|n| *n <= MAX_GROUP_DEPTH).ok_or_else(|| bad("layer depth limit"))?;
            pending.try_reserve(children.len()).map_err(|_| bad("layer traversal allocation"))?;
            pending.extend(children.iter().map(|child| (child, next)));
        }
    }
    if wanted.is_some() && found.is_none() {
        return Err(bad("no current layer with layerId"));
    }
    Ok(found)
}

fn native(doc: &Document, selected: &Layer, x: i32, y: i32) -> Result<Value> {
    let LayerContent::Raster(surface) = &selected.content else {
        return Err(bad("native view requires a current resident raster, not a text/shape cache or backed material"));
    };
    let format = surface.format();
    let count = format.bytes_per_pixel();
    if count == 0 || count > MAX_NATIVE_BYTES || surface.default_bytes().len() != count {
        return Err(bad("invalid native pixel storage envelope"));
    }
    let mut storage = [0u8; MAX_NATIVE_BYTES];
    let bytes = storage.get_mut(..count).ok_or_else(|| bad("native pixel byte limit"))?;
    surface.try_read_pixel_bytes(x, y, bytes).map_err(|error| bad(error.to_string()))?;
    let hex_len = count.checked_mul(2).ok_or_else(|| bad("native pixel hex limit"))?;
    let mut hex = String::new();
    hex.try_reserve_exact(hex_len).map_err(|_| bad("native pixel allocation"))?;
    for byte in bytes {
        write!(&mut hex, "{byte:02x}").map_err(|_| bad("native pixel formatting"))?;
    }
    Ok(json!({"layerId":selected.id.0.to_string(),"model":format!("{:?}",format.mode),"sampleType":format!("{:?}",format.sample),
        "channels":format.channels(),"hasAlpha":format.alpha,"sampleEncoding":doc.sample_encoding,
        "byteOrder":if cfg!(target_endian="little") {"littleEndian"} else {"bigEndian"},"encodedHex":hex,
        "meaning":"current resident raster storage, before composition; unallocated tiles return its stored default pixel"}))
}

pub fn inspect_with_interrupt(session: &Session, value: &Value, external: &Interrupt<'_>) -> Result<Value> {
    let p = params(value)?;
    let x1 = p.x.checked_add(1).ok_or_else(|| bad("x cannot form a half-open pixel rectangle"))?;
    let y1 = p.y.checked_add(1).ok_or_else(|| bad("y cannot form a half-open pixel rectangle"))?;
    external.check().map_err(|_| EngineError::Cancelled)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let doc = &state.doc;
    // A resident snapshot is a cheap carrier check. Native-only inspection needs
    // no validation/sample reads of unrelated resources, masks or raster caches.
    if doc.resident_arc().is_none() {
        return Err(bad("inspection requires a resident snapshot"));
    }
    let selected = layer(doc, p.layer, external)?;
    let native = selected.map(|selected| native(doc, selected, p.x, p.y)).transpose()?;
    let indexed = selected.map(|selected| current_indexed(doc, selected.id, p.x, p.y, external)).transpose()?.flatten();
    if p.native_only {
        external.check().map_err(|_| EngineError::Cancelled)?;
        return Ok(json!({"documentId":doc.id.0.to_string(),"revisionString":state.revision.to_string(),"position":[p.x,p.y],
            "view":"native","nativeSample":native,"currentIndexed":indexed,"compositeRgbaF64":null,"compositeRequested":false}));
    }
    let checks = AtomicU64::new(0);
    let cancel = || external.cancelled() || checks.fetch_add(1, Ordering::Relaxed) >= MAX_CHECKS;
    let control = Interrupt::cancel_only(&cancel);
    let error = |error: CompositionError| {
        if external.cancelled() {
            EngineError::Cancelled
        } else if checks.load(Ordering::Relaxed) > MAX_CHECKS {
            bad("source/render check limit")
        } else {
            bad(error.to_string())
        }
    };
    let renderer = PreparedRenderer::new_with_interrupt(doc, &control).map_err(error)?;
    let mut reserved = 0u64;
    let mut units = 0u64;
    let mut poll = |work| -> std::result::Result<(), CompositionError> {
        control.check().map_err(|_| CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled))?;
        match work {
            WideWork::ReserveBytes(bytes) => {
                reserved = reserved.checked_add(bytes).filter(|n| *n <= MAX_RENDER_BYTES).ok_or(CompositionError::Invalid("pixel query render byte limit"))?;
            }
            WideWork::ReleaseBytes(bytes) => {
                reserved = reserved.checked_sub(bytes).ok_or(CompositionError::Invalid("pixel query render accounting"))?;
            }
            WideWork::SampleUnits(amount) => {
                units = units.checked_add(amount).filter(|n| *n <= MAX_SAMPLE_UNITS).ok_or(CompositionError::Invalid("pixel query sample work limit"))?;
            }
        }
        Ok(())
    };
    let buffer = renderer.render_with_interrupt(Rect::new(p.x, p.y, x1, y1), &mut poll, &control).map_err(error)?;
    let pixel = buffer.px.first().copied().ok_or_else(|| bad("qualified compositor returned no pixel"))?;
    if pixel.iter().any(|component| !component.is_finite()) {
        return Err(bad("nonfinite composite cannot be represented as JSON numbers; use explicit native view for stored bits"));
    }
    external.check().map_err(|_| EngineError::Cancelled)?;
    Ok(json!({"documentId":doc.id.0.to_string(),"revisionString":state.revision.to_string(),"position":[p.x,p.y],"view":"composite",
        "compositeRgbaF64":pixel,"compositeFloat64Bits":pixel.map(|value|format!("0x{:016x}",value.to_bits())),"compositeRequested":true,
        "compositeMeaning":"qualified compositor RGBA output, not native encoded samples","nativeSample":native,"currentIndexed":indexed}))
}

fn current_indexed(document: &Document, layer: LayerId, x: i32, y: i32, control: &Interrupt<'_>) -> Result<Option<Value>> {
    let Some(state) = document.xcf_state.as_ref() else {
        return Ok(None);
    };
    if state.editable_indexed.len() > MAX_LAYERS {
        return Err(bad("indexed query owner limit"));
    }
    let mut selected = None;
    for indexed in &state.editable_indexed {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if indexed.owner == layer && selected.replace(&indexed.raster).is_some() {
            return Err(bad("ambiguous current indexed owner"));
        }
    }
    let Some(raster) = selected else {
        return Ok(None);
    };
    if document.mode != photocraft_color::ColorMode::Indexed || document.depth != photocraft_color::SampleType::U8 {
        return Err(bad("invalid current indexed context"));
    }
    let bounds = raster.bounds();
    let width = i64::from(bounds.x1) - i64::from(bounds.x0);
    let height = i64::from(bounds.y1) - i64::from(bounds.y0);
    let count = width
        .checked_mul(height)
        .and_then(|count| usize::try_from(count).ok())
        .filter(|count| width > 0 && height > 0 && width <= 262144 && height <= 262144 && *count <= 64 << 20)
        .ok_or_else(|| bad("invalid current indexed extent"))?;
    if raster.indices().len() != count
        || raster.alpha().is_some_and(|alpha| alpha.len() != count)
        || raster.palette().is_empty()
        || raster.palette().len() > 256
    {
        return Err(bad("invalid current indexed planes"));
    }
    let extent = json!([bounds.x0, bounds.y0, bounds.x1, bounds.y1]);
    if x < bounds.x0 || x >= bounds.x1 || y < bounds.y0 || y >= bounds.y1 {
        return Ok(Some(json!({"insideBounds":false,"bounds":extent,"index":null,"independentAlpha":null,
            "meaning":"outside the authoritative current indexed plane; no ordinal default is inferred"})));
    }
    let offset = (i64::from(y) - i64::from(bounds.y0))
        .checked_mul(width)
        .and_then(|row| row.checked_add(i64::from(x) - i64::from(bounds.x0)))
        .and_then(|offset| usize::try_from(offset).ok())
        .ok_or_else(|| bad("indexed query offset overflow"))?;
    let index = *raster.indices().get(offset).ok_or_else(|| bad("missing current ordinal"))?;
    let color = *raster.palette().get(usize::from(index)).ok_or_else(|| bad("current ordinal outside palette"))?;
    let alpha = raster.alpha().map_or(Ok(255), |values| values.get(offset).copied().ok_or_else(|| bad("missing current alpha")))?;
    let effective_alpha = if document.color_table.as_ref().and_then(|table| table.transparent) == Some(index) { 0 } else { alpha };
    control.check().map_err(|_| EngineError::Cancelled)?;
    Ok(Some(json!({"insideBounds":true,"bounds":extent,"index":index,"independentAlpha":alpha,"hasIndependentAlpha":raster.alpha().is_some(),
        "paletteRgbU8":color,"effectiveAlpha":effective_alpha,"paletteEntries":raster.palette().len(),
        "meaning":"current logical indexed ordinal and independent alpha; duplicate palette entries retain distinct ordinals"})))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Inspect Pixel Precision",
        menu: &[],
        shortcut: None,
        params: r#"{x?:i32=0,y?:i32=0,layerId?:canonical nonzero decimal U64 string,view?:composite|native=composite} -> qualified f64 RGBA/bits plus optional current resident raster encoded sample; native-only requires layerId and skips compositor. Off-canvas coordinates are explicit document-space samples; absent raster tiles return stored default. No f32 fallback or external reads."#,
        enabled: |session| session.active().map(|_| ()).ok_or_else(|| "no document open".into()),
        journal: false,
        run: |session, params| inspect_with_interrupt(session, params, &Interrupt::NONE),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexed_native_inspection_distinguishes_duplicate_ordinals_and_has_no_offbounds_default() {
        use photocraft_doc::{ColorTable, EditableIndexedLayer, EditableIndexedRaster, XcfState};
        use std::sync::Arc;
        let format = photocraft_color::PixelFormat::RGBA8;
        let raster =
            EditableIndexedRaster::try_new(Rect::new(-1, 1, 1, 2), Arc::new(vec![0, 1]), Some(Arc::new(vec![255, 128])), vec![[255, 0, 0], [255, 0, 0]])
                .unwrap();
        let mut document =
            Document::new("Duplicate ordinals", photocraft_doc::Size::new(2, 2), photocraft_color::ColorMode::Indexed, photocraft_color::SampleType::U8);
        let mut owner = Layer::raster("Indexed", format);
        owner.content = LayerContent::Raster(crate::xcf_editing::expanded_with(&raster, format, None, &Interrupt::NONE).unwrap());
        let layer = owner.id;
        document.layers.push(owner);
        document.color_table = Some(ColorTable { colors: vec![[255, 0, 0], [255, 0, 0]], transparent: Some(1) });
        let mut imported = XcfState::default();
        imported.reconcile_layers(&document).unwrap();
        imported.editable_indexed.push(EditableIndexedLayer { owner: layer, raster });
        document.xcf_state = Some(imported);
        let before = document.clone();
        let result = current_indexed(&document, layer, 0, 1, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(result["index"], 1);
        assert_eq!(result["independentAlpha"], 128);
        assert_eq!(result["effectiveAlpha"], 0);
        assert_eq!(result["paletteRgbU8"], json!([255, 0, 0]));
        let outside = current_indexed(&document, layer, 1, 1, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(outside["insideBounds"], false);
        assert!(outside["index"].is_null());
        assert!(matches!(current_indexed(&document, layer, 0, 1, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert_eq!(document, before);
        document.xcf_state.as_mut().unwrap().editable_indexed.push(before.xcf_state.as_ref().unwrap().editable_indexed[0].clone());
        assert!(current_indexed(&document, layer, 0, 1, &Interrupt::NONE).is_err());
    }
    #[test]
    fn strict_coordinates_identity_and_explicit_native_mode() {
        assert!(params(&json!({})).is_ok());
        assert!(dispatch_preflight(&Value::Null).is_ok());
        let empty = params(&Value::Null).unwrap();
        assert_eq!((empty.x, empty.y, empty.layer, empty.native_only), (0, 0, None, false));
        assert!(params(&json!({"view":"native","layerId":"18446744073709551615"})).is_ok());
        for p in [
            json!({"x":1.5}),
            json!({"x":"1"}),
            json!({"x":2147483648_i64}),
            json!({"layerId":"01"}),
            json!({"layerId":1}),
            json!({"view":"native"}),
            json!({"view":"fallback"}),
            json!({"path":"ambient"}),
        ] {
            assert!(params(&p).is_err());
        }
    }

    #[test]
    fn layer_identity_traversal_refuses_duplicates_caps_and_cancellation() {
        let mut doc = Document::new("bounded", photocraft_doc::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let selected = Layer::raster("selected", photocraft_color::PixelFormat::RGBA8);
        let id = selected.id;
        doc.layers = vec![selected.clone(), selected];
        assert!(layer(&doc, Some(id), &Interrupt::NONE).is_err());
        doc.layers.truncate(1);
        let stop = || true;
        assert!(matches!(layer(&doc, Some(id), &Interrupt::cancel_only(&stop)), Err(EngineError::Cancelled)));
        doc.layers.resize(MAX_LAYERS + 1, doc.layers.first().unwrap().clone());
        assert!(layer(&doc, Some(id), &Interrupt::NONE).is_err());
    }

    #[test]
    fn native_query_cancellation_preserves_source_and_real_redo() {
        let mut session = Session::new();
        session
            .open_document(
                Document::with_background(
                    "native",
                    photocraft_doc::Size::new(1, 1),
                    photocraft_color::ColorMode::Rgb,
                    photocraft_color::SampleType::U8,
                    photocraft_color::Color::WHITE,
                ),
                None,
            )
            .unwrap();
        session
            .edit("Real redo", |doc, _| {
                doc.name = "renamed".into();
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let id = session.active().unwrap().active_layer.unwrap();
        let stop = || true;
        assert!(matches!(
            inspect_with_interrupt(&session, &json!({"view":"native","layerId":id.0.to_string()}), &Interrupt::cancel_only(&stop)),
            Err(EngineError::Cancelled)
        ));
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.redo());
        assert_eq!(session.active().unwrap().doc.name, "renamed");
    }
    #[test]
    fn native_raw_default_preserves_f64_nonfinite_and_negative_zero_bits() {
        let format = photocraft_color::PixelFormat::RGBA64F;
        let raw: Vec<u8> = [0x8000000000000000_u64, 0x7ff8000000000042, 1.0_f64.to_bits(), 1.0_f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect();
        let mut selected = Layer::raster("native", format);
        selected.content = LayerContent::Raster(photocraft_raster::Surface::try_with_default_bytes(format, &raw).unwrap());
        let doc = Document::new("native", photocraft_doc::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::F64);
        let result = native(&doc, &selected, -3, 5).unwrap();
        assert_eq!(result["encodedHex"], raw.iter().map(|b| format!("{b:02x}")).collect::<String>());
    }

    #[test]
    fn registered_native_query_keeps_stale_floating_preview_and_real_redo() {
        let mut session = Session::new();
        session
            .open_document(
                Document::with_background(
                    "query",
                    photocraft_doc::Size::new(3, 2),
                    photocraft_color::ColorMode::Rgb,
                    photocraft_color::SampleType::U8,
                    photocraft_color::Color::WHITE,
                ),
                None,
            )
            .unwrap();
        session.execute("select.all", json!({})).unwrap();
        session
            .edit("Real selected redo", |doc, _| {
                doc.name = "query redo".into();
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        session.execute("select.float", json!({"dx":1,"dy":0,"copy":true})).unwrap();
        session.active_mut().unwrap().floating.as_mut().unwrap().revision = u64::MAX;
        let state = session.active().unwrap();
        let floating = state.floating.clone().unwrap();
        let source = state.doc.clone();
        let revision = state.revision;
        let redo = state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
        let layer = state.active_layer.unwrap();
        session.execute(ID, json!({"view":"native","layerId":layer.0.to_string(),"x":0,"y":0})).unwrap();
        session.execute(ID, json!({"view":"native","layerId":layer.0.to_string(),"x":"invalid"})).unwrap_err();
        let state = session.active().unwrap();
        assert_eq!(state.doc, source);
        assert_eq!(state.revision, revision);
        assert_eq!(state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
        let after = state.floating.as_ref().unwrap();
        assert_eq!((after.layer, after.offset, after.revision), (floating.layer, floating.offset, floating.revision));
        assert!(std::sync::Arc::ptr_eq(&after.parts, &floating.parts));
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.name, "query redo");
    }
}
