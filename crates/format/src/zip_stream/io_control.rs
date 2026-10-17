//! Cooperative exact I/O for ZIP decode. Standard partial/Interrupted semantics
//! are retained, but every returned syscall is inside the cancellation fence.
//! This cannot interrupt a currently blocked operating-system call.
use crate::{FormatError, Result};
use photocraft_raster::Interrupt;
use std::io::{self, Read, Write};
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| FormatError::Cancelled)
}
/// One read with the decoder's original error/retry policy; unlike read_exact,
/// Interrupted is returned unchanged when the caller has not cancelled.
pub(super) fn read(input: &mut impl Read, out: &mut [u8], control: &Interrupt<'_>) -> Result<usize> {
    poll(control)?;
    let offered = out.len();
    let result = input.read(out);
    poll(control)?;
    let count = result.map_err(FormatError::Io)?;
    if count > offered {
        return Err(FormatError::corrupt("zip reader returned invalid count"));
    }
    Ok(count)
}
pub(super) fn read_exact(input: &mut impl Read, out: &mut [u8], control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let mut filled = 0usize;
    while filled < out.len() {
        poll(control)?;
        let target = out.get_mut(filled..).ok_or_else(|| FormatError::corrupt("zip read offset"))?;
        let offered = target.len();
        let result = input.read(target);
        poll(control)?;
        let count = match result {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(FormatError::Io(e)),
            Ok(0) => return Err(FormatError::Io(io::Error::new(io::ErrorKind::UnexpectedEof, "failed to fill whole buffer"))),
            Ok(n) if n > offered => return Err(FormatError::corrupt("zip reader returned invalid count")),
            Ok(n) => n,
        };
        filled = filled.checked_add(count).ok_or_else(|| FormatError::corrupt("zip read offset overflow"))?;
    }
    poll(control)
}
pub(super) fn write_all(output: &mut impl Write, bytes: &[u8], control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let mut written = 0usize;
    while written < bytes.len() {
        poll(control)?;
        let remaining = bytes.get(written..).ok_or_else(|| FormatError::corrupt("zip write offset"))?;
        let result = output.write(remaining);
        poll(control)?;
        let count = match result {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(FormatError::Io(e)),
            Ok(0) => return Err(FormatError::Io(io::Error::new(io::ErrorKind::WriteZero, "failed to write whole buffer"))),
            Ok(n) if n > remaining.len() => return Err(FormatError::corrupt("zip writer returned invalid count")),
            Ok(n) => n,
        };
        written = written.checked_add(count).ok_or_else(|| FormatError::corrupt("zip write offset overflow"))?;
    }
    poll(control)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    struct ShortRead<'a> {
        cancel: &'a AtomicBool,
        calls: usize,
    }
    impl Read for ShortRead<'_> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.calls += 1;
            if let Some(first) = bytes.first_mut() {
                *first = 7;
                self.cancel.store(true, Ordering::Relaxed);
                Ok(1)
            } else {
                Ok(0)
            }
        }
    }
    struct ShortWrite<'a> {
        cancel: &'a AtomicBool,
        bytes: Vec<u8>,
    }
    impl Write for ShortWrite<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some(first) = bytes.first() {
                self.bytes.push(*first);
                self.cancel.store(true, Ordering::Relaxed);
                Ok(1)
            } else {
                Ok(0)
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn token_changed_by_first_short_io_stops_before_second_attempt() {
        let cancel = AtomicBool::new(false);
        let cancelled = || cancel.load(Ordering::Relaxed);
        let control = Interrupt::cancel_only(&cancelled);
        let mut reader = ShortRead { cancel: &cancel, calls: 0 };
        assert!(matches!(read_exact(&mut reader, &mut [0; 8], &control), Err(FormatError::Cancelled)));
        assert_eq!(reader.calls, 1);
        cancel.store(false, Ordering::Relaxed);
        let mut writer = ShortWrite { cancel: &cancel, bytes: Vec::new() };
        assert!(matches!(write_all(&mut writer, &[1; 8], &control), Err(FormatError::Cancelled)));
        assert_eq!(writer.bytes.len(), 1);
    }
    #[test]
    fn eof_and_write_zero_keep_standard_error_kinds() {
        assert!(matches!(read_exact(&mut io::empty(),&mut[0;1],&Interrupt::NONE),Err(FormatError::Io(e)) if e.kind()==io::ErrorKind::UnexpectedEof));
        struct Zero;
        impl Write for Zero {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Ok(0)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(matches!(write_all(&mut Zero,&[1],&Interrupt::NONE),Err(FormatError::Io(e)) if e.kind()==io::ErrorKind::WriteZero));
    }
}
