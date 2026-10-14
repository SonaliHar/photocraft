//! Nonforgeable, physically charged native pixel-selection payload ownership.
//! Only borrowed native words enter the constructor; external lease assertions
//! or arbitrary Surface wrappers cannot become a selection guard. Map/Arc
//! envelopes are conservative standard-allocator boundaries, not exact BTreeMap
//! node/RSS measurements. Native tile/default payloads and inline headers are
//! measured; source providers/enclosing borrowed input ownership remain caller-held.
//! Raw borrowed Document/Surface Clone outside the typed lifecycle is not guarded;
//! callers must retain its physical owner. The new selection surface accessor is
//! crate-private so arbitrary callers cannot extract an unowned raw view.
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use photocraft_raster::{MAX_CHECKED_RASTER_BYTES, Surface, Tile};
use std::sync::Arc;
#[path = "accounted_selection_owned.rs"]
mod owned;
#[path = "accounted_selection_bands.rs"]
mod bands;
pub use bands::AccountedSelectionBandBuilder;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverageDomain {
    FiniteUnit,
}
struct SelectionOwner {
    surface: Surface,
    bounds: Rect,
    quota: BackingQuota,
    _retention: Option<Arc<dyn std::any::Any + Send + Sync>>,
    _payload: Option<QuotaLease>,
    _index: Option<QuotaLease>,
}
/// Sharing keeps the original charged native Tile owners alive. This exposes no
/// mutable Surface, payload-taking accessor, Weak owner or arbitrary lease hook.
#[derive(Clone)]
pub struct AccountedSelection {
    owner: Arc<SelectionOwner>,
    _allocation: Arc<QuotaLease>,
}
impl std::fmt::Debug for AccountedSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountedSelection").field("format", &self.format()).field("bounds", &self.bounds()).finish_non_exhaustive()
    }
}
/// Each installed/copied Document has its own by-value map and default allocation.
/// It shares only the physically charged immutable tiles of AccountedSelection.
pub(crate) struct SelectionViewCharges {
    _default: QuotaLease,
    _index: QuotaLease,
}
pub(crate) struct SelectionView {
    pub(crate) surface: Surface,
    pub(crate) charges: SelectionViewCharges,
}
fn add(a: u64, b: u64) -> Result<u64, BackingError> {
    a.checked_add(b).ok_or(BackingError::Length)
}
fn mul(a: u64, b: u64) -> Result<u64, BackingError> {
    a.checked_mul(b).ok_or(BackingError::Length)
}
fn bounds(r: Rect) -> Result<u64, BackingError> {
    if r.is_empty() || r.x1 < r.x0 || r.y1 < r.y0 || [r.x0, r.x1, r.y0, r.y1].iter().any(|x| !(-(1 << 29)..=1 << 29).contains(x)) {
        return Err(BackingError::Bounds);
    }
    if r.width() > 262144 || r.height() > 262144 {
        return Err(BackingError::Length);
    }
    let first = TileCoord::containing(r.x0, r.y0);
    let last = TileCoord::containing(r.x1.checked_sub(1).ok_or(BackingError::Bounds)?, r.y1.checked_sub(1).ok_or(BackingError::Bounds)?);
    mul(
        u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| BackingError::Bounds)?,
        u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| BackingError::Bounds)?,
    )
}
fn valid(format: PixelFormat) -> Result<(), BackingError> {
    if format.mode != ColorMode::Grayscale
        || format.alpha
        || !matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
    {
        return Err(BackingError::Unsupported);
    }
    Ok(())
}
fn coverage(format: PixelFormat, raw: &[u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
    let bpp = format.bytes_per_pixel();
    if raw.len() % bpp != 0 {
        return Err(BackingError::Length);
    }
    for block in raw.chunks(64 * 1024) {
        control.check(block.len() as u64)?;
        if matches!(format.sample, SampleType::F32 | SampleType::F64) {
            for pixel in block.chunks_exact(bpp) {
                let v = try_read_sample_f64(pixel, format.sample, 0).map_err(|_| BackingError::Integrity)?;
                if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                    return Err(BackingError::Unsupported);
                }
            }
        }
    }
    Ok(())
}
fn raster(error: photocraft_raster::RasterInputError) -> BackingError {
    use photocraft_raster::RasterInputError as E;
    match error {
        E::Cancelled => BackingError::Cancelled,
        E::AllocationFailed => BackingError::Allocation,
        E::LimitExceeded => BackingError::Quota,
        E::InvalidBounds => BackingError::Bounds,
        _ => BackingError::Integrity,
    }
}
impl AccountedSelection {
    #[allow(clippy::too_many_arguments)]
    pub fn try_from_native(
        format: PixelFormat,
        extent: Rect,
        default: &[u8],
        native: &[u8],
        coverage_domain: CoverageDomain,
        quota: &BackingQuota,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        control.check(1)?;
        valid(format)?;
        match coverage_domain {
            CoverageDomain::FiniteUnit => {}
        }
        let count = bounds(extent)?;
        let bpp = format.bytes_per_pixel();
        let length = mul(mul(u64::from(extent.width()), u64::from(extent.height()))?, bpp as u64)?;
        if length > MAX_CHECKED_RASTER_BYTES || native.len() as u64 != length || default.len() != bpp {
            return Err(BackingError::Length);
        }
        // Finite-unit default and every logical native word precede private output.
        // Integer codes are intrinsically normalized unit coverage; no f32 ingress.
        let tile_bytes = mul(65536, bpp as u64)?;
        let payload = add(mul(count, tile_bytes)?, default.len() as u64)?;
        let index = add(mul(count, add(1024, std::mem::size_of::<Tile>() as u64 + 128)?)?, std::mem::size_of::<SelectionOwner>() as u64)?;
        let allocation = (std::mem::size_of::<SelectionOwner>() + std::mem::size_of::<QuotaLease>() + 256) as u64;
        let payload_lease = quota.reserve(QuotaKind::Resident, payload).map_err(|_| BackingError::Quota)?;
        let index_lease = quota.reserve(QuotaKind::Index, index).map_err(|_| BackingError::Quota)?;
        let allocation_lease = quota.reserve(QuotaKind::Index, allocation).map_err(|_| BackingError::Quota)?;
        // Native writer may retain a staged map/default and one whole-tile copy/fill.
        let scratch_bytes = add(mul(tile_bytes, 4)?, add(mul(count, 1024)?, default.len() as u64 + 4096)?)?;
        let scratch_lease = quota.reserve(QuotaKind::Scratch, scratch_bytes).map_err(|_| BackingError::Quota)?;
        coverage(format, default, control)?;
        coverage(format, native, control)?;
        let mut surface = Surface::try_with_default_bytes(format, default).map_err(raster)?;
        let mut raw = Vec::new();
        let capacity = usize::try_from(tile_bytes).map_err(|_| BackingError::Length)?;
        control.check(tile_bytes)?;
        raw.try_reserve_exact(capacity).map_err(|_| BackingError::Allocation)?;
        if raw.capacity() > capacity.checked_mul(2).ok_or(BackingError::Length)? {
            return Err(BackingError::Allocation);
        }
        let width = usize::try_from(extent.width()).map_err(|_| BackingError::Length)?;
        for coordinate in extent.tiles() {
            control.check(1)?;
            let part = coordinate.rect().intersect(&extent);
            let part_width = usize::try_from(part.width()).map_err(|_| BackingError::Length)?;
            let part_height = usize::try_from(part.height()).map_err(|_| BackingError::Length)?;
            let span = part_width.checked_mul(bpp).ok_or(BackingError::Length)?;
            let part_bytes = span.checked_mul(part_height).ok_or(BackingError::Length)?;
            raw.clear();
            while raw.len() < part_bytes {
                let next = raw.len().checked_add(64 * 1024).ok_or(BackingError::Length)?.min(part_bytes);
                control.check((next - raw.len()) as u64)?;
                raw.resize(next, 0);
            }
            for (row, y) in (part.y0..part.y1).enumerate() {
                control.check(span as u64)?;
                let iy = usize::try_from(i64::from(y) - i64::from(extent.y0)).map_err(|_| BackingError::Bounds)?;
                let ix = usize::try_from(i64::from(part.x0) - i64::from(extent.x0)).map_err(|_| BackingError::Bounds)?;
                let start = iy.checked_mul(width).and_then(|v| v.checked_add(ix)).and_then(|v| v.checked_mul(bpp)).ok_or(BackingError::Length)?;
                let stop = start.checked_add(span).ok_or(BackingError::Length)?;
                let target = row.checked_mul(span).ok_or(BackingError::Length)?;
                raw.get_mut(target..target.checked_add(span).ok_or(BackingError::Length)?)
                    .ok_or(BackingError::Length)?
                    .copy_from_slice(native.get(start..stop).ok_or(BackingError::Length)?);
            }
            // Existing writer polls at tile boundaries; one full native fill/copy is
            // bounded by512KiB for this closed Gray domain and precharged before entry.
            control.check(add(mul(tile_bytes, 3)?, mul(surface.tile_count() as u64 + 1, 1024)?)?)?;
            surface.try_write_interleaved_with(part, &raw, control.interrupt).map_err(raster)?;
        }
        control.check(1)?;
        drop(raw);
        drop(scratch_lease);
        // All physical tiles are fresh; borrowed caller input cannot mutate them.
        Ok(Self {
            owner: Arc::new(SelectionOwner { surface, bounds: extent, quota: quota.clone(), _retention: None, _payload: Some(payload_lease), _index: Some(index_lease) }),
            _allocation: Arc::new(allocation_lease),
        })
    }
    pub(crate) fn surface(&self) -> &Surface {
        &self.owner.surface
    }
    pub fn format(&self) -> PixelFormat {
        self.owner.surface.format()
    }
    pub fn bounds(&self) -> Rect {
        self.owner.bounds
    }
    pub fn quota(&self) -> &BackingQuota {
        &self.owner.quota
    }
    pub(crate) fn reserve_view(&self, control: &ReadControl<'_>) -> Result<SelectionViewCharges, BackingError> {
        control.check(1)?;
        let _default = self.quota().reserve(QuotaKind::Resident, self.surface().default_bytes().len() as u64).map_err(|_| BackingError::Quota)?;
        let index = add(mul(self.surface().tile_count() as u64, 1024)?, std::mem::size_of::<Surface>() as u64)?;
        let _index = self.quota().reserve(QuotaKind::Index, index).map_err(|_| BackingError::Quota)?;
        Ok(SelectionViewCharges { _default, _index })
    }
    pub(crate) fn clone_view(&self, control: &ReadControl<'_>) -> Result<SelectionView, BackingError> {
        let charges = self.reserve_view(control)?;
        let mut surface = Surface::try_with_default_bytes(self.format(), self.surface().default_bytes()).map_err(raster)?;
        for (coord, tile) in self.surface().tiles() {
            control.check(1024)?;
            surface.put_tiles(std::iter::once((*coord, tile.clone())));
        }
        control.check(1)?;
        Ok(SelectionView { surface, charges })
    }
    /// Exact borrowed native storage equality for no-op decisions only. This
    /// grants no source, ledger, installation or mutation proof. The caller owns
    /// the borrowed surface; this owner retains its own charged storage throughout.
    pub fn matches_native_surface(&self, surface: &Surface, control: &ReadControl<'_>) -> Result<bool, BackingError> {
        control.check(1)?;
        if surface.format() != self.format() || surface.default_bytes() != self.surface().default_bytes() || surface.tile_count() != self.surface().tile_count()
        {
            return Ok(false);
        }
        for ((a, left), (b, right)) in surface.tiles().zip(self.surface().tiles()) {
            control.check(1)?;
            if a != b || left.bytes().len() != right.bytes().len() {
                return Ok(false);
            }
            for (left, right) in left.bytes().chunks(64 * 1024).zip(right.bytes().chunks(64 * 1024)) {
                control.check(left.len() as u64)?;
                if left != right {
                    return Ok(false);
                }
            }
        }
        control.check(1)?;
        Ok(true)
    }
    pub(crate) fn matches(&self, surface: &Surface, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<bool, BackingError> {
        control.check(1)?;
        if !self.quota().shares_ledger(quota)
            || surface.format() != self.format()
            || surface.default_bytes() != self.surface().default_bytes()
            || surface.tile_count() != self.surface().tile_count()
        {
            return Ok(false);
        }
        for ((a, ta), (b, tb)) in surface.tiles().zip(self.surface().tiles()) {
            control.check(1)?;
            if a != b || !Arc::ptr_eq(ta, tb) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
#[cfg(test)]
#[path = "accounted_selection_tests.rs"]
mod tests;
