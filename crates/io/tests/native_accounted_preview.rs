//! Native accounting migration: immutable BEFORE archives remain the output authority.
use photocraft_doc::{ColorMode, PixelFormat, SampleType};
use photocraft_format::{PreparationLimits, load_from_bytes};
use photocraft_io::{NativeSaveLimits, save_native_stream};
use photocraft_raster::{Interrupt, Surface};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
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
fn packet() -> PathBuf {
    PathBuf::from(std::env::var("PHOTOCRAFT_ACCOUNTED_BASELINE").unwrap())
}
#[test]
#[ignore = "requires frozen unpublished accounted BEFORE authority"]
fn accounted_native16_matches_frozen_native_and_both_previews() {
    let raw = std::fs::read(packet().join("baseline.json")).unwrap();
    assert_eq!(hash(&raw), "f43c5751352d9a8251454e8d3322c4c1014a4ddf86540ab6320b3c6621a364f5");
    let baseline: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    for row in baseline["records"].as_array().unwrap() {
        if let Some(original) = row["original"].as_array() {
            let root = PathBuf::from(std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
            let source = std::fs::read(root.join(original[0].as_str().unwrap())).unwrap();
            assert_eq!(hash(&source), original[1]);
        }
        let archive = std::fs::read(packet().join(row["native_file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&archive), row["native_sha256"]);
        let doc = load_from_bytes(&archive).unwrap();
        let before = doc.clone();
        assert_eq!(hash(&serde_json::to_vec(&doc.xcf_state).unwrap()), row["canonical_sha256"]);
        assert_eq!(serde_json::to_value(&doc.working_profile_interpretation).unwrap(), row["current_marker"]);
        // The frozen input uses manifest v3; this writer uses the current schema.
        let current = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap(), current);
        assert_eq!(load_from_bytes(&current).unwrap(), doc);
        let mut output = Cursor::new(Vec::new());
        let result = save_native_stream(&doc, &mut output, limits(16 << 20), &Interrupt::NONE);
        assert!(result.is_ok(), "{}: {result:?}", row["id"]);
        assert!(result.unwrap().1.warnings.is_empty());
        assert_eq!(pngs(output.get_ref()), pngs(&archive));
        assert_eq!(load_from_bytes(output.get_ref()).unwrap(), doc);
        assert_eq!(doc, before);
    }
}

#[test]
#[ignore = "requires frozen unpublished accounted BEFORE authority"]
fn malformed_accounted_members_do_not_fallback_or_publish() {
    let original = load_from_bytes(&std::fs::read(packet().join("stack.source.before.pcraft")).unwrap()).unwrap();
    for case in 0..9 {
        let mut doc = original.clone();
        match case {
            0 => doc.icc_profile = None,
            1 => std::sync::Arc::make_mut(doc.icc_profile.as_mut().unwrap())[20] ^= 1,
            2 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "unknown".into(),
            3 => {
                let nodes = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes;
                let node = nodes.iter_mut().find(|node| !node.arguments.is_empty()).unwrap();
                let arg = node.arguments[0].clone();
                node.arguments.push(arg);
            }
            4 => {
                doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.iter_mut().find(|node| !node.arguments.is_empty()).unwrap().arguments[0].value =
                    photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float64Bits(f64::NAN.to_bits()));
            }
            5 => {
                let nodes = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes;
                let node = nodes[0].clone();
                nodes.resize(257, node);
            }
            6 => {
                let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
                let raw: Vec<_> = [f32::NAN, 0.5, 0.5, 1.0].into_iter().flat_map(f32::to_ne_bytes).collect();
                *doc.layers[0].surface_mut().unwrap() = Surface::try_with_default_bytes(format, &raw).unwrap();
            }
            _ => {}
        }
        assert!(
            photocraft_compose::gimp_render::PreparedRenderer::supports_accounted_scalar_preparation(&doc),
            "malformed scoped member {case} must reach checked constructor"
        );
        let before = doc.clone();
        let stop = || case == 7;
        let mut output = Cursor::new(Vec::new());
        let result = save_native_stream(&doc, &mut output, limits(if case == 8 { 1 } else { 16 << 20 }), &Interrupt::cancel_only(&stop));
        assert!(result.is_err(), "case {case}");
        assert!(output.get_ref().is_empty(), "case {case}");
        assert_eq!(doc, before);
    }
}
