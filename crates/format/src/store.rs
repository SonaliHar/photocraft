//! Object storage (ZIP or directory), incremental writer and loader.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::SystemTime;

use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::Document;
use photocraft_raster::{Rgba8Image, Tile};

use crate::convert::{self, Fetch, Loader, Sink, is_valid_hash, swap_to_le};
use crate::manifest::{ContentM, FORMAT_VERSION, Hash, LayerM, Manifest};
use crate::zip::ZipWriter;
use crate::{FormatError, LoadOptions, Result, SaveOptions};

#[path = "store/directory_stream.rs"]
pub mod directory_stream;

pub(crate) const MANIFEST: &str = "manifest.json";
pub(crate) const THUMB: &str = "thumb.png";
pub(crate) const COMPOSITE: &str = "composite/preview.png";

fn tile_path(h: &str) -> String {
    format!("tiles/{h}.zst")
}
fn blob_path(h: &str) -> String {
    format!("blobs/{h}.zst")
}

fn compress(data: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest)
}

fn decompress(data: &[u8], expected: usize, what: &str) -> Result<Vec<u8>> {
    decompress_with(data, expected, what, &photocraft_raster::Interrupt::NONE)
}
fn decompress_with(data: &[u8], expected: usize, what: &str, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<u8>> {
    if control.cancelled() {
        return Err(FormatError::Cancelled);
    }
    let mut dec = ruzstd::decoding::StreamingDecoder::new(data).map_err(|e| FormatError::corrupt(format!("{what}: zstd: {e}")))?;
    let out = crate::read::read_all_bounded(&mut dec, expected as u64, expected, control)
        .map_err(|error| if control.cancelled() { FormatError::Cancelled } else { FormatError::corrupt(format!("{what}: zstd: {error}")) })?;
    if out.len() != expected {
        return Err(FormatError::corrupt(format!("{what}: expected {expected} bytes, got {}", out.len())));
    }
    Ok(out)
}

fn hash_bytes(b: &[u8]) -> Hash {
    blake3::hash(b).to_hex().to_string()
}

fn png(img: &Rgba8Image) -> Result<Vec<u8>> {
    let image = photocraft_codecs::Image::from_u8(img.width, img.height, photocraft_codecs::ChannelLayout::Rgba, img.pixels.clone())
        .map_err(|e| FormatError::Unsupported(format!("preview: {e}")))?;
    photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).map_err(|e| FormatError::Unsupported(format!("preview: {e}")))
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

pub(crate) use ByteSource as Source;

pub trait ByteSource {
    fn get(&self, path: &str, max: usize) -> Result<Vec<u8>>;
    fn get_with_interrupt(&self, path: &str, max: usize, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<u8>> {
        if control.cancelled() {
            return Err(FormatError::Cancelled);
        }
        let bytes = self.get(path, max)?;
        if control.cancelled() {
            return Err(FormatError::Cancelled);
        }
        Ok(bytes)
    }
    fn model_control(&self) -> Option<&photocraft_raster::backing::ReadControl<'_>> {
        None
    }
}

pub(crate) struct ZipSource<'a> {
    zip: std::cell::RefCell<crate::zip::stream::Reader<std::io::Cursor<&'a [u8]>>>,
}

impl<'a> ZipSource<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        Ok(ZipSource {
            zip: std::cell::RefCell::new(crate::zip::stream::Reader::new(
                std::io::Cursor::new(bytes),
                Default::default(),
                &photocraft_raster::Interrupt::NONE,
            )?),
        })
    }
}

impl ByteSource for ZipSource<'_> {
    fn get(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        self.get_with_interrupt(path, max, &photocraft_raster::Interrupt::NONE)
    }
    fn get_with_interrupt(&self, path: &str, max: usize, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<u8>> {
        self.zip.try_borrow_mut().map_err(|_| FormatError::corrupt("recursive zip access"))?.read_by_name(path, max, control)
    }
}

pub(crate) struct StreamSource<'a, R> {
    zip: std::cell::RefCell<crate::zip::stream::Reader<R>>,
    control: photocraft_raster::Interrupt<'a>,
}
impl<'a, R: std::io::Read + std::io::Seek> StreamSource<'a, R> {
    pub(crate) fn new(input: R, limits: crate::zip::stream::Limits, control: photocraft_raster::Interrupt<'a>) -> Result<Self> {
        Ok(Self { zip: std::cell::RefCell::new(crate::zip::stream::Reader::new(input, limits, &control)?), control })
    }
}
impl<R: std::io::Read + std::io::Seek> Source for StreamSource<'_, R> {
    fn get(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        self.get_with_interrupt(path, max, &self.control)
    }
    fn get_with_interrupt(&self, path: &str, max: usize, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<u8>> {
        let own = self.control;
        let cancel = || own.cancelled() || control.cancelled();
        let progress = |fraction| {
            own.progress(fraction);
            control.progress(fraction);
        };
        let combined = photocraft_raster::Interrupt::new(&cancel, &progress);
        self.zip.try_borrow_mut().map_err(|_| FormatError::corrupt("recursive streaming zip access"))?.read_by_name(path, max, &combined)
    }
}

pub(crate) struct DirSource {
    pub root: PathBuf,
}

impl ByteSource for DirSource {
    fn get(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        self.get_with_interrupt(path, max, &photocraft_raster::Interrupt::NONE)
    }
    fn get_with_interrupt(&self, path: &str, max: usize, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<u8>> {
        if control.cancelled() {
            return Err(FormatError::Cancelled);
        }
        let mut file = std::fs::File::open(self.root.join(path))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(FormatError::corrupt(format!("`{path}` is not a regular file")));
        }
        let len = metadata.len();
        if len > max as u64 {
            return Err(FormatError::LimitExceeded(format!("`{path}` is {len} bytes (max {max})")));
        }
        crate::read::read_all_bounded(&mut file, len, max, control)
            .map_err(|error| if control.cancelled() { FormatError::Cancelled } else { FormatError::Io(error) })
    }
}

