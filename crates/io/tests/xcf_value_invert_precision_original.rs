#![cfg(feature = "corpus")]
//! Mixed-alpha Value Invert originals at three independently observed precisions.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, io::Cursor, path::PathBuf};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
#[test]
#[ignore = "requires reviewed independently authored Value Invert precision originals"]
fn precision_enable_states_preserve_native_samples_masks_and_streaming_reopen() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("value-invert-precision-evidence");
    let manifest = fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "8a75ccbb88e61611d3c3523db8145f922b63676601986c124d2d8493a27e73a1");
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    for asset in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(hash(&fs::read(root.join(asset["path"].as_str().unwrap())).unwrap()), asset["sha256"]);
    }
    let mut identities = std::collections::BTreeSet::new();
    for family in ["u16_non_linear", "float_linear", "double_non_linear"] {
        for stage in ["source", "disabled", "reenabled"] {
            let prefix = format!("{family}.{stage}");
            let bytes = fs::read(root.join(format!("{prefix}.xcf"))).unwrap();
            identities.insert(hash(&bytes));
            let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let expected: Value = serde_json::from_slice(&fs::read(root.join(format!("{prefix}.fresh.state.json"))).unwrap()).unwrap();
            let doc = photocraft_io::import("value-invert.xcf", &bytes).unwrap().document;
            let before = doc.clone();
            let native = fs::read(root.join(expected["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&native), expected["layers"][0]["native"]["sha256"]);
            assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), native);
            assert_eq!(raw.layers[0].plane.as_ref().unwrap().bytes, native);
            let state = doc.xcf_state.as_ref().unwrap();
            let node = &state.filter_stacks[0].nodes[0];
            assert_eq!(node.operation.name, "gegl:value-invert");
            assert_eq!(node.operation.schema_epoch, "xcf-22-stock-value-invert-precision-v1");
            assert_eq!(node.enabled, stage != "disabled");
            assert!(node.arguments.is_empty());
            let mask = raw.layers[0].effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap();
            assert_eq!(mask.sample, raw.sample);
            assert_eq!(mask.channels, 1);
            assert!(mask.bytes.iter().all(|v| *v == 0));
            assert_eq!(state.filter_masks.len(), 1);
            assert_eq!(state.filter_masks[0].surface.try_read_interleaved(doc.bounds()).unwrap(), mask.bytes);
            let native = photocraft_io::export(&doc, "value-invert.pcraft", &Default::default()).unwrap();
            let reopened = photocraft_io::import("value-invert.pcraft", &native.bytes).unwrap().document;
            assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "value-invert.pcraft"));
            let (stream, _) = photocraft_format::PcraftWriter::new()
                .save_zip_stream(&doc, &Default::default(), Cursor::new(Vec::new()), Default::default(), 64 << 20, &photocraft_raster::Interrupt::NONE)
                .unwrap();
            let streamed = photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
            assert_eq!(streamed, doc);
            let projection = fs::read(root.join(expected["projection"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&projection), expected["projection"]["sha256"]);
            assert_eq!(projection.len(), 48 * 32);
            use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
            let encoding = if family == "float_linear" { Encoding::Linear } else { Encoding::NonlinearProfile };
            let profile = implicit_source_profile(doc.icc_profile.as_ref().unwrap(), photocraft_cms::ColorSpace::Rgb, encoding).unwrap();
            for candidate in [&doc, &reopened, &streamed] {
                let actual = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
                assert_eq!(actual.px.len(), 48);
                for (pixel, reference) in actual.px.iter().zip(projection.as_chunks::<32>().0.iter().map(|chunk| chunk.as_slice())) {
                    let mut encoded = [0.0; 3];
                    convert_encoding_f64(&profile, encoding, Encoding::NonlinearProfile, &pixel[..3], &mut encoded).unwrap();
                    let alpha = f64::from_le_bytes(reference[24..32].try_into().unwrap());
                    // GIMP's double projection exposes an intermediate f32
                    // normalization for integer alpha. Native U16 bits above
                    // remain exact; compare this derived projection separately.
                    assert!((pixel[3] - alpha).abs() <= 1.0 / 65535.0);
                    for channel in 0..3 {
                        let expected = f64::from_le_bytes(reference[channel * 8..channel * 8 + 8].try_into().unwrap());
                        assert!((encoded[channel] * pixel[3] - expected * alpha).abs() <= 2.0 / 255.0, "{prefix} premultiplied channel {channel}");
                        if alpha > 0.0 {
                            assert!((encoded[channel] - expected).abs() <= 2.0 / 255.0, "{prefix} visible channel {channel}");
                        }
                    }
                }
            }
            assert_eq!(doc, before);
            assert_eq!(fs::read(root.join(format!("{prefix}.xcf"))).unwrap(), bytes);
        }
    }
    assert_eq!(identities.len(), 6, "reenables repeat the original content; do not double-count");
}
