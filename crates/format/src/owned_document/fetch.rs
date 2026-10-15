use super::{
    OwnedDocumentLimits,
    accounting::{ChargedVec, RetainedReservations},
    decode::{ObjectDecodeLimits, decode_owned_object},
    error,
};
use crate::canonical_archive::{CanonicalArchivePlaneStore, CanonicalTileReference};
use crate::{FormatError, LoadOptions, Result, convert::Fetch, store::Source};
use photocraft_doc::BackedLayer;
use photocraft_geom::TileCoord;
use photocraft_raster::backing::{
    BackedRaster, BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind},
};
use std::{cell::RefCell, sync::Arc};

pub(super) struct OwnedFetch<'a, 'c> {
    archive: Arc<dyn crate::canonical_archive::ImmutableCanonicalArchive>,
    source: &'a dyn Source,
    ownership: &'a RefCell<RetainedReservations>,
    quota: BackingQuota,
    control: &'a ReadControl<'c>,
    options: LoadOptions,
    limits: OwnedDocumentLimits,
    total: u64,
    // Fixed prepaid catalogue; Arc clones share one actual decoded allocation.
    blobs: ChargedVec<([u8; 32], Arc<Vec<u8>>)>,
}
impl<'a, 'c> OwnedFetch<'a, 'c> {
    pub(super) fn new(
        archive: Arc<dyn crate::canonical_archive::ImmutableCanonicalArchive>,
        source: &'a dyn Source,
        ownership: &'a RefCell<RetainedReservations>,
        quota: BackingQuota,
        control: &'a ReadControl<'c>,
        options: LoadOptions,
        limits: OwnedDocumentLimits,
    ) -> Result<Self> {
        let blobs = ChargedVec::with_capacity(limits.max_reservations, &quota, control).map_err(error)?;
        Ok(Self { archive, source, ownership, quota, control, options, limits, total: 0, blobs })
    }
    fn retained(&self, bytes: ChargedVec<u8>) -> Result<Vec<u8>> {
        let (payload, lease) = bytes.into_parts();
        let work = |_| Ok(());
        let transfer = ReadControl { interrupt: self.control.interrupt, work: &work };
        let result = match self.ownership.try_borrow_mut() {
            Ok(mut owner) => owner.retain(lease, &transfer),
            Err(_) => {
                drop(payload);
                drop(lease);
                return Err(error(photocraft_raster::backing::BackingError::Work));
            }
        };
        if let Err((failure, lease)) = result {
            drop(payload);
            drop(lease);
            return Err(error(failure));
        }
        Ok(payload)
    }
    fn account(&mut self, bytes: usize) -> Result<()> {
        let next = self.total.checked_add(bytes as u64).ok_or_else(|| FormatError::LimitExceeded("owned expanded objects".into()))?;
        if next > self.options.max_total_bytes {
            return Err(FormatError::LimitExceeded("owned expanded objects".into()));
        }
        self.total = next;
        Ok(())
    }
    fn object(&self, hash: &str, tile: bool, max: usize, expected: Option<usize>) -> Result<ChargedVec<u8>> {
        let hash_bytes = parse_hash(hash)?;
        let mut path = [0_u8; 74];
        path.get_mut(..6).ok_or_else(|| FormatError::Corrupt("object path".into()))?.copy_from_slice(if tile { b"tiles/" } else { b"blobs/" });
        path.get_mut(6..70).ok_or_else(|| FormatError::Corrupt("object path".into()))?.copy_from_slice(hash.as_bytes());
        path.get_mut(70..).ok_or_else(|| FormatError::Corrupt("object path".into()))?.copy_from_slice(b".zst");
        let path = std::str::from_utf8(&path).map_err(|_| FormatError::Corrupt("object path UTF-8".into()))?;
        let encoded_limit =
            max.checked_mul(2).and_then(|n| n.checked_add(64 << 10)).ok_or_else(|| FormatError::LimitExceeded("owned encoded object limit".into()))?;
        let encoded = self.source.get(path, encoded_limit)?;
        decode_owned_object(&encoded, expected, hash_bytes, ObjectDecodeLimits { expanded: max, window: self.limits.decoder_window }, &self.quota, self.control)
            .map_err(error)
    }
}
fn parse_hash(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64 {
        return Err(FormatError::Corrupt("invalid object hash".into()));
    }
    let mut hash = [0_u8; 32];
    for (slot, pair) in hash.iter_mut().zip(value.as_bytes().as_chunks::<2>().0.iter()) {
        let digit = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        };
        let high = pair.first().copied().and_then(digit).ok_or_else(|| FormatError::Corrupt("invalid object hash".into()))?;
        let low = pair.get(1).copied().and_then(digit).ok_or_else(|| FormatError::Corrupt("invalid object hash".into()))?;
        *slot = (high << 4) | low;
    }
    Ok(hash)
}
impl Fetch for OwnedFetch<'_, '_> {
    fn finite_raster(&mut self, metadata: &crate::finite_raster::FiniteRasterM) -> Result<photocraft_doc::BackedLayer> {
        let descriptor = metadata.descriptor_with(
            crate::finite_raster::FiniteRasterLimits { max_logical_bytes: self.limits.max_finite_logical_bytes, max_tiles: self.limits.max_reservations },
            self.control,
        )?;
        let count = metadata.tiles.len();
        let bytes = count.checked_mul(std::mem::size_of::<CanonicalTileReference>()).ok_or_else(|| error(BackingError::Bounds))?;
        let mut lease = self
            .quota
            .reserve(QuotaKind::Index, bytes.checked_mul(2).and_then(|n| n.checked_add(128)).ok_or_else(|| error(BackingError::Bounds))? as u64)
            .map_err(|_| error(BackingError::Quota))?;
        let mut refs = Vec::new();
        refs.try_reserve_exact(count).map_err(|_| error(BackingError::Allocation))?;
        lease
            .reconcile_capacity(
                refs.capacity()
                    .checked_mul(std::mem::size_of::<CanonicalTileReference>())
                    .and_then(|n| n.checked_add(128))
                    .ok_or_else(|| error(BackingError::Bounds))? as u64,
            )
            .map_err(|_| error(BackingError::Quota))?;
        for tile in &metadata.tiles {
            self.control.check(64).map_err(error)?;
            let mut hash = [0; 32];
            for (i, byte) in hash.iter_mut().enumerate() {
                let at = i.checked_mul(2).ok_or_else(|| error(BackingError::Bounds))?;
                let high = finite_nibble(*tile.hash.as_bytes().get(at).ok_or_else(|| error(BackingError::Integrity))?)?;
                let low = finite_nibble(*tile.hash.as_bytes().get(at + 1).ok_or_else(|| error(BackingError::Integrity))?)?;
                *byte = (high << 4) | low;
            }
            refs.push(CanonicalTileReference { coord: TileCoord::new(tile.tx, tile.ty), hash });
        }
        let store = CanonicalArchivePlaneStore::new_default_padded(self.archive.clone(), descriptor, &refs, self.quota.clone(), self.control).map_err(error)?;
        drop(refs);
        drop(lease);
        let raster = BackedRaster::new(Arc::new(store), self.quota.clone(), 2, self.control).map_err(error)?;
        let layer = BackedLayer::new(raster, descriptor, self.control).map_err(|error| FormatError::Io(std::io::Error::other(error)))?;
        self.control.check(0).map_err(error)?;
        Ok(layer)
    }