/// Read one opened bundle entry under its `max` budget. The stat `len` is a buffer hint only
/// (files can change while open, and a FIFO reports 0), so the bound is enforced on the read
/// itself, never on the hint.
pub fn read_entry(file: &mut impl std::io::Read, len: u64, max: usize, path: &str) -> Result<Vec<u8>> {
    let mut limited = file.take((max as u64).saturating_add(1));
    let hint = len.min((max as u64).saturating_add(1));
    let bytes = crate::read::read_all(&mut limited, hint)?;
    if bytes.len() > max {
        return Err(FormatError::LimitExceeded(format!("`{path}` exceeds {max} bytes")));
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Deepest JSON nesting a manifest may have: each group level adds three (the layer object, its
/// content object and the `children` array), with headroom for the document and leaf layer fields.
/// The saver checks the same bound, so every bundle it writes loads again.
const MAX_MANIFEST_DEPTH: usize = 3 * (convert::MAX_GROUP_DEPTH + 1) + 64;

/// Rejects JSON nested deeper than [`MAX_MANIFEST_DEPTH`] with a linear scan (no recursion), so
/// parsing with serde_json's recursion limit disabled can't exhaust the stack on hostile input.
fn check_manifest_depth(json: &[u8]) -> Result<()> {
    check_manifest_depth_with(json, photocraft_raster::Interrupt::NONE)
}
fn check_manifest_depth_with(json: &[u8], control: photocraft_raster::Interrupt<'_>) -> Result<()> {
    let (mut depth, mut in_str, mut escaped) = (0usize, false, false);
    for (index, &b) in json.iter().enumerate() {
        if index.is_multiple_of(8192) && control.cancelled() {
            return Err(FormatError::Cancelled);
        }
        if in_str {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > MAX_MANIFEST_DEPTH {
                    return Err(FormatError::LimitExceeded(format!("manifest JSON nested deeper than {MAX_MANIFEST_DEPTH} levels")));
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn read_manifest(src: &dyn Source, opts: &LoadOptions) -> Result<Manifest> {
    read_manifest_with_interrupt(src, opts, &photocraft_raster::Interrupt::NONE)
}
fn read_manifest_with_interrupt(src: &dyn Source, opts: &LoadOptions, interrupt: &photocraft_raster::Interrupt<'_>) -> Result<Manifest> {
    let raw = src.get_with_interrupt(MANIFEST, opts.max_manifest_bytes, interrupt)?;
    if interrupt.cancelled() {
        return Err(FormatError::Cancelled);
    }
    check_manifest_depth_with(&raw, *interrupt)?;
    // serde_json stops at 128 levels (about 40 nested groups); the bounded check replaces that limit.
    let mut v = if let Some(control) = src.model_control() {
        check_manifest_depth_with(&raw, *control.interrupt)?;
        struct Checked<'a, 'c> {
            raw: &'a [u8],
            position: usize,
            control: &'a photocraft_raster::backing::ReadControl<'c>,
            failure: Option<photocraft_raster::backing::BackingError>,
        }
        impl std::io::Read for Checked<'_, '_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let left = self.raw.len().checked_sub(self.position).ok_or_else(|| std::io::Error::other("manifest read position"))?;
                let count = left.min(out.len()).min(4096);
                if let Err(error) = self.control.check(count as u64) {
                    self.failure = Some(error);
                    return Err(std::io::Error::other(error));
                }
                let next = self.position.checked_add(count).ok_or_else(|| std::io::Error::other("manifest read position"))?;
                out.get_mut(..count)
                    .ok_or_else(|| std::io::Error::other("manifest read length"))?
                    .copy_from_slice(self.raw.get(self.position..next).ok_or_else(|| std::io::Error::other("manifest read length"))?);
                self.position = next;
                Ok(count)
            }
        }
        let mut checked = Checked { raw: &raw, position: 0, control, failure: None };
        let result = {
            let mut de = serde_json::Deserializer::from_reader(&mut checked);
            de.disable_recursion_limit();
            <serde_json::Value as serde::Deserialize>::deserialize(&mut de).and_then(|v| {
                de.end()?;
                Ok(v)
            })
        };
        if let Some(error) = checked.failure {
            return Err(crate::owned_document::error(error));
        }
        result?
    } else {
        check_manifest_depth(&raw)?;
        let mut de = serde_json::Deserializer::from_slice(&raw);
        de.disable_recursion_limit();
        let value = <serde_json::Value as serde::Deserialize>::deserialize(&mut de)?;
        de.end()?;
        value
    };
    if let Some(minimum) = v.get("minimum_reader_version") {
        let found = minimum.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| FormatError::corrupt("invalid minimum_reader_version"))?;
        if found == 0 {
            return Err(FormatError::corrupt("minimum_reader_version 0 is invalid"));
        }
        if found > FORMAT_VERSION {
            return Err(FormatError::TooNew { found, supported: FORMAT_VERSION });
        }
    }
    if let Some(hint) = v.get("current_storage_hint").filter(|hint| !hint.is_null()) {
        let floor = match hint.as_str(){Some("masked_filtered_flat_point")=>12,Some("point_f32_coverage")=>11,_=>10};
        if v.get("format_version").and_then(serde_json::Value::as_u64).is_none_or(|version| version < floor)
            || v.get("minimum_reader_version").and_then(serde_json::Value::as_u64).is_none_or(|version| version < floor)
        {
            return Err(FormatError::corrupt("current storage hint has invalid original reader/format floor"));
        }
    }
    crate::migrate::migrate(&mut v)?;
    let layers = take_array(&mut v, "/document/layers");
    let mut m: Manifest = serde_json::from_value(v)?;
    if m.minimum_reader_version == 0 {
        return Err(FormatError::corrupt("minimum_reader_version 0 is invalid"));
    }
    if m.minimum_reader_version > FORMAT_VERSION {
        return Err(FormatError::TooNew { found: m.minimum_reader_version, supported: FORMAT_VERSION });
    }
    let hint_floor = match m.current_storage_hint {
        Some(crate::NativeCurrentStorageHint::MaskedFilteredFlatPoint) => 12,
        Some(crate::NativeCurrentStorageHint::PointF32Coverage) => 11,
        Some(crate::NativeCurrentStorageHint::FilteredFlatPoint) => 10,
        None => 1,
    };
    if m.minimum_reader_version < hint_floor {
        return Err(FormatError::corrupt("current storage hint requires its minimum reader version"));
    }
    if (m.document.physical_resolution.is_some() || m.document.xcf_state.is_some() || !m.document.xcf_filter_masks.is_empty()) && m.minimum_reader_version < 2 {
        return Err(FormatError::corrupt("imported state requires minimum reader version 2"));
    }
    if (!m.document.xcf_source_caches.is_empty() || m.document.working_profile_interpretation != photocraft_doc::WorkingProfileInterpretation::Icc)
        && m.minimum_reader_version < 3
    {
        return Err(FormatError::corrupt("source cache blobs or working profile interpretation require minimum reader version 3"));
    }
    m.document.layers = layers_from_json(layers)?;
    if manifest_has_finite(&m.document.layers, 0, src.model_control())? && m.minimum_reader_version < 5 {
        return Err(FormatError::corrupt("finite raster requires minimum reader version 5"));
    }
    Ok(m)
}

fn manifest_has_finite(layers: &[LayerM], depth: usize, control: Option<&photocraft_raster::backing::ReadControl<'_>>) -> Result<bool> {
    if depth > convert::MAX_GROUP_DEPTH {
        return Err(FormatError::LimitExceeded("native layer nesting".into()));
    }
    for layer in layers {
        if let Some(control) = control {
            control.check(1).map_err(crate::owned_document::error)?;
        }
        match &layer.content {
            ContentM::FiniteRaster { .. } => return Ok(true),
            ContentM::Group { children, .. } if manifest_has_finite(children, depth + 1, control)? => return Ok(true),
            _ => {}
        }
    }
    Ok(false)
}
/// Decodes layers one at a time, group children first. Decoding the whole tree in one go costs
/// kilobytes of stack per nesting level; here each level only holds a few pointers.
fn layers_from_json(values: Vec<serde_json::Value>) -> Result<Vec<LayerM>> {
    let mut out = Vec::with_capacity(values.len());
    for mut v in values {
        let children = layers_from_json(take_array(&mut v, "/content/children"))?;
        out.push(layer_from_json(v, children)?);
    }
    Ok(out)
}

/// One layer without its children (kept out of line so its large locals stay off the recursion).
#[inline(never)]
fn layer_from_json(v: serde_json::Value, children: Vec<LayerM>) -> Result<LayerM> {
    let mut layer: LayerM = serde_json::from_value(v)?;
    if let ContentM::Group { children: c, .. } = &mut layer.content {
        *c = children;
    }
    Ok(layer)
}

/// Takes the array at JSON `pointer` out of `v`, leaving an empty one (nothing if absent).
fn take_array(v: &mut serde_json::Value, pointer: &str) -> Vec<serde_json::Value> {
    match v.pointer_mut(pointer) {
        Some(serde_json::Value::Array(a)) => std::mem::take(a),
        _ => Vec::new(),
    }
}

struct LoadFetch<'a, 'c> {
    src: &'a dyn Source,
    opts: LoadOptions,
    total: u64,
    retained_capacity: u64,
    control: photocraft_raster::Interrupt<'c>,
    blobs: HashMap<String, Arc<Vec<u8>>>,
    source_cache_hashes: std::collections::HashSet<String>,
}

impl LoadFetch<'_, '_> {
    fn account(&mut self, n: usize) -> Result<()> {
        self.total = self.total.checked_add(n as u64).ok_or_else(|| FormatError::LimitExceeded("bundle expansion overflow".into()))?;
        if self.total > self.opts.max_total_bytes {
            return Err(FormatError::LimitExceeded(format!("bundle expands beyond {} bytes", self.opts.max_total_bytes)));
        }
        Ok(())
    }
    fn capacity(&mut self, n: usize) -> Result<()> {
        self.retained_capacity = self
            .retained_capacity
            .checked_add(n as u64)
            .filter(|total| *total <= self.opts.max_total_bytes)
            .ok_or_else(|| FormatError::LimitExceeded("bundle retained Vec capacity".into()))?;
        Ok(())
    }
}

impl Fetch for LoadFetch<'_, '_> {
    fn source_cache_bytes(&mut self, hash: &str, max: usize, control: &photocraft_raster::Interrupt<'_>) -> Result<Arc<Vec<u8>>> {
        let bytes = self.blob_bounded_with(hash, max, control)?;
        // Keep authenticated blob sharing, while retaining the old logical
        // repeated-descriptor expansion charge (formerly caused by eviction).
        if self.source_cache_hashes.contains(hash) {
            self.account(bytes.len())?;
        } else {
            if self.source_cache_hashes.len() >= 10_000 {
                return Err(FormatError::LimitExceeded("source caches".into()));
            }
            self.source_cache_hashes.try_reserve(1).map_err(|_| FormatError::LimitExceeded("source cache identity allocation".into()))?;
            let mut key = String::new();
            key.try_reserve_exact(hash.len()).map_err(|_| FormatError::LimitExceeded("source cache identity allocation".into()))?;
            key.push_str(hash);
            self.source_cache_hashes.insert(key);
        }
        if control.cancelled() {
            return Err(FormatError::Cancelled);
        }
        Ok(bytes)
    }
    fn tile(&mut self, hash: &str, len: usize) -> Result<Vec<u8>> {
        if !is_valid_hash(hash) {
            return Err(FormatError::corrupt(format!("invalid tile hash `{hash}`")));
        }
        self.account(len)?;
        let path = tile_path(hash);
        let encoded_limit =
            len.checked_add(len / 8).and_then(|n| n.checked_add(4096)).ok_or_else(|| FormatError::LimitExceeded("tile encoded length overflow".into()))?;
        let z = self.src.get_with_interrupt(&path, encoded_limit, &self.control)?;
        let remaining =
            self.opts.max_total_bytes.checked_sub(self.retained_capacity).ok_or_else(|| FormatError::LimitExceeded("tile retained capacity".into()))?;
        if (z.capacity() as u64).checked_add(len as u64).and_then(|n| n.checked_add(16)).is_none_or(|n| n > remaining) {
            return Err(FormatError::LimitExceeded("tile decode buffer envelope".into()));
        }
        let data = decompress_with(&z, len, &path, &self.control)?;
        self.capacity(data.capacity())?;
        let mut observed = blake3::Hasher::new();
        for chunk in data.chunks(128 << 10) {
            if self.control.cancelled() {
                return Err(FormatError::Cancelled);
            }
            observed.update(chunk);
        }
        if self.control.cancelled() {
            return Err(FormatError::Cancelled);
        }
        if observed.finalize().to_hex().as_str() != hash {
            return Err(FormatError::corrupt(format!("{path}: content does not match its hash")));
        }
        Ok(data)
    }

    fn blob(&mut self, hash: &str) -> Result<Arc<Vec<u8>>> {
        self.blob_bounded(hash, self.opts.max_blob_bytes)
    }
    fn blob_bounded(&mut self, hash: &str, max: usize) -> Result<Arc<Vec<u8>>> {
        let control = self.control;
        self.blob_bounded_with(hash, max, &control)
    }
    fn blob_bounded_with(&mut self, hash: &str, max: usize, control: &photocraft_raster::Interrupt<'_>) -> Result<Arc<Vec<u8>>> {
        if control.cancelled() {
            return Err(FormatError::Cancelled);
        }
        let max = max.min(self.opts.max_blob_bytes);
        if let Some(b) = self.blobs.get(hash) {
            if b.len() > max {
                return Err(FormatError::LimitExceeded("bounded source cache blob".into()));
            }
            return Ok(b.clone());
        }
        if !is_valid_hash(hash) {
            return Err(FormatError::corrupt(format!("invalid blob hash `{hash}`")));
        }
        let path = blob_path(hash);
        let encoded_limit = max.saturating_mul(2).saturating_add(64 << 10).min(self.opts.max_blob_bytes);
        let z = self.src.get_with_interrupt(&path, encoded_limit, control)?;
        // Blob length is not in the manifest; the frame header bounds it and
        // we cap at max_blob_bytes.
        let remaining = self
            .opts
            .max_total_bytes
            .checked_sub(self.retained_capacity)
            .and_then(|n| n.checked_sub(z.capacity() as u64))
            .and_then(|n| n.checked_sub(16))
            .ok_or_else(|| FormatError::LimitExceeded("blob decode buffer envelope".into()))?;
        let decoded_limit = max.min(usize::try_from(remaining).unwrap_or(usize::MAX));
        // Keep the existing decoder window ceiling: lowering it to the output
        // size would reject valid small objects encoded with a larger window.
        let mut dec = ruzstd::decoding::StreamingDecoder::new(&z[..]).map_err(|e| FormatError::corrupt(format!("{path}: zstd: {e}")))?;
        let data = crate::read::read_all_bounded(&mut dec, 0, decoded_limit, control).map_err(|error| {
            if control.cancelled() {
                FormatError::Cancelled
            } else if crate::read::is_bounded_read_limit(&error) {
                FormatError::LimitExceeded(format!("{path}: {error}"))
            } else {
                FormatError::corrupt(format!("{path}: zstd: {error}"))
            }
        })?;
        self.capacity(data.capacity())?;
        self.account(data.len())?;
        let mut observed = blake3::Hasher::new();
        for chunk in data.chunks(128 << 10) {
            if control.cancelled() {
                return Err(FormatError::Cancelled);
            }
            observed.update(chunk);
        }
        if observed.finalize().to_hex().as_str() != hash {
            return Err(FormatError::corrupt(format!("{path}: content does not match its hash")));
        }
        let data = Arc::new(data);
        self.blobs.insert(hash.to_owned(), data.clone());
        Ok(data)
    }
}

pub(crate) fn load(src: &dyn Source, opts: &LoadOptions) -> Result<Document> {
    load_with_interrupt(src, opts, &photocraft_raster::Interrupt::NONE)
}
pub(crate) fn load_with_interrupt(src: &dyn Source, opts: &LoadOptions, control: &photocraft_raster::Interrupt<'_>) -> Result<Document> {
    if control.cancelled() {
        return Err(FormatError::Cancelled);
    }
    let m = read_manifest_with_interrupt(src, opts, control)?;
    if m.current_storage_hint.is_some() {
        return Err(FormatError::Unsupported("native current storage hint requires owned handoff".into()));
    }
    admit_canvas(m.document.size, opts)?;
    let mut fetch = LoadFetch {
        src,
        opts: *opts,
        total: 0,
        retained_capacity: 0,
        control: *control,
        blobs: HashMap::new(),
        source_cache_hashes: std::collections::HashSet::new(),
    };
    let mut loader = Loader {
        fetch: &mut fetch,
        preserve_ids: opts.preserve_ids,
        max_id: 0,
        id_map: HashMap::new(),
        channel_map: HashMap::new(),
        path_map: HashMap::new(),
        contents_id_map: HashMap::new(),
        legacy_linked_contents: HashMap::new(),
        tiles: HashMap::new(),
    };
    let doc = loader.document_with_interrupt(&m.document, control)?;
    if convert::requires_reader_v9_state_with(&doc, *control)? && m.minimum_reader_version < 9 {
        return Err(FormatError::corrupt("raw stored color requires minimum reader version 9"));
    }
    if convert::requires_reader_v3_state(&doc) && m.minimum_reader_version < 3 {
        return Err(FormatError::corrupt("required source/working state requires minimum reader version 3"));
    }
    if convert::requires_reader_v4_state_with(&doc, *control)? && m.minimum_reader_version < 4 {
        return Err(FormatError::corrupt("operative text placement requires minimum reader version 4"));
    }
    if convert::requires_reader_v6_state_with(&doc, *control)? && m.minimum_reader_version < 6 {
        return Err(FormatError::corrupt("current resource resolution requires minimum reader version 6"));
    }
    if convert::requires_reader_v8_state_with(&doc, *control)? && m.minimum_reader_version < 8 {
        return Err(FormatError::corrupt("Brown layer label requires minimum reader version 8"));
    }
    if convert::requires_reader_v7_state_with(&doc, *control)? && m.minimum_reader_version < 7 {
        return Err(FormatError::corrupt("current linked geometry requires minimum reader version 7"));
    }
    if opts.preserve_ids && !convert::reserve_ids_through(loader.max_id) {
        // Ids far beyond our counter: remap instead of risking collisions.
        let fresh = LoadOptions { preserve_ids: false, ..*opts };
        return load_with_interrupt(src, &fresh, control);
    }
    Ok(doc)
}

/// Refuses a declared canvas outside `opts`' budget before anything is decoded for it (#2935).
fn admit_canvas(size: photocraft_geom::Size, opts: &LoadOptions) -> Result<()> {
    let (w, h) = (size.width, size.height);
    if w == 0 || h == 0 {
        return Err(FormatError::corrupt(format!("canvas size {w}x{h} is empty")));
    }
    if w > opts.max_canvas_side || h > opts.max_canvas_side {
        return Err(FormatError::LimitExceeded(format!("canvas {w}x{h} exceeds {} pixels per side", opts.max_canvas_side)));
    }
    if size.area() > opts.max_canvas_pixels {
        return Err(FormatError::LimitExceeded(format!("canvas {w}x{h} exceeds {} pixels", opts.max_canvas_pixels)));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------------

/// What a save did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SaveStats {
    /// Distinct tiles referenced by the document.
    pub tiles_total: usize,
    /// Tiles that had to be compressed (ZIP) or written (directory).
    pub tiles_written: usize,
    /// Tiles reused from the previous save.
    pub tiles_reused: usize,
    pub blobs_total: usize,
    pub blobs_written: usize,
    /// Unreferenced objects removed (directory bundles).
    pub objects_removed: usize,
    /// Bytes of the manifest.
    pub manifest_bytes: usize,
}

/// Lowerable limits for additional allocations during save preparation.
/// Existing input document residency must be accounted by the caller; these are
/// sub-budgets, not permission to exceed the selected transaction profile.
/// Complex cloned elements currently have a separate 64 KiB measurement cap.
#[derive(Clone, Copy, Debug)]
pub struct PreparationLimits {
    pub resident_bytes: usize,
    /// Conservative index allowance for new object/cache bookkeeping.
    pub index_bytes: usize,
    pub manifest_bytes: usize,
    pub object_bytes: usize,
}
impl Default for PreparationLimits {
    fn default() -> Self {
        Self {
            resident_bytes: if cfg!(target_arch = "wasm32") { 512 << 20 } else { 2usize << 30 },
            index_bytes: if cfg!(target_arch = "wasm32") { 16 << 20 } else { 64 << 20 },
            manifest_bytes: 256 << 20,
            object_bytes: if cfg!(target_arch = "wasm32") { 256 << 20 } else { 1 << 30 },
        }
    }
}
impl PreparationLimits {
    fn validate(self) -> Result<()> {
        let ceiling = Self::default();
        if self.resident_bytes > ceiling.resident_bytes
            || self.index_bytes > ceiling.index_bytes
            || self.manifest_bytes > ceiling.manifest_bytes
            || self.object_bytes > ceiling.object_bytes
        {
            return Err(FormatError::LimitExceeded("native preparation limits exceed platform ceiling".into()));
        }
        Ok(())
    }
}
/// Optional shared job ledger. Reservations are conservative requested capacities;
/// release must not fail. The observed fresh-writer API retains them through writing.
pub trait PreparationObserver {
    fn reserve(&self, bytes: usize) -> Result<()>;
    fn release(&self, bytes: usize);
}
struct ReservationScope<'a> {
    observer: &'a dyn PreparationObserver,
    live: std::cell::Cell<usize>,
}
impl PreparationObserver for ReservationScope<'_> {
    fn reserve(&self, bytes: usize) -> Result<()> {
        let next = self.live.get().checked_add(bytes).ok_or_else(|| FormatError::LimitExceeded("native shared reservation overflow".into()))?;
        self.observer.reserve(bytes)?;
        self.live.set(next);
        Ok(())
    }
    fn release(&self, bytes: usize) {
        let released = bytes.min(self.live.get());
        self.live.set(self.live.get() - released);
        self.observer.release(released);
    }
}
impl Drop for ReservationScope<'_> {
    fn drop(&mut self) {
        self.observer.release(self.live.get());
    }
}
type TileHashCache = HashMap<(usize, SampleType), (Weak<Tile>, Hash)>;
struct Collect<'doc, 'c, 'i, 'o> {
    tiles: BTreeMap<Hash, (Arc<Tile>, SampleType)>,
    blobs: BTreeMap<Hash, Object<'doc>>,
    hash_cache: Option<&'c mut TileHashCache>,
    observer: Option<&'o dyn PreparationObserver>,
    control: photocraft_raster::Interrupt<'i>,
    limits: PreparationLimits,
    resident: usize,
    index: usize,
}
impl Collect<'_, '_, '_, '_> {
    fn reserve_index(&mut self, bytes: usize) -> Result<()> {
        let next = self.index.checked_add(bytes).ok_or_else(|| FormatError::LimitExceeded("native preparation index overflow".into()))?;
        if next > self.limits.index_bytes {
            return Err(FormatError::LimitExceeded("native preparation index bytes".into()));
        }
        self.reserve(bytes)?;
        self.index = next;
        Ok(())
    }
    fn hash(&self, data: &[u8], sample: Option<SampleType>) -> Result<Hash> {
        self.poll()?;
        if data.len() > self.limits.object_bytes {
            return Err(FormatError::LimitExceeded("native preparation object bytes".into()));
        }
        let mut hasher = blake3::Hasher::new();
        let mut scratch = [0u8; 8192];
        for chunk in data.chunks(8192) {
            self.poll()?;
            if cfg!(target_endian = "big")
                && let Some(sample) = sample
            {
                let target = scratch.get_mut(..chunk.len()).ok_or_else(|| FormatError::LimitExceeded("native hash chunk".into()))?;
                target.copy_from_slice(chunk);
                swap_to_le(target, sample);
                hasher.update(target);
            } else {
                hasher.update(chunk);
            }
        }
        Ok(hasher.finalize().to_hex().to_string())
    }
}
impl<'doc> Sink<'doc> for Collect<'doc, '_, '_, '_> {
    fn poll(&self) -> Result<()> {
        if self.control.cancelled() { Err(FormatError::Cancelled) } else { Ok(()) }
    }
    fn reserve(&mut self, bytes: usize) -> Result<()> {
        self.poll()?;
        let next = self.resident.checked_add(bytes).ok_or_else(|| FormatError::LimitExceeded("native preparation resident overflow".into()))?;
        if next > self.limits.resident_bytes {
            return Err(FormatError::LimitExceeded("native preparation resident bytes".into()));
        }
        if let Some(observer) = self.observer {
            observer.reserve(bytes)?;
        }
        self.resident = next;
        Ok(())
    }
    fn release(&mut self, bytes: usize) {
        let released = bytes.min(self.resident);
        self.resident -= released;
        if let Some(observer) = self.observer {
            observer.release(released);
        }
    }
    fn max_manifest_bytes(&self) -> usize {
        self.limits.manifest_bytes
    }
    fn control(&self) -> photocraft_raster::Interrupt<'_> {
        self.control
    }
    fn tile(&mut self, format: PixelFormat, tile: &Arc<Tile>) -> Result<Hash> {
        self.poll()?;
        if tile.bytes().len() > self.limits.object_bytes {
            return Err(FormatError::LimitExceeded("native preparation object bytes".into()));
        }
        let key = (Arc::as_ptr(tile) as usize, format.sample);
        let cached = self
            .hash_cache
            .as_deref()
            .and_then(|cache| cache.get(&key))
            .filter(|(w, _)| w.upgrade().is_some_and(|t| Arc::ptr_eq(&t, tile)))
            .map(|(_, h)| h.clone());
        let h = match cached {
            Some(h) => h,
            None => self.hash(tile.bytes(), Some(format.sample))?,
        };
        if !self.tiles.contains_key(&h) {
            self.reserve_index(1024)?;
            self.tiles.insert(h.clone(), (tile.clone(), format.sample));
        }
        if self.hash_cache.as_deref().is_some_and(|cache| !cache.contains_key(&key)) {
            self.reserve_index(256)?;
        }
        if let Some(cache) = self.hash_cache.as_deref_mut() {
            cache.try_reserve(1).map_err(|_| FormatError::LimitExceeded("native hash cache allocation".into()))?;
            cache.insert(key, (Arc::downgrade(tile), h.clone()));
        }
        Ok(h)
    }
    fn blob(&mut self, data: &Arc<Vec<u8>>) -> Result<Hash> {
        let h = self.hash(data, None)?;
        if !self.blobs.contains_key(&h) {
            self.reserve_index(1024)?;
            self.blobs.insert(h.clone(), Object::Blob(data.clone()));
        }
        Ok(h)
    }
    fn borrowed_blob(&mut self, data: &'doc [u8]) -> Result<Hash> {
        let h = self.hash(data, None)?;
        if !self.blobs.contains_key(&h) {
            self.reserve_index(1024)?;
            self.blobs.insert(h.clone(), Object::BorrowedBlob(data));
        }
        Ok(h)
    }
}

fn tile_le(t: &Tile, sample: SampleType) -> std::borrow::Cow<'_, [u8]> {
    if cfg!(target_endian = "big") {
        let mut b = t.bytes().to_vec();
        swap_to_le(&mut b, sample);
        std::borrow::Cow::Owned(b)
    } else {
        std::borrow::Cow::Borrowed(t.bytes())
    }
}

