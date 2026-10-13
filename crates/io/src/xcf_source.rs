//! Owned input snapshots. Source locators never grant filesystem or resource access here.
use photocraft_xcf::{Budget, BudgetKind, ByteSource, ErrorKind, Reservation, Source, XcfError};
#[cfg(not(target_arch = "wasm32"))]
mod scratch;
#[cfg(not(target_arch = "wasm32"))]
pub use scratch::{NativeScratchHandle, OwnedScratchXcfSnapshot, ScratchHandle, ServiceAuthorizedScratchDirectory};

/// Outer signatures that warrant bounded inspection before replacing an
/// existing destination. A wrapper alone does not establish XCF identity.
pub fn needs_content_inspection(prefix: &[u8]) -> bool {
    photocraft_xcf::classify_container(prefix).is_some()
}

/// Definitively classify a plain or strictly decoded wrapped XCF. This parses
/// inert records only; it performs no mapping, resource read or evaluation.
/// A malformed candidate is an error, allowing save services to fail closed.
pub fn contains_xcf(bytes: &[u8]) -> Result<bool, XcfError> {
    contains_xcf_with_interrupt(bytes, &photocraft_raster::Interrupt::NONE)
}

pub fn contains_xcf_with_interrupt(bytes: &[u8], interrupt: &photocraft_raster::Interrupt<'_>) -> Result<bool, XcfError> {
    contains_xcf_with_retained_capacity(bytes, bytes.len(), interrupt)
}

/// The decoder already charges its borrowed input length. Reserve any additional
/// caller Vec capacity by lowering the same resident ceiling, not raising it or
/// charging the input twice. This is declared retention, not process RSS.
pub(crate) fn contains_xcf_with_retained_capacity(
    bytes: &[u8],
    retained_capacity: usize,
    interrupt: &photocraft_raster::Interrupt<'_>,
) -> Result<bool, XcfError> {
    contains_xcf_with_retained_capacity_and_resident_limit(bytes, retained_capacity, photocraft_xcf::Limits::default().resident_bytes as u64, interrupt)
}

/// Caller-reserved shared retention envelope, including the encoded Vec capacity.
/// No source/decoder component limit is raised and unknown wrappers fail closed.
pub(crate) fn contains_xcf_with_retained_capacity_and_resident_limit(
    bytes: &[u8], retained_capacity: usize, resident_limit: u64,
    interrupt: &photocraft_raster::Interrupt<'_>,
) -> Result<bool, XcfError> {
    let poll = || interrupt.check().map_err(|_| error(ErrorKind::Cancelled, "source classification cancelled"));
    poll()?;
    let additional = retained_capacity.checked_sub(bytes.len()).ok_or_else(|| error(ErrorKind::Limit, "source retained capacity is smaller than length"))?;
    if photocraft_xcf::is_xcf(bytes) {
        poll()?;
        return Ok(true);
    }
    let limits = photocraft_xcf::ResourceLimits::for_policy(photocraft_xcf::ResourcePolicy::default())?;
    if resident_limit > limits.get(BudgetKind::Resident) || retained_capacity as u64 > resident_limit {
        return Err(error(ErrorKind::Limit, "source inspection exceeds admitted resident envelope"));
    }
    let resident = resident_limit
        .checked_sub(additional as u64)
        .ok_or_else(|| error(ErrorKind::Limit, "source retained capacity exceeds resident ceiling"))?;
    let limits = limits.lowered(&[("residentBytes", resident)])?;
    let cancelled = || interrupt.cancelled();
    let options = photocraft_xcf::DecodeOptions { cancelled: Some(&cancelled), ..Default::default() };
    let result = photocraft_xcf::decode_candidate(bytes, &options, &limits);
    poll()?;
    let contains = result?.is_some();
    poll()?;
    Ok(contains)
}

pub fn encoded_limit() -> u64 {
    photocraft_xcf::Limits::default().input_bytes as u64
}

fn error(kind: ErrorKind, message: &str) -> XcfError {
    XcfError { kind, offset: 0, item: String::new(), message: message.into() }
}

/// A transaction-owned, immutable copy of admitted source bytes. Budget reservations stay
/// live for its entire lifetime, including decoding. It never reopens an ambient source path.
pub struct OwnedXcfSnapshot<'b, 'a> {
    bytes: Vec<u8>,
    _encoded: Option<Reservation<'b, 'a>>,
    _resident: Option<Reservation<'b, 'a>>,
}

