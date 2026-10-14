//! Full CR3 imports, with metadata/depth and native-format persistence.
#![cfg(feature = "corpus")]
// Keep the exact sensor-reference oracles in the default I/O corpus gate, while
// the older multi-format pixls corpus remains opt-in on the RAW crate.
#[path = "../../raw/tests/cr3_corpus.rs"]
mod sensor_oracles;

use photocraft_color::SampleType;
use photocraft_io::import;
fn sample(id: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../corpus/cr3/{id}.CR3"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}; run cargo xtask corpus --cr3", path.display()))
}
// Force an unimplemented primary-sensor version while retaining the real
// camera previews. Native support must not erase coverage of explicit fallback.
fn unsupported_sensor(id: &str) -> Vec<u8> {
    let mut bytes = sample(id);
    let track = photocraft_raw::cr3::sensor_track(&bytes, &Default::default()).unwrap();
    let width = track.width.to_be_bytes();
    let height = track.height.to_be_bytes();
    let at = bytes.windows(20).position(|b| &b[..4] == b"CMP1" && b[12..16] == width && b[16..20] == height).unwrap();
    bytes[at + 8..at + 10].copy_from_slice(&0x300u16.to_be_bytes());
    bytes
}
#[test]
fn unsupported_sensor_jpeg_previews_import_with_container_metadata() {
    for (id, w, h) in [("4696", 5088, 3392), ("4698", 8192, 5464), ("8962", 6960, 4640)] {
        let b = unsupported_sensor(id);
        let r = import("camera.CR3", &b).unwrap();
        assert_eq!((r.document.size.width, r.document.size.height), (w, h));
        assert_eq!(r.document.depth, SampleType::U8);
        assert!(r.document.metadata.exif.is_some());
        assert!(r.document.metadata.xmp.is_some());
        assert!(r.warnings.iter().any(|w| w.contains("preview") && w.contains("not the RAW sensor data")));
        assert!(!r.warnings.iter().any(|w| w.contains("could not be preserved")), "{:?}", r.warnings);
    }
}
#[cfg(feature = "heif")]
#[test]
fn r5_mark_ii_hevc_previews_keep_ten_bit_pq_data_and_metadata() {
    {
        let id = "7882";
        let r = import("camera.CR3", &unsupported_sensor(id)).unwrap();
        assert_eq!((r.document.size.width, r.document.size.height), (8192, 5464));
        assert_eq!(r.document.depth, SampleType::U16);
        let profile = photocraft_cms::Profile::parse(r.document.icc_profile.as_ref().unwrap()).unwrap();
        assert!(profile.description.contains("PQ"));
        assert!(r.document.metadata.exif.is_some());
        assert!(r.document.metadata.xmp.is_some());
        assert!(r.warnings.iter().any(|w| w.contains("10-bit HDR PQ HEVC") && w.contains("not the RAW sensor data")));
    }
}
#[cfg(not(feature = "heif"))]
#[test]
fn builds_without_heif_explain_why_hevc_previews_cannot_open() {
    let e = import("camera.CR3", &unsupported_sensor("7882")).unwrap_err();
    assert!(e.to_string().contains("HEIC/HEIF support isn't included"), "{e}");
}
#[test]
fn lossless_cr3_imports_are_developed_sixteen_bit_documents() {
    for (id, w, h) in [("4657", 6000, 4000), ("4693", 8192, 5464), ("4694", 8192, 5464), ("7881", 8192, 5464), ("8961", 6960, 4640)] {
        let r = import("camera.CR3", &sample(id)).unwrap();
        assert_eq!((r.document.size.width, r.document.size.height), (w, h));
        assert_eq!(r.document.depth, SampleType::U16);
        let p = photocraft_cms::Profile::parse(r.document.icc_profile.as_ref().unwrap()).unwrap();
        assert!(p.description.contains("ProPhoto"));
        assert!(r.document.metadata.exif.as_ref().unwrap().len() > 40000);
        assert!(r.document.metadata.xmp.is_some());
        assert!(r.warnings.iter().any(|w| w.contains("developed with default settings")));
        assert!(!r.warnings.iter().any(|w| w.contains("preview") || w.contains("estimated") || w.contains("could not be preserved")), "{:?}", r.warnings);
    }
}

#[test]
fn expanded_corpus_imports_native_sensors_or_explicit_camera_previews() {
    for line in include_str!("../../../xtask/cr3-corpus.tsv").lines().skip(1) {
        let c: Vec<_> = line.split('\t').collect();
        let b = sample(c[0]);
        let native = c[13] == "native";
        if !native && c[8] == "hevc" && !cfg!(feature = "heif") {
            assert!(import("camera.CR3", &b).unwrap_err().to_string().contains("HEIC/HEIF support isn't included"));
            continue;
        }
        let r = import("camera.CR3", &b).unwrap_or_else(|e| panic!("{}: {e}", c[0]));
        let (w, h) = if native { (c[11], c[12]) } else { (c[9], c[10]) };
        let size = (w.parse::<u32>().unwrap(), h.parse::<u32>().unwrap());
        let rotated = (5..=8).contains(&photocraft_raw::orientation(&b).unwrap_or(1));
        assert_eq!((r.document.size.width, r.document.size.height), if rotated { (size.1, size.0) } else { size }, "{}", c[0]);
        assert_eq!(r.document.depth, if native || c[8] == "hevc" { SampleType::U16 } else { SampleType::U8 }, "{}", c[0]);
        let profile = photocraft_cms::Profile::parse(r.document.icc_profile.as_ref().unwrap()).unwrap();
        if native {
            assert!(profile.description.contains("ProPhoto"));
            assert!(r.warnings.iter().any(|w| w.contains("developed with default settings")));
            assert!(!r.warnings.iter().any(|w| w.contains("not the RAW sensor data")));
        } else {
            assert!(r.warnings.iter().any(|w| w.contains("preview") && w.contains("not the RAW sensor data")));
        }
        assert!(r.document.metadata.exif.is_some(), "{}", c[0]);
    }
}
