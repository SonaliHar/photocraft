//! Charged capture from an already authorized reader into the existing owned
//! plain-XCF class. No path, resource locator or profile policy is resolved here.
use super::{OwnedPlainXcfImport, backing, import_owned_staged_plain_bytes_in_quota};
use crate::IoError;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use photocraft_xcf::{BudgetKind, DecodeOptions, ErrorKind, ResourceLimits, XcfError};
use std::{io::Read, sync::Arc};

const CHUNK: usize = 65_536;
const CAPACITY_SLACK: usize = 64;
const OWNER_SLACK: usize = 128;
const INTERRUPTED_RETRIES: u8 = 8;

fn error(kind: ErrorKind, message: &str) -> IoError {
    IoError::Xcf(XcfError { kind, offset: 0, item: String::new(), message: message.into() })
}
fn check(control: &ReadControl<'_>, options: &DecodeOptions<'_>, units: u64) -> Result<(), IoError> {
    control.check(units).map_err(backing)?;
    if options.cancelled.is_some_and(|cancelled| cancelled()) {
        return Err(IoError::Cancelled);
    }
    Ok(())
}
// Declaration order is significant on every failed capture/promotion: storage
// is destroyed before its genuine same-ledger reservation is released.
struct Capturing {
    bytes: Vec<u8>,
    lease: QuotaLease,
}
struct Captured {
    bytes: Arc<Vec<u8>>,
    _lease: QuotaLease,
}
impl Captured {
    fn read(
        reader: &mut (impl Read + ?Sized),
        exact_length: u64,
        options: &DecodeOptions<'_>,
        quota: &BackingQuota,
        resident_limit: u64,
        control: &ReadControl<'_>,
    ) -> Result<Self, IoError> {
        let length = usize::try_from(exact_length).map_err(|_| backing(BackingError::Length))?;
        let capacity_limit = length.checked_add(CAPACITY_SLACK).ok_or_else(|| backing(BackingError::Length))?;
        // Vec/Arc payload/header plus the stack capture headers are conservatively
        // admitted before either heap allocation. Arc initialization remains the
        // existing standard-library allocator boundary, not an RSS guarantee.
        let header = std::mem::size_of::<Capturing>()
            .checked_add(std::mem::size_of::<Captured>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Vec<u8>>()))
            .and_then(|n| n.checked_add(OWNER_SLACK))
            .ok_or_else(|| backing(BackingError::Length))?;
        let envelope = capacity_limit.checked_add(header).ok_or_else(|| backing(BackingError::Length))?;
        if envelope as u64 > resident_limit {
            return Err(error(ErrorKind::Limit, "owned XCF reader capture exceeds caller resident limits"));
        }
        let lease = quota.reserve(QuotaKind::Resident, envelope as u64).map_err(|_| backing(BackingError::Quota))?;
        let mut capture = Capturing { bytes: Vec::new(), lease };
        check(control, options, 1)?;
        capture.bytes.try_reserve_exact(length).map_err(|_| backing(BackingError::Allocation))?;
        if capture.bytes.capacity() > capacity_limit {
            return Err(backing(BackingError::Allocation));
        }
        let actual = capture.bytes.capacity().checked_add(header).ok_or_else(|| backing(BackingError::Length))?;
        capture.lease.reconcile_capacity(actual as u64).map_err(|_| backing(BackingError::Quota))?;
        while capture.bytes.len() < length {
            let begin = capture.bytes.len();
            let count = CHUNK.min(length - begin);
            let end = begin.checked_add(count).ok_or_else(|| backing(BackingError::Length))?;
            check(control, options, (count as u64).checked_mul(2).ok_or_else(|| backing(BackingError::Length))?)?;
            capture.bytes.resize(end, 0);
            let output = capture.bytes.get_mut(begin..end).ok_or_else(|| backing(BackingError::Bounds))?;
            read_chunk(reader, output, options, control)?;
        }
        let mut extra = [0u8; 1];
        let mut interrupted = 0u8;
        loop {
            check(control, options, 1)?;
            match reader.read(&mut extra) {
                Ok(0) => break,
                Ok(_) => return Err(error(ErrorKind::Structure, "owned XCF reader exceeds its declared exact length")),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted && interrupted < INTERRUPTED_RETRIES => interrupted += 1,
                Err(_) => return Err(error(ErrorKind::Structure, "owned XCF reader failed at exact EOF")),
            }
        }
        check(control, options, 1)?;
        let Capturing { bytes, lease } = capture;
        Ok(Self { bytes: Arc::new(bytes), _lease: lease })
    }
}
fn read_chunk(reader: &mut (impl Read + ?Sized), output: &mut [u8], options: &DecodeOptions<'_>, control: &ReadControl<'_>) -> Result<(), IoError> {
    if output.len() > CHUNK {
        return Err(backing(BackingError::Bounds));
    }
    let mut filled = 0usize;
    let mut interrupted = 0u8;
    while filled < output.len() {
        check(control, options, 1)?;
        let remaining = output.get_mut(filled..).ok_or_else(|| backing(BackingError::Bounds))?;
        match reader.read(remaining) {
            Ok(0) => return Err(error(ErrorKind::Truncated, "owned XCF reader ended before its declared exact length")),
            Ok(count) if count <= remaining.len() => {
                filled = filled.checked_add(count).ok_or_else(|| backing(BackingError::Length))?;
                interrupted = 0;
            }
            Ok(_) => return Err(backing(BackingError::Bounds)),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted && interrupted < INTERRUPTED_RETRIES => interrupted += 1,
            Err(_) => return Err(error(ErrorKind::Structure, "owned XCF reader failed during capture")),
        }
    }
    check(control, options, 0)
}

