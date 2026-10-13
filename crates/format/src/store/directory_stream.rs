//! Prepared native entries, with filesystem/publication authority entirely in
//! the caller. No garbage collection, path opens, directory swap or ZIP decode.
use super::{COMPOSITE, MANIFEST, Object, PcraftWriter, Prepared, SaveStats, THUMB};
use crate::{Document, FormatError, Result, SaveOptions};
use photocraft_color::SampleType;
use photocraft_raster::Interrupt;
use std::io::{self, Read};

/// Fixed stack scratch retained while hashing a prepared entry stream.
pub const DIRECTORY_STREAM_SCRATCH_BYTES: usize = 64 * 1024;
/// Encoded digest and decoded verification stack buffers may coexist.
pub const DIRECTORY_VERIFICATION_FIXED_BYTES: u64 = (DIRECTORY_STREAM_SCRATCH_BYTES as u64) * 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryEntryKind {
    Tile,
    Blob,
    Preview,
    Manifest,
}
#[derive(Clone, Copy, Debug)]
pub struct DirectoryEntry<'a> {
    pub name: &'a str,
    pub kind: DirectoryEntryKind,
    pub encoded_length: u64,
    /// BLAKE3 of the exact supplied encoded stream, including raw Zstd framing.
    pub encoded_hash: [u8; 32],
    /// Native canonical decoded identity, not a filename/authority heuristic.
    /// Allows a sink to validate an existing differently-compressed native object.
    pub decoded_length: u64,
    pub decoded_hash: [u8; 32],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryEntryDisposition {
    Written,
    ReusedIdentical,
}
/// Implementations must validate names against their held root, regular-file
/// identities and ALL caller source exclusions. Reuse requires exact canonical
/// content verification; different content cannot bypass final source guards.
/// Streams are read-only borrowed data, never ambient locators or grants.
pub trait DirectoryEntrySink {
    /// Caller-reported maximum simultaneously retained verification/window/path
    /// scratch, admitted before native preparation. No authority is conferred.
    fn resident_scratch_bytes(&self) -> u64 {
        0
    }

    fn publish_entry(&mut self, entry: DirectoryEntry<'_>, input: &mut dyn Read, control: &Interrupt<'_>) -> Result<DirectoryEntryDisposition>;
    /// Called last. Guard/sync/recheck/token BEFORE the final atomic rename.
    /// Success is the commit point: a late cancellation must not relabel it.
    fn commit_manifest(&mut self, entry: DirectoryEntry<'_>, input: &mut dyn Read, control: &Interrupt<'_>) -> Result<DirectoryEntryDisposition>;
}
pub struct PreparedDirectory<'doc, 'control, 'interrupt> {
    prepared: Prepared<'doc>,
    control: &'control Interrupt<'interrupt>,
    maximum: u64,
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| FormatError::Cancelled)
}
fn length() -> FormatError {
    FormatError::LimitExceeded("native directory entry length".into())
}

