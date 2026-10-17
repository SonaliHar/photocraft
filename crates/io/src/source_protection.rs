//! Existing-destination protection on an already-authorized reader. No paths or grants.
use std::io::{self, Read};

/// One original fingerprint plus the existing bounded provenance history.
pub const MAX_FINGERPRINTS: usize = 4097;
/// Standard inspection ceiling, exposed without granting any reader authority.
pub fn resident_ceiling() -> u64 {
    photocraft_xcf::Limits::default().resident_bytes as u64
}
/// Actual retained owned hash storage, including caller-known Vec capacity.
/// This performs no IO and grants no source authority.
pub fn fingerprint_resident_bytes(values: &[String], slots: usize, interrupt: &photocraft_raster::Interrupt<'_>) -> Result<usize, SourceProtectionError> {
    validate_fingerprints_with_interrupt(values, interrupt)?;
    if slots < values.len() {
        return Err(SourceProtectionError::InvalidFingerprint);
    }
    let mut bytes = slots
        .checked_mul(std::mem::size_of::<String>())
        .and_then(|n| n.checked_add(std::mem::size_of::<Vec<String>>()))
        .ok_or(SourceProtectionError::ResidentLimit)?;
    for value in values {
        poll(interrupt)?;
        bytes = bytes.checked_add(value.capacity()).ok_or(SourceProtectionError::ResidentLimit)?;
    }
    poll(interrupt)?;
    Ok(bytes)
}
const MAX_INTERRUPTED: usize = 1024;
const MAX_READ_CALLS: usize = 1_000_000;
const HEADER_BYTES: usize = 32;
const CHUNK_BYTES: usize = 64 << 10;

#[derive(Debug)]
pub enum SourceProtectionError {
    Cancelled,
    FingerprintLimit,
    InvalidFingerprint,
    ByteLimit,
    ResidentLimit,
    Allocation,
    Read(io::Error),
    InterruptedLimit,
    WorkLimit,
    InvalidReadCount,
    LengthChanged,
    XcfDestination,
    ProtectedFingerprint,
    MalformedCandidate,
}
impl std::fmt::Display for SourceProtectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "source-protection.cancelled",
            Self::FingerprintLimit => "source-protection.fingerprint-limit",
            Self::InvalidFingerprint => "source-protection.invalid-fingerprint",
            Self::ByteLimit => "source-protection.byte-limit",
            Self::ResidentLimit => {
                "source-protection.resident-limit: choose another destination; protected-content inspection exceeds the admitted save memory"
            }
            Self::Allocation => "source-protection.allocation",
            Self::Read(_) => "source-protection.read-failed",
            Self::InterruptedLimit => "source-protection.interrupted-limit",
            Self::WorkLimit => "source-protection.work-limit",
            Self::InvalidReadCount => "source-protection.invalid-read-count",
            Self::LengthChanged => "source-protection.length-changed",
            Self::XcfDestination => "source-protection.xcf-destination",
            Self::ProtectedFingerprint => "source-protection.protected-fingerprint",
            Self::MalformedCandidate => "source-protection.malformed-candidate",
        })
    }
}
impl std::error::Error for SourceProtectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            _ => None,
        }
    }
}

/// Metadata validation is independent of destination existence and performs no IO.
pub fn validate_fingerprints(values: &[String]) -> Result<(), SourceProtectionError> {
    validate_fingerprints_with_interrupt(values, &photocraft_raster::Interrupt::NONE)
}
fn poll(interrupt: &photocraft_raster::Interrupt<'_>) -> Result<(), SourceProtectionError> {
    interrupt.check().map_err(|_| SourceProtectionError::Cancelled)
}
/// Validate bounded inert hash metadata without filesystem access, cooperatively
/// polling each hash and completion. Cancellation never changes caller values.
pub fn validate_fingerprints_with_interrupt(values: &[String], interrupt: &photocraft_raster::Interrupt<'_>) -> Result<(), SourceProtectionError> {
    poll(interrupt)?;
    if values.len() > MAX_FINGERPRINTS {
        return Err(SourceProtectionError::FingerprintLimit);
    }
    for value in values {
        poll(interrupt)?;
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) || blake3::Hash::from_hex(value).is_err() {
            return Err(SourceProtectionError::InvalidFingerprint);
        }
    }
    poll(interrupt)?;
    Ok(())
}

