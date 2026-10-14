//! CR3 preview extraction never mistakes encoded sensor data for a JPEG.
use photocraft_raw::testgen::cr3_preview;
use photocraft_raw::{RawFormat, embedded_preview, embedded_previews, identify, orientation};

fn jpeg() -> Vec<u8> {
    vec![0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 20, 0, 32, 1, 1, 0x11, 0, 0xff, 0xd9]
}

fn position(b: &[u8], marker: &[u8]) -> usize {
    b.windows(marker.len()).position(|w| w == marker).unwrap()
}

#[test]
fn full_size_jpeg_and_orientation_are_read_from_their_boxes() {
    let j = jpeg();
    let b = cr3_preview(&j, 6);
    assert_eq!(identify(&b), Some(RawFormat::Cr3));
    let p = embedded_preview(&b).unwrap();
    assert_eq!((p.width, p.height), (32, 20));
    assert_eq!(p.jpeg, j);
    assert_eq!(embedded_previews(&b).len(), 1);
    assert_eq!(orientation(&b), Some(6));
}

#[test]
fn variable_sizes_and_32_bit_chunk_offsets_work() {
    let mut b = cr3_preview(&jpeg(), 1);
    let at = position(&b, b"stsz") + 8;
    // Add a single variable-size entry and repair ancestor lengths/offset.
    let len = u32::from_be_bytes(b[at..at + 4].try_into().unwrap());
    b[at..at + 4].fill(0);
    b.splice(at + 8..at + 8, len.to_be_bytes());
    for marker in [b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stsz"] {
        let at = position(&b, marker) - 4;
        let n = u32::from_be_bytes(b[at..at + 4].try_into().unwrap());
        b[at..at + 4].copy_from_slice(&(n + 4).to_be_bytes());
    }
    let at = position(&b, b"co64");
    let off = u64::from_be_bytes(b[at + 12..at + 20].try_into().unwrap()) + 4;
    b[at..at + 4].copy_from_slice(b"stco");
    b[at + 12..at + 16].copy_from_slice(&(off as u32).to_be_bytes());
    assert_eq!(embedded_preview(&b).unwrap().jpeg, jpeg());
}

#[test]
fn broken_tables_and_raw_codec_tracks_are_not_used() {
    let good = cr3_preview(&jpeg(), 1);
    for marker in [b"stsz", b"co64", b"stsc", b"JPEG"] {
        let mut b = good.clone();
        let at = position(&b, marker);
        b[at..at + 4].copy_from_slice(b"xxxx");
        assert!(embedded_preview(&b).is_none());
    }
    let mut b = good.clone();
    let at = position(&b, b"co64") + 12;
    b[at..at + 8].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(embedded_preview(&b).is_none());
    let mut b = good;
    let at = position(&b, b"stsz") + 12;
    b[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(embedded_preview(&b).is_none());
}

#[test]
fn truncations_and_corrupted_box_lengths_never_panic() {
    let b = cr3_preview(&jpeg(), 8);
    for n in 0..b.len() {
        let _ = embedded_previews(&b[..n]);
        let _ = orientation(&b[..n]);
    }
    for marker in [b"ftyp", b"moov", b"trak", b"mdat"] {
        for len in [0u32, 1, 2, 7, u32::MAX] {
            let mut corrupt = b.clone();
            let at = position(&b, marker) - 4;
            corrupt[at..at + 4].copy_from_slice(&len.to_be_bytes());
            let _ = embedded_previews(&corrupt);
        }
    }
}

#[test]
fn extended_mdat_length_is_supported() {
    let mut b = cr3_preview(&jpeg(), 1);
    let at = position(&b, b"mdat") - 4;
    let size = u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) as u64 + 8;
    b[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
    b.splice(at + 8..at + 8, size.to_be_bytes());
    let off = position(&b, b"co64") + 12;
    let n = u64::from_be_bytes(b[off..off + 8].try_into().unwrap()) + 8;
    b[off..off + 8].copy_from_slice(&n.to_be_bytes());
    assert_eq!(embedded_preview(&b).unwrap().jpeg, jpeg());
}

#[test]
fn prvw_is_used_when_no_jpeg_track_exists() {
    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [(body.len() as u32 + 8).to_be_bytes().as_slice(), kind, body].concat()
    }
    let ftyp = bx(b"ftyp", b"crx \0\0\0\x01crx isom");
    let mut body = vec![0; 12];
    body.extend_from_slice(&(jpeg().len() as u32).to_be_bytes());
    body.extend(jpeg());
    let mut uuid = vec![0xea, 0xf4, 0x2b, 0x5e, 0x1c, 0x98, 0x4b, 0x88, 0xb9, 0xfb, 0xb7, 0xdc, 0x40, 0x6e, 0x4d, 0x16];
    uuid.extend([0; 8]);
    uuid.extend(bx(b"PRVW", &body));
    let b = [ftyp, bx(b"uuid", &uuid)].concat();
    assert_eq!(embedded_preview(&b).unwrap().jpeg, jpeg());
    assert_eq!(orientation(&b), None);
}

