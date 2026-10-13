//! Prepared exact current canonical tile objects for the existing native ZIP path.
//! ZIP writer/container staging remains caller-owned under its existing Limits.
use crate::{
    FormatError,
    canonical_tile::{CanonicalTileError, CanonicalTileReader},
    zip::stream::Writer,
    zstd_stream::RawFrame,
};
use photocraft_color::PixelFormat;
use photocraft_doc::BackedLayer;
use photocraft_geom::TileCoord;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use std::io::{Seek, SeekFrom, Write};
#[derive(Debug)]
pub enum TileObjectError {
    Canonical(CanonicalTileError),
    Backing(BackingError),
    Format(FormatError),
    Length,
}
impl std::fmt::Display for TileObjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "canonical tile object: {self:?}")
    }
}
impl std::error::Error for TileObjectError {}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileObjectMetadata {
    format: PixelFormat,
    length: u64,
    hash: [u8; 32],
    path: [u8; 74],
}
impl TileObjectMetadata {
    pub fn format(&self) -> PixelFormat {
        self.format
    }
    pub fn length(&self) -> u64 {
        self.length
    }
    pub fn hash(&self) -> [u8; 32] {
        self.hash
    }
    pub fn entry_name(&self) -> Result<&str, TileObjectError> {
        std::str::from_utf8(&self.path).map_err(|_| TileObjectError::Length)
    }
}
/// Same-ledger pin+canonical scratch+metadata ownership. No source locator or
/// mutable layer reference remains; local layer/history owners can be dropped.
/// Source payload drops before metadata lease. No whole-tile hashing allocation.
pub struct PreparedCanonicalTile<'c, 'a> {
    reader: CanonicalTileReader<'c, 'a>,
    metadata: TileObjectMetadata,
    control: &'c ReadControl<'a>,
    quota: BackingQuota,
    _lease: QuotaLease,
}
impl<'c, 'a> PreparedCanonicalTile<'c, 'a> {
    pub fn prepare(layer: &BackedLayer, coord: TileCoord, control: &'c ReadControl<'a>) -> Result<Self, TileObjectError> {
        control.check(1).map_err(TileObjectError::Backing)?;
        let metadata_bytes = std::mem::size_of::<Self>()
            .checked_sub(std::mem::size_of::<CanonicalTileReader<'c, 'a>>())
            .and_then(|n| n.checked_add(128))
            .ok_or(TileObjectError::Length)?;
        let metadata_lease = layer.quota().reserve(QuotaKind::Resident, metadata_bytes as u64).map_err(|_| TileObjectError::Backing(BackingError::Quota))?;
        let (reader, hash, path) = {
            let temporary_bytes =
                4096_usize.checked_add(std::mem::size_of::<blake3::Hasher>()).and_then(|n| n.checked_add(128)).ok_or(TileObjectError::Length)?;
            let _temporary_lease =
                layer.quota().reserve(QuotaKind::Resident, temporary_bytes as u64).map_err(|_| TileObjectError::Backing(BackingError::Quota))?;
            let mut reader = CanonicalTileReader::new(layer, coord, control).map_err(TileObjectError::Canonical)?;
            let mut buffer = [0; 4096];
            let mut hasher = blake3::Hasher::new();
            while reader.position() < reader.length() {
                let count = reader.try_read(&mut buffer).map_err(TileObjectError::Canonical)?;
                if count == 0 {
                    return Err(TileObjectError::Length);
                }
                control.check(count as u64).map_err(TileObjectError::Backing)?;
                hasher.update(buffer.get(..count).ok_or(TileObjectError::Length)?);
                control.check(0).map_err(TileObjectError::Backing)?;
            }
            let hash = *hasher.finalize().as_bytes();
            let mut path = [0; 74];
            path.get_mut(..6).ok_or(TileObjectError::Length)?.copy_from_slice(b"tiles/");
            path.get_mut(70..).ok_or(TileObjectError::Length)?.copy_from_slice(b".zst");
            for (index, byte) in hash.iter().enumerate() {
                let at = index.checked_mul(2).and_then(|n| n.checked_add(6)).ok_or(TileObjectError::Length)?;
                *path.get_mut(at).ok_or(TileObjectError::Length)? = *b"0123456789abcdef".get(usize::from(byte >> 4)).ok_or(TileObjectError::Length)?;
                *path.get_mut(at + 1).ok_or(TileObjectError::Length)? = *b"0123456789abcdef".get(usize::from(byte & 15)).ok_or(TileObjectError::Length)?;
            }
            control.check(74).map_err(TileObjectError::Backing)?;
            reader.try_seek(SeekFrom::Start(0)).map_err(TileObjectError::Canonical)?;
            control.check(1).map_err(TileObjectError::Backing)?;
            (reader, hash, path)
        };
        let metadata = TileObjectMetadata { format: layer.format(), length: reader.length(), hash, path };
        Ok(Self { reader, metadata, control, quota: layer.quota().clone(), _lease: metadata_lease })
    }
    pub fn metadata(&self) -> TileObjectMetadata {
        self.metadata
    }
    /// Consume the prepared pin into one existing ZIP64 STORE entry containing a
    /// raw-block Zstandard frame. ZIP owns its Limits/staging; outer atomic output
    /// is a caller responsibility. A failed add poisons ZIP finish. Earlier success
    /// bytes may remain uncommitted; this is not an atomic destination API.
    pub fn write_entry<W: Write + Seek>(self, writer: &mut Writer<W>) -> Result<TileObjectMetadata, TileObjectError> {
        let metadata = self.metadata;
        let size = crate::zstd_stream::encoded_len(metadata.length).map_err(TileObjectError::Format)?;
        let mut input = RawFrame::from_reader(self.reader, metadata.length).map_err(TileObjectError::Format)?;
        writer.add(metadata.entry_name()?, size, &mut input, self.control.interrupt).map_err(TileObjectError::Format)?;
        Ok(metadata)
    }
    /// Consume into an accounted writer on the same persistent ledger. The pin,
    /// canonical scratch and object metadata stay charged through entry writing.
    /// Input work/cancel uses this object's borrowed control. Caller owns output
    /// storage separately; incomplete bytes are not an atomic destination commit.
    pub fn write_entry_accounted<W: Write + Seek>(self, writer: &mut Writer<W>) -> Result<TileObjectMetadata, TileObjectError> {
        writer.validate_accounted_quota(&self.quota).map_err(TileObjectError::Format)?;
        self.control.check(1).map_err(TileObjectError::Backing)?;
        let metadata = self.metadata;
        let size = crate::zstd_stream::encoded_len(metadata.length).map_err(TileObjectError::Format)?;
        let mut input = RawFrame::from_reader(self.reader, metadata.length).map_err(TileObjectError::Format)?;
        writer.add_accounted(metadata.entry_name()?, size, &mut input, self.control).map_err(TileObjectError::Format)?;
        Ok(metadata)
    }
}
