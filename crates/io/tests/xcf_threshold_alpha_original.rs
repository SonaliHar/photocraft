#![cfg(feature = "corpus")]
//! Independent linear GEGL Alpha Threshold precision/model/alpha references.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, io::Cursor, path::PathBuf};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
#[test]
#[ignore = "requires local unpublished independently authored Alpha Threshold precision originals"]
fn original_alpha_threshold_states_native_projection_and_streaming_reopen() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("alpha-threshold-precision-evidence");
    let bytes = fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&bytes), "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263");
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    for (name, expected) in manifest.as_object().unwrap() {
        assert_eq!(hash(&fs::read(root.join(name)).unwrap()), expected.as_str().unwrap());
    }
    let requests: Value = serde_json::from_slice(&fs::read(root.join("observations.json")).unwrap()).unwrap();
    let proof: Value = serde_json::from_slice(&fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    let mut identities = std::collections::BTreeSet::new();
    let mut worst = 0.0_f64;
    for row in proof["checks"].as_array().unwrap().iter() {
        let prefix = row["id"].as_str().unwrap();
        let expected = &row["fresh"];
        let bytes = fs::read(root.join(format!("{prefix}.xcf"))).unwrap();
        identities.insert(hash(&bytes));
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let mask = raw.layers[0].effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap();
        assert_eq!(mask.sample, raw.sample);
        assert_eq!(mask.channels, 1);
        assert!(mask.bytes.iter().all(|v| *v == 0));
        let doc = photocraft_io::import("threshold.xcf", &bytes).unwrap().document;
        let before = doc.clone();
        let native = fs::read(root.join(expected["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&native), expected["layers"][0]["native"]["sha256"]);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), native);
        assert_eq!(raw.layers[0].plane.as_ref().unwrap().bytes, native);
        let state = doc.xcf_state.as_ref().unwrap();
        let node = &state.filter_stacks[0].nodes[0];
        assert_eq!(node.operation.name, "gimp:threshold-alpha");
        assert_eq!(node.operation.stored_version, "0:0");
        assert_eq!(node.operation.schema_epoch, "xcf-22-stock-threshold-alpha-precision-v1");
        let requested = &requests["records"].as_array().unwrap().iter().find(|r| r["id"] == prefix).unwrap()["requested"];
        assert_eq!(node.enabled, requested["enabled"].as_bool().unwrap());
        assert_eq!(node.arguments.len(), 1);
        {
            let name = "value";
            let argument = node.arguments.iter().find(|a| a.name == name).unwrap();
            assert_eq!(
                argument.value,
                photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(
                    (requested[name].as_f64().unwrap() as f32).to_bits()
                ))
            );
        }
        assert_eq!(state.filter_masks.len(), 1);
        assert_eq!(state.filter_masks[0].surface.try_read_interleaved(doc.bounds()).unwrap(), mask.bytes);
        let native = photocraft_io::export(&doc, "threshold.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("threshold.pcraft", &native.bytes).unwrap().document;
        assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "threshold.pcraft"));
        let (stream, _) = photocraft_format::PcraftWriter::new()
            .save_zip_stream(&doc, &Default::default(), Cursor::new(Vec::new()), Default::default(), 64 << 20, &photocraft_raster::Interrupt::NONE)
            .unwrap();
        let streamed = photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(streamed, doc);
        let projection = fs::read(root.join(expected["projection"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&projection), expected["projection"]["sha256"]);
        assert_eq!(projection.len(), 48 * 32);
        use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
        let linear = doc.sample_encoding == Some(photocraft_doc::SampleEncoding::Linear);
        let encoding = if linear { Encoding::Linear } else { Encoding::NonlinearProfile };
        let gray = doc.mode == photocraft_doc::ColorMode::Grayscale;
        let channels = if gray { 1 } else { 3 };
        let profile = implicit_source_profile(
            doc.icc_profile.as_ref().unwrap(),
            if gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb },
            encoding,
        )
        .unwrap();
        for candidate in [&doc, &reopened, &streamed] {
            let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(rendered.px.len(), 48);
            for (pixel, reference) in rendered.px.iter().zip(projection.as_chunks::<32>().0.iter().map(|chunk| chunk.as_slice())) {
                // U16 generated projection storage is bounded; floating samples remain unclipped.
                let stored: [f64; 3] = std::array::from_fn(|i| if doc.depth == photocraft_doc::SampleType::U16 { pixel[i].clamp(0.0, 1.0) } else { pixel[i] });
                let mut encoded = [0.0; 3];
                convert_encoding_f64(&profile, encoding, Encoding::NonlinearProfile, &stored[..channels], &mut encoded[..channels]).unwrap();
                if gray {
                    encoded = [encoded[0]; 3];
                }
                let alpha = f64::from_le_bytes(reference[24..32].try_into().unwrap());
                assert!((pixel[3] - alpha).abs() <= 1.0 / 65535.0);
                for channel in 0..3 {
                    let value = f64::from_le_bytes(reference[channel * 8..channel * 8 + 8].try_into().unwrap());
                    let delta = (encoded[channel] * pixel[3] - value * alpha).abs();
                    worst = worst.max(delta);
                    assert!(delta <= 2.0 / 255.0, "{prefix} premultiplied colour");
                    if alpha > 0.0 {
                        assert!((encoded[channel] - value).abs() <= 2.0 / 255.0, "{prefix} visible colour");
                    }
                }
            }
        }
        assert_eq!(doc, before);
        assert_eq!(fs::read(root.join(format!("{prefix}.xcf"))).unwrap(), bytes);
    }
    assert_eq!(identities.len(), 60);
    println!("60 independent RGB/Gray Alpha Threshold states native/streaming/projection max {}/255", worst * 255.0);
}