#[derive(Default)]
struct Reads {
    calls: usize,
    interrupted: usize,
    bytes: u64,
}
impl Reads {
    fn read<R: Read>(&mut self, reader: &mut R, output: &mut [u8], interrupt: &photocraft_raster::Interrupt<'_>) -> Result<usize, SourceProtectionError> {
        loop {
            poll(interrupt)?;
            self.calls = self.calls.checked_add(1).filter(|n| *n <= MAX_READ_CALLS).ok_or(SourceProtectionError::WorkLimit)?;
            let read = reader.read(output);
            poll(interrupt)?;
            let count = match read {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                    self.interrupted = self.interrupted.checked_add(1).filter(|n| *n <= MAX_INTERRUPTED).ok_or(SourceProtectionError::InterruptedLimit)?;
                    continue;
                }
                Err(error) => return Err(SourceProtectionError::Read(error)),
                Ok(count) => count,
            };
            if count > output.len() {
                return Err(SourceProtectionError::InvalidReadCount);
            }
            self.bytes = self.bytes.checked_add(count as u64).ok_or(SourceProtectionError::ByteLimit)?;
            poll(interrupt)?;
            return Ok(count);
        }
    }
}

fn append(buffer: &mut Vec<u8>, bytes: &[u8], cap: u64, interrupt: &photocraft_raster::Interrupt<'_>) -> Result<(), SourceProtectionError> {
    poll(interrupt)?;
    let total = buffer.len().checked_add(bytes.len()).ok_or(SourceProtectionError::ByteLimit)?;
    if total as u64 > cap {
        return Err(SourceProtectionError::ByteLimit);
    }
    buffer.try_reserve_exact(bytes.len()).map_err(|_| SourceProtectionError::Allocation)?;
    if buffer.capacity() as u64 > cap {
        return Err(SourceProtectionError::ByteLimit);
    }
    poll(interrupt)?;
    buffer.extend_from_slice(bytes);
    poll(interrupt)?;
    Ok(())
}

/// Conservative fixed reader/hash/bookkeeping scratch, separate from wrapped
/// encoded capacity and the existing decoder's declared retention budget.
/// Allocator/private decoder overhead remains outside an RSS guarantee.
pub const SOURCE_PROTECTION_FIXED_BYTES: u64 =
    (CHUNK_BYTES + HEADER_BYTES + std::mem::size_of::<blake3::Hasher>() + std::mem::size_of::<Reads>()) as u64 + 4096;

/// Reader begins at byte zero and is owned/authorized by the caller. Full reads
/// reject length drift and use the existing XCF encoded cap. With no fingerprints,
/// a noncandidate may return after a bounded prefix without scanning the full file.
/// Blocking syscall cancellation and same-length mutation are not guaranteed here.
pub fn check_reader<R: Read>(reader: &mut R, expected_len: u64, protected_fingerprints: &[String]) -> Result<(), SourceProtectionError> {
    check_reader_with_interrupt(reader, expected_len, protected_fingerprints, &photocraft_raster::Interrupt::NONE)
}

/// Cooperative cancellation before/after reads, hashes, appends and decoding.
/// This cannot interrupt a blocked operating-system Read syscall.
pub fn check_reader_with_interrupt<R: Read>(
    reader: &mut R,
    expected_len: u64,
    protected_fingerprints: &[String],
    interrupt: &photocraft_raster::Interrupt<'_>,
) -> Result<(), SourceProtectionError> {
    check_reader_core(reader, expected_len, protected_fingerprints, None, interrupt)
}

