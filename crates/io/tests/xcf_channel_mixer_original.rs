#![cfg(feature = "corpus")]
//! Local original Channel Mixer candidates; source wire and current evaluator are separate gates.
use serde_json::Value;
use sha2::{Digest, Sha256};

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}

fn observations() -> (std::path::PathBuf, Value) {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("channel-mixer-precision-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "91be8128c1d351336cd7ef304dd81d42b010aedba1610384aed9526a7ca04f20");
    let census: Value = serde_json::from_slice(&manifest).unwrap();
    let descriptor = census["artifacts"].as_array().unwrap().iter().find(|v| v["path"] == "observations.json").unwrap();
    let raw = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&raw), descriptor["sha256"]);
    (root, serde_json::from_slice(&raw).unwrap())
}

#[test]
#[ignore = "requires local unpublished144 original Channel Mixer candidate states"]
fn original_typed_gain_widths_mask_and_raw_envelope_are_exact() {
    let (root, observations) = observations();
    let rows = observations["records"].as_array().unwrap();
    assert_eq!(rows.len(), 144);
    for row in rows {
        let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let parsed = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let owner = &parsed.layers[0];
        assert_eq!((owner.mode, owner.blend_space, owner.composite_space, owner.composite_mode), (28, 0, -1, -1));
        let effect = &owner.effects[0];
        assert_eq!(effect.operation, "gegl:channel-mixer");
        assert_eq!(effect.version.as_deref(), Some("0:0"));
        assert_eq!(effect.arguments.len(), 10);
        let config = &row["current"]["layers"][0]["filters"][0]["config"];
        for argument in &effect.arguments {
            let value = config.get(&argument.name).unwrap();
            if argument.name == "preserve-luminosity" {
                assert_eq!(argument.value, photocraft_xcf::ArgumentValue::Boolean(value.as_bool().unwrap()));
            } else {
                assert_eq!(argument.value, photocraft_xcf::ArgumentValue::Float32Bits((value.as_f64().unwrap() as f32).to_bits()));
            }
        }
        assert_eq!((effect.state.mode, effect.state.clipping, effect.state.region), (Some(62), Some(0), Some(0)));
        assert_eq!(
            (effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode),
            (Some(-1150320047), Some(-1150320047), Some(-1150320047))
        );
        let mask = effect.mask.as_ref().unwrap();
        assert_eq!((mask.width, mask.height, mask.offset), (8, 6, (0, 0)));
        assert_eq!(mask.model, photocraft_xcf::Model::Gray);
        assert!(!mask.alpha && mask.visible && mask.mask_enabled && !mask.mask_shown && !mask.mask_editing && !mask.legacy_linked);
        assert!(mask.effects.is_empty() && mask.unsupported.is_empty());
        assert!(mask.plane.as_ref().unwrap().bytes.iter().all(|b| *b == 0));
    }
}

#[test]
#[ignore = "requires local unpublished144 originals; actual source admission TDD"]
fn original_channel_mixer_import_preserves_typed_source_before_evaluation() {
    let (root, observations) = observations();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let imported = photocraft_io::import("independent-channel-mixer.xcf", &bytes).unwrap();
        imported.document.validate_xcf_state().unwrap();
        assert_eq!(imported.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].operation.name, "gegl:channel-mixer");
        count += 1;
    }
    assert_eq!(count, 144);
}

fn blob(root: &std::path::Path, descriptor: &Value) -> Vec<u8> {
    let bytes = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
    assert!(bytes.len() <= 16384);
    assert_eq!(bytes.len() as u64, descriptor["bytes"].as_u64().unwrap());
    assert_eq!(hash(&bytes), descriptor["sha256"]);
    bytes
}

