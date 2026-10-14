//! Document â‡„ manifest conversion. Binary data is routed through a
//! [`Sink`] (save) or [`Fetch`] (load) keyed by BLAKE3 hash.

use std::sync::Arc;

use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::{
    AlphaChannel, CompAppearance, CompLayerState, DocId, Document, Effects, FillCache, Group, Layer, LayerComp, LayerContent, LayerId, LayerMask, Metadata,
    NamedPath, Pattern, ShapeLayer, SmartContentsId, SmartObject, SmartSource, TextLayer,
};
use photocraft_geom::{TILE_SIZE, TileCoord};
use photocraft_raster::{Surface, Tile};

use crate::manifest::*;
use crate::{FormatError, Result};

/// Deepest group nesting a bundle may hold â€” the one nesting limit every consumer of the layer
/// tree shares; see [`photocraft_doc::MAX_GROUP_DEPTH`].
pub use photocraft_doc::MAX_GROUP_DEPTH;

fn too_deep() -> FormatError {
    FormatError::LimitExceeded(format!("layer groups nested deeper than {MAX_GROUP_DEPTH}"))
}

/// Refuses to save a document the loader would reject for its group nesting.
pub(crate) fn check_nesting(layers: &[Layer], control: photocraft_raster::Interrupt<'_>) -> Result<()> {
    fn visit(layers: &[Layer], depth: usize, control: photocraft_raster::Interrupt<'_>) -> Result<()> {
        if depth > MAX_GROUP_DEPTH && !layers.is_empty() {
            return Err(too_deep());
        }
        for layer in layers {
            if control.cancelled() {
                return Err(FormatError::Cancelled);
            }
            if let LayerContent::Group(group) = &layer.content {
                visit(&group.children, depth + 1, control)?;
            }
        }
        Ok(())
    }
    visit(layers, 0, control)
}

pub(crate) trait Sink<'doc> {
    fn finite_raster(&mut self, _layer: &photocraft_doc::BackedLayer) -> Result<crate::finite_raster::FiniteRasterM> {
        Err(FormatError::Unsupported("backed raster requires accounted streaming writer".into()))
    }
    fn poll(&self) -> Result<()>;
    fn reserve(&mut self, bytes: usize) -> Result<()>;
    fn release(&mut self, bytes: usize);
    fn max_manifest_bytes(&self) -> usize;
    fn control(&self) -> photocraft_raster::Interrupt<'_>;
    /// Register a tile, returning its hash.
    fn tile(&mut self, format: PixelFormat, tile: &Arc<Tile>) -> Result<Hash>;
    fn blob(&mut self, data: &Arc<Vec<u8>>) -> Result<Hash>;
    fn borrowed_blob(&mut self, data: &'doc [u8]) -> Result<Hash>;
}

pub(crate) trait Fetch {
    fn finite_raster(&mut self, _raster: &crate::finite_raster::FiniteRasterM) -> Result<photocraft_doc::BackedLayer> {
        Err(FormatError::Unsupported("finite raster requires owned authenticated archive".into()))
    }
    fn accounts_model(&self) -> bool {
        false
    }
    fn model_reserve(&mut self, _bytes: u64) -> Result<()> {
        Ok(())
    }
    fn model_poll(&self, _units: u64) -> Result<()> {
        Ok(())
    }
    /// Canonical (little-endian) tile bytes of the given expected length.
    fn tile(&mut self, hash: &str, len: usize) -> Result<Vec<u8>>;
    fn blob(&mut self, hash: &str) -> Result<Arc<Vec<u8>>>;
    fn blob_bounded(&mut self, hash: &str, max: usize) -> Result<Arc<Vec<u8>>> {
        let bytes = self.blob(hash)?;
        if bytes.len() > max {
            return Err(FormatError::LimitExceeded("bounded source cache blob".into()));
        }
        Ok(bytes)
    }
    fn blob_bounded_with(&mut self, hash: &str, max: usize, control: &photocraft_raster::Interrupt<'_>) -> Result<Arc<Vec<u8>>> {
        if control.cancelled() {
            return Err(FormatError::Cancelled);
        }
        let bytes = self.blob_bounded(hash, max)?;
        if control.cancelled() {
            return Err(FormatError::Cancelled);
        }
        Ok(bytes)
    }
    fn source_cache_bytes(&mut self, hash: &str, max: usize, control: &photocraft_raster::Interrupt<'_>) -> Result<Arc<Vec<u8>>> {
        self.blob_bounded_with(hash, max, control)
    }
}

#[cfg(test)]
#[cfg(test)]
pub(crate) fn checked_owned_bytes(bytes: Arc<Vec<u8>>, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<u8>> {
    if control.cancelled() {
        return Err(FormatError::Cancelled);
    }
    match Arc::try_unwrap(bytes) {
        Ok(bytes) => Ok(bytes),
        Err(bytes) => {
            let mut output = Vec::new();
            output.try_reserve_exact(bytes.len()).map_err(|_| FormatError::LimitExceeded("source cache allocation".into()))?;
            for chunk in bytes.chunks(8192) {
                if control.cancelled() {
                    return Err(FormatError::Cancelled);
                }
                output.extend_from_slice(chunk);
            }
            Ok(output)
        }
    }
}

fn decode_xcf_state(bytes: &[u8], control: &photocraft_raster::Interrupt<'_>) -> Result<photocraft_doc::XcfState> {
    struct CheckedReader<'a, 'b> {
        bytes: std::io::Cursor<&'a [u8]>,
        control: &'a photocraft_raster::Interrupt<'b>,
        remaining: usize,
        cancelled: bool,
    }
    impl std::io::Read for CheckedReader<'_, '_> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.remaining == 0 {
                if self.control.cancelled() {
                    self.cancelled = true;
                    // Read::read_exact retries Interrupted; cancellation must end this parse.
                    return Err(std::io::Error::other("editable state parsing cancelled"));
                }
                self.remaining = 8192;
            }
            let count = buffer.len().min(self.remaining);
            let output = buffer.get_mut(..count).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid JSON read bound"))?;
            let count = std::io::Read::read(&mut self.bytes, output)?;
            self.remaining = self.remaining.saturating_sub(count);
            Ok(count)
        }
    }
    let mut reader = CheckedReader { bytes: std::io::Cursor::new(bytes), control, remaining: 0, cancelled: false };
    let result = serde_json::from_reader(&mut reader);
    if reader.cancelled { Err(FormatError::Cancelled) } else { result.map_err(FormatError::from) }
}