impl<'b, 'a> OwnedXcfSnapshot<'b, 'a> {
    /// Capture an already authorized source into owned bytes. This guarantees independence
    /// from changes after capture; the source service is responsible for its read authority
    /// and consistency during capture. Existing resident inputs must be charged by their owner.
    pub fn from_source(source: &mut dyn Source, budget: &'b Budget<'a>) -> Result<Self, XcfError> {
        let len = source.len();
        let mut out = Self { bytes: Vec::new(), _encoded: None, _resident: None };
        out._encoded = Some(budget.reserve(BudgetKind::Encoded, len, "snapshot admission")?);
        let envelope = len.checked_mul(2).ok_or_else(|| error(ErrorKind::Limit, "snapshot capacity envelope overflow"))?;
        out._resident = Some(budget.reserve(BudgetKind::Resident, envelope, "snapshot allocation envelope")?);
        let size = usize::try_from(len).map_err(|_| error(ErrorKind::Limit, "snapshot exceeds address space"))?;
        budget.poll("snapshot allocation")?;
        let allocation = out.bytes.try_reserve_exact(size);
        budget.poll("snapshot allocation completed")?;
        allocation.map_err(|_| error(ErrorKind::Limit, "snapshot allocation failed"))?;
        let capacity = out.bytes.capacity() as u64;
        if capacity > envelope {
            return Err(error(ErrorKind::Limit, "snapshot allocation capacity exceeds envelope"));
        }
        if let Some(resident) = out._resident.as_mut() {
            resident.resize(capacity, "snapshot actual capacity")?;
        }
        let mut offset = 0u64;
        // Initialize and capture one bounded slice at a time. Zeroing the whole
        // admitted source before the first poll can stall a cancelled import.
        while out.bytes.len() < size {
            let start = out.bytes.len();
            let end = start.saturating_add(4096).min(size);
            budget.consume_work((end - start) as u64, "snapshot capture")?;
            out.bytes.resize(end, 0);
            let chunk = out.bytes.get_mut(start..end).ok_or_else(|| error(ErrorKind::Structure, "snapshot initialization bounds"))?;
            let read = source.read_exact_at(offset, chunk, budget);
            budget.poll("snapshot source read completed")?;
            read?;
            offset = offset.checked_add(chunk.len() as u64).ok_or_else(|| error(ErrorKind::Structure, "snapshot offset overflow"))?;
            budget.poll("snapshot capture slice completed")?;
        }
        budget.poll("snapshot completed")?;
        Ok(out)
    }

    /// Transfer caller-owned bytes without making a second source copy. Admission is checked
    /// before the snapshot becomes available; no caller can mutate the moved buffer afterward.
    pub fn from_owned(bytes: Vec<u8>, budget: &'b Budget<'a>) -> Result<Self, XcfError> {
        let mut out = Self { bytes, _encoded: None, _resident: None };
        let len = out.bytes.len() as u64;
        out._encoded = Some(budget.reserve(BudgetKind::Encoded, len, "owned snapshot admission")?);
        out._resident = Some(budget.reserve(BudgetKind::Resident, out.bytes.capacity() as u64, "owned snapshot retained capacity")?);
        budget.poll("owned snapshot completed")?;
        Ok(out)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl Source for OwnedXcfSnapshot<'_, '_> {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_exact_at(&mut self, offset: u64, output: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
        ByteSource::new(&self.bytes).read_exact_at(offset, output, budget)
    }
}

#[cfg(test)]
mod classification_cancellation_tests {
    use super::*;
    #[test]
    fn buffered_candidate_decoder_observes_cancelled_callback() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::SeqCst) > 0;
        let interrupt = photocraft_raster::Interrupt::cancel_only(&cancel);
        let bytes = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0];
        assert!(matches!(contains_xcf_with_interrupt(&bytes,&interrupt),Err(error) if error.kind == ErrorKind::Cancelled));
        assert_eq!(bytes, [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0]);
    }
    #[test]
    fn excess_retained_capacity_cannot_raise_decoder_ceiling() {
        let limits = photocraft_xcf::ResourceLimits::for_policy(photocraft_xcf::ResourcePolicy::default()).unwrap();
        let excess = usize::try_from(limits.get(BudgetKind::Resident)).unwrap().checked_add(1).unwrap();
        assert!(matches!(contains_xcf_with_retained_capacity(&[],excess,&photocraft_raster::Interrupt::NONE),Err(error) if error.kind == ErrorKind::Limit));
    }
}
