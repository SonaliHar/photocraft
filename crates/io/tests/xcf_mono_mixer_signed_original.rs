#![cfg(feature = "corpus")]
//! Independent saved expanded Mono Mixer source, typed state, live render and native round trips.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|n| format!("{n:02x}")).collect()
}
fn blob(root: &Path, d: &Value) -> Vec<u8> {
    let bytes = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
    assert!(bytes.len() <= 16384);
    assert_eq!(bytes.len() as u64, d["bytes"].as_u64().unwrap());
    assert_eq!(hash(&bytes), d["sha256"]);
    bytes
}
#[test]
#[ignore = "reviewed unpublished144 independent expanded Mono Mixer originals"]
fn original_expanded_mono_mixer_all_native_typed_projection_legacy_and_zip64_reopen() {
    use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("mono-mixer-signed-precision-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "e63618b1a99e38852cd35207e9120e2c6e3a2700433b2a7e281683650155e194");
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
        let imported = photocraft_io::import("expanded-mono-mixer.xcf", &bytes).unwrap();
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
        assert_eq!(node.operation.schema_epoch, "xcf-22-stock-mono-mixer-signed-precision-v1");
        assert_eq!(node.arguments.len(), 4);
        assert_eq!(node.enabled, row["enabled"].as_bool().unwrap());
        if node.enabled {
            active += 1;
        } else {
            disabled += 1;
        }
        for (i, name) in ["red", "green", "blue"].iter().enumerate() {
            assert_eq!(
                node.arguments.iter().find(|a| a.name == *name).unwrap().value,
                FilterValue::Number(ExactNumber::Float32Bits((row["weights"][i].as_f64().unwrap() as f32).to_bits()))
            );
        }
        assert_eq!(node.arguments.iter().find(|a| a.name == "preserve-luminosity").unwrap().value, FilterValue::Bool(row["preserve"].as_bool().unwrap()));
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
    println!("144 expanded Mono Mixer source/native/typed/legacy/ZIP64/live projections max {}/255", maximum * 255.);
}

#[test]
#[ignore = "independent unpublished144 expanded Mono Mixer source and empty-mask observations"]
fn independent_expanded_mono_raw_typed_envelope_and_mask_audit() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("mono-mixer-signed-precision-evidence");
    let observations: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let rows = observations["records"].as_array().unwrap();
    assert_eq!(rows.len(), 144);
    for row in rows {
        let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!((file.version, file.width, file.height), (22, 8, 6));
        let owner = &file.layers[0];
        assert_eq!((owner.mode, owner.blend_space, owner.composite_space, owner.composite_mode), (28, 0, -1, -1));
        let effect = &owner.effects[0];
        assert_eq!(effect.operation, "gegl:mono-mixer");
        assert_eq!(effect.version.as_deref(), Some("0:0"));
        assert_eq!(effect.arguments.len(), 4);
        assert_eq!(
            (effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode),
            (Some(1799848529), Some(1799848529), Some(1799848529))
        );
        assert_eq!((effect.state.mode, effect.state.clipping, effect.state.region), (Some(62), Some(0), Some(0)));
        assert_eq!(effect.state.opacity_bits, Some(1f32.to_bits()));
        assert_eq!(effect.state.enabled, Some(row["enabled"].as_bool().unwrap()));
        for (index, name) in ["red", "green", "blue"].iter().enumerate() {
            let a = effect.arguments.iter().find(|a| a.name == *name).unwrap();
            assert!(a.configuration.is_none() && a.graph.is_none());
            assert_eq!(a.value, photocraft_xcf::ArgumentValue::Float32Bits((row["weights"][index].as_f64().unwrap() as f32).to_bits()));
        }
        assert_eq!(
            effect.arguments.iter().find(|a| a.name == "preserve-luminosity").unwrap().value,
            photocraft_xcf::ArgumentValue::Boolean(row["preserve"].as_bool().unwrap())
        );
        let mask = effect.mask.as_ref().unwrap();
        assert_eq!((mask.width, mask.height, mask.offset), (8, 6, (0, 0)));
        assert_eq!(mask.model, photocraft_xcf::Model::Gray);
        assert!(!mask.alpha && mask.visible && mask.mask_enabled && !mask.mask_shown && !mask.mask_editing && !mask.legacy_linked);
        assert_eq!((mask.mode, mask.blend_space, mask.composite_space, mask.composite_mode), (0, 2, 2, 1));
        assert!(mask.effects.is_empty() && mask.unsupported.is_empty());
        let plane = mask.plane.as_ref().unwrap();
        assert_eq!(plane.channels, 1);
        assert_eq!((plane.width, plane.height), (8, 6));
        assert_eq!(plane.sample, if file.sample == photocraft_xcf::Sample::F16 { photocraft_xcf::Sample::F32 } else { file.sample });
        assert!(plane.bytes.iter().all(|byte| *byte == 0));
    }
}