/// Lower-only shared save-envelope admission. Reader authority is still entirely
/// caller-owned. A too-large wrapped destination is refused before collection,
/// not treated as safe or inspected with an independent full default budget.
pub fn check_reader_with_resident_limit<R: Read>(
    reader: &mut R,
    expected_len: u64,
    protected_fingerprints: &[String],
    resident_limit: u64,
    interrupt: &photocraft_raster::Interrupt<'_>,
) -> Result<(), SourceProtectionError> {
    poll(interrupt)?;
    if resident_limit > photocraft_xcf::Limits::default().resident_bytes as u64 {
        return Err(SourceProtectionError::ResidentLimit);
    }
    let remaining = resident_limit.checked_sub(SOURCE_PROTECTION_FIXED_BYTES).ok_or(SourceProtectionError::ResidentLimit)?;
    check_reader_core(reader, expected_len, protected_fingerprints, Some(remaining), interrupt)
}
fn check_reader_core<R: Read>(
    reader: &mut R,
    expected_len: u64,
    protected_fingerprints: &[String],
    resident_limit: Option<u64>,
    interrupt: &photocraft_raster::Interrupt<'_>,
) -> Result<(), SourceProtectionError> {
    validate_fingerprints_with_interrupt(protected_fingerprints, interrupt)?;
    let mut reads = Reads::default();
    let mut header = [0u8; HEADER_BYTES];
    let mut filled = 0usize;
    while filled < header.len() {
        poll(interrupt)?;
        let count = reads.read(reader, header.get_mut(filled..).ok_or(SourceProtectionError::InvalidReadCount)?, interrupt)?;
        if reads.bytes > expected_len {
            return Err(SourceProtectionError::LengthChanged);
        }
        if count == 0 {
            if reads.bytes != expected_len {
                return Err(SourceProtectionError::LengthChanged);
            }
            break;
        }
        filled = filled.checked_add(count).ok_or(SourceProtectionError::ByteLimit)?;
    }
    let prefix = header.get(..filled).ok_or(SourceProtectionError::InvalidReadCount)?;
    // Existing public magic predicate, not a duplicate parser/decoder.
    if photocraft_xcf::is_xcf(prefix) {
        poll(interrupt)?;
        return Err(SourceProtectionError::XcfDestination);
    }
    let candidate = crate::xcf_source::needs_content_inspection(prefix);
    if !candidate && protected_fingerprints.is_empty() {
        poll(interrupt)?;
        return Ok(());
    }
    let default_cap = crate::xcf_source::encoded_limit();
    if expected_len > default_cap {
        return Err(SourceProtectionError::ByteLimit);
    }
    let cap = if candidate { resident_limit.map_or(default_cap, |remaining| remaining.min(default_cap)) } else { default_cap };
    if candidate && expected_len > cap {
        return Err(SourceProtectionError::ResidentLimit);
    }
    let mut candidate_bytes = Vec::new();
    if candidate {
        if resident_limit.is_some() {
            // One admitted allocation, avoiding old+new growth allocations while
            // the prepared document and native output plan remain retained.
            let requested = usize::try_from(expected_len).map_err(|_| SourceProtectionError::ResidentLimit)?;
            poll(interrupt)?;
            candidate_bytes.try_reserve_exact(requested).map_err(|_| SourceProtectionError::Allocation)?;
            if candidate_bytes.capacity() as u64 > cap {
                return Err(SourceProtectionError::ResidentLimit);
            }
            poll(interrupt)?;
        }
        append(&mut candidate_bytes, prefix, cap, interrupt)?;
    }
    let mut hasher = blake3::Hasher::new();
    if !protected_fingerprints.is_empty() {
        poll(interrupt)?;
        hasher.update(prefix);
        poll(interrupt)?;
    }
    let mut chunk = [0u8; CHUNK_BYTES];
    loop {
        poll(interrupt)?;
        // Read at most one byte beyond the observed length to detect growth.
        let remaining = expected_len.checked_add(1).and_then(|n| n.checked_sub(reads.bytes)).ok_or(SourceProtectionError::LengthChanged)?;
        let offered = usize::try_from(remaining.min(CHUNK_BYTES as u64)).map_err(|_| SourceProtectionError::ByteLimit)?;
        if offered == 0 {
            return Err(SourceProtectionError::LengthChanged);
        }
        let count = reads.read(reader, chunk.get_mut(..offered).ok_or(SourceProtectionError::InvalidReadCount)?, interrupt)?;
        if reads.bytes > expected_len {
            return Err(SourceProtectionError::LengthChanged);
        }
        if count == 0 {
            if reads.bytes != expected_len {
                return Err(SourceProtectionError::LengthChanged);
            }
            break;
        }
        let bytes = chunk.get(..count).ok_or(SourceProtectionError::InvalidReadCount)?;
        if candidate {
            append(&mut candidate_bytes, bytes, cap, interrupt)?;
        }
        if !protected_fingerprints.is_empty() {
            poll(interrupt)?;
            hasher.update(bytes);
            poll(interrupt)?;
        }
    }
    poll(interrupt)?;
    if candidate {
        let checked = match resident_limit {
            Some(remaining) => {
                crate::xcf_source::contains_xcf_with_retained_capacity_and_resident_limit(&candidate_bytes, candidate_bytes.capacity(), remaining, interrupt)
            }
            None => crate::xcf_source::contains_xcf_with_retained_capacity(&candidate_bytes, candidate_bytes.capacity(), interrupt),
        };
        let contains = checked.map_err(|error| {
            if error.kind == photocraft_xcf::ErrorKind::Cancelled {
                SourceProtectionError::Cancelled
            } else if resident_limit.is_some() && error.kind == photocraft_xcf::ErrorKind::Limit {
                SourceProtectionError::ResidentLimit
            } else {
                SourceProtectionError::MalformedCandidate
            }
        })?;
        if contains {
            return Err(SourceProtectionError::XcfDestination);
        }
    }
    if !protected_fingerprints.is_empty() {
        poll(interrupt)?;
        let digest = hasher.finalize();
        poll(interrupt)?;
        for value in protected_fingerprints {
            poll(interrupt)?;
            if blake3::Hash::from_hex(value).map_err(|_| SourceProtectionError::InvalidFingerprint)? == digest {
                return Err(SourceProtectionError::ProtectedFingerprint);
            }
        }
    }
    poll(interrupt)?;
    Ok(())
}

