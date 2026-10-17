//! Explicit byte-owned PNG16 preparation, not an imported-source qualifier.
//! The caller supplies a same-document preaccounted retention/validation envelope
//! and an already-resolved exact encoded profile. Decoder internals remain a
//! bounded provider operation with token fences, not an internally polled codec.
use crate::gimp_composite::{CompositionError, Result};
use crate::gimp_link_native_precision_candidate::{self as precision, PreparationPolicy, WorkingContext};
use crate::gimp_link_wide_cubic_candidate::{self as cubic, BoundProfile, InputUnitPrecision, IntermediatePrecision};
use photocraft_cms::{ColorSpace, Curve};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    Document, LayerId, Rect, SampleEncoding,
    item_identity::ItemRef,
    xcf_items::{EditableSource, SourceItem, TransformState},
    xcf_resources::ResourceKind,
};
use photocraft_geom::{TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
pub use precision::{FinalQuantization, Limits};
const DECODE_LIMIT: u64 = 16 << 20;
/// Strict existing APIs remain exact-present. The opt-in never makes an absent
/// PNG/resource profile implicit, and never changes Document profile markers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OriginalProfilePolicy {
    #[default]
    RequireExactPresent,
    ExplicitProvenImportedImplicit,
}

/// Explicit caller census; never a self-proving whole-document/RSS estimate.
/// `retained_bytes` includes payload/model/map/raster owners. `validation_scratch`
/// and `work_units` also cover two full canonical state validation passes. Obtain
/// these before calling from the existing IO/snapshot admission layer. No default.
#[derive(Clone, Copy)]
pub struct HeldDocumentEnvelope<'a> {
    pub document: &'a Document,
    pub retained_bytes: u64,
    pub validation_scratch: u64,
    pub work_units: u64,
}
#[derive(Clone, Copy)]
pub enum Reconstruction {
    Nearest,
    Cubic { input: InputUnitPrecision, intermediate: IntermediatePrecision, final_quantization: FinalQuantization },
}
#[derive(Clone, Copy)]
pub struct Policy {
    pub source_context: WorkingContext,
    pub preparation: PreparationPolicy,
    pub reconstruction: Reconstruction,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub held_document_bytes: u64,
    pub decoder_reserve_bytes: u64,
    pub candidate_reserve_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
    pub profile_verification_scratch_bytes: u64,
    pub profile_verification_work_units: u64,
}
fn refused() -> CompositionError {
    CompositionError::Unsupported("unqualified owned PNG16 linked candidate")
}
fn limit() -> CompositionError {
    RasterInputError::LimitExceeded.into()
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn plus(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(limit)
}
fn times(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(limit)
}
fn equal(a: &[u8], b: &[u8], c: &Interrupt<'_>) -> Result<bool> {
    if a.len() != b.len() {
        return Ok(false);
    }
    for (x, y) in a.chunks(32 << 10).zip(b.chunks(32 << 10)) {
        poll(c)?;
        if x != y {
            return Ok(false);
        }
    }
    poll(c)?;
    Ok(true)
}
fn validation(e: photocraft_doc::XcfValidationError) -> CompositionError {
    if e.code == "cancelled" { RasterInputError::Cancelled.into() } else { CompositionError::Invalid("invalid owned linked source state") }
}
fn cms_error(e: photocraft_cms::control::Error) -> CompositionError {
    match e {
        photocraft_cms::control::Error::Cancelled => RasterInputError::Cancelled.into(),
        photocraft_cms::control::Error::Allocation => RasterInputError::AllocationFailed.into(),
        photocraft_cms::control::Error::Limit(_) => limit(),
        photocraft_cms::control::Error::Cms(e) => e.into(),
    }
}
#[derive(Clone, Copy)]
struct VerificationPlan {
    scratch: u64,
    work: u64,
    implicit_work: u64,
    clone_work: u64,
    encode_work: u64,
}
fn verification_plan(
    doc: &Document,
    profile: BoundProfile<'_>,
    policy: OriginalProfilePolicy,
    limits: photocraft_cms::control::Limits,
    c: &Interrupt<'_>,
) -> Result<VerificationPlan> {
    poll(c)?;
    if policy == OriginalProfilePolicy::RequireExactPresent {
        return Ok(VerificationPlan { scratch: 0, work: 0, implicit_work: 0, clone_work: 0, encode_work: 0 });
    }
    let icc = doc.icc_profile.as_ref().ok_or_else(refused)?;
    let cancelled = || c.cancelled();
    // Genuine controlled effective-profile and fresh writer censuses; no cached
    // profile hash/encoding is accepted as a model equality proof.
    let implicit = photocraft_cms::xcf_color::effective_profile_requirements_with_control(
        photocraft_cms::Builtin::Srgb.profile(),
        photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        limits,
        &cancelled,
    )
    .map_err(cms_error)?;
    let clone = profile.definition.clone_requirements_with_control(limits, &cancelled).map_err(cms_error)?;
    let encode = profile.definition.encode_requirements_with_control(limits, &cancelled).map_err(cms_error)?;
    let scratch = plus(
        plus(implicit.peak_resident_bytes as u64, icc.len() as u64)?,
        plus(plus(clone.retained_bytes as u64, clone.cloned_model_bytes as u64)?, encode.peak_resident_bytes as u64)?,
    )?;
    // Includes initial/repeated borrowed admission plus the actual controlled
    // helper/clone/writer passes. Sequential phases receive debited subquotas.
    let implicit_work = plus(times(implicit.work_units, 5)?, times(icc.len() as u64, 4)?)?;
    let clone_work = times(clone.work_units, 4)?;
    let encode_work = times(encode.work_units, 4)?;
    let work = plus(plus(implicit_work, clone_work)?, plus(encode_work, plus(times(icc.len() as u64, 4)?, 4096)?)?)?;
    if scratch > limits.resident_bytes as u64 || work > limits.work_units {
        return Err(limit());
    }
    poll(c)?;
    Ok(VerificationPlan { scratch, work, implicit_work, clone_work, encode_work })
}
fn verify_current_model(doc: &Document, profile: BoundProfile<'_>, q: VerificationPlan, c: &Interrupt<'_>) -> Result<()> {
    if q.work == 0 {
        return poll(c);
    }
    let icc = doc.icc_profile.as_ref().ok_or_else(refused)?;
    let cancelled = || c.cancelled();
    let caps = |work| -> Result<photocraft_cms::control::Limits> {
        Ok(photocraft_cms::control::Limits { resident_bytes: usize::try_from(q.scratch).map_err(|_| limit())?, work_units: work })
    };
    let verified = photocraft_cms::xcf_color::effective_implicit_profile_with_control(
        icc,
        ColorSpace::Rgb,
        photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        caps(q.implicit_work)?,
        &cancelled,
    )
    .map_err(cms_error)?;
    drop(verified);
    poll(c)?;
    let copy = profile.definition.try_clone_with_control(caps(q.clone_work)?, &cancelled).map_err(cms_error)?;
    let fresh = copy.with_encoded_bytes_with_control(caps(q.encode_work)?, &cancelled).map_err(cms_error)?;
    if !equal(fresh.encoded_bytes().ok_or_else(refused)?, icc, c)? {
        return Err(refused());
    }
    poll(c)
}
#[derive(Debug)]
struct Png {
    width: u32,
    height: u32,
}
fn word(b: &[u8], at: usize) -> Result<u32> {
    let end = at.checked_add(4).ok_or_else(limit)?;
    Ok(u32::from_be_bytes(b.get(at..end).and_then(|b| b.try_into().ok()).ok_or_else(refused)?))
}
// PNG CRC polynomial, reflected byte order, initial/final all-one complement:
// https://www.w3.org/TR/png/#5CRC-algorithm. Fixed state, no lookup allocation.
fn chunk_crc(bytes: &[u8], c: &Interrupt<'_>) -> Result<u32> {
    let mut crc = u32::MAX;
    for chunk in bytes.chunks(1024) {
        poll(c)?;
        for byte in chunk {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 0 { crc >> 1 } else { (crc >> 1) ^ 0xedb8_8320 };
            }
        }
    }
    poll(c)?;
    Ok(!crc)
}
/// Closed ancillary domain prevents independent decoded text/EXIF/APNG buffers
/// and unqualified transfer/orientation semantics. All chunk CRCs (including
/// IEND) are checked here. The decoder remains responsible for deflate/pixels.
fn png(bytes: &[u8], c: &Interrupt<'_>) -> Result<Png> {
    poll(c)?;
    if bytes.len() > 8 << 20 || bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err(refused());
    }
    let (mut at, mut count, mut icc, mut data, mut header) = (8usize, 0usize, false, false, None);
    while at < bytes.len() {
        poll(c)?;
        count = count.checked_add(1).ok_or_else(limit)?;
        if count > 4096 {
            return Err(limit());
        }
        let size = usize::try_from(word(bytes, at)?).map_err(|_| limit())?;
        let start = at.checked_add(8).ok_or_else(limit)?;
        let end = start.checked_add(size).ok_or_else(limit)?;
        let next = end.checked_add(4).ok_or_else(limit)?;
        let tag = bytes.get(at.checked_add(4).ok_or_else(limit)?..start).ok_or_else(refused)?;
        let payload = bytes.get(start..end).ok_or_else(refused)?;
        if next > bytes.len() {
            return Err(refused());
        }
        let crc_input = bytes.get(at.checked_add(4).ok_or_else(limit)?..end).ok_or_else(refused)?;
        if chunk_crc(crc_input, c)? != word(bytes, end)? {
            return Err(CompositionError::Invalid("owned PNG chunk CRC mismatch"));
        }
        match tag {
            b"IHDR" if count == 1 && size == 13 => {
                let width = word(payload, 0)?;
                let height = word(payload, 4)?;
                if payload.get(8..) != Some([16, 6, 0, 0, 0].as_slice())
                    || width == 0
                    || height == 0
                    || width > 8192
                    || height > 8192
                    || times(u64::from(width), u64::from(height))? > 4_000_000
                    || times(times(u64::from(width), u64::from(height))?, 8)? > DECODE_LIMIT
                {
                    return Err(refused());
                }
                header = Some(Png { width, height });
            }
            b"iCCP" if header.is_some() && !icc && !data => {
                let nul = payload.iter().take(80).position(|b| *b == 0).ok_or_else(refused)?;
                if nul == 0
                    || nul > 79
                    || payload.get(nul.checked_add(1).ok_or_else(limit)?) != Some(&0)
                    || payload.len() <= nul.checked_add(2).ok_or_else(limit)?
                {
                    return Err(refused());
                }
                icc = true;
            }
            b"IDAT" if header.is_some() && icc => {
                data = true;
            }
            b"IEND" if size == 0 && data && icc && next == bytes.len() => {
                poll(c)?;
                return header.ok_or_else(refused);
            }
            _ => return Err(refused()),
        }
        at = next;
    }
    Err(refused())
}
struct Bound<'a> {
    owner: LayerId,
    bytes: &'a [u8],
    transform: &'a TransformState,
    bounds: Rect,
    extent: Rect,
    width: u32,
    height: u32,
}
fn bind<'a>(
    doc: &'a Document,
    source: &'a SourceItem,
    policy: Policy,
    profile: BoundProfile<'_>,
    original_policy: OriginalProfilePolicy,
    c: &Interrupt<'_>,
) -> Result<Bound<'a>> {
    poll(c)?;
    let state = doc.xcf_state.as_ref().ok_or_else(refused)?;
    if state.source_items.len() > 100_000
        || state.items.len() > 100_000
        || state.resources.len() > 100_000
        || !{
            let mut found = false;
            for item in &state.source_items {
                poll(c)?;
                if std::ptr::eq(item, source) {
                    found = true;
                    break;
                }
            }
            found
        }
    {
        return Err(refused());
    }
    if profile.definition.color_space != ColorSpace::Rgb {
        return Err(refused());
    }
    if matches!(policy.reconstruction, Reconstruction::Cubic { .. })
        && (profile.definition.a2b.iter().chain(&profile.definition.b2a).any(Option::is_some)
            || profile
                .definition
                .trc
                .as_ref()
                .is_none_or(|curves| curves.iter().any(|curve| !matches!(curve, Curve::Identity | Curve::Gamma(_) | Curve::Parametric { kind: 0..=4, .. }))))
    {
        return Err(refused());
    }
    let ItemRef::Layer(owner) = source.owner else {
        return Err(refused());
    };
    let EditableSource::Linked { resource, last_verified_hash, configuration, current } = &source.source else {
        return Err(refused());
    };
    if doc.mode != ColorMode::Rgb
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || policy.source_context != profile.context
        || profile.context.encoding != SampleEncoding::NonlinearProfile
        || profile.context.profile_identity == [0; 32]
    {
        return Err(refused());
    }
    let expected = match policy.preparation {
        PreparationPolicy::PreserveU16Words => SampleType::U16,
        PreparationPolicy::NormalizeU16ToF32 => SampleType::F32,
        PreparationPolicy::NormalizeU16ToF32ThenPromoteF64 => SampleType::F64,
    };
    if doc.depth != expected {
        return Err(refused());
    }
    let icc = doc.icc_profile.as_ref().ok_or_else(refused)?;
    let original_context = doc.source_color_context.as_ref().ok_or_else(refused)?;
    let profile_icc = profile.definition.encoded_bytes().ok_or_else(refused)?;
    if icc.len() > DECODE_LIMIT as usize || profile_icc.len() > DECODE_LIMIT as usize || !equal(profile_icc, icc, c)? {
        return Err(refused());
    }
    match original_policy {
        OriginalProfilePolicy::RequireExactPresent => {
            let original = original_context.original_icc.as_ref().ok_or_else(refused)?;
            if original.len() > DECODE_LIMIT as usize || !equal(original, icc, c)? {
                return Err(refused());
            }
        }
        OriginalProfilePolicy::ExplicitProvenImportedImplicit => {
            let precision = match doc.depth {
                SampleType::U16 => 250,
                SampleType::F32 => 650,
                SampleType::F64 => 750,
                _ => return Err(refused()),
            };
            if original_context.original_icc.is_some()
                || original_context.source_precision != precision
                || !matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. })
            {
                return Err(refused());
            }
            doc.working_profile_interpretation.validate_with_interrupt(Some(icc.as_slice()), doc.mode, c).map_err(validation)?;
        }
    }
    let mut retained = None;
    for entry in &state.resources {
        poll(c)?;
        if entry.id == *resource {
            retained = Some(entry);
            break;
        }
    }
    let retained = retained.ok_or_else(refused)?;
    if retained.kind != ResourceKind::LinkedImage || retained.resolution.as_ref().is_none_or(|r| r.payload_hash.is_empty()) {
        return Err(refused());
    }
    let resolution = retained.resolution.as_ref().ok_or_else(refused)?;
    if last_verified_hash.as_deref() != Some(resolution.payload_hash.as_str()) {
        return Err(refused());
    }
    let bytes = retained.owned_bytes.as_ref().ok_or_else(refused)?.as_slice();
    let descriptor = png(bytes, c)?;
    if configuration.len() > 3 {
        return Err(refused());
    }
    for (i, a) in configuration.iter().enumerate() {
        poll(c)?;
        if configuration.get(..i).ok_or_else(refused)?.iter().any(|b| b.name == a.name) {
            return Err(refused());
        }
        let photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Unsigned { width: 32, value }) = a.value else {
            return Err(refused());
        };
        match a.name.as_str() {
            "flags" if value <= 3 => (),
            "source-width" if value == u64::from(descriptor.width) => (),
            "source-height" if value == u64::from(descriptor.height) => (),
            _ => return Err(refused()),
        }
    }
    let mut bounds = None;
    for item in &state.items {
        poll(c)?;
        if item.item == source.owner {
            bounds = item.raster_bounds;
            break;
        }
    }
    let bounds = bounds.ok_or_else(refused)?;
    if current.as_ref().is_some_and(|s| s.bounds != bounds) {
        return Err(refused());
    }
    let transform = current.as_ref().map_or(source.transform.as_ref(), |s| s.transform.as_ref()).ok_or_else(refused)?;
    match policy.reconstruction {
        Reconstruction::Nearest if transform.interpolation == 0 => {
            crate::gimp_link_nearest::qualify_geometry(descriptor.width, descriptor.height, transform, bounds)?
        }
        Reconstruction::Cubic { final_quantization, .. } if transform.interpolation == 2 => {
            let final_sample = match final_quantization {
                FinalQuantization::Float32ThenU16Nearest => SampleType::U16,
                FinalQuantization::Float32 => SampleType::F32,
                FinalQuantization::Float32ThenPromoteF64 => SampleType::F64,
            };
            if final_sample != doc.depth {
                return Err(refused());
            }
            crate::gimp_link_cubic::qualify_geometry(descriptor.width, descriptor.height, transform, bounds)?;
        }
        _ => return Err(refused()),
    }
    let output_bytes = checked_region_byte_len(bounds, doc.pixel_format())?;
    if matches!(policy.reconstruction, Reconstruction::Nearest) && output_bytes > 64 << 20 {
        return Err(refused());
    }
    let x1 = transform.pre_offset[0].checked_add(i32::try_from(descriptor.width).map_err(|_| limit())?).ok_or_else(limit)?;
    let y1 = transform.pre_offset[1].checked_add(i32::try_from(descriptor.height).map_err(|_| limit())?).ok_or_else(limit)?;
    let extent = Rect::new(transform.pre_offset[0], transform.pre_offset[1], x1, y1);
    checked_region_byte_len(extent, PixelFormat::new(ColorMode::Rgb, SampleType::U16, true))?;
    if matches!(policy.reconstruction, Reconstruction::Nearest) && checked_region_byte_len(extent, doc.pixel_format())? > 64 << 20 {
        return Err(refused());
    }
    Ok(Bound { owner, bytes, transform, bounds, extent, width: descriptor.width, height: descriptor.height })
}
/// Validate local observable floors; caller census still accounts the rest of the
/// document and validation's private maps. No locator is read or serialized.
fn held_floor(doc: &Document, c: &Interrupt<'_>) -> Result<(u64, u64, u64)> {
    let s = doc.xcf_state.as_ref().ok_or_else(refused)?;
    let mut bytes = std::mem::size_of::<Document>() as u64;
    let mut model = plus(
        times(s.items.capacity() as u64, std::mem::size_of::<photocraft_doc::xcf_state::ItemState>() as u64)?,
        times(s.source_items.capacity() as u64, std::mem::size_of::<SourceItem>() as u64)?,
    )?;
    model = plus(model, times(s.resources.capacity() as u64, std::mem::size_of::<photocraft_doc::xcf_resources::ResourceReference>() as u64)?)?;
    if let Some(icc) = &doc.icc_profile {
        bytes = plus(bytes, icc.capacity() as u64)?;
    }
    if let Some(original) = doc.source_color_context.as_ref().and_then(|context| context.original_icc.as_ref()) {
        bytes = plus(bytes, original.capacity() as u64)?;
    }
    for item in &s.items {
        poll(c)?;
        model = plus(model, item.name.capacity() as u64)?;
    }
    for resource in &s.resources {
        poll(c)?;
        for name in [Some(&resource.name), resource.collection.as_ref(), resource.locator.as_ref(), resource.content_hash.as_ref()].into_iter().flatten() {
            model = plus(model, name.capacity() as u64)?;
        }
        if let Some(resolution) = &resource.resolution {
            model = plus(model, resolution.payload_hash.capacity() as u64)?;
        }
        if let Some(payload) = &resource.owned_bytes {
            bytes = plus(bytes, payload.capacity() as u64)?;
        }
    }
    for source in &s.source_items {
        poll(c)?;
        if let Some(cache) = &source.cache {
            bytes = plus(bytes, cache.bytes.capacity() as u64)?;
            model = plus(model, cache.blake3.capacity() as u64)?;
        }
    }
    // Conservative required local hash passes plus bounded relation-map scratch.
    Ok((
        plus(bytes, model)?,
        plus(times(plus(plus(s.items.len() as u64, s.source_items.len() as u64)?, s.resources.len() as u64)?, 512)?, 4096)?,
        times(plus(bytes, model)?, 4)?,
    ))
}
/// Admit decode, original retention, current precision preparation and sampler
/// before any decoded pixel/ICC allocation. Profile and caller census are borrowed.
pub fn requirements(
    doc: &Document,
    source: &SourceItem,
    envelope: HeldDocumentEnvelope<'_>,
    policy: Policy,
    profile: BoundProfile<'_>,
    limits: Limits,
    c: &Interrupt<'_>,
) -> Result<Requirements> {
    requirements_with_original_profile_policy(doc, source, envelope, policy, profile, OriginalProfilePolicy::RequireExactPresent, limits, c)
}
/// Additive opt-in; profile verification is admitted before decode/ICC copies.
#[allow(clippy::too_many_arguments)]
pub fn requirements_with_original_profile_policy(
    doc: &Document,
    source: &SourceItem,
    envelope: HeldDocumentEnvelope<'_>,
    policy: Policy,
    profile: BoundProfile<'_>,
    original_policy: OriginalProfilePolicy,
    limits: Limits,
    c: &Interrupt<'_>,
) -> Result<Requirements> {
    poll(c)?;
    if !std::ptr::eq(doc, envelope.document) || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || limits.work_units > 8_000_000_000 {
        return Err(limit());
    }
    let b = bind(doc, source, policy, profile, original_policy, c)?;
    let (floor, scratch, work) = held_floor(doc, c)?;
    if envelope.retained_bytes < floor || envelope.validation_scratch < scratch || envelope.work_units < work {
        return Err(limit());
    }
    let held = plus(envelope.retained_bytes, envelope.validation_scratch)?;
    // PNG library state, decoded pixels and an independently owned decoded ICC
    // may coexist. No text/EXIF/APNG metadata is permitted in this closed lane.
    let decoder = plus(times(DECODE_LIMIT, 3)?, 4096)?;
    let prepared = plus(times(checked_region_byte_len(b.extent, doc.pixel_format())? as u64, 2)?, 64)?;
    let first = TileCoord::containing(b.bounds.x0, b.bounds.y0);
    let last = TileCoord::containing(b.bounds.x1.checked_sub(1).ok_or_else(limit)?, b.bounds.y1.checked_sub(1).ok_or_else(limit)?);
    let tiles = times(
        u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| limit())?,
        u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| limit())?,
    )?;
    let tile = times(times(TILE_SIZE as u64, TILE_SIZE as u64)?, doc.pixel_format().bytes_per_pixel() as u64)?;
    let native = plus(plus(times(tiles, plus(tile, 1024)?)?, times(tile, 3)?)?, plus(times(tiles, 32)?, 8192)?)?;
    let candidate =
        plus(plus(prepared, checked_region_byte_len(b.extent, PixelFormat::new(ColorMode::Rgb, SampleType::U16, true))? as u64)?, plus(native, 4096)?)?;
    let base = plus(plus(held, decoder)?, candidate)?;
    let comparison_work = times(doc.icc_profile.as_ref().ok_or_else(refused)?.len() as u64, 16)?;
    let mut base_work = plus(envelope.work_units, plus(times(b.bytes.len() as u64, 128)?, comparison_work)?)?;
    for amount in
        [times(b.extent.size().area(), 256)?, times(prepared, 4)?, times(b.bounds.size().area(), 16384)?, times(native, 6)?, times(times(tiles, tiles)?, 512)?]
    {
        base_work = plus(base_work, amount)?;
    }
    if base > limits.resident_bytes || base_work > limits.work_units {
        return Err(limit());
    }
    let max = photocraft_cms::control::Limits::default();
    let remaining = photocraft_cms::control::Limits {
        resident_bytes: usize::try_from((limits.resident_bytes - base).min(max.resident_bytes as u64)).map_err(|_| limit())?,
        work_units: (limits.work_units - base_work).min(max.work_units),
    };
    let pr = profile.definition.clone_requirements_with_control(remaining, &|| c.cancelled()).map_err(|e| match e {
        photocraft_cms::control::Error::Cancelled => RasterInputError::Cancelled.into(),
        photocraft_cms::control::Error::Allocation => RasterInputError::AllocationFailed.into(),
        photocraft_cms::control::Error::Limit(_) => limit(),
        photocraft_cms::control::Error::Cms(e) => e.into(),
    })?;
    let candidate = plus(candidate, pr.retained_bytes as u64)?;
    let peak = plus(plus(held, decoder)?, candidate)?;
    let work = plus(base_work, plus(plus(times(pr.work_units, 5)?, times(pr.retained_bytes as u64, 4)?)?, (257 + 511) * 3 * 128 + 4096)?)?;
    if peak > limits.resident_bytes || work > limits.work_units {
        return Err(limit());
    }
    let verification = verification_plan(
        doc,
        profile,
        original_policy,
        photocraft_cms::control::Limits {
            resident_bytes: usize::try_from((limits.resident_bytes - peak).min(max.resident_bytes as u64)).map_err(|_| limit())?,
            work_units: (limits.work_units - work).min(max.work_units),
        },
        c,
    )?;
    let candidate = plus(candidate, verification.scratch)?;
    let peak = plus(peak, verification.scratch)?;
    let work = plus(work, verification.work)?;
    poll(c)?;
    Ok(Requirements {
        held_document_bytes: held,
        decoder_reserve_bytes: decoder,
        candidate_reserve_bytes: candidate,
        peak_resident_bytes: peak,
        work_units: work,
        profile_verification_scratch_bytes: verification.scratch,
        profile_verification_work_units: verification.work,
    })
}
/// Return private current pixels only; no SourceCache/resource/provenance writes,
/// no renderer admission, Session mutation, locator reads or fallback dispatch.
pub fn prepare_owned_current_candidate(
    doc: &Document,
    source: &SourceItem,
    envelope: HeldDocumentEnvelope<'_>,
    policy: Policy,
    profile: BoundProfile<'_>,
    limits: Limits,
    c: &Interrupt<'_>,
) -> Result<(LayerId, Surface)> {
    prepare_owned_current_candidate_with_original_profile_policy(doc, source, envelope, policy, profile, OriginalProfilePolicy::RequireExactPresent, limits, c)
}
/// Explicit caller-proven imported marker only; decoded PNG ICC still must be
/// present and byte-exact current. Private output never mutates source metadata.
#[allow(clippy::too_many_arguments)]
pub fn prepare_owned_current_candidate_with_original_profile_policy(
    doc: &Document,
    source: &SourceItem,
    envelope: HeldDocumentEnvelope<'_>,
    policy: Policy,
    profile: BoundProfile<'_>,
    original_policy: OriginalProfilePolicy,
    limits: Limits,
    c: &Interrupt<'_>,
) -> Result<(LayerId, Surface)> {
    let r = requirements_with_original_profile_policy(doc, source, envelope, policy, profile, original_policy, limits, c)?;
    let b = bind(doc, source, policy, profile, original_policy, c)?;
    if original_policy == OriginalProfilePolicy::ExplicitProvenImportedImplicit {
        let q = verification_plan(
            doc,
            profile,
            original_policy,
            photocraft_cms::control::Limits {
                resident_bytes: usize::try_from(r.profile_verification_scratch_bytes).map_err(|_| limit())?,
                work_units: r.profile_verification_work_units,
            },
            c,
        )?;
        if q.scratch > r.profile_verification_scratch_bytes || q.work > r.profile_verification_work_units {
            return Err(limit());
        }
        verify_current_model(doc, profile, q, c)?;
    }
    doc.validate_xcf_state_with(c).map_err(validation)?;
    poll(c)?;
    let image = photocraft_codecs::decode_with(
        b.bytes,
        &photocraft_codecs::DecodeOptions {
            limits: photocraft_codecs::Limits { max_width: 8192, max_height: 8192, max_pixels: 4_000_000, max_alloc: DECODE_LIMIT },
            keep_orientation: true,
        },
    )
    .map_err(|_| CompositionError::Invalid("invalid bounded owned PNG16 image"))?;
    poll(c)?; // The provider does not expose inner-loop cancellation.
    if image.dimensions() != (b.width, b.height)
        || image.layout() != photocraft_codecs::ChannelLayout::Rgba
        || image.sample_type() != photocraft_codecs::SampleType::U16
        || !image.warnings.is_empty()
        || image.data().len() != checked_region_byte_len(b.extent, PixelFormat::new(ColorMode::Rgb, SampleType::U16, true))?
        || !equal(image.icc.as_deref().ok_or_else(refused)?, doc.icc_profile.as_ref().ok_or_else(refused)?.as_slice(), c)?
    {
        return Err(refused());
    }
    let request = precision::Request {
        raw_source: image.data(),
        source_format: PixelFormat::new(ColorMode::Rgb, SampleType::U16, true),
        current_format: doc.pixel_format(),
        source_extent: b.extent,
        source_context: policy.source_context,
        current_context: profile.context,
        policy: policy.preparation,
    };
    // Whole-input and decoder slack remain reserved while lower allocates output.
    let inner = Limits {
        resident_bytes: limits
            .resident_bytes
            .checked_sub(plus(plus(r.held_document_bytes, r.decoder_reserve_bytes)?, r.profile_verification_scratch_bytes)?)
            .ok_or_else(limit)?,
        work_units: limits
            .work_units
            .checked_sub(plus(
                envelope.work_units,
                plus(times(b.bytes.len() as u64, 128)?, times(doc.icc_profile.as_ref().ok_or_else(refused)?.len() as u64, 16)?)?,
            )?)
            .and_then(|work| work.checked_sub(r.profile_verification_work_units))
            .ok_or_else(limit)?,
    };
    let output = match policy.reconstruction {
        Reconstruction::Nearest => precision::prepare_nearest(request, b.transform, b.bounds, inner, c)?,
        Reconstruction::Cubic { input, intermediate, final_quantization } => cubic::prepare_transform(
            cubic::Request {
                preparation: request,
                profile,
                transform: b.transform,
                bounds: b.bounds,
                input_precision: input,
                intermediate,
                final_quantization,
            },
            inner,
            c,
        )?,
    };
    // Immutable borrowed inputs cannot change during this invocation. Revalidate
    // full source/hash relationships under the token before returning private work.
    doc.validate_xcf_state_with(c).map_err(validation)?;
    let final_binding = bind(doc, source, policy, profile, original_policy, c)?;
    if final_binding.owner != b.owner
        || !std::ptr::eq(final_binding.bytes, b.bytes)
        || !std::ptr::eq(final_binding.transform, b.transform)
        || final_binding.bounds != b.bounds
    {
        return Err(refused());
    }
    poll(c)?;
    Ok((b.owner, output))
}
#[cfg(test)]
mod tests {
    use super::*;
    // Unrun real owned-resource control. Original cache deliberately differs
    // from selected bytes; only the latter can establish prepared current pixels.
    #[test]
    fn owned_png16_nearest_keeps_raw_words_original_cache_and_rejects_foreign_envelope() {
        use photocraft_doc::{
            Size,
            item_identity::ResourceId,
            xcf_items::SourceCache,
            xcf_resources::{ResourceFidelity, ResourceReference, ResourceResolution},
            xcf_state::XcfState,
        };
        let raw = [17u16, 32769, 60003, 0, 12007, 48011, 24013, 65535].into_iter().flat_map(u16::to_ne_bytes).collect::<Vec<_>>();
        let encoded = photocraft_cms::builtin::srgb().to_bytes();
        let profile = photocraft_cms::Profile::parse(&encoded).unwrap();
        let mut doc = Document::with_background("Owned candidate", Size::new(2, 1), ColorMode::Rgb, SampleType::U16, photocraft_color::Color::WHITE);
        doc.icc_profile = Some(encoded.clone());
        doc.source_color_context = Some(photocraft_doc::SourceColorContext { original_icc: Some(encoded.clone()), source_precision: 250 });
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        let owner = doc.layers[0].id;
        let mut image =
            photocraft_codecs::Image::from_raw(2, 1, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U16, raw.clone()).unwrap();
        image.icc = Some(encoded.as_ref().clone());
        let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let hash = blake3::hash(&png).to_hex().to_string();
        let original = vec![211u8; 16];
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        state.items[0].raster_bounds = Some(doc.bounds());
        state.resources.push(ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::LinkedImage,
            name: "Owned PNG16".into(),
            collection: None,
            internal: false,
            locator: Some("https://invalid.example/inert".into()),
            content_hash: Some(hash.clone()),
            owned_bytes: Some(png.into()),
            resolution: Some(ResourceResolution { fidelity: ResourceFidelity::Exact, payload_hash: hash.clone() }),
        });
        state.source_items.push(SourceItem {
            owner: ItemRef::Layer(owner),
            source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: Some(hash), configuration: vec![], current: None },
            transform: Some(TransformState { matrix_f64_bits: [1f64, 0., 0., 0., 1., 0., 0., 0., 1.].map(f64::to_bits), interpolation: 0, pre_offset: [0, 0] }),
            cache: Some(SourceCache {
                blake3: blake3::hash(&original).to_hex().to_string(),
                bytes: original.into(),
                width: 2,
                height: 1,
                format: doc.pixel_format(),
            }),
        });
        doc.xcf_state = Some(state);
        let source = &doc.xcf_state.as_ref().unwrap().source_items[0];
        let context = WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [9; 32] };
        let policy = Policy { source_context: context, preparation: PreparationPolicy::PreserveU16Words, reconstruction: Reconstruction::Nearest };
        let bound = BoundProfile { definition: &profile, context };
        // Explicit synthetic caller census, not a claimed native production scan.
        let envelope = HeldDocumentEnvelope { document: &doc, retained_bytes: 1 << 20, validation_scratch: 1 << 20, work_units: 1 << 20 };
        let limits = Limits { resident_bytes: 128 << 20, work_units: 100_000_000 };
        let before = doc.xcf_state.clone();
        let (id, output) = prepare_owned_current_candidate(&doc, source, envelope, policy, bound, limits, &Interrupt::NONE).unwrap();
        assert_eq!(id, owner);
        assert_eq!(output.try_read_interleaved(doc.bounds()).unwrap(), raw);
        assert_eq!(doc.xcf_state, before);
        let mut implicit = doc.clone();
        implicit.source_color_context.as_mut().unwrap().original_icc = None;
        implicit.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_srgb(&encoded);
        let implicit_source = &implicit.xcf_state.as_ref().unwrap().source_items[0];
        let implicit_envelope = HeldDocumentEnvelope { document: &implicit, ..envelope };
        let original = implicit.clone();
        assert!(prepare_owned_current_candidate(&implicit, implicit_source, implicit_envelope, policy, bound, limits, &Interrupt::NONE).is_err());
        let choice = OriginalProfilePolicy::ExplicitProvenImportedImplicit;
        let (id, output) = prepare_owned_current_candidate_with_original_profile_policy(
            &implicit,
            implicit_source,
            implicit_envelope,
            policy,
            bound,
            choice,
            limits,
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(id, owner);
        assert_eq!(output.try_read_interleaved(implicit.bounds()).unwrap(), raw);
        assert_eq!(implicit.icc_profile, original.icc_profile);
        assert_eq!(implicit.source_color_context, original.source_color_context);
        assert_eq!(implicit.working_profile_interpretation, original.working_profile_interpretation);
        assert_eq!(implicit.xcf_state, original.xcf_state);
        let req =
            requirements_with_original_profile_policy(&implicit, implicit_source, implicit_envelope, policy, bound, choice, limits, &Interrupt::NONE).unwrap();
        assert!(req.profile_verification_work_units > 0 && req.profile_verification_scratch_bytes > 0);
        assert!(
            prepare_owned_current_candidate_with_original_profile_policy(
                &implicit,
                implicit_source,
                implicit_envelope,
                policy,
                bound,
                choice,
                Limits { resident_bytes: req.peak_resident_bytes - 1, ..limits },
                &Interrupt::NONE
            )
            .is_err()
        );
        assert!(
            prepare_owned_current_candidate_with_original_profile_policy(
                &implicit,
                implicit_source,
                implicit_envelope,
                policy,
                bound,
                choice,
                limits,
                &Interrupt::cancel_only(&|| true)
            )
            .is_err()
        );
        // Public model mutation keeps its old cached encoding; exact fresh model
        // verification must reject it before the decoder/current-pixel stage.
        let mut stale = profile.clone();
        stale.trc = Some([Curve::Gamma(1.0), Curve::Gamma(1.0), Curve::Gamma(1.0)]);
        assert_eq!(stale.encoded_bytes(), profile.encoded_bytes());
        assert!(
            prepare_owned_current_candidate_with_original_profile_policy(
                &implicit,
                implicit_source,
                implicit_envelope,
                policy,
                BoundProfile { definition: &stale, context },
                choice,
                limits,
                &Interrupt::NONE
            )
            .is_err()
        );
        let mut absent = implicit.clone();
        absent.icc_profile = None;
        let absent_source = &absent.xcf_state.as_ref().unwrap().source_items[0];
        assert!(
            requirements_with_original_profile_policy(
                &absent,
                absent_source,
                HeldDocumentEnvelope { document: &absent, ..envelope },
                policy,
                bound,
                choice,
                limits,
                &Interrupt::NONE
            )
            .is_err()
        );
        let mut ordinary = implicit.clone();
        ordinary.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc;
        let ordinary_source = &ordinary.xcf_state.as_ref().unwrap().source_items[0];
        assert!(
            requirements_with_original_profile_policy(
                &ordinary,
                ordinary_source,
                HeldDocumentEnvelope { document: &ordinary, ..envelope },
                policy,
                bound,
                choice,
                limits,
                &Interrupt::NONE
            )
            .is_err()
        );
        let other = doc.clone();
        assert!(
            prepare_owned_current_candidate(&doc, source, HeldDocumentEnvelope { document: &other, ..envelope }, policy, bound, limits, &Interrupt::NONE)
                .is_err()
        );
        assert!(
            prepare_owned_current_candidate(&doc, source, HeldDocumentEnvelope { retained_bytes: 0, ..envelope }, policy, bound, limits, &Interrupt::NONE)
                .is_err()
        );
        assert!(prepare_owned_current_candidate(&doc, source, envelope, policy, bound, limits, &Interrupt::cancel_only(&|| true)).is_err());
        let req = requirements(&doc, source, envelope, policy, bound, limits, &Interrupt::NONE).unwrap();
        assert!(
            prepare_owned_current_candidate(
                &doc,
                source,
                envelope,
                policy,
                bound,
                Limits { resident_bytes: req.peak_resident_bytes - 1, ..limits },
                &Interrupt::NONE
            )
            .is_err()
        );
        for variant in 0..4 {
            let mut changed = doc.clone();
            match variant {
                0 => changed.source_color_context = None,
                1 => changed.source_color_context.as_mut().unwrap().original_icc = Some(vec![1, 2, 3].into()),
                _ => {
                    let EditableSource::Linked { last_verified_hash, .. } = &mut changed.xcf_state.as_mut().unwrap().source_items[0].source else {
                        panic!("fixture");
                    };
                    *last_verified_hash = if variant == 2 { None } else { Some("0".repeat(64)) };
                }
            }
            let changed_source = &changed.xcf_state.as_ref().unwrap().source_items[0];
            assert!(
                requirements(&changed, changed_source, HeldDocumentEnvelope { document: &changed, ..envelope }, policy, bound, limits, &Interrupt::NONE)
                    .is_err()
            );
        }
        assert_eq!(doc.xcf_state, before);
    }
    #[test]
    fn descriptor_rejects_truncation_ancillary_and_cancellation_before_decode() {
        fn chunk(tag: &[u8; 4], payload: &[u8], b: &mut Vec<u8>) {
            b.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            b.extend_from_slice(tag);
            b.extend_from_slice(payload);
            let mut crc_input = tag.to_vec();
            crc_input.extend_from_slice(payload);
            b.extend_from_slice(&chunk_crc(&crc_input, &Interrupt::NONE).unwrap().to_be_bytes());
        }
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut header = Vec::new();
        header.extend_from_slice(&2u32.to_be_bytes());
        header.extend_from_slice(&1u32.to_be_bytes());
        header.extend_from_slice(&[16, 6, 0, 0, 0]);
        chunk(b"IHDR", &header, &mut bytes);
        chunk(b"iCCP", b"Own\0\0x", &mut bytes);
        chunk(b"IDAT", b"x", &mut bytes);
        chunk(b"IEND", b"", &mut bytes);
        // Descriptor only; valid CRCs, deliberately fake deflate still refused by codec.
        assert_eq!(chunk_crc(b"123456789", &Interrupt::NONE).unwrap(), 0xcbf4_3926);
        assert_eq!(chunk_crc(b"IEND", &Interrupt::NONE).unwrap(), 0xae42_6082);
        assert_eq!(png(&bytes, &Interrupt::NONE).unwrap().width, 2);
        let mut bad_tail = bytes.clone();
        *bad_tail.last_mut().unwrap() ^= 1;
        assert!(png(&bad_tail, &Interrupt::NONE).is_err());
        assert!(png(&bytes[..bytes.len() - 1], &Interrupt::NONE).is_err());
        assert!(png(&bytes, &Interrupt::cancel_only(&|| true)).is_err());
        let mut ancillary = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(b"IHDR", &header, &mut ancillary);
        chunk(b"gAMA", &[0; 4], &mut ancillary);
        ancillary.extend_from_slice(&bytes[33..]);
        assert!(png(&ancillary, &Interrupt::NONE).is_err());
    }
}