/// Canonical little-endian tile bytes without allocating a swapped tile buffer.
/// This uses the same sample-byte reversal as the existing native tile encoder.
struct Content<'a, 'c, 'i> {
    bytes: &'a [u8],
    sample: Option<SampleType>,
    at: usize,
    control: &'c Interrupt<'i>,
}
impl<'a, 'c, 'i> Content<'a, 'c, 'i> {
    fn bytes(bytes: &'a [u8], control: &'c Interrupt<'i>) -> Self {
        Self { bytes, sample: None, at: 0, control }
    }
    fn object(object: &'a Object<'_>, control: &'c Interrupt<'i>) -> Result<Self> {
        let (bytes, sample) = match object {
            Object::Tile(tile, sample) => (tile.bytes(), Some(*sample)),
            Object::Blob(bytes) => (bytes.as_slice(), None),
            Object::BorrowedBlob(bytes) => (*bytes, None),
        };
        if sample.is_some_and(|sample| !bytes.len().is_multiple_of(sample.bytes())) {
            return Err(length());
        }
        Ok(Self { bytes, sample, at: 0, control })
    }
}
impl Read for Content<'_, '_, '_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.control.check().map_err(|_| io::Error::other("native directory cancelled"))?;
        let count = out.len().min(64 * 1024).min(self.bytes.len().checked_sub(self.at).ok_or_else(|| io::Error::other("native directory offset"))?);
        let end = self.at.checked_add(count).ok_or_else(|| io::Error::other("native directory offset"))?;
        let output = out.get_mut(..count).ok_or_else(|| io::Error::other("native directory output"))?;
        if cfg!(target_endian = "big") && self.sample.is_some() {
            let width = self.sample.map_or(1, SampleType::bytes);
            for (i, byte) in output.iter_mut().enumerate() {
                if i.is_multiple_of(1024) {
                    self.control.check().map_err(|_| io::Error::other("native directory cancelled"))?;
                }
                let logical = self.at.checked_add(i).ok_or_else(|| io::Error::other("native directory offset"))?;
                let group = logical / width * width;
                let index = group.checked_add(width - 1 - logical % width).ok_or_else(|| io::Error::other("native directory offset"))?;
                *byte = *self.bytes.get(index).ok_or_else(|| io::Error::other("native directory tile length"))?;
            }
        } else {
            output.copy_from_slice(self.bytes.get(self.at..end).ok_or_else(|| io::Error::other("native directory length"))?);
        }
        self.control.check().map_err(|_| io::Error::other("native directory cancelled"))?;
        self.at = end;
        Ok(count)
    }
}
fn digest(mut input: impl Read, expected: u64, control: &Interrupt<'_>) -> Result<[u8; 32]> {
    let mut hash = blake3::Hasher::new();
    let mut bytes = [0u8; DIRECTORY_STREAM_SCRATCH_BYTES];
    let mut total = 0u64;
    loop {
        poll(control)?;
        let read = input.read(&mut bytes);
        poll(control)?;
        let count = match read {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            other => other?,
        };
        if count == 0 {
            break;
        }
        total = total.checked_add(count as u64).filter(|n| *n <= expected).ok_or_else(length)?;
        hash.update(bytes.get(..count).ok_or_else(length)?);
    }
    if total != expected {
        return Err(length());
    }
    poll(control)?;
    Ok(*hash.finalize().as_bytes())
}
impl PcraftWriter {
    /// Preparation is the existing bounded native manifest/object collection.
    /// Caller admits retained input and preview preparation separately. No file
    /// is created and no directory authority is retained by this plan.
    pub fn prepare_directory_stream<'doc, 'control, 'interrupt>(
        &mut self,
        document: &'doc Document,
        options: &SaveOptions,
        max_object_bytes: usize,
        control: &'control Interrupt<'interrupt>,
    ) -> Result<PreparedDirectory<'doc, 'control, 'interrupt>> {
        poll(control)?;
        if max_object_bytes > self.preparation_limits.object_bytes {
            return Err(FormatError::LimitExceeded("native directory object limit may only be lowered".into()));
        }
        // Leave actual fixed hashing scratch within the selected preparation
        // envelope. Restore writer settings even when preparation refuses.
        let saved_limits = self.preparation_limits;
        let remaining = saved_limits.resident_bytes.checked_sub(DIRECTORY_STREAM_SCRATCH_BYTES).ok_or_else(length)?;
        self.preparation_limits.resident_bytes = remaining;
        let prepared = self.prepare(document, options, *control, Some(max_object_bytes), None);
        self.preparation_limits = saved_limits;
        let prepared = prepared?;
        let maximum = max_object_bytes as u64;
        for bytes in std::iter::once(&prepared.manifest).chain(prepared.previews.iter().map(|(_, bytes)| bytes)) {
            poll(control)?;
            if bytes.len() as u64 > maximum {
                return Err(length());
            }
        }
        for (name, object) in &prepared.objects {
            poll(control)?;
            native_identity(name)?;
            let content = Content::object(object, control)?;
            if content.bytes.len() as u64 > maximum || crate::zstd_stream::encoded_len(content.bytes.len() as u64)? > maximum {
                return Err(length());
            }
        }
        poll(control)?;
        Ok(PreparedDirectory { prepared, control, maximum })
    }
}
fn native_identity(name: &str) -> Result<(DirectoryEntryKind, [u8; 32])> {
    let (kind, hash) = if let Some(hash) = name.strip_prefix("tiles/") {
        (DirectoryEntryKind::Tile, hash)
    } else if let Some(hash) = name.strip_prefix("blobs/") {
        (DirectoryEntryKind::Blob, hash)
    } else {
        return Err(FormatError::corrupt("native directory entry name"));
    };
    let hash = hash
        .strip_suffix(".zst")
        .filter(|hash| crate::convert::is_valid_hash(hash))
        .ok_or_else(|| FormatError::corrupt("native directory content identity"))?;
    let parsed = blake3::Hash::from_hex(hash).map_err(|_| FormatError::corrupt("native directory content identity"))?;
    Ok((kind, *parsed.as_bytes()))
}
impl PreparedDirectory<'_, '_, '_> {
    pub fn write(self, sink: &mut impl DirectoryEntrySink) -> Result<SaveStats> {
        poll(self.control)?;
        let mut stats = self.prepared.stats;
        for (name, object) in &self.prepared.objects {
            poll(self.control)?;
            let (kind, decoded_hash) = native_identity(name)?;
            let raw = Content::object(object, self.control)?;
            let decoded_length = raw.bytes.len() as u64;
            let encoded_length = crate::zstd_stream::encoded_len(decoded_length)?;
            if encoded_length > self.maximum {
                return Err(length());
            }
            let encoded_hash = digest(crate::zstd_stream::RawFrame::from_reader(raw, decoded_length)?, encoded_length, self.control)?;
            let entry = DirectoryEntry { name, kind, encoded_length, encoded_hash, decoded_length, decoded_hash };
            let mut input = crate::zstd_stream::RawFrame::from_reader(Content::object(object, self.control)?, decoded_length)?;
            let disposition = sink.publish_entry(entry, &mut input, self.control)?;
            poll(self.control)?;
            match (kind, disposition) {
                (DirectoryEntryKind::Tile, DirectoryEntryDisposition::Written) => {
                    stats.tiles_written = stats.tiles_written.checked_add(1).ok_or_else(length)?
                }
                (DirectoryEntryKind::Tile, DirectoryEntryDisposition::ReusedIdentical) => {
                    stats.tiles_reused = stats.tiles_reused.checked_add(1).ok_or_else(length)?
                }
                (DirectoryEntryKind::Blob, DirectoryEntryDisposition::Written) => {
                    stats.blobs_written = stats.blobs_written.checked_add(1).ok_or_else(length)?
                }
                _ => {}
            }
        }
        for (name, bytes) in &self.prepared.previews {
            poll(self.control)?;
            if !matches!(*name, THUMB | COMPOSITE) {
                return Err(FormatError::corrupt("native directory preview name"));
            }
            let hash = digest(Content::bytes(bytes, self.control), bytes.len() as u64, self.control)?;
            let entry = DirectoryEntry {
                name,
                kind: DirectoryEntryKind::Preview,
                encoded_length: bytes.len() as u64,
                encoded_hash: hash,
                decoded_length: bytes.len() as u64,
                decoded_hash: hash,
            };
            sink.publish_entry(entry, &mut Content::bytes(bytes, self.control), self.control)?;
            poll(self.control)?;
        }
        let bytes = &self.prepared.manifest;
        let hash = digest(Content::bytes(bytes, self.control), bytes.len() as u64, self.control)?;
        let entry = DirectoryEntry {
            name: MANIFEST,
            kind: DirectoryEntryKind::Manifest,
            encoded_length: bytes.len() as u64,
            encoded_hash: hash,
            decoded_length: bytes.len() as u64,
            decoded_hash: hash,
        };
        poll(self.control)?;
        sink.commit_manifest(entry, &mut Content::bytes(bytes, self.control), self.control)?;
        // Manifest commit is success: no token/error work follows publication.
        Ok(stats)
    }
}

