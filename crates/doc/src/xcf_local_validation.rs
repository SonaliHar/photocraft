//! Private selected-purpose synchronous polling. Public Interrupt remains Sync.
use crate::{Document, FilterMask, WorkingProfileInterpretation, XcfValidationError};
use photocraft_raster::{Cancelled, Interrupt, backing::BackingError};
use std::cell::Cell;
fn invalid() -> XcfValidationError {
    XcfValidationError::new("cancelled", "selected validation interrupted")
}
pub(crate) trait ValidationControl {
    fn interrupt(&self) -> &Interrupt<'_>;
    fn cancelled(&self) -> bool {
        self.interrupt().cancelled()
    }
    fn check(&self) -> Result<(), Cancelled> {
        if self.cancelled() { Err(Cancelled) } else { Ok(()) }
    }
    fn hash(&self, bytes: &[u8]) -> Result<blake3::Hash, XcfValidationError> {
        crate::xcf_items::checked_payload_hash(bytes, self.interrupt())
    }
    fn profile(&self, d: &Document) -> Result<(), XcfValidationError> {
        d.working_profile_interpretation.validate_with_interrupt(d.icc_profile.as_deref().map(Vec::as_slice), d.mode, self.interrupt())
    }
    fn mask(&self, m: &FilterMask) -> Result<(), XcfValidationError> {
        m.validate_with(self.interrupt())
    }
}
impl ValidationControl for Interrupt<'_> {
    fn interrupt(&self) -> &Interrupt<'_> {
        self
    }
}
pub(crate) struct LocalValidation<'a, 'i> {
    pub interrupt: &'a Interrupt<'i>,
    pub work: &'a dyn Fn(u64) -> Result<(), BackingError>,
    pub failure: Cell<Option<BackingError>>,
}
impl ValidationControl for LocalValidation<'_, '_> {
    fn interrupt(&self) -> &Interrupt<'_> {
        self.interrupt
    }
    fn cancelled(&self) -> bool {
        if self.interrupt.cancelled() {
            self.failure.set(Some(BackingError::Cancelled));
            return true;
        }
        if self.failure.get().is_some() {
            return true;
        }
        if let Err(e) = (self.work)(0) {
            self.failure.set(Some(e));
            return true;
        }
        false
    }
    fn hash(&self, bytes: &[u8]) -> Result<blake3::Hash, XcfValidationError> {
        let mut hash = blake3::Hasher::new();
        for chunk in bytes.chunks(32768) {
            if self.cancelled() {
                return Err(invalid());
            }
            hash.update(chunk);
        }
        if self.cancelled() {
            return Err(invalid());
        }
        Ok(hash.finalize())
    }
    fn profile(&self, d: &Document) -> Result<(), XcfValidationError> {
        let (hash, valid) = match &d.working_profile_interpretation {
            WorkingProfileInterpretation::Icc => return Ok(()),
            WorkingProfileInterpretation::ImplicitSrgb { icc_blake3 } => (icc_blake3, matches!(d.mode, crate::ColorMode::Rgb | crate::ColorMode::Indexed)),
            WorkingProfileInterpretation::ImplicitSGray { icc_blake3 } => (icc_blake3, d.mode == crate::ColorMode::Grayscale),
        };
        let bytes = d.icc_profile.as_deref().ok_or_else(|| XcfValidationError::new("working_profile_identity", "missing profile"))?;
        if !valid
            || bytes.is_empty()
            || bytes.len() > 64 << 20
            || hash.len() != 64
            || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            || self.hash(bytes)?.to_hex().as_str() != hash
        {
            return Err(XcfValidationError::new("working_profile_identity", "stale or incompatible current profile"));
        }
        Ok(())
    }
    fn mask(&self, m: &FilterMask) -> Result<(), XcfValidationError> {
        // The hierarchy phase has already admitted byte traversal and96/sample
        // scalar validation. Poll the original local budget without redebiting.
        m.validate_checked_f32_coverage_with_local_work(self.interrupt, &|_| {
            if self.cancelled() { Err(self.failure.get().unwrap_or(BackingError::Cancelled)) } else { Ok(()) }
        })
        .map(|_| ())
        .map_err(|e| {
            self.failure.set(Some(e));
            invalid()
        })
    }
}
