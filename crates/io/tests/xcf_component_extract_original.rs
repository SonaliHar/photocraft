#![cfg(feature = "corpus")]
//! Independently authored four-selector Component Extract; seventeen choices remain closed.
use std::io::Read;
fn data(root: &std::path::Path, descriptor: &serde_json::Value) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let encoded = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    assert_eq!(hash(&encoded), descriptor["sha256"].as_str().unwrap());
    let limit = descriptor["decoded_bytes"].as_u64().unwrap();
    assert!(limit <= 4096);
    let mut decoded = Vec::new();
    flate2::read::ZlibDecoder::new(encoded.as_slice()).take(limit + 1).read_to_end(&mut decoded).unwrap();
    assert_eq!(decoded.len() as u64, limit);
    assert_eq!(hash(&decoded), descriptor["decoded_sha256"].as_str().unwrap());
    decoded
}
#[test]
#[ignore = "local unpublished Component Extract public source/native controls"]
fn original_component_extract_imports_with_typed_opaque_output() {
    let packet = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-component-extract-evidence");
    use sha2::{Digest, Sha256};
    let manifest = std::fs::read(packet.join("batch-manifest.json")).unwrap();
    assert_eq!(
        Sha256::digest(&manifest).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "6b8e14eb4367f101dd65d072c3daa17c6f2577a4ccd1e9a1030289ebc7b231b8"
    );
    let mut count = 0;
    let mut worst = 0f64;
    for group in ["enabled", "disabled"] {
        let root = packet.join(group);
        let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        let rows = observations["records"].as_array().unwrap();
        for row in rows {
            let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
            assert_eq!(Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), row["sha256"].as_str().unwrap());
            let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert_eq!(raw.layers[0].mode, 28);
            assert_eq!((raw.layers[0].blend_space, raw.layers[0].composite_space, raw.layers[0].composite_mode), (0, -1, -1));
            let effect = &raw.layers[0].effects[0];
            assert_eq!(effect.operation, "gegl:component-extract");
            assert_eq!(effect.version.as_deref(), Some("0:0"));
            assert_eq!(effect.arguments.len(), 3);
            assert_eq!(effect.state.mode, Some(62));
            assert_eq!(effect.state.clipping, Some(0));
            assert_eq!(effect.state.region, Some(0));
            assert_eq!(
                (effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode),
                (Some(-466517423), Some(-466517423), Some(-466517423))
            );
            let doc = photocraft_io::import("component-extract.xcf", &bytes).unwrap().document;
            doc.validate_xcf_state().unwrap();
            let state = doc.xcf_state.as_ref().unwrap();
            assert_eq!(state.filter_stacks[0].nodes[0].operation.schema_epoch, "xcf-22-stock-component-extract-four-precision-v1");
            let expected = &row["current"];
            let original = data(&root, &expected["layers"][0]["native"]);
            let native = if raw.sample == photocraft_xcf::Sample::F16 {
                original
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|chunk| chunk.as_slice())
                    .flat_map(|word| photocraft_color::widen_f16_to_f32_bits(u16::from_le_bytes(word.try_into().unwrap())).to_ne_bytes())
                    .collect::<Vec<_>>()
            } else {
                original
            };
            assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), native);
            assert_eq!(raw.layers[0].plane.as_ref().unwrap().bytes, native);
            assert_eq!(
                state.current_filter_output_precision,
                if raw.sample == photocraft_xcf::Sample::F16 {
                    photocraft_doc::FilterOutputPrecision::Half
                } else {
                    photocraft_doc::FilterOutputPrecision::Canonical
                }
            );
            let mask = effect.mask.as_ref().unwrap();
            assert_eq!((mask.width, mask.height, mask.offset), (8, 6, (0, 0)));
            assert!(mask.plane.as_ref().unwrap().bytes.iter().all(|byte| *byte == 0));
            assert_eq!(state.filter_masks[0].surface.try_read_interleaved(doc.bounds()).unwrap(), mask.plane.as_ref().unwrap().bytes);
            let node = &state.filter_stacks[0].nodes[0];
            assert_eq!(node.enabled, group == "enabled");
            assert_eq!(node.operation.stored_version, "0:0");
            assert_eq!(node.arguments.len(), 3);
            let expected_component = match row["component"].as_str().unwrap() {
                "rgb-r" => 0,
                "rgb-g" => 1,
                "rgb-b" => 2,
                "alpha" => 20,
                _ => panic!("unqualified choice"),
            };
            for argument in &node.arguments {
                use photocraft_doc::xcf_filters::FilterValue;
                match argument.name.as_str() {
                    "component" => {
                        assert_eq!(argument.value, FilterValue::Enum { domain: "gegl:component-extract/component".into(), value: expected_component })
                    }
                    "invert" => assert_eq!(argument.value, FilterValue::Bool(row["invert"].as_bool().unwrap())),
                    "linear" => assert_eq!(argument.value, FilterValue::Bool(row["linear"].as_bool().unwrap())),
                    _ => panic!("unknown typed field"),
                }
            }
            let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_eq!(reopened, doc);
            let (stream, _) = photocraft_format::PcraftWriter::new()
                .save_zip_stream(&doc, &Default::default(), std::io::Cursor::new(Vec::new()), Default::default(), 64 << 20, &photocraft_raster::Interrupt::NONE)
                .unwrap();
            let streamed = photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
            assert_eq!(streamed, doc);
            let expected_projection = data(&root, &expected["projection"]);
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
            for candidate in [&doc, &reopened, &streamed] {
                let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
                assert_eq!(rendered.px.len(), 48);
                for (pixel, reference) in rendered.px.iter().zip(expected_projection.as_chunks::<32>().0.iter().map(|chunk| chunk.as_slice())) {
                    let mut encoded = [0.; 3];
                    convert_encoding_f64(&profile, encoding, Encoding::NonlinearProfile, &pixel[..channels], &mut encoded[..channels]).unwrap();
                    if gray {
                        encoded = [encoded[0]; 3];
                    }
                    let alpha = f64::from_le_bytes(reference[24..32].try_into().unwrap());
                    assert!((pixel[3] - alpha).abs() <= 1. / 65535., "{} alpha", row["id"]);
                    if group == "enabled" {
                        assert_eq!(pixel[3], 1.);
                    }
                    for channel in 0..3 {
                        let expected = f64::from_le_bytes(reference[channel * 8..channel * 8 + 8].try_into().unwrap());
                        let delta = (encoded[channel] * pixel[3] - expected * alpha).abs();
                        worst = worst.max(delta);
                        assert!(delta <= 2. / 255., "{} {} /255", row["id"], delta * 255.);
                    }
                }
            }
            assert_eq!(std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap(), bytes);
            count += 1;
        }
    }
    assert_eq!(count, 720);
    println!("720 wholeDoc native legacy/ZIP64 Component states max {}/255", worst * 255.);
}
