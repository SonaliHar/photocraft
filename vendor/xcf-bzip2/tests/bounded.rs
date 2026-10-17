#![forbid(unsafe_code)]
use std::io::{self, Write};
use xcf_bzip2::{Reservation, decode_with_reservation};
fn encode(raw: &[u8]) -> Vec<u8> {
    let mut writer = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
    writer.write_all(raw).unwrap();
    writer.finish().unwrap()
}
fn decode(
    bytes: &[u8],
    output: usize,
    poll: &mut dyn FnMut() -> io::Result<()>,
    admit: &mut dyn FnMut(Reservation, usize) -> io::Result<()>,
) -> io::Result<Vec<u8>> {
    decode_with_reservation(bytes, 2_000_000, output, 4_000_000, poll, admit)
}
fn payload() -> Vec<u8> {
    let mut state = 0x12345678u32;
    (0..131072)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}
#[test]
fn exact_empty_concat_crc_trailing_limits() {
    let raw = payload();
    let bytes = encode(&raw);
    assert_eq!(decode(&bytes, raw.len(), &mut || Ok(()), &mut |_, _| Ok(())).unwrap(), raw);
    let empty = encode(&[]);
    assert!(decode(&empty, 0, &mut || Ok(()), &mut |_, _| Ok(())).unwrap().is_empty());
    let concat = [empty, bytes.clone(), bytes.clone()].concat();
    assert_eq!(decode(&concat, 2 * raw.len(), &mut || Ok(()), &mut |_, _| Ok(())).unwrap(), [raw.clone(), raw.clone()].concat());
    assert!(decode(&bytes, raw.len() - 1, &mut || Ok(()), &mut |_, _| Ok(())).is_err());
    let mut tail = bytes.clone();
    tail.push(1);
    assert!(decode(&tail, raw.len(), &mut || Ok(()), &mut |_, _| Ok(())).is_err());
    let mut corrupt = bytes.clone();
    let index = corrupt.len() / 2;
    corrupt[index] ^= 128;
    assert!(decode(&corrupt, raw.len(), &mut || Ok(()), &mut |_, _| Ok(())).is_err());
    assert!(decode(&bytes[..bytes.len() - 1], raw.len(), &mut || Ok(()), &mut |_, _| Ok(())).is_err());
}
#[test]
fn cancellation_reservations_and_mutations() {
    let bytes = encode(&payload());
    let mut calls = 0;
    assert!(
        decode(
            &bytes,
            1_000_000,
            &mut || {
                calls += 1;
                if calls == 10 { Err(io::Error::other("cancel")) } else { Ok(()) }
            },
            &mut |_, _| Ok(())
        )
        .is_err()
    );
    assert_eq!(calls, 10);
    assert!(decode(&bytes, 1_000_000, &mut || Ok(()), &mut |_, _| Err(io::Error::other("reservation denied"))).is_err());
    for index in 0..128 {
        let mut changed = bytes.clone();
        let len = changed.len();
        changed[index % len] ^= 1 << (index % 8);
        if index % 2 != 0 {
            changed.truncate(index % len);
        }
        assert!(
            std::panic::catch_unwind(|| {
                let _ = decode(&changed, 1_000_000, &mut || Ok(()), &mut |_, _| Ok(()));
            })
            .is_ok()
        );
    }
}

#[test]
fn generated_historical_randomized_block_matches_independent_decoder() {
    use std::io::Read;
    let raw: Vec<u8> = (0..131072).map(|n| n as u8).collect();
    let mut bytes = encode(&raw);
    let source = include_str!("../src/randtable.rs");
    let table: Vec<u16> = source.split("= [").nth(1).unwrap().split("];").next().unwrap().split(',').filter_map(|v| v.trim().parse().ok()).collect();
    assert_eq!(table.len(), 512);
    let mut expected = raw;
    let mut left = 0;
    let mut index = 0;
    for byte in &mut expected {
        if left == 0 {
            left = table[index];
            index = (index + 1) % 512;
        }
        left -= 1;
        *byte ^= u8::from(left == 1);
    }
    let mut crc = u32::MAX;
    for byte in &expected {
        crc ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x80000000 != 0 { (crc << 1) ^ 0x04c11db7 } else { crc << 1 };
        }
    }
    crc = !crc;
    bytes[14] |= 128;
    bytes[10..14].copy_from_slice(&crc.to_be_bytes());
    let mut bits: Vec<bool> = bytes.iter().flat_map(|b| (0..8).rev().map(move |i| b & (1 << i) != 0)).collect();
    let magic = 0x177245385090u64;
    let pos = (112..bits.len() - 80).find(|p| (0..48).all(|i| bits[p + i] == (magic & (1 << (47 - i)) != 0))).unwrap();
    for i in 0..32 {
        bits[pos + 48 + i] = crc & (1 << (31 - i)) != 0;
    }
    for (dst, chunk) in bytes.iter_mut().zip(bits.chunks_exact(8)) {
        *dst = chunk.iter().fold(0, |v, b| (v << 1) | u8::from(*b));
    }
    let mut oracle = Vec::new();
    bzip2::read::BzDecoder::new(bytes.as_slice()).read_to_end(&mut oracle).unwrap();
    assert_eq!(oracle, expected);
    assert_eq!(decode(&bytes, expected.len(), &mut || Ok(()), &mut |_, _| Ok(())).unwrap(), expected);
}

#[test]
fn signature_probe_is_bounded_not_full_validation_and_keeps_original_controls() {
    let raw = [b"<svg xmlns".as_slice(), &vec![b' '; 131072]].concat();
    let bytes = encode(&raw);
    let mut peak = 0;
    let mut polls = 0;
    let prefix = xcf_bzip2::decode_prefix_with_reservation(
        &bytes,
        2_000_000,
        9,
        4_000_000,
        &mut || {
            polls += 1;
            Ok(())
        },
        &mut |kind, n| {
            if matches!(kind, Reservation::OutputOverlap) {
                peak = peak.max(n);
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(prefix, &raw[..9]);
    assert!(peak <= 18, "only actual prefix output, not full logical payload");
    assert!(polls > 0);
    assert!(xcf_bzip2::decode_prefix_with_reservation(&bytes, 2_000_000, 8, 4_000_000, &mut || Ok(()), &mut |_, _| Ok(())).is_err());
    assert!(
        xcf_bzip2::decode_prefix_with_reservation(&bytes, 2_000_000, 9, 4_000_000, &mut || Err(io::Error::other("cancelled")), &mut |_, _| Ok(())).is_err()
    );
    assert!(
        xcf_bzip2::decode_prefix_with_reservation(&bytes, 2_000_000, 9, 4_000_000, &mut || Ok(()), &mut |_, _| Err(io::Error::other("reservation denied")))
            .is_err()
    );
    let mut trailing = bytes.clone();
    trailing.push(1);
    assert!(decode(&trailing, raw.len(), &mut || Ok(()), &mut |_, _| Ok(())).is_err(), "strict decoder remains authoritative");
}
