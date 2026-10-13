//! STAGED bridge to the existing manifest/object lookup policy. This is a
//! Source adapter, not a persistent Document filesystem capability.
use super::accounting::RetainedReservations;
use crate::{
    FormatError, Result,
    canonical_archive::{ImmutableCanonicalArchive, ReadAtCursor},
    zip::stream::{Limits, Reader},
};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use std::{cell::RefCell, io::Write, sync::Arc};

fn error(error: BackingError) -> FormatError {
    FormatError::Io(std::io::Error::other(error))
}

/// Exact capacity is prepaid before ZIP copy. The completed buffer's token is
/// retained through Loader mapping because Source::get cannot return an owner.
/// This intentionally conservatively retains encoded buffers' capacity until
/// the owned model is dropped; it does not pretend early temporary refunds.
pub(crate) struct OwnedArchiveSource<'a, 'c> {
    archive: Arc<dyn ImmutableCanonicalArchive>,
    quota: BackingQuota,
    limits: Limits,
    maximum_object: usize,
    control: &'a ReadControl<'c>,
    ownership: &'a RefCell<RetainedReservations>,
}
impl<'a, 'c> OwnedArchiveSource<'a, 'c> {
    pub(crate) fn new(
        archive: Arc<dyn ImmutableCanonicalArchive>,
        quota: BackingQuota,
        limits: Limits,
        maximum_object: usize,
        control: &'a ReadControl<'c>,
        ownership: &'a RefCell<RetainedReservations>,
    ) -> Result<Self> {
        if maximum_object == 0 || maximum_object > 16 << 20 {
            return Err(FormatError::Unsupported("owned object input capacity".into()));
        }
        control.check(0).map_err(error)?;
        archive.validate_quota(&quota).map_err(error)?;
        if !ownership.try_borrow().map_err(|_| error(BackingError::Work))?.quota().shares_ledger(&quota) {
            return Err(error(BackingError::Quota));
        }
        Ok(Self { archive, quota, limits, maximum_object, control, ownership })
    }
}
struct Staging {
    bytes: Vec<u8>,
    _lease: QuotaLease,
    maximum: usize,
}
impl Write for Staging {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let next = self.bytes.len().checked_add(bytes.len()).ok_or_else(|| std::io::Error::other(BackingError::Length))?;
        if next > self.maximum || next > self.bytes.capacity() {
            return Err(std::io::Error::other(BackingError::Length));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl crate::store::Source for OwnedArchiveSource<'_, '_> {
    fn model_control(&self) -> Option<&ReadControl<'_>> {
        Some(self.control)
    }
    fn get(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        self.control.check(0).map_err(error)?;
        let max = max.min(self.maximum_object);
        let mut reader = Reader::new_accounted(ReadAtCursor::new(self.archive.as_ref(), self.control), self.limits, self.control, &self.quota)?;
        // STORE is the existing native archive contract. A compressed outer
        // entry is refused before allocating/copying; inner Zstd stays supported.
        let length = usize::try_from(reader.stored_size(path)?).map_err(|_| error(BackingError::Length))?;
        if length > max {
            return Err(FormatError::LimitExceeded("owned object encoded capacity".into()));
        }
        let envelope = length.checked_add(16).ok_or_else(|| error(BackingError::Length))?;
        let mut lease = self.quota.reserve(QuotaKind::Resident, envelope as u64).map_err(|_| error(BackingError::Quota))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| error(BackingError::Allocation))?;
        if bytes.capacity() > envelope {
            drop(bytes);
            return Err(error(BackingError::Allocation));
        }
        lease.reconcile_capacity(bytes.capacity() as u64).map_err(|_| error(BackingError::Quota))?;
        let mut staged = Staging { bytes, _lease: lease, maximum: length };
        reader.copy_by_name(path, &mut staged, length as u64, self.control.interrupt)?;
        self.control.check(length as u64).map_err(error)?;
        if staged.bytes.len() != length {
            return Err(error(BackingError::Integrity));
        }
        let Staging { bytes, _lease: lease, .. } = staged;
        // Reservation transfer cannot call arbitrary callbacks under RefCell:
        // use a zero-work control after checking the actual operation above.
        let no_work = |_| Ok(());
        let transfer = ReadControl { interrupt: self.control.interrupt, work: &no_work };
        let retained = match self.ownership.try_borrow_mut() {
            Ok(mut ownership) => ownership.retain(lease, &transfer),
            Err(_) => {
                drop(bytes);
                drop(lease);
                return Err(error(BackingError::Work));
            }
        };
        if let Err((failure, lease)) = retained {
            drop(bytes);
            drop(lease);
            return Err(error(failure));
        }
        if path == "manifest.json" {
            // Raw text bounds decoded string payloads; each object bounds one
            // largest inline document/manifest construction slot across the
            // Value -> Manifest -> Document overlap. Std map/Arc owner overhead
            // remains the documented conservative logical envelope limitation.
            let mut objects = 0_u64;
            let mut quoted = false;
            let mut escaped = false;
            for chunk in bytes.chunks(4096) {
                self.control.check(chunk.len() as u64).map_err(error)?;
                for byte in chunk {
                    if quoted {
                        if escaped {
                            escaped = false;
                        } else if *byte == b'\\' {
                            escaped = true;
                        } else if *byte == b'"' {
                            quoted = false;
                        }
                    } else if *byte == b'"' {
                        quoted = true;
                    } else if *byte == b'{' {
                        objects = objects.checked_add(1).ok_or_else(|| error(BackingError::Length))?;
                    }
                }
            }
            let slot = std::mem::size_of::<photocraft_doc::Document>()
                .max(std::mem::size_of::<photocraft_doc::Layer>())
                .max(std::mem::size_of::<crate::manifest::DocM>())
                .max(std::mem::size_of::<crate::manifest::LayerM>())
                .max(std::mem::size_of::<crate::manifest::Manifest>()) as u64;
            let model = (bytes.len() as u64)
                .checked_mul(64)
                .and_then(|n| objects.checked_mul(slot.checked_add(256)?).and_then(|m| m.checked_mul(4)).and_then(|m| n.checked_add(m)))
                .and_then(|n| n.checked_add(65536))
                .ok_or_else(|| error(BackingError::Length))?;
            let work = |_| Ok(());
            let transfer = ReadControl { interrupt: self.control.interrupt, work: &work };
            self.ownership.try_borrow_mut().map_err(|_| error(BackingError::Work))?.reserve(model, &transfer).map_err(error)?;
        }
        self.control.check(0).map_err(error)?;
        Ok(bytes)
    }
}
