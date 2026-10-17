//! Header/trailer-strict gzip facade over the allocation-free safe miniz core.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
use miniz_oxide::inflate::{
    TINFLStatus,
    core::{DecompressorOxide, decompress_with_limit, inflate_flags::TINFL_FLAG_HAS_MORE_INPUT},
};
use std::io;
fn bad(s: &str) -> io::Error {
    io::Error::other(s)
}
fn take<'a>(b: &'a [u8], pos: &mut usize, n: usize) -> io::Result<&'a [u8]> {
    let end = pos.checked_add(n).ok_or_else(|| bad("offset overflow"))?;
    let out = b.get(*pos..end).ok_or_else(|| bad("truncated gzip"))?;
    *pos = end;
    Ok(out)
}

#[derive(Clone, Copy, Debug)]
pub enum Reservation {
    DecoderState,
    OutputOverlap,
}
pub fn decode_with_reservation(
    bytes: &[u8],
    input: usize,
    output: usize,
    state: usize,
    poll: &mut dyn FnMut() -> io::Result<()>,
    admit: &mut dyn FnMut(Reservation, usize) -> io::Result<()>,
) -> io::Result<Vec<u8>> {
    if bytes.len() > input {
        return Err(bad("encoded budget"));
    }
    let required = core::mem::size_of::<DecompressorOxide>() + 32768 + 4096;
    poll()?;
    if required > state {
        return Err(bad("decoder state budget"));
    }
    admit(Reservation::DecoderState, required)?;
    let mut result = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        poll()?;
        let start = pos;
        let header = take(bytes, &mut pos, 10)?;
        if header.get(..3) != Some(&[31, 139, 8]) || header[3] & 224 != 0 {
            return Err(bad("gzip magic/method/flags"));
        }
        let flags = header[3];
        if flags & 4 != 0 {
            let n = u16::from_le_bytes(take(bytes, &mut pos, 2)?.try_into().map_err(|_| bad("extra length"))?) as usize;
            for _ in 0..n {
                if (pos - start).is_multiple_of(4096) {
                    poll()?;
                }
                take(bytes, &mut pos, 1)?;
            }
        }
        for flag in [8, 16] {
            if flags & flag != 0 {
                let mut count = 0usize;
                loop {
                    if count >= 65536 {
                        return Err(bad("gzip header field budget"));
                    }
                    if count.is_multiple_of(4096) {
                        poll()?;
                    }
                    count += 1;
                    if take(bytes, &mut pos, 1)?[0] == 0 {
                        break;
                    }
                }
            }
        }
        if flags & 2 != 0 {
            let mut hash = crc32fast::Hasher::new();
            for chunk in bytes.get(start..pos).ok_or_else(|| bad("header extent"))?.chunks(4096) {
                poll()?;
                hash.update(chunk);
            }
            let crc = hash.finalize() as u16;
            let declared = u16::from_le_bytes(take(bytes, &mut pos, 2)?.try_into().map_err(|_| bad("header CRC"))?);
            if crc != declared {
                return Err(bad("header CRC"));
            }
        }
        let mut decoder = DecompressorOxide::new();
        let mut ring = [0; 32768];
        let mut ring_pos = 0;
        let mut crc = crc32fast::Hasher::new();
        let mut member_len = 0u32;
        loop {
            poll()?;
            let end = pos.saturating_add(65536).min(bytes.len());
            let flags = if end < bytes.len() { TINFL_FLAG_HAS_MORE_INPUT } else { 0 };
            let (status, used, written) =
                decompress_with_limit(&mut decoder, bytes.get(pos..end).ok_or_else(|| bad("payload extent"))?, &mut ring, ring_pos, 4096, flags);
            pos = pos.checked_add(used).ok_or_else(|| bad("offset overflow"))?;
            let chunk = ring.get(ring_pos..ring_pos + written).ok_or_else(|| bad("ring extent"))?;
            if result.len().checked_add(written).ok_or_else(|| bad("logical overflow"))? > output {
                return Err(bad("logical budget"));
            }
            poll()?;
            if result.len() + written > result.capacity() {
                admit(Reservation::OutputOverlap, result.capacity().checked_add(result.len() + written).ok_or_else(|| bad("output overlap"))?)?;
            }
            result.try_reserve_exact(written).map_err(|_| bad("output allocation"))?;
            result.extend_from_slice(chunk);
            crc.update(chunk);
            member_len = member_len.wrapping_add(written as u32);
            ring_pos = (ring_pos + written) % 32768;
            if status == TINFLStatus::Done {
                break;
            }
            if matches!(status, TINFLStatus::Failed | TINFLStatus::BadParam | TINFLStatus::Adler32Mismatch | TINFLStatus::FailedCannotMakeProgress)
                || used == 0 && written == 0
            {
                return Err(bad("invalid deflate"));
            }
        }
        let trailer = take(bytes, &mut pos, 8)?;
        let expected_crc = u32::from_le_bytes(trailer[..4].try_into().map_err(|_| bad("CRC size"))?);
        let expected_len = u32::from_le_bytes(trailer[4..].try_into().map_err(|_| bad("ISIZE"))?);
        if crc.finalize() != expected_crc || member_len != expected_len {
            return Err(bad("gzip CRC/ISIZE"));
        }
    }
    if pos == 0 {
        return Err(bad("empty gzip"));
    }
    Ok(result)
}

