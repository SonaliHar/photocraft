//! Dedicated explicit read-root material; no locator resolution or document edit.
use crate::AutomationError;
use cap_std::fs::{Dir, OpenOptions};
use photocraft_raster::Interrupt;
use std::{
    io::{self, Read},
    path::Path,
};

const MAX_BYTES: usize = 8 << 20;
const SCRATCH_BYTES: usize = 64 << 10;
const MAX_INTERRUPTED: usize = 1024;
fn bad(code: &str) -> AutomationError {
    AutomationError::BadRequest(format!("xcf.link.substituteFromRoot.{code}"))
}
fn poll(control: &Interrupt<'_>) -> Result<(), AutomationError> {
    control.check().map_err(|_| AutomationError::Engine(photocraft_engine::EngineError::Cancelled))
}
fn io_error(error: io::Error) -> AutomationError {
    AutomationError::Io(format!("xcf.link.substituteFromRoot.read ({:?}): {error}", error.kind()))
}

pub(super) fn read(root: &Dir, relative: &Path, control: &Interrupt<'_>) -> Result<Vec<u8>, AutomationError> {
    poll(control)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        let flags = i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).map_err(|_| bad("context"))?;
        options.custom_flags(flags);
    }
    let mut file = root.open_with(relative, &options).map_err(io_error)?;
    poll(control)?;
    let initial = file.metadata().map_err(io_error)?;
    poll(control)?;
    if !initial.is_file() {
        return Err(bad("regular-file"));
    }
    let bytes = read_bounded(&mut file, initial.len(), control)?;
    let final_metadata = file.metadata().map_err(io_error)?;
    poll(control)?;
    if !final_metadata.is_file() || final_metadata.len() != initial.len() {
        return Err(bad("changed-length"));
    }
    Ok(bytes)
}

// Independent reader seam: every offered buffer is bounded by the remaining cap+1.
// Polling is cooperative; it cannot interrupt a blocking filesystem syscall.
fn read_bounded(reader: &mut impl Read, expected_len: u64, control: &Interrupt<'_>) -> Result<Vec<u8>, AutomationError> {
    poll(control)?;
    if expected_len == 0 {
        return Err(bad("empty"));
    }
    let expected = usize::try_from(expected_len).map_err(|_| bad("limit"))?;
    if expected > MAX_BYTES {
        return Err(bad("limit"));
    }
    let total_limit = MAX_BYTES.checked_add(1).ok_or_else(|| bad("limit"))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(expected).map_err(|_| bad("allocation"))?;
    let mut scratch = [0u8; SCRATCH_BYTES];
    let mut interrupted = 0usize;
    loop {
        poll(control)?;
        let remaining = total_limit.checked_sub(bytes.len()).ok_or_else(|| bad("limit"))?;
        let offered = remaining.min(scratch.len());
        if offered == 0 {
            return Err(bad("limit"));
        }
        let buffer = scratch.get_mut(..offered).ok_or_else(|| bad("read-count"))?;
        let count = match reader.read(buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                interrupted = interrupted.checked_add(1).filter(|count| *count <= MAX_INTERRUPTED).ok_or_else(|| bad("read-interrupted"))?;
                continue;
            }
            result => result.map_err(io_error)?,
        };
        poll(control)?;
        if count > offered {
            return Err(bad("read-count"));
        }
        if count == 0 {
            if bytes.len() != expected {
                return Err(bad("changed-length"));
            }
            return Ok(bytes);
        }
        let total = bytes.len().checked_add(count).ok_or_else(|| bad("limit"))?;
        if total > MAX_BYTES {
            return Err(bad("limit"));
        }
        bytes.try_reserve(count).map_err(|_| bad("allocation"))?;
        bytes.extend_from_slice(scratch.get(..count).ok_or_else(|| bad("read-count"))?);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Generated {
        left: usize,
        delivered: usize,
        calls: usize,
    }
    impl Read for Generated {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.calls += 1;
            let n = bytes.len().min(self.left);
            bytes.get_mut(..n).unwrap().fill(17);
            self.left -= n;
            self.delivered += n;
            Ok(n)
        }
    }
    #[test]
    fn exact_cap_preserves_data_and_oversize_metadata_starts_no_read() {
        let mut reader = Generated { left: MAX_BYTES, delivered: 0, calls: 0 };
        let bytes = read_bounded(&mut reader, MAX_BYTES as u64, &Interrupt::NONE).unwrap();
        assert_eq!(bytes.len(), MAX_BYTES);
        assert!(bytes.iter().all(|b| *b == 17));
        let mut denied = Generated { left: MAX_BYTES + 1, delivered: 0, calls: 0 };
        assert!(read_bounded(&mut denied, (MAX_BYTES + 1) as u64, &Interrupt::NONE).unwrap_err().to_string().contains("limit"));
        assert_eq!(denied.calls, 0);
    }
    #[test]
    fn growth_is_total_cap_plus_one_bounded_and_truncation_is_not_padding() {
        let mut grown = Generated { left: MAX_BYTES + 50_000, delivered: 0, calls: 0 };
        assert!(read_bounded(&mut grown, 1, &Interrupt::NONE).unwrap_err().to_string().contains("limit"));
        assert_eq!(grown.delivered, MAX_BYTES + 1);
        let mut changed = Generated { left: 5, delivered: 0, calls: 0 };
        assert!(read_bounded(&mut changed, 4, &Interrupt::NONE).unwrap_err().to_string().contains("changed-length"));
        let mut short = Generated { left: 3, delivered: 0, calls: 0 };
        assert!(read_bounded(&mut short, 4, &Interrupt::NONE).unwrap_err().to_string().contains("changed-length"));
        assert!(read_bounded(&mut short, 0, &Interrupt::NONE).unwrap_err().to_string().contains("empty"));
    }
    #[test]
    fn cancellation_before_and_during_read_never_returns_partial_material() {
        let mut reader = Generated { left: MAX_BYTES, delivered: 0, calls: 0 };
        assert!(read_bounded(&mut reader, MAX_BYTES as u64, &Interrupt::cancel_only(&|| true)).is_err());
        assert_eq!(reader.calls, 0);
        let polls = AtomicUsize::new(0usize);
        let cancel = || {
            let n = polls.fetch_add(1, Ordering::Relaxed);
            n >= 4
        };
        assert!(read_bounded(&mut reader, MAX_BYTES as u64, &Interrupt::cancel_only(&cancel)).is_err());
        assert!(reader.delivered < MAX_BYTES);
    }
    #[test]
    fn errors_interruption_and_invalid_read_counts_are_named() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("owned reader failure"))
            }
        }
        assert!(read_bounded(&mut Broken, 1, &Interrupt::NONE).unwrap_err().to_string().contains("read"));
        struct Count;
        impl Read for Count {
            fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
                Ok(b.len() + 1)
            }
        }
        assert!(read_bounded(&mut Count, 1, &Interrupt::NONE).unwrap_err().to_string().contains("read-count"));
        // Interruption semantics are tested with a real cursor after one Interrupted.
        struct InterruptedThen(std::io::Cursor<Vec<u8>>, bool);
        impl Read for InterruptedThen {
            fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
                if !self.1 {
                    self.1 = true;
                    return Err(io::ErrorKind::Interrupted.into());
                }
                self.0.read(b)
            }
        }
        let mut retry = InterruptedThen(std::io::Cursor::new(vec![3]), false);
        assert_eq!(read_bounded(&mut retry, 1, &Interrupt::NONE).unwrap(), vec![3]);
    }
}
