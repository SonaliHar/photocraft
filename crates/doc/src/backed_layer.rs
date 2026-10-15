//! Preparatory document ownership of an immutable native backing and COW edits.
//! Immutable backing ownership carries no persistence grant. No IO occurs in equality.
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::backing::{BackedRaster, BackingError, NativePlaneDescriptor, ReadControl};
use photocraft_raster::checked_view::CheckedRasterView;

/// Exact operative descriptor mismatch or an actionable backing operation error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackedLayerError {
    DescriptorMismatch,
    Backing(BackingError),
}
impl std::fmt::Display for BackedLayerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "backed layer: {self:?}")
    }
}
impl std::error::Error for BackedLayerError {}

/// Owned native storage, persistent quota leases and exact COW overrides.
/// Cloning shares immutable physical payloads (suitable for history ownership).
/// Equality is conservative shared-state identity: separately captured equivalent
/// stores or separately written equivalent overrides may compare unequal. Use
/// try_content_equal for an explicit fallible comparison; never perform IO in Eq.
#[derive(Clone, Debug)]
pub struct BackedLayer {
    raster: BackedRaster,
    descriptor: NativePlaneDescriptor,
}
impl PartialEq for BackedLayer {
    fn eq(&self, other: &Self) -> bool {
        self.raster.shares_exact_state(&other.raster)
    }
}
impl Eq for BackedLayer {}

impl BackedLayer {
    /// Match the caller's operative native format, bounds and exact default bits.
    /// This checks layout identity, not color/profile/sample-domain authority.
    /// No job callback is stored. Storage must already satisfy NativePlaneStore's
    /// immutable verified ownership contract; no path authority is inferred.
    pub fn new(raster: BackedRaster, expected: NativePlaneDescriptor, control: &ReadControl<'_>) -> Result<Self, BackedLayerError> {
        control.check(1).map_err(BackedLayerError::Backing)?;
        let actual = raster.descriptor();
        let validated = NativePlaneDescriptor::new(expected.format(), expected.bounds(), expected.default_bytes()).map_err(BackedLayerError::Backing)?;
        if actual != validated || expected != validated {
            return Err(BackedLayerError::DescriptorMismatch);
        }
        control.check(1).map_err(BackedLayerError::Backing)?;
        Ok(Self { raster, descriptor: validated })
    }
    pub fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    /// Reuse the actual backing ledger for derived outputs/preparation.
    pub fn quota(&self) -> &photocraft_raster::backing::quota::BackingQuota {
        self.raster.quota()
    }
    pub fn format(&self) -> PixelFormat {
        self.descriptor().format()
    }
    pub fn operative_bounds(&self) -> Rect {
        self.descriptor().bounds()
    }
    pub fn default_bytes(&self) -> &[u8] {
        self.descriptor.default_bytes()
    }
    /// The logical read methods reject outside operative_bounds; raw reads and
    /// pins expose storage only and must not be treated as drawable coverage.
    pub fn checked_view(&self) -> CheckedRasterView<'_> {
        CheckedRasterView::backed(&self.raster)
    }
    pub fn try_content_equal(&self, other: &Self, control: &ReadControl<'_>) -> Result<bool, BackedLayerError> {
        self.raster.try_content_equal(&other.raster, control).map_err(BackedLayerError::Backing)
    }
    /// Translate the operative coordinate view while retaining the original
    /// immutable storage, quota and COW history. Native format and default bits
    /// remain exact; this grants no source, sample-domain or rendering authority.
    pub fn try_translated(&self, dx: i32, dy: i32, control: &ReadControl<'_>) -> Result<Self, BackedLayerError> {
        let raster = self.raster.try_translated(dx, dy, control).map_err(BackedLayerError::Backing)?;
        let descriptor = raster.descriptor();
        Self::new(raster, descriptor, control)
    }
    /// Stage exact native bytes inside the operative extent. The original owner
    /// and history clones remain unchanged on IO, quota, work or cancellation.
    /// This does not admit a color/sample edit domain or infer finite alpha.
    pub fn try_edited_native(&self, bounds: Rect, bytes: &[u8], control: &ReadControl<'_>) -> Result<Self, BackedLayerError> {
        control.check(1).map_err(BackedLayerError::Backing)?;
        let mut raster = self.raster.clone();
        raster.write_region(bounds, bytes, control).map_err(BackedLayerError::Backing)?;
        control.check(1).map_err(BackedLayerError::Backing)?;
        Ok(Self { raster, descriptor: self.descriptor })
    }
    /// Publish only a successfully staged edit, after its final cancellation poll.
    pub fn try_write_native(&mut self, bounds: Rect, bytes: &[u8], control: &ReadControl<'_>) -> Result<(), BackedLayerError> {
        let staged = self.try_edited_native(bounds, bytes, control)?;
        *self = staged;
        Ok(())
    }
}