/// Default for [`PcraftWriter::set_reverify_budget`]: compressed bytes of already-verified
/// objects re-read per directory save.
pub const DEFAULT_REVERIFY_BUDGET: u64 = 16 * 1024 * 1024;

/// Incremental `.pcraft` writer. Keep one per open document: it remembers
/// tile hashes (by `Arc` identity), compressed objects, and objects it has
/// verified in the current directory, so repeated saves stay cheap.
///
/// **What the directory cache trusts.** A directory save reuses an existing object file only
/// after decompressing it and checking its content hash. After that, the writer remembers the
/// file's signature (length and modification time, plus inode, device and change time on Unix)
/// and skips the full check on later saves while the signature is unchanged. A file damaged in
/// place without changing its signature (bit rot, or a tool that restores timestamps where
/// there is no change time) would be trusted, so each save also re-verifies a rolling share of
/// the cached objects, oldest-checked first, up to [`DEFAULT_REVERIFY_BUDGET`] bytes (see
/// [`Self::set_reverify_budget`]). Over enough saves every object is read again, and a damaged
/// one is rewritten.
pub struct PcraftWriter {
    hash_cache: TileHashCache,
    /// Compressed objects by bundle path (ZIP mode).
    compressed: HashMap<String, Arc<Vec<u8>>>,
    verified_directory: Option<DirectoryVerificationCache>,
    reverify_budget: u64,
    preparation_limits: PreparationLimits,
}