#[test]
#[ignore = "unpublished independent144 Channel Mixer original native/projection/legacy/ZIP64 references"]
fn original_channel_mixer_all_native_typed_projection_legacy_and_zip64_reopen() {
    use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("channel-mixer-precision-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "91be8128c1d351336cd7ef304dd81d42b010aedba1610384aed9526a7ca04f20");
    let census: Value = serde_json::from_slice(&manifest).unwrap();
    let entry = census["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == "observations.json").unwrap();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), entry["sha256"]);
    let observations: Value = serde_json::from_slice(&bytes).unwrap();
    let rows = observations["records"].as_array().unwrap();
    assert_eq!(rows.len(), 144);
    let mut maximum = 0f64;
    let mut active = 0;
    let mut disabled = 0;
    for row in rows {
        let path = root.join(row["file"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let imported = photocraft_io::import("independent-channel-mixer.xcf", &bytes).unwrap();
        let doc = imported.document;
        doc.validate_xcf_state().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, raw.precision);
        let native = blob(&root, &row["current"]["layers"][0]["native"]);
        let canonical = if raw.sample == photocraft_xcf::Sample::F16 {
            native
                .as_chunks::<2>()
                .0
                .iter()
                .map(|chunk| chunk.as_slice())
                .flat_map(|v| photocraft_color::widen_f16_to_f32_bits(u16::from_le_bytes(v.try_into().unwrap())).to_ne_bytes())
                .collect::<Vec<_>>()
        } else {
            native
        };
        assert_eq!(raw.layers[0].plane.as_ref().unwrap().bytes, canonical);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), canonical);
        let state = doc.xcf_state.as_ref().unwrap();
        let node = &state.filter_stacks[0].nodes[0];
        assert_eq!(node.operation.schema_epoch, "xcf-22-stock-channel-mixer-precision-v1");
        assert_eq!(node.arguments.len(), 10);
        assert_eq!(node.enabled, row["current"]["layers"][0]["filters"][0]["visible"].as_bool().unwrap());
        if node.enabled {
            active += 1;
        } else {
            disabled += 1;
        }
        let config = &row["current"]["layers"][0]["filters"][0]["config"];
        for argument in &node.arguments {
            let expected = if argument.name == "preserve-luminosity" {
                FilterValue::Bool(config[&argument.name].as_bool().unwrap())
            } else {
                FilterValue::Number(ExactNumber::Float32Bits((config[&argument.name].as_f64().unwrap() as f32).to_bits()))
            };
            assert_eq!(argument.value, expected);
        }
        assert_eq!(
            state.current_filter_output_precision,
            if raw.sample == photocraft_xcf::Sample::F16 {
                photocraft_doc::FilterOutputPrecision::Half
            } else {
                photocraft_doc::FilterOutputPrecision::Canonical
            }
        );
        assert_eq!(
            state.filter_masks[0].surface.try_read_interleaved(doc.bounds()).unwrap(),
            raw.layers[0].effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap().bytes
        );
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened, doc);
        let (stream, _) = photocraft_format::PcraftWriter::new()
            .save_zip_stream(&doc, &Default::default(), std::io::Cursor::new(Vec::new()), Default::default(), 64 << 20, &photocraft_raster::Interrupt::NONE)
            .unwrap();
        let streamed = photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(streamed, doc);
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
        let projection = blob(&root, &row["current"]["projection"]);
        assert_eq!(projection.len(), 8 * 6 * 32);
        for candidate in [&doc, &reopened, &streamed] {
            let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(rendered.px.len(), 48);
            for (pixel, reference) in rendered.px.iter().zip(projection.as_chunks::<32>().0.iter().map(|chunk| chunk.as_slice())) {
                let mut color = [0.; 3];
                convert_encoding_f64(&profile, encoding, Encoding::NonlinearProfile, &pixel[..channels], &mut color[..channels]).unwrap();
                if gray {
                    color = [color[0]; 3];
                }
                let alpha = f64::from_le_bytes(reference[24..32].try_into().unwrap());
                assert!((pixel[3] - alpha).abs() <= 1. / 65535., "{} alpha {}", row["id"], (pixel[3] - alpha).abs());
                for (i, value) in color.iter().enumerate() {
                    let expected = f64::from_le_bytes(reference[i * 8..i * 8 + 8].try_into().unwrap());
                    let delta = (value * pixel[3] - expected * alpha).abs();
                    maximum = maximum.max(delta);
                    assert!(delta <= 2. / 255., "{} {} /255", row["id"], delta * 255.);
                }
            }
        }
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    assert_eq!((active, disabled), (108, 36));
    println!("144 Channel Mixer source/native/typed/legacy/ZIP64/live projections max {}/255", maximum * 255.);
}