/// Native state metadata borrows the model; cache payloads are registered as blobs separately.
/// This avoids cloning large source byte buffers just to remove them from JSON.
struct StoredXcfState<'a>(&'a photocraft_doc::XcfState);
/// Raw stored-color enum identity needs reader 9, including nested source configurations.
/// This scans storage identities only; it neither interprets layouts nor resolves profiles.
pub(crate) fn requires_reader_v9_state_with(doc: &Document, control: photocraft_raster::Interrupt<'_>) -> Result<bool> {
    use photocraft_doc::xcf_filters::{FilterArgument, FilterValue};
    fn visit(control: photocraft_raster::Interrupt<'_>, visits: &mut usize) -> Result<()> {
        if control.cancelled() { return Err(FormatError::Cancelled); }
        *visits = visits.checked_add(1).filter(|n| *n <= 100_000).ok_or_else(|| FormatError::LimitExceeded("stored-color feature scan visits".into()))?;
        Ok(())
    }
    fn arguments(values: &[FilterArgument], depth: usize, visits: &mut usize, control: photocraft_raster::Interrupt<'_>) -> Result<bool> {
        if depth > 32 || values.len() > 100_000 { return Err(FormatError::LimitExceeded("stored-color feature scan depth/count".into())); }
        if control.cancelled() { return Err(FormatError::Cancelled); }
        let mut found = false;
        for argument in values {
            visit(control, visits)?;
            match &argument.value {
                FilterValue::StoredColor(_) => found = true,
                FilterValue::Config(config) => found |= arguments(config, depth + 1, visits, control)?,
                FilterValue::Graph(graph) => {
                    if graph.nodes.len() > 4096 || graph.edges.len() > 16384 { return Err(FormatError::LimitExceeded("stored-color graph count".into())); }
                    *visits = visits.checked_add(graph.nodes.len()).and_then(|n| n.checked_add(graph.edges.len()))
                        .and_then(|n| n.checked_add(usize::from(graph.input.is_some()))).filter(|n| *n <= 100_000)
                        .ok_or_else(|| FormatError::LimitExceeded("stored-color graph visits".into()))?;
                    for node in &graph.nodes {
                        if control.cancelled() { return Err(FormatError::Cancelled); }
                        found |= arguments(&node.arguments, depth + 2, visits, control)?;
                    }
                }
                _ => {}
            }
        }
        Ok(found)
    }
    let mut visits = 0;
    if control.cancelled() { return Err(FormatError::Cancelled); }
    let mut found = false;
    if let Some(state) = &doc.xcf_state {
        if state.source_items.len() > 10_000 || state.filter_stacks.len() > 10_000 { return Err(FormatError::LimitExceeded("stored-color state scan count".into())); }
        for source in &state.source_items {
            if control.cancelled() { return Err(FormatError::Cancelled); }
            let configuration = match &source.source {
                photocraft_doc::xcf_items::EditableSource::Text { configuration, .. }
                | photocraft_doc::xcf_items::EditableSource::Vector { configuration, .. }
                | photocraft_doc::xcf_items::EditableSource::Linked { configuration, .. } => configuration,
            };
            found |= arguments(configuration, 0, &mut visits, control)?;
        }
        for stack in &state.filter_stacks {
            if control.cancelled() { return Err(FormatError::Cancelled); }
            if stack.nodes.len() > 1024 { return Err(FormatError::LimitExceeded("stored-color filter scan count".into())); }
            for node in &stack.nodes {
                if control.cancelled() { return Err(FormatError::Cancelled); }
                found |= arguments(&node.arguments, 0, &mut visits, control)?;
            }
        }
    }
    if control.cancelled() { return Err(FormatError::Cancelled); }
    Ok(found)
}
#[cfg(test)]
#[path = "stored_color_reader_tests.rs"]
mod stored_color_reader_tests;
/// Brown is a new enum identity. Older bundles/labels are never renamed.
pub(crate) fn requires_reader_v8_state_with(doc: &Document, control: photocraft_raster::Interrupt<'_>) -> Result<bool> {
    fn visit(layers: &[Layer], depth: usize, control: photocraft_raster::Interrupt<'_>) -> Result<bool> {
        // Match check_nesting exactly: a deepest allowed group may have an
        // empty children slice beyond the limit. This feature scan imposes no
        // new layer-count policy; save/load callers retain their own ceilings.
        if depth > MAX_GROUP_DEPTH && !layers.is_empty() { return Err(too_deep()); }
        for layer in layers {
            if control.cancelled() { return Err(FormatError::Cancelled); }
            if layer.label == photocraft_doc::LabelColor::Brown { return Ok(true); }
            if let LayerContent::Group(group) = &layer.content {
                if visit(&group.children, depth + 1, control)? { return Ok(true); }
            }
        }
        Ok(false)
    }
    if control.cancelled() { return Err(FormatError::Cancelled); }
    visit(&doc.layers, 0, control)
}
pub(crate) fn requires_reader_v7_state_with(doc: &Document, control: photocraft_raster::Interrupt<'_>) -> Result<bool> {
    if control.cancelled() {
        return Err(FormatError::Cancelled);
    }
    if let Some(state) = &doc.xcf_state {
        for source in &state.source_items {
            if control.cancelled() {
                return Err(FormatError::Cancelled);
            }
            if matches!(&source.source, photocraft_doc::xcf_items::EditableSource::Linked { current: Some(_), .. }) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
pub(crate) fn requires_reader_v6_state_with(doc: &Document, control: photocraft_raster::Interrupt<'_>) -> Result<bool> {
    if control.cancelled() {
        return Err(FormatError::Cancelled);
    }
    if let Some(state) = &doc.xcf_state {
        for resource in &state.resources {
            if control.cancelled() {
                return Err(FormatError::Cancelled);
            }
            if resource.resolution.is_some() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
pub(crate) fn requires_reader_v4_state_with(doc: &Document, control: photocraft_raster::Interrupt<'_>) -> Result<bool> {
    if control.cancelled() {
        return Err(FormatError::Cancelled);
    }
    if let Some(state) = &doc.xcf_state {
        for source in &state.source_items {
            if control.cancelled() {
                return Err(FormatError::Cancelled);
            }
            if matches!(&source.source, photocraft_doc::xcf_items::EditableSource::Text { current_origin: Some(_), .. }) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
pub(crate) fn requires_reader_v3_state(doc: &Document) -> bool {
    requires_reader_v3_state_with(doc, photocraft_raster::Interrupt::NONE).unwrap_or(true)
}
pub(crate) fn requires_reader_v3_state_with(doc: &Document, control: photocraft_raster::Interrupt<'_>) -> Result<bool> {
    let cancelled = std::cell::Cell::new(false);
    let poll = || {
        if control.cancelled() {
            cancelled.set(true);
        }
        cancelled.get()
    };
    fn arguments_need_v3(arguments: &[photocraft_doc::xcf_filters::FilterArgument], depth: usize, poll: &impl Fn() -> bool) -> bool {
        if poll() {
            return true;
        }
        use photocraft_doc::xcf_filters::{FilterValue, GraphVersionOrigin};
        if depth > 32 {
            return true;
        }
        arguments.iter().any(|argument| {
            if poll() {
                return true;
            }
            match &argument.value {
                FilterValue::Config(arguments) => arguments_need_v3(arguments, depth + 1, poll),
                FilterValue::Graph(graph) => {
                    graph.input.is_some()
                        || graph
                            .nodes
                            .iter()
                            .any(|node| node.version_origin == GraphVersionOrigin::SchemaResolved || arguments_need_v3(&node.arguments, depth + 1, poll))
                }
                _ => false,
            }
        })
    }
    fn composition_needs_v3(composition: &Option<photocraft_doc::import_compositing::GimpComposition>) -> bool {
        composition
            .as_ref()
            .is_some_and(|composition| composition.compatibility_epoch == photocraft_doc::import_compositing::XCF13_ISOLATED_MULTIPLY_SEED_EPOCH)
    }
    poll();
    let required = doc.working_profile_interpretation != photocraft_doc::WorkingProfileInterpretation::Icc
        || doc.xcf_state.as_ref().is_some_and(|state| {
            state.current_filter_output_precision != photocraft_doc::FilterOutputPrecision::Canonical
                || state.source_items.iter().any(|source| {
                    if poll() {
                        return true;
                    }
                    match &source.source {
                        photocraft_doc::xcf_items::EditableSource::Linked { configuration, .. } => !configuration.is_empty(),
                        photocraft_doc::xcf_items::EditableSource::Text { configuration, .. }
                        | photocraft_doc::xcf_items::EditableSource::Vector { configuration, .. } => arguments_need_v3(configuration, 0, &poll),
                    }
                })
                || state.items.iter().any(|item| {
                    if poll() {
                        return true;
                    }
                    composition_needs_v3(&item.composition)
                })
                || state
                    .filter_stacks
                    .iter()
                    .any(|stack| stack.nodes.iter().any(|node| composition_needs_v3(&node.composition) || arguments_need_v3(&node.arguments, 0, &poll)))
        });
    if cancelled.get() { Err(FormatError::Cancelled) } else { Ok(required) }
}
impl serde::Serialize for StoredXcfState<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        #[derive(serde::Serialize)]
        struct Source<'a> {
            owner: photocraft_doc::item_identity::ItemRef,
            source: &'a photocraft_doc::xcf_items::EditableSource,
            transform: &'a Option<photocraft_doc::xcf_items::TransformState>,
            cache: Option<()>,
        }
        let photocraft_doc::XcfState {
            provenance,
            items,
            resources,
            source_items,
            editable_indexed,
            filter_stacks,
            current_filter_output_precision,
            filter_masks: _,
            floating,
            selection,
            item_sets,
            opaque,
        } = self.0;
        struct Sources<'a>(&'a [photocraft_doc::xcf_items::SourceItem]);
        impl serde::Serialize for Sources<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
                for source in self.0 {
                    sequence.serialize_element(&Source { owner: source.owner, source: &source.source, transform: &source.transform, cache: None })?;
                }
                sequence.end()
            }
        }
        let sources = Sources(source_items);
        let mut map = serializer.serialize_map(Some(if current_filter_output_precision.is_canonical() { 10 } else { 11 }))?;
        map.serialize_entry("provenance", provenance)?;
        map.serialize_entry("items", items)?;
        map.serialize_entry("resources", resources)?;
        map.serialize_entry("source_items", &sources)?;
        map.serialize_entry("editable_indexed", editable_indexed)?;
        map.serialize_entry("filter_stacks", filter_stacks)?;
        if !current_filter_output_precision.is_canonical() {
            map.serialize_entry("current_filter_output_precision", current_filter_output_precision)?;
        }
        map.serialize_entry("floating", floating)?;
        map.serialize_entry("selection", selection)?;
        map.serialize_entry("item_sets", item_sets)?;
        // Shared model ownership is not a wire reference: Arc<Vec<u8>> still
        // serializes as the original inline byte array. Reader floors/defaults
        // and authenticated opaque-record hash semantics remain unchanged.
        map.serialize_entry("opaque", opaque)?;
        map.end()
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 15) as usize] as char);
    }
    s
}

pub(crate) fn unhex(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return Err(FormatError::corrupt(format!("bad hex `{s}`")));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            let pair = s.get(i..i + 2).unwrap_or("x");
            // `from_str_radix` accepts a leading `+`; reject anything that is not
            // two ASCII hex digits so a malformed string fails as corrupt (#1818).
            if !pair.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(FormatError::corrupt(format!("bad hex `{s}`")));
            }
            u8::from_str_radix(pair, 16).map_err(|_| FormatError::corrupt(format!("bad hex `{s}`")))
        })
        .collect()
}

fn key4(s: &str) -> Result<[u8; 4]> {
    unhex(s)?.try_into().map_err(|_| FormatError::corrupt(format!("bad 4-byte key `{s}`")))
}