#[test]
fn deeply_nested_containers_are_bounded() {
    let mut b = vec![0; 8];
    for _ in 0..1000 {
        b = [((b.len() + 8) as u32).to_be_bytes().as_slice(), b"moov", &b].concat();
    }
    let ftyp = [24u32.to_be_bytes().as_slice(), b"ftypcrx \0\0\0\x01crx isom"].concat();
    assert!(embedded_previews(&[ftyp, b].concat()).is_empty());
}

mod native_sensor_and_metadata {
    //! CR3 preview tables and container metadata are independently bounded.
    use photocraft_raw::{Limits, RawError, RawFormat, cr3, embedded_preview, identify, orientation, testgen};
    fn jpeg() -> Vec<u8> {
        vec![0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 20, 0, 32, 1, 1, 0x11, 0, 0xff, 0xd9]
    }
    fn position(b: &[u8], kind: &[u8]) -> usize {
        b.windows(kind.len()).position(|w| w == kind).unwrap()
    }
    #[test]
    fn full_size_preview_and_container_orientation_are_read() {
        let b = testgen::cr3::preview(&jpeg(), 6);
        assert_eq!(identify(&b), Some(RawFormat::Cr3));
        assert_eq!(orientation(&b), Some(6));
        let p = embedded_preview(&b).unwrap();
        assert_eq!((p.width, p.height), (32, 20));
        assert_eq!(p.jpeg, jpeg());
    }
    #[test]
    fn metadata_relocates_exif_maker_and_gps_values() {
        let b = testgen::cr3::with_metadata(&jpeg(), 8, 2, "<xmp custom='retained'/>");
        let metadata = cr3::metadata(&b);
        assert_eq!(metadata.color_space(), Some(2));
        assert_eq!(metadata.xmp, Some("<xmp custom='retained'/>"));
        let exif = metadata.exif(&Limits::default()).unwrap().unwrap();
        assert_eq!(orientation(&exif), Some(8));
        assert!(exif.windows(b"Canon Synthetic CR3\0".len()).any(|w| w == b"Canon Synthetic CR3\0"));
        assert!(exif.windows(5).any(|w| w == [7, 9, 11, 13, 15]));
        assert!(metadata.exif(&Limits { max_alloc: 32, ..Default::default() }).is_err());
    }
    #[test]
    fn invalid_tables_do_not_extract_sensor_data_or_out_of_media_bytes() {
        let b = testgen::cr3::preview(&jpeg(), 1);
        for kind in [b"stsz", b"co64", b"stsc", b"JPEG"] {
            let mut broken = b.clone();
            let at = position(&broken, kind);
            broken[at..at + 4].copy_from_slice(b"xxxx");
            assert!(embedded_preview(&broken).is_none());
        }
        let mut broken = b.clone();
        let at = position(&broken, b"co64") + 12;
        broken[at..at + 8].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(embedded_preview(&broken).is_none());
        // The single-frame reader must not silently pick a frame from a burst.
        let mut burst = b;
        let at = position(&burst, b"stsz") + 12;
        burst[at..at + 4].copy_from_slice(&2u32.to_be_bytes());
        assert!(embedded_preview(&burst).is_none());
    }
    #[test]
    fn truncated_and_corrupted_containers_never_panic() {
        let b = testgen::cr3::preview(&jpeg(), 8);
        for n in 0..b.len() {
            let _ = embedded_preview(&b[..n]);
            let _ = cr3::hevc_previews(&b[..n]);
            let _ = cr3::metadata(&b[..n]).exif(&Limits::default());
        }
        for kind in [b"moov", b"trak", b"mdat"] {
            for size in [0u32, 1, 2, 7, u32::MAX] {
                let mut broken = b.clone();
                let at = position(&broken, kind) - 4;
                broken[at..at + 4].copy_from_slice(&size.to_be_bytes());
                let _ = embedded_preview(&broken);
                let _ = cr3::metadata(&broken).exif(&Limits::default());
            }
        }
    }
    #[test]
    fn hevc_adapter_rejects_absurd_geometry_and_short_bitstreams() {
        let configuration = vec![0u8; 23];
        let mut p = cr3::HevcPreview {
            width: 32,
            height: 20,
            tile_width: 32,
            tile_height: 20,
            rows: 1,
            columns: 1,
            configuration: &configuration,
            sample: &[],
            color: [1, 13, 1],
            full_range: true,
            bits: 8,
        };
        assert!(p.to_heif(&Limits::default()).is_err());
        p.width = u32::MAX;
        assert!(matches!(p.to_heif(&Limits::default()), Err(RawError::LimitExceeded(_))));
        p.width = 32;
        p.rows = 0;
        assert!(p.to_heif(&Limits::default()).is_err());
    }
    #[test]
    fn native_lossless_sensor_is_distinct_from_preview_and_checks_hostile_input() {
        let b = testgen::cr3::lossless(6);
        let s = photocraft_raw::decode(&b, &Limits::default()).unwrap();
        assert_eq!((s.width, s.height, s.orientation), (8, 4, 6));
        let want: Vec<_> = (0..4).flat_map(|y| (0..8).map(move |x| 8192 + 100 * (1 + (y % 2) * 2 + x % 2))).collect();
        assert_eq!(s.data, want);
        assert_eq!(s.camera_wb, Some([1., 1., 1.]));
        assert_eq!(s.crop, photocraft_raw::Rect::new(0, 0, 8, 4));
        assert_eq!(s.cfa.unwrap().colors, [0, 1, 1, 2]);
        assert!(matches!(photocraft_raw::decode(&b, &Limits { max_alloc: 64, ..Default::default() }), Err(RawError::LimitExceeded(_))));
        for n in 0..b.len() {
            assert!(photocraft_raw::decode(&b[..n], &Limits::default()).is_err());
        }
        let mut bad = b.clone();
        let at = position(&b, b"CMP1") + 12;
        bad[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(photocraft_raw::decode(&bad, &Limits::default()).is_err());
        let at = position(&b, b"mdat") + 4 + 112 + 5;
        let mut bad = b;
        bad[at] = 0;
        assert!(matches!(photocraft_raw::decode(&bad, &Limits::default()), Err(RawError::Malformed(_))));
        let valid = testgen::cr3::lossless(1);
        for i in 0..valid.len() {
            let mut mutated = valid.clone();
            mutated[i] ^= 0xff;
            let _ = photocraft_raw::decode(&mutated, &Limits::default());
        }
        let ptr = testgen::cr3::bx(b"CCDT", &[0u64.to_be_bytes().as_slice(), 0u32.to_be_bytes().as_slice(), 3u32.to_be_bytes().as_slice()].concat());
        let cctp = testgen::cr3::bx(b"CCTP", &[0u32.to_be_bytes().as_slice(), 1u32.to_be_bytes().as_slice(), 1u32.to_be_bytes().as_slice(), &ptr].concat());
        let mut duplicate = valid;
        duplicate.extend(testgen::cr3::bx(
            b"uuid",
            &[&[0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48][..], &cctp].concat(),
        ));
        assert!(matches!(photocraft_raw::decode(&duplicate, &Limits::default()), Err(RawError::Malformed(_))));
    }

    #[test]
    fn native_craw_sensor_and_hostile_wavelet_headers() {
        let b = testgen::cr3::craw(8);
        let s = photocraft_raw::decode(&b, &Limits::default()).unwrap();
        assert_eq!((s.width, s.height, s.orientation), (64, 48, 8));
        assert_eq!(s.data, vec![8192; 64 * 48]);
        assert_eq!(s.camera_wb, Some([1.; 3]));
        assert!(matches!(photocraft_raw::decode(&b, &Limits { max_alloc: 8192, ..Default::default() }), Err(RawError::LimitExceeded(_))));
        for n in (0..b.len()).step_by(13) {
            assert!(photocraft_raw::decode(&b[..n], &Limits::default()).is_err());
        }
        // Corrupt every header/entropy byte in isolation: never panic on input.
        let start = position(&b, b"mdat") + 4;
        for i in start..b.len() {
            let mut mutated = b.clone();
            mutated[i] ^= 0xff;
            let _ = photocraft_raw::decode(&mutated, &Limits::default());
        }
        let mut broken = b.clone();
        broken[start + 4..start + 8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(photocraft_raw::decode(&broken, &Limits::default()), Err(RawError::Malformed(_))));
        let mut unsupported = b;
        let cmp = position(&unsupported, b"CMP1");
        unsupported[cmp + 8..cmp + 10].copy_from_slice(&0x300u16.to_be_bytes());
        assert!(matches!(photocraft_raw::decode(&unsupported, &Limits::default()), Err(RawError::Unsupported(_))));
    }

    #[test]
    fn adaptive_craw_quantization_and_hostile_extended_headers() {
        let bytes = testgen::cr3::adaptive(6);
        let s = photocraft_raw::decode(&bytes, &Limits::default()).unwrap();
        assert_eq!((s.width, s.height, s.orientation), (64, 48, 6));
        // QP22 -> HL1 coefficient24, QP26 ->40. Integer lifting yields these
        // independently calculated pixels, including horizontal/vertical joins.
        let at = |x: usize, y: usize| s.data[(y * 2) * 64 + x * 2];
        assert_eq!((at(0, 0), at(1, 0), at(15, 0), at(16, 0), at(17, 0)), (8180, 8204, 8202, 8176, 8214));
        assert_eq!((at(0, 1), at(0, 2)), (8176, 8172));
        assert!(matches!(photocraft_raw::decode(&bytes, &Limits { max_alloc: 8192, ..Default::default() }), Err(RawError::LimitExceeded(_))));
        for n in (0..bytes.len()).step_by(11) {
            assert!(photocraft_raw::decode(&bytes[..n], &Limits::default()).is_err());
        }
        let start = position(&bytes, b"mdat") + 4;
        for i in start..bytes.len() {
            let mut mutated = bytes.clone();
            mutated[i] ^= 0xff;
            let _ = photocraft_raw::decode(&mutated, &Limits::default());
        }
        for offset in [4, 12, 24, 36, 44] {
            let mut damaged = bytes.clone();
            damaged[start + offset..start + offset + 4].fill(0xff);
            assert!(matches!(photocraft_raw::decode(&damaged, &Limits::default()), Err(RawError::Malformed(_))));
        }
        // A bounded v0x200 masked-readout overhang is clipped to stored rows.
        let iad = position(&bytes, b"IAD1") + 4;
        let mut padded = bytes.clone();
        padded[iad + 46..iad + 48].copy_from_slice(&51u16.to_be_bytes());
        assert_eq!(photocraft_raw::decode(&padded, &Limits::default()).unwrap().active.height, 48);
        padded[iad + 46..iad + 48].copy_from_slice(&55u16.to_be_bytes());
        assert!(matches!(photocraft_raw::decode(&padded, &Limits::default()), Err(RawError::Malformed(_))));
    }
}
