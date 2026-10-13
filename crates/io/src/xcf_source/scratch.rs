//! Trusted service capability, not a filesystem locator or default import route.
use super::error;
use photocraft_xcf::{Budget, BudgetKind, ErrorKind, Reservation, Source, XcfError};
use std::io;
const SLICE: usize = 4096;
mod provider;
pub use provider::{NativeScratchHandle, ServiceAuthorizedScratchDirectory};

/// Exclusive newly empty scratch storage supplied by an audited caller service.
///
/// Implementations must own immutable storage after `seal`, bound each method's
/// internal memory and work, and destroy storage on drop before its quota can be
/// refunded. Integration must use an audited kernel-lifetime provider (delete-on-close
/// or unlinked open file), not named-file best-effort cleanup. No arbitrary adapter
/// establishes those properties. Construction is authorized and charged by the
/// caller service; `resident_bytes` includes retained capacities/guards, including
/// this handle, but excludes externally charged service state. No ambient source
/// path, cloning, handle extraction, or resource grants are provided here.
pub trait ScratchHandle {
    fn resident_bytes(&self) -> u64;
    fn len(&mut self) -> io::Result<u64>;
    fn is_empty(&mut self) -> io::Result<bool> {
        self.len().map(|len| len == 0)
    }
    fn read_at(&mut self, offset: u64, output: &mut [u8]) -> io::Result<usize>;
    fn write_at(&mut self, offset: u64, input: &[u8]) -> io::Result<usize>;
    /// Flush and irreversibly seal against writes/aliases before publication.
    fn seal(&mut self) -> io::Result<()>;
}

/// Immutable U64 source snapshot. It never accesses its original source again.
/// Source consistency during capture remains the authorized source service's duty.
/// Standard Vec routes and native_large policy are unchanged.
pub struct OwnedScratchXcfSnapshot<'b, 'a, H: ScratchHandle> {
    // Field drop order is essential: destroy storage before refundable leases.
    handle: H,
    encoded: Option<Reservation<'b, 'a>>,
    scratch: Option<Reservation<'b, 'a>>,
    resident: Option<Reservation<'b, 'a>>,
    len: u64,
}
struct CopyBuffer<'b, 'a> {
    bytes: Vec<u8>,
    _lease: Reservation<'b, 'a>,
}
fn storage_error(e: io::Error) -> XcfError {
    error(ErrorKind::Structure, &format!("scratch storage: {e}"))
}

impl<'b, 'a, H: ScratchHandle> OwnedScratchXcfSnapshot<'b, 'a, H> {
    pub fn capture(source: &mut dyn Source, handle: H, budget: &'b Budget<'a>) -> Result<Self, XcfError> {
        let mut snapshot = Self { handle, encoded: None, scratch: None, resident: None, len: source.len() };
        budget.poll("scratch admission")?;
        snapshot.resident = Some(budget.reserve(BudgetKind::Resident, snapshot.handle.resident_bytes(), "scratch handle")?);
        if snapshot.handle.len().map_err(storage_error)? != 0 {
            return Err(error(ErrorKind::Structure, "scratch handle must be empty"));
        }
        snapshot.encoded = Some(budget.reserve(BudgetKind::Encoded, snapshot.len, "scratch encoded admission")?);
        snapshot.scratch = Some(budget.reserve(BudgetKind::Scratch, snapshot.len, "scratch storage admission")?);
        let lease = budget.reserve(BudgetKind::Resident, (SLICE * 2) as u64, "scratch copy allocation")?;
        let mut buffer = CopyBuffer { bytes: Vec::new(), _lease: lease };
        buffer.bytes.try_reserve_exact(SLICE).map_err(|_| error(ErrorKind::Limit, "scratch copy allocation failed"))?;
        if buffer.bytes.capacity() > SLICE * 2 {
            return Err(error(ErrorKind::Limit, "scratch copy allocation capacity"));
        }
        buffer._lease.resize(buffer.bytes.capacity() as u64, "scratch actual copy capacity")?;
        buffer.bytes.resize(SLICE, 0);
        let mut offset = 0;
        while offset < snapshot.len {
            let count = (snapshot.len - offset).min(SLICE as u64) as usize;
            let chunk = buffer.bytes.get_mut(..count).ok_or_else(|| error(ErrorKind::Structure, "scratch copy bounds"))?;
            budget.consume_work(count as u64, "scratch source copy")?;
            source.read_exact_at(offset, chunk, budget)?;
            budget.poll("scratch source copy completed")?;
            snapshot.transfer_write(offset, chunk, budget)?;
            offset = offset.checked_add(count as u64).ok_or_else(|| error(ErrorKind::Structure, "scratch offset overflow"))?;
        }
        budget.poll("scratch seal")?;
        snapshot.handle.seal().map_err(storage_error)?;
        budget.poll("scratch sealed")?;
        if snapshot.handle.len().map_err(storage_error)? != snapshot.len {
            return Err(error(ErrorKind::Truncated, "scratch length mismatch"));
        }
        budget.poll("scratch publication")?;
        drop(buffer);
        Ok(snapshot)
    }
    fn transfer_write(&mut self, mut offset: u64, mut input: &[u8], budget: &Budget<'_>) -> Result<(), XcfError> {
        while !input.is_empty() {
            budget.consume_work(input.len() as u64, "scratch write")?;
            match self.handle.write_at(offset, input) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(storage_error(e)),
                Ok(0) => return Err(error(ErrorKind::Truncated, "scratch zero write")),
                Ok(n) => {
                    input = input.get(n..).ok_or_else(|| error(ErrorKind::Structure, "scratch write count"))?;
                    offset = offset.checked_add(n as u64).ok_or_else(|| error(ErrorKind::Structure, "scratch write overflow"))?;
                }
            }
            budget.poll("scratch write completed")?;
        }
        Ok(())
    }
}
impl<H: ScratchHandle> Source for OwnedScratchXcfSnapshot<'_, '_, H> {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_exact_at(&mut self, offset: u64, output: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
        budget.check(BudgetKind::Encoded, self.len, "scratch read admission")?;
        let end = offset.checked_add(output.len() as u64).ok_or_else(|| error(ErrorKind::Structure, "scratch read U64 overflow"))?;
        if offset > self.len || end > self.len {
            return Err(error(ErrorKind::Truncated, "scratch read outside source"));
        }
        let mut position = offset;
        for chunk in output.chunks_mut(SLICE) {
            let mut remaining = chunk;
            while !remaining.is_empty() {
                budget.consume_work(remaining.len() as u64, "scratch read")?;
                match self.handle.read_at(position, remaining) {
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(storage_error(e)),
                    Ok(0) => return Err(error(ErrorKind::Truncated, "scratch zero read")),
                    Ok(n) => {
                        remaining = remaining.get_mut(n..).ok_or_else(|| error(ErrorKind::Structure, "scratch read count"))?;
                        position = position.checked_add(n as u64).ok_or_else(|| error(ErrorKind::Structure, "scratch read overflow"))?;
                    }
                }
                budget.poll("scratch read completed")?;
            }
        }
        budget.poll("scratch read completed")
    }
}