pub(crate) fn is_valid_hash(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Convert native-endian sample bytes to little-endian in place (and back:
/// the swap is an involution).
pub(crate) fn swap_to_le(bytes: &mut [u8], sample: SampleType) {
    if cfg!(target_endian = "big") {
        match sample {
            SampleType::U8 => {}
            SampleType::U16 => bytes.as_chunks_mut::<2>().0.iter_mut().for_each(|c| c.reverse()),
            SampleType::U32 | SampleType::F32 => bytes.as_chunks_mut::<4>().0.iter_mut().for_each(|c| c.reverse()),
            SampleType::F64 => bytes.as_chunks_mut::<8>().0.iter_mut().for_each(|c| c.reverse()),
        }
    }
}

pub(crate) fn tile_len(format: &PixelFormat) -> usize {
    (TILE_SIZE * TILE_SIZE) as usize * format.bytes_per_pixel()
}

// ---------------------------------------------------------------------------
// Save
// ---------------------------------------------------------------------------

/// Bounded JSON output. Capacity is charged before allocation, and all large
/// writes are subdivided, including metadata strings and opaque byte arrays.
pub(crate) fn json_bytes<T: serde::Serialize + ?Sized>(value: &T, sink: &mut dyn Sink<'_>, maximum: usize, pretty: bool) -> Result<Vec<u8>> {
    struct Output<'a, 'doc> {
        bytes: Vec<u8>,
        sink: &'a mut dyn Sink<'doc>,
        maximum: usize,
        failure: Option<FormatError>,
        remaining: usize,
    }
    impl std::io::Write for Output<'_, '_> {
        fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
            let result = (|| -> Result<()> {
                for chunk in input.chunks(8192) {
                    if self.remaining < chunk.len() {
                        self.sink.poll()?;
                        self.remaining = 8192;
                    }
                    let end = self.bytes.len().checked_add(chunk.len()).ok_or_else(|| FormatError::LimitExceeded("native JSON size overflow".into()))?;
                    if end > self.maximum {
                        return Err(FormatError::LimitExceeded("native preparation JSON bytes".into()));
                    }
                    if end > self.bytes.capacity() {
                        self.sink.poll()?;
                        let capacity = end.checked_add(65535).map(|v| v / 65536 * 65536).unwrap_or(self.maximum).min(self.maximum);
                        let additional = capacity.saturating_sub(self.bytes.capacity());
                        self.sink.reserve(additional)?;
                        self.bytes
                            .try_reserve_exact(capacity.saturating_sub(self.bytes.len()))
                            .map_err(|_| FormatError::LimitExceeded("native JSON allocation".into()))?;
                    }
                    self.bytes.extend_from_slice(chunk);
                    self.remaining = self.remaining.saturating_sub(chunk.len());
                }
                Ok(())
            })();
            if let Err(error) = result {
                self.failure = Some(error);
                return Err(std::io::Error::other("native JSON preparation interrupted"));
            }
            Ok(input.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    sink.poll()?;
    let mut output = Output { bytes: Vec::new(), sink, maximum, failure: None, remaining: 0 };
    let wrapped = interruptible_json::Value { value };
    let result = if pretty { serde_json::to_writer_pretty(&mut output, &wrapped) } else { serde_json::to_writer(&mut output, &wrapped) };
    if let Some(error) = output.failure {
        return Err(error);
    }
    output.sink.poll()?;
    result?;
    Ok(output.bytes)
}

fn string(value: &str, sink: &mut dyn Sink<'_>) -> Result<String> {
    sink.poll()?;
    sink.reserve(value.len())?;
    let mut result = String::new();
    result.try_reserve_exact(value.len()).map_err(|_| FormatError::LimitExceeded("native string allocation".into()))?;
    let mut start = 0;
    while start < value.len() {
        sink.poll()?;
        let mut end = start.saturating_add(8192).min(value.len());
        while !value.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        result.push_str(value.get(start..end).ok_or_else(|| FormatError::Unsupported("native UTF-8 slice".into()))?);
        start = end;
    }
    Ok(result)
}

fn optional_string(value: &Option<String>, sink: &mut dyn Sink<'_>) -> Result<Option<String>> {
    value.as_ref().map(|v| string(v, sink)).transpose()
}

/// Remaining complex Clone elements are admitted only after bounded measurement.
/// Their original values are cloned, preserving float bits and private fields;
/// JSON is never decoded back into authoritative metadata.
fn metadata<T: Clone + serde::Serialize>(value: &T, sink: &mut dyn Sink<'_>) -> Result<T> {
    let bytes = json_bytes(value, sink, 64 << 10, false)?;
    let estimate = bytes
        .len()
        .checked_mul(64)
        .and_then(|v| v.checked_add(std::mem::size_of::<T>()))
        .ok_or_else(|| FormatError::LimitExceeded("native metadata accounting overflow".into()))?;
    sink.reserve(estimate)?;
    sink.poll()?;
    let result = value.clone();
    sink.poll()?;
    sink.release(bytes.capacity());
    Ok(result)
}

fn mapped<'doc, T, U>(values: &[T], sink: &mut dyn Sink<'doc>, mut convert: impl FnMut(&T, &mut dyn Sink<'doc>) -> Result<U>) -> Result<Vec<U>> {
    sink.poll()?;
    sink.reserve(values.len().checked_mul(std::mem::size_of::<U>()).ok_or_else(|| FormatError::LimitExceeded("native vector accounting overflow".into()))?)?;
    let mut result = Vec::new();
    result.try_reserve_exact(values.len()).map_err(|_| FormatError::LimitExceeded("native metadata vector allocation".into()))?;
    for value in values {
        sink.poll()?;
        result.push(convert(value, sink)?);
    }
    Ok(result)
}

fn metadata_vec<T: Clone + serde::Serialize>(values: &[T], sink: &mut dyn Sink<'_>) -> Result<Vec<T>> {
    mapped(values, sink, |value, sink| metadata(value, sink))
}

fn adjustment(value: &photocraft_doc::Adjustment, sink: &mut dyn Sink<'_>) -> Result<photocraft_doc::Adjustment> {
    if let photocraft_doc::Adjustment::ColorLookup { name, lut, size, tetrahedral, dither } = value {
        // LUT samples are already owned immutable Arc storage. Manifest staging
        // shares that allocation; measuring it as deep-cloned JSON both rejects
        // ordinary tables at 64KiB and charges storage that is never copied.
        sink.reserve(std::mem::size_of::<photocraft_doc::Adjustment>())?;
        let result =
            photocraft_doc::Adjustment::ColorLookup { name: string(name, sink)?, lut: lut.clone(), size: *size, tetrahedral: *tetrahedral, dither: *dither };
        sink.poll()?;
        Ok(result)
    } else {
        metadata(value, sink)
    }
}

fn surface_m(s: &Surface, sink: &mut dyn Sink<'_>) -> Result<SurfaceM> {
    let f = s.format();
    sink.poll()?;
    sink.reserve(s.default_bytes().len().checked_mul(3).ok_or_else(|| FormatError::LimitExceeded("native default bytes overflow".into()))?)?;
    let mut dp = Vec::new();
    dp.try_reserve_exact(s.default_bytes().len()).map_err(|_| FormatError::LimitExceeded("native default allocation".into()))?;
    dp.extend_from_slice(s.default_bytes());
    swap_to_le(&mut dp, f.sample);
    sink.poll()?;
    s.try_validate_tile_origins_with(&sink.control())
        .map_err(|e| if sink.control().cancelled() { FormatError::Cancelled } else { FormatError::Unsupported(e.to_string()) })?;
    sink.reserve(s.tile_count().checked_mul(std::mem::size_of::<TileRef>()).ok_or_else(|| FormatError::LimitExceeded("native tile references".into()))?)?;
    let mut tiles = Vec::new();
    tiles.try_reserve_exact(s.tile_count()).map_err(|_| FormatError::LimitExceeded("native tile reference allocation".into()))?;
    for (c, t) in s.tiles() {
        sink.poll()?;
        tiles.push(TileRef { tx: c.tx, ty: c.ty, hash: sink.tile(f, t)? });
    }
    Ok(SurfaceM { format: f, default: hex(&dp), tiles })
}

fn opt_blob(b: &Option<Arc<Vec<u8>>>, sink: &mut dyn Sink<'_>) -> Result<Option<Hash>> {
    b.as_ref().map(|b| sink.blob(b)).transpose()
}

fn video_m(v: &photocraft_doc::VideoData, sink: &mut dyn Sink<'_>) -> Result<crate::manifest::VideoDataM> {
    Ok(crate::manifest::VideoDataM {
        frames: mapped(&v.frames, sink, surface_m)?,
        original_frames: v.original_frames.as_ref().map(|frames| mapped(frames, sink, surface_m)).transpose()?,
        source: metadata(&v.source, sink)?,
        fps: v.fps,
        show_altered: v.show_altered,
    })
}

fn layer_m(l: &Layer, sink: &mut dyn Sink<'_>) -> Result<LayerM> {
    sink.poll()?;
    let content = match &l.content {
        LayerContent::Raster(s) => ContentM::Raster { surface: surface_m(s, sink)? },
        LayerContent::BackedRaster(backed) => ContentM::FiniteRaster { raster: sink.finite_raster(backed)? },
        LayerContent::Group(g) => {
            ContentM::Group { children: mapped(&g.children, sink, layer_m)?, expanded: g.expanded, artboard: metadata(&g.artboard, sink)? }
        }
        LayerContent::Adjustment(a) => ContentM::Adjustment { adjustment: adjustment(a, sink)? },
        LayerContent::Fill(f) => ContentM::Fill { fill: metadata(f, sink)? },
        LayerContent::Text(t) => ContentM::Text {
            text: string(&t.text, sink)?,
            font_family: string(&t.font_family, sink)?,
            size_pt: t.size_pt,
            color: t.color,
            transform: t.transform,
            cache: t.cache.as_ref().map(|s| surface_m(s, sink)).transpose()?,
            psd_raw: opt_blob(&t.psd_raw, sink)?,
            runs: metadata_vec(&t.runs, sink)?,
            paragraphs: metadata_vec(&t.paragraphs, sink)?,
            shape: metadata(&t.shape, sink)?,
            orientation: t.orientation,
            antialias: t.antialias,
            warp: metadata(&t.warp, sink)?,
        },
        LayerContent::Shape(s) => ContentM::Shape {
            fill: metadata(&s.fill, sink)?,
            cache: s.cache.as_ref().map(|c| surface_m(c, sink)).transpose()?,
            psd_raw: opt_blob(&s.psd_raw, sink)?,
            path: metadata(&s.path, sink)?,
            stroke: metadata(&s.stroke, sink)?,
            live: metadata(&s.live, sink)?,
        },
        LayerContent::Smart(s) => ContentM::Smart {
            contents_id: Some(s.contents_id.0),
            source: match &s.source {
                SmartSource::Embedded { file_name, bytes } => SmartSourceM::Embedded { file_name: string(file_name, sink)?, blob: sink.blob(bytes)? },
                SmartSource::Linked { path } => SmartSourceM::Linked { path: metadata(path, sink)? },
            },
            transform: s.transform,
            smart_filters: metadata(&s.smart_filters, sink)?,
            cache: s.cache.as_ref().map(|c| surface_m(c, sink)).transpose()?,
            psd_raw: opt_blob(&s.psd_raw, sink)?,
            filters_enabled: s.filters_enabled,
            filter_mask: s
                .filter_mask
                .as_ref()
                .map(|m| -> Result<_> {
                    Ok(MaskM { surface: surface_m(&m.surface, sink)?, enabled: m.enabled, linked: m.linked, density: m.density, feather: m.feather })
                })
                .transpose()?,
            warp: metadata(&s.warp, sink)?,
            stack_mode: s.stack_mode,
            perspective: s.perspective,
        },
    };
    Ok(LayerM {
        indexed_source: l
            .indexed_source
            .as_ref()
            .map(|i| -> Result<_> {
                Ok(IndexedSourceM {
                    indices: sink.blob(i.indices())?,
                    alpha: opt_blob(i.alpha(), sink)?,
                    palette: mapped(i.palette(), sink, |color, _| Ok(*color))?,
                    bounds: i.bounds(),
                    fingerprint: hex(&i.expanded_fingerprint()),
                })
            })
            .transpose()?,
        import_compositing: l.import_compositing,
        id: l.id.0,
        name: string(&l.name, sink)?,
        visible: l.visible,
        locks: l.locks,
        blend: l.blend,
        opacity: l.opacity,
        fill_opacity: l.fill_opacity,
        clipped: l.clipped,
        mask: l
            .mask
            .as_ref()
            .map(|m| -> Result<_> {
                Ok(MaskM { surface: surface_m(&m.surface, sink)?, enabled: m.enabled, linked: m.linked, density: m.density, feather: m.feather })
            })
            .transpose()?,
        vector_mask: metadata(&l.vector_mask, sink)?,
        effects: EffectsM {
            enabled: l.effects.enabled,
            items: metadata_vec(&l.effects.items, sink)?,
            psd_raw: opt_blob(&l.effects.psd_raw, sink)?,
            reference: l.effects.reference,
        },
        label: l.label,
        content,
        psd_blocks: mapped(&l.psd_blocks, sink, |(k, d), sink| Ok((hex(k), sink.blob(d)?)))?,
        psd_id: l.psd_id,
        fill_cache: l
            .fill_cache
            .as_ref()
            .map(|fc| -> Result<_> { Ok(FillCacheM { fill: metadata(&fc.fill, sink)?, surface: surface_m(&fc.surface, sink)? }) })
            .transpose()?,
        link_group: l.link_group,
        excluded_channels: l.excluded_channels,
        blend_if: metadata(&l.blend_if, sink)?,
        advanced: l.advanced,
        video: l.video.as_ref().map(|v| video_m(v, sink)).transpose()?,
    })
}

fn channel_m(c: &AlphaChannel, sink: &mut dyn Sink<'_>) -> Result<ChannelM> {
    // Destructured so a new `AlphaChannel` field fails to compile here until it is saved.
    let AlphaChannel { id, name, surface, spot, color, opacity, indicates } = c;
    Ok(ChannelM {
        id: Some(*id),
        name: string(name, sink)?,
        surface: surface_m(surface, sink)?,
        spot: *spot,
        color: *color,
        opacity: *opacity,
        indicates: *indicates,
    })
}

pub(crate) fn doc_m<'doc>(d: &'doc Document, sink: &mut dyn Sink<'doc>) -> Result<DocM> {
    sink.poll()?;
    if !d.resolution_dpi.is_finite() {
        return Err(FormatError::NonFinite { path: "document.resolution_dpi".into() });
    }
    d.validate_xcf_state_with(&sink.control())
        .map_err(|e| if sink.control().cancelled() { FormatError::Cancelled } else { FormatError::Unsupported(e.to_string()) })?;
    let xcf_state = d
        .xcf_state
        .as_ref()
        .map(|state| -> Result<_> {
            let bytes = json_bytes(&StoredXcfState(state), sink, 256 << 20, false)?;
            if bytes.len() > 256 << 20 {
                return Err(FormatError::LimitExceeded("XCF editable state".into()));
            }
            sink.blob(&Arc::new(bytes))
        })
        .transpose()?;
    Ok(DocM {
        physical_resolution: metadata(&d.physical_resolution, sink)?,
        working_profile_interpretation: metadata(&d.working_profile_interpretation, sink)?,
        xcf_source_caches: {
            let mut caches = Vec::new();
            if let Some(state) = &d.xcf_state {
                sink.reserve(
                    state
                        .source_items
                        .len()
                        .checked_mul(std::mem::size_of::<SourceCacheM>())
                        .ok_or_else(|| FormatError::LimitExceeded("native cache references".into()))?,
                )?;
                caches.try_reserve_exact(state.source_items.len()).map_err(|_| FormatError::LimitExceeded("native cache reference allocation".into()))?;
                for source in &state.source_items {
                    sink.poll()?;
                    if let Some(cache) = &source.cache {
                        caches.push(SourceCacheM {
                            owner: source.owner,
                            blob: sink.borrowed_blob(&cache.bytes)?,
                            byte_len: cache.bytes.len(),
                            width: cache.width,
                            height: cache.height,
                            format: cache.format,
                        });
                    }
                }
            }
            caches
        },
        xcf_filter_masks: if let Some(state) = &d.xcf_state {
            mapped(&state.filter_masks, sink, |mask, sink| {
                Ok(FilterMaskM { id: mask.id, owner: mask.owner, bounds: mask.bounds, surface: surface_m(&mask.surface, sink)? })
            })?
        } else {
            Vec::new()
        },
        xcf_state,
        sample_encoding: d.sample_encoding,
        source_color_context: d
            .source_color_context
            .as_ref()
            .map(|c| -> Result<_> { Ok(SourceColorContextM { original_icc: opt_blob(&c.original_icc, sink)?, source_precision: c.source_precision }) })
            .transpose()?,
        id: d.id.0,
        name: string(&d.name, sink)?,
        size: d.size,
        resolution_dpi: d.resolution_dpi,
        mode: d.mode,
        depth: d.depth,
        icc_profile: opt_blob(&d.icc_profile, sink)?,
        layers: mapped(&d.layers, sink, layer_m)?,
        channels: mapped(&d.channels, sink, channel_m)?,
        guides: metadata(&d.guides, sink)?,
        selection: d.selection.as_ref().map(|s| surface_m(s, sink)).transpose()?,
        selection_channel_id: d.selection_channel_id,
        ruler_origin: d.ruler_origin,
        metadata: MetadataM {
            xmp: optional_string(&d.metadata.xmp, sink)?,
            exif: opt_blob(&d.metadata.exif, sink)?,
            text: metadata_vec(&d.metadata.text, sink)?,
            psd_resources: mapped(&d.metadata.psd_resources, sink, |(id, n, b), sink| Ok((*id, string(n, sink)?, sink.blob(b)?)))?,
            psd_global_blocks: mapped(&d.metadata.psd_global_blocks, sink, |(s, k, b), sink| Ok((hex(s), hex(k), sink.blob(b)?)))?,
        },
        global_light: d.global_light,
        paths: mapped(&d.paths, sink, |p, sink| {
            Ok(NamedPathM { id: Some(p.id), name: string(&p.name, sink)?, path: metadata(&p.path, sink)?, psd_raw: opt_blob(&p.psd_raw, sink)? })
        })?,
        work_path: metadata(&d.work_path, sink)?,
        clipping_path: metadata(&d.clipping_path, sink)?,
        quick_mask: d.quick_mask.as_ref().map(|c| channel_m(c, sink)).transpose()?,
        quick_mask_unselected: d.quick_mask_unselected,
        patterns: mapped(&d.patterns, sink, |p, sink| {
            Ok(PatternM { id: string(&p.id, sink)?, name: string(&p.name, sink)?, width: p.width, height: p.height, surface: surface_m(&p.surface, sink)? })
        })?,
        color_table: metadata(&d.color_table, sink)?,
        duotone: metadata(&d.duotone, sink)?,
        variables: metadata(&d.variables, sink)?,
        timeline: metadata(&d.timeline, sink)?,
        file_encoding: d.file_encoding,
        layer_comps: mapped(&d.layer_comps, sink, comp_m)?,
        last_applied_comp: d.last_applied_comp,
        last_document_state: d.last_document_state.as_ref().map(|c| comp_m(c, sink)).transpose()?,
        measurement: metadata(&d.measurement, sink)?,
        notes: metadata_vec(&d.notes, sink)?,
        text_styles: metadata(&d.text_styles, sink)?,
        slices: metadata(&d.slices, sink)?,
    })
}

fn effects_m(e: &Effects, sink: &mut dyn Sink<'_>) -> Result<EffectsM> {
    Ok(EffectsM { enabled: e.enabled, items: metadata_vec(&e.items, sink)?, psd_raw: opt_blob(&e.psd_raw, sink)?, reference: e.reference })
}

fn comp_m(c: &LayerComp, sink: &mut dyn Sink<'_>) -> Result<LayerCompM> {
    let LayerComp { id, name, comment, apply_visibility, apply_position, apply_appearance, states } = c;
    Ok(LayerCompM {
        id: *id,
        name: string(name, sink)?,
        comment: string(comment, sink)?,
        apply_visibility: *apply_visibility,
        apply_position: *apply_position,
        apply_appearance: *apply_appearance,
        states: mapped(states, sink, |s, sink| {
            let CompLayerState { layer, visible, position, appearance } = s;
            Ok(CompStateM {
                layer: layer.0,
                visible: *visible,
                position: *position,
                appearance: appearance
                    .as_ref()
                    .map(|a| -> Result<_> {
                        let CompAppearance { blend, opacity, fill_opacity, effects } = a;
                        Ok(CompAppearanceM { blend: *blend, opacity: *opacity, fill_opacity: *fill_opacity, effects: effects_m(effects, sink)? })
                    })
                    .transpose()?,
            })
        })?,
    })
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Load
// ---------------------------------------------------------------------------

pub(crate) struct Loader<'a> {
    pub fetch: &'a mut dyn Fetch,
    pub preserve_ids: bool,
    pub max_id: u64,
    /// Stored layer id â†’ loaded id (differs when ids are remapped), for layer comp states.
    pub id_map: std::collections::HashMap<u64, LayerId>,
    pub channel_map: std::collections::HashMap<photocraft_doc::item_identity::ChannelId, photocraft_doc::item_identity::ChannelId>,
    pub path_map: std::collections::HashMap<photocraft_doc::item_identity::PathId, photocraft_doc::item_identity::PathId>,
    pub contents_id_map: std::collections::HashMap<u64, SmartContentsId>,
    pub legacy_linked_contents: std::collections::HashMap<String, SmartContentsId>,
    /// Decoded tiles by content hash and sample type. Saves store a tile once however often it
    /// is referenced (a solid fill shares one tile), so a load decodes and charges the byte
    /// budget once per distinct tile and hands every reference the same copy-on-write `Arc`
    /// (#3315). Grouped by sample type: in-memory byte order depends on it.
    pub tiles: std::collections::HashMap<SampleType, std::collections::HashMap<Hash, Arc<Tile>>>,
}

impl Loader<'_> {
    fn model_clone<T: Clone + serde::Serialize>(&mut self, value: &T) -> Result<T> {
        crate::model_accounting::cloned(value, self.fetch)
    }
    fn model_vec<T>(&mut self, count: usize) -> Result<Vec<T>> {
        crate::model_accounting::vector(count, self.fetch)
    }

    fn video(&mut self, m: &crate::manifest::VideoDataM) -> Result<photocraft_doc::VideoData> {
        let mut frames = self.model_vec(m.frames.len())?;
        for frame in &m.frames {
            frames.push(self.surface(frame)?);
        }
        if m.original_frames.as_ref().is_some_and(|o| o.len() != frames.len()) {
            return Err(FormatError::corrupt("video original frame count differs from edited frame count"));
        }
        let original_frames = m
            .original_frames
            .as_ref()
            .map(|source| {
                let mut original = self.model_vec(source.len())?;
                for frame in source {
                    original.push(self.surface(frame)?);
                }
                Ok::<_, FormatError>(original)
            })
            .transpose()?;
        Ok(photocraft_doc::VideoData { frames, original_frames, source: self.model_clone(&m.source)?, fps: m.fps, show_altered: m.show_altered })
    }

    fn surface(&mut self, m: &SurfaceM) -> Result<Surface> {
        let f = m.format;
        let mut dp = unhex(&m.default)?;
        if dp.len() != f.bytes_per_pixel() {
            return Err(FormatError::corrupt("default pixel has wrong size for its format"));
        }
        swap_to_le(&mut dp, f.sample);
        let mut s = Surface::try_with_default_bytes(f, &dp).map_err(|e| FormatError::corrupt(e.to_string()))?;
        let len = tile_len(&f);
        // Both edges of a tile's rectangle must fit in i32 (`TileCoord::rect` multiplies by the
        // tile size): -8388608 ..= 8388606. A damaged manifest can name any index (#938).
        let fits = |i: i32| i.checked_mul(TILE_SIZE).is_some() && i.checked_add(1).and_then(|e| e.checked_mul(TILE_SIZE)).is_some();
        for t in &m.tiles {
            self.fetch.model_poll(1)?;
            if !fits(t.tx) || !fits(t.ty) {
                return Err(FormatError::corrupt(format!("tile ({}, {}) is outside the coordinate range", t.tx, t.ty)));
            }
            let c = TileCoord::new(t.tx, t.ty);
            if s.tile(c).is_some() {
                return Err(FormatError::corrupt(format!("duplicate tile ({}, {})", t.tx, t.ty)));
            }
            if let Some(tile) = self.tiles.get(&f.sample).and_then(|m| m.get(&t.hash)) {
                // One hash names one byte string, so a cached tile of another length means
                // the same hash was referenced from formats of different pixel sizes.
                if tile.bytes().len() != len {
                    return Err(FormatError::corrupt(format!("tile {} is referenced with different sizes", t.hash)));
                }
                s.put_tiles([(c, tile.clone())]);
                continue;
            }
            let mut bytes = self.fetch.tile(&t.hash, len)?;
            swap_to_le(&mut bytes, f.sample);
            self.fetch.model_reserve(1024)?;
            s.try_tile_mut(c).map_err(|e| FormatError::LimitExceeded(e.to_string()))?.bytes_mut().copy_from_slice(&bytes);
            if let Some(tile) = s.tile(c) {
                self.tiles.entry(f.sample).or_default().insert(t.hash.clone(), tile.clone());
            }
        }
        Ok(s)
    }

    fn mask_surface(&mut self, m: &SurfaceM) -> Result<Arc<Surface>> {
        // The Arc payload/control block is additional to accounted Surface storage.
        self.fetch.model_reserve((std::mem::size_of::<Surface>() + 128) as u64)?;
        let surface = self.surface(m)?;
        self.fetch.model_poll(1)?;
        Ok(Arc::new(surface))
    }

    fn opt_surface(&mut self, m: &Option<SurfaceM>) -> Result<Option<Surface>> {
        m.as_ref().map(|s| self.surface(s)).transpose()
    }

    fn opt_blob(&mut self, h: &Option<Hash>) -> Result<Option<Arc<Vec<u8>>>> {
        h.as_ref().map(|h| self.fetch.blob(h)).transpose()
    }

    fn id(&mut self, raw: u64) -> LayerId {
        let id = if self.preserve_ids { LayerId(raw) } else { LayerId::fresh() };
        self.id_map.insert(raw, id);
        id
    }

    fn contents_id(&mut self, raw: Option<u64>, source: &SmartSourceM) -> SmartContentsId {
        if let Some(raw) = raw {
            return *self.contents_id_map.entry(raw).or_insert_with(|| if self.preserve_ids { SmartContentsId(raw) } else { SmartContentsId::fresh() });
        }
        // Legacy embedded blobs may be deduplicated despite representing independent objects.
        // Only a nonempty linked path provides an existing shared-source identity.
        if let SmartSourceM::Linked { path } = source
            && !path.is_empty()
        {
            return *self.legacy_linked_contents.entry(path.clone()).or_insert_with(SmartContentsId::fresh);
        }
        SmartContentsId::fresh()
    }

    fn comp(&mut self, m: &LayerCompM) -> Result<LayerComp> {
        let mut states = self.model_vec(m.states.len())?;
        for s in &m.states {
            let appearance = match &s.appearance {
                Some(a) => Some(CompAppearance {
                    blend: a.blend,
                    opacity: a.opacity,
                    fill_opacity: a.fill_opacity,
                    effects: Effects {
                        enabled: a.effects.enabled,
                        items: self.model_clone(&a.effects.items)?,
                        psd_raw: self.opt_blob(&a.effects.psd_raw)?,
                        reference: a.effects.reference,
                    },
                }),
                None => None,
            };
            // States of layers deleted before the save keep their (now dangling) id, which
            // `layerComp.updateWarnings` reports; it must not collide with a live layer.
            let layer = self.id_map.get(&s.layer).copied().unwrap_or(LayerId(u64::MAX - s.layer.min(u64::MAX / 2)));
            states.push(CompLayerState { layer, visible: s.visible, position: s.position, appearance });
        }
        Ok(LayerComp {
            id: m.id,
            name: self.model_clone(&m.name)?,
            comment: self.model_clone(&m.comment)?,
            apply_visibility: m.apply_visibility,
            apply_position: m.apply_position,
            apply_appearance: m.apply_appearance,
            states,
        })
    }

    fn layer(&mut self, m: &LayerM, depth: usize) -> Result<Layer> {
        if depth > MAX_GROUP_DEPTH {
            return Err(too_deep());
        }
        // Children first, from this small frame: the conversion's large locals stay off the stack
        // that grows with each nesting level.
        let mut children = Vec::new();
        if let ContentM::Group { children: kids, .. } = &m.content {
            children = self.model_vec(kids.len())?;
            for c in kids {
                children.push(self.layer(c, depth + 1)?);
            }
        }
        self.layer_with(m, children)
    }

    /// One layer, given its already converted group children.
    #[inline(never)]
    fn layer_with(&mut self, m: &LayerM, children: Vec<Layer>) -> Result<Layer> {
        let content = match &m.content {
            ContentM::Raster { surface } => LayerContent::Raster(self.surface(surface)?),
            ContentM::FiniteRaster { raster } => LayerContent::BackedRaster(self.fetch.finite_raster(raster)?),
            ContentM::Group { expanded, artboard, .. } => LayerContent::Group(Group { artboard: self.model_clone(artboard)?, children, expanded: *expanded }),
            ContentM::Adjustment { adjustment } => LayerContent::Adjustment(self.model_clone(adjustment)?),
            ContentM::Fill { fill } => LayerContent::Fill(self.model_clone(fill)?),
            ContentM::Text { text, font_family, size_pt, color, transform, cache, psd_raw, runs, paragraphs, shape, orientation, antialias, warp } => {
                LayerContent::Text(TextLayer {
                    text: self.model_clone(text)?,
                    font_family: self.model_clone(font_family)?,
                    size_pt: *size_pt,
                    color: *color,
                    transform: *transform,
                    cache: self.opt_surface(cache)?,
                    psd_raw: self.opt_blob(psd_raw)?,
                    runs: self.model_clone(runs)?,
                    paragraphs: self.model_clone(paragraphs)?,
                    shape: self.model_clone(shape)?,
                    orientation: *orientation,
                    antialias: *antialias,
                    warp: self.model_clone(warp)?,
                })
            }
            ContentM::Shape { fill, cache, psd_raw, path, stroke, live } => LayerContent::Shape(ShapeLayer {
                path: self.model_clone(path)?,
                fill: self.model_clone(fill)?,
                stroke: self.model_clone(stroke)?,
                live: self.model_clone(live)?,
                cache: self.opt_surface(cache)?,
                psd_raw: self.opt_blob(psd_raw)?,
            }),
            ContentM::Smart { contents_id, source, transform, smart_filters, cache, psd_raw, filters_enabled, filter_mask, warp, stack_mode, perspective } => {
                LayerContent::Smart(SmartObject {
                    contents_id: self.contents_id(*contents_id, source),
                    source: match source {
                        SmartSourceM::Embedded { file_name, blob } => {
                            SmartSource::Embedded { file_name: self.model_clone(file_name)?, bytes: self.fetch.blob(blob)? }
                        }
                        SmartSourceM::Linked { path } => SmartSource::Linked { path: self.model_clone(path)? },
                    },
                    transform: *transform,
                    smart_filters: self.model_clone(smart_filters)?,
                    cache: self.opt_surface(cache)?,
                    psd_raw: self.opt_blob(psd_raw)?,
                    filters_enabled: *filters_enabled,
                    filter_mask: match filter_mask {
                        Some(mm) => Some(LayerMask {
                            surface: self.mask_surface(&mm.surface)?,
                            enabled: mm.enabled,
                            linked: mm.linked,
                            density: mm.density,
                            feather: mm.feather,
                        }),
                        None => None,
                    },
                    warp: self.model_clone(warp)?.filter(|w| w.mesh.as_ref().is_none_or(|m| m.is_valid())),
                    stack_mode: *stack_mode,
                    perspective: perspective.filter(|p| p.iter().all(|v| v.is_finite())),
                })
            }
        };
        let mask = match &m.mask {
            Some(mm) => {
                Some(LayerMask { surface: self.mask_surface(&mm.surface)?, enabled: mm.enabled, linked: mm.linked, density: mm.density, feather: mm.feather })
            }
            None => None,
        };
        let fill_cache = match &m.fill_cache {
            Some(fc) => Some(FillCache { fill: self.model_clone(&fc.fill)?, surface: self.surface(&fc.surface)? }),
            None => None,
        };
        let mut psd_blocks = self.model_vec(m.psd_blocks.len())?;
        for (k, h) in &m.psd_blocks {
            psd_blocks.push((key4(k)?, self.fetch.blob(h)?));
        }
        Ok(Layer {
            indexed_source: m
                .indexed_source
                .as_ref()
                .map(|i| {
                    let fingerprint: [u8; 32] = unhex(&i.fingerprint)?.try_into().map_err(|_| FormatError::corrupt("indexed fingerprint length"))?;
                    photocraft_doc::IndexedSource::from_parts(
                        self.fetch.blob(&i.indices)?,
                        self.opt_blob(&i.alpha)?,
                        self.model_clone(&i.palette)?,
                        i.bounds,
                        fingerprint,
                    )
                    .map_err(|_| FormatError::corrupt("invalid indexed-source planes, palette or bounds"))
                })
                .transpose()?,
            import_compositing: m.import_compositing,
            id: self.id(m.id),
            name: self.model_clone(&m.name)?,
            visible: m.visible,
            locks: m.locks,
            blend: m.blend,
            opacity: m.opacity,
            fill_opacity: m.fill_opacity,
            clipped: m.clipped,
            mask,
            vector_mask: self.model_clone(&m.vector_mask)?,
            effects: Effects {
                enabled: m.effects.enabled,
                items: self.model_clone(&m.effects.items)?,
                psd_raw: self.opt_blob(&m.effects.psd_raw)?,
                reference: m.effects.reference,
            },
            label: m.label,
            content,
            psd_blocks,
            psd_id: m.psd_id,
            fill_cache,
            link_group: m.link_group,
            excluded_channels: m.excluded_channels,
            blend_if: self.model_clone(&m.blend_if)?,
            advanced: m.advanced,
            video: m.video.as_ref().map(|v| self.video(v)).transpose()?,
        })
    }

    pub(crate) fn document_with_interrupt(&mut self, m: &DocM, control: &photocraft_raster::Interrupt<'_>) -> Result<Document> {
        // Reserve every stored id before allocating identities missing from older bundles. If an
        // id is implausibly large, remap the whole document while retaining shared contents groups.
        if self.preserve_ids {
            fn max_ids(layers: &[LayerM], depth: usize, max: &mut u64, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
                if depth > MAX_GROUP_DEPTH && !layers.is_empty() {
                    return Err(too_deep());
                }
                for l in layers {
                    control.check().map_err(|_| FormatError::Cancelled)?;
                    *max = (*max).max(l.id);
                    match &l.content {
                        ContentM::Group { children, .. } => max_ids(children, depth + 1, max, control)?,
                        ContentM::Smart { contents_id: Some(id), .. } => *max = (*max).max(*id),
                        _ => {}
                    }
                }
                Ok(())
            }
            let mut max = m.id;
            max_ids(&m.layers, 0, &mut max, control)?;
            for channel in m.channels.iter().chain(m.quick_mask.iter()) {
                control.check().map_err(|_| FormatError::Cancelled)?;
                if let Some(id) = channel.id {
                    max = max.max(id.0);
                }
            }
            for path in &m.paths {
                control.check().map_err(|_| FormatError::Cancelled)?;
                if let Some(id) = path.id {
                    max = max.max(id.0);
                }
            }
            self.max_id = self.max_id.max(max);
            if !reserve_ids_through(max) {
                self.preserve_ids = false;
            }
        }
        if m.channels.len() > 10000 {
            return Err(FormatError::LimitExceeded("channels".into()));
        }
        let mut layers = self.model_vec(m.layers.len())?;
        for layer in &m.layers {
            layers.push(self.layer(layer, 0)?);
        }
        let mut channels = self.model_vec(m.channels.len())?;
        for c in &m.channels {
            channels.push(self.channel(c)?);
        }
        let quick_mask = m.quick_mask.as_ref().map(|c| self.channel(c)).transpose()?;
        let mut md = Metadata {
            xmp: self.model_clone(&m.metadata.xmp)?,
            exif: self.opt_blob(&m.metadata.exif)?,
            text: self.model_clone(&m.metadata.text)?,
            psd_resources: self.model_vec(m.metadata.psd_resources.len())?,
            psd_global_blocks: self.model_vec(m.metadata.psd_global_blocks.len())?,
        };
        for (id, n, h) in &m.metadata.psd_resources {
            md.psd_resources.push((*id, self.model_clone(n)?, self.fetch.blob(h)?));
        }
        for (s, k, h) in &m.metadata.psd_global_blocks {
            md.psd_global_blocks.push((key4(s)?, key4(k)?, self.fetch.blob(h)?));
        }
        let mut paths = self.model_vec(m.paths.len())?;
        for p in &m.paths {
            let id = self.path_id(p.id)?;
            paths.push(NamedPath { id, name: self.model_clone(&p.name)?, path: self.model_clone(&p.path)?, psd_raw: self.opt_blob(&p.psd_raw)? });
        }
        let mut patterns = self.model_vec(m.patterns.len())?;
        for p in &m.patterns {
            // Sizes come from the file: a pattern with no tiles can claim any size, and renderers
            // allocate per pixel.
            let max = photocraft_doc::pattern::MAX_EDGE;
            if p.width == 0 || p.height == 0 {
                return Err(FormatError::corrupt(format!("pattern `{}` is {}×{} px", p.name, p.width, p.height)));
            }
            if p.width > max || p.height > max {
                return Err(FormatError::LimitExceeded(format!("pattern `{}` is {}×{} px (max {max} per side)", p.name, p.width, p.height)));
            }
            patterns.push(Pattern {
                id: self.model_clone(&p.id)?,
                name: self.model_clone(&p.name)?,
                width: p.width,
                height: p.height,
                surface: self.surface(&p.surface)?,
            });
        }
        let mut layer_comps = self.model_vec(m.layer_comps.len())?;
        for comp in &m.layer_comps {
            layer_comps.push(self.comp(comp)?);
        }
        let last_document_state = m.last_document_state.as_ref().map(|c| self.comp(c)).transpose()?;
        // Layer-based slices follow their layer through id remapping; a dangling one becomes a
        // user slice (its rect stays where it was).
        let mut slices = self.model_clone(&m.slices)?;
        for sl in &mut slices.list {
            if let Some(l) = sl.layer {
                sl.layer = self.id_map.get(&l.0).copied();
                if sl.layer.is_none() && sl.origin == photocraft_doc::SliceOrigin::Layer {
                    sl.origin = photocraft_doc::SliceOrigin::User;
                }
            }
        }
        let id = if self.preserve_ids {
            self.max_id = self.max_id.max(m.id);
            DocId(m.id)
        } else {
            DocId::fresh()
        };
        let selection_channel_id = m.selection_channel_id.map(|id| self.channel_id(Some(id))).transpose()?;
        let mut xcf_state = m
            .xcf_state
            .as_ref()
            .map(|hash| -> Result<photocraft_doc::XcfState> {
                let bytes = self.fetch.blob_bounded_with(hash, 256 << 20, control)?;
                if bytes.len() > 256 << 20 {
                    return Err(FormatError::LimitExceeded("XCF editable state".into()));
                }
                self.fetch.model_reserve(
                    (bytes.len() as u64).checked_mul(64).ok_or_else(|| FormatError::LimitExceeded("owned editable-state model envelope".into()))?,
                )?;
                decode_xcf_state(&bytes, control)
            })
            .transpose()?;
        if m.xcf_filter_masks.len() > 10000 {
            return Err(FormatError::LimitExceeded("filter masks".into()));
        }
        if (!m.xcf_filter_masks.is_empty() || !m.xcf_source_caches.is_empty()) && xcf_state.is_none() {
            return Err(FormatError::corrupt("filter masks lack editable state"));
        }
        if let Some(state) = &mut xcf_state {
            if m.xcf_source_caches.len() > 10000 {
                return Err(FormatError::LimitExceeded("source caches".into()));
            }
            let mut owners = std::collections::BTreeSet::new();
            let mut total = 0usize;
            for source in &state.source_items {
                if control.cancelled() {
                    return Err(FormatError::Cancelled);
                }
                if let Some(cache) = &source.cache {
                    let len = cache.checked_byte_len().map_err(|e| FormatError::corrupt(e.to_string()))?;
                    total = total.checked_add(len).ok_or_else(|| FormatError::LimitExceeded("source cache bytes".into()))?;
                    if len != cache.bytes.len() || total > 256 << 20 {
                        return Err(FormatError::LimitExceeded("source cache bytes".into()));
                    }
                }
            }
            for cache in &m.xcf_source_caches {
                if control.cancelled() {
                    return Err(FormatError::Cancelled);
                }
                if !owners.insert(cache.owner) {
                    return Err(FormatError::corrupt("duplicate source cache owner"));
                }
                let len = photocraft_doc::xcf_items::SourceCache::checked_descriptor_len(cache.width, cache.height, cache.format)
                    .map_err(|e| FormatError::corrupt(e.to_string()))?;
                total = total.checked_add(len).ok_or_else(|| FormatError::LimitExceeded("source cache bytes".into()))?;
                if len != cache.byte_len || total > 256 << 20 {
                    return Err(FormatError::LimitExceeded("source cache bytes".into()));
                }
                let source = state
                    .source_items
                    .iter_mut()
                    .find(|source| source.owner == cache.owner)
                    .ok_or_else(|| FormatError::corrupt("missing source cache owner"))?;
                if source.cache.is_some() {
                    return Err(FormatError::corrupt("source cache has both inline and blob payloads"));
                }
            }
            // Keep all legacy inline payloads out of transactional identity remapping.
            let mut inline_caches = self.model_vec(state.source_items.len())?;
            for (index, source) in state.source_items.iter_mut().enumerate() {
                if control.cancelled() {
                    return Err(FormatError::Cancelled);
                }
                if let Some(cache) = source.cache.take() {
                    inline_caches.push((index, cache));
                }
            }
            for mask in &m.xcf_filter_masks {
                state.filter_masks.push(
                    photocraft_doc::FilterMask::try_new(mask.id, mask.owner, self.surface(&mask.surface)?, mask.bounds)
                        .map_err(|e| FormatError::corrupt(e.to_string()))?,
                );
            }
            if control.cancelled() {
                return Err(FormatError::Cancelled);
            }
            state.remap_layers(|id| self.id_map.get(&id.0).copied()).map_err(|e| FormatError::corrupt(e.to_string()))?;
            if control.cancelled() {
                return Err(FormatError::Cancelled);
            }
            state.remap_channels(|id| self.channel_map.get(&id).copied()).map_err(|e| FormatError::corrupt(e.to_string()))?;
            if control.cancelled() {
                return Err(FormatError::Cancelled);
            }
            state.remap_paths(|id| self.path_map.get(&id).copied()).map_err(|e| FormatError::corrupt(e.to_string()))?;
            for (index, cache) in inline_caches {
                if control.cancelled() {
                    return Err(FormatError::Cancelled);
                }
                state.source_items.get_mut(index).ok_or_else(|| FormatError::corrupt("source order changed during remapping"))?.cache = Some(cache);
            }
            // Every descriptor/owner and the combined inline/blob budget was qualified above.
            for cache in &m.xcf_source_caches {
                if control.cancelled() {
                    return Err(FormatError::Cancelled);
                }
                let photocraft_doc::item_identity::ItemRef::Layer(raw) = cache.owner else {
                    return Err(FormatError::corrupt("invalid source cache owner"));
                };
                let owner =
                    photocraft_doc::item_identity::ItemRef::Layer(*self.id_map.get(&raw.0).ok_or_else(|| FormatError::corrupt("missing source cache layer"))?);
                let source = state
                    .source_items
                    .iter_mut()
                    .find(|source| source.owner == owner)
                    .ok_or_else(|| FormatError::corrupt("missing remapped source cache owner"))?;
                let bytes = self.fetch.source_cache_bytes(&cache.blob, cache.byte_len, control)?;
                if bytes.len() != cache.byte_len {
                    return Err(FormatError::corrupt("source cache blob length mismatch"));
                }
                source.cache = Some(photocraft_doc::xcf_items::SourceCache {
                    bytes,
                    blake3: self.model_clone(&cache.blob)?,
                    width: cache.width,
                    height: cache.height,
                    format: cache.format,
                });
            }
        }
        let document = Document {
            physical_resolution: self.model_clone(&m.physical_resolution)?,
            working_profile_interpretation: self.model_clone(&m.working_profile_interpretation)?,
            xcf_state,
            sample_encoding: m.sample_encoding,
            source_color_context: m
                .source_color_context
                .as_ref()
                .map(|c| -> Result<_> {
                    let original_icc = self.opt_blob(&c.original_icc)?;
                    if original_icc.as_ref().is_some_and(|p| p.len() > 16 << 20) {
                        return Err(FormatError::LimitExceeded("source ICC profile".into()));
                    }
                    Ok(photocraft_doc::SourceColorContext { original_icc, source_precision: c.source_precision })
                })
                .transpose()?,
            id,
            name: self.model_clone(&m.name)?,
            size: m.size,
            resolution_dpi: m.resolution_dpi,
            mode: m.mode,
            depth: m.depth,
            icc_profile: self.opt_blob(&m.icc_profile)?,
            layers,
            channels,
            guides: self.model_clone(&m.guides)?,
            ruler_origin: m.ruler_origin,
            selection: self.opt_surface(&m.selection)?,
            selection_channel_id,
            metadata: md,
            global_light: m.global_light,
            paths,
            work_path: self.model_clone(&m.work_path)?,
            clipping_path: self.model_clone(&m.clipping_path)?,
            quick_mask,
            quick_mask_unselected: m.quick_mask_unselected,
            patterns,
            color_table: self.model_clone(&m.color_table)?,
            duotone: self.model_clone(&m.duotone)?,
            variables: self.model_clone(&m.variables)?,
            timeline: self.model_clone(&m.timeline)?,
            file_encoding: m.file_encoding,
            layer_comps,
            last_applied_comp: m.last_applied_comp,
            last_document_state,
            measurement: self.model_clone(&m.measurement)?,
            notes: self.model_clone(&m.notes)?,
            text_styles: self.model_clone(&m.text_styles)?,
            slices,
        };
        document.validate_xcf_state_with(control).map_err(
            |e| {
                if e.code == "cancelled" { FormatError::Cancelled } else { FormatError::corrupt(e.to_string()) }
            },
        )?;
        Ok(document)
    }

    fn path_id(&mut self, raw: Option<photocraft_doc::item_identity::PathId>) -> Result<photocraft_doc::item_identity::PathId> {
        use photocraft_doc::item_identity::PathId;
        match raw {
            Some(raw) => {
                if raw.0 == 0 || self.path_map.contains_key(&raw) {
                    return Err(FormatError::corrupt("path identity"));
                }
                let id = if self.preserve_ids {
                    self.max_id = self.max_id.max(raw.0);
                    raw
                } else {
                    PathId::fresh()
                };
                self.path_map.insert(raw, id);
                Ok(id)
            }
            None => Ok(PathId::fresh()),
        }
    }
    fn channel_id(&mut self, raw: Option<photocraft_doc::item_identity::ChannelId>) -> Result<photocraft_doc::item_identity::ChannelId> {
        use photocraft_doc::item_identity::ChannelId;
        let id = match raw {
            Some(raw) => {
                if raw.0 == 0 || self.channel_map.contains_key(&raw) {
                    return Err(FormatError::corrupt("channel identity"));
                }
                let id = if self.preserve_ids {
                    self.max_id = self.max_id.max(raw.0);
                    raw
                } else {
                    ChannelId::fresh()
                };
                self.channel_map.insert(raw, id);
                id
            }
            None => ChannelId::fresh(),
        };
        Ok(id)
    }
    fn channel(&mut self, c: &ChannelM) -> Result<AlphaChannel> {
        let id = self.channel_id(c.id)?;
        Ok(AlphaChannel {
            id,
            name: self.model_clone(&c.name)?,
            surface: self.surface(&c.surface)?,
            spot: c.spot,
            color: c.color,
            opacity: c.opacity,
            indicates: c.indicates,
        })
    }
}

/// Advance the global id counter past `max` so ids loaded from a file never
/// collide with ids minted later in this process. Returns `false` if `max`
/// is implausibly far ahead (the caller then remaps ids instead).
pub(crate) fn reserve_ids_through(max: u64) -> bool {
    let probe = LayerId::fresh().0;
    if max < probe {
        return true;
    }
    if max - probe > 10_000_000 {
        return false;
    }
    photocraft_doc::ensure_ids_above(max);
    true
}

#[cfg(test)]
mod cache_cancellation_tests {
    use super::*;
    #[test]
    fn legacy_inline_state_json_checks_cancellation_mid_parse() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut state = photocraft_doc::XcfState::default();
        state.provenance.source_label = "own literal source label ".repeat(8192);
        let bytes = serde_json::to_vec(&state).unwrap();
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) > 3;
        assert!(matches!(decode_xcf_state(&bytes, &photocraft_raster::Interrupt::cancel_only(&cancel)), Err(FormatError::Cancelled)));
        assert_eq!(decode_xcf_state(&bytes, &photocraft_raster::Interrupt::NONE).unwrap(), state);
    }
    #[test]
    fn aliased_cache_copy_is_cancellable_and_unique_cache_moves_its_allocation() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let bytes = Arc::new(vec![42_u8; 1 << 20]);
        let pointer = bytes.as_ptr();
        let moved = checked_owned_bytes(bytes, &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(moved.as_ptr(), pointer);
        let bytes = Arc::new(moved);
        let retained = bytes.clone();
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) > 3;
        assert!(matches!(checked_owned_bytes(bytes, &photocraft_raster::Interrupt::cancel_only(&cancel)), Err(FormatError::Cancelled)));
        assert!(retained.iter().all(|value| *value == 42));
    }
}

