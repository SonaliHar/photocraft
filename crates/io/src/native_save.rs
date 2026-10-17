//! Controlled native-save payload admission and atomic streamed output.
//! These standard-profile seams do not qualify native_large or all metadata heap capacity.
use crate::IoError;
use photocraft_doc::{Document, Layer, LayerContent, SmartSource};
use photocraft_format::{FormatError, PcraftWriter, PreparationLimits, SaveStats};
use photocraft_raster::{Interrupt, Surface};
use std::{
    collections::HashSet,
    io::{Seek, Write},
    sync::Arc,
};
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeSaveLimits {
    pub preparation: PreparationLimits,
    pub archive: photocraft_format::zip::stream::Limits,
}
#[derive(Debug)]
pub struct NativeSaveResult {
    pub warnings: Vec<String>,
    pub stats: SaveStats,
    pub retained_payload_bytes: usize,
    pub retained_model_slice_bytes: usize,
}
fn limit(name: &str) -> IoError {
    FormatError::LimitExceeded(name.into()).into()
}
fn poll(control: &Interrupt<'_>) -> Result<(), IoError> {
    if control.cancelled() { Err(IoError::Cancelled) } else { Ok(()) }
}
/// Actual outer visitor counts, not a CPU-time or complete provider-work estimate.
/// Surface origin validation has its own controlled inner traversal, not counted here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeCensusVisits {
    pub identity_visits: usize,
    pub layer_visits: usize,
    pub surface_visits: usize,
    pub model_visits: usize,
}
/// Borrowed exact-document model capacity receipt. No caller-asserted totals.
pub struct NativeModelCensus<'d> {
    document: &'d Document,
    retained: usize,
    index: usize,
    visits: NativeCensusVisits,
}
/// Borrowed exact-document unique payload capacity receipt.
pub struct NativePayloadCensus<'d> {
    document: &'d Document,
    retained: usize,
    index: usize,
    maps: usize,
    visits: NativeCensusVisits,
}
macro_rules! census_accessors {
    ($ty:ident) => {
        impl<'d> $ty<'d> {
            pub fn document(&self) -> &'d Document {
                self.document
            }
            pub fn belongs_to(&self, document: &Document) -> bool {
                std::ptr::eq(self.document, document)
            }
            pub fn retained_bytes(&self) -> usize {
                self.retained
            }
            /// Conservative identity-index reservation, not measured HashSet allocation.
            pub fn known_index_reservation_bytes(&self) -> usize {
                self.index
            }
            pub fn visit_scope(&self) -> NativeCensusVisits {
                self.visits
            }
        }
    };
}
census_accessors!(NativeModelCensus);
census_accessors!(NativePayloadCensus);
impl NativeModelCensus<'_> {
    pub fn known_map_reservation_bytes(&self) -> usize {
        0
    }
}
impl NativePayloadCensus<'_> {
    /// Conservative raster-map reservation (256 bytes per observed tile), not private node capacity.
    pub fn known_map_reservation_bytes(&self) -> usize {
        self.maps
    }
}
/// Two authentic receipts for the same immutable current Document.
/// The slices preserve their old scopes; this does not measure total private heap/RSS.
pub struct NativeCurrentCensus<'d> {
    model: NativeModelCensus<'d>,
    payload: NativePayloadCensus<'d>,
    retained: usize,
    index: usize,
    visits: NativeCensusVisits,
}
impl<'d> NativeCurrentCensus<'d> {
    pub fn join(model: NativeModelCensus<'d>, payload: NativePayloadCensus<'d>) -> Result<Self, IoError> {
        if !std::ptr::eq(model.document, payload.document) {
            return Err(IoError::Unsupported("native census document identity mismatch".into()));
        }
        let retained = model.retained.checked_add(payload.retained).ok_or_else(|| limit("native census retained overflow"))?;
        let index = model.index.checked_add(payload.index).ok_or_else(|| limit("native census index overflow"))?;
        let sum = |a: usize, b: usize| a.checked_add(b).ok_or_else(|| limit("native census visit overflow"));
        let visits = NativeCensusVisits {
            identity_visits: sum(model.visits.identity_visits, payload.visits.identity_visits)?,
            layer_visits: sum(model.visits.layer_visits, payload.visits.layer_visits)?,
            surface_visits: sum(model.visits.surface_visits, payload.visits.surface_visits)?,
            model_visits: sum(model.visits.model_visits, payload.visits.model_visits)?,
        };
        Ok(Self { model, payload, retained, index, visits })
    }
    pub fn document(&self) -> &'d Document {
        self.model.document
    }
    pub fn belongs_to(&self, document: &Document) -> bool {
        std::ptr::eq(self.document(), document)
    }
    pub fn retained_bytes(&self) -> usize {
        self.retained
    }
    pub fn model_retained_bytes(&self) -> usize {
        self.model.retained
    }
    pub fn payload_retained_bytes(&self) -> usize {
        self.payload.retained
    }
    pub fn known_index_reservation_bytes(&self) -> usize {
        self.index
    }
    pub fn known_map_reservation_bytes(&self) -> usize {
        self.payload.maps
    }
    pub fn visit_scope(&self) -> NativeCensusVisits {
        self.visits
    }
}
pub(super) struct Payloads<'a, 'i> {
    seen: HashSet<(u8, usize)>,
    bytes: usize,
    index: usize,
    map_bytes: usize,
    limits: PreparationLimits,
    control: &'a Interrupt<'i>,
    visits: NativeCensusVisits,
    work: Option<&'a dyn Fn(u64) -> Result<(), IoError>>,
}
impl Payloads<'_, '_> {
    pub(super) fn debit(&self, units: u64) -> Result<(), IoError> {
        if let Some(work) = self.work {
            poll(self.control)?;
            work(units)?;
        }
        Ok(())
    }
    pub(super) fn add(&mut self, tag: u8, pointer: usize, bytes: usize) -> Result<(), IoError> {
        poll(self.control)?;
        self.debit(64)?;
        self.visits.identity_visits = self.visits.identity_visits.checked_add(1).ok_or_else(|| limit("native census visit overflow"))?;
        if bytes == 0 || self.seen.contains(&(tag, pointer)) {
            return Ok(());
        }
        let index = self.index.checked_add(128).ok_or_else(|| limit("native input index overflow"))?;
        if index > self.limits.index_bytes {
            return Err(limit("native input index bytes"));
        }
        let total = self.bytes.checked_add(bytes).ok_or_else(|| limit("native retained payload overflow"))?;
        if total.checked_add(index).is_none_or(|value| value > self.limits.resident_bytes) {
            return Err(limit("native retained payload and index resident bytes"));
        }
        self.seen.try_reserve(1).map_err(|_| limit("native input identity allocation"))?;
        self.seen.insert((tag, pointer));
        self.bytes = total;
        self.index = index;
        Ok(())
    }
    fn blob(&mut self, bytes: &Arc<Vec<u8>>) -> Result<(), IoError> {
        self.add(1, Arc::as_ptr(bytes) as usize, bytes.capacity())
    }
    fn owned(&mut self, bytes: &Vec<u8>) -> Result<(), IoError> {
        self.add(2, bytes.as_ptr() as usize, bytes.capacity())
    }
    fn surface(&mut self, surface: &Surface) -> Result<(), IoError> {
        // Origin validation performs two checked coordinate products per retained
        // tile, independently of the following identity traversal; no pixel read.
        if self.work.is_some() {
            let origins = u64::try_from(surface.tile_count())
                .ok()
                .and_then(|n| n.checked_add(1))
                .and_then(|n| n.checked_mul(64))
                .ok_or_else(|| limit("native census origin work overflow"))?;
            self.debit(origins)?;
        }
        self.visits.surface_visits = self.visits.surface_visits.checked_add(1).ok_or_else(|| limit("native census visit overflow"))?;
        surface.try_validate_tile_origins_with(self.control).map_err(|e| match e {
            photocraft_raster::RasterInputError::Cancelled => IoError::Cancelled,
            other => IoError::Unsupported(other.to_string()),
        })?;
        self.map_bytes = self
            .map_bytes
            .checked_add(surface.tile_count().checked_mul(256).ok_or_else(|| limit("native raster map overflow"))?)
            .ok_or_else(|| limit("native raster map overflow"))?;
        self.add(3, surface.default_bytes().as_ptr() as usize, surface.default_bytes().len())?;
        for (_, tile) in surface.tiles() {
            self.add(4, Arc::as_ptr(tile) as usize, tile.bytes().len())?;
        }
        Ok(())
    }
    fn layer(&mut self, layer: &Layer, depth: usize) -> Result<(), IoError> {
        self.debit(64)?;
        self.visits.layer_visits = self.visits.layer_visits.checked_add(1).ok_or_else(|| limit("native census visit overflow"))?;
        poll(self.control)?;
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(limit("native layer nesting"));
        }
        if let Some(surface) = layer.surface() {
            self.surface(surface)?;
        }
        if let Some(mask) = &layer.mask {
            self.surface(&mask.surface)?;
        }
        if let Some(cache) = &layer.fill_cache {
            self.surface(&cache.surface)?;
        }
        if let Some(bytes) = &layer.effects.psd_raw {
            self.blob(bytes)?;
        }
        for (_, bytes) in &layer.psd_blocks {
            self.blob(bytes)?;
        }
        if let Some(indexed) = &layer.indexed_source {
            self.blob(indexed.indices())?;
            if let Some(alpha) = indexed.alpha() {
                self.blob(alpha)?;
            }
        }
        if let Some(video) = &layer.video {
            for surface in &video.frames {
                self.surface(surface)?;
            }
            if let Some(original) = &video.original_frames {
                for surface in original {
                    self.surface(surface)?;
                }
            }
        }
        match &layer.content {
            LayerContent::BackedRaster(_) => {
                return Err(IoError::Unsupported("backed raster requires the accounted native archive writer".into()));
            }
            LayerContent::Group(group) => {
                for child in &group.children {
                    self.layer(child, depth + 1)?;
                }
            }
            LayerContent::Text(text) => {
                if let Some(bytes) = &text.psd_raw {
                    self.blob(bytes)?;
                }
            }
            LayerContent::Shape(shape) => {
                if let Some(bytes) = &shape.psd_raw {
                    self.blob(bytes)?;
                }
            }
            LayerContent::Smart(smart) => {
                if let SmartSource::Embedded { bytes, .. } = &smart.source {
                    self.blob(bytes)?;
                }
                if let Some(bytes) = &smart.psd_raw {
                    self.blob(bytes)?;
                }
                if let Some(mask) = &smart.filter_mask {
                    self.surface(&mask.surface)?;
                }
            }
            LayerContent::Raster(_) | LayerContent::Adjustment(_) | LayerContent::Fill(_) => {}
        }
        Ok(())
    }
}
/// Unique retained raster/source payloads, including hidden layers and immutable caches.
/// This measures payload capacity, not every model container or arbitrary metadata allocation.
pub fn retained_native_payload_census<'d>(doc: &'d Document, limits: PreparationLimits, control: &Interrupt<'_>) -> Result<NativePayloadCensus<'d>, IoError> {
    retained_native_payload_census_inner(doc, limits, control, None)
}
/// Same borrowed receipt and admission scope, with prospective original-operation
/// Work at actual bounded visitor steps. Does not price private provider work.
pub fn retained_native_payload_census_with_work<'d>(
    doc: &'d Document,
    limits: PreparationLimits,
    control: &Interrupt<'_>,
    work: &dyn Fn(u64) -> Result<(), IoError>,
) -> Result<NativePayloadCensus<'d>, IoError> {
    retained_native_payload_census_inner(doc, limits, control, Some(work))
}
fn retained_native_payload_census_inner<'d>(
    doc: &'d Document,
    limits: PreparationLimits,
    control: &Interrupt<'_>,
    work: Option<&dyn Fn(u64) -> Result<(), IoError>>,
) -> Result<NativePayloadCensus<'d>, IoError> {
    // Selected-only fixed setup is paid before constructing writer/visitor
    // headers, including empty payload censuses with no identity records.
    if let Some(work) = work {
        poll(control)?;
        work(64)?;
    }
    let mut check = PcraftWriter::new();
    check.set_preparation_limits(limits)?;
    poll(control)?;
    let mut payloads = Payloads { seen: HashSet::new(), bytes: 0, index: 0, map_bytes: 0, limits, control, visits: NativeCensusVisits::default(), work };
    for bytes in
        [doc.icc_profile.as_ref(), doc.metadata.exif.as_ref(), doc.source_color_context.as_ref().and_then(|c| c.original_icc.as_ref())].into_iter().flatten()
    {
        payloads.blob(bytes)?;
    }
    for (_, _, bytes) in &doc.metadata.psd_resources {
        payloads.blob(bytes)?;
    }
    for (_, _, bytes) in &doc.metadata.psd_global_blocks {
        payloads.blob(bytes)?;
    }
    for layer in &doc.layers {
        payloads.layer(layer, 0)?;
    }
    for channel in doc.channels.iter().chain(doc.quick_mask.iter()) {
        payloads.surface(&channel.surface)?;
    }
    if let Some(selection) = &doc.selection {
        payloads.surface(selection)?;
    }
    for pattern in &doc.patterns {
        payloads.surface(&pattern.surface)?;
    }
    for path in &doc.paths {
        payloads.debit(64)?;
        if let Some(bytes) = &path.psd_raw {
            payloads.blob(bytes)?;
        }
    }
    if let Some(state) = &doc.xcf_state {
        for resource in &state.resources {
            payloads.debit(64)?;
            if let Some(bytes) = &resource.owned_bytes {
                payloads.owned(bytes.as_ref())?;
            }
        }
        for source in &state.source_items {
            payloads.debit(64)?;
            if let Some(cache) = &source.cache {
                payloads.owned(cache.bytes.as_ref())?;
            }
        }
        for record in &state.opaque {
            payloads.debit(64)?;
            payloads.owned(record.bytes.as_ref())?;
        }
        for mask in &state.filter_masks {
            payloads.surface(&mask.surface)?;
        }
        for indexed in &state.editable_indexed {
            payloads.blob(indexed.raster.indices())?;
            if let Some(alpha) = indexed.raster.alpha() {
                payloads.blob(alpha)?;
            }
        }
    }
    poll(control)?;
    Ok(NativePayloadCensus { document: doc, retained: payloads.bytes, index: payloads.index, maps: payloads.map_bytes, visits: payloads.visits })
}
pub fn retained_native_payload_bytes(doc: &Document, limits: PreparationLimits, control: &Interrupt<'_>) -> Result<usize, IoError> {
    Ok(retained_native_payload_census(doc, limits, control)?.retained_bytes())
}
pub(crate) fn retained_native_map_bytes(doc: &Document, limits: PreparationLimits, control: &Interrupt<'_>) -> Result<usize, IoError> {
    Ok(retained_native_payload_census(doc, limits, control)?.known_map_reservation_bytes())
}
/// Measured public canonical/structural String and Vec capacities, excluding
/// retained raster, cache, resource and other immutable byte payloads.
///
/// This is a model-container census for clone admission, not a complete clone
/// allocation or allocator/RSS estimate. Legacy text/effect/vector internals,
/// private map nodes and allocator overhead remain unmeasured. The visitor also
/// conservatively charges distinct mask Surface owners independently of tiles.
/// Limits apply to this slice and its bounded identity index only; callers must
/// admit raster/COW work and immutable retained payloads separately.
pub fn retained_native_model_census<'d>(doc: &'d Document, limits: PreparationLimits, control: &Interrupt<'_>) -> Result<NativeModelCensus<'d>, IoError> {
    retained_native_model_census_inner(doc, limits, control, None)
}
/// Same borrowed receipt and admission scope, with prospective original-operation
/// Work at actual bounded visitor steps. Does not price private provider work.
pub fn retained_native_model_census_with_work<'d>(
    doc: &'d Document,
    limits: PreparationLimits,
    control: &Interrupt<'_>,
    work: &dyn Fn(u64) -> Result<(), IoError>,
) -> Result<NativeModelCensus<'d>, IoError> {
    retained_native_model_census_inner(doc, limits, control, Some(work))
}
fn retained_native_model_census_inner<'d>(
    doc: &'d Document,
    limits: PreparationLimits,
    control: &Interrupt<'_>,
    work: Option<&dyn Fn(u64) -> Result<(), IoError>>,
) -> Result<NativeModelCensus<'d>, IoError> {
    // Selected-only fixed setup is paid before constructing writer/visitor
    // headers, including empty payload censuses with no identity records.
    if let Some(work) = work {
        poll(control)?;
        work(64)?;
    }
    let mut check = PcraftWriter::new();
    check.set_preparation_limits(limits)?;
    poll(control)?;
    let mut counter = Payloads { seen: HashSet::new(), bytes: 0, index: 0, map_bytes: 0, limits, control, visits: NativeCensusVisits::default(), work };
    counter.visits.model_visits = super::native_save_model::measure(doc, &mut counter, control)?;
    poll(control)?;
    Ok(NativeModelCensus { document: doc, retained: counter.bytes, index: counter.index, visits: counter.visits })
}
pub fn retained_native_model_bytes(doc: &Document, limits: PreparationLimits, control: &Interrupt<'_>) -> Result<usize, IoError> {
    Ok(retained_native_model_census(doc, limits, control)?.retained_bytes())
}

