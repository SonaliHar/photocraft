//! Explicit original bytes below a held read capability, not source authority.
use crate::AutomationError;
use cap_std::fs::{Dir, OpenOptions};
use photocraft_raster::Interrupt;
use std::{
    io::{self, Read},
    path::Path,
};
pub(super) const MAX_BYTES: usize = 16384;
pub(super) const SCRATCH_BYTES: usize = 4096;
const INTERRUPTED_LIMIT: usize = 64;
fn bad(message: &str) -> AutomationError {
    AutomationError::BadRequest(format!("select.loadSelection.originalPath: {message}"))
}
fn poll(ctl: &Interrupt<'_>) -> Result<(), AutomationError> {
    ctl.check().map_err(|_| AutomationError::Engine(photocraft_engine::EngineError::Cancelled))
}
fn error(e: io::Error) -> AutomationError {
    AutomationError::Io(format!("stored channel original read: {e}"))
}
pub(super) fn read(root: &Dir, path: &Path, ctl: &Interrupt<'_>) -> Result<Vec<u8>, AutomationError> {
    poll(ctl)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).map_err(|_| bad("open flag range"))?);
    }
    let mut file = root.open_with(path, &options).map_err(error)?;
    poll(ctl)?;
    let initial = file.metadata().map_err(error)?;
    if !initial.is_file() {
        return Err(bad("regular file required"));
    }
    let initial_modified = initial.modified().map_err(error)?;
    poll(ctl)?;
    let bytes = bounded(&mut file, initial.len(), ctl)?;
    let final_metadata = file.metadata().map_err(error)?;
    let final_modified = final_metadata.modified().map_err(error)?;
    poll(ctl)?;
    if !final_metadata.is_file() || final_metadata.len() != initial.len() || final_modified != initial_modified {
        return Err(bad("held file metadata changed"));
    }
    Ok(bytes)
}
// No read can grow the owned buffer. A final one-byte probe detects growth;
// short reads and bounded Interrupted retries preserve exact byte order.
fn bounded(reader: &mut impl Read, declared: u64, ctl: &Interrupt<'_>) -> Result<Vec<u8>, AutomationError> {
    poll(ctl)?;
    let expected = usize::try_from(declared).map_err(|_| bad("length range"))?;
    if expected == 0 || expected > MAX_BYTES {
        return Err(bad("original must contain 1..16384 bytes"));
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(expected).map_err(|_| bad("allocation"))?;
    if bytes.capacity() > MAX_BYTES {
        return Err(bad("actual allocation exceeds original cap"));
    }
    let mut scratch = [0u8; SCRATCH_BYTES];
    let mut interrupted = 0usize;
    let mut calls = 0usize;
    let max_calls = expected.checked_add(INTERRUPTED_LIMIT).and_then(|n| n.checked_add(1)).ok_or_else(|| bad("call range"))?;
    loop {
        poll(ctl)?;
        calls = calls.checked_add(1).filter(|n| *n <= max_calls).ok_or_else(|| bad("read call cap"))?;
        let offered = expected.checked_sub(bytes.len()).and_then(|n| n.checked_add(1)).ok_or_else(|| bad("read extent"))?.min(SCRATCH_BYTES);
        let count = match reader.read(scratch.get_mut(..offered).ok_or_else(|| bad("read window"))?) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                interrupted = interrupted.checked_add(1).filter(|n| *n <= INTERRUPTED_LIMIT).ok_or_else(|| bad("Interrupted retry cap"))?;
                continue;
            }
            value => value.map_err(error)?,
        };
        poll(ctl)?;
        if count > offered {
            return Err(bad("invalid returned read count"));
        }
        if count == 0 {
            if bytes.len() != expected {
                return Err(bad("short original file"));
            }
            return Ok(bytes);
        }
        let total = bytes.len().checked_add(count).ok_or_else(|| bad("read length overflow"))?;
        if total > expected || total > MAX_BYTES {
            return Err(bad("original grew beyond held length/cap"));
        }
        bytes.extend_from_slice(scratch.get(..count).ok_or_else(|| bad("read buffer"))?);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cap_short_growth_and_cancellation_are_checked_before_returning_bytes() {
        assert_eq!(bounded(&mut io::Cursor::new(vec![7; MAX_BYTES]), MAX_BYTES as u64, &Interrupt::NONE).unwrap(), vec![7; MAX_BYTES]);
        assert!(bounded(&mut io::Cursor::new(vec![7; MAX_BYTES + 1]), MAX_BYTES as u64, &Interrupt::NONE).is_err());
        assert!(bounded(&mut io::Cursor::new(vec![7; 3]), 4, &Interrupt::NONE).is_err());
        assert!(bounded(&mut io::Cursor::new(vec![7; 4]), 4, &Interrupt::cancel_only(&|| true)).is_err());
    }
    #[test]
    fn interrupted_reader_cannot_retry_forever() {
        struct Retry;
        impl Read for Retry {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::Interrupted.into())
            }
        }
        assert!(bounded(&mut Retry, 1, &Interrupt::NONE).unwrap_err().to_string().contains("retry cap"));
    }
    #[test]
    fn absent_read_authority_and_relative_escape_refuse_without_file_access() {
        let workspace = super::super::AuthorizedWorkspace::default();
        assert!(workspace.read_xcf_stored_channel_source("own.xcf", &Interrupt::NONE).is_err());
        assert!(super::super::request_path("../own.xcf", None).is_err());
        assert!(super::super::request_path("a/../../own.xcf", None).is_err());
    }
}