// Public serde forwarding: retain serde_json formatting while bounded Display
// fragments prevent an entire long string being scanned between token checks.
pub(crate) mod interruptible_json {
    use serde::ser::*;
    use serde::{Serialize, Serializer};
    use std::fmt::{self, Write};
    pub(crate) struct Value<'a, T: ?Sized> {
        pub value: &'a T,
    }
    impl<T: Serialize + ?Sized> Serialize for Value<'_, T> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            self.value.serialize(Adapter { inner: serializer })
        }
    }
    struct Adapter<S> {
        inner: S,
    }
    struct Compound<C> {
        inner: C,
    }
    struct ChunkWriter<'a, 'b> {
        output: &'a mut fmt::Formatter<'b>,
    }
    impl Write for ChunkWriter<'_, '_> {
        fn write_str(&mut self, value: &str) -> fmt::Result {
            let mut start = 0;
            while start < value.len() {
                let mut end = start.saturating_add(8192).min(value.len());
                while !value.is_char_boundary(end) {
                    end = end.saturating_sub(1);
                }
                self.output.write_str(value.get(start..end).ok_or(fmt::Error)?)?;
                start = end;
            }
            Ok(())
        }
    }
    struct DisplayValue<'a, T: ?Sized> {
        value: &'a T,
    }
    impl<T: fmt::Display + ?Sized> fmt::Display for DisplayValue<'_, T> {
        fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(&mut ChunkWriter { output }, "{}", self.value)
        }
    }
    impl<S: Serializer> Serializer for Adapter<S> {
        type Ok = S::Ok;
        type Error = S::Error;
        type SerializeSeq = Compound<S::SerializeSeq>;
        type SerializeTuple = Compound<S::SerializeTuple>;
        type SerializeTupleStruct = Compound<S::SerializeTupleStruct>;
        type SerializeTupleVariant = Compound<S::SerializeTupleVariant>;
        type SerializeMap = Compound<S::SerializeMap>;
        type SerializeStruct = Compound<S::SerializeStruct>;
        type SerializeStructVariant = Compound<S::SerializeStructVariant>;
        fn serialize_bool(self, value: bool) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_bool(value)
        }
        fn serialize_i8(self, value: i8) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_i8(value)
        }
        fn serialize_i16(self, value: i16) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_i16(value)
        }
        fn serialize_i32(self, value: i32) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_i32(value)
        }
        fn serialize_i64(self, value: i64) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_i64(value)
        }
        fn serialize_i128(self, value: i128) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_i128(value)
        }
        fn serialize_u8(self, value: u8) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_u8(value)
        }
        fn serialize_u16(self, value: u16) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_u16(value)
        }
        fn serialize_u32(self, value: u32) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_u32(value)
        }
        fn serialize_u64(self, value: u64) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_u64(value)
        }
        fn serialize_u128(self, value: u128) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_u128(value)
        }
        fn serialize_f32(self, value: f32) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_f32(value)
        }
        fn serialize_f64(self, value: f64) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_f64(value)
        }
        fn serialize_char(self, value: char) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_char(value)
        }

        fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
            self.inner.collect_str(&DisplayValue { value })
        }
        fn serialize_bytes(self, value: &[u8]) -> Result<Self::Ok, Self::Error> {
            let mut sequence = self.serialize_seq(Some(value.len()))?;
            for byte in value {
                sequence.serialize_element(byte)?;
            }
            sequence.end()
        }
        fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_none()
        }
        fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_some(&Value { value })
        }
        fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_unit()
        }
        fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_unit_struct(name)
        }
        fn serialize_unit_variant(self, name: &'static str, index: u32, variant: &'static str) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_unit_variant(name, index, variant)
        }
        fn serialize_newtype_struct<T: Serialize + ?Sized>(self, name: &'static str, value: &T) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_newtype_struct(name, &Value { value })
        }
        fn serialize_newtype_variant<T: Serialize + ?Sized>(
            self,
            name: &'static str,
            index: u32,
            variant: &'static str,
            value: &T,
        ) -> Result<Self::Ok, Self::Error> {
            self.inner.serialize_newtype_variant(name, index, variant, &Value { value })
        }
        fn collect_str<T: fmt::Display + ?Sized>(self, value: &T) -> Result<Self::Ok, Self::Error> {
            self.inner.collect_str(&DisplayValue { value })
        }
        fn is_human_readable(&self) -> bool {
            self.inner.is_human_readable()
        }
        fn serialize_seq(self, length: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
            Ok(Compound { inner: self.inner.serialize_seq(length)? })
        }
        fn serialize_tuple(self, length: usize) -> Result<Self::SerializeTuple, Self::Error> {
            Ok(Compound { inner: self.inner.serialize_tuple(length)? })
        }
        fn serialize_tuple_struct(self, name: &'static str, length: usize) -> Result<Self::SerializeTupleStruct, Self::Error> {
            Ok(Compound { inner: self.inner.serialize_tuple_struct(name, length)? })
        }
        fn serialize_tuple_variant(
            self,
            name: &'static str,
            index: u32,
            variant: &'static str,
            length: usize,
        ) -> Result<Self::SerializeTupleVariant, Self::Error> {
            Ok(Compound { inner: self.inner.serialize_tuple_variant(name, index, variant, length)? })
        }
        fn serialize_map(self, length: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
            Ok(Compound { inner: self.inner.serialize_map(length)? })
        }
        fn serialize_struct(self, name: &'static str, length: usize) -> Result<Self::SerializeStruct, Self::Error> {
            Ok(Compound { inner: self.inner.serialize_struct(name, length)? })
        }
        fn serialize_struct_variant(
            self,
            name: &'static str,
            index: u32,
            variant: &'static str,
            length: usize,
        ) -> Result<Self::SerializeStructVariant, Self::Error> {
            Ok(Compound { inner: self.inner.serialize_struct_variant(name, index, variant, length)? })
        }
    }
    impl<C: SerializeSeq> SerializeSeq for Compound<C> {
        type Ok = C::Ok;
        type Error = C::Error;
        fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
            self.inner.serialize_element(&Value { value })
        }
        fn end(self) -> Result<Self::Ok, Self::Error> {
            self.inner.end()
        }
    }
    impl<C: SerializeTuple> SerializeTuple for Compound<C> {
        type Ok = C::Ok;
        type Error = C::Error;
        fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
            self.inner.serialize_element(&Value { value })
        }
        fn end(self) -> Result<Self::Ok, Self::Error> {
            self.inner.end()
        }
    }
    impl<C: SerializeTupleStruct> SerializeTupleStruct for Compound<C> {
        type Ok = C::Ok;
        type Error = C::Error;
        fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
            self.inner.serialize_field(&Value { value })
        }
        fn end(self) -> Result<Self::Ok, Self::Error> {
            self.inner.end()
        }
    }
    impl<C: SerializeTupleVariant> SerializeTupleVariant for Compound<C> {
        type Ok = C::Ok;
        type Error = C::Error;
        fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
            self.inner.serialize_field(&Value { value })
        }
        fn end(self) -> Result<Self::Ok, Self::Error> {
            self.inner.end()
        }
    }
    impl<C: SerializeMap> SerializeMap for Compound<C> {
        type Ok = C::Ok;
        type Error = C::Error;
        fn serialize_key<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
            self.inner.serialize_key(&Value { value })
        }
        fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
            self.inner.serialize_value(&Value { value })
        }
        fn end(self) -> Result<Self::Ok, Self::Error> {
            self.inner.end()
        }
    }
    impl<C: SerializeStruct> SerializeStruct for Compound<C> {
        type Ok = C::Ok;
        type Error = C::Error;
        fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error> {
            self.inner.serialize_field(key, &Value { value })
        }
        fn skip_field(&mut self, key: &'static str) -> Result<(), Self::Error> {
            self.inner.skip_field(key)
        }
        fn end(self) -> Result<Self::Ok, Self::Error> {
            self.inner.end()
        }
    }
    impl<C: SerializeStructVariant> SerializeStructVariant for Compound<C> {
        type Ok = C::Ok;
        type Error = C::Error;
        fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error> {
            self.inner.serialize_field(key, &Value { value })
        }
        fn skip_field(&mut self, key: &'static str) -> Result<(), Self::Error> {
            self.inner.skip_field(key)
        }
        fn end(self) -> Result<Self::Ok, Self::Error> {
            self.inner.end()
        }
    }
}

