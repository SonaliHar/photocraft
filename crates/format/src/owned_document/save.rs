//! Optional accounted save bridge, using the existing document field mapper.
//! Logical conservative metadata envelopes are prepaid and retained throughout
//! preparation. This is not an allocator-header/RSS census or atomic file save.
use super::*;
use crate::{
    convert::Sink,
    manifest::{Hash, Manifest},
    zstd_stream::RawFrame,
};
use accounting::ChargedVec;
use photocraft_color::{PixelFormat, SampleType};
use photocraft_raster::{Interrupt, Tile, backing::quota::QuotaKind};
use std::io::{Read, Seek, Write};

enum Object<'a> {
    Tile(Arc<Tile>, SampleType),
    Blob(Arc<Vec<u8>>),
    Borrowed(&'a [u8]),
}
impl Object<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Tile(t, _) => t.bytes(),
            Self::Blob(b) => b,
            Self::Borrowed(b) => b,
        }
    }
    fn sample(&self) -> Option<SampleType> {
        match self {
            Self::Tile(_, s) => Some(*s),
            _ => None,
        }
    }
    fn tile(&self) -> bool {
        matches!(self, Self::Tile(..))
    }
}
struct Entry<'a> {
    hash: [u8; 32],
    object: Object<'a>,
}
struct OwnedSink<'a, 'c, 'i> {
    entries: ChargedVec<Entry<'a>>,
    finite: Option<super::finite_catalog::OwnedFiniteCatalog>,
    reservations: RetainedReservations,
    limits: OwnedDocumentLimits,
    control: &'c ReadControl<'i>,
}
/// Canonical sample transport, without a whole-tile endian conversion buffer.
struct CanonicalBytes<'a, 'c, 'i> {
    bytes: &'a [u8],
    sample: Option<SampleType>,
    position: usize,
    control: &'c ReadControl<'i>,
}
impl Read for CanonicalBytes<'_, '_, '_> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let count = output.len().min(4096).min(self.bytes.len().saturating_sub(self.position));
        self.control.check(count as u64).map_err(std::io::Error::other)?;
        let end = self.position.checked_add(count).ok_or_else(|| std::io::Error::other("canonical range"))?;
        let width = self.sample.map_or(1, |sample| sample.bytes());
        for (index, byte) in output.iter_mut().take(count).enumerate() {
            let native = self.position.checked_add(index).ok_or_else(|| std::io::Error::other("canonical offset"))?;
            let source = if cfg!(target_endian = "big") && width > 1 {
                (native / width)
                    .checked_mul(width)
                    .and_then(|base| base.checked_add(width - 1 - native % width))
                    .ok_or_else(|| std::io::Error::other("canonical endian"))?
            } else {
                native
            };
            *byte = *self.bytes.get(source).ok_or_else(|| std::io::Error::other("canonical byte"))?;
        }
        self.control.check(0).map_err(std::io::Error::other)?;
        self.position = end;
        Ok(count)
    }
}
impl OwnedSink<'_, '_, '_> {
    fn register(&mut self, object: Object<'_>) -> Result<([u8; 32], String)> {
        if object.bytes().len() > self.limits.max_object_bytes {
            return Err(FormatError::LimitExceeded("owned save object bytes".into()));
        }
        // Caller retains this lease until the stack hasher and buffer are gone.
        let quota = self.reservations.quota();
        let _scratch = quota.reserve(QuotaKind::Resident, (4096 + std::mem::size_of::<blake3::Hasher>()) as u64).map_err(|_| error(BackingError::Quota))?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0_u8; 4096];
        let mut reader = CanonicalBytes { bytes: object.bytes(), sample: object.sample(), position: 0, control: self.control };
        loop {
            let count = reader.read(&mut buffer).map_err(|failure| {
                if let Some(backing) = failure.get_ref().and_then(|inner| inner.downcast_ref::<BackingError>()) {
                    error(*backing)
                } else {
                    FormatError::Io(failure)
                }
            })?;
            if count == 0 {
                break;
            }
            hasher.update(buffer.get(..count).ok_or_else(|| error(BackingError::Length))?);
        }
        let hash = *hasher.finalize().as_bytes();
        self.reserve(80)?;
        let mut string = String::new();
        string.try_reserve_exact(64).map_err(|_| error(BackingError::Allocation))?;
        if string.capacity() > 80 {
            return Err(error(BackingError::Allocation));
        }
        const HEX: &[u8] = b"0123456789abcdef";
        for byte in hash {
            string.push(char::from(*HEX.get((byte >> 4) as usize).ok_or_else(|| error(BackingError::Length))?));
            string.push(char::from(*HEX.get((byte & 15) as usize).ok_or_else(|| error(BackingError::Length))?));
        }
        self.poll()?;
        Ok((hash, string))
    }
    fn contains(&self, hash: [u8; 32], tile: bool) -> Result<bool> {
        for entry in self.entries.as_slice() {
            self.control.check(1).map_err(error)?;
            if entry.hash == hash && entry.object.tile() == tile {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
impl<'a> Sink<'a> for OwnedSink<'a, '_, '_> {
    fn finite_raster(&mut self, layer: &photocraft_doc::BackedLayer) -> Result<crate::finite_raster::FiniteRasterM> {
        if self.finite.is_none() {
            self.finite = Some(super::finite_catalog::OwnedFiniteCatalog::new(
                self.reservations.quota().clone(),
                self.limits.max_reservations,
                self.limits.max_layers,
                self.limits.max_finite_logical_bytes,
                self.control,
            )?);
        }
        self.finite.as_mut().ok_or_else(|| error(BackingError::Integrity))?.capture_finite(layer, self.control)
    }
    fn poll(&self) -> Result<()> {
        self.control.check(1).map_err(error)
    }
    fn reserve(&mut self, bytes: usize) -> Result<()> {
        // Conservative simultaneous old/new logical capacity plus small owner
        // envelope. No refund is issued until all staged model buffers drop.
        let envelope = bytes.checked_mul(2).and_then(|n| n.checked_add(128)).ok_or_else(|| error(BackingError::Length))?;
        self.reservations.reserve(envelope as u64, self.control).map_err(error)
    }
    fn release(&mut self, _bytes: usize) { /* conservative retention through preparation */
    }
    fn max_manifest_bytes(&self) -> usize {
        self.limits.max_manifest_bytes
    }
    fn control(&self) -> Interrupt<'_> {
        *self.control.interrupt
    }
    fn tile(&mut self, format: PixelFormat, tile: &Arc<Tile>) -> Result<Hash> {
        let (hash, string) = self.register(Object::Tile(tile.clone(), format.sample))?;
        if !self.contains(hash, true)? {
            self.entries.push(Entry { hash, object: Object::Tile(tile.clone(), format.sample) }, self.control).map_err(error)?;
        }
        Ok(string)
    }
    fn blob(&mut self, bytes: &Arc<Vec<u8>>) -> Result<Hash> {
        let (hash, string) = self.register(Object::Blob(bytes.clone()))?;
        if !self.contains(hash, false)? {
            self.entries.push(Entry { hash, object: Object::Blob(bytes.clone()) }, self.control).map_err(error)?;
        }
        Ok(string)
    }
    fn borrowed_blob(&mut self, bytes: &'a [u8]) -> Result<Hash> {
        let (hash, string) = self.register(Object::Borrowed(bytes))?;
        if !self.contains(hash, false)? {
            self.entries.push(Entry { hash, object: Object::Borrowed(bytes) }, self.control).map_err(error)?;
        }
        Ok(string)
    }
}
impl OwnedDocumentArchive {
    /// Same-ledger save of the retained current model; output storage is external.
    pub fn save_archive<W: Write + Seek>(&self, output: W, limits: OwnedDocumentLimits, control: &ReadControl<'_>) -> Result<W> {
        save_current_document_archive(&self.document, output, limits, self._model.quota(), control)
    }
}
/// Additional preparation is accounted here. The borrowed input model and
/// output storage must already be caller-accounted on the supplied ledger.
/// Streams current ordinary objects through the existing manifest mapper.
/// Output storage remains caller-owned/accounted; failure can leave partial
/// uncommitted bytes. No default save routing or atomic destination claim.
pub fn save_current_document_archive<W: Write + Seek>(
    doc: &Document,
    output: W,
    limits: OwnedDocumentLimits,
    quota: &BackingQuota,
    control: &ReadControl<'_>,
) -> Result<W> {
    limits.validate()?;
    control.check(0).map_err(error)?;

    fn count_layers(layers: &[photocraft_doc::Layer], depth: usize, total: &mut usize, limit: usize, control: &ReadControl<'_>) -> Result<()> {
        if depth > crate::MAX_GROUP_DEPTH && !layers.is_empty() {
            return Err(FormatError::LimitExceeded("owned document layer depth".into()));
        }
        for layer in layers {
            control.check(1).map_err(error)?;
            *total = total.checked_add(1).ok_or_else(|| FormatError::LimitExceeded("owned layer count".into()))?;
            if *total > limit {
                return Err(FormatError::LimitExceeded("owned document layer count".into()));
            }
            if let photocraft_doc::LayerContent::Group(group) = &layer.content {
                count_layers(&group.children, depth + 1, total, limit, control)?;
            }
        }
        Ok(())
    }
    count_layers(&doc.layers, 0, &mut 0, limits.max_layers, control)?;
    let reservations = RetainedReservations::new(limits.max_reservations, quota.clone(), control).map_err(error)?;
    let entries = ChargedVec::with_capacity(limits.max_reservations, quota, control).map_err(error)?;
    let mut sink = OwnedSink { entries, finite: None, reservations, limits, control };
    crate::convert::check_nesting(&doc.layers, *control.interrupt)?;
    let document = crate::convert::doc_m(doc, &mut sink)?;
    let version = if crate::convert::requires_reader_v8_state_with(doc, *control.interrupt)? {
        8
    } else if crate::convert::requires_reader_v7_state_with(doc, *control.interrupt)? {
        7
    } else if crate::convert::requires_reader_v6_state_with(doc, *control.interrupt)? {
        6
    } else if doc.has_backed_rasters().map_err(|_| error(BackingError::Bounds))? {
        5
    } else if crate::convert::requires_reader_v4_state_with(doc, *control.interrupt)? {
        4
    } else if document.xcf_source_caches.is_empty() && !crate::convert::requires_reader_v3_state_with(doc, *control.interrupt)? {
        2
    } else {
        3
    };
    sink.reserve(128)?;
    let manifest = Manifest {
        format_version: version,
        minimum_reader_version: version,
        generator: concat!("photocraft-format ", env!("CARGO_PKG_VERSION")).into(),
        document,
        thumbnail: None,
        composite: None,
    };
    crate::finite::check_with_interrupt(&manifest, *control.interrupt).map_err(|problem| match problem {
        crate::finite::Problem::NonFinite { path } => FormatError::NonFinite { path },
        crate::finite::Problem::Cancelled => FormatError::Cancelled,
        crate::finite::Problem::Other(message) => FormatError::Unsupported(message),
    })?;
    let json = crate::convert::json_bytes(&manifest, &mut sink, limits.max_manifest_bytes, true)?;
    let mut writer = crate::zip::stream::Writer::new_accounted(output, limits.zip, control, quota)?;
    writer.add_accounted("manifest.json", json.len() as u64, &mut json.as_slice(), control)?;
    let _transport = quota
        .reserve(
            QuotaKind::Resident,
            (74 + std::mem::size_of::<CanonicalBytes<'_, '_, '_>>() + std::mem::size_of::<RawFrame<CanonicalBytes<'_, '_, '_>>>()) as u64,
        )
        .map_err(|_| error(BackingError::Quota))?;
    for entry in sink.entries.as_slice() {
        if entry.object.tile() && sink.finite.as_ref().map(|catalog| catalog.contains_hash(entry.hash, control)).transpose()?.unwrap_or(false) {
            continue;
        }
        control.check(1).map_err(error)?;
        let mut path = [0_u8; 74];
        let prefix: &[u8] = if entry.object.tile() { b"tiles/" } else { b"blobs/" };
        path.get_mut(..6).ok_or_else(|| error(BackingError::Length))?.copy_from_slice(prefix);
        const HEX: &[u8] = b"0123456789abcdef";
        for (index, byte) in entry.hash.iter().enumerate() {
            *path.get_mut(6 + index * 2).ok_or_else(|| error(BackingError::Length))? =
                *HEX.get((byte >> 4) as usize).ok_or_else(|| error(BackingError::Length))?;
            *path.get_mut(7 + index * 2).ok_or_else(|| error(BackingError::Length))? =
                *HEX.get((byte & 15) as usize).ok_or_else(|| error(BackingError::Length))?;
        }
        path.get_mut(70..).ok_or_else(|| error(BackingError::Length))?.copy_from_slice(b".zst");
        let name = std::str::from_utf8(&path).map_err(|_| error(BackingError::Integrity))?;
        let reader = CanonicalBytes { bytes: entry.object.bytes(), sample: entry.object.sample(), position: 0, control };
        let length = entry.object.bytes().len() as u64;
        let mut frame = RawFrame::from_reader(reader, length)?;
        writer.add_accounted(name, crate::zstd_stream::encoded_len(length)?, &mut frame, control)?;
    }
    if let Some(catalog) = &sink.finite {
        catalog.write_accounted(&mut writer, control)?;
    }
    let output = writer.finish_accounted(control)?;
    control.check(0).map_err(error)?;
    Ok(output)
}
