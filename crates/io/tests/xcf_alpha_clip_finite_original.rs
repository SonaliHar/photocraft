#![cfg(feature = "corpus")]
//! Independent Alpha Clip source/native/derived projection controls; diagnostics remain closed.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|byte| format!("{byte:02x}")).collect()
}
fn blob(root: &Path, v: &Value) -> Vec<u8> {
    let raw = fs::read(root.join(v["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&raw), v["sha256"]);
    let n = v["decoded_bytes"].as_u64().unwrap();
    assert!(n <= 16384);
    let mut bytes = Vec::new();
    flate2::read::ZlibDecoder::new(raw.as_slice()).take(n + 1).read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes.len() as u64, n);
    assert_eq!(hash(&bytes), v["decoded_sha256"]);
    bytes
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_alpha_clip_finite_80_unit_states_and_80_nonunit_negatives() {
    let corpus = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let mut count = 0;
    let mut rejected = 0;
    let mut worst = 0f64;
    for (folder, manifest) in [
        ("point-alpha-clip-value-domain-evidence", "f2086a1aa7289d539eafc88267100904f63af8dc2acd882a032923b5f14b6ab4"),
        ("point-alpha-clip-value-domain-nonlinear-controls", "b06006d13044f754a9cfa1b7e0877ea144ce4584d6e2b8e16fd49d2ad4bc6f9b"),
    ] {
        let root = corpus.join(folder);
        assert_eq!(hash(&fs::read(root.join("batch-manifest.json")).unwrap()), manifest);
        let obs: Value = serde_json::from_slice(&fs::read(root.join("observations.json")).unwrap()).unwrap();
        for row in obs["records"].as_array().unwrap() {
            let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&bytes), row["sha256"]);
            if row["id"].as_str().unwrap().contains(".alpha-nonunit.") {
                assert!(photocraft_io::import("alpha-clip-finite.xcf", &bytes).is_err());
                rejected += 1;
                continue;
            }
            if row["id"].as_str().unwrap().contains(".color-hdr.") {
                continue;
            }
            let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let doc = photocraft_io::import("alpha-clip.xcf", &bytes).unwrap().document;
            doc.validate_xcf_state().unwrap();
            let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            assert_eq!(node.operation.schema_epoch, "xcf22-stock-alpha-clip-finite-precision-v1");
            assert_eq!(node.arguments.len(), 4);
            assert_eq!(node.enabled, row["requested"]["enabled"].as_bool().unwrap());
            let original = blob(&root, &row["reopened"]["layers"][0]["native"]);
            let native = if raw.sample == photocraft_xcf::Sample::F16 {
                original
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|chunk| chunk.as_slice())
                    .flat_map(|v| photocraft_color::widen_f16_to_f32_bits(u16::from_le_bytes(v.try_into().unwrap())).to_ne_bytes())
                    .collect::<Vec<_>>()
            } else {
                original
            };
            assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), native);
            assert_eq!(raw.layers[0].plane.as_ref().unwrap().bytes, native);
            assert_eq!(
                doc.xcf_state.as_ref().unwrap().current_filter_output_precision,
                if raw.sample == photocraft_xcf::Sample::F16 {
                    photocraft_doc::FilterOutputPrecision::Half
                } else {
                    photocraft_doc::FilterOutputPrecision::Canonical
                }
            );
            let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_eq!(reopened, doc);
            let mut stream = std::io::Cursor::new(Vec::new());
            let (_, result) = photocraft_io::save_native_stream(&doc, &mut stream, Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
            assert!(result.warnings.is_empty());
            assert!(stream.get_ref().windows(4).any(|bytes| bytes == b"PK\x06\x06"));
            let stream_reopened =
                photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
            assert_eq!(stream_reopened, doc);
            let expected = blob(&root, &row["reopened"]["projection"]);
            use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
            let encoding = match raw.encoding {
                photocraft_xcf::Encoding::Linear => Encoding::Linear,
                photocraft_xcf::Encoding::NonlinearProfile => Encoding::NonlinearProfile,
                photocraft_xcf::Encoding::Perceptual => Encoding::Perceptual,
            };
            let gray = doc.mode == photocraft_doc::ColorMode::Grayscale;
            let channels = if gray { 1 } else { 3 };
            let profile = implicit_source_profile(
                doc.icc_profile.as_ref().unwrap(),
                if gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb },
                encoding,
            )
            .unwrap();
            for candidate in [&doc, &reopened, &stream_reopened] {
                let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
                for (actual, reference) in rendered.px.iter().zip(expected.as_chunks::<32>().0.iter().map(|chunk| chunk.as_slice())) {
                    let mut color = [0.; 3];
                    convert_encoding_f64(&profile, encoding, Encoding::NonlinearProfile, &actual[..channels], &mut color[..channels]).unwrap();
                    if gray {
                        color = [color[0]; 3];
                    }
                    let alpha = f64::from_le_bytes(reference[24..32].try_into().unwrap());
                    assert!((actual[3] - alpha).abs() <= 1. / 65535., "{} alpha {}", row["id"], (actual[3] - alpha).abs());
                    for (i, value) in color.iter().enumerate() {
                        let expected = f64::from_le_bytes(reference[i * 8..i * 8 + 8].try_into().unwrap());
                        let error = (value * actual[3] - expected * alpha).abs();
                        worst = worst.max(error);
                        assert!(error <= 2. / 255., "{} color {} /255", row["id"], error * 255.);
                    }
                }
            }
            assert_eq!(fs::read(root.join(row["file"].as_str().unwrap())).unwrap(), bytes);
            count += 1;
        }
    }
    assert_eq!(rejected, 80);
    assert_eq!(count, 80);
    println!("80 finite Alpha Clip unit-source states; max {} /255", worst * 255.);
}