#[cfg(test)]
mod json_adapter_tests {
    use super::interruptible_json::Value;
    use serde::Serialize;
    #[derive(Serialize)]
    enum Variant {
        Unit,
        Newtype(String),
        Tuple(i32, String),
        Struct { text: String, bytes: Vec<u8> },
    }
    #[test]
    fn forwarding_preserves_compact_pretty_numbers_keys_and_all_enum_forms() {
        let variants = vec![
            Variant::Unit,
            Variant::Newtype("Own Ω\n\\\"".repeat(10000)),
            Variant::Tuple(-12, "Own😀\t".repeat(3000)),
            Variant::Struct { text: "\u{0001}\u{0008}\u{000c}\r\n".repeat(3000), bytes: vec![0, 127, 255] },
        ];
        let keys = [(7u64, "Own Ω".repeat(3000)), (u64::MAX, "\u{001f}".repeat(10000))].into_iter().collect::<std::collections::BTreeMap<_, _>>();
        let value = (variants, keys, (-0.0f64, f64::MIN_POSITIVE, f64::MAX, 1.2345678901234567));
        let wrapped = Value { value: &value };
        assert_eq!(serde_json::to_vec(&wrapped).unwrap(), serde_json::to_vec(&value).unwrap());
        assert_eq!(serde_json::to_vec_pretty(&wrapped).unwrap(), serde_json::to_vec_pretty(&value).unwrap());
    }
    #[test]
    fn long_unescaped_string_scan_is_split_before_formatter_callbacks() {
        use std::cell::Cell;
        use std::io::Write;
        struct Observed<'a>(&'a Cell<usize>);
        impl serde_json::ser::Formatter for Observed<'_> {
            fn write_string_fragment<W: ?Sized + Write>(&mut self, writer: &mut W, fragment: &str) -> std::io::Result<()> {
                self.0.set(self.0.get().max(fragment.len()));
                writer.write_all(fragment.as_bytes())
            }
        }
        let text = "Own Ω unescaped".repeat(100000);
        let maximum = Cell::new(0);
        let mut bytes = Vec::new();
        let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, Observed(&maximum));
        Value { value: &text }.serialize(&mut serializer).unwrap();
        assert!(maximum.get() <= 8192, "formatter received {} bytes without a bounded scan", maximum.get());
        assert_eq!(bytes, serde_json::to_vec(&text).unwrap());
        // Negative control: ordinary serde_json receives the entire unescaped run.
        let maximum = Cell::new(0);
        let mut bytes = Vec::new();
        text.serialize(&mut serde_json::Serializer::with_formatter(&mut bytes, Observed(&maximum))).unwrap();
        assert!(maximum.get() > 8192);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unhex_rejects_leading_plus() {
        // `u8::from_str_radix` accepts a leading `+`; unhex must not (#1818).
        assert!(unhex("+5").is_err());
        assert!(unhex("ab+5").is_err());
        assert!(unhex("+5cd").is_err());
    }

    #[test]
    fn unhex_rejects_non_hex_characters() {
        assert!(unhex("zz").is_err());
        assert!(unhex(" 1").is_err());
        assert!(unhex("g0").is_err());
    }

    #[test]
    fn brown_feature_scan_preserves_the_existing_empty_deepest_group_boundary() {
        let mut doc = Document::new("existing depth boundary", photocraft_geom::Size::new(1, 1), photocraft_color::ColorMode::Rgb, SampleType::U8);
        let mut layers = vec![Layer::group("empty deepest group", Vec::new())];
        for _ in 0..MAX_GROUP_DEPTH { layers = vec![Layer::group("ancestor", layers)]; }
        doc.layers = layers;
        check_nesting(&doc.layers, photocraft_raster::Interrupt::NONE).unwrap();
        assert!(!requires_reader_v8_state_with(&doc, photocraft_raster::Interrupt::NONE).unwrap());
        doc.layers[0].label = photocraft_doc::LabelColor::Brown;
        assert!(requires_reader_v8_state_with(&doc, photocraft_raster::Interrupt::NONE).unwrap());
        assert!(matches!(requires_reader_v8_state_with(&doc, photocraft_raster::Interrupt::cancel_only(&|| true)), Err(FormatError::Cancelled)));
    }

    #[test]
    fn unhex_accepts_valid_hex() {
        assert_eq!(unhex("00").unwrap(), vec![0x00]);
        assert_eq!(unhex("ff").unwrap(), vec![0xff]);
        assert_eq!(unhex("0123456789abcdef").unwrap(), vec![0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]);
        assert_eq!(unhex("FF").unwrap(), vec![0xff]);
    }

    #[test]
    fn unhex_rejects_odd_length() {
        assert!(unhex("0").is_err());
        assert!(unhex("abc").is_err());
    }
}