/// Retained payloads plus the measured public canonical/structural String and Vec capacity slice.
/// Legacy text/effect/vector/private map allocations and allocator overhead remain unmeasured.
pub fn retained_native_input_bytes(doc: &Document, limits: PreparationLimits, control: &Interrupt<'_>) -> Result<usize, IoError> {
    let payload = retained_native_payload_bytes(doc, limits, control)?;
    let mut counter =
        Payloads { seen: HashSet::new(), bytes: payload, index: 0, map_bytes: 0, limits, control, visits: NativeCensusVisits::default(), work: None };
    super::native_save_model::measure(doc, &mut counter, control)?;
    Ok(counter.bytes)
}

fn prepare(
    doc: &Document,
    limits: NativeSaveLimits,
    control: &Interrupt<'_>,
) -> Result<(PcraftWriter, photocraft_format::SaveOptions, Vec<String>, usize, usize), IoError> {
    let retained = retained_native_payload_bytes(doc, limits.preparation, control)?;
    let total = retained_native_input_bytes(doc, limits.preparation, control)?;
    let model = total.checked_sub(retained).ok_or_else(|| limit("native retained capacity accounting"))?;
    let ceiling = photocraft_format::zip::stream::Limits::default();
    if limits.archive.max_archive_bytes > ceiling.max_archive_bytes
        || limits.archive.max_metadata_bytes > ceiling.max_metadata_bytes
        || limits.archive.max_entries > ceiling.max_entries
    {
        return Err(limit("native archive limits may only be lowered"));
    }
    // Legacy faithful proxies clone the document. Qualified render-only proxies
    // charge their owned snapshot in the live preview ledger instead; they borrow
    // original persistence metadata and do not clone the document.
    let checked_preview = (doc.sample_encoding.is_some()
        || matches!(doc.depth, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64)
        || crate::has_enabled_xcf_filters(doc))
        && matches!(doc.mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale);
    let proxy_payload = if !checked_preview
        && !photocraft_compose::proxy_preview::proxy_preview_supported(doc)
        && doc.size.area() > photocraft_compose::PROXY_THUMBNAIL_PIXELS
        && photocraft_compose::proxy::proxy_faithful(doc)
    {
        total
    } else {
        0
    };
    let available = limits
        .preparation
        .resident_bytes
        .checked_sub(total)
        .and_then(|n| n.checked_sub(proxy_payload))
        .ok_or_else(|| limit("native retained and proxy payload resident bytes"))?;
    let (previews, warnings) = crate::native_previews_with_interrupt(doc, control, available)?;
    let preview_bytes = previews
        .thumbnail
        .iter()
        .chain(previews.composite.iter())
        .try_fold(0usize, |sum, image| sum.checked_add(image.pixels.capacity()).ok_or_else(|| limit("native preview resident overflow")))?;
    let archive_workspace = usize::try_from(limits.archive.max_metadata_bytes)
        .ok()
        .and_then(|n| n.checked_add(256 << 10))
        .and_then(|n| n.checked_add(if cfg!(target_endian = "big") { 2 << 20 } else { 0 }))
        .ok_or_else(|| limit("native archive workspace exceeds address space"))?;
    let remaining = limits
        .preparation
        .resident_bytes
        .checked_sub(total)
        .and_then(|n| n.checked_sub(preview_bytes))
        .and_then(|n| n.checked_sub(archive_workspace))
        .ok_or_else(|| limit("native retained payload plus preview and archive resident bytes"))?;
    let mut writer = PcraftWriter::new();
    writer.set_preparation_limits(PreparationLimits { resident_bytes: remaining, ..limits.preparation })?;
    poll(control)?;
    Ok((writer, previews, warnings, retained, model))
}
struct ByteLedger {
    live: std::cell::Cell<usize>,
    maximum: usize,
}
#[cfg(not(target_arch = "wasm32"))]
struct NativeSaveCommitError(IoError);
#[cfg(not(target_arch = "wasm32"))]
impl From<std::io::Error> for NativeSaveCommitError {
    fn from(error: std::io::Error) -> Self {
        Self(FormatError::Io(error).into())
    }
}
impl photocraft_format::PreparationObserver for ByteLedger {
    fn reserve(&self, bytes: usize) -> photocraft_format::Result<()> {
        let next = self.live.get().checked_add(bytes).ok_or_else(|| FormatError::LimitExceeded("native byte-export resident overflow".into()))?;
        if next > self.maximum {
            return Err(FormatError::LimitExceeded("native byte-export combined resident bytes".into()));
        }
        self.live.set(next);
        Ok(())
    }
    fn release(&self, bytes: usize) {
        self.live.set(self.live.get().saturating_sub(bytes));
    }
}
#[path = "native_output_control.rs"]
mod output_control;