/// Decode at most the nine-byte logical signature. This is NOT trailer/EOF
/// validation; a claimed XCF must subsequently use the strict full decoder.
pub fn decode_prefix_with_reservation(
    bytes: &[u8],
    input: usize,
    output: usize,
    state: usize,
    poll: &mut dyn FnMut() -> io::Result<()>,
    admit: &mut dyn FnMut(Reservation, usize) -> io::Result<()>,
) -> io::Result<Vec<u8>> {
    if bytes.len() > input {
        return Err(bad("encoded budget"));
    }
    let required = core::mem::size_of::<DecompressorOxide>() + 32768 + 4096;
    poll()?;
    if required > state {
        return Err(bad("decoder state budget"));
    }
    admit(Reservation::DecoderState, required)?;
    let mut result = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        poll()?;
        let start = pos;
        let header = take(bytes, &mut pos, 10)?;
        if header.get(..3) != Some(&[31, 139, 8]) || header[3] & 224 != 0 {
            return Err(bad("gzip magic/method/flags"));
        }
        let flags = header[3];
        if flags & 4 != 0 {
            let n = u16::from_le_bytes(take(bytes, &mut pos, 2)?.try_into().map_err(|_| bad("extra length"))?) as usize;
            for _ in 0..n {
                if (pos - start).is_multiple_of(4096) {
                    poll()?;
                }
                take(bytes, &mut pos, 1)?;
            }
        }
        for flag in [8, 16] {
            if flags & flag != 0 {
                let mut count = 0usize;
                loop {
                    if count >= 65536 {
                        return Err(bad("gzip header field budget"));
                    }
                    if count.is_multiple_of(4096) {
                        poll()?;
                    }
                    count += 1;
                    if take(bytes, &mut pos, 1)?[0] == 0 {
                        break;
                    }
                }
            }
        }
        if flags & 2 != 0 {
            let mut hash = crc32fast::Hasher::new();
            for chunk in bytes.get(start..pos).ok_or_else(|| bad("header extent"))?.chunks(4096) {
                poll()?;
                hash.update(chunk);
            }
            let crc = hash.finalize() as u16;
            let declared = u16::from_le_bytes(take(bytes, &mut pos, 2)?.try_into().map_err(|_| bad("header CRC"))?);
            if crc != declared {
                return Err(bad("header CRC"));
            }
        }
        let mut decoder = DecompressorOxide::new();
        let mut ring = [0; 32768];
        let mut ring_pos = 0;
        let mut crc = crc32fast::Hasher::new();
        let mut member_len = 0u32;
        loop {
            poll()?;
            let end = pos.saturating_add(65536).min(bytes.len());
            let flags = if end < bytes.len() { TINFL_FLAG_HAS_MORE_INPUT } else { 0 };
            let (status, used, written) = decompress_with_limit(
                &mut decoder,
                bytes.get(pos..end).ok_or_else(|| bad("payload extent"))?,
                &mut ring,
                ring_pos,
                9usize.checked_sub(result.len()).ok_or_else(|| bad("prefix extent"))?,
                flags,
            );
            pos = pos.checked_add(used).ok_or_else(|| bad("offset overflow"))?;
            let chunk = ring.get(ring_pos..ring_pos + written).ok_or_else(|| bad("ring extent"))?;
            if result.len().checked_add(written).ok_or_else(|| bad("logical overflow"))? > output {
                return Err(bad("logical budget"));
            }
            poll()?;
            if result.len() + written > result.capacity() {
                admit(Reservation::OutputOverlap, result.capacity().checked_add(result.len() + written).ok_or_else(|| bad("output overlap"))?)?;
            }
            result.try_reserve_exact(written).map_err(|_| bad("output allocation"))?;
            result.extend_from_slice(chunk);
            crc.update(chunk);
            member_len = member_len.wrapping_add(written as u32);
            ring_pos = (ring_pos + written) % 32768;
            if result.len() == 9 {
                return Ok(result);
            }
            if status == TINFLStatus::Done {
                break;
            }
            if matches!(status, TINFLStatus::Failed | TINFLStatus::BadParam | TINFLStatus::Adler32Mismatch | TINFLStatus::FailedCannotMakeProgress)
                || used == 0 && written == 0
            {
                return Err(bad("invalid deflate"));
            }
        }
        let trailer = take(bytes, &mut pos, 8)?;
        let expected_crc = u32::from_le_bytes(trailer[..4].try_into().map_err(|_| bad("CRC size"))?);
        let expected_len = u32::from_le_bytes(trailer[4..].try_into().map_err(|_| bad("ISIZE"))?);
        if crc.finalize() != expected_crc || member_len != expected_len {
            return Err(bad("gzip CRC/ISIZE"));
        }
    }
    if pos == 0 {
        return Err(bad("empty gzip"));
    }
    Ok(result)
}