/// Capture at the current reader position and require exact EOF. The host owns
/// and closes its authorized reader; this API grants no reads from saved locators
/// and does not assert an atomic external file snapshot. Read syscalls remain a
/// provider boundary; hosts must reject nonregular/blocking-special-file inputs.
///
/// Caller limits must explicitly select the existing scratch-free memory route.
/// Input capacity/header is reserved before allocation on the supplied actual
/// catalog quota. That capture charge overlaps the importer's genuine source and
/// preparation receipts throughout decoding/handoff; it is released only after
/// the returned owner has retained those receipts. No work is reset or refunded.
/// Existing source eligibility and profile/metadata interpretation are unchanged.
pub fn import_owned_staged_plain_from_reader_in_quota(
    name: &str,
    reader: &mut (impl Read + ?Sized),
    exact_length: u64,
    options: &DecodeOptions<'_>,
    limits: &ResourceLimits,
    quota: BackingQuota,
    control: &ReadControl<'_>,
) -> Result<OwnedPlainXcfImport, IoError> {
    check(control, options, 1)?;
    let cap = limits.get(BudgetKind::Encoded).min(limits.get(BudgetKind::Logical)).min(options.limits.input_bytes as u64);
    if name.len() > 1 << 20 || exact_length == 0 || exact_length > cap {
        return Err(error(ErrorKind::Limit, "owned XCF reader name or encoded length exceeds caller limits"));
    }
    if limits.get(BudgetKind::Scratch) != 0 {
        return Err(error(ErrorKind::Unsupported, "owned XCF reader requires explicit scratchBytes:0 memory limits"));
    }
    let resident_limit = limits.get(BudgetKind::Resident).min(options.limits.resident_bytes as u64);
    let capture = Captured::read(reader, exact_length, options, &quota, resident_limit, control)?;
    check(control, options, 1)?;
    let result = import_owned_staged_plain_bytes_in_quota(name, capture.bytes.clone(), options, limits, quota, control);
    // This overlapping reservation must outlive the entire import call, including
    // all failed private phases. The importer retains its own actual source owner.
    drop(capture);
    result
}

#[cfg(test)]
#[path = "reader_tests.rs"]
mod tests;
