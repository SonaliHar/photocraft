//! Scratch-only bounded structural preflight; decoder verifies payload/checks afterwards.
use std::io;
fn bad(s: &str) -> io::Error {
    io::Error::other(s)
}
fn u32le(b: &[u8]) -> io::Result<u32> {
    Ok(u32::from_le_bytes(b.try_into().map_err(|_| bad("u32 size"))?))
}
fn vli(b: &[u8], p: &mut usize) -> io::Result<u64> {
    let mut n = 0u64;
    for i in 0..9 {
        let c = *b.get(*p).ok_or_else(|| bad("truncated VLI"))?;
        *p += 1;
        n |= u64::from(c & 127) << (7 * i);
        if c & 128 == 0 {
            if i > 0 && c == 0 {
                return Err(bad("noncanonical VLI"));
            }
            return Ok(n);
        }
    }
    Err(bad("VLI overflow"))
}
fn take<'a>(b: &'a [u8], p: &mut usize, n: usize) -> io::Result<&'a [u8]> {
    let end = p.checked_add(n).ok_or_else(|| bad("offset overflow"))?;
    let out = b.get(*p..end).ok_or_else(|| bad("truncated field"))?;
    *p = end;
    Ok(out)
}
/// Returns stream extents and their exact declared logical lengths, bounded before backend allocation.
pub fn validate(
    bytes: &[u8],
    input_limit: usize,
    index_limit: usize,
    record_limit: usize,
    poll: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Vec<(usize, usize, u64)>> {
    if bytes.len() > input_limit {
        return Err(bad("encoded budget"));
    }
    let mut end = bytes.len();
    let mut streams = Vec::new();
    let mut records_total = 0usize;
    let mut charged = 0usize;
    while end > 0 {
        poll()?;
        let before_padding = end;
        while bytes.get(end.wrapping_sub(1)) == Some(&0) {
            end -= 1;
            if (before_padding - end).is_multiple_of(4096) {
                poll()?;
            }
        }
        if !(before_padding - end).is_multiple_of(4) {
            return Err(bad("stream padding alignment"));
        }
        let foot_start = end.checked_sub(12).ok_or_else(|| bad("missing footer"))?;
        let f = bytes.get(foot_start..end).ok_or_else(|| bad("footer"))?;
        if f.get(10..12) != Some(b"YZ") || crc32fast::hash(&f[4..10]) != u32le(&f[..4])? {
            return Err(bad("footer CRC/magic"));
        }
        let index_len = (u64::from(u32le(&f[4..8])?) + 1).checked_mul(4).ok_or_else(|| bad("index length"))?;
        let index_len = usize::try_from(index_len).map_err(|_| bad("index width"))?;
        if index_len > index_limit {
            return Err(bad("index budget"));
        }
        let index_start = foot_start.checked_sub(index_len).ok_or_else(|| bad("backward size"))?;
        let index = bytes.get(index_start..foot_start).ok_or_else(|| bad("index bounds"))?;
        let body_end = index.len().checked_sub(4).ok_or_else(|| bad("index truncated"))?;
        let mut index_crc = crc32fast::Hasher::new();
        for chunk in index[..body_end].chunks(4096) {
            poll()?;
            index_crc.update(chunk);
        }
        if index_crc.finalize() != u32le(&index[body_end..])? {
            return Err(bad("index CRC"));
        }
        if index.first() != Some(&0) {
            return Err(bad("index indicator"));
        }
        let mut p = 1;
        let count = usize::try_from(vli(&index[..body_end], &mut p)?).map_err(|_| bad("record width"))?;
        records_total = records_total.checked_add(count).ok_or_else(|| bad("record overflow"))?;
        if records_total > record_limit || count > body_end.saturating_sub(p) / 2 {
            return Err(bad("record budget/length"));
        }
        charged = charged.checked_add(count.checked_mul(16).ok_or_else(|| bad("index charge"))?).ok_or_else(|| bad("index charge"))?;
        if charged > index_limit {
            return Err(bad("index resident budget"));
        }
        let mut records = Vec::new();
        records.try_reserve_exact(count).map_err(|_| bad("index allocation"))?;
        let mut block_bytes = 0usize;
        let mut logical = 0u64;
        for i in 0..count {
            if i.is_multiple_of(256) {
                poll()?;
            }
            let raw = usize::try_from(vli(&index[..body_end], &mut p)?).map_err(|_| bad("block width"))?;
            let decoded = vli(&index[..body_end], &mut p)?;
            if raw < 5 {
                return Err(bad("block size"));
            }
            block_bytes = block_bytes.checked_add(raw.checked_add(3).ok_or_else(|| bad("block overflow"))? & !3).ok_or_else(|| bad("block overflow"))?;
            logical = logical.checked_add(decoded).ok_or_else(|| bad("logical overflow"))?;
            records.push((raw, decoded));
        }
        if index[p..body_end].len() > 3 || index[p..body_end].iter().any(|x| *x != 0) {
            return Err(bad("index padding"));
        }
        let start = index_start.checked_sub(block_bytes).and_then(|n| n.checked_sub(12)).ok_or_else(|| bad("stream extent"))?;
        let h = bytes.get(start..start + 12).ok_or_else(|| bad("stream header"))?;
        if &h[..6] != b"\xfd7zXZ\0" || h[6..8] != f[8..10] || crc32fast::hash(&h[6..8]) != u32le(&h[8..12])? {
            return Err(bad("header/flags CRC"));
        }
        let check_len = match h[7] {
            0 => 0,
            1 => 4,
            4 => 8,
            10 => 32,
            _ => return Err(bad("check type")),
        };
        if h[6] != 0 {
            return Err(bad("reserved flags"));
        }
        let mut bp = start + 12;
        for (raw, expected_decoded) in records {
            poll()?;
            let block_start = bp;
            let header_len = (usize::from(*bytes.get(bp).ok_or_else(|| bad("block header"))?) + 1) * 4;
            let bh = take(bytes, &mut bp, header_len)?;
            if header_len < 8 || crc32fast::hash(&bh[..header_len - 4]) != u32le(&bh[header_len - 4..])? {
                return Err(bad("block header CRC"));
            }
            let flags = bh[1];
            if flags & 0x3c != 0 {
                return Err(bad("block flags"));
            }
            let mut hp = 2;
            let compressed_declared = if flags & 64 != 0 { Some(vli(bh, &mut hp)?) } else { None };
            let uncompressed_declared = if flags & 128 != 0 { Some(vli(bh, &mut hp)?) } else { None };
            let filters = usize::from(flags & 3) + 1;
            for i in 0..filters {
                let id = vli(bh, &mut hp)?;
                let size = usize::try_from(vli(bh, &mut hp)?).map_err(|_| bad("filter properties"))?;
                let props = take(&bh[..header_len - 4], &mut hp, size)?;
                if i + 1 == filters && (id != 0x21 || props.len() != 1 || props[0] > 40) {
                    return Err(bad("LZMA2 terminating filter"));
                }
            }
            if bh[hp..header_len - 4].iter().any(|x| *x != 0) {
                return Err(bad("block header padding"));
            }
            let compressed_start = bp;
            let mut decoded = 0u64;
            loop {
                poll()?;
                let control = take(bytes, &mut bp, 1)?[0];
                if control == 0 {
                    break;
                }
                let (out, compressed) = if control >= 128 {
                    let fields = take(bytes, &mut bp, 4)?;
                    let out = (u64::from(control & 31) << 16) + u64::from(u16::from_be_bytes([fields[0], fields[1]])) + 1;
                    let packed = usize::from(u16::from_be_bytes([fields[2], fields[3]])) + 1;
                    if control >= 192 {
                        take(bytes, &mut bp, 1)?;
                    }
                    (out, packed)
                } else if control == 1 || control == 2 {
                    let fields = take(bytes, &mut bp, 2)?;
                    let n = usize::from(u16::from_be_bytes([fields[0], fields[1]])) + 1;
                    (n as u64, n)
                } else {
                    return Err(bad("LZMA2 control"));
                };
                // No CPU scan of packed data; bounded input ranges are verified before skipping.
                take(bytes, &mut bp, compressed)?;
                decoded = decoded.checked_add(out).ok_or_else(|| bad("decoded size"))?;
            }
            let actual_compressed = bp - compressed_start;
            if compressed_declared.is_some_and(|n| n != actual_compressed as u64)
                || uncompressed_declared.is_some_and(|n| n != decoded)
                || decoded != expected_decoded
                || bp - block_start + check_len != raw
            {
                return Err(bad("index/block sizes mismatch"));
            }
            let padding = (4 - (bp - block_start) % 4) % 4;
            if take(bytes, &mut bp, padding)?.iter().any(|x| *x != 0) {
                return Err(bad("block padding"));
            }
            take(bytes, &mut bp, check_len)?;
        }
        if bp != index_start {
            return Err(bad("index boundary"));
        }
        charged = charged.checked_add(2 * core::mem::size_of::<(usize, usize, u64)>()).ok_or_else(|| bad("stream metadata overflow"))?;
        if charged > index_limit {
            return Err(bad("stream metadata budget"));
        }
        poll()?;
        streams.try_reserve_exact(1).map_err(|_| bad("stream allocation"))?;
        streams.push((start, end, logical));
        end = start;
    }
    if streams.is_empty() {
        return Err(bad("empty wrapper"));
    }
    streams.reverse();
    Ok(streams)
}