/// Collect only inert provenance hashes, never an original/resource locator.
pub fn document_fingerprints(document: &photocraft_doc::Document) -> Result<Vec<String>, SourceProtectionError> {
    document_fingerprints_with_interrupt(document, &photocraft_raster::Interrupt::NONE)
}

/// Preserve the first spelling/order of each validated 256-bit source identity.
/// Cancellation abandons only private staged output; provenance is borrowed.
pub fn document_fingerprints_with_interrupt(
    document: &photocraft_doc::Document,
    interrupt: &photocraft_raster::Interrupt<'_>,
) -> Result<Vec<String>, SourceProtectionError> {
    poll(interrupt)?;
    let Some(state) = &document.xcf_state else {
        poll(interrupt)?;
        return Ok(Vec::new());
    };
    let provenance = &state.provenance;
    if provenance.protected_source_fingerprints.len() > MAX_FINGERPRINTS - 1 {
        return Err(SourceProtectionError::FingerprintLimit);
    }
    let count = provenance
        .protected_source_fingerprints
        .len()
        .checked_add(usize::from(provenance.source_hash.is_some()))
        .ok_or(SourceProtectionError::FingerprintLimit)?;
    let mut values: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::<[u8; 32]>::new();
    poll(interrupt)?;
    values.try_reserve_exact(count).map_err(|_| SourceProtectionError::Allocation)?;
    seen.try_reserve(count).map_err(|_| SourceProtectionError::Allocation)?;
    poll(interrupt)?;
    for value in provenance.source_hash.iter().chain(provenance.protected_source_fingerprints.iter()) {
        poll(interrupt)?;
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(SourceProtectionError::InvalidFingerprint);
        }
        let hash = blake3::Hash::from_hex(value).map_err(|_| SourceProtectionError::InvalidFingerprint)?;
        poll(interrupt)?;
        if !seen.insert(*hash.as_bytes()) {
            poll(interrupt)?;
            continue;
        }
        poll(interrupt)?;
        let mut copy = String::new();
        copy.try_reserve_exact(64).map_err(|_| SourceProtectionError::Allocation)?;
        poll(interrupt)?;
        copy.push_str(value);
        poll(interrupt)?;
        values.push(copy);
    }
    poll(interrupt)?;
    Ok(values)
}