/// Lower-only verification caps for existing native entries. Decoder internal
/// allocation remains ruzstd's bounded legacy allocator, not an RSS guarantee.
#[derive(Clone, Copy, Debug)]
pub struct DirectoryVerificationLimits {
    pub encoded_bytes: u64,
    pub decoded_bytes: u64,
    pub window_bytes: u64,
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for DirectoryVerificationLimits {
    fn default() -> Self {
        let object = super::PreparationLimits::default().object_bytes as u64;
        Self {
            encoded_bytes: object,
            decoded_bytes: object,
            window_bytes: ruzstd::decoding::DEFAULT_MAX_WINDOW_SIZE,
            resident_bytes: photocraft_raster::MAX_CHECKED_RASTER_BYTES,
            // Two encoded passes, decoded bytes and bounded read bookkeeping.
            work_units: object.saturating_mul(8).saturating_add(1024 * 1024),
        }
    }
}
struct VerifyWork {
    used: std::cell::Cell<u64>,
    denied: std::cell::Cell<bool>,
    maximum: u64,
}
impl VerifyWork {
    fn charge(&self, units: u64) -> io::Result<()> {
        let Some(next) = self.used.get().checked_add(units).filter(|n| *n <= self.maximum) else {
            self.denied.set(true);
            return Err(io::Error::other("native directory verification work limit"));
        };
        self.used.set(next);
        Ok(())
    }
    fn io_error(&self, error: io::Error, control: &Interrupt<'_>) -> FormatError {
        if control.cancelled() {
            FormatError::Cancelled
        } else if self.denied.get() {
            FormatError::LimitExceeded("native directory verification work".into())
        } else {
            FormatError::Io(error)
        }
    }
    fn error(&self, error: impl std::fmt::Display, control: &Interrupt<'_>) -> FormatError {
        if control.cancelled() {
            FormatError::Cancelled
        } else if self.denied.get() {
            FormatError::LimitExceeded("native directory verification work".into())
        } else {
            FormatError::corrupt(format!("native directory verification: {error}"))
        }
    }
}
struct VerifyInput<'a, 'c, 'i, R> {
    reader: &'a mut R,
    at: u64,
    length: u64,
    work: &'a VerifyWork,
    control: &'c Interrupt<'i>,
}
impl<R: Read> Read for VerifyInput<'_, '_, '_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.control.check().map_err(|_| io::Error::other("native directory verification cancelled"))?;
        self.work.charge(1)?;
        let remaining = self.length.checked_sub(self.at).ok_or_else(|| io::Error::other("native directory verification offset"))?;
        let count = output.len().min(64 * 1024).min(usize::try_from(remaining).unwrap_or(usize::MAX));
        if count == 0 {
            return Ok(0);
        }
        let result = self.reader.read(output.get_mut(..count).ok_or_else(|| io::Error::other("native directory verification range"))?);
        self.control.check().map_err(|_| io::Error::other("native directory verification cancelled"))?;
        let count = result?;
        if count as u64 > remaining || count > output.len().min(64 * 1024) {
            return Err(io::Error::other("native directory verification read count"));
        }
        self.work.charge(count as u64)?;
        self.at = self.at.checked_add(count as u64).ok_or_else(|| io::Error::other("native directory verification offset"))?;
        Ok(count)
    }
}
/// Verify a caller-opened regular entry. True allows reuse WITHOUT mutation;
/// it never authorizes overwriting a protected entry. Original cursor restored.
pub fn verify_existing_directory_entry<R: Read + io::Seek>(entry: &DirectoryEntry<'_>, reader: &mut R, control: &Interrupt<'_>) -> Result<bool> {
    verify_existing_directory_entry_with_limits(entry, reader, DirectoryVerificationLimits::default(), control)
}
pub fn verify_existing_directory_entry_with_limits<R: Read + io::Seek>(
    entry: &DirectoryEntry<'_>,
    reader: &mut R,
    limits: DirectoryVerificationLimits,
    control: &Interrupt<'_>,
) -> Result<bool> {
    poll(control)?;
    let ceiling = DirectoryVerificationLimits::default();
    if limits.encoded_bytes > ceiling.encoded_bytes
        || limits.decoded_bytes > ceiling.decoded_bytes
        || limits.window_bytes > ceiling.window_bytes
        || limits.resident_bytes > ceiling.resident_bytes
        || limits.work_units > ceiling.work_units
        || limits.window_bytes.checked_add(DIRECTORY_VERIFICATION_FIXED_BYTES).is_none_or(|n| n > limits.resident_bytes)
        || entry.encoded_length > limits.encoded_bytes
        || entry.decoded_length > limits.decoded_bytes
    {
        return Err(length());
    }
    match entry.kind {
        DirectoryEntryKind::Tile | DirectoryEntryKind::Blob => {
            let (kind, hash) = native_identity(entry.name)?;
            if kind != entry.kind || hash != entry.decoded_hash {
                return Err(FormatError::corrupt("native directory entry identity"));
            }
        }
        DirectoryEntryKind::Preview if matches!(entry.name, THUMB | COMPOSITE) => {}
        DirectoryEntryKind::Manifest if entry.name == MANIFEST => {}
        _ => return Err(FormatError::corrupt("native directory entry name")),
    }
    let original = reader.stream_position()?;
    poll(control)?;
    let work = VerifyWork { used: std::cell::Cell::new(0), denied: std::cell::Cell::new(false), maximum: limits.work_units };
    let result = (|| -> Result<bool> {
        let actual = reader.seek(io::SeekFrom::End(0))?;
        poll(control)?;
        if actual > limits.encoded_bytes {
            return Err(length());
        }
        reader.seek(io::SeekFrom::Start(0))?;
        poll(control)?;
        let encoded =
            digest(VerifyInput { reader, at: 0, length: actual, work: &work, control }, actual, control).map_err(|error| work.error(error, control))?;
        if actual == entry.encoded_length && encoded == entry.encoded_hash {
            let end = reader.seek(io::SeekFrom::End(0))?;
            poll(control)?;
            return Ok(end == actual);
        }
        if !matches!(entry.kind, DirectoryEntryKind::Tile | DirectoryEntryKind::Blob) {
            return Ok(false);
        }
        reader.seek(io::SeekFrom::Start(0))?;
        poll(control)?;
        let source = VerifyInput { reader, at: 0, length: actual, work: &work, control };
        let mut decoder =
            ruzstd::decoding::StreamingDecoder::new_with_max_window_size(source, limits.window_bytes).map_err(|error| work.error(error, control))?;
        poll(control)?;
        let mut scratch = [0u8; DIRECTORY_STREAM_SCRATCH_BYTES];
        let mut decoded = 0u64;
        let mut hash = blake3::Hasher::new();
        loop {
            poll(control)?;
            work.charge(1).map_err(|error| work.error(error, control))?;
            let remaining = entry.decoded_length.checked_sub(decoded).ok_or_else(length)?;
            let offered = scratch.len().min(usize::try_from(remaining.saturating_add(1)).unwrap_or(usize::MAX));
            let result = decoder.read(scratch.get_mut(..offered).ok_or_else(length)?);
            poll(control)?;
            let count = match result {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                other => other.map_err(|error| work.error(error, control))?,
            };
            if count == 0 {
                break;
            }
            work.charge(count as u64).map_err(|error| work.error(error, control))?;
            decoded = decoded.checked_add(count as u64).ok_or_else(length)?;
            if decoded > entry.decoded_length {
                return Ok(false);
            }
            hash.update(scratch.get(..count).ok_or_else(length)?);
        }
        if decoded != entry.decoded_length || !decoder.decoder.is_finished() || decoder.decoder.can_collect() != 0 {
            return Ok(false);
        }
        if let Some(expected) = decoder.decoder.get_checksum_from_data()
            && decoder.decoder.get_calculated_checksum() != Some(expected)
        {
            return Ok(false);
        }
        let consumed = decoder.decoder.bytes_read_from_source();
        let (source, _) = decoder.into_parts();
        if source.at != actual || consumed != actual {
            return Ok(false);
        }
        let matched = *hash.finalize().as_bytes() == entry.decoded_hash;
        let end = reader.seek(io::SeekFrom::End(0))?;
        poll(control)?;
        Ok(matched && end == actual)
    })();
    // Restore even on cancellation/false; restoration must not hide cancellation.
    let restored = reader.seek(io::SeekFrom::Start(original));
    if matches!(result, Err(FormatError::Cancelled)) {
        return result;
    }
    restored?;
    poll(control)?;
    result
}

