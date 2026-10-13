//! Bounded current-state finite object catalog for the charged whole-document codec.
//! Retains immutable layer owners+coordinates, never all tile pins. Other ordinary
//! objects are delegated to the existing exact Sink with shared observer accounting.
use crate::{FormatError, Result, canonical_object::PreparedCanonicalTile, finite_raster::FiniteRasterM, manifest::TileRef, zip::stream::Writer};
use photocraft_doc::{
    BackedLayer,
    backed_tiles::{BackedTilePlan, TileEnumerationLimits},
};
use photocraft_geom::TileCoord;
use photocraft_raster::{
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
    checked_view::NativeTileBytes,
};
use std::io::{Seek, Write};
struct Source {
    hash: [u8; 32],
    layer: BackedLayer,
    coord: TileCoord,
}
pub(crate) struct OwnedFiniteCatalog {
    sources: Vec<Source>,
    manifest_leases: Vec<QuotaLease>,
    quota: BackingQuota,
    max_entries: usize,
    max_logical_bytes: u64,
    visited: usize,
    failed: bool,
    _owner: QuotaLease,
}
impl OwnedFiniteCatalog {
    pub(crate) fn new(q: BackingQuota, max_entries: usize, max_layers: usize, max_logical_bytes: u64, ctl: &ReadControl<'_>) -> Result<Self> {
        ctl.check(1).map_err(backing)?;
        if max_entries > 4096 || max_layers > 4096 {
            return Err(backing(BackingError::Bounds));
        }
        let envelope = max_entries
            .checked_mul(std::mem::size_of::<Source>())
            .and_then(|n| max_layers.checked_mul(std::mem::size_of::<QuotaLease>()).and_then(|m| n.checked_add(m)))
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>() + 128))
            .ok_or_else(|| backing(BackingError::Bounds))?;
        let mut owner = q.reserve(QuotaKind::Index, envelope as u64).map_err(|_| backing(BackingError::Quota))?;
        let mut sources = Vec::new();
        sources.try_reserve_exact(max_entries).map_err(|_| backing(BackingError::Allocation))?;
        let mut manifest_leases = Vec::new();
        manifest_leases.try_reserve_exact(max_layers).map_err(|_| backing(BackingError::Allocation))?;
        let actual = sources
            .capacity()
            .checked_mul(std::mem::size_of::<Source>())
            .and_then(|n| manifest_leases.capacity().checked_mul(std::mem::size_of::<QuotaLease>()).and_then(|m| n.checked_add(m)))
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>() + 128))
            .ok_or_else(|| backing(BackingError::Bounds))?;
        owner.reconcile_capacity(actual as u64).map_err(|_| backing(BackingError::Quota))?;
        ctl.check(0).map_err(backing)?;
        Ok(Self { sources, manifest_leases, quota: q, max_entries, max_logical_bytes, visited: 0, failed: false, _owner: owner })
    }
    pub(crate) fn capture_finite(&mut self, layer: &BackedLayer, ctl: &ReadControl<'_>) -> Result<FiniteRasterM> {
        if self.failed {
            return Err(backing(BackingError::Integrity));
        }
        let result = self.capture_inner(layer, ctl);
        if result.is_err() {
            self.failed = true
        }
        result
    }
    fn capture_inner(&mut self, layer: &BackedLayer, ctl: &ReadControl<'_>) -> Result<FiniteRasterM> {
        ctl.check(1).map_err(backing)?;
        if !layer.quota().shares_ledger(&self.quota) || self.manifest_leases.len() >= self.manifest_leases.capacity() {
            return Err(backing(BackingError::Quota));
        }
        let d = layer.descriptor();
        if d.logical_bytes() > self.max_logical_bytes {
            return Err(backing(BackingError::Bounds));
        }
        let plan =
            BackedTilePlan::new(d, TileEnumerationLimits { max_tiles: self.max_entries as u64, max_index_bytes: u64::MAX, index_bytes_per_tile: 0 }, ctl)
                .map_err(enumeration)?;
        let count = usize::try_from(plan.tile_count()).map_err(|_| backing(BackingError::Bounds))?;
        let next = self.visited.checked_add(count).ok_or_else(|| backing(BackingError::Bounds))?;
        if next > self.max_entries {
            return Err(backing(BackingError::Bounds));
        }
        let refs_bytes = count
            .checked_mul(std::mem::size_of::<TileRef>() + 64)
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(512))
            .ok_or_else(|| backing(BackingError::Bounds))?;
        let mut lease = self.quota.reserve(QuotaKind::Index, refs_bytes as u64).map_err(|_| backing(BackingError::Quota))?;
        let mut refs = Vec::new();
        refs.try_reserve_exact(count).map_err(|_| backing(BackingError::Allocation))?;
        let mut grid = plan.iter(ctl);
        while let Some(coord) = grid.next_tile().map_err(enumeration)? {
            if exact_default(layer, coord, ctl)? {
                continue;
            }
            let prepared = PreparedCanonicalTile::prepare(layer, coord, ctl).map_err(object)?;
            let hash = prepared.metadata().hash();
            drop(prepared);
            let mut low = 0;
            let mut high = self.sources.len();
            while low < high {
                ctl.check(32).map_err(backing)?;
                let mid = low + (high - low) / 2;
                if self.sources.get(mid).ok_or_else(|| backing(BackingError::Integrity))?.hash < hash { low = mid + 1 } else { high = mid }
            }
            if self.sources.get(low).is_none_or(|entry| entry.hash != hash) {
                if self.sources.len() >= self.sources.capacity() {
                    return Err(backing(BackingError::Bounds));
                }
                self.sources.push(Source { hash, layer: layer.clone(), coord });
                let mut at = self.sources.len() - 1;
                while at > low {
                    ctl.check(32).map_err(backing)?;
                    self.sources.swap(at, at - 1);
                    at -= 1
                }
            }
            refs.push(TileRef { tx: coord.tx, ty: coord.ty, hash: hex(&hash)? });
        }
        // Plan is row-major; FiniteRasterM uses TileCoord Ord (tx first). Keep
        // this bounded polled insertion order instead of uninterruptible sort.
        for i in 1..refs.len() {
            let mut at = i;
            while at > 0 {
                ctl.check(32).map_err(backing)?;
                let prev = refs.get(at - 1).ok_or_else(|| backing(BackingError::Integrity))?;
                let cur = refs.get(at).ok_or_else(|| backing(BackingError::Integrity))?;
                if (prev.tx, prev.ty) <= (cur.tx, cur.ty) {
                    break;
                }
                refs.swap(at, at - 1);
                at -= 1
            }
        }
        let mut default = [0; 40];
        let size = d.default_bytes().len();
        default.get_mut(..size).ok_or_else(|| backing(BackingError::Bounds))?.copy_from_slice(d.default_bytes());
        let bytes = default.get_mut(..size).ok_or_else(|| backing(BackingError::Bounds))?;
        if cfg!(target_endian = "big") {
            for value in bytes.chunks_exact_mut(d.format().sample.bytes()) {
                value.reverse()
            }
        }
        let default = hex(bytes)?;
        let actual = refs
            .capacity()
            .checked_mul(std::mem::size_of::<TileRef>())
            .and_then(|n| refs.iter().try_fold(n, |n, r| n.checked_add(r.hash.capacity())))
            .and_then(|n| n.checked_add(default.capacity()))
            .and_then(|n| n.checked_add(512))
            .ok_or_else(|| backing(BackingError::Bounds))?;
        lease.reconcile_capacity(actual as u64).map_err(|_| backing(BackingError::Quota))?;
        ctl.check(0).map_err(backing)?;
        self.visited = next;
        self.manifest_leases.push(lease);
        Ok(FiniteRasterM { format: d.format(), bounds: d.bounds(), default, tiles: refs })
    }
    pub(crate) fn contains_hash(&self, hash: [u8; 32], ctl: &ReadControl<'_>) -> Result<bool> {
        for source in &self.sources {
            ctl.check(1).map_err(backing)?;
            if source.hash == hash {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(crate) fn write_accounted<W: Write + Seek>(&self, writer: &mut Writer<W>, ctl: &ReadControl<'_>) -> Result<()> {
        if self.failed {
            return Err(backing(BackingError::Integrity));
        }
        for source in &self.sources {
            ctl.check(1).map_err(backing)?;
            let prepared = PreparedCanonicalTile::prepare(&source.layer, source.coord, ctl).map_err(object)?;
            if prepared.metadata().hash() != source.hash {
                return Err(backing(BackingError::Integrity));
            }
            prepared.write_entry_accounted(writer).map_err(object)?;
        }
        ctl.check(0).map_err(backing)
    }
}
fn exact_default(layer: &BackedLayer, coord: TileCoord, ctl: &ReadControl<'_>) -> Result<bool> {
    let pin = layer.checked_view().pin_tile(coord, ctl).map_err(view)?;
    let default = layer.default_bytes();
    if default.is_empty() {
        return Err(backing(BackingError::Bounds));
    }
    match pin.encoded() {
        NativeTileBytes::DefaultPixel(value) => {
            ctl.check(value.len() as u64).map_err(backing)?;
            Ok(value == default)
        }
        NativeTileBytes::FullTile(bytes) => {
            let chunk = (4096 / default.len()) * default.len();
            for block in bytes.chunks(chunk) {
                ctl.check(block.len() as u64).map_err(backing)?;
                for pixel in block.chunks_exact(default.len()) {
                    if pixel != default {
                        return Ok(false);
                    }
                }
            }
            ctl.check(0).map_err(backing)?;
            Ok(true)
        }
    }
}
fn hex(bytes: &[u8]) -> Result<String> {
    let n = bytes.len().checked_mul(2).ok_or_else(|| backing(BackingError::Bounds))?;
    let mut text = String::new();
    text.try_reserve_exact(n).map_err(|_| backing(BackingError::Allocation))?;
    if text.capacity() > n.checked_mul(2).ok_or_else(|| backing(BackingError::Bounds))? {
        return Err(backing(BackingError::Quota));
    }
    for byte in bytes {
        for value in [byte >> 4, byte & 15] {
            text.push(char::from(*b"0123456789abcdef".get(value as usize).ok_or_else(|| backing(BackingError::Integrity))?));
        }
    }
    Ok(text)
}
fn backing(error: BackingError) -> FormatError {
    match error {
        BackingError::Cancelled => FormatError::Cancelled,
        other => FormatError::Io(std::io::Error::other(other)),
    }
}

fn enumeration(error: photocraft_doc::backed_tiles::TileEnumerationError) -> FormatError {
    match error {
        photocraft_doc::backed_tiles::TileEnumerationError::Control(value) => backing(value),
        other => FormatError::Io(std::io::Error::other(other)),
    }
}
fn view(error: photocraft_raster::checked_view::RasterViewError) -> FormatError {
    use photocraft_raster::checked_view::RasterViewError;
    match error {
        RasterViewError::Backed(value) | RasterViewError::Control(value) | RasterViewError::Descriptor(value) => backing(value),
        RasterViewError::Resident(photocraft_raster::RasterInputError::Cancelled) => FormatError::Cancelled,
        other => FormatError::Io(std::io::Error::other(other)),
    }
}
fn canonical(error: crate::canonical_tile::CanonicalTileError) -> FormatError {
    match error {
        crate::canonical_tile::CanonicalTileError::Backing(value) => backing(value),
        crate::canonical_tile::CanonicalTileError::View(value) => view(value),
        other => FormatError::Io(std::io::Error::other(other)),
    }
}
fn object(error: crate::canonical_object::TileObjectError) -> FormatError {
    match error {
        crate::canonical_object::TileObjectError::Backing(value) => backing(value),
        crate::canonical_object::TileObjectError::Canonical(value) => canonical(value),
        crate::canonical_object::TileObjectError::Format(value) => value,
        other => FormatError::Io(std::io::Error::other(other)),
    }
}