/// Lower-only admission before copying inert provenance. A bounded Vec of exact
/// digests replaces an opaque HashSet allocation in this additive path; first
/// spelling/order and case-insensitive hash deduplication remain identical.
/// Caller must separately charge the borrowed Document. No paths are inspected.
pub fn document_fingerprints_with_resident_limit(
    document: &photocraft_doc::Document,
    resident_limit: u64,
    interrupt: &photocraft_raster::Interrupt<'_>,
) -> Result<Vec<String>, SourceProtectionError> {
    poll(interrupt)?;
    if resident_limit > resident_ceiling() {
        return Err(SourceProtectionError::ResidentLimit);
    }
    let Some(state) = &document.xcf_state else {
        poll(interrupt)?;
        return Ok(Vec::new());
    };
    let provenance = &state.provenance;
    if provenance.protected_source_fingerprints.len() > MAX_FINGERPRINTS - 1 {
        return Err(SourceProtectionError::FingerprintLimit);
    }
    let count = provenance
        .protected_source_fingerprints
        .len()
        .checked_add(usize::from(provenance.source_hash.is_some()))
        .ok_or(SourceProtectionError::FingerprintLimit)?;
    // Validate every literal before allocation, even duplicate entries.
    for value in provenance.source_hash.iter().chain(&provenance.protected_source_fingerprints) {
        poll(interrupt)?;
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) || blake3::Hash::from_hex(value).is_err() {
            return Err(SourceProtectionError::InvalidFingerprint);
        }
    }
    let headers = std::mem::size_of::<Vec<String>>() + std::mem::size_of::<Vec<[u8; 32]>>();
    let requested =
        count.checked_mul(std::mem::size_of::<String>() + 32 + 64).and_then(|n| n.checked_add(headers)).ok_or(SourceProtectionError::ResidentLimit)?;
    let maximum = usize::try_from(resident_limit).map_err(|_| SourceProtectionError::ResidentLimit)?;
    if requested > maximum {
        return Err(SourceProtectionError::ResidentLimit);
    }
    poll(interrupt)?;
    let mut values: Vec<String> = Vec::new();
    let mut seen: Vec<[u8; 32]> = Vec::new();
    values.try_reserve_exact(count).map_err(|_| SourceProtectionError::Allocation)?;
    let after_values = values
        .capacity()
        .checked_mul(std::mem::size_of::<String>())
        .and_then(|n| count.checked_mul(32 + 64).and_then(|v| n.checked_add(v)))
        .and_then(|n| n.checked_add(headers))
        .ok_or(SourceProtectionError::ResidentLimit)?;
    if after_values > maximum {
        return Err(SourceProtectionError::ResidentLimit);
    }
    poll(interrupt)?;
    seen.try_reserve_exact(count).map_err(|_| SourceProtectionError::Allocation)?;
    let base = values
        .capacity()
        .checked_mul(std::mem::size_of::<String>())
        .and_then(|n| seen.capacity().checked_mul(32).and_then(|v| n.checked_add(v)))
        .and_then(|n| n.checked_add(headers))
        .ok_or(SourceProtectionError::ResidentLimit)?;
    if base.checked_add(count.checked_mul(64).ok_or(SourceProtectionError::ResidentLimit)?).is_none_or(|n| n > maximum) {
        return Err(SourceProtectionError::ResidentLimit);
    }
    let mut payload = 0usize;
    for value in provenance.source_hash.iter().chain(&provenance.protected_source_fingerprints) {
        poll(interrupt)?;
        let hash = blake3::Hash::from_hex(value).map_err(|_| SourceProtectionError::InvalidFingerprint)?;
        let mut duplicate = false;
        for previous in &seen {
            poll(interrupt)?;
            if previous == hash.as_bytes() {
                duplicate = true;
                break;
            }
        }
        if duplicate {
            continue;
        }
        let prospective = payload.checked_add(64).and_then(|n| n.checked_add(base)).ok_or(SourceProtectionError::ResidentLimit)?;
        if prospective > maximum {
            return Err(SourceProtectionError::ResidentLimit);
        }
        let mut copy = String::new();
        copy.try_reserve_exact(64).map_err(|_| SourceProtectionError::Allocation)?;
        payload = payload.checked_add(copy.capacity()).ok_or(SourceProtectionError::ResidentLimit)?;
        if base.checked_add(payload).is_none_or(|n| n > maximum) {
            return Err(SourceProtectionError::ResidentLimit);
        }
        poll(interrupt)?;
        copy.push_str(value);
        seen.push(*hash.as_bytes());
        values.push(copy);
    }
    poll(interrupt)?;
    Ok(values)
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    struct CancellingReader<'a> {
        data: io::Cursor<Vec<u8>>,
        cancel: &'a AtomicBool,
    }
    impl Read for CancellingReader<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let read = self.data.read(output);
            self.cancel.store(true, Ordering::SeqCst);
            read
        }
    }
    #[test]
    fn cancel_after_actual_read_preserves_caller_source_bytes() {
        let cancel = AtomicBool::new(false);
        let callback = || cancel.load(Ordering::SeqCst);
        let interrupt = photocraft_raster::Interrupt::cancel_only(&callback);
        let original = vec![b'a'; 128];
        let mut reader = CancellingReader { data: io::Cursor::new(original.clone()), cancel: &cancel };
        assert!(matches!(check_reader_with_interrupt(&mut reader, 128, &[], &interrupt), Err(SourceProtectionError::Cancelled)));
        assert_eq!(reader.data.get_ref(), &original);
    }
    #[test]
    fn malformed_candidate_remains_fail_closed_without_cancellation() {
        let bytes = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0];
        let mut reader = io::Cursor::new(bytes.clone());
        assert!(matches!(
            check_reader_with_interrupt(&mut reader, bytes.len() as u64, &[], &photocraft_raster::Interrupt::NONE),
            Err(SourceProtectionError::MalformedCandidate)
        ));
        assert_eq!(reader.get_ref(), &bytes);
    }
}

