#![cfg(feature = "corpus")]
//! Independent saved Color Overlay source, typed state, live render and native round trips.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|n| format!("{n:02x}")).collect()
}
fn blob(root: &Path, d: &Value) -> Vec<u8> {
    let encoded = std::fs::read(root.join(d["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&encoded), d["sha256"]);
    let size = d["decoded_bytes"].as_u64().unwrap();
    assert!(size <= 16384);
    let mut bytes = Vec::new();
    flate2::read::ZlibDecoder::new(encoded.as_slice()).take(size + 1).read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes.len() as u64, size);
    assert_eq!(hash(&bytes), d["decoded_sha256"]);
    bytes
}
#[test]
#[ignore = "reviewed unpublished360 independent Color Overlay originals"]
fn original_color_overlay_all_native_typed_projection_legacy_and_zip64_reopen() {
    use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-color-overlay-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "828c1da757d0706cd6eb502699c74c0c2856745d3603c6ab6d1dd56d70df9831");
    let census: Value = serde_json::from_slice(&manifest).unwrap();
    let entry = census["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == "observations.json").unwrap();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), entry["sha256"]);
    let observations: Value = serde_json::from_slice(&bytes).unwrap();
    let rows = observations["records"].as_array().unwrap();
    assert_eq!(rows.len(), 360);
    let mut maximum = 0f64;
    let mut active = 0;
    let mut disabled = 0;
    for row in rows {
        let path = root.join(row["source"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let imported = photocraft_io::import("color-overlay.xcf", &bytes).unwrap();
        let doc = imported.document;
        doc.validate_xcf_state().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, raw.precision);
        let native = blob(&root, &row["reopened"]["layers"][0]["native"]);
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
        assert_eq!(node.operation.schema_epoch, "xcf-22-stock-color-overlay-six-precision-v1");
        assert_eq!(node.arguments.len(), 2);
        assert_eq!(node.enabled, row["enabled"].as_bool().unwrap());
        if node.enabled {
            active += 1;
        } else {
            disabled += 1;
        }
        let FilterValue::Color { components, encoding: photocraft_doc::SampleEncoding::NonlinearProfile, profile: None } =
            &node.arguments.iter().find(|a| a.name == "value").unwrap().value
        else {
            panic!("wrong typed literal")
        };
        for (i, component) in components.iter().enumerate() {
            assert_eq!(*component, ExactNumber::Float32Bits((row["requested_color"][i].as_f64().unwrap() as f32).to_bits()));
        }
        assert_eq!(node.arguments.iter().find(|a| a.name == "srgb").unwrap().value, FilterValue::Bool(row["srgb"].as_bool().unwrap()));
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
        let (public_stream, _) =
            photocraft_io::save_native_stream(&doc, std::io::Cursor::new(Vec::new()), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
        let public_reopened =
            photocraft_format::load_from_stream(public_stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(public_reopened, doc);
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
        let projection = blob(&root, &row["reopened"]["projection"]);
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
    assert_eq!((active, disabled), (288, 72));
    println!("360 Color Overlay source/native/typed/legacy/ZIP64/live projections max {}/255", maximum * 255.);
}
