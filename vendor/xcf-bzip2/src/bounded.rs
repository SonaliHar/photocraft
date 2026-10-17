//! Isolated bounded prototype, adapted from bzip2-rs 0.1.2 under adjacent licenses.
use crate::{
    bitreader::BitReader,
    block::{Block, BlockError},
    header::Header,
};
use std::io;

/// Decode bounded, already-owned input. Polling aborts the transaction and drops partial output.
/// Streaming/snapshot sources remain the caller's responsibility.
///
/// Reservations are complete peak requirements, not cumulative deltas. Source is caller-owned.
#[derive(Clone, Copy, Debug)]
pub enum Reservation {
    DecoderState,
    OutputOverlap,
}
/// Integration entry: admit all fallible allocation peaks before attempting allocation.
pub fn decode_with_reservation(
    bytes: &[u8],
    input_limit: usize,
    output_limit: usize,
    state_limit: usize,
    poll: &mut dyn FnMut() -> io::Result<()>,
    admit: &mut dyn FnMut(Reservation, usize) -> io::Result<()>,
) -> io::Result<Vec<u8>> {
    if bytes.len() > input_limit {
        return Err(io::Error::other("encoded budget"));
    }
    poll()?;
    let mut output = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        poll()?;
        let header_bytes = bytes.get(offset..offset.saturating_add(4)).ok_or_else(|| io::Error::other("truncated header"))?;
        let header = Header::parse(header_bytes.try_into().map_err(|_| io::Error::other("header size"))?).map_err(io::Error::other)?;
        // BWT plane, largest selector buffer and fixed stack/tree state. Charged before allocation.
        let charge = (header.max_blocksize() as usize).checked_mul(4).and_then(|n| n.checked_add(131072)).ok_or_else(|| io::Error::other("state overflow"))?;
        if charge > state_limit {
            return Err(io::Error::other("decoder state budget"));
        }
        poll()?;
        admit(Reservation::DecoderState, charge)?;
        let mut block = Block::try_new(header).map_err(io::Error::other)?;
        let mut reader = BitReader::new(bytes.get(offset + 4..).ok_or_else(|| io::Error::other("header offset"))?);
        let mut crc = 0u32;
        let mut inner_poll = || poll().map_err(|_| BlockError::new("cancelled or deadline"));
        loop {
            inner_poll().map_err(io::Error::other)?;
            let magic = reader.read_u64(48).ok_or_else(|| io::Error::other("truncated block magic"))?;
            match magic {
                0x314159265359 => {
                    block.read_block_checked(&mut reader, &mut inner_poll).map_err(io::Error::other)?;
                    let mut chunk = [0u8; 4096];
                    loop {
                        inner_poll().map_err(io::Error::other)?;
                        let n = block.read_from_block_checked(&mut chunk, &mut inner_poll).map_err(io::Error::other)?;
                        if n == 0 {
                            break;
                        }
                        let next = output.len().checked_add(n).ok_or_else(|| io::Error::other("logical overflow"))?;
                        if next > output_limit {
                            return Err(io::Error::other("logical budget"));
                        }
                        if next > output.capacity() {
                            admit(Reservation::OutputOverlap, output.capacity().checked_add(next).ok_or_else(|| io::Error::other("output overlap overflow"))?)?;
                        }
                        output.try_reserve_exact(n).map_err(|_| io::Error::other("output allocation failed"))?;
                        output.extend_from_slice(chunk.get(..n).ok_or_else(|| io::Error::other("output size"))?);
                    }
                    crc = crc.rotate_left(1) ^ block.expected_crc();
                }
                0x177245385090 => {
                    let expected = reader.read_u32(32).ok_or_else(|| io::Error::other("truncated stream CRC"))?;
                    if crc != expected {
                        return Err(io::Error::other("stream CRC mismatch"));
                    }
                    let padding = (8 - reader.position() % 8) % 8;
                    if padding != 0 && reader.read_u8(padding as u8) != Some(0) {
                        return Err(io::Error::other("nonzero stream padding"));
                    }
                    offset = offset.checked_add(4).and_then(|n| n.checked_add(reader.position() / 8)).ok_or_else(|| io::Error::other("stream offset"))?;
                    break;
                }
                _ => return Err(io::Error::other("bad block magic")),
            }
        }
    }
    if offset == 0 {
        return Err(io::Error::other("empty wrapped input"));
    }
    Ok(output)
}