#[cfg(test)]
mod fingerprint_collector_tests {
    use super::*;
    fn document() -> photocraft_doc::Document {
        let mut document =
            photocraft_doc::Document::new("source identity", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        document.xcf_state = Some(photocraft_doc::XcfState::default());
        document
    }
    #[test]
    fn original_first_spelling_and_history_order_survive_case_dedup() {
        let mut document = document();
        let original = "aB".repeat(32);
        let second = "Cd".repeat(32);
        let state = document.xcf_state.as_mut().unwrap();
        state.provenance.source_hash = Some(original.clone());
        state.provenance.protected_source_fingerprints = vec![original.to_uppercase(), second.clone(), second.to_lowercase()];
        assert_eq!(document_fingerprints(&document).unwrap(), vec![original, second]);
    }
    #[test]
    fn late_collector_cancellation_does_not_change_provenance() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut document = document();
        document.xcf_state.as_mut().unwrap().provenance.protected_source_fingerprints = (0..32).map(|n| format!("{n:064x}")).collect();
        let before = document.clone();
        let calls = AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, Ordering::SeqCst) >= 10;
        let interrupt = photocraft_raster::Interrupt::cancel_only(&cancel);
        assert!(matches!(document_fingerprints_with_interrupt(&document, &interrupt), Err(SourceProtectionError::Cancelled)));
        assert_eq!(document, before);
    }
    #[test]
    fn original_plus_4096_history_boundary_and_next_entry_refusal() {
        let mut document = document();
        let state = document.xcf_state.as_mut().unwrap();
        state.provenance.source_hash = Some(format!("{:064x}", 0));
        state.provenance.protected_source_fingerprints = (1..MAX_FINGERPRINTS).map(|n| format!("{n:064x}")).collect();
        assert_eq!(document_fingerprints(&document).unwrap().len(), MAX_FINGERPRINTS);
        document.xcf_state.as_mut().unwrap().provenance.protected_source_fingerprints.push(format!("{:064x}", MAX_FINGERPRINTS));
        assert!(matches!(document_fingerprints(&document), Err(SourceProtectionError::FingerprintLimit)));
    }
}

