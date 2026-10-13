#![cfg(feature = "corpus")]
//! Independently authored Absolute source authority; admission is separate.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{io::Read, path::PathBuf};

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn observations() -> (PathBuf, Value) {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-absolute-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "00689e14034ac9fc1753c426650213353035307b271c4bbe3279ab1b1172c3bf");
    let census: Value = serde_json::from_slice(&manifest).unwrap();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    let entry = census["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == "observations.json").unwrap();
    assert_eq!(hash(&bytes), entry["sha256"]);
    (root, serde_json::from_slice(&bytes).unwrap())
}
fn blob(root: &std::path::Path, descriptor: &Value) -> Vec<u8> {
    let bytes = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
    assert!(bytes.len() <= 4096);
    assert_eq!(hash(&bytes), descriptor["sha256"]);
    if descriptor["encoding"] == "zlib" {
        let length = descriptor["decoded_bytes"].as_u64().unwrap();
        assert!(length <= 2048);
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(bytes.as_slice()).take(length + 1).read_to_end(&mut raw).unwrap();
        assert_eq!(raw.len() as u64, length);
        assert_eq!(hash(&raw), descriptor["decoded_sha256"]);
        raw
    } else {
        bytes
    }
}

#[test]
#[ignore = "unpublished108 signed-HDR Absolute originals; raw contract before admission"]
fn original_absolute_zero_argument_native_and_mask_wire_are_exact() {
    let (root, report) = observations();
    let rows = report["records"].as_array().unwrap();
    assert_eq!(rows.len(), 108);
    let mut triplets = std::collections::BTreeSet::new();
    for row in rows {
        let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!(file.layers.len(), 1);
        let owner = &file.layers[0];
        assert_eq!((owner.mode, owner.blend_space, owner.composite_space, owner.composite_mode), (28, 0, -1, -1));
        assert_eq!(owner.effects.len(), 1);
        let effect = &owner.effects[0];
        assert_eq!(effect.operation, "gegl:absolute");
        assert_eq!(effect.version.as_deref(), Some("0:0"));
        assert!(effect.arguments.is_empty());
        assert_eq!((effect.state.mode, effect.state.clipping, effect.state.region), (Some(62), Some(0), Some(0)));
        triplets.insert((effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode));
        let mask = effect.mask.as_ref().unwrap();
        assert_eq!((mask.width, mask.height, mask.offset), (8, 6, (0, 0)));
        assert_eq!(mask.model, photocraft_xcf::Model::Gray);
        assert!(!mask.alpha && mask.visible && mask.mask_enabled && !mask.mask_shown && !mask.mask_editing && !mask.legacy_linked);
        assert_eq!((mask.mode, mask.blend_space, mask.composite_space, mask.composite_mode), (0, 2, 2, 1));
        assert!(mask.effects.is_empty() && mask.unsupported.is_empty());
        assert!(mask.plane.as_ref().unwrap().bytes.iter().all(|b| *b == 0));
        let native = blob(&root, &row["reopened"]["layers"][0]["native"]);
        let canonical = if file.sample == photocraft_xcf::Sample::F16 {
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
        assert_eq!(owner.plane.as_ref().unwrap().bytes, canonical);
    }
    println!("independently parsed Absolute108 exact raw triplets={triplets:?}");
    assert_eq!(triplets, std::collections::BTreeSet::from([(Some(1574142545), Some(1574142545), Some(1574142545))]));
}

#[test]
#[ignore = "unpublished independent108 Absolute import/reference candidate; meaningful unsupported RED"]
fn original_absolute_import_preserves_source_before_live_evaluation() {
    let (root, report) = observations();
    for row in report["records"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let imported = photocraft_io::import("independent-absolute.xcf", &bytes).unwrap();
        imported.document.validate_xcf_state().unwrap();
        assert_eq!(imported.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].operation.name, "gegl:absolute");
    }
}

#[test]
#[ignore = "independent36 qualified Linear Absolute originals; remaining72 transfer candidates refused"]
fn original_absolute_linear_import_is_distinct_from_unqualified_transfers() {
    let (root, report) = observations();
    let mut counts = (0, 0);
    for row in report["records"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let precision = row["precision"].as_str().unwrap();
        if precision.ends_with("_LINEAR") && !precision.ends_with("_NON_LINEAR") {
            let imported = photocraft_io::import("independent-absolute.xcf", &bytes).unwrap();
            imported.document.validate_xcf_state().unwrap();
            let node = &imported.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            assert_eq!(node.operation.name, "gegl:absolute");
            assert_eq!(node.operation.schema_epoch, "xcf-22-stock-absolute-linear-precision-v1");
            assert!(node.arguments.is_empty());
            photocraft_compose::gimp_render::PreparedRenderer::new(&imported.document).unwrap();
            counts.0 += 1;
        } else {
            assert!(photocraft_io::import("independent-absolute.xcf", &bytes).is_err());
            counts.1 += 1;
        }
    }
    assert_eq!(counts, (36, 72));
}

#[test]
#[ignore = "unpublished independent36 Linear Absolute original native/projection/legacy/ZIP64 references"]
fn original_absolute_linear_native_projection_legacy_and_zip64_reopen() {
    use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-absolute-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "00689e14034ac9fc1753c426650213353035307b271c4bbe3279ab1b1172c3bf");
    let census: Value = serde_json::from_slice(&manifest).unwrap();
    let entry = census["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == "observations.json").unwrap();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), entry["sha256"]);
    let observations: Value = serde_json::from_slice(&bytes).unwrap();
    let rows = observations["records"].as_array().unwrap();
    assert_eq!(rows.len(), 108);
    let mut maximum = 0f64;
    let mut active = 0;
    let mut disabled = 0;
    for row in rows.iter().filter(|r| r["precision"].as_str().is_some_and(|p| p.ends_with("_LINEAR") && !p.ends_with("_NON_LINEAR"))) {
        let path = root.join(row["file"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let imported = photocraft_io::import("independent-channel-mixer.xcf", &bytes).unwrap();
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
        assert_eq!(node.operation.schema_epoch, "xcf-22-stock-absolute-linear-precision-v1");
        assert!(node.arguments.is_empty());
        assert_eq!(node.enabled, row["reopened"]["layers"][0]["filters"][0]["visible"].as_bool().unwrap());
        if node.enabled {
            active += 1;
        } else {
            disabled += 1;
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
    assert_eq!((active, disabled), (24, 12));
    println!("36 Absolute Linear source/native/typed/legacy/ZIP64/live projections max {}/255", maximum * 255.);
}
