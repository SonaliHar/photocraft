#![cfg(feature = "corpus")]
//! Independent linear GEGL Color Temperature precision/model/alpha references.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, io::Cursor, path::PathBuf};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
#[test]
#[ignore = "requires local unpublished independently authored Color Temperature precision originals"]
fn original_precision_model_states_native_projection_and_streaming_reopen() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("color-temperature-precision-evidence");
    let bytes = fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "f727e979471d42eff4535cda98f5adcef9f43cf0f47578dae99a43f2fb104b3c");
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    for asset in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(hash(&fs::read(root.join(asset["path"].as_str().unwrap())).unwrap()), asset["sha256"]);
    }
    let proof: Value = serde_json::from_slice(&fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    let mut identities = std::collections::BTreeSet::new();
    let mut worst = 0.0_f64;
    for row in proof["checks"].as_array().unwrap() {
        let prefix = row["id"].as_str().unwrap();
        let expected = &row["fresh"];
        let bytes = fs::read(root.join(format!("{prefix}.xcf"))).unwrap();
        identities.insert(hash(&bytes));
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let mask = raw.layers[0].effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap();
        assert_eq!(mask.sample, raw.sample);
        assert_eq!(mask.channels, 1);
        assert!(mask.bytes.iter().all(|v| *v == 0));
        let doc = photocraft_io::import("color-temperature.xcf", &bytes).unwrap().document;
        let before = doc.clone();
        let native = fs::read(root.join(expected["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&native), expected["layers"][0]["native"]["sha256"]);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), native);
        assert_eq!(raw.layers[0].plane.as_ref().unwrap().bytes, native);
        let state = doc.xcf_state.as_ref().unwrap();
        let node = &state.filter_stacks[0].nodes[0];
        assert_eq!(node.operation.name, "gegl:color-temperature");
        assert_eq!(node.operation.stored_version, "0:0");
        assert_eq!(node.operation.schema_epoch, "xcf-22-stock-color-temperature-precision-v1");
        assert_eq!(node.enabled, expected["layers"][0]["filters"][0]["visible"].as_bool().unwrap());
        assert_eq!(node.arguments.len(), 2);
        for argument in &node.arguments {
            let photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(bits)) = argument.value else {
                panic!("wrong stored scalar type")
            };
            assert_eq!(f64::from(f32::from_bits(bits)), expected["layers"][0]["filters"][0]["config"][&argument.name].as_f64().unwrap());
        }
        assert_eq!(state.filter_masks.len(), 1);
        assert_eq!(state.filter_masks[0].surface.try_read_interleaved(doc.bounds()).unwrap(), mask.bytes);
        let native = photocraft_io::export(&doc, "color-temperature.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("color-temperature.pcraft", &native.bytes).unwrap().document;
        assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "color-temperature.pcraft"));
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
    assert_eq!(identities.len(), 48);
    println!("48 independent Color Temperature states native/streaming/projection max {}/255", worst * 255.0);
}

#[test]
#[ignore = "local unpublished stock Color Temperature raw-mask observations"]
fn raw_original_mask_envelopes_and_four_state_configs() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("color-temperature-precision-evidence");
    let manifest: Value = serde_json::from_slice(&fs::read(root.join("batch-manifest.json")).unwrap()).unwrap();
    let proof: Value = serde_json::from_slice(&fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    for row in proof["checks"].as_array().unwrap() {
        let name = format!("{}.xcf", row["id"].as_str().unwrap());
        let bytes = fs::read(root.join(&name)).unwrap();
        assert_eq!(hash(&bytes), manifest["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == name).unwrap()["sha256"]);
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let layer = &raw.layers[0];
        let effect = &layer.effects[0];
        let mask = effect.mask.as_ref().unwrap();
        let plane = mask.plane.as_ref().unwrap();
        assert_eq!(
            (raw.version, layer.width, layer.height, layer.offset, layer.mode, layer.blend_space, layer.composite_space, layer.composite_mode),
            (22, 8, 6, (0, 0), 28, 0, -1, -1)
        );
        assert_eq!(
            (mask.width, mask.height, mask.offset, mask.visible, mask.mask_enabled, mask.mask_shown, mask.mask_editing, mask.legacy_linked),
            (8, 6, (0, 0), true, true, false, false, false)
        );
        assert_eq!((mask.mode, mask.blend_space, mask.composite_space, mask.composite_mode), (0, 2, 2, 1));
        assert!(!mask.alpha);
        assert_eq!(plane.sample, raw.sample);
        assert_eq!(plane.channels, 1);
        assert!(plane.bytes.iter().all(|b| *b == 0));
    }
}

#[test]
#[ignore = "requires local unpublished independently authored Color Temperature precision originals"]
fn off_grid_precision_model_states_native_projection_and_streaming_reopen() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("color-temperature-offgrid-evidence");
    let bytes = fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "8105afb381c5178a375138b3ee6de73f477bf06c3a16d589a0373fc6b76e57de");
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    for asset in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(hash(&fs::read(root.join(asset["path"].as_str().unwrap())).unwrap()), asset["sha256"]);
    }
    let packet: Value = serde_json::from_slice(&fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    let checks: Vec<Value> = packet["checks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| {
            ["source", "edit"].map(|role| serde_json::json!({"id":format!("{}.{}",row["id"].as_str().unwrap(),role),"fresh":row["checks"][role]["fresh"]}))
        })
        .collect();
    let proof = serde_json::json!({"checks":checks});
    let mut identities = std::collections::BTreeSet::new();
    let mut worst = 0.0_f64;
    for row in proof["checks"].as_array().unwrap() {
        let prefix = row["id"].as_str().unwrap();
        let expected = &row["fresh"];
        let bytes = fs::read(root.join(format!("{prefix}.xcf"))).unwrap();
        identities.insert(hash(&bytes));
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let mask = raw.layers[0].effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap();
        assert_eq!(mask.sample, raw.sample);
        assert_eq!(mask.channels, 1);
        assert!(mask.bytes.iter().all(|v| *v == 0));
        let doc = photocraft_io::import("color-temperature.xcf", &bytes).unwrap().document;
        let before = doc.clone();
        let native = fs::read(root.join(expected["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&native), expected["layers"][0]["native"]["sha256"]);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), native);
        assert_eq!(raw.layers[0].plane.as_ref().unwrap().bytes, native);
        let state = doc.xcf_state.as_ref().unwrap();
        let node = &state.filter_stacks[0].nodes[0];
        assert_eq!(node.operation.name, "gegl:color-temperature");
        assert_eq!(node.operation.stored_version, "0:0");
        assert_eq!(
            node.operation.schema_epoch,
            if prefix.ends_with(".edit") {
                let needle = 7031.125_f32.to_be_bytes();
                let offsets: Vec<_> = bytes.windows(4).enumerate().filter_map(|(offset, value)| (value == needle).then_some(offset)).collect();
                assert_eq!(offsets.len(), 1, "bound stored original-temperature payload");
                let mut altered_source = bytes.clone();
                altered_source[offsets[0]..offsets[0] + 4].copy_from_slice(&7031.25_f32.to_be_bytes());
                assert!(photocraft_io::import("altered-source.xcf", &altered_source).is_err());
                "xcf-22-stock-color-temperature-offgrid-v1"
            } else {
                "xcf-22-stock-color-temperature-precision-v1"
            }
        );
        assert_eq!(node.enabled, expected["layers"][0]["filters"][0]["visible"].as_bool().unwrap());
        assert_eq!(node.arguments.len(), 2);
        if prefix.ends_with(".edit") {
            for variant in 0..4 {
                let mut changed = doc.clone();
                let node = &mut changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
                match variant {
                    0 => node.operation.schema_epoch = "unqualified-offgrid".into(),
                    1 => node.composition.as_mut().unwrap().stored_operator += 1,
                    2 => {
                        node.arguments.iter_mut().find(|argument| argument.name == "original-temperature").unwrap().value =
                            photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(999.0_f32.to_bits()))
                    }
                    _ => {
                        node.arguments.iter_mut().find(|argument| argument.name == "intended-temperature").unwrap().value =
                            photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float64Bits(5299.375_f64.to_bits()))
                    }
                }
                assert!(
                    photocraft_compose::gimp_render::try_render(&changed, changed.bounds(), &mut |_| Ok(())).is_err(),
                    "{prefix} altered off-grid context {variant}"
                );
                assert_eq!(doc, before);
            }
        }

        for argument in &node.arguments {
            let photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(bits)) = argument.value else {
                panic!("wrong stored scalar type")
            };
            assert_eq!(f64::from(f32::from_bits(bits)), expected["layers"][0]["filters"][0]["config"][&argument.name].as_f64().unwrap());
        }
        assert_eq!(state.filter_masks.len(), 1);
        assert_eq!(state.filter_masks[0].surface.try_read_interleaved(doc.bounds()).unwrap(), mask.bytes);
        let native = photocraft_io::export(&doc, "color-temperature.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("color-temperature.pcraft", &native.bytes).unwrap().document;
        assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "color-temperature.pcraft"));
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
    assert_eq!(identities.len(), 24);
    println!("24 source/off-grid Color Temperature states native/streaming/projection max {}/255", worst * 255.0);
}
