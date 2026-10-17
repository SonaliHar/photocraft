#![forbid(unsafe_code)]
use std::io::{self, Write};
use xcf_gzip::{Reservation, decode_with_reservation};
fn encode(raw: &[u8]) -> Vec<u8> {
    let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
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
fn header_crc_reserved_flags_and_field_caps() {
    let gz = encode(&payload());
    let mut header = gz[..10].to_vec();
    header[3] |= 2;
    let crc = crc32fast::hash(&header) as u16;
    header.extend(crc.to_le_bytes());
    header.extend(&gz[10..]);
    assert!(decode(&header, 1_000_000, &mut || Ok(()), &mut |_, _| Ok(())).is_ok());
    header[10] ^= 1;
    assert!(decode(&header, 1_000_000, &mut || Ok(()), &mut |_, _| Ok(())).is_err());
    let mut flags = gz.clone();
    flags[3] |= 128;
    assert!(decode(&flags, 1_000_000, &mut || Ok(()), &mut |_, _| Ok(())).is_err());
    let mut long = gz[..10].to_vec();
    long[3] |= 8;
    long.extend(std::iter::repeat_n(b'x', 65536));
    long.push(0);
    long.extend(&gz[10..]);
    assert!(decode(&long, 1_000_000, &mut || Ok(()), &mut |_, _| Ok(())).is_err());
    assert!(decode_with_reservation(&gz, gz.len() - 1, 1_000_000, 100000, &mut || Ok(()), &mut |_, _| Ok(())).is_err());
    assert!(decode_with_reservation(&gz, gz.len(), 1_000_000, 1, &mut || Ok(()), &mut |_, _| Ok(())).is_err());
}

#[test]
fn signature_probe_is_bounded_not_full_validation_and_keeps_original_controls() {
    let raw = [b"<svg xmlns".as_slice(), &vec![b' '; 131072]].concat();
    let bytes = encode(&raw);
    let mut peak = 0;
    let mut polls = 0;
    let prefix = xcf_gzip::decode_prefix_with_reservation(
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
    assert!(xcf_gzip::decode_prefix_with_reservation(&bytes, 2_000_000, 8, 4_000_000, &mut || Ok(()), &mut |_, _| Ok(())).is_err());
    assert!(xcf_gzip::decode_prefix_with_reservation(&bytes, 2_000_000, 9, 4_000_000, &mut || Err(io::Error::other("cancelled")), &mut |_, _| Ok(())).is_err());
    assert!(
        xcf_gzip::decode_prefix_with_reservation(&bytes, 2_000_000, 9, 4_000_000, &mut || Ok(()), &mut |_, _| Err(io::Error::other("reservation denied")))
            .is_err()
    );
    let mut trailing = bytes.clone();
    trailing.push(1);
    assert!(decode(&trailing, raw.len(), &mut || Ok(()), &mut |_, _| Ok(())).is_err(), "strict decoder remains authoritative");
}
