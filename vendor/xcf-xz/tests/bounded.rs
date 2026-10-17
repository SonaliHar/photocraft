#![forbid(unsafe_code)]
use std::io::{self, Write};
use xcf_xz::{Reservation, decode_with_reservation};
fn encode(raw: &[u8]) -> Vec<u8> {
    let mut writer = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(0)).unwrap();
    writer.write_all(raw).unwrap();
    writer.finish().unwrap()
}
fn decode(
    bytes: &[u8],
    output: usize,
    poll: &mut dyn FnMut() -> io::Result<()>,
    admit: &mut dyn FnMut(Reservation, usize) -> io::Result<()>,
) -> io::Result<Vec<u8>> {
    decode_with_reservation(bytes, 2_000_000, output, 32 * 1024 * 1024, 16384, poll, admit)
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
fn multiple_blocks_filters_checks_and_header_limits() {
    let raw = payload();
    for check in [lzma_rust2::CheckType::None, lzma_rust2::CheckType::Crc32, lzma_rust2::CheckType::Crc64, lzma_rust2::CheckType::Sha256] {
        let mut options = lzma_rust2::XzOptions::with_preset(0);
        options.lzma_options.dict_size = 4096;
        options.block_size = std::num::NonZeroU64::new(65536);
        options.check_type = check;
        options.prepend_pre_filter(lzma_rust2::FilterType::Delta, 2);
        options.prepend_pre_filter(lzma_rust2::FilterType::BcjX86, 16);
        let mut writer = lzma_rust2::XzWriter::new(Vec::new(), options).unwrap();
        writer.write_all(&raw).unwrap();
        let bytes = writer.finish().unwrap();
        assert_eq!(decode(&bytes, raw.len(), &mut || Ok(()), &mut |_, _| Ok(())).unwrap(), raw);
        assert!(decode_with_reservation(&bytes, bytes.len() - 1, raw.len(), 32 * 1024 * 1024, 16384, &mut || Ok(()), &mut |_, _| Ok(())).is_err());
        assert!(decode_with_reservation(&bytes, bytes.len(), raw.len(), 1, 16384, &mut || Ok(()), &mut |_, _| Ok(())).is_err());
        assert!(decode_with_reservation(&bytes, bytes.len(), raw.len(), 32 * 1024 * 1024, 1, &mut || Ok(()), &mut |_, _| Ok(())).is_err());
    }
}

#[test]
fn signature_probe_is_bounded_not_full_validation_and_keeps_original_controls() {
    let raw = [b"<svg xmlns".as_slice(), &vec![b' '; 131072]].concat();
    let bytes = encode(&raw);
    let mut peak = 0;
    let mut polls = 0;
    let prefix = xcf_xz::decode_prefix_with_reservation(
        &bytes,
        2_000_000,
        9,
        32 * 1024 * 1024,
        16384,
        &mut || {
            polls += 1;
            Ok(())
        },
        &mut |kind, n| {
            if matches!(kind, Reservation::OutputPeak) {
                peak = peak.max(n);
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(prefix, &raw[..9]);
    assert!(peak <= 18, "only actual prefix output, not full logical payload");
    assert!(polls > 0);
    assert!(xcf_xz::decode_prefix_with_reservation(&bytes, 2_000_000, 8, 32 * 1024 * 1024, 16384, &mut || Ok(()), &mut |_, _| Ok(())).is_err());
    assert!(
        xcf_xz::decode_prefix_with_reservation(&bytes, 2_000_000, 9, 32 * 1024 * 1024, 16384, &mut || Err(io::Error::other("cancelled")), &mut |_, _| Ok(()))
            .is_err()
    );
    assert!(
        xcf_xz::decode_prefix_with_reservation(&bytes, 2_000_000, 9, 32 * 1024 * 1024, 16384, &mut || Ok(()), &mut |_, _| Err(io::Error::other(
            "reservation denied"
        )))
        .is_err()
    );
    let mut trailing = bytes.clone();
    trailing.push(1);
    assert!(decode(&trailing, raw.len(), &mut || Ok(()), &mut |_, _| Ok(())).is_err(), "strict decoder remains authoritative");
}