impl Default for PcraftWriter {
    fn default() -> Self {
        Self {
            hash_cache: HashMap::new(),
            compressed: HashMap::new(),
            verified_directory: None,
            reverify_budget: DEFAULT_REVERIFY_BUDGET,
            preparation_limits: PreparationLimits::default(),
        }
    }
}

#[derive(Default)]
struct DirectoryVerificationCache {
    directory: PathBuf,
    /// Directory saves so far; an object's `checked_at` is the save that last read it fully.
    generation: u64,
    objects: HashMap<String, Verified>,
}

#[derive(Clone, Copy)]
struct Verified {
    signature: FileSignature,
    checked_at: u64,
}

/// What a file looked like when it was verified. Any change means "verify again".
#[derive(Clone, Copy, PartialEq, Eq)]
struct FileSignature {
    len: u64,
    modified: SystemTime,
    /// Inode, device and status-change time (s, ns): an in-place rewrite changes the change
    /// time even when a tool restores the modification time.
    #[cfg(unix)]
    unix: (u64, u64, i64, i64),
}

struct Prepared<'doc> {
    manifest: Vec<u8>,
    objects: BTreeMap<String, Object<'doc>>,
    previews: Vec<(&'static str, Vec<u8>)>,
    stats: SaveStats,
}

enum Object<'doc> {
    Tile(Arc<Tile>, SampleType),
    Blob(Arc<Vec<u8>>),
    BorrowedBlob(&'doc [u8]),
}