struct MemoryOutput<'a, 'i> {
    bytes: Vec<u8>,
    position: usize,
    ledger: &'a ByteLedger,
    control: &'a Interrupt<'i>,
}
impl MemoryOutput<'_, '_> {
    fn error(&self, message: &'static str) -> std::io::Error {
        std::io::Error::other(message)
    }
    fn grow(&mut self, end: usize) -> std::io::Result<()> {
        use photocraft_format::PreparationObserver;
        if self.control.cancelled() {
            return Err(self.error("native byte-export cancelled"));
        }
        if end > self.bytes.capacity() {
            let target = end.checked_add(8191).map(|n| n / 8192 * 8192).ok_or_else(|| self.error("native buffer size overflow"))?;
            // Reserve old and new allocation simultaneously, with allocator growth headroom.
            let reservation = target.checked_mul(2).ok_or_else(|| self.error("native buffer growth overflow"))?;
            self.ledger.reserve(reservation).map_err(|_| self.error("native byte-export combined resident bytes"))?;
            let old = self.bytes.capacity();
            if self.bytes.try_reserve_exact(target.saturating_sub(self.bytes.len())).is_err() {
                self.ledger.release(reservation);
                return Err(self.error("native buffer allocation denied"));
            }
            let actual = self.bytes.capacity();
            if actual > reservation {
                // Do not expose an allocation whose actual capacity exceeded reserved headroom.
                return Err(self.error("native allocator capacity exceeds reservation"));
            }
            self.ledger.release(reservation - actual);
            self.ledger.release(old);
        }
        output_control::zero_extend(&mut self.bytes, end, self.control)
    }
}
impl Write for MemoryOutput<'_, '_> {
    fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
        let end = self.position.checked_add(input.len()).ok_or_else(|| self.error("native output offset overflow"))?;
        self.grow(end)?;
        let destination = self.bytes.get_mut(self.position..end).ok_or_else(|| std::io::Error::other("native output bounds"))?;
        output_control::copy(destination, input, self.control)?;
        self.position = end;
        Ok(input.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if self.control.cancelled() { Err(self.error("native byte-export cancelled")) } else { Ok(()) }
    }
}
impl Seek for MemoryOutput<'_, '_> {
    fn seek(&mut self, from: std::io::SeekFrom) -> std::io::Result<u64> {
        if self.control.cancelled() {
            return Err(self.error("native byte-export cancelled"));
        }
        let position = match from {
            std::io::SeekFrom::Start(n) => i128::from(n),
            std::io::SeekFrom::Current(n) => self.position as i128 + i128::from(n),
            std::io::SeekFrom::End(n) => self.bytes.len() as i128 + i128::from(n),
        };
        self.position = usize::try_from(position).map_err(|_| self.error("native seek out of range"))?;
        u64::try_from(self.position).map_err(|_| self.error("native seek out of range"))
    }
}
/// Controlled in-memory ZIP64 native export. Output capacity shares preparation's
/// resident ledger until all prepared objects and fresh-writer caches are dropped.
/// ZIP framing is deterministic; raw-Zstd streaming encoding differs from the legacy
/// compressed archive, while manifest, object and preview decoded contents are unchanged.
pub fn save_native_bytes(doc: &Document, limits: NativeSaveLimits, control: &Interrupt<'_>) -> Result<(Vec<u8>, NativeSaveResult), IoError> {
    poll(control)?;
    control.progress(0.05);
    let (writer, previews, warnings, retained_payload_bytes, retained_model_slice_bytes) = prepare(doc, limits, control)?;
    control.progress(0.7);
    poll(control)?;
    let preparation = writer.preparation_limits();
    let base = limits.preparation.resident_bytes.checked_sub(preparation.resident_bytes).ok_or_else(|| limit("native byte-export budget"))?;
    drop(writer);
    let ledger = ByteLedger { live: std::cell::Cell::new(base), maximum: limits.preparation.resident_bytes };
    let output = MemoryOutput { bytes: Vec::new(), position: 0, ledger: &ledger, control };
    let result = PcraftWriter::save_zip_stream_observed(doc, &previews, output, limits.archive, preparation, control, &ledger);
    if control.cancelled() {
        return Err(IoError::Cancelled);
    }
    let (output, stats) = result.map_err(|error| if matches!(error, FormatError::Cancelled) { IoError::Cancelled } else { error.into() })?;
    control.progress(0.95);
    poll(control)?;
    Ok((output.bytes, NativeSaveResult { warnings, stats, retained_payload_bytes, retained_model_slice_bytes }))
}
/// Ephemeral standard-profile preparation for a caller-owned staged sink.
/// Holds immutable document/control borrows, never a locator or filesystem grant.
/// The caller must retain source/alias guards through every final rename and
/// remove unpublished output on failure; write() itself does not publish a file.
pub struct PreparedNativeStream<'doc, 'control, 'interrupt> {
    document: &'doc Document,
    control: &'control Interrupt<'interrupt>,
    limits: NativeSaveLimits,
    writer: PcraftWriter,
    previews: photocraft_format::SaveOptions,
    warnings: Vec<String>,
    retained_payload_bytes: usize,
    retained_model_slice_bytes: usize,
}

