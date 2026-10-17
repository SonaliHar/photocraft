//! Explicit current linked geometry from owned material; original source state stays inert.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    Document, LayerContent, LayerId, Rect,
    item_identity::ItemRef,
    xcf_items::{EditableSource, LinkedCurrentState, SourceItem, TransformState},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::Other(message.into())
}
/// Protocol geometry keeps IEEE bits in strings; native model storage remains u64.
pub(crate) fn current_geometry_json(current: &LinkedCurrentState) -> Value {
    let transform = current.transform.as_ref().map(|transform| {
        let name = match transform.interpolation {
            0 => Some("none"),
            1 => Some("linear"),
            2 => Some("cubic"),
            3 => Some("nohalo"),
            _ => None,
        };
        json!({"matrixF64Bits":transform.matrix_f64_bits.map(|bits|format!("{bits:016x}")),"preOffset":transform.pre_offset,
            "interpolation":name,"interpolationId":transform.interpolation})
    });
    json!({"bounds":current.bounds,"transform":transform})
}
fn enabled(s: &Session) -> std::result::Result<(), String> {
    let doc = &s.active().ok_or_else(|| "no document open".to_string())?.doc;
    if doc.validate_resident().is_err() {
        return Err("linked geometry requires a resident document".into());
    }
    let state = doc.xcf_state.as_ref().ok_or_else(|| "no linked image source".to_string())?;
    if state.source_items.iter().any(|source| editable_candidate(doc, source)) {
        Ok(())
    } else {
        Err("no unlocked qualified linked image with explicitly recovered material".into())
    }
}
// Enabled queries examine bounded metadata, never decode or scan raster pixels.
// The run path validates the complete document, resource identity and consumer.
fn editable_candidate(doc: &Document, source: &SourceItem) -> bool {
    let ItemRef::Layer(owner) = source.owner else { return false };
    let EditableSource::Linked { resource, configuration, .. } = &source.source else { return false };
    let Some(state) = &doc.xcf_state else { return false };
    let Ok(locks) = crate::xcf_item_locks::effective_layer_locks(doc, owner) else { return false };
    if locks.require(crate::xcf_item_locks::EditKind::Content).is_err()
        || locks.require(crate::xcf_item_locks::EditKind::Position).is_err()
        || doc.sample_encoding != Some(photocraft_doc::SampleEncoding::NonlinearProfile)
        || doc.icc_profile.is_none()
        || !matches!(doc.mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale)
        || !matches!(doc.depth, photocraft_doc::SampleType::U8 | photocraft_doc::SampleType::U16)
        || doc.layer(owner).is_none_or(|layer| !matches!(layer.content, LayerContent::Raster(_)))
    {
        return false;
    }
    let Some(bounds) = state.items.iter().find(|item| item.item == source.owner).and_then(|item| item.raster_bounds) else { return false };
    let Some(material) = state.resources.iter().find(|entry| entry.id == *resource) else { return false };
    let Some(resolution) = &material.resolution else { return false };
    let Some(bytes) = material.owned_bytes.as_ref().map(|bytes| bytes.as_slice()) else { return false };
    if material.kind != photocraft_doc::xcf_resources::ResourceKind::LinkedImage
        || resolution.payload_hash.len() != 64
        || !resolution.payload_hash.bytes().all(|value| value.is_ascii_hexdigit())
        || bytes.len() > 8 << 20
        || photocraft_codecs::detect(bytes) != Some(photocraft_codecs::Format::Png)
        || bytes.get(8..12) != Some(13u32.to_be_bytes().as_slice())
        || bytes.get(12..16) != Some(b"IHDR".as_slice())
    {
        return false;
    }
    let Some(width) = bytes.get(16..20).and_then(|value| value.try_into().ok()).map(u32::from_be_bytes) else { return false };
    let Some(height) = bytes.get(20..24).and_then(|value| value.try_into().ok()).map(u32::from_be_bytes) else { return false };
    let expected_depth = if doc.depth == photocraft_doc::SampleType::U8 { 8 } else { 16 };
    let expected_layout = if doc.mode == photocraft_doc::ColorMode::Rgb { 6 } else { 4 };
    if bytes.get(24) != Some(&expected_depth)
        || bytes.get(25) != Some(&expected_layout)
        || width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > 4_000_000
        || bounds.is_empty()
    {
        return false;
    }
    let mut seen = 0u8;
    if configuration.len() > 3 {
        return false;
    }
    for argument in configuration {
        let photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Unsigned { width: 32, value }) = argument.value else {
            return false;
        };
        let bit = match argument.name.as_str() {
            "flags" if value <= 3 => 1,
            "source-width" if value == u64::from(width) => 2,
            "source-height" if value == u64::from(height) => 4,
            _ => return false,
        };
        if seen & bit != 0 {
            return false;
        }
        seen |= bit;
    }
    match effective_transform(source) {
        Ok(Some(transform)) => {
            (transform.interpolation == 0 || (doc.mode == photocraft_doc::ColorMode::Rgb && doc.depth == photocraft_doc::SampleType::U8))
                && photocraft_compose::gimp_link_items::qualify_linked_geometry(width, height, transform, bounds).is_ok()
        }
        Ok(None) => (bounds.width(), bounds.height()) == (width, height),
        Err(_) => false,
    }
}
fn owner(params: &Value, allowed: &[&str]) -> Result<LayerId> {
    let object = params.as_object().ok_or_else(|| bad("linked geometry parameters require an object"))?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(bad("unknown linked geometry parameter"));
    }
    let text = params.get("layerId").and_then(Value::as_str).ok_or_else(|| bad("layerId requires a canonical decimal string"))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|value| value.is_ascii_digit()) {
        return Err(bad("invalid layer identity"));
    }
    let id = text.parse::<u64>().map_err(|_| bad("invalid layer identity"))?;
    if id == 0 || id.to_string() != text {
        return Err(bad("invalid layer identity"));
    }
    Ok(LayerId(id))
}
fn origin(value: &Value) -> Result<[i32; 2]> {
    let values = value.as_array().filter(|values| values.len() == 2).ok_or_else(|| bad("originPx requires two integer coordinates"))?;
    let mut out = [0; 2];
    for (target, value) in out.iter_mut().zip(values) {
        let value = value.as_i64().ok_or_else(|| bad("originPx requires integer coordinates"))?;
        if !(-(1 << 29)..=(1 << 29)).contains(&value) {
            return Err(bad("linked geometry coordinate exceeds bounds"));
        }
        *target = i32::try_from(value).map_err(|_| bad("linked geometry coordinate exceeds bounds"))?;
    }
    Ok(out)
}
fn context<'a>(snapshot: &'a crate::DocumentSnapshot, owner: LayerId, ctl: &Interrupt<'_>) -> Result<(&'a SourceItem, Rect, String)> {
    ctl.check().map_err(|_| EngineError::Cancelled)?;
    snapshot.validate_resident().map_err(|_| bad("linked geometry requires a resident document; accounted edit adoption is not available"))?;
    let doc: &Document = snapshot;
    doc.validate_xcf_state_with(ctl).map_err(|e| if e.code == "cancelled" { EngineError::Cancelled } else { bad(e.to_string()) })?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("missing linked source state"))?;
    let source = state.source_items.iter().find(|source| source.owner == ItemRef::Layer(owner)).ok_or_else(|| bad("missing linked source owner"))?;
    let EditableSource::Linked { resource, .. } = &source.source else { return Err(bad("owner is not a linked image")) };
    require_geometry_locks(doc, owner)?;
    let bounds = state.items.iter().find(|item| item.item == source.owner).and_then(|item| item.raster_bounds).ok_or_else(|| bad("missing linked bounds"))?;
    let material = state.resources.iter().find(|entry| entry.id == *resource).ok_or_else(|| bad("missing linked resource"))?;
    let digest = material.resolution.as_ref().ok_or_else(|| bad("linked geometry requires explicit resource recovery"))?.payload_hash.clone();
    if material.owned_bytes.is_none() {
        return Err(bad("linked geometry requires owned material"));
    }
    Ok((source, bounds, digest))
}
fn effective_transform(source: &SourceItem) -> Result<Option<&TransformState>> {
    let EditableSource::Linked { current, .. } = &source.source else { return Err(bad("owner is not a linked image")) };
    Ok(current.as_ref().map_or(source.transform.as_ref(), |state| state.transform.as_ref()))
}
// This descriptor decode accesses owned bytes only and is bounded identically to
// the existing linked consumer. Actual color/metadata/pixel qualification follows
// in prepare_current_source before any staged edit can commit.
fn dimensions(doc: &Document, source: &SourceItem, ctl: &Interrupt<'_>) -> Result<(u32, u32)> {
    let EditableSource::Linked { resource, .. } = &source.source else { return Err(bad("owner is not a linked image")) };
    let bytes = doc
        .xcf_state
        .as_ref()
        .and_then(|state| state.resources.iter().find(|entry| entry.id == *resource))
        .and_then(|entry| entry.owned_bytes.as_ref().map(|bytes| bytes.as_slice()))
        .ok_or_else(|| bad("linked geometry requires owned material"))?;
    if bytes.len() > 8 << 20 || photocraft_codecs::detect(bytes) != Some(photocraft_codecs::Format::Png) {
        return Err(bad("linked geometry requires a qualified bounded PNG"));
    }
    ctl.check().map_err(|_| EngineError::Cancelled)?;
    let image = photocraft_codecs::decode_with(
        bytes,
        &photocraft_codecs::DecodeOptions {
            limits: photocraft_codecs::Limits { max_width: 8192, max_height: 8192, max_pixels: 4_000_000, max_alloc: 16 << 20 },
            ..Default::default()
        },
    )
    .map_err(|_| bad("invalid or oversized linked material"))?;
    ctl.check().map_err(|_| EngineError::Cancelled)?;
    Ok(image.dimensions())
}
fn placed_bounds(size: (u32, u32), origin: [i32; 2]) -> Result<Rect> {
    let [x, y] = origin;
    let x1 = i64::from(x).checked_add(i64::from(size.0)).ok_or_else(|| bad("linked bounds overflow"))?;
    let y1 = i64::from(y).checked_add(i64::from(size.1)).ok_or_else(|| bad("linked bounds overflow"))?;
    if size.0 == 0 || size.1 == 0 || x1 > 1 << 29 || y1 > 1 << 29 {
        return Err(bad("linked bounds exceed the supported domain"));
    }
    Ok(Rect::new(x, y, i32::try_from(x1).map_err(|_| bad("linked bounds overflow"))?, i32::try_from(y1).map_err(|_| bad("linked bounds overflow"))?))
}
fn natural_size(size: (u32, u32), state: &TransformState) -> Result<(u32, u32)> {
    use photocraft_algo::transform::Homography;
    let matrix = state.matrix_f64_bits.map(f64::from_bits);
    if size.0 == 0 || size.1 == 0 || matrix.iter().any(|value| !value.is_finite()) {
        return Err(bad("invalid linked transform"));
    }
    let forward = Homography(matrix);
    if forward.inverse().is_none_or(|inverse| inverse.0.iter().any(|value| !value.is_finite())) {
        return Err(bad("singular or nonfinite linked transform"));
    }
    let [ox, oy] = state.pre_offset.map(f64::from);
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let mut sign = None;
    for (x, y) in [(ox, oy), (ox + f64::from(size.0), oy), (ox, oy + f64::from(size.1)), (ox + f64::from(size.0), oy + f64::from(size.1))] {
        let denominator = matrix.get(6).ok_or_else(|| bad("invalid linked transform"))? * x
            + matrix.get(7).ok_or_else(|| bad("invalid linked transform"))? * y
            + matrix.get(8).ok_or_else(|| bad("invalid linked transform"))?;
        if !denominator.is_finite() || denominator == 0.0 || sign.is_some_and(|sign| sign != denominator.is_sign_positive()) {
            return Err(bad("linked transform crosses a projective horizon"));
        }
        sign = Some(denominator.is_sign_positive());
        let (x, y) = forward.apply(x, y);
        if !x.is_finite() || !y.is_finite() {
            return Err(bad("nonfinite linked transform extent"));
        }
        for (channel, value) in [x, y].into_iter().enumerate() {
            let lower = min.get_mut(channel).ok_or_else(|| bad("invalid linked transform"))?;
            *lower = lower.min(value);
            let upper = max.get_mut(channel).ok_or_else(|| bad("invalid linked transform"))?;
            *upper = upper.max(value);
        }
    }
    let mut out = [0; 2];
    for ((lower, upper), out) in min.into_iter().zip(max).zip(out.iter_mut()) {
        let (lower, upper) = (lower.floor(), upper.ceil());
        let width = upper - lower;
        if lower.abs() > f64::from(1 << 29) || upper.abs() > f64::from(1 << 29) || !(1.0..=8192.0).contains(&width) {
            return Err(bad("linked transform extent exceeds bounds"));
        }
        *out = width as u32;
    }
    Ok((*out.first().ok_or_else(|| bad("invalid linked extent"))?, *out.get(1).ok_or_else(|| bad("invalid linked extent"))?))
}
/// Admission runs before dispatch may drop a transient pixel-move preview.
/// It borrows the parameter envelope and never decodes, clones or reads material.
pub(crate) fn dispatch_preflight(s: &Session, id: &str, params: &Value) -> Result<bool> {
    if !matches!(id, "xcf.link.setPlacement" | "xcf.link.setTransform") {
        return Ok(false);
    }
    let active = s.active().ok_or(EngineError::NoDocument)?;
    if id == "xcf.link.setPlacement" {
        owner(params, &["layerId", "originPx"])?;
        origin(params.get("originPx").ok_or_else(|| bad("missing originPx"))?)?;
    } else {
        owner(params, &["layerId", "matrixF64Bits", "interpolation", "originPx"])?;
        transform_params(params)?;
        if let Some(value) = params.get("originPx") {
            origin(value)?;
        }
    }
    if active.floating.is_some() {
        return Err(bad("complete or cancel the pixel-move preview before editing linked geometry"));
    }
    Ok(true)
}
// Raster storage allocates complete tiles even at off-canvas/unaligned edges.
fn output_envelope(bounds: Rect, bpp: usize) -> Result<u64> {
    if bounds.is_empty() {
        return Err(bad("empty linked raster bounds"));
    }
    let tile = i64::from(photocraft_geom::TILE_SIZE);
    let axis = |lo: i32, hi: i32| -> Result<u64> {
        let count = (i64::from(hi) - 1)
            .div_euclid(tile)
            .checked_sub(i64::from(lo).div_euclid(tile))
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| bad("linked tile count overflow"))?;
        u64::try_from(count).map_err(|_| bad("linked tile count overflow"))
    };
    let tile_bytes = u64::try_from(tile)
        .ok()
        .and_then(|n| n.checked_mul(n))
        .and_then(|n| n.checked_mul(bpp as u64))
        .and_then(|n| n.checked_add(256))
        .ok_or_else(|| bad("linked tile size overflow"))?;
    axis(bounds.x0, bounds.x1)?
        .checked_mul(axis(bounds.y0, bounds.y1)?)
        .and_then(|n| n.checked_mul(tile_bytes))
        .and_then(|n| n.checked_add(bpp as u64))
        .ok_or_else(|| bad("linked raster envelope overflow"))
}
fn require_geometry_locks(doc: &Document, owner: LayerId) -> Result<()> {
    let locks = crate::xcf_item_locks::effective_layer_locks(doc, owner)?;
    locks.require(crate::xcf_item_locks::EditKind::Content)?;
    locks.require(crate::xcf_item_locks::EditKind::Position)
}
fn install_geometry(doc: &mut Document, owner: LayerId, current: LinkedCurrentState) -> Result<()> {
    let state = doc.xcf_state.as_mut().ok_or_else(|| bad("missing linked source state"))?;
    let item = state.items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).ok_or_else(|| bad("missing linked owner"))?;
    item.raster_bounds = Some(current.bounds);
    let source = state.source_items.iter_mut().find(|source| source.owner == ItemRef::Layer(owner)).ok_or_else(|| bad("missing linked source"))?;
    let EditableSource::Linked { current: target, .. } = &mut source.source else { return Err(bad("owner is not a linked image")) };
    *target = Some(current);
    Ok(())
}
fn install_pixels(doc: &mut Document, owner: LayerId, pixels: photocraft_raster::Surface, digest: String) -> Result<()> {
    let layer = doc.layer_mut(owner).ok_or_else(|| bad("missing linked owner"))?;
    if !matches!(layer.content, LayerContent::Raster(_)) {
        return Err(bad("linked geometry requires a resident raster"));
    }
    layer.content = LayerContent::Raster(pixels);
    let source = doc
        .xcf_state
        .as_mut()
        .and_then(|state| state.source_items.iter_mut().find(|source| source.owner == ItemRef::Layer(owner)))
        .ok_or_else(|| bad("missing linked source"))?;
    let EditableSource::Linked { last_verified_hash, .. } = &mut source.source else { return Err(bad("owner is not a linked image")) };
    *last_verified_hash = Some(digest);
    Ok(())
}
fn apply(s: &mut Session, owner: LayerId, current: LinkedCurrentState, digest: String, label: &str, ctl: &Interrupt<'_>) -> Result<Value> {
    let baseline = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let revision = s.active().ok_or(EngineError::NoDocument)?.revision;
    let (source, bounds, _) = context(&baseline, owner, ctl)?;
    let EditableSource::Linked { last_verified_hash, .. } = &source.source else { return Err(bad("owner is not a linked image")) };
    let size = dimensions(&baseline, source, ctl)?;
    if let Some(transform) = &current.transform {
        if transform.interpolation != 0 && !(baseline.mode == photocraft_doc::ColorMode::Rgb && baseline.depth == photocraft_doc::SampleType::U8) {
            return Err(bad("linked interpolation is not qualified for this color model or depth"));
        }
        photocraft_compose::gimp_link_items::qualify_linked_geometry(size.0, size.1, transform, current.bounds).map_err(|e| bad(e.to_string()))?;
    } else if (current.bounds.width(), current.bounds.height()) != size {
        return Err(bad("untransformed linked bounds differ from owned material"));
    }
    // Bound both sparse-map COW clones and the staged output before detaching any
    // model. This raster guard does not claim to meter all document metadata/RSS.
    let footprint = baseline.resident_raster_footprint_with(ctl)?;
    let output_bytes = u64::from(current.bounds.width())
        .checked_mul(u64::from(current.bounds.height()))
        .and_then(|n| n.checked_mul(baseline.pixel_format().bytes_per_pixel() as u64))
        .ok_or_else(|| bad("linked raster size overflow"))?;
    let tiled_output = output_envelope(current.bounds, baseline.pixel_format().bytes_per_pixel())?;
    if output_bytes > 64 << 20
        || footprint
            .clone_metadata_bytes
            .checked_mul(2)
            .and_then(|n| n.checked_add(footprint.retained_bytes))
            .and_then(|n| n.checked_add(tiled_output))
            .is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES)
    {
        return Err(bad("linked geometry raster staging exceeds bounds"));
    }
    if bounds == current.bounds && effective_transform(source)? == current.transform.as_ref() && last_verified_hash.as_deref() == Some(digest.as_str()) {
        let (_, expected) = photocraft_compose::gimp_link_items::prepare_current_source(&baseline, source, ctl)
            .map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) })?;
        if baseline.layer(owner).and_then(|layer| layer.surface()) == Some(&expected) {
            return Ok(json!({"layerId":owner.0.to_string(),"changed":false}));
        }
    }
    let mut staged = baseline.clone();
    let doc = staged.try_resident_mut_with_raster_limit(photocraft_raster::MAX_CHECKED_RASTER_BYTES, ctl)?;
    install_geometry(doc, owner, current.clone())?;
    let source = doc
        .xcf_state
        .as_ref()
        .and_then(|state| state.source_items.iter().find(|source| source.owner == ItemRef::Layer(owner)))
        .ok_or_else(|| bad("missing linked source"))?;
    let (_, pixels) = photocraft_compose::gimp_link_items::prepare_current_source(doc, source, ctl)
        .map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) })?;
    install_pixels(doc, owner, pixels.clone(), digest.clone())?;
    photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, ctl)
        .map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) })?;
    let staged_footprint = staged.resident_raster_footprint_with(ctl)?;
    if staged_footprint
        .retained_bytes
        .checked_add(staged_footprint.clone_metadata_bytes)
        .and_then(|n| n.checked_add(footprint.clone_metadata_bytes))
        .is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES)
    {
        return Err(bad("linked geometry commit COW exceeds bounds"));
    }
    ctl.check().map_err(|_| EngineError::Cancelled)?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    if active.revision != revision || !active.doc.shares_state(&baseline) {
        return Err(bad("linked geometry context changed while preparing"));
    }
    context(&active.doc, owner, ctl)?;
    let output = current.clone();
    s.edit_with_interrupt(label, ctl, move |doc, _| {
        // No await or IO lies between this final ancestry/identity validation and
        // installation. Original transform/cache/resource/provenance stay intact.
        require_geometry_locks(doc, owner)?;
        doc.validate_xcf_state_with(ctl).map_err(|e| if e.code == "cancelled" { EngineError::Cancelled } else { bad(e.to_string()) })?;
        install_geometry(doc, owner, current)?;
        install_pixels(doc, owner, pixels, digest)
    })?;
    Ok(json!({"layerId":owner.0.to_string(),"changed":true,"current":current_geometry_json(&output)}))
}
pub fn set_placement_with_interrupt(s: &mut Session, params: &Value, ctl: &Interrupt<'_>) -> Result<Value> {
    dispatch_preflight(s, "xcf.link.setPlacement", params)?;
    let owner = owner(params, &["layerId", "originPx"])?;
    let origin = origin(params.get("originPx").ok_or_else(|| bad("missing originPx"))?)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let (source, bounds, digest) = context(doc, owner, ctl)?;
    let current = LinkedCurrentState { transform: effective_transform(source)?.cloned(), bounds: placed_bounds((bounds.width(), bounds.height()), origin)? };
    apply(s, owner, current, digest, "Set XCF Linked Image Placement", ctl)
}
fn transform_params(params: &Value) -> Result<([u64; 9], i32)> {
    let values = params
        .get("matrixF64Bits")
        .and_then(Value::as_array)
        .filter(|values| values.len() == 9)
        .ok_or_else(|| bad("matrixF64Bits requires nine exact bit strings"))?;
    let mut matrix = [0; 9];
    for (out, value) in matrix.iter_mut().zip(values) {
        let text = value.as_str().ok_or_else(|| bad("matrixF64Bits requires exact bit strings"))?;
        if text.len() != 16 || !text.bytes().all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value)) {
            return Err(bad("matrixF64Bits requires lower-case sixteen-digit hex strings"));
        }
        *out = u64::from_str_radix(text, 16).map_err(|_| bad("invalid transform bit string"))?;
        if !f64::from_bits(*out).is_finite() {
            return Err(bad("nonfinite linked transform"));
        }
    }
    let interpolation = match params.get("interpolation").and_then(Value::as_str) {
        Some("none") => 0,
        Some("linear") => 1,
        Some("cubic") => 2,
        Some("nohalo") => 3,
        _ => return Err(bad("unsupported linked interpolation")),
    };
    let forward = photocraft_algo::transform::Homography(matrix.map(f64::from_bits));
    if forward.inverse().is_none_or(|inverse| inverse.0.iter().any(|value| !value.is_finite())) {
        return Err(bad("singular or nonfinite linked transform"));
    }
    Ok((matrix, interpolation))
}
pub fn set_transform_with_interrupt(s: &mut Session, params: &Value, ctl: &Interrupt<'_>) -> Result<Value> {
    dispatch_preflight(s, "xcf.link.setTransform", params)?;
    let owner = owner(params, &["layerId", "matrixF64Bits", "interpolation", "originPx"])?;
    let (matrix, interpolation) = transform_params(params)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let (source, bounds, digest) = context(doc, owner, ctl)?;
    let transform = TransformState { matrix_f64_bits: matrix, interpolation, pre_offset: source.transform.as_ref().map_or([0, 0], |state| state.pre_offset) };
    let size = dimensions(doc, source, ctl)?;
    let extent = natural_size(size, &transform)?;
    let origin = params.get("originPx").map(origin).transpose()?.unwrap_or([bounds.x0, bounds.y0]);
    let bounds = placed_bounds(extent, origin)?;
    photocraft_compose::gimp_link_items::qualify_linked_geometry(size.0, size.1, &transform, bounds).map_err(|e| bad(e.to_string()))?;
    apply(s, owner, LinkedCurrentState { transform: Some(transform), bounds }, digest, "Set XCF Linked Image Transform", ctl)
}
fn placement(s: &mut Session, params: &Value) -> Result<Value> {
    set_placement_with_interrupt(s, params, &Interrupt::NONE)
}
fn transform(s: &mut Session, params: &Value) -> Result<Value> {
    set_transform_with_interrupt(s, params, &Interrupt::NONE)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "xcf.link.setPlacement",
            label: "Set XCF Linked Image Placement",
            menu: &[],
            shortcut: None,
            params: r#"{"layerId":"canonical decimal U64","originPx":[i32,i32]} → regenerate current placement from explicitly owned material; preserve original transform/cache; content and position locks apply"#,
            enabled,
            run: placement,
            journal: false,
        },
        CommandSpec {
            id: "xcf.link.setTransform",
            label: "Set XCF Linked Image Transform",
            menu: &[],
            shortcut: None,
            params: r#"{"layerId":"canonical decimal U64","matrixF64Bits":["nine lower-case sixteen-digit hex strings"],"interpolation":"none|linear|cubic|nohalo","originPx":[i32,i32]?} → bounded current transform from explicitly owned material; original pre-offset retained; same qualified sampler domains"#,
            enabled,
            run: transform,
            journal: false,
        },
    ]
}