    fn accounts_model(&self) -> bool {
        true
    }
    fn model_poll(&self, units: u64) -> Result<()> {
        self.control.check(units).map_err(error)
    }
    fn model_reserve(&mut self, bytes: u64) -> Result<()> {
        self.control.check(1).map_err(error)?;
        // Nonreentrant owner mutation contains no application callback.
        let work = |_| Ok(());
        let ctl = ReadControl { interrupt: self.control.interrupt, work: &work };
        self.ownership.try_borrow_mut().map_err(|_| error(photocraft_raster::backing::BackingError::Work))?.reserve(bytes, &ctl).map_err(error)
    }
    fn tile(&mut self, hash: &str, len: usize) -> Result<Vec<u8>> {
        if len > self.limits.max_object_bytes {
            return Err(FormatError::LimitExceeded("owned tile size".into()));
        }
        self.account(len)?;
        // The Loader copies decoded bytes to a new resident tile. Charge that
        // distinct payload before tile_mut, including bounded Arc/map owners.
        self.model_reserve((len as u64).checked_add(512).ok_or_else(|| FormatError::LimitExceeded("owned surface tile".into()))?)?;
        let decoded = self.object(hash, true, len, Some(len))?;
        self.retained(decoded)
    }
    fn blob(&mut self, hash: &str) -> Result<Arc<Vec<u8>>> {
        self.blob_bounded(hash, self.options.max_blob_bytes)
    }
    fn blob_bounded(&mut self, hash: &str, max: usize) -> Result<Arc<Vec<u8>>> {
        self.control.check(0).map_err(error)?;
        let key = parse_hash(hash)?;
        let max = max.min(self.options.max_blob_bytes).min(self.limits.max_object_bytes);
        if let Some((_, bytes)) = self.blobs.as_slice().iter().find(|(stored, _)| stored == &key) {
            if bytes.len() > max {
                return Err(FormatError::LimitExceeded("owned bounded blob".into()));
            }
            return Ok(bytes.clone());
        }
        let decoded = self.object(hash, false, max, None)?;
        self.account(decoded.as_slice().len())?;
        let bytes = Arc::new(self.retained(decoded)?);
        self.blobs.push((key, bytes.clone()), self.control).map_err(error)?;
        Ok(bytes)
    }
    fn source_cache_bytes(&mut self, hash: &str, max: usize, interrupt: &photocraft_raster::Interrupt<'_>) -> Result<Arc<Vec<u8>>> {
        interrupt.check().map_err(|_| FormatError::Cancelled)?;
        let bytes = self.blob_bounded(hash, max)?;
        // No duplicate payload allocation. Preserve the former per-descriptor
        // copy reservation/work conservatively, including its 16-byte envelope.
        self.model_reserve((bytes.len() as u64).checked_add(16).ok_or_else(|| FormatError::LimitExceeded("source cache model reservation".into()))?)?;
        for chunk in bytes.chunks(4096) { self.control.check(chunk.len() as u64).map_err(error)?; }
        interrupt.check().map_err(|_| FormatError::Cancelled)?;
        Ok(bytes)
    }
}

fn finite_nibble(value: u8) -> Result<u8> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(error(BackingError::Integrity)),
    }
}
