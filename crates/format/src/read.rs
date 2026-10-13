//! Reading a whole file to open it: the read side of [`crate::atomic`].
//!
//! What it adds over `std::fs::read` for multi-gigabyte PSB files:
//!
//! - **Bounded reads.** Each read asks for at most [`READ_CHUNK`] bytes. `std` asks Windows for up
//!   to 4 GiB in one `ReadFile`, and some file systems (exFAT, network redirectors) reject a read
//!   that large with `ERROR_INVALID_PARAMETER` ("The parameter is incorrect", #375).
//! - **A clear out-of-memory error.** The buffer is reserved up front with `try_reserve_exact`,
//!   and a file larger than the memory available says how much it needed.
//!
//! On `wasm32` there is no file system and every call returns the `std` "unsupported" error.

use std::io::{self, Read};
use std::path::Path;

/// Largest single read: big enough that a 5 GB file takes under a hundred reads.
pub const READ_CHUNK: usize = 64 << 20;

/// Read all of `path`, in reads of at most [`READ_CHUNK`] bytes.
pub fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    read_file_with(path, READ_CHUNK)
}

/// [`read_file`] with a chosen read size (tests use small ones to exercise the loop).
pub fn read_file_with(path: &Path, chunk: usize) -> io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    read_all_with(&mut f, len, chunk)
}

/// Read `reader` to its end in reads of at most [`READ_CHUNK`] bytes, for an already open file
/// (a capability-scoped handle, say). `size_hint` (the file's length) sizes the buffer up front.
pub fn read_all(reader: &mut impl Read, size_hint: u64) -> io::Result<Vec<u8>> {
    read_all_with(reader, size_hint, READ_CHUNK)
}

/// Controlled authorized-size read. The hint is not an input cap; growth
/// remains allowed with checked, fallible allocation. Polling cannot interrupt
/// a blocked operating-system read.
pub fn read_all_with_interrupt(reader: &mut impl Read, size_hint: u64, control: &photocraft_raster::Interrupt<'_>) -> io::Result<Vec<u8>> {
    read_all_with_control(reader, size_hint, READ_CHUNK, control)
}
fn read_all_with(reader: &mut impl Read, size_hint: u64, chunk: usize) -> io::Result<Vec<u8>> {
    read_all_with_control(reader, size_hint, chunk, &photocraft_raster::Interrupt::NONE)
}
fn read_all_with_control(reader: &mut impl Read, size_hint: u64, chunk: usize, control: &photocraft_raster::Interrupt<'_>) -> io::Result<Vec<u8>> {
    let poll = || if control.cancelled() { Err(io::Error::new(io::ErrorKind::Interrupted, "file read cancelled")) } else { Ok(()) };
    poll()?;
    // The hint only sizes the buffer: a file that grows or shrinks while being read is still read
    // to its end.
    let mut buf = buffer_for(size_hint)?;
    let request = chunk.clamp(1, 64 << 10);
    let mut scratch = [0u8; 64 << 10];
    let mut interrupted = 0usize;
    loop {
        poll()?;
        let destination = scratch.get_mut(..request).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file read request"))?;
        let count = match reader.read(destination) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                interrupted = interrupted
                    .checked_add(1)
                    .filter(|count| *count <= 1024)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::Interrupted, "file read retry limit"))?;
                continue;
            }
            result => result?,
        };
        poll()?;
        interrupted = 0;
        if count > request {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "reader returned invalid byte count"));
        }
        if count == 0 {
            return Ok(buf);
        }
        let next = buf.len().checked_add(count).ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "file read length overflow"))?;
        if next > buf.capacity() {
            let target = buf.capacity().checked_mul(2).unwrap_or(next).max(next);
            let additional = target.checked_sub(buf.len()).ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "file read reserve overflow"))?;
            buf.try_reserve_exact(additional).map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "not enough memory for file growth"))?;
        }
        buf.extend_from_slice(scratch.get(..count).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "file read byte count"))?);
        if size_hint > 0 {
            control.progress((buf.len() as f64 / size_hint as f64) as f32);
        }
    }
}

#[derive(Debug)]
struct BoundedReadLimit(&'static str);
impl std::fmt::Display for BoundedReadLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for BoundedReadLimit {}
pub(crate) fn is_bounded_read_limit(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|inner| inner.is::<BoundedReadLimit>())
}
fn bounded_limit(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, BoundedReadLimit(message))
}

