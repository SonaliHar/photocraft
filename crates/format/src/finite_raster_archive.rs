//! Optional standalone finite raster archives. No Document/default native routing.
//! Output storage and outer atomic destination protection remain caller-owned.
use crate::{
    FormatError,
    canonical_archive::{CanonicalArchivePlaneStore, CanonicalTileReference, ImmutableCanonicalArchive, ReadAtCursor},
    canonical_object::{PreparedCanonicalTile, TileObjectError},
    finite_raster::{FiniteRasterLimits, OwnedFiniteRasterMetadata},
    zip::stream::{Limits, Reader, Writer},
};
use photocraft_doc::BackedLayer;
use photocraft_geom::TileCoord;
use photocraft_raster::{
    backing::{
        BackingError, NativePlaneDescriptor, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
    checked_view::{NativeTileBytes, RasterViewError},
};
use serde::{
    Serialize, Serializer,
    ser::{SerializeSeq, SerializeStruct},
};
use std::{
    io::{Seek, Write},
    sync::Arc,
};
const METADATA: &str = "finite-raster.json";
#[derive(Clone, Copy, Debug)]
pub struct FiniteArchiveLimits {
    pub raster: FiniteRasterLimits,
    pub zip: Limits,
    pub max_metadata_bytes: usize,
}
#[derive(Debug)]
pub enum FiniteArchiveError {
    Format(FormatError),
    Backing(BackingError),
    View(RasterViewError),
    Object(TileObjectError),
}
impl std::fmt::Display for FiniteArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "finite raster archive: {self:?}")
    }
}
impl std::error::Error for FiniteArchiveError {}
impl From<FormatError> for FiniteArchiveError {
    fn from(error: FormatError) -> Self {
        if let FormatError::Io(io) = &error
            && let Some(backing) = io.get_ref().and_then(|e| e.downcast_ref::<BackingError>())
        {
            return Self::Backing(*backing);
        }
        Self::Format(error)
    }
}
impl From<BackingError> for FiniteArchiveError {
    fn from(error: BackingError) -> Self {
        Self::Backing(error)
    }
}
impl From<RasterViewError> for FiniteArchiveError {
    fn from(error: RasterViewError) -> Self {
        Self::View(error)
    }
}
impl From<TileObjectError> for FiniteArchiveError {
    fn from(error: TileObjectError) -> Self {
        Self::Object(error)
    }
}
type Result<T> = std::result::Result<T, FiniteArchiveError>;
fn reserve(q: &BackingQuota, kind: QuotaKind, n: usize) -> Result<QuotaLease> {
    q.reserve(kind, n as u64).map_err(|_| BackingError::Quota.into())
}
struct OwnedVector<T> {
    data: Vec<T>,
    _lease: QuotaLease,
}
impl<T> std::ops::Deref for OwnedVector<T> {
    type Target = Vec<T>;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
impl<T> std::ops::DerefMut for OwnedVector<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}
fn vector<T>(count: usize, q: &BackingQuota) -> Result<OwnedVector<T>> {
    let bytes = count.checked_mul(std::mem::size_of::<T>()).ok_or(BackingError::Bounds)?;
    let owner = std::mem::size_of::<OwnedVector<T>>().checked_add(64).ok_or(BackingError::Bounds)?;
    let mut lease = reserve(q, QuotaKind::Index, bytes.checked_mul(2).and_then(|n| n.checked_add(owner)).ok_or(BackingError::Bounds)?)?;
    let mut data = Vec::new();
    data.try_reserve_exact(count).map_err(|_| BackingError::Allocation)?;
    lease
        .reconcile_capacity(data.capacity().checked_mul(std::mem::size_of::<T>()).and_then(|n| n.checked_add(owner)).ok_or(BackingError::Bounds)? as u64)
        .map_err(|_| BackingError::Quota)?;
    Ok(OwnedVector { data, _lease: lease })
}
struct Index {
    refs: OwnedVector<CanonicalTileReference>,
    hashes: OwnedVector<[u8; 32]>,
}
fn grid(d: NativePlaneDescriptor, limits: FiniteArchiveLimits) -> Result<(TileCoord, TileCoord, usize)> {
    NativePlaneDescriptor::new(d.format(), d.bounds(), d.default_bytes())?;
    if d.logical_bytes() > limits.raster.max_logical_bytes {
        return Err(BackingError::Bounds.into());
    }
    let b = d.bounds();
    let first = TileCoord::containing(b.x0, b.y0);
    let last = TileCoord::containing(b.x1.checked_sub(1).ok_or(BackingError::Bounds)?, b.y1.checked_sub(1).ok_or(BackingError::Bounds)?);
    let x = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| BackingError::Bounds)?;
    let y = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| BackingError::Bounds)?;
    let count = usize::try_from(x.checked_mul(y).ok_or(BackingError::Bounds)?).map_err(|_| BackingError::Bounds)?;
    if count > limits.raster.max_tiles.min(4096) {
        return Err(BackingError::Bounds.into());
    }
    Ok((first, last, count))
}
fn exact_default(layer: &BackedLayer, coord: TileCoord, c: &ReadControl<'_>) -> Result<bool> {
    let pin = layer.checked_view().pin_tile(coord, c)?;
    let descriptor = layer.descriptor();
    let default = descriptor.default_bytes();
    if default.is_empty() || default.len() > 40 {
        return Err(BackingError::Bounds.into());
    }
    match pin.encoded() {
        NativeTileBytes::DefaultPixel(bytes) => {
            c.check(bytes.len() as u64)?;
            Ok(bytes == default)
        }
        NativeTileBytes::FullTile(bytes) => {
            let chunk = (4096 / default.len()).checked_mul(default.len()).ok_or(BackingError::Bounds)?;
            for block in bytes.chunks(chunk) {
                c.check(block.len() as u64)?;
                for pixel in block.chunks(default.len()) {
                    if pixel != default {
                        return Ok(false);
                    }
                }
            }
            c.check(0)?;
            Ok(true)
        }
    }
}
/// Fresh standalone archive only. Visits every current operative tile (<=4096),
/// omits only byte-proven defaults, and deduplicates content into this fresh writer.
/// Errors do not return a finished output; partial stream bytes may remain.
pub fn save_finite_raster_archive<W: Write + Seek>(layer: &BackedLayer, output: W, limits: FiniteArchiveLimits, control: &ReadControl<'_>) -> Result<W> {
    control.check(1)?;
    let descriptor = layer.descriptor();
    let (first, last, count) = grid(descriptor, limits)?;
    let q = layer.quota();
    let refs = vector(count, q)?;
    let hashes = vector(count, q)?;
    let mut index = Index { refs, hashes };
    let mut writer = Writer::new_accounted(output, limits.zip, control, q)?;
    for tx in first.tx..=last.tx {
        for ty in first.ty..=last.ty {
            control.check(1)?;
            let coord = TileCoord::new(tx, ty);
            if exact_default(layer, coord, control)? {
                continue;
            }
            let object = PreparedCanonicalTile::prepare(layer, coord, control)?;
            let hash = object.metadata().hash();
            let mut low = 0;
            let mut high = index.hashes.len();
            while low < high {
                control.check(32)?;
                let mid = low + (high - low) / 2;
                let value = index.hashes.get(mid).ok_or(BackingError::Integrity)?;
                if value < &hash { low = mid + 1 } else { high = mid }
            }
            if index.hashes.get(low) != Some(&hash) {
                object.write_entry_accounted(&mut writer)?;
                index.hashes.push(hash);
                let mut at = index.hashes.len() - 1;
                while at > low {
                    control.check(32)?;
                    index.hashes.swap(at, at - 1);
                    at -= 1
                }
            }
            index.refs.push(CanonicalTileReference { coord, hash });
        }
    }
    let mut json = Buffer::new(limits.max_metadata_bytes.min(1 << 20), q, control)?;
    let _serializer = reserve(q, QuotaKind::Resident, 2048)?;
    let mut default = [0; 80];
    let bpp = descriptor.default_bytes().len();
    let mut native = [0; 40];
    native.get_mut(..bpp).ok_or(BackingError::Bounds)?.copy_from_slice(descriptor.default_bytes());
    if cfg!(target_endian = "big") {
        for sample in native.get_mut(..bpp).ok_or(BackingError::Bounds)?.chunks_exact_mut(descriptor.format().sample.bytes()) {
            sample.reverse()
        }
    }
    hex(native.get(..bpp).ok_or(BackingError::Bounds)?, default.get_mut(..bpp * 2).ok_or(BackingError::Bounds)?)?;
    let default = std::str::from_utf8(default.get(..bpp * 2).ok_or(BackingError::Bounds)?).map_err(|_| BackingError::Integrity)?;
    let metadata = Metadata { descriptor, default, refs: &index.refs };
    if let Err(error) = serde_json::to_writer(&mut json, &metadata) {
        return Err(json.json_error(error));
    }
    control.check(0)?;
    writer.add_accounted(METADATA, json.bytes.len() as u64, &mut json.bytes.as_slice(), control)?;
    control.check(0)?;
    Ok(writer.finish_accounted(control)?)
}
/// Authenticate standalone metadata plus every referenced raw canonical object
/// before returning an owned store. No whole archive/plane or standard loader route.
pub fn load_finite_raster_archive(
    source: Arc<dyn ImmutableCanonicalArchive>,
    quota: BackingQuota,
    limits: FiniteArchiveLimits,
    control: &ReadControl<'_>,
) -> Result<CanonicalArchivePlaneStore> {
    control.check(1)?;
    source.validate_quota(&quota)?;
    let mut archive = Reader::new_accounted(ReadAtCursor::new(source.as_ref(), control), limits.zip, control, &quota)?;
    let size = archive.stored_size(METADATA)?;
    if size > limits.max_metadata_bytes.min(1 << 20) as u64 {
        return Err(BackingError::Bounds.into());
    }
    let mut json = Buffer::new(usize::try_from(size).map_err(|_| BackingError::Bounds)?, &quota, control)?;
    {
        let _copier = reserve(&quota, QuotaKind::Resident, 65536 + 256)?;
        if let Err(error) = archive.copy_by_name(METADATA, &mut json, size, control.interrupt) {
            return Err(json.format_error(error));
        }
    }
    drop(archive);
    let metadata = OwnedFiniteRasterMetadata::parse_with(&json.bytes, limits.max_metadata_bytes, limits.raster, quota.clone(), control)?;
    let mut refs = vector(metadata.metadata().tiles.len(), &quota)?;
    for tile in &metadata.metadata().tiles {
        control.check(64)?;
        let mut hash = [0; 32];
        unhex(tile.hash.as_bytes(), &mut hash)?;
        refs.push(CanonicalTileReference { coord: TileCoord::new(tile.tx, tile.ty), hash })
    }
    let store = CanonicalArchivePlaneStore::new_default_padded(source, metadata.descriptor(), &refs, quota, control)?;
    drop(refs);
    drop(metadata);
    drop(json);
    control.check(0)?;
    Ok(store)
}
struct Buffer<'c, 'a> {
    bytes: Vec<u8>,
    max: usize,
    control: &'c ReadControl<'a>,
    failure: Option<BackingError>,
    _lease: QuotaLease,
}
impl<'c, 'a> Buffer<'c, 'a> {
    fn new(max: usize, q: &BackingQuota, control: &'c ReadControl<'a>) -> Result<Self> {
        control.check(1)?;
        let mut lease = reserve(q, QuotaKind::Resident, max.checked_mul(2).and_then(|n| n.checked_add(128)).ok_or(BackingError::Bounds)?)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(max).map_err(|_| BackingError::Allocation)?;
        lease.reconcile_capacity(bytes.capacity().checked_add(128).ok_or(BackingError::Bounds)? as u64).map_err(|_| BackingError::Quota)?;
        Ok(Self { bytes, max, control, failure: None, _lease: lease })
    }
    fn json_error(&self, error: serde_json::Error) -> FiniteArchiveError {
        self.failure.map(FiniteArchiveError::Backing).unwrap_or_else(|| FiniteArchiveError::Format(FormatError::Json(error)))
    }
    fn format_error(&self, error: FormatError) -> FiniteArchiveError {
        self.failure.map(FiniteArchiveError::Backing).unwrap_or_else(|| error.into())
    }
}
impl Write for Buffer<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let result = (|| -> std::result::Result<(), BackingError> {
            let end = self.bytes.len().checked_add(bytes.len()).ok_or(BackingError::Bounds)?;
            if end > self.max || end > self.bytes.capacity() {
                return Err(BackingError::Bounds);
            }
            for block in bytes.chunks(4096) {
                self.control.check(block.len() as u64)?;
                self.bytes.extend_from_slice(block)
            }
            self.control.check(0)?;
            Ok(())
        })();
        match result {
            Ok(()) => Ok(bytes.len()),
            Err(error) => {
                self.failure = Some(error);
                Err(std::io::Error::other(error))
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.control.check(0).map_err(std::io::Error::other)
    }
}
fn hex(input: &[u8], out: &mut [u8]) -> Result<()> {
    if input.len().checked_mul(2) != Some(out.len()) {
        return Err(BackingError::Bounds.into());
    }
    for (i, byte) in input.iter().enumerate() {
        *out.get_mut(i * 2).ok_or(BackingError::Bounds)? = *b"0123456789abcdef".get(usize::from(byte >> 4)).ok_or(BackingError::Bounds)?;
        *out.get_mut(i * 2 + 1).ok_or(BackingError::Bounds)? = *b"0123456789abcdef".get(usize::from(byte & 15)).ok_or(BackingError::Bounds)?;
    }
    Ok(())
}
fn unhex(input: &[u8], out: &mut [u8]) -> Result<()> {
    if input.len() != out.len() * 2 {
        return Err(BackingError::Integrity.into());
    }
    for (pair, byte) in input.as_chunks::<2>().0.iter().zip(out) {
        let nibble = |b: u8| match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            _ => Err(BackingError::Integrity),
        };
        *byte = nibble(*pair.first().ok_or(BackingError::Integrity)?)? * 16 + nibble(*pair.get(1).ok_or(BackingError::Integrity)?)?;
    }
    Ok(())
}
struct Metadata<'a> {
    descriptor: NativePlaneDescriptor,
    default: &'a str,
    refs: &'a [CanonicalTileReference],
}
impl Serialize for Metadata<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut m = s.serialize_struct("FiniteRasterM", 4)?;
        m.serialize_field("format", &self.descriptor.format())?;
        m.serialize_field("bounds", &self.descriptor.bounds())?;
        m.serialize_field("default", self.default)?;
        m.serialize_field("tiles", &Refs(self.refs))?;
        m.end()
    }
}
struct Refs<'a>(&'a [CanonicalTileReference]);
impl Serialize for Refs<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for r in self.0 {
            seq.serialize_element(&Ref(r))?;
        }
        seq.end()
    }
}
struct Ref<'a>(&'a CanonicalTileReference);
impl Serialize for Ref<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::Error;
        let mut hash = [0; 64];
        hex(&self.0.hash, &mut hash).map_err(S::Error::custom)?;
        let hash = std::str::from_utf8(&hash).map_err(S::Error::custom)?;
        let mut m = s.serialize_struct("TileRef", 3)?;
        m.serialize_field("tx", &self.0.coord.tx)?;
        m.serialize_field("ty", &self.0.coord.ty)?;
        m.serialize_field("hash", hash)?;
        m.end()
    }
}
