//! Reviewed facade: no backend invocation without bounded structural preflight.
use crate::Control;
use std::io::{self, Read};
#[derive(Clone, Copy, Debug)]
pub enum Reservation {
    DecoderState,
    IndexPeak,
    OutputPeak,
}
struct Guard<'a, 'p> {
    bytes: &'a [u8],
    poll: &'p mut dyn FnMut() -> io::Result<()>,
    admit: &'p mut dyn FnMut(Reservation, usize) -> io::Result<()>,
    state: usize,
    index: usize,
}
impl Control for Guard<'_, '_> {
    fn poll(&mut self) -> io::Result<()> {
        (self.poll)()
    }
    fn admit_state(&mut self, n: usize) -> io::Result<()> {
        self.poll()?;
        if n > self.state { Err(io::Error::other("decoder state budget")) } else { (self.admit)(Reservation::DecoderState, n) }
    }
    fn admit_index(&mut self, n: usize) -> io::Result<()> {
        self.poll()?;
        if n > self.index { Err(io::Error::other("index budget")) } else { Ok(()) }
    }
}
impl Read for Guard<'_, '_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.poll()?;
        let n = out.len().min(65536);
        self.bytes.read(&mut out[..n])
    }
}

/// Hook values are complete peak reservations, held until this transaction returns.
#[allow(clippy::too_many_arguments)]
pub fn decode_with_reservation(
    bytes: &[u8],
    input: usize,
    output: usize,
    state: usize,
    index: usize,
    poll: &mut dyn FnMut() -> io::Result<()>,
    admit: &mut dyn FnMut(Reservation, usize) -> io::Result<()>,
) -> io::Result<Vec<u8>> {
    poll()?;
    admit(Reservation::IndexPeak, index)?;
    let streams = crate::strict_xz::validate(bytes, input, index, 1_000_000, poll)?;
    let total = streams.iter().try_fold(0u64, |n, s| n.checked_add(s.2).ok_or_else(|| io::Error::other("logical overflow")))?;
    if total > output as u64 {
        return Err(io::Error::other("logical budget"));
    }
    poll()?;
    admit(Reservation::OutputPeak, usize::try_from(total).map_err(|_| io::Error::other("logical width"))?)?;
    let mut result = Vec::new();
    result.try_reserve_exact(usize::try_from(total).map_err(|_| io::Error::other("logical width"))?).map_err(|_| io::Error::other("output allocation"))?;
    for (start, end, logical) in streams {
        let source = Guard { bytes: bytes.get(start..end).ok_or_else(|| io::Error::other("stream extent"))?, poll, admit, state, index };
        let cap = u32::try_from(state / 1024).map_err(|_| io::Error::other("state width"))?;
        let mut reader = crate::XzReader::new_mem_limit(source, cap);
        let mut buf = [0; 4096];
        let mut decoded = 0u64;
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            decoded = decoded.checked_add(n as u64).ok_or_else(|| io::Error::other("output overflow"))?;
            if decoded > logical {
                return Err(io::Error::other("output mismatch"));
            }
            result.extend_from_slice(&buf[..n]);
        }
        if decoded != logical {
            return Err(io::Error::other("output mismatch"));
        }
    }
    Ok(result)
}

/// Prefix-only content routing. Metadata/state/index remain bounded, but this
/// does not validate payload checksums/EOF and never allocates full footer output.
#[allow(clippy::too_many_arguments)]
pub fn decode_prefix_with_reservation(
    bytes: &[u8],
    input: usize,
    output: usize,
    state: usize,
    index: usize,
    poll: &mut dyn FnMut() -> io::Result<()>,
    admit: &mut dyn FnMut(Reservation, usize) -> io::Result<()>,
) -> io::Result<Vec<u8>> {
    poll()?;
    admit(Reservation::IndexPeak, index)?;
    let streams = crate::strict_xz::validate(bytes, input, index, 1_000_000, poll)?;
    let total = streams.iter().try_fold(0u64, |n, s| n.checked_add(s.2).ok_or_else(|| io::Error::other("logical overflow")))?;
    let prefix_bytes = total.min(9);
    if prefix_bytes > output as u64 {
        return Err(io::Error::other("logical budget"));
    }
    poll()?;
    admit(Reservation::OutputPeak, usize::try_from(prefix_bytes).map_err(|_| io::Error::other("logical width"))?)?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(usize::try_from(prefix_bytes).map_err(|_| io::Error::other("logical width"))?)
        .map_err(|_| io::Error::other("output allocation"))?;
    for (start, end, logical) in streams {
        let source = Guard { bytes: bytes.get(start..end).ok_or_else(|| io::Error::other("stream extent"))?, poll, admit, state, index };
        let cap = u32::try_from(state / 1024).map_err(|_| io::Error::other("state width"))?;
        let mut reader = crate::XzReader::new_mem_limit(source, cap);
        let mut buf = [0; 4096];
        let mut decoded = 0u64;
        loop {
            let count = 9usize.checked_sub(result.len()).ok_or_else(|| io::Error::other("prefix extent"))?;
            let target = buf.get_mut(..count).ok_or_else(|| io::Error::other("prefix buffer"))?;
            let n = reader.read(target)?;
            if n == 0 {
                break;
            }
            decoded = decoded.checked_add(n as u64).ok_or_else(|| io::Error::other("output overflow"))?;
            if decoded > logical {
                return Err(io::Error::other("output mismatch"));
            }
            result.extend_from_slice(buf.get(..n).ok_or_else(|| io::Error::other("prefix output"))?);
            if result.len() == 9 {
                return Ok(result);
            }
        }
        if decoded != logical {
            return Err(io::Error::other("output mismatch"));
        }
    }
    Ok(result)
}