#[cfg(test)]
mod bounded_reader_drafts {
    use super::*;
    use std::io::Write;
    #[test]
    fn hash_owner_admission_counts_capacity_and_rejects_invalid_slots() {
        let mut hash = String::with_capacity(256);
        hash.push_str(blake3::hash(b"own fingerprint input").to_hex().as_ref());
        let values = vec![hash];
        let bytes = fingerprint_resident_bytes(&values, 8, &photocraft_raster::Interrupt::NONE).unwrap();
        assert!(bytes >= 256 + 8 * std::mem::size_of::<String>());
        assert!(fingerprint_resident_bytes(&values, 0, &photocraft_raster::Interrupt::NONE).is_err());
        assert!(matches!(fingerprint_resident_bytes(&values, usize::MAX, &photocraft_raster::Interrupt::NONE), Err(SourceProtectionError::ResidentLimit)));
        assert!(matches!(fingerprint_resident_bytes(&values, 8, &photocraft_raster::Interrupt::cancel_only(&|| true)), Err(SourceProtectionError::Cancelled)));
        assert_eq!(values[0], blake3::hash(b"own fingerprint input").to_hex().to_string());
    }
    #[test]
    fn wrapped_encoded_admission_refuses_before_collecting_body() {
        struct Count {
            calls: usize,
            bytes: io::Cursor<Vec<u8>>,
        }
        impl Read for Count {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                self.calls += 1;
                self.bytes.read(out)
            }
        }
        let mut bytes = vec![0u8; HEADER_BYTES];
        bytes[..3].copy_from_slice(&[0x1f, 0x8b, 0x08]);
        let mut reader = Count { calls: 0, bytes: io::Cursor::new(bytes.clone()) };
        let result = check_reader_with_resident_limit(&mut reader, 1 << 20, &[], SOURCE_PROTECTION_FIXED_BYTES + 16, &photocraft_raster::Interrupt::NONE);
        assert!(matches!(result, Err(SourceProtectionError::ResidentLimit)));
        assert_eq!(reader.calls, 1);
        assert_eq!(reader.bytes.get_ref(), &bytes);
    }
    #[test]
    fn native_fingerprint_and_plain_source_refusals_do_not_need_wrapper_heap() {
        let bytes = b"ordinary immutable bytes";
        let fingerprint = blake3::hash(bytes).to_hex().to_string();
        assert!(matches!(
            check_reader_with_resident_limit(
                &mut bytes.as_slice(),
                bytes.len() as u64,
                &[fingerprint],
                SOURCE_PROTECTION_FIXED_BYTES + 1,
                &photocraft_raster::Interrupt::NONE
            ),
            Err(SourceProtectionError::ProtectedFingerprint)
        ));
        let plain = b"gimp xcf v013\0";
        assert!(matches!(
            check_reader_with_resident_limit(
                &mut plain.as_slice(),
                plain.len() as u64,
                &[],
                SOURCE_PROTECTION_FIXED_BYTES + 1,
                &photocraft_raster::Interrupt::NONE
            ),
            Err(SourceProtectionError::XcfDestination)
        ));
    }
    #[test]
    fn decoder_retention_is_lowered_while_default_guard_stays_compatible() {
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&b"ordinary owned data".repeat(10000)).unwrap();
        let bytes = gzip.finish().unwrap();
        let mut original = bytes.as_slice();
        check_reader_with_interrupt(&mut original, bytes.len() as u64, &[], &photocraft_raster::Interrupt::NONE).unwrap();
        let mut limited = bytes.as_slice();
        assert!(matches!(
            check_reader_with_resident_limit(
                &mut limited,
                bytes.len() as u64,
                &[],
                SOURCE_PROTECTION_FIXED_BYTES + bytes.len() as u64 + 256,
                &photocraft_raster::Interrupt::NONE
            ),
            Err(SourceProtectionError::ResidentLimit)
        ));
    }
}

#[cfg(test)]
mod resident_collector_tests {
    use super::*;
    fn document(hash: &str) -> photocraft_doc::Document {
        let mut doc =
            photocraft_doc::Document::new("own collector", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        doc.xcf_state = Some(photocraft_doc::XcfState::default());
        doc.xcf_state.as_mut().unwrap().provenance.source_hash = Some(hash.into());
        doc
    }
    #[test]
    fn malformed_precedes_zero_budget_and_valid_zero_refuses() {
        assert!(matches!(
            document_fingerprints_with_resident_limit(&document("not a hash"), 0, &photocraft_raster::Interrupt::NONE),
            Err(SourceProtectionError::InvalidFingerprint)
        ));
        assert!(matches!(
            document_fingerprints_with_resident_limit(&document(&"ab".repeat(32)), 0, &photocraft_raster::Interrupt::NONE),
            Err(SourceProtectionError::ResidentLimit)
        ));
    }
    #[test]
    fn exact_lower_envelope_case_dedup_and_cancel_preserve_original() {
        let original = "aB".repeat(32);
        let mut doc = document(&original);
        doc.xcf_state.as_mut().unwrap().provenance.protected_source_fingerprints = vec![original.to_uppercase(), "Cd".repeat(32)];
        let before = doc.clone();
        let requested = 3 * (std::mem::size_of::<String>() + 32 + 64) + std::mem::size_of::<Vec<String>>() + std::mem::size_of::<Vec<[u8; 32]>>();
        assert!(matches!(
            document_fingerprints_with_resident_limit(&doc, requested as u64 - 1, &photocraft_raster::Interrupt::NONE),
            Err(SourceProtectionError::ResidentLimit)
        ));
        assert_eq!(
            document_fingerprints_with_resident_limit(&doc, requested as u64, &photocraft_raster::Interrupt::NONE).unwrap(),
            vec![original, "Cd".repeat(32)]
        );
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancelled = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 10;
        assert!(matches!(
            document_fingerprints_with_resident_limit(&doc, requested as u64, &photocraft_raster::Interrupt::cancel_only(&cancelled)),
            Err(SourceProtectionError::Cancelled)
        ));
        assert_eq!(doc, before);
    }
}
