//! A qualified Median Blur singleton uses the native job preparation/output ledger.
use photocraft_format::{PreparationLimits, load_from_bytes};
use photocraft_io::{NativeSaveLimits, save_native_stream};
use photocraft_raster::Interrupt;
use sha2::{Digest, Sha256};
use std::io::Cursor;
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn limits(bytes: usize) -> NativeSaveLimits {
    NativeSaveLimits {
        preparation: PreparationLimits { resident_bytes: bytes, ..Default::default() },
        archive: photocraft_format::zip::stream::Limits { max_metadata_bytes: 1 << 20, ..Default::default() },
    }
}
fn pngs(bytes: &[u8]) -> [Vec<u8>; 2] {
    let mut reader = photocraft_format::zip::stream::Reader::new(Cursor::new(bytes), limits(128 << 20).archive, &Interrupt::NONE).unwrap();
    [reader.read_by_name("thumb.png", 8 << 20, &Interrupt::NONE).unwrap(), reader.read_by_name("composite/preview.png", 8 << 20, &Interrupt::NONE).unwrap()]
}
#[test]
#[ignore = "requires reviewed unpublished Median Blur source/edit and fixed BEFORE archives"]
fn median_blur_native48mib_matches_intact_before_source_and_dual_previews() {
    let packet = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_XCF_MEDIAN_PREVIEW_BASELINE").unwrap());
    let root = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let raw = std::fs::read(packet.join("baseline.json")).unwrap();
    assert_eq!(hash(&raw), "d481ed948004f9ee4c21fca12da99e79422cc1a4b8ce75670c27b05752cd397d");
    let baseline: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    for row in baseline["records"].as_array().unwrap() {
        let original = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&original), row["source_xcf_sha256"]);
        let archive = std::fs::read(packet.join(row["native_before_file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&archive), row["native_before_sha256"]);
        let doc = load_from_bytes(&archive).unwrap();
        let before = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        // Frozen input archive hashes remain exact; current versioned serialization
        // is deterministic and preserves the complete Document.
        assert_eq!(photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap(), before);
        assert_eq!(load_from_bytes(&before).unwrap(), doc);
        assert_eq!(hash(&serde_json::to_vec(&doc.xcf_state).unwrap()), row["canonical_metadata_sha256"]);
        let mut output = Cursor::new(Vec::new());
        let result = save_native_stream(&doc, &mut output, limits(48 << 20), &Interrupt::NONE);
        assert!(result.is_ok(), "actual lowered-budget Median Blur native regression:{}:{result:?}", row["id"]);
        assert!(result.unwrap().1.warnings.is_empty());
        assert_eq!(pngs(output.get_ref()), pngs(&archive));
        assert_eq!(load_from_bytes(output.get_ref()).unwrap(), doc);
        assert_eq!(photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap(), before);
    }
}
#[test]
#[ignore = "requires reviewed unpublished frozen Median Blur native BEFORE authority"]
fn median_blur_native_failures_preserve_destination_source_and_metadata() {
    let packet = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_XCF_MEDIAN_PREVIEW_BASELINE").unwrap());
    let original = load_from_bytes(&std::fs::read(packet.join("median-blur-source.before.pcraft")).unwrap()).unwrap();
    for case in 0..4 {
        let mut doc = original.clone();
        if case == 0 {
            std::sync::Arc::make_mut(doc.icc_profile.as_mut().unwrap())[20] ^= 1;
        } else if case == 1 {
            doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "unqualified".into();
        }
        let before = doc.clone();
        let stop = || case == 2;
        let control = Interrupt::cancel_only(&stop);
        let mut output = Cursor::new(Vec::new());
        let result = save_native_stream(&doc, &mut output, limits(if case == 3 { 1 } else { 16 << 20 }), &control);
        assert!(result.is_err(), "unsupported current binding/version/cancel/budget case {case}");
        if case == 2 {
            assert!(matches!(result, Err(photocraft_io::IoError::Cancelled)));
        }
        assert!(output.get_ref().is_empty(), "failure must precede archive publication {case}");
        assert_eq!(doc, before);
    }
}