/// Copy one prepared encoded entry into a caller-owned UNPUBLISHED temporary.
/// Exact length/digest validation completes before success. This neither syncs
/// nor renames output and never authorizes mutation of a protected destination.
pub fn copy_directory_entry_stream(entry: &DirectoryEntry<'_>, input: &mut dyn Read, output: &mut impl io::Write, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let limits = DirectoryVerificationLimits::default();
    if entry.encoded_length > limits.encoded_bytes {
        return Err(length());
    }
    let work = VerifyWork { used: std::cell::Cell::new(0), denied: std::cell::Cell::new(false), maximum: limits.work_units };
    let mut scratch = [0u8; DIRECTORY_STREAM_SCRATCH_BYTES];
    let mut copied = 0u64;
    let mut hash = blake3::Hasher::new();
    loop {
        poll(control)?;
        work.charge(1).map_err(|error| work.io_error(error, control))?;
        let remaining = entry.encoded_length.checked_sub(copied).ok_or_else(length)?;
        let offered = scratch.len().min(usize::try_from(remaining.saturating_add(1)).unwrap_or(usize::MAX));
        let result = input.read(scratch.get_mut(..offered).ok_or_else(length)?);
        poll(control)?;
        let count = match result {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            other => other.map_err(|error| work.io_error(error, control))?,
        };
        if count > offered {
            return Err(FormatError::corrupt("native directory copy read count"));
        }
        if count == 0 {
            break;
        }
        work.charge(count as u64).map_err(|error| work.io_error(error, control))?;
        copied = copied.checked_add(count as u64).filter(|n| *n <= entry.encoded_length).ok_or_else(length)?;
        let bytes = scratch.get(..count).ok_or_else(length)?;
        hash.update(bytes);
        let mut written = 0usize;
        while written < count {
            poll(control)?;
            work.charge(1).map_err(|error| work.io_error(error, control))?;
            let remaining = bytes.get(written..).ok_or_else(length)?;
            let result = output.write(remaining);
            poll(control)?;
            let amount = match result {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(work.io_error(error, control)),
                Ok(0) => return Err(FormatError::Io(io::Error::new(io::ErrorKind::WriteZero, "failed to write whole native directory entry"))),
                Ok(amount) if amount > remaining.len() => return Err(FormatError::corrupt("native directory copy write count")),
                Ok(amount) => amount,
            };
            work.charge(amount as u64).map_err(|error| work.io_error(error, control))?;
            written = written.checked_add(amount).ok_or_else(length)?;
        }
    }
    if copied != entry.encoded_length {
        return Err(FormatError::Io(io::Error::new(io::ErrorKind::UnexpectedEof, "native directory entry ended before declared length")));
    }
    if *hash.finalize().as_bytes() != entry.encoded_hash {
        return Err(FormatError::corrupt("native directory encoded content hash"));
    }
    poll(control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Sink<'a> {
        entries: BTreeMap<String, Vec<u8>>,
        order: Vec<String>,
        writes: usize,
        cancel: Option<&'a AtomicBool>,
    }
    impl Sink<'_> {
        fn accept(&mut self, entry: DirectoryEntry<'_>, input: &mut dyn Read) -> Result<DirectoryEntryDisposition> {
            let mut bytes = Vec::new();
            input.read_to_end(&mut bytes)?;
            assert_eq!(bytes.len() as u64, entry.encoded_length);
            assert_eq!(*blake3::hash(&bytes).as_bytes(), entry.encoded_hash);
            let decoded = if matches!(entry.kind, DirectoryEntryKind::Tile | DirectoryEntryKind::Blob) {
                super::super::decompress_with(&bytes, entry.decoded_length as usize, "own directory entry", &Interrupt::NONE)?
            } else {
                bytes.clone()
            };
            assert_eq!(decoded.len() as u64, entry.decoded_length);
            assert_eq!(*blake3::hash(&decoded).as_bytes(), entry.decoded_hash);
            self.order.push(entry.name.to_string());
            if let Some(existing) = self.entries.get(entry.name) {
                if existing == &bytes {
                    return Ok(DirectoryEntryDisposition::ReusedIdentical);
                }
                // Model a protected existing entry: no mutation on mismatch.
                return Err(FormatError::Unsupported("own protected entry differs".into()));
            }
            self.entries.insert(entry.name.to_string(), bytes);
            self.writes += 1;
            Ok(DirectoryEntryDisposition::Written)
        }
    }
    impl DirectoryEntrySink for Sink<'_> {
        fn publish_entry(&mut self, entry: DirectoryEntry<'_>, input: &mut dyn Read, _: &Interrupt<'_>) -> Result<DirectoryEntryDisposition> {
            assert_ne!(entry.kind, DirectoryEntryKind::Manifest);
            let result = self.accept(entry, input)?;
            if let Some(cancel) = self.cancel {
                cancel.store(true, Ordering::Relaxed);
            }
            Ok(result)
        }
        fn commit_manifest(&mut self, entry: DirectoryEntry<'_>, input: &mut dyn Read, _: &Interrupt<'_>) -> Result<DirectoryEntryDisposition> {
            assert_eq!(entry.name, MANIFEST);
            assert_eq!(entry.kind, DirectoryEntryKind::Manifest);
            self.accept(entry, input)
        }
    }
    fn document() -> Document {
        let mut doc =
            Document::new("Own directory fixture", photocraft_geom::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let mut surface = photocraft_raster::Surface::new(doc.pixel_format());
        surface.try_write_interleaved(photocraft_geom::Rect::new(0, 0, 1, 1), &[29, 71, 139, 255]).unwrap();
        doc.layers.push(photocraft_doc::Layer::new("Pixels", photocraft_doc::LayerContent::Raster(surface)));
        // An inert blob containing source-like bytes exercises decoded/encoded
        // identity, not ICC validity or source renderer qualification.
        doc.icc_profile = Some(std::sync::Arc::new(b"gimp xcf v013\0owned inert blob".to_vec()));
        doc
    }
    fn sink() -> Sink<'static> {
        Sink { entries: BTreeMap::new(), order: Vec::new(), writes: 0, cancel: None }
    }
    #[test]
    fn objects_stream_with_exact_identity_manifest_last_and_repeat_reuses() {
        let doc = document();
        let original = crate::save_to_bytes(&doc, &Default::default()).unwrap();
        let mut writer = PcraftWriter::new();
        let mut sink = sink();
        let maximum = writer.preparation_limits().object_bytes;
        writer.prepare_directory_stream(&doc, &Default::default(), maximum, &Interrupt::NONE).unwrap().write(&mut sink).unwrap();
        assert_eq!(sink.order.last().map(String::as_str), Some(MANIFEST));
        assert!(sink.order.iter().any(|name| name.starts_with("blobs/")));
        let entries = sink.entries.clone();
        let writes = sink.writes;
        sink.order.clear();
        let stats = writer.prepare_directory_stream(&doc, &Default::default(), maximum, &Interrupt::NONE).unwrap().write(&mut sink).unwrap();
        assert_eq!(sink.entries, entries);
        assert_eq!(sink.writes, writes);
        assert_eq!(stats.tiles_written, 0);
        assert!(stats.tiles_reused > 0);
        assert_eq!(sink.order.last().map(String::as_str), Some(MANIFEST));
        assert_eq!(crate::save_to_bytes(&doc, &Default::default()).unwrap(), original);
    }
    #[test]
    fn cancellation_after_object_never_reaches_manifest_commit() {
        let doc = document();
        let cancelled = AtomicBool::new(false);
        let check = || cancelled.load(Ordering::Relaxed);
        let control = Interrupt::cancel_only(&check);
        let mut writer = PcraftWriter::new();
        let maximum = writer.preparation_limits().object_bytes;
        let mut sink = Sink { entries: BTreeMap::new(), order: Vec::new(), writes: 0, cancel: Some(&cancelled) };
        let result = writer.prepare_directory_stream(&doc, &Default::default(), maximum, &control).unwrap().write(&mut sink);
        assert!(matches!(result, Err(FormatError::Cancelled)));
        assert_eq!(sink.order.len(), 1);
        assert!(!sink.entries.contains_key(MANIFEST));
    }
    #[test]
    fn existing_different_entry_refuses_without_replacement_or_manifest_change() {
        let doc = document();
        let mut writer = PcraftWriter::new();
        let maximum = writer.preparation_limits().object_bytes;
        let mut sink = sink();
        writer.prepare_directory_stream(&doc, &Default::default(), maximum, &Interrupt::NONE).unwrap().write(&mut sink).unwrap();
        let name = sink.entries.keys().find(|name| name.starts_with("blobs/")).unwrap().clone();
        sink.entries.insert(name, b"own protected different contents".to_vec());
        let before = sink.entries.clone();
        assert!(writer.prepare_directory_stream(&doc, &Default::default(), maximum, &Interrupt::NONE).unwrap().write(&mut sink).is_err());
        assert_eq!(sink.entries, before);
    }
    #[test]
    fn legacy_compression_verifies_canonical_content_and_restores_cursor() {
        let raw = b"owned source-like blob bytes".repeat(4000);
        let name = format!("blobs/{}.zst", blake3::hash(&raw).to_hex());
        let encoded_length = crate::zstd_stream::encoded_len(raw.len() as u64).unwrap();
        let encoded_hash = digest(crate::zstd_stream::RawFrame::new(&raw).unwrap(), encoded_length, &Interrupt::NONE).unwrap();
        let entry = DirectoryEntry {
            name: &name,
            kind: DirectoryEntryKind::Blob,
            encoded_length,
            encoded_hash,
            decoded_length: raw.len() as u64,
            decoded_hash: *blake3::hash(&raw).as_bytes(),
        };
        let legacy = ruzstd::encoding::compress_to_vec(&raw, ruzstd::encoding::CompressionLevel::Fastest);
        let mut reader = io::Cursor::new(legacy.clone());
        reader.set_position(7);
        assert!(verify_existing_directory_entry(&entry, &mut reader, &Interrupt::NONE).unwrap());
        assert_eq!(reader.position(), 7);
        for trailing in [b"owned trailing bytes".to_vec(), legacy.clone()] {
            let mut bytes = legacy.clone();
            bytes.extend(trailing);
            let mut reader = io::Cursor::new(bytes);
            reader.set_position(3);
            assert!(!verify_existing_directory_entry(&entry, &mut reader, &Interrupt::NONE).unwrap());
            assert_eq!(reader.position(), 3);
        }
        let changed = ruzstd::encoding::compress_to_vec(&vec![17; raw.len()], ruzstd::encoding::CompressionLevel::Fastest);
        assert!(!verify_existing_directory_entry(&entry, &mut io::Cursor::new(changed), &Interrupt::NONE).unwrap());
        let mut truncated = legacy;
        truncated.pop();
        assert!(verify_existing_directory_entry(&entry, &mut io::Cursor::new(truncated), &Interrupt::NONE).is_err());
    }
    #[test]
    fn verification_cancellation_and_lowered_work_refuse_without_mutation() {
        let raw = vec![31u8; 100000];
        let name = format!("blobs/{}.zst", blake3::hash(&raw).to_hex());
        let length = crate::zstd_stream::encoded_len(raw.len() as u64).unwrap();
        let hash = digest(crate::zstd_stream::RawFrame::new(&raw).unwrap(), length, &Interrupt::NONE).unwrap();
        let entry = DirectoryEntry {
            name: &name,
            kind: DirectoryEntryKind::Blob,
            encoded_length: length,
            encoded_hash: hash,
            decoded_length: raw.len() as u64,
            decoded_hash: *blake3::hash(&raw).as_bytes(),
        };
        let legacy = ruzstd::encoding::compress_to_vec(&raw, ruzstd::encoding::CompressionLevel::Fastest);
        let mut reader = io::Cursor::new(legacy.clone());
        reader.set_position(2);
        let limits = DirectoryVerificationLimits { work_units: 0, ..Default::default() };
        assert!(matches!(verify_existing_directory_entry_with_limits(&entry, &mut reader, limits, &Interrupt::NONE), Err(FormatError::LimitExceeded(_))));
        assert_eq!(reader.position(), 2);
        assert_eq!(reader.get_ref(), &legacy);
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= 5;
        assert!(matches!(verify_existing_directory_entry(&entry, &mut reader, &Interrupt::cancel_only(&cancelled)), Err(FormatError::Cancelled)));
        assert_eq!(reader.position(), 2);
        assert_eq!(reader.get_ref(), &legacy);
    }
    #[test]
    fn copy_checks_hash_length_partial_writes_and_writezero() {
        struct Short {
            bytes: Vec<u8>,
            interrupt: bool,
        }
        impl io::Write for Short {
            fn write(&mut self, input: &[u8]) -> io::Result<usize> {
                if self.interrupt {
                    self.interrupt = false;
                    return Err(io::Error::from(io::ErrorKind::Interrupted));
                }
                if let Some(byte) = input.first() {
                    self.bytes.push(*byte);
                    Ok(1)
                } else {
                    Ok(0)
                }
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let bytes = b"owned copy";
        let entry = DirectoryEntry {
            name: THUMB,
            kind: DirectoryEntryKind::Preview,
            encoded_length: bytes.len() as u64,
            encoded_hash: *blake3::hash(bytes).as_bytes(),
            decoded_length: bytes.len() as u64,
            decoded_hash: *blake3::hash(bytes).as_bytes(),
        };
        let mut output = Short { bytes: Vec::new(), interrupt: true };
        copy_directory_entry_stream(&entry, &mut bytes.as_slice(), &mut output, &Interrupt::NONE).unwrap();
        assert_eq!(output.bytes, bytes);
        assert!(
            matches!(copy_directory_entry_stream(&entry,&mut b"short".as_slice(),&mut Vec::new(),&Interrupt::NONE),Err(FormatError::Io(error)) if error.kind()==io::ErrorKind::UnexpectedEof)
        );
        let wrong = DirectoryEntry { encoded_hash: [0; 32], ..entry };
        assert!(matches!(copy_directory_entry_stream(&wrong, &mut bytes.as_slice(), &mut Vec::new(), &Interrupt::NONE), Err(FormatError::Corrupt(_))));
        struct Zero;
        impl io::Write for Zero {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Ok(0)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(
            matches!(copy_directory_entry_stream(&entry,&mut bytes.as_slice(),&mut Zero,&Interrupt::NONE),Err(FormatError::Io(error)) if error.kind()==io::ErrorKind::WriteZero)
        );
    }
}