/// Perform retained-payload/model admission, preview preparation and remaining
/// writer-budget setup before the caller creates a temporary output. The native
/// writer still validates/collects its bounded persistence objects at write time.
/// This does not enable native_large or claim all Document/RSS allocations.
pub fn prepare_native_stream<'doc, 'control, 'interrupt>(
    document: &'doc Document,
    limits: NativeSaveLimits,
    control: &'control Interrupt<'interrupt>,
) -> Result<PreparedNativeStream<'doc, 'control, 'interrupt>, IoError> {
    let (writer, previews, warnings, retained_payload_bytes, retained_model_slice_bytes) = prepare(document, limits, control)?;
    poll(control)?;
    Ok(PreparedNativeStream { document, control, limits, writer, previews, warnings, retained_payload_bytes, retained_model_slice_bytes })
}

impl PreparedNativeStream<'_, '_, '_> {
    /// Consume the prepared plan exactly once. Output remains caller-owned and
    /// unpublished; bounded standard ZIP writing never materializes an archive
    /// Vec. Callback/capability publication authority belongs to the caller.
    pub fn write<W: Write + Seek>(mut self, output: W) -> Result<(W, NativeSaveResult), IoError> {
        poll(self.control)?;
        let (output, stats) = self
            .writer
            .save_zip_stream(self.document, &self.previews, output, self.limits.archive, self.limits.preparation.object_bytes, self.control)
            .map_err(|error| if matches!(error, FormatError::Cancelled) { IoError::Cancelled } else { error.into() })?;
        poll(self.control)?;
        Ok((
            output,
            NativeSaveResult {
                warnings: self.warnings,
                stats,
                retained_payload_bytes: self.retained_payload_bytes,
                retained_model_slice_bytes: self.retained_model_slice_bytes,
            },
        ))
    }

    /// Consume the same prepared plan for an actual UTF-8 destination basename.
    /// Serialization owns the paid title string; publication remains caller-owned.
    pub fn write_named<W: Write + Seek>(mut self, destination_name: &str, output: W) -> Result<(W, NativeSaveResult), IoError> {
        poll(self.control)?;
        if destination_name.is_empty()
            || destination_name.len() > 4096
            || matches!(destination_name, "." | "..")
            || destination_name.contains(['/', '\\', '\0'])
        {
            return Err(IoError::Unsupported("native destination requires a bounded UTF-8 file name".into()));
        }
        let (output, stats) = self
            .writer
            .save_zip_stream_named(
                self.document,
                destination_name,
                &self.previews,
                output,
                self.limits.archive,
                self.limits.preparation.object_bytes,
                self.control,
            )
            .map_err(|error| if matches!(error, FormatError::Cancelled) { IoError::Cancelled } else { error.into() })?;
        poll(self.control)?;
        Ok((
            output,
            NativeSaveResult {
                warnings: self.warnings,
                stats,
                retained_payload_bytes: self.retained_payload_bytes,
                retained_model_slice_bytes: self.retained_model_slice_bytes,
            },
        ))
    }

    /// Consume the same admitted document/previews into a caller-owned native
    /// directory sink. The sink alone owns entry identity, exclusions, temporary
    /// writes and manifest-last publication; IO acquires no file authority.
    pub fn write_directory(mut self, sink: &mut impl photocraft_format::DirectoryEntrySink) -> Result<NativeSaveResult, IoError> {
        poll(self.control)?;
        let previous = self.writer.preparation_limits();
        let scratch = usize::try_from(sink.resident_scratch_bytes()).map_err(|_| limit("native directory sink scratch"))?;
        let remaining = previous.resident_bytes.checked_sub(scratch).ok_or_else(|| limit("native directory sink resident envelope"))?;
        self.writer.set_preparation_limits(PreparationLimits { resident_bytes: remaining, ..previous })?;
        let plan = self.writer.prepare_directory_stream(self.document, &self.previews, self.limits.preparation.object_bytes, self.control);
        self.writer.set_preparation_limits(previous)?;
        let plan = plan.map_err(|error| if matches!(error, FormatError::Cancelled) { IoError::Cancelled } else { error.into() })?;
        let stats = plan.write(sink).map_err(|error| if matches!(error, FormatError::Cancelled) { IoError::Cancelled } else { error.into() })?;
        // Manifest commit is success. No cancellation/error work follows it.
        Ok(NativeSaveResult {
            warnings: self.warnings,
            stats,
            retained_payload_bytes: self.retained_payload_bytes,
            retained_model_slice_bytes: self.retained_model_slice_bytes,
        })
    }
}

