//! Pinned CC0 real-camera regressions. Missing files fail, never silently skip.
#![cfg(feature = "corpus")]
use photocraft_raw::{Limits, RawError, RawFormat, cr3, decode, embedded_preview, identify};
fn sample(id: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../corpus/cr3/{id}.CR3"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}; run cargo xtask corpus --cr3", path.display()))
}
const CASES: &[(&str, u32, u32, bool)] = &[
    ("4656", 6000, 4000, false),
    ("4657", 6000, 4000, false),
    ("4693", 8192, 5464, false),
    ("4694", 8192, 5464, false),
    ("4696", 5088, 3392, false),
    ("4698", 8192, 5464, false),
    ("7881", 8192, 5464, true),
    ("7882", 8192, 5464, true),
    ("8961", 6960, 4640, false),
    ("8962", 6960, 4640, false),
];
#[test]
fn every_camera_has_a_correctly_sized_explicit_preview() {
    for &(id, w, h, hevc) in CASES {
        let b = sample(id);
        assert_eq!(identify(&b), Some(RawFormat::Cr3));
        if hevc {
            assert!(embedded_preview(&b).is_none());
            let candidates = cr3::hevc_previews(&b);
            assert_eq!(candidates.len(), 2);
            let p = candidates.iter().max_by_key(|p| u64::from(p.width) * u64::from(p.height)).unwrap();
            assert_eq!((p.width, p.height), (w, h));
            assert_eq!(p.bits, 10);
            assert_eq!(p.color, [9, 16, 9]);
            for p in candidates {
                assert!(p.to_heif(&Limits::default()).is_ok(), "{id}");
            }
        } else {
            let p = embedded_preview(&b).unwrap();
            assert_eq!((p.width, p.height), (w, h));
        }
        let metadata = cr3::metadata(&b);
        assert!(metadata.blocks.iter().all(Option::is_some), "{id}");
        assert!(metadata.exif(&Limits::default()).unwrap().unwrap().len() > 1000, "{id}");
        assert!(metadata.xmp.is_some(), "{id}");
    }
}
#[test]
fn every_lossless_sensor_sample_matches_the_independent_release_binary() {
    for (id, w, h, cw, ch, hash) in [
        ("4657", 6288, 4056, 6000, 4000, 0x62261f0ba81cfcd2u64),
        ("4693", 8352, 5586, 8192, 5464, 0x1f9a92d870791620),
        ("4694", 8352, 5586, 8192, 5464, 0xb009f1986c482e28),
        ("7881", 8480, 5650, 8192, 5464, 0xff716fb30248496f),
        ("8961", 7144, 4760, 6960, 4640, 0x47a4cb401233cc80),
        ("4656", 6288, 4056, 6000, 4000, 0x9bfade480f63376f),
        ("4671", 4032, 2656, 3888, 2592, 0x39a60c4efed4e1f4),
    ] {
        let b = sample(id);
        let track = cr3::sensor_track(&b, &Limits::default()).unwrap();
        assert_eq!(track.id, 3, "{id}: Dual Pixel auxiliary must not replace main sensor");
        let s = decode(&b, &Limits::default()).unwrap();
        assert_eq!((s.width, s.height, s.crop.width, s.crop.height), (w, h, cw, ch), "{id}");
        assert!(s.camera_wb.is_some(), "{id}: {:?}", s.warnings);
        assert!(s.black.values.iter().all(|v| (400.0..600.0).contains(v)), "{id}");
        assert!(s.white.iter().all(|v| (10000.0..16384.0).contains(v)), "{id}");
        let actual = s.data.iter().flat_map(|v| v.to_le_bytes()).fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
        assert_eq!(actual, hash, "{id}");
    }
}

#[test]
fn expanded_corpus_sensor_hashes_and_explicit_fallbacks() {
    let mut native = 0;
    let mut craw = 0;
    let mut previews = 0;
    for line in include_str!("../../../xtask/cr3-corpus.tsv").lines().skip(1) {
        let c: Vec<_> = line.split('\t').collect();
        assert_eq!(c.len(), 15, "{line}");
        let b = sample(c[0]);
        let limits = Limits::default();
        let track = cr3::sensor_track(&b, &limits).unwrap();
        assert_eq!((track.width, track.height), (c[3].parse::<u32>().unwrap(), c[4].parse::<u32>().unwrap()), "{}", c[0]);
        assert_eq!(track.version, u16::from_str_radix(c[5], 16).unwrap());
        assert_eq!(track.wavelet_levels, c[6].parse::<u8>().unwrap());
        assert_eq!(track.bits, c[7].parse::<u8>().unwrap());
        let size = (c[9].parse::<u32>().unwrap(), c[10].parse::<u32>().unwrap());
        if c[8] == "jpeg" {
            let p = embedded_preview(&b).unwrap();
            assert_eq!((p.width, p.height), size);
        } else {
            let candidates = cr3::hevc_previews(&b);
            let p = candidates.iter().max_by_key(|p| u64::from(p.width) * u64::from(p.height)).unwrap();
            assert_eq!((p.width, p.height), size);
            assert!(p.to_heif(&limits).is_ok());
        }
        match c[13] {
            "native" => {
                let s = decode(&b, &limits).unwrap_or_else(|e| panic!("{}: {e}", c[0]));
                assert_eq!((s.crop.width, s.crop.height), (c[11].parse::<usize>().unwrap(), c[12].parse::<usize>().unwrap()));
                assert_eq!(s.data.len(), s.width * s.height);
                assert!(s.camera_wb.is_some(), "{}", c[0]);
                let actual = s.data.iter().flat_map(|v| v.to_le_bytes()).fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
                assert_eq!(actual, u64::from_str_radix(c[14], 16).unwrap(), "{}", c[0]);
                native += 1;
                craw += usize::from(track.wavelet_levels == 3);
            }
            "preview" => {
                assert!(matches!(decode(&b, &limits), Err(RawError::Unsupported(_))), "{}", c[0]);
                previews += 1;
            }
            other => panic!("unknown corpus expectation {other}"),
        }
    }
    assert_eq!((native, craw, previews), (48, 22, 0));
}
