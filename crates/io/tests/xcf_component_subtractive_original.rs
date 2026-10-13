#![cfg(feature = "corpus")]
//! Independent CMYK/YCbCr source/current/native controls; other selectors remain closed.
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
fn check_family(family: &str, manifest_hash: &str, expected_count: usize, expected_epoch: &str) {
    let packet = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(format!("point-component-extract-{family}-evidence"));
    use sha2::{Digest, Sha256};
    let manifest = std::fs::read(packet.join("batch-manifest.json")).unwrap();
    assert_eq!(Sha256::digest(&manifest).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), manifest_hash);
    let mut count = 0;
    let mut worst = 0f64;
    for group in ["states"] {
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
                (Some(-305954223), Some(-305954223), Some(-305954223))
            );
            let doc = photocraft_io::import("component-extract.xcf", &bytes).unwrap().document;
            doc.validate_xcf_state().unwrap();
            let state = doc.xcf_state.as_ref().unwrap();
            assert_eq!(state.filter_stacks[0].nodes[0].operation.schema_epoch, expected_epoch);
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
            assert_eq!(node.enabled, row["enabled"].as_bool().unwrap());
            assert_eq!(node.operation.stored_version, "0:0");
            assert_eq!(node.arguments.len(), 3);
            let expected_component = match row["component"].as_str().unwrap() {
                "cmyk-c" => 8,
                "cmyk-m" => 9,
                "cmyk-y" => 10,
                "cmyk-k" => 11,
                "ycbcr-y" => 12,
                "ycbcr-cb" => 13,
                "ycbcr-cr" => 14,
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
                    if row["enabled"].as_bool().unwrap() {
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
    assert_eq!(count, expected_count);
    println!("{count} {family} wholeDoc native legacy/ZIP64 Component states max {}/255", worst * 255.);
}

#[test]
#[ignore = "local unpublished1260 independent CMYK/YCbCr originals"]
fn original_subtractive_components_import_native_and_projection() {
    check_family("cmyk", "ed5791616c890aeed174d57b086a104e79179b098a62e4335c7b86236bd9d163", 720, "xcf-22-stock-component-extract-cmyk-precision-v1");
    check_family("ycbcr", "6bb9b502dd09dcffa69d717f871bc91678acfa26fea1e3c345176722112d4623", 540, "xcf-22-stock-component-extract-ycbcr-precision-v1");
}

#[test]
#[ignore = "local unpublished CMYK/YCbCr source envelope and typed schema controls"]
fn subtractive_original_source_policy_and_typed_argument_negatives() {
    let mut rejected = 0;
    for (family, first) in [("cmyk", "cmyk-c"), ("ycbcr", "ycbcr-y")] {
        let root =
            std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(format!("point-component-extract-{family}-evidence/states"));
        let rows: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        for row in rows["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["component"] == first && row["enabled"] == true && row["invert"] == false && row["linear"] == false)
        {
            let path = root.join(row["file"].as_str().unwrap());
            let bytes = std::fs::read(&path).unwrap();
            // Positive baseline prevents a globally closed importer from passing.
            photocraft_io::import("positive.xcf", &bytes).unwrap();
            let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let effect = &raw.layers[0].effects[0];
            for (tag, value) in [(7, 28u32), (35, 0), (36, 0), (37, 0), (44, 1), (46, 1), (33, 0.5f32.to_bits())] {
                let property = effect.properties.iter().find(|p| p.tag == tag).unwrap();
                assert_eq!(property.payload.len(), 4);
                let start = usize::try_from(property.offset).unwrap();
                let mut changed = bytes.clone();
                changed[start..start + 4].copy_from_slice(&value.to_be_bytes());
                assert!(photocraft_io::import("declared-hostile-policy.xcf", &changed).is_err(), "{} tag{tag}", row["id"]);
                rejected += 1;
            }
            for name in ["component", "invert", "linear"] {
                let argument = effect.arguments.iter().find(|a| a.name == name).unwrap();
                let property = &effect.properties[argument.property_index];
                let end = usize::try_from(property.offset).unwrap() + property.payload.len();
                let mut changed = bytes.clone();
                if name == "component" {
                    // The remaining ten operation choices have no admission.
                    changed[end - 4..end].copy_from_slice(&3u32.to_be_bytes());
                } else {
                    // An exact Float32 zero must never become a Boolean alias.
                    assert_eq!(&changed[end - 8..end - 4], &2u32.to_be_bytes());
                    changed[end - 8..end - 4].copy_from_slice(&3u32.to_be_bytes());
                }
                assert!(photocraft_io::import("declared-hostile-argument.xcf", &changed).is_err(), "{} {name}", row["id"]);
                rejected += 1;
            }
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }
    assert_eq!(rejected, 720);
    println!("{rejected} source policy/typed argument rejections after actual positive baselines");
}