/// Additive bounded/cancellable read for untrusted retained objects. The hint is
/// only an allocation hint; growth is checked before publication of each chunk.
/// Cancellation is cooperative around reads, not interruption of a blocked syscall.
pub fn read_all_bounded(reader: &mut impl Read, size_hint: u64, max: usize, control: &photocraft_raster::Interrupt<'_>) -> io::Result<Vec<u8>> {
    const CHUNK: usize = 8192;
    const MAX_CALLS: usize = 1_000_000;
    const MAX_INTERRUPTED: usize = 1024;
    let envelope = max.checked_add(16).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bounded read capacity overflow"))?;
    let poll = || if control.cancelled() { Err(io::Error::new(io::ErrorKind::Interrupted, "bounded read cancelled")) } else { Ok(()) };
    poll()?;
    let hint = usize::try_from(size_hint).unwrap_or(max).min(max);
    let mut out = Vec::new();
    out.try_reserve_exact(hint).map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "bounded read allocation"))?;
    if out.capacity() > envelope {
        return Err(io::Error::new(io::ErrorKind::OutOfMemory, "bounded read actual capacity"));
    }
    let mut scratch = [0u8; CHUNK];
    let mut calls = 0usize;
    let mut interrupted = 0usize;
    loop {
        poll()?;
        calls = calls.checked_add(1).filter(|count| *count <= MAX_CALLS).ok_or_else(|| bounded_limit("bounded read work limit"))?;
        let remaining = max.checked_sub(out.len()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bounded read length overflow"))?;
        let request = remaining.saturating_add(1).min(CHUNK);
        let buffer = scratch.get_mut(..request).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bounded read request"))?;
        let count = match reader.read(buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                interrupted = interrupted
                    .checked_add(1)
                    .filter(|count| *count <= MAX_INTERRUPTED)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::Interrupted, "bounded read retry limit"))?;
                continue;
            }
            result => result?,
        };
        poll()?;
        interrupted = 0;
        if count > request {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "reader returned invalid byte count"));
        }
        if count == 0 {
            break;
        }
        let next = out.len().checked_add(count).filter(|next| *next <= max).ok_or_else(|| bounded_limit("bounded read output limit"))?;
        if next > out.capacity() {
            // Geometric requested growth avoids copying the whole retained buffer on
            // every short read, while staying inside the caller's payload ceiling.
            let target = out.capacity().checked_mul(2).unwrap_or(max).max(next).min(max);
            let additional = target.checked_sub(out.len()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bounded read reserve overflow"))?;
            out.try_reserve_exact(additional).map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "bounded read allocation"))?;
            if out.capacity() > envelope {
                return Err(io::Error::new(io::ErrorKind::OutOfMemory, "bounded read actual capacity"));
            }
        }
        out.extend_from_slice(scratch.get(..count).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bounded read byte count"))?);
        if size_hint > 0 {
            control.progress((out.len() as f64 / size_hint as f64) as f32);
        }
    }
    poll()?;
    Ok(out)
}

/// An empty buffer with room for `len` bytes, or an `OutOfMemory` error saying how much was needed.
pub fn buffer_for(len: u64) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let reserved = usize::try_from(len).ok().and_then(|n| buf.try_reserve_exact(n).ok());
    match reserved {
        Some(()) => Ok(buf),
        None => Err(io::Error::new(io::ErrorKind::OutOfMemory, format!("not enough memory to read the whole file ({})", size_text(len)))),
    }
}

/// `len` bytes for people: "4.75 GB", "759 MB", "12 KB".
fn size_text(len: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = len as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit + 1 < UNITS.len() {
        v /= 1000.0;
        unit += 1;
    }
    match UNITS.get(unit) {
        Some(name) if unit > 0 => format!("{v:.2} {name}"),
        _ => format!("{len} bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("photocraft-read-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.bin");
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn reads_whole_file_across_many_chunks() {
        let data: Vec<u8> = (0..100_003u32).map(|i| (i * 7 % 251) as u8).collect();
        let p = temp("chunks", &data);
        for chunk in [1, 7, 4096, 100_003, 1 << 20] {
            assert_eq!(read_file_with(&p, chunk).unwrap(), data, "chunk {chunk}");
        }
        assert_eq!(read_file(&p).unwrap(), data);
        assert_eq!(read_file_with(&p, 0).unwrap(), data, "a zero chunk still makes progress");
        // From an open reader, whatever the hint says.
        for hint in [0, 10, data.len() as u64, 1 << 20] {
            assert_eq!(read_all(&mut std::io::Cursor::new(&data), hint).unwrap(), data, "hint {hint}");
        }
    }

    #[test]
    fn empty_and_missing_files() {
        assert!(read_file(&temp("empty", b"")).unwrap().is_empty());
        let missing = std::env::temp_dir().join("photocraft-read-does-not-exist.psb");
        assert_eq!(read_file(&missing).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_file_too_large_for_memory_is_a_clear_error() {
        // More than any address space can hold.
        let e = buffer_for(u64::MAX).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::OutOfMemory);
        assert!(e.to_string().contains("18446744.07 TB"), "{e}");
        let e = buffer_for(1 << 62).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::OutOfMemory);
        assert_eq!(buffer_for(1024).unwrap().capacity(), 1024);
    }

    #[test]
    fn sizes_read_like_the_os_shows_them() {
        assert_eq!(size_text(512), "512 bytes");
        assert_eq!(size_text(4_750_000_000), "4.75 GB");
        assert_eq!(size_text(759_000_000), "759.00 MB");
    }
}

#[cfg(test)]
mod bounded_tests {
    use super::*;
    #[test]
    fn short_reads_growth_and_exact_ceiling_are_checked() {
        struct Short(std::io::Cursor<Vec<u8>>);
        impl Read for Short {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                let count = out.len().min(3);
                self.0.read(out.get_mut(..count).unwrap())
            }
        }
        let bytes = vec![7; 30];
        assert_eq!(read_all_bounded(&mut Short(std::io::Cursor::new(bytes.clone())), 1, 30, &photocraft_raster::Interrupt::NONE).unwrap(), bytes);
        assert_eq!(
            read_all_bounded(&mut std::io::Cursor::new(vec![7; 31]), 0, 30, &photocraft_raster::Interrupt::NONE).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    #[test]
    fn repeated_interrupted_and_cancel_refuse_without_unbounded_retry() {
        struct Retry;
        impl Read for Retry {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::Interrupted))
            }
        }
        assert_eq!(read_all_bounded(&mut Retry, 0, 10, &photocraft_raster::Interrupt::NONE).unwrap_err().kind(), io::ErrorKind::Interrupted);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 3;
        assert_eq!(
            read_all_bounded(&mut std::io::Cursor::new(vec![1; 32768]), 0, 32768, &photocraft_raster::Interrupt::cancel_only(&cancel)).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
    }
}