impl Object<'_> {
    fn compressed(&self) -> Vec<u8> {
        compress(&self.content())
    }

    /// The bytes this object stores (tiles little-endian).
    fn content(&self) -> std::borrow::Cow<'_, [u8]> {
        match self {
            Object::Tile(tile, sample) => tile_le(tile, *sample),
            Object::Blob(blob) => std::borrow::Cow::Borrowed(blob.as_slice()),
            Object::BorrowedBlob(blob) => std::borrow::Cow::Borrowed(*blob),
        }
    }
}

/// Whether `compressed` decodes to exactly `expected`.
fn content_matches(expected: &[u8], compressed: &[u8], what: &str) -> bool {
    decompress(compressed, expected.len(), what).is_ok_and(|actual| hash_bytes(&actual) == hash_bytes(expected))
}

fn file_signature(metadata: &std::fs::Metadata) -> Option<FileSignature> {
    #[cfg(unix)]
    let unix = {
        use std::os::unix::fs::MetadataExt;
        (metadata.ino(), metadata.dev(), metadata.ctime(), metadata.ctime_nsec())
    };
    Some(FileSignature {
        len: metadata.len(),
        modified: metadata.modified().ok()?,
        #[cfg(unix)]
        unix,
    })
}

fn path_signature(path: &Path) -> Result<Option<FileSignature>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(file_signature(&metadata)),
        Ok(_) => Err(unsafe_bundle_entry(path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn unsafe_bundle_entry(path: &Path) -> FormatError {
    FormatError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("refusing directory bundle save: {} is a symlink or not a regular file", path.display()),
    ))
}