/// Signature-only probe; not a stream CRC/trailing-data validation result.
pub fn decode_prefix_with_reservation(
    bytes: &[u8],
    input_limit: usize,
    output_limit: usize,
    state_limit: usize,
    poll: &mut dyn FnMut() -> io::Result<()>,
    admit: &mut dyn FnMut(Reservation, usize) -> io::Result<()>,
) -> io::Result<Vec<u8>> {
    if bytes.len() > input_limit {
        return Err(io::Error::other("encoded budget"));
    }
    poll()?;
    let mut output = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        poll()?;
        let header_bytes = bytes.get(offset..offset.saturating_add(4)).ok_or_else(|| io::Error::other("truncated header"))?;
        let header = Header::parse(header_bytes.try_into().map_err(|_| io::Error::other("header size"))?).map_err(io::Error::other)?;
        // BWT plane, largest selector buffer and fixed stack/tree state. Charged before allocation.
        let charge = (header.max_blocksize() as usize).checked_mul(4).and_then(|n| n.checked_add(131072)).ok_or_else(|| io::Error::other("state overflow"))?;
        if charge > state_limit {
            return Err(io::Error::other("decoder state budget"));
        }
        poll()?;
        admit(Reservation::DecoderState, charge)?;
        let mut block = Block::try_new(header).map_err(io::Error::other)?;
        let mut reader = BitReader::new(bytes.get(offset + 4..).ok_or_else(|| io::Error::other("header offset"))?);
        let mut crc = 0u32;
        let mut inner_poll = || poll().map_err(|_| BlockError::new("cancelled or deadline"));
        loop {
            inner_poll().map_err(io::Error::other)?;
            let magic = reader.read_u64(48).ok_or_else(|| io::Error::other("truncated block magic"))?;
            match magic {
                0x314159265359 => {
                    block.read_block_checked(&mut reader, &mut inner_poll).map_err(io::Error::other)?;
                    let mut chunk = [0u8; 4096];
                    loop {
                        inner_poll().map_err(io::Error::other)?;
                        let count = 9usize.checked_sub(output.len()).ok_or_else(|| io::Error::other("prefix extent"))?;
                        let target = chunk.get_mut(..count).ok_or_else(|| io::Error::other("prefix buffer"))?;
                        let n = block.read_from_block_checked(target, &mut inner_poll).map_err(io::Error::other)?;
                        if n == 0 {
                            break;
                        }
                        let next = output.len().checked_add(n).ok_or_else(|| io::Error::other("logical overflow"))?;
                        if next > output_limit {
                            return Err(io::Error::other("logical budget"));
                        }
                        if next > output.capacity() {
                            admit(Reservation::OutputOverlap, output.capacity().checked_add(next).ok_or_else(|| io::Error::other("output overlap overflow"))?)?;
                        }
                        output.try_reserve_exact(n).map_err(|_| io::Error::other("output allocation failed"))?;
                        output.extend_from_slice(chunk.get(..n).ok_or_else(|| io::Error::other("output size"))?);
                        if output.len() == 9 {
                            return Ok(output);
                        }
                    }
                    crc = crc.rotate_left(1) ^ block.expected_crc();
                }
                0x177245385090 => {
                    let expected = reader.read_u32(32).ok_or_else(|| io::Error::other("truncated stream CRC"))?;
                    if crc != expected {
                        return Err(io::Error::other("stream CRC mismatch"));
                    }
                    let padding = (8 - reader.position() % 8) % 8;
                    if padding != 0 && reader.read_u8(padding as u8) != Some(0) {
                        return Err(io::Error::other("nonzero stream padding"));
                    }
                    offset = offset.checked_add(4).and_then(|n| n.checked_add(reader.position() / 8)).ok_or_else(|| io::Error::other("stream offset"))?;
                    break;
                }
                _ => return Err(io::Error::other("bad block magic")),
            }
        }
    }
    if offset == 0 {
        return Err(io::Error::other("empty wrapped input"));
    }
    Ok(output)
}
