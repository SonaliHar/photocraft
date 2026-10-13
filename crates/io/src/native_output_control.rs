//! Cooperative writes to an already-reserved, unpublished native byte buffer.
//! Reservation/capacity authority stays in MemoryOutput's existing ByteLedger.
use photocraft_raster::Interrupt;
use std::io;
const CHUNK: usize = 64 * 1024;
fn poll(control: &Interrupt<'_>) -> io::Result<()> {
    control.check().map_err(|_| io::Error::other("native byte-export cancelled"))
}
/// Never reserves or grows capacity. Failure may leave a zeroed private prefix.
pub(super) fn zero_extend(bytes: &mut Vec<u8>, end: usize, control: &Interrupt<'_>) -> io::Result<()> {
    poll(control)?;
    if end > bytes.capacity() {
        return Err(io::Error::other("native output bounds"));
    }
    while bytes.len() < end {
        poll(control)?;
        let next = bytes.len().checked_add(CHUNK.min(end - bytes.len())).ok_or_else(|| io::Error::other("native output offset overflow"))?;
        bytes.resize(next, 0);
        poll(control)?;
    }
    poll(control)
}
/// Exact copy order and bytes. Failure never reports a successful full write.
pub(super) fn copy(destination: &mut [u8], input: &[u8], control: &Interrupt<'_>) -> io::Result<()> {
    poll(control)?;
    if destination.len() != input.len() {
        return Err(io::Error::other("native output bounds"));
    }
    let mut offset = 0usize;
    while offset < input.len() {
        poll(control)?;
        let end = offset.checked_add(CHUNK.min(input.len() - offset)).ok_or_else(|| io::Error::other("native output offset overflow"))?;
        let from = input.get(offset..end).ok_or_else(|| io::Error::other("native output bounds"))?;
        let to = destination.get_mut(offset..end).ok_or_else(|| io::Error::other("native output bounds"))?;
        to.copy_from_slice(from);
        poll(control)?;
        offset = end;
    }
    poll(control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn private_copy_stops_between_chunks() {
        let calls = AtomicUsize::new(0);
        let cancelled = || calls.fetch_add(1, Ordering::Relaxed) >= 3;
        let control = Interrupt::cancel_only(&cancelled);
        let input = vec![7; CHUNK * 3];
        let mut output = vec![0; input.len()];
        assert!(copy(&mut output, &input, &control).is_err());
        assert_eq!(&output[..CHUNK], &input[..CHUNK]);
        assert!(output[CHUNK..].iter().all(|value| *value == 0));
    }
    #[test]
    fn zero_extension_uses_only_reserved_capacity_and_polls() {
        let mut bytes = Vec::with_capacity(CHUNK * 3);
        let capacity = bytes.capacity();
        let calls = AtomicUsize::new(0);
        let cancelled = || calls.fetch_add(1, Ordering::Relaxed) >= 3;
        assert!(zero_extend(&mut bytes, CHUNK * 3, &Interrupt::cancel_only(&cancelled)).is_err());
        assert_eq!(bytes.len(), CHUNK);
        assert_eq!(bytes.capacity(), capacity);
        assert!(bytes.iter().all(|value| *value == 0));
        assert!(zero_extend(&mut bytes, capacity + 1, &Interrupt::NONE).is_err());
        assert_eq!(bytes.len(), CHUNK);
    }
}