/// The generic atomic writer intentionally follows a destination symlink. Directory bundles
/// must reject one instead: all of their managed entries stay below the bundle root.
fn plain_file_or_missing(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(unsafe_bundle_entry(path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn plain_directory_or_missing(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(FormatError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("refusing directory bundle save: {} is a symlink or not a directory", path.display()),
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Use the canonical root after creation so a later replacement of the caller's path cannot
/// redirect subsequent bundle operations. The caller may choose a symlinked root, but managed
/// subdirectories and files below its resolved target must not be symlinks.
fn bundle_root(dir: &Path) -> Result<PathBuf> {
    let dir: PathBuf = dir.components().collect();
    std::fs::create_dir_all(&dir)?;
    let root = std::fs::canonicalize(&dir)?;
    plain_directory_or_missing(&root)?;
    for sub in ["tiles", "blobs", "composite"] {
        let path = root.join(sub);
        plain_directory_or_missing(&path)?;
        if !path.exists() {
            match std::fs::create_dir(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        plain_directory_or_missing(&path)?;
        if std::fs::canonicalize(&path)? != path {
            return Err(unsafe_bundle_entry(&path));
        }
    }
    Ok(root)
}

fn validate_bundle_file(root: &Path, path: &Path) -> Result<()> {
    let parent = path.parent().ok_or_else(|| unsafe_bundle_entry(path))?;
    if parent != root && parent != root.join("tiles") && parent != root.join("blobs") && parent != root.join("composite") {
        return Err(unsafe_bundle_entry(path));
    }
    plain_directory_or_missing(parent)?;
    if std::fs::canonicalize(parent)? != parent {
        return Err(unsafe_bundle_entry(parent));
    }
    plain_file_or_missing(path)
}

fn write_bundle_file(root: &Path, path: &Path, data: &[u8]) -> Result<()> {
    validate_bundle_file(root, path)?;
    let parent = path.parent().ok_or_else(|| unsafe_bundle_entry(path))?;
    let name = path.file_name().ok_or_else(|| unsafe_bundle_entry(path))?;
    let mut temporary = None;
    for _ in 0..8 {
        let candidate = parent.join(crate::atomic::temp_name(&name.to_string_lossy()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    let (tmp, mut file) = temporary.ok_or_else(|| {
        FormatError::Io(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("could not create a temporary file beside {}", path.display())))
    })?;
    let result = (|| -> Result<()> {
        // The parent may have changed between validation and create_new. Check the new file's
        // resolved location before putting document bytes into it.
        if std::fs::canonicalize(&tmp)? != tmp {
            return Err(unsafe_bundle_entry(&tmp));
        }
        validate_bundle_file(root, path)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        // Preserve the existing mode where possible, as the generic atomic writer does.
        if let Ok(permissions) = std::fs::metadata(path).map(|m| m.permissions()) {
            let _ = std::fs::set_permissions(&tmp, permissions);
        }
        // Recheck the parent and target just before publishing. rename replaces a symlink
        // itself, never its referent, even if one appears after this check.
        validate_bundle_file(root, path)?;
        crate::atomic::retry_rename(crate::atomic::RenameRetry::platform(), || std::fs::rename(&tmp, path))?;
        #[cfg(unix)]
        {
            let _ = std::fs::File::open(parent).and_then(|d| d.sync_all());
        }
        Ok(())
    })();
    if result.is_err() && validate_bundle_file(root, &tmp).is_ok() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Keep the existing leaf/path validation and pin the actual unlink to opened
/// directory handles on Unix. This closes the check-then-remove path-swap gap.
fn remove_bundle_file(root: &Path, path: &Path, gc: &BundleGc) -> Result<()> {
    validate_bundle_file(root, path)?;
    let relative = path.strip_prefix(root).map_err(|_| unsafe_bundle_entry(path))?;
    gc.remove(relative)
}

#[cfg(unix)]
struct BundleGc {
    root: rustix::fd::OwnedFd,
    tiles: rustix::fd::OwnedFd,
    blobs: rustix::fd::OwnedFd,
    composite: rustix::fd::OwnedFd,
}

#[cfg(unix)]
fn gc_directory_flags() -> rustix::fs::OFlags {
    use rustix::fs::OFlags;

    // GC needs directory traversal, not permission to read a directory listing.
    // Both modes retain a descriptor for the directory opened here, so a later
    // pathname replacement cannot redirect descriptor-relative cleanup.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let access = OFlags::PATH;
    #[cfg(target_vendor = "apple")]
    let access = OFlags::from_bits_retain(libc::O_SEARCH.cast_unsigned());
    #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
    let access = OFlags::RDONLY;
    access | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

#[cfg(unix)]
impl BundleGc {
    fn new(root: &Path) -> Result<Self> {
        use rustix::fs::{Mode, open, openat};

        let flags = gc_directory_flags();
        let root = open(root, flags, Mode::empty()).map_err(std::io::Error::from)?;
        let tiles = openat(&root, "tiles", flags, Mode::empty()).map_err(std::io::Error::from)?;
        let blobs = openat(&root, "blobs", flags, Mode::empty()).map_err(std::io::Error::from)?;
        let composite = openat(&root, "composite", flags, Mode::empty()).map_err(std::io::Error::from)?;
        Ok(Self { root, tiles, blobs, composite })
    }

    fn remove(&self, relative: &Path) -> Result<()> {
        let relative = relative.to_str().ok_or_else(|| unsafe_bundle_entry(relative))?;
        let (handle, name) = match relative {
            THUMB => (&self.root, THUMB),
            COMPOSITE => (&self.composite, "preview.png"),
            _ => match relative.split_once('/') {
                Some(("tiles", name)) if name.strip_suffix(".zst").is_some_and(is_valid_hash) => (&self.tiles, name),
                Some(("blobs", name)) if name.strip_suffix(".zst").is_some_and(is_valid_hash) => (&self.blobs, name),
                _ => return Err(FormatError::Unsupported(format!("invalid bundle cleanup target: {relative}"))),
            },
        };
        rustix::fs::unlinkat(handle, name, rustix::fs::AtFlags::empty()).map_err(std::io::Error::from)?;
        Ok(())
    }
}

#[cfg(not(unix))]
struct BundleGc {
    root: PathBuf,
}

#[cfg(not(unix))]
impl BundleGc {
    fn new(root: &Path) -> Result<Self> {
        Ok(Self { root: root.to_path_buf() })
    }

    fn remove(&self, relative: &Path) -> Result<()> {
        std::fs::remove_file(self.root.join(relative))?;
        Ok(())
    }
}

fn existing_object_is_valid(path: &Path, object: &Object, what: &str) -> Result<(bool, Option<FileSignature>)> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((false, None)),
        Err(e) => return Err(e.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Ok((false, None));
    }
    let signature = file_signature(&metadata);

    let expected_len = match object {
        Object::Tile(tile, sample) => tile_le(tile, *sample).len(),
        Object::Blob(blob) => blob.len(),
        Object::BorrowedBlob(blob) => blob.len(),
    };
    // This is the same bound used by the tile loader. It comfortably includes
    // zstd framing overhead while keeping damaged files bounded when inspected.
    let max_compressed_len = expected_len.saturating_add(expected_len / 8).saturating_add(4096);
    let max_compressed_len_u64 = u64::try_from(max_compressed_len).unwrap_or(u64::MAX);
    if metadata.len() > max_compressed_len_u64 {
        return Ok((false, None));
    }

    let compressed = match crate::read::read_all_bounded(&mut file, metadata.len(), max_compressed_len, &photocraft_raster::Interrupt::NONE) {
        Ok(bytes) => bytes,
        Err(error) if crate::read::is_bounded_read_limit(&error) => return Ok((false, None)),
        Err(error) => return Err(error.into()),
    };
    Ok((content_matches(&object.content(), &compressed, what), signature))
}

/// A bundle path and its compressed bytes.
type Compressed<'a> = (&'a String, Arc<Vec<u8>>);

/// Compresses `objects` on scoped worker threads (sequentially on wasm).
fn par_compress<'a>(objects: &[(&'a String, &'a Object)]) -> Result<Vec<Compressed<'a>>> {
    let threads = if cfg!(target_arch = "wasm32") { 1 } else { std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 32) };
    if threads < 2 || objects.len() < 2 {
        return Ok(objects.iter().map(|(p, o)| (*p, Arc::new(o.compressed()))).collect());
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|sc| {
        let hs: Vec<_> = (0..threads.min(objects.len()))
            .map(|_| {
                sc.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((p, o)) = objects.get(i) else { break };
                        out.push((*p, Arc::new(o.compressed())));
                    }
                    out
                })
            })
            .collect();
        // Join every worker before looking at the results, so one failure can't leave
        // another worker unjoined (the scope would then panic).
        let joined: Vec<_> = hs.into_iter().map(|h| h.join()).collect();
        let mut out = Vec::with_capacity(objects.len());
        for r in joined {
            out.extend(r.map_err(|_| FormatError::Io(std::io::Error::other("compression worker panicked")))?);
        }
        Ok(out)
    })
}

impl PcraftWriter {
    /// Lower preparation ceilings; zero denies any phase requiring that budget.
    /// Existing input residency and preview codec work remain caller/backend responsibilities.
    /// Current lower-only preparation limits (requested capacities, not allocator RSS).
    pub fn preparation_limits(&self) -> PreparationLimits {
        self.preparation_limits
    }
    pub fn set_preparation_limits(&mut self, limits: PreparationLimits) -> Result<()> {
        limits.validate()?;
        self.preparation_limits = limits;
        Ok(())
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// How many compressed bytes of already-verified objects each directory save re-reads and
    /// re-checks (oldest-checked first; at least one object when the budget is non-zero). `0`
    /// trusts unchanged signatures entirely; `u64::MAX` re-verifies everything on every save.
    pub fn set_reverify_budget(&mut self, bytes: u64) {
        self.reverify_budget = bytes;
    }

    fn prepare<'doc>(
        &mut self,
        doc: &'doc Document,
        opts: &SaveOptions,
        control: photocraft_raster::Interrupt<'_>,
        object_limit: Option<usize>,
        observer: Option<&dyn PreparationObserver>,
    ) -> Result<Prepared<'doc>> {
        let mut cancelled = false;
        self.hash_cache.retain(|_, (w, _)| {
            cancelled |= control.cancelled();
            w.strong_count() > 0
        });
        if cancelled {
            return Err(FormatError::Cancelled);
        }
        convert::check_nesting(&doc.layers, control)?;
        self.preparation_limits.validate()?;
        let mut c = Collect {
            hash_cache: Some(&mut self.hash_cache),
            observer,
            tiles: BTreeMap::new(),
            blobs: BTreeMap::new(),
            control,
            limits: PreparationLimits {
                object_bytes: object_limit.unwrap_or(self.preparation_limits.object_bytes).min(self.preparation_limits.object_bytes),
                ..self.preparation_limits
            },
            resident: 0,
            index: 0,
        };
        let document = convert::doc_m(doc, &mut c)?;
        let mut previews = Vec::new();
        if let Some(t) = &opts.thumbnail {
            c.poll()?;
            c.reserve(t.pixels.len().checked_mul(8).ok_or_else(|| FormatError::LimitExceeded("native preview resident overflow".into()))?)?;
            previews.push((THUMB, png(t)?));
            c.poll()?;
        }
        if let Some(t) = &opts.composite {
            c.poll()?;
            c.reserve(t.pixels.len().checked_mul(8).ok_or_else(|| FormatError::LimitExceeded("native preview resident overflow".into()))?)?;
            previews.push((COMPOSITE, png(t)?));
            c.poll()?;
        }
        let required_version = if convert::requires_reader_v9_state_with(doc, control)? {
            9
        } else if convert::requires_reader_v8_state_with(doc, control)? {
            8
        } else if convert::requires_reader_v7_state_with(doc, control)? {
            7
        } else if convert::requires_reader_v6_state_with(doc, control)? {
            6
        } else if doc.has_backed_rasters().map_err(|_| FormatError::LimitExceeded("native layer nesting".into()))? {
            5
        } else if convert::requires_reader_v4_state_with(doc, control)? {
            4
        } else if document.xcf_source_caches.is_empty() && !convert::requires_reader_v3_state_with(doc, control)? {
            2
        } else {
            3
        };
        let manifest = Manifest {
            format_version: required_version,
            minimum_reader_version: required_version,
            current_storage_hint: None,
            generator: format!("photocraft-format {}", env!("CARGO_PKG_VERSION")),
            document,
            thumbnail: opts.thumbnail.as_ref().map(|_| THUMB.to_owned()),
            composite: opts.composite.as_ref().map(|_| COMPOSITE.to_owned()),
        };
        // serde_json writes NaN and the infinities as `null`, which the manifest's float fields
        // refuse on load (issue #1101): refuse the save instead, naming the value.
        crate::finite::check_with_interrupt(&manifest, control).map_err(|e| match e {
            crate::finite::Problem::NonFinite { path } => FormatError::NonFinite { path },
            crate::finite::Problem::Cancelled => FormatError::Cancelled,
            crate::finite::Problem::Other(m) => FormatError::Unsupported(format!("manifest: {m}")),
        })?;
        let maximum = c.max_manifest_bytes();
        let manifest = convert::json_bytes(&manifest, &mut c, maximum, true)?;
        check_manifest_depth_with(&manifest, control)?;
        let stats = SaveStats { tiles_total: c.tiles.len(), blobs_total: c.blobs.len(), manifest_bytes: manifest.len(), ..Default::default() };
        let mut objects = BTreeMap::new();
        for (h, (t, s)) in c.tiles {
            if control.cancelled() {
                return Err(FormatError::Cancelled);
            }
            objects.insert(tile_path(&h), Object::Tile(t, s));
        }
        for (h, b) in c.blobs {
            if control.cancelled() {
                return Err(FormatError::Cancelled);
            }
            objects.insert(blob_path(&h), b);
        }
        Ok(Prepared { manifest, objects, previews, stats })
    }

    /// Save as a ZIP bundle in memory.
    pub fn save_zip(&mut self, doc: &Document, opts: &SaveOptions) -> Result<(Vec<u8>, SaveStats)> {
        let p = self.prepare(doc, opts, photocraft_raster::Interrupt::NONE, None, None)?;
        let mut stats = p.stats;
        let mut z = ZipWriter::new();
        z.add(MANIFEST, &p.manifest)?;
        for (name, data) in &p.previews {
            z.add(name, data)?;
        }
        let mut next = HashMap::with_capacity(p.objects.len());
        // New objects are compressed on all cores first (zstd dominates a full save).
        let todo: Vec<(&String, &Object)> = p.objects.iter().filter(|(path, _)| !self.compressed.contains_key(*path)).collect();
        let mut fresh: HashMap<&String, Arc<Vec<u8>>> = par_compress(&todo)?.into_iter().collect();
        for (path, obj) in &p.objects {
            let data = match self.compressed.get(path) {
                Some(d) => {
                    if matches!(obj, Object::Tile(..)) {
                        stats.tiles_reused += 1;
                    }
                    d.clone()
                }
                None => {
                    match obj {
                        Object::Tile(..) => stats.tiles_written += 1,
                        Object::Blob(_) | Object::BorrowedBlob(_) => stats.blobs_written += 1,
                    }
                    fresh.remove(path).unwrap_or_else(|| Arc::new(obj.compressed()))
                }
            };
            z.add(path, &data)?;
            next.insert(path.clone(), data);
        }
        self.compressed = next;
        Ok((z.finish()?, stats))
    }

    /// Write ZIP64 entries directly to an empty seekable output. Objects use
    /// Zstandard raw blocks, trading compression for bounded, cancellable copying
    /// without retaining another object-sized buffer or a complete archive.
    /// Existing manifest/object preparation is in-memory; this API alone does
    /// not qualify native_large source residency. Preparation uses bounded interruptible
    /// metadata and hash passes; preview codec calls retain their own backend limits.
    pub fn save_zip_stream<W: std::io::Write + std::io::Seek>(
        &mut self,
        doc: &Document,
        opts: &SaveOptions,
        output: W,
        limits: crate::zip::stream::Limits,
        max_object_bytes: usize,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<(W, SaveStats)> {
        self.save_zip_stream_inner(doc, opts, output, limits, control, (max_object_bytes, None))
    }

    /// Fresh writer with shared accounting. Preparation reservations remain live until
    /// Prepared data and the temporary tile hash cache have both been dropped.
    pub fn save_zip_stream_observed<W: std::io::Write + std::io::Seek>(
        doc: &Document,
        opts: &SaveOptions,
        output: W,
        limits: crate::zip::stream::Limits,
        preparation: PreparationLimits,
        control: &photocraft_raster::Interrupt<'_>,
        observer: &dyn PreparationObserver,
    ) -> Result<(W, SaveStats)> {
        let scope = ReservationScope { observer, live: std::cell::Cell::new(0) };
        let mut writer = Self::new();
        writer.set_preparation_limits(preparation)?;
        // writer drops before scope, including on error; no live cache is refunded.
        writer.save_zip_stream_inner(doc, opts, output, limits, control, (preparation.object_bytes, Some(&scope)))
    }
    fn save_zip_stream_inner<W: std::io::Write + std::io::Seek>(
        &mut self,
        doc: &Document,
        opts: &SaveOptions,
        output: W,
        limits: crate::zip::stream::Limits,
        control: &photocraft_raster::Interrupt<'_>,
        (max_object_bytes, observer): (usize, Option<&dyn PreparationObserver>),
    ) -> Result<(W, SaveStats)> {
        if control.cancelled() {
            return Err(FormatError::Cancelled);
        }
        let prepared = self.prepare(doc, opts, *control, Some(max_object_bytes), observer)?;
        if control.cancelled() {
            return Err(FormatError::Cancelled);
        }
        if prepared.manifest.len() > max_object_bytes || prepared.previews.iter().any(|(_, bytes)| bytes.len() > max_object_bytes) {
            return Err(FormatError::LimitExceeded("streaming native object bytes".into()));
        }
        let mut writer = crate::zip::stream::Writer::new(output, limits)?;
        let write = |writer: &mut crate::zip::stream::Writer<W>, name: &str, data: &[u8]| -> Result<()> {
            if data.len() > max_object_bytes {
                return Err(FormatError::LimitExceeded("streaming native object bytes".into()));
            }
            writer.add(name, data.len() as u64, &mut std::io::Cursor::new(data), control)
        };
        write(&mut writer, MANIFEST, &prepared.manifest)?;
        for (name, data) in &prepared.previews {
            write(&mut writer, name, data)?;
        }
        let mut stats = prepared.stats;
        for (name, object) in &prepared.objects {
            if control.cancelled() {
                return Err(FormatError::Cancelled);
            }
            let raw = match object {
                Object::Tile(tile, sample) => tile_le(tile, *sample),
                Object::Blob(bytes) => std::borrow::Cow::Borrowed(bytes.as_slice()),
                Object::BorrowedBlob(bytes) => std::borrow::Cow::Borrowed(*bytes),
            };
            if raw.len() > max_object_bytes {
                return Err(FormatError::LimitExceeded("streaming native expanded object bytes".into()));
            }
            let size = crate::zstd_stream::encoded_len(u64::try_from(raw.len()).map_err(|_| FormatError::LimitExceeded("native object length".into()))?)?;
            if size > max_object_bytes as u64 {
                return Err(FormatError::LimitExceeded("streaming native framed object bytes".into()));
            }
            writer.add(name, size, &mut crate::zstd_stream::RawFrame::new(&raw)?, control)?;
            match object {
                Object::Tile(..) => stats.tiles_written += 1,
                Object::Blob(..) | Object::BorrowedBlob(..) => stats.blobs_written += 1,
            }
        }
        Ok((writer.finish(control)?, stats))
    }

    /// Stream a ZIP64 bundle into a sibling temporary file, flush it and replace
    /// the destination only after all writes and the final cancellation check.
    /// This does not change the default incremental save cache or qualify the
    /// separately gated native_large input policy.
    pub fn save_path_stream(
        &mut self,
        doc: &Document,
        opts: &SaveOptions,
        path: &Path,
        limits: crate::zip::stream::Limits,
        max_object_bytes: usize,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<SaveStats> {
        crate::atomic::atomic_write_stream(
            path,
            |file| self.save_zip_stream(doc, opts, file, limits, max_object_bytes, control).map(|(_, stats)| stats),
            || if control.cancelled() { Err(FormatError::Cancelled) } else { Ok(()) },
        )
    }

    /// Save into a directory bundle: reuses only valid existing objects,
    /// rewrites missing or damaged objects, then writes the manifest
    /// atomically and removes unreferenced objects.
    pub fn save_dir(&mut self, doc: &Document, dir: &Path, opts: &SaveOptions) -> Result<SaveStats> {
        let canonical_dir = bundle_root(dir)?;
        let dir = canonical_dir.as_path();
        let existing = list_objects(dir)?;
        // Reject a hostile existing entry before writing any object or manifest. This also
        // covers entries that this save would otherwise leave untouched or garbage-collect.
        for path in [MANIFEST, THUMB, COMPOSITE] {
            validate_bundle_file(dir, &dir.join(path))?;
        }
        let p = self.prepare(doc, opts, photocraft_raster::Interrupt::NONE, None, None)?;
        let mut stats = p.stats;
        if self.verified_directory.as_ref().is_none_or(|cache| cache.directory != canonical_dir) {
            self.verified_directory = Some(DirectoryVerificationCache { directory: canonical_dir.clone(), ..Default::default() });
        }
        let generation = match &mut self.verified_directory {
            Some(cache) => {
                cache.generation = cache.generation.saturating_add(1);
                cache.generation
            }
            None => 0,
        };
        let recheck = self.rolling_recheck(&p.objects);
        for (path, obj) in &p.objects {
            let object_path = dir.join(path);
            // bundle_root and list_objects validated the fixed parent directories once. Check
            // each leaf here; write_bundle_file rechecks its parent around every mutation.
            plain_file_or_missing(&object_path)?;
            let signature = path_signature(&object_path)?;
            let already_verified = !recheck.contains(path)
                && signature.is_some_and(|signature| {
                    self.verified_directory.as_ref().is_some_and(|cache| cache.objects.get(path).is_some_and(|v| v.signature == signature))
                });
            if existing.contains(path) && already_verified {
                if matches!(obj, Object::Tile(..)) {
                    stats.tiles_reused += 1;
                }
                continue;
            }
            if existing.contains(path) {
                let (valid, signature) = existing_object_is_valid(&object_path, obj, path)?;
                if valid {
                    if let Some(signature) = signature
                        && let Some(cache) = &mut self.verified_directory
                    {
                        cache.objects.insert(path.clone(), Verified { signature, checked_at: generation });
                    }
                    if matches!(obj, Object::Tile(..)) {
                        stats.tiles_reused += 1;
                    }
                    continue;
                }
            }
            match obj {
                Object::Tile(..) => stats.tiles_written += 1,
                Object::Blob(_) | Object::BorrowedBlob(_) => stats.blobs_written += 1,
            }
            write_bundle_file(dir, &object_path, &obj.compressed())?;
            let written = path_signature(&object_path)?;
            if let Some(cache) = &mut self.verified_directory {
                match written {
                    Some(signature) => {
                        cache.objects.insert(path.clone(), Verified { signature, checked_at: generation });
                    }
                    None => {
                        cache.objects.remove(path);
                    }
                }
            }
        }
        for (name, data) in &p.previews {
            write_bundle_file(dir, &dir.join(name), data)?;
        }
        // Pin all GC directories before removing stale previews or objects.
        // Unix cleanup then stays within these directories even if a pathname is swapped.
        let gc = BundleGc::new(dir)?;
        for stale in [THUMB, COMPOSITE] {
            if !p.previews.iter().any(|(n, _)| *n == stale) {
                let path = dir.join(stale);
                validate_bundle_file(dir, &path)?;
                match remove_bundle_file(dir, &path, &gc) {
                    Err(FormatError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
                    other => other?,
                }
            }
        }
        write_bundle_file(dir, &dir.join(MANIFEST), &p.manifest)?;
        // The manifest is committed: the save has succeeded. Removing objects it no longer
        // references is best-effort housekeeping (a vanished or locked file must not turn a
        // complete save into a reported failure); a leftover is retried on the next save.
        for path in existing {
            if !p.objects.contains_key(&path) {
                if remove_bundle_file(dir, &dir.join(&path), &gc).is_ok() {
                    stats.objects_removed += 1;
                }
                if let Some(cache) = &mut self.verified_directory {
                    cache.objects.remove(&path);
                }
            }
        }
        Ok(stats)
    }

    /// The cached objects this save re-verifies even though their signature is unchanged:
    /// oldest-checked first, until their recorded sizes reach the re-verify budget (always at
    /// least one, so an object larger than the budget is still reached in turn).
    fn rolling_recheck(&self, objects: &BTreeMap<String, Object>) -> HashSet<String> {
        let mut out = HashSet::new();
        let Some(cache) = &self.verified_directory else { return out };
        if self.reverify_budget == 0 {
            return out;
        }
        let mut candidates: Vec<(&String, &Verified)> = cache.objects.iter().filter(|(path, _)| objects.contains_key(*path)).collect();
        candidates.sort_by(|a, b| a.1.checked_at.cmp(&b.1.checked_at).then_with(|| a.0.cmp(b.0)));
        let mut spent = 0u64;
        for (path, verified) in candidates {
            let next = spent.saturating_add(verified.signature.len);
            if !out.is_empty() && next > self.reverify_budget {
                break;
            }
            spent = next;
            out.insert(path.clone());
        }
        out
    }

    /// Save to `path`: a directory bundle if `path` is an existing directory
    /// or ends with a path separator, otherwise a ZIP file (written atomically).
    pub fn save_path(&mut self, doc: &Document, path: &Path, opts: &SaveOptions) -> Result<SaveStats> {
        let s = path.to_string_lossy();
        if path.is_dir() || s.ends_with('/') || s.ends_with('\\') {
            self.save_dir(doc, path, opts)
        } else {
            let (bytes, stats) = self.save_zip(doc, opts)?;
            write_atomic(path, &bytes)?;
            Ok(stats)
        }
    }
}

#[cfg(all(test, unix))]
#[test]
fn gc_pinned_directory_handles_resist_subdirectory_swap() {
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!("pcraft-gc-swap-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let bundle = root.join("project.pcraft");
    let outside = root.join("outside");
    for sub in ["tiles", "blobs", "composite"] {
        std::fs::create_dir_all(bundle.join(sub)).unwrap();
    }
    std::fs::create_dir(&outside).unwrap();
    let filename = format!("{}.zst", "c".repeat(64));
    std::fs::write(bundle.join("tiles").join(&filename), b"stale bundle file").unwrap();
    std::fs::write(outside.join(&filename), b"outside file must survive").unwrap();

    let gc = BundleGc::new(&bundle).unwrap();
    std::fs::rename(bundle.join("tiles"), bundle.join("old-tiles")).unwrap();
    symlink(&outside, bundle.join("tiles")).unwrap();

    // Exercise the descriptor-relative unlink directly: the public save path
    // also validates the leaf, but that check itself cannot prevent a later swap.
    gc.remove(Path::new(&format!("tiles/{filename}"))).unwrap();
    assert!(!bundle.join("old-tiles").join(&filename).exists());
    assert_eq!(std::fs::read(outside.join(&filename)).unwrap(), b"outside file must survive");
    std::fs::remove_dir_all(root).unwrap();
}

fn list_objects(dir: &Path) -> Result<HashSet<String>> {
    let mut out = HashSet::new();
    for sub in ["tiles", "blobs"] {
        let object_dir = dir.join(sub);
        plain_directory_or_missing(&object_dir)?;
        if std::fs::canonicalize(&object_dir)? != object_dir {
            return Err(unsafe_bundle_entry(&object_dir));
        }
        let rd = std::fs::read_dir(&object_dir)?;
        for e in rd {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if name.strip_suffix(".zst").is_some_and(is_valid_hash) {
                let path = e.path();
                match std::fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.file_type().is_symlink() => return Err(unsafe_bundle_entry(&path)),
                    Ok(metadata) if metadata.is_file() => {
                        out.insert(format!("{sub}/{name}"));
                    }
                    // Stale directories cannot be removed with remove_file; leave them for later
                    // cleanup without failing a save of otherwise valid content.
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    Ok(out)
}

/// Crash-safe replace of one file (see [`crate::atomic`]).
pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    crate::atomic::atomic_write(path, data)?;
    Ok(())
}

#[cfg(test)]
mod preparation_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn collect(control: photocraft_raster::Interrupt<'_>, limits: PreparationLimits) -> Collect<'static, 'static, '_, 'static> {
        Collect { tiles: BTreeMap::new(), blobs: BTreeMap::new(), hash_cache: None, observer: None, control, limits, resident: 0, index: 0 }
    }
    #[test]
    fn blob_hash_is_chunked_interruptible_and_identical() {
        let bytes = Arc::new((0..200000).map(|i| (i * 37) as u8).collect::<Vec<_>>());
        let mut sink = collect(photocraft_raster::Interrupt::NONE, Default::default());
        assert_eq!(sink.blob(&bytes).unwrap(), hash_bytes(&bytes));
        let polls = AtomicUsize::new(0);
        let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= 3;
        let mut sink = collect(photocraft_raster::Interrupt::cancel_only(&cancelled), Default::default());
        assert!(matches!(sink.blob(&bytes), Err(FormatError::Cancelled)));
        assert!(sink.blobs.is_empty());
    }
    #[test]
    fn borrowed_source_registration_retains_exact_input_slice() {
        let bytes = [1u8, 2, 3, 4];
        let mut sink = Collect {
            tiles: BTreeMap::new(),
            blobs: BTreeMap::new(),
            hash_cache: None,
            observer: None,
            control: photocraft_raster::Interrupt::NONE,
            limits: Default::default(),
            resident: 0,
            index: 0,
        };
        let hash = sink.borrowed_blob(&bytes).unwrap();
        match sink.blobs.get(&hash).unwrap() {
            Object::BorrowedBlob(retained) => assert_eq!(retained.as_ptr(), bytes.as_ptr()),
            _ => panic!("source cache was duplicated"),
        }
        assert_eq!(sink.resident, 1024);
    }
    #[test]
    fn json_cancellation_occurs_inside_large_string_and_opaque_bytes() {
        let text = "literal Ω".repeat(100000);
        let opaque = vec![73u8; 100000];
        for value in [serde_json::json!(text), serde_json::json!(opaque)] {
            let polls = AtomicUsize::new(0);
            let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= 4;
            let mut sink = collect(photocraft_raster::Interrupt::cancel_only(&cancelled), Default::default());
            assert!(matches!(convert::json_bytes(&value, &mut sink, 4 << 20, false), Err(FormatError::Cancelled)));
        }
    }
    #[test]
    fn json_output_matches_existing_pretty_serialization_and_caps_before_growth() {
        let value = serde_json::json!({"literal":"Ω\n", "bits":[-0.0,1.5],"opaque":[1,2,3]});
        let mut sink = collect(photocraft_raster::Interrupt::NONE, Default::default());
        assert_eq!(convert::json_bytes(&value, &mut sink, 1 << 20, true).unwrap(), serde_json::to_vec_pretty(&value).unwrap());
        let mut sink = collect(photocraft_raster::Interrupt::NONE, PreparationLimits { resident_bytes: 0, ..Default::default() });
        assert!(matches!(convert::json_bytes(&value, &mut sink, 1 << 20, true), Err(FormatError::LimitExceeded(_))));
        assert_eq!(sink.resident, 0);
        let mut sink = collect(photocraft_raster::Interrupt::NONE, Default::default());
        assert!(matches!(convert::json_bytes(&value, &mut sink, 1, true), Err(FormatError::LimitExceeded(_))));
    }
}

#[cfg(test)]
mod bounded_decode_drafts {
    use super::*;
    #[test]
    fn decoded_size_ceiling_and_corruption_are_refused() {
        let encoded = compress(&vec![7; 32769]);
        assert!(decompress_with(&encoded, 32768, "fixture", &photocraft_raster::Interrupt::NONE).is_err());
        assert_eq!(decompress_with(&encoded, 32769, "fixture", &photocraft_raster::Interrupt::NONE).unwrap(), vec![7; 32769]);
        let truncated = encoded.get(..encoded.len().saturating_sub(1)).unwrap();
        assert!(decompress_with(truncated, 32769, "fixture", &photocraft_raster::Interrupt::NONE).is_err());
    }
    #[test]
    fn interrupted_decode_reports_cancelled_not_io() {
        let encoded = compress(&vec![11; 65536]);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 3;
        assert!(matches!(decompress_with(&encoded, 65536, "fixture", &photocraft_raster::Interrupt::cancel_only(&cancel)), Err(FormatError::Cancelled)));
    }
}