pub fn save_native_stream<W: Write + Seek>(
    doc: &Document,
    output: W,
    limits: NativeSaveLimits,
    control: &Interrupt<'_>,
) -> Result<(W, NativeSaveResult), IoError> {
    prepare_native_stream(doc, limits, control)?.write(output)
}
#[cfg(not(target_arch = "wasm32"))]
pub fn save_native_path(doc: &Document, path: &std::path::Path, limits: NativeSaveLimits, control: &Interrupt<'_>) -> Result<NativeSaveResult, IoError> {
    save_native_path_guarded(doc, path, limits, control, &|_| Ok(()))
}

/// Compatibility callback without a shared inspection allowance. The callback
/// still runs before preparation and at every final commit guard. Callers that
/// inspect destination content should use the resident-aware additive API.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_native_path_guarded(
    doc: &Document,
    path: &std::path::Path,
    limits: NativeSaveLimits,
    control: &Interrupt<'_>,
    before_commit: &dyn Fn(&std::path::Path) -> Result<(), IoError>,
) -> Result<NativeSaveResult, IoError> {
    poll(control)?;
    before_commit(path)?;
    save_native_path_guarded_with_resident_guard(doc, path, limits, control, &|target, _| before_commit(target))
}

/// Stream native output with one shared resident envelope for prepared owners,
/// persistence caches and sequential destination inspection. The callback must
/// finish/release its inspection scratch before returning; its allowance includes
/// SOURCE_PROTECTION_FIXED_BYTES and is lower-only. No blocked-read or RSS claim.
/// External callback captures (including all-open WriteProtection/hash owners)
/// remain caller-precharged storage; this function charges its own hash copies.
/// Preparation is private; guards run before output creation and every rename.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_native_path_guarded_with_resident_guard(
    doc: &Document,
    path: &std::path::Path,
    limits: NativeSaveLimits,
    control: &Interrupt<'_>,
    before_commit: &dyn Fn(&std::path::Path, u64) -> Result<(), IoError>,
) -> Result<NativeSaveResult, IoError> {
    poll(control)?;
    let destination_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && name.len() <= 4096)
        .ok_or_else(|| IoError::Unsupported("native destination requires a bounded UTF-8 file name".into()))?;
    if path.as_os_str().len() > 4096 || crate::is_xcf_filename(&path.to_string_lossy()) {
        return Err(IoError::Unsupported("source-protection.destination: choose a writable native destination".into()));
    }
    // Validate the caller's original envelope before lowering it for retained
    // fingerprint storage; subtraction must not hide a raised standard limit.
    let mut limit_check = PcraftWriter::new();
    limit_check.set_preparation_limits(limits.preparation)?;
    drop(limit_check);
    let retained_input = retained_native_input_bytes(doc, limits.preparation, control)?;
    let collection_resident = limits
        .preparation
        .resident_bytes
        .checked_sub(retained_input)
        .ok_or_else(|| limit("source-protection.resident-limit: retained document leaves no fingerprint headroom"))?;
    let fingerprints = crate::source_protection::document_fingerprints_with_resident_limit(
        doc,
        u64::try_from(collection_resident).map_err(|_| limit("source-protection.resident-limit"))?.min(crate::source_protection::resident_ceiling()),
        control,
    )
    .map_err(|error| {
        if matches!(error, crate::source_protection::SourceProtectionError::Cancelled) { IoError::Cancelled } else { IoError::Unsupported(error.to_string()) }
    })?;
    let mut fingerprint_bytes = fingerprints
        .capacity()
        .checked_mul(std::mem::size_of::<String>())
        .and_then(|n| n.checked_add(std::mem::size_of::<Vec<String>>()))
        .ok_or_else(|| limit("source-protection.fingerprint resident overflow"))?;
    for fingerprint in &fingerprints {
        poll(control)?;
        fingerprint_bytes = fingerprint_bytes.checked_add(fingerprint.capacity()).ok_or_else(|| limit("source-protection.fingerprint resident overflow"))?;
    }
    let prepared_limits = NativeSaveLimits {
        preparation: PreparationLimits {
            resident_bytes: limits
                .preparation
                .resident_bytes
                .checked_sub(fingerprint_bytes)
                .ok_or_else(|| limit("source-protection.resident-limit: fingerprint owners exceed save envelope"))?,
            ..limits.preparation
        },
        ..limits
    };
    // prepare() debits retained document payload/model, previews and archive
    // workspace. This fresh writer has no persistence objects/caches yet, so
    // its remaining envelope is available to the initial sequential inspection.
    // The FnOnce writer closure below drops every writer cache and preview
    // before any final/retry guard. Reuse the same conservative allowance then
    // without refunding the already-debited preview/archive reservation.
    let (mut writer, previews, warnings, retained_payload_bytes, retained_model_slice_bytes) = prepare(doc, prepared_limits, control)?;
    let inspection_resident = u64::try_from(writer.preparation_limits().resident_bytes)
        .map_err(|_| limit("source-protection.resident-limit"))?
        .min(crate::source_protection::resident_ceiling());
    if inspection_resident < crate::source_protection::SOURCE_PROTECTION_FIXED_BYTES {
        return Err(limit("source-protection.resident-limit: no shared native-save inspection headroom"));
    }
    poll(control)?;
    let guard = |target: &std::path::Path| -> Result<(), IoError> {
        poll(control)?;
        if target.as_os_str().len() > 4096 || crate::is_xcf_filename(&target.to_string_lossy()) {
            return Err(IoError::Unsupported("source-protection.destination: choose a writable native destination".into()));
        }
        match std::fs::File::open(target) {
            Ok(mut file) => {
                let metadata = file.metadata().map_err(FormatError::from)?;
                if !metadata.is_file() {
                    return Err(IoError::Unsupported("source-protection.destination: not a regular file".into()));
                }
                crate::source_protection::check_reader_with_resident_limit(&mut file, metadata.len(), &fingerprints, inspection_resident, control).map_err(
                    |error| {
                        if matches!(error, crate::source_protection::SourceProtectionError::Cancelled) {
                            IoError::Cancelled
                        } else {
                            IoError::Unsupported(error.to_string())
                        }
                    },
                )?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(FormatError::Io(error).into()),
        }
        poll(control)?;
        before_commit(target, inspection_resident)?;
        poll(control)
    };
    guard(path)?;
    let stats = photocraft_format::atomic::atomic_write_stream_guarded(
        path,
        move |file| {
            let result = writer
                .save_zip_stream_named(doc, destination_name, &previews, file, limits.archive, limits.preparation.object_bytes, control)
                .map(|(_, stats)| stats)
                .map_err(|error| NativeSaveCommitError(if matches!(error, FormatError::Cancelled) { IoError::Cancelled } else { error.into() }));
            // Atomic's final guard runs only after this closure returns. Retained
            // manifests/source caches therefore cannot overlap decoder scratch.
            drop(writer);
            drop(previews);
            result
        },
        |target| {
            guard(path).map_err(NativeSaveCommitError)?;
            if target != path {
                guard(target).map_err(NativeSaveCommitError)?;
            }
            Ok(())
        },
    )
    .map_err(|error: NativeSaveCommitError| error.0)?;
    Ok(NativeSaveResult { warnings, stats, retained_payload_bytes, retained_model_slice_bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn memory_sink_bounds_seek_growth_and_actual_write_cancellation() {
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let predicate = || cancelled.load(std::sync::atomic::Ordering::Relaxed);
        let control = Interrupt::cancel_only(&predicate);
        let ledger = ByteLedger { live: std::cell::Cell::new(0), maximum: 32768 };
        let mut sink = MemoryOutput { bytes: Vec::new(), position: 0, ledger: &ledger, control: &control };
        sink.write_all(b"own").unwrap();
        assert_eq!(ledger.live.get(), sink.bytes.capacity());
        assert!(sink.seek(std::io::SeekFrom::Current(-4)).is_err());
        assert_eq!(sink.position, 3);
        cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(sink.write_all(b"more").is_err());
        assert_eq!(sink.bytes, b"own");
        assert_eq!(sink.position, 3);
        cancelled.store(false, std::sync::atomic::Ordering::Relaxed);
        assert!(sink.write_all(&vec![0; 32768]).is_err());
        assert_eq!(sink.bytes, b"own");
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn shared_inspection_allowance_refusal_precedes_output_creation() {
        let doc = Document::new("guard", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let dir = std::env::temp_dir().join(format!(
            "photocraft-native-guard-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("output.pcraft");
        let limits = NativeSaveLimits::default();
        let observed = std::cell::Cell::new(0u64);
        let result = save_native_path_guarded_with_resident_guard(&doc, &path, limits, &Interrupt::NONE, &|_, allowance| {
            observed.set(allowance);
            assert!(allowance >= crate::source_protection::SOURCE_PROTECTION_FIXED_BYTES);
            assert!(allowance < limits.preparation.resident_bytes as u64);
            assert!(allowance <= crate::source_protection::resident_ceiling());
            Err(IoError::Unsupported("own guard refusal".into()))
        });
        assert!(result.is_err());
        assert!(observed.get() > 0);
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir(&dir).unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn final_shared_guard_cancellation_keeps_original_destination() {
        let doc = Document::new("guard", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let dir = std::env::temp_dir().join(format!(
            "photocraft-native-final-guard-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("output.pcraft");
        std::fs::write(&path, b"own unrelated original").unwrap();
        let calls = std::cell::Cell::new(0usize);
        let allowance = std::cell::Cell::new(None);
        let result = save_native_path_guarded_with_resident_guard(&doc, &path, NativeSaveLimits::default(), &Interrupt::NONE, &|_, resident| {
            if let Some(previous) = allowance.get() {
                assert_eq!(resident, previous);
            }
            allowance.set(Some(resident));
            calls.set(calls.get() + 1);
            if calls.get() >= 3 { Err(IoError::Cancelled) } else { Ok(()) }
        });
        assert!(matches!(result, Err(IoError::Cancelled)));
        assert!(calls.get() >= 3);
        assert_eq!(std::fs::read(&path).unwrap(), b"own unrelated original");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn path_title_changes_only_serialized_name_while_anonymous_bytes_keep_original() {
        let doc = Document::with_background(
            "original imported title",
            photocraft_doc::Size::new(2, 1),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::F64,
            photocraft_color::Color::WHITE,
        );
        let original = doc.clone(); // External assertion snapshot, not a publication receipt.
        let dir = std::env::temp_dir().join(format!(
            "photocraft-native-title-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("named output.pcraft");
        let guards = std::cell::Cell::new(0usize);
        save_native_path_guarded(&doc, &path, NativeSaveLimits::default(), &Interrupt::NONE, &|_| {
            guards.set(guards.get() + 1);
            Ok(())
        })
        .unwrap();
        assert!(guards.get() >= 2);
        let reopened = photocraft_format::load_from_bytes(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(reopened, *crate::named_for_file(&doc, "named output.pcraft"));
        let (bytes, _) = save_native_bytes(&doc, NativeSaveLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), original);
        assert_eq!(doc, original);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn non_utf8_destination_title_refuses_before_output_after_initial_guard() {
        use std::os::windows::ffi::OsStringExt;
        let doc = Document::new("unchanged", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let name = std::ffi::OsString::from_wide(&[0xd800, b'.' as u16, b'p' as u16]);
        let path = std::env::temp_dir().join(name);
        let calls = std::cell::Cell::new(0usize);
        let result = save_native_path_guarded(&doc, &path, NativeSaveLimits::default(), &Interrupt::NONE, &|_| {
            calls.set(calls.get() + 1);
            Ok(())
        });
        assert!(matches!(result, Err(IoError::Unsupported(_))));
        assert_eq!(calls.get(), 1, "the established outer guard runs once; preparation/output/final guards are not reached");
        assert!(!path.exists());
    }
    #[test]
    fn controlled_previews_match_previous_native_save_values() {
        for depth in [photocraft_doc::SampleType::U16, photocraft_doc::SampleType::F64] {
            let doc = Document::with_background(
                "own",
                photocraft_doc::Size::new(17, 13),
                photocraft_doc::ColorMode::Rgb,
                depth,
                photocraft_doc::Color::rgba(0.2, 0.6, 0.8, 0.5),
            );
            let old = crate::native_previews(&doc);
            let new = crate::native_previews_with_interrupt(&doc, &Interrupt::NONE, 1 << 30).unwrap();
            assert_eq!(new.0.thumbnail, old.0.thumbnail);
            assert_eq!(new.0.composite, old.0.composite);
            assert_eq!(new.1, old.1);
        }
    }
}

#[cfg(test)]
#[path = "native_save_hdr_tests.rs"]
mod hdr_tests;

#[cfg(test)]
mod census_receipt_tests {
    use super::*;
    #[test]
    fn receipt_join_is_exact_document_not_equal_document_or_asserted_bytes() {
        let doc = Document::new("receipt", photocraft_doc::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let other = doc.clone();
        let model = retained_native_model_census(&doc, PreparationLimits::default(), &Interrupt::NONE).unwrap();
        let payload = retained_native_payload_census(&other, PreparationLimits::default(), &Interrupt::NONE).unwrap();
        assert!(NativeCurrentCensus::join(model, payload).is_err());
        let model = retained_native_model_census(&doc, PreparationLimits::default(), &Interrupt::NONE).unwrap();
        let payload = retained_native_payload_census(&doc, PreparationLimits::default(), &Interrupt::NONE).unwrap();
        let model_bytes = model.retained_bytes();
        let payload_bytes = payload.retained_bytes();
        let joined = NativeCurrentCensus::join(model, payload).unwrap();
        assert!(joined.belongs_to(&doc));
        assert!(!joined.belongs_to(&other));
        assert_eq!(joined.retained_bytes(), model_bytes + payload_bytes);
        assert_eq!(joined.model_retained_bytes(), retained_native_model_bytes(&doc, PreparationLimits::default(), &Interrupt::NONE).unwrap());
        assert_eq!(joined.payload_retained_bytes(), retained_native_payload_bytes(&doc, PreparationLimits::default(), &Interrupt::NONE).unwrap());
        assert!(joined.visit_scope().model_visits > 0);
    }
    #[test]
    fn cancellation_during_origin_validation_preserves_typed_error() {
        let format = photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::U8, false);
        let mut surface = Surface::try_with_default_bytes(format, &[0]).unwrap();
        surface.try_write_pixel_f64(0, 0, &[1.0]).unwrap();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancelled = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1 > 1;
        let control = Interrupt::cancel_only(&cancelled);
        let mut counter = Payloads {
            seen: HashSet::new(),
            bytes: 0,
            index: 0,
            map_bytes: 0,
            limits: PreparationLimits::default(),
            control: &control,
            visits: NativeCensusVisits::default(),
            work: None,
        };
        assert!(matches!(counter.surface(&surface), Err(IoError::Cancelled)));
        assert!(calls.load(std::sync::atomic::Ordering::Relaxed) >= 2);
        assert_eq!(counter.visits.identity_visits, 0);
        assert_eq!(counter.bytes, 0);
        assert_eq!(counter.map_bytes, 0);
    }
    #[test]
    fn cancelled_census_cannot_produce_a_receipt() {
        let doc = Document::new("cancel", photocraft_doc::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let cancelled = || true;
        let control = Interrupt::cancel_only(&cancelled);
        assert!(matches!(retained_native_model_census(&doc, PreparationLimits::default(), &control), Err(IoError::Cancelled)));
        assert!(matches!(retained_native_payload_census(&doc, PreparationLimits::default(), &control), Err(IoError::Cancelled)));
    }
}

#[cfg(test)]
#[path = "native_save_census_work_tests.rs"]
mod census_work_tests;
