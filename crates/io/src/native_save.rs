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
pub(super) struct Payloads<'a, 'i> {
    seen: HashSet<(u8, usize)>,
    bytes: usize,
    index: usize,
    map_bytes: usize,
    limits: PreparationLimits,
    control: &'a Interrupt<'i>,
}
impl Payloads<'_, '_> {
    pub(super) fn add(&mut self, tag: u8, pointer: usize, bytes: usize) -> Result<(), IoError> {
        poll(self.control)?;
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
        surface.try_validate_tile_origins_with(self.control).map_err(|e| IoError::Unsupported(e.to_string()))?;
        self.map_bytes = self.map_bytes.checked_add(surface.tile_count().checked_mul(256).ok_or_else(|| limit("native raster map overflow"))?).ok_or_else(|| limit("native raster map overflow"))?;
        self.add(3, surface.default_bytes().as_ptr() as usize, surface.default_bytes().len())?;
        for (_, tile) in surface.tiles() {
            self.add(4, Arc::as_ptr(tile) as usize, tile.bytes().len())?;
        }
        Ok(())
    }
    fn layer(&mut self, layer: &Layer, depth: usize) -> Result<(), IoError> {
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
            if let Some(original)=&video.original_frames {
                for surface in original {self.surface(surface)?;}
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
fn retained_native_payload_and_maps(doc: &Document, limits: PreparationLimits, control: &Interrupt<'_>) -> Result<(usize, usize), IoError> {
    let mut check = PcraftWriter::new();
    check.set_preparation_limits(limits)?;
    poll(control)?;
    let mut payloads = Payloads { seen: HashSet::new(), bytes: 0, index: 0, map_bytes: 0, limits, control };
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
        if let Some(bytes) = &path.psd_raw {
            payloads.blob(bytes)?;
        }
    }
    if let Some(state) = &doc.xcf_state {
        for resource in &state.resources {
            if let Some(bytes) = &resource.owned_bytes {
                payloads.owned(bytes.as_ref())?;
            }
        }
        for source in &state.source_items {
            if let Some(cache) = &source.cache {
                payloads.owned(cache.bytes.as_ref())?;
            }
        }
        for record in &state.opaque {
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
    Ok((payloads.bytes, payloads.map_bytes))
}
pub fn retained_native_payload_bytes(doc: &Document, limits: PreparationLimits, control: &Interrupt<'_>) -> Result<usize, IoError> {
    Ok(retained_native_payload_and_maps(doc, limits, control)?.0)
}
pub(crate) fn retained_native_map_bytes(doc: &Document, limits: PreparationLimits, control: &Interrupt<'_>) -> Result<usize, IoError> {
    Ok(retained_native_payload_and_maps(doc, limits, control)?.1)
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
pub fn retained_native_model_bytes(doc: &Document, limits: PreparationLimits, control: &Interrupt<'_>) -> Result<usize, IoError> {
    let mut check = PcraftWriter::new();
    check.set_preparation_limits(limits)?;
    poll(control)?;
    let mut counter = Payloads { seen: HashSet::new(), bytes: 0, index: 0, map_bytes: 0, limits, control };
    super::native_save_model::measure(doc, &mut counter, control)?;
    poll(control)?;
    Ok(counter.bytes)
}

/// Retained payloads plus the measured public canonical/structural String and Vec capacity slice.
/// Legacy text/effect/vector/private map allocations and allocator overhead remain unmeasured.
pub fn retained_native_input_bytes(doc: &Document, limits: PreparationLimits, control: &Interrupt<'_>) -> Result<usize, IoError> {
    let payload = retained_native_payload_bytes(doc, limits, control)?;
    let mut counter = Payloads { seen: HashSet::new(), bytes: payload, index: 0, map_bytes: 0, limits, control };
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
    let (writer, previews, warnings, retained_payload_bytes, retained_model_slice_bytes) = prepare(doc, limits, control)?;
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
        let (output, stats) = self.writer
            .save_zip_stream(self.document, &self.previews, output, self.limits.archive, self.limits.preparation.object_bytes, self.control)
            .map_err(|error| if matches!(error, FormatError::Cancelled) { IoError::Cancelled } else { error.into() })?;
        poll(self.control)?;
        Ok((output, NativeSaveResult { warnings: self.warnings, stats, retained_payload_bytes: self.retained_payload_bytes, retained_model_slice_bytes: self.retained_model_slice_bytes }))
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
        let plan = self.writer.prepare_directory_stream(
            self.document, &self.previews, self.limits.preparation.object_bytes, self.control,
        );
        self.writer.set_preparation_limits(previous)?;
        let plan = plan.map_err(|error| if matches!(error, FormatError::Cancelled) { IoError::Cancelled } else { error.into() })?;
        let stats = plan.write(sink)
            .map_err(|error| if matches!(error, FormatError::Cancelled) { IoError::Cancelled } else { error.into() })?;
        // Manifest commit is success. No cancellation/error work follows it.
        Ok(NativeSaveResult { warnings: self.warnings, stats, retained_payload_bytes: self.retained_payload_bytes, retained_model_slice_bytes: self.retained_model_slice_bytes })
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

/// Add caller-owned runtime exclusions without importing engine types or authority.
/// The callback runs before preparation and before every final rename attempt.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_native_path_guarded(
    doc: &Document,
    path: &std::path::Path,
    limits: NativeSaveLimits,
    control: &Interrupt<'_>,
    before_commit: &dyn Fn(&std::path::Path) -> Result<(), IoError>,
) -> Result<NativeSaveResult, IoError> {
    let fingerprints = crate::source_protection::document_fingerprints_with_interrupt(doc, control)
        .map_err(|error| if matches!(error, crate::source_protection::SourceProtectionError::Cancelled) {
            IoError::Cancelled
        } else { IoError::Unsupported(error.to_string()) })?;
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
                crate::source_protection::check_reader_with_interrupt(&mut file, metadata.len(), &fingerprints, control)
                    .map_err(|error| if matches!(error, crate::source_protection::SourceProtectionError::Cancelled) { IoError::Cancelled } else { IoError::Unsupported(error.to_string()) })?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(FormatError::Io(error).into()),
        }
        poll(control)?;
        before_commit(target)?;
        poll(control)
    };
    guard(path)?;
    let (mut writer, previews, warnings, retained_payload_bytes, retained_model_slice_bytes) = prepare(doc, limits, control)?;
    let stats = photocraft_format::atomic::atomic_write_stream_guarded(
        path,
        |file| {
            writer
                .save_zip_stream(doc, &previews, file, limits.archive, limits.preparation.object_bytes, control)
                .map(|(_, stats)| stats)
                .map_err(|error| NativeSaveCommitError(if matches!(error, FormatError::Cancelled) { IoError::Cancelled } else { error.into() }))
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
