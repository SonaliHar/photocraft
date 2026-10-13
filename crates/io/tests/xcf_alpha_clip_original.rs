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
fn original_alpha_clip_132_native_states_and_diagnostics() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-alpha-clip-evidence");
    assert_eq!(hash(&fs::read(root.join("batch-manifest.json")).unwrap()), "54e7058bbbb97c7fbfe189cce0dbcf6d23c91be0022c35f6cc59183ae6a1c19d");
    let bytes = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), "527bc502322bcbe33d0ecb7a659363be85d0ce99a46e1fc552a77d0c77cef38d");
    let obs: Value = serde_json::from_slice(&bytes).unwrap();
    let mut count = 0;
    let mut worst = 0f64;
    for row in obs["records"].as_array().unwrap() {
        let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        if row["role"].as_str().unwrap().starts_with("diagnostic") {
            assert!(photocraft_io::import("alpha-clip.xcf", &bytes).is_err());
            continue;
        }
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let doc = photocraft_io::import("alpha-clip.xcf", &bytes).unwrap().document;
        doc.validate_xcf_state().unwrap();
        let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert_eq!(node.operation.schema_epoch, "xcf22-stock-alpha-clip-precision-v1");
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
        for candidate in [&doc, &reopened] {
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
    let diagnostic_bytes = fs::read(root.join("range-diagnostics.json")).unwrap();
    assert_eq!(hash(&diagnostic_bytes), "c6f216919102ad61e629e16b7cc50e0e31b3ef8a336b26c0649249d0cb1faf66");
    let diagnostic: Value = serde_json::from_slice(&diagnostic_bytes).unwrap();
    assert_eq!(diagnostic["records"].as_array().unwrap().len(), 4);
    for row in diagnostic["records"].as_array().unwrap() {
        let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        assert!(photocraft_io::import("alpha-clip-diagnostic.xcf", &bytes).is_err(), "{}", row["id"]);
    }
    assert_eq!(count, 132);
    println!("132 AlphaClip source/native/reopen/projection states max{} /255;diagnostics rejected", worst * 255.);
}
