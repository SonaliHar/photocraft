#![cfg(feature = "corpus")]
//! Independent seven-family point Replace extent and offset references.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read},
    path::PathBuf,
};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn read(root: &std::path::Path, descriptor: &Value) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
    let path = root.join(descriptor["file"].as_str().ok_or("missing sidecar path")?);
    if std::fs::metadata(&path)?.len() > 2 << 20 {
        return Err("compressed sidecar limit".into());
    }
    let raw = std::fs::read(path)?;
    assert_eq!(hash(&raw), descriptor["sha256"]);
    let limit = descriptor["decoded_bytes"].as_u64().ok_or("missing decoded length")?;
    assert!(limit <= 2 << 20);
    let mut output = Vec::new();
    flate2::read::ZlibDecoder::new(raw.as_slice()).take(limit + 1).read_to_end(&mut output)?;
    assert_eq!(output.len() as u64, limit);
    assert_eq!(hash(&output), descriptor["decoded_sha256"]);
    Ok(output)
}

// Source Half bytes are authoritative; the canonical raster is exact IEEE F32
// widening, not a reinterpretation of two-byte native samples as normalized U16.
fn canonical_source(bytes: &[u8], sample: photocraft_xcf::Sample) -> Vec<u8> {
    if sample == photocraft_xcf::Sample::F16 {
        assert_eq!(bytes.len() % 2, 0);
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|chunk| chunk.as_slice())
            .flat_map(|word| photocraft_color::widen_f16_to_f32_bits(u16::from_le_bytes(word.try_into().unwrap())).to_ne_bytes())
            .collect()
    } else {
        bytes.to_vec()
    }
}

#[test]
#[ignore = "requires local unpublished independently authored seven-family point Replace six-precision Perceptual originals"]
fn original_point_replace_perceptual_native_projection_and_streaming_reopen() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-replace-perceptual-evidence");
    let bytes = fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "e6c2dcd48b3aff0ecddb2dacc134a897d83983c38eeacc79898d7a50818576c8");
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    for asset in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(hash(&fs::read(root.join(asset["path"].as_str().unwrap())).unwrap()), asset["sha256"]);
    }
    let proof: Value = serde_json::from_slice(&fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut identities = std::collections::BTreeSet::new();
    let mut worst = 0.0_f64;
    for row in proof["records"].as_array().unwrap() {
        let prefix = row["id"].as_str().unwrap();
        let expected = &row["reopened"];
        let bytes = fs::read(root.join(format!("{prefix}.xcf"))).unwrap();
        identities.insert(hash(&bytes));
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        println!(
            "{prefix}: owner mode={} tuple={}/{}/{} offset={:?} version={} precision={} alpha={} opacity={}",
            raw.layers[0].mode,
            raw.layers[0].blend_space,
            raw.layers[0].composite_space,
            raw.layers[0].composite_mode,
            raw.layers[0].offset,
            raw.version,
            raw.precision,
            raw.layers[0].alpha,
            raw.layers[0].opacity
        );
        let mask = raw.layers[0].effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap();
        assert_eq!(mask.sample, if raw.sample == photocraft_xcf::Sample::F16 { photocraft_xcf::Sample::F32 } else { raw.sample });
        assert_eq!(mask.channels, 1);
        assert_eq!(raw.layers[0].effects[0].mask.as_ref().unwrap().offset, (0, 0));
        assert_eq!(
            (raw.layers[0].effects[0].mask.as_ref().unwrap().width, raw.layers[0].effects[0].mask.as_ref().unwrap().height),
            (row["canvas"][0].as_u64().unwrap() as u32, row["canvas"][1].as_u64().unwrap() as u32)
        );
        assert!(mask.bytes.iter().all(|v| *v == 0));
        let doc = photocraft_io::import("exposure-black-level.xcf", &bytes).unwrap().document;
        let before = doc.clone();
        let native = read(&root, &expected["layers"][0]["native"]).unwrap();
        let offset = &row["offset"];
        let extent = photocraft_geom::Rect::from_xywh(
            offset[0].as_i64().unwrap() as i32,
            offset[1].as_i64().unwrap() as i32,
            row["drawable"][0].as_u64().unwrap() as u32,
            row["drawable"][1].as_u64().unwrap() as u32,
        );
        let canonical_native = canonical_source(&native, raw.sample);
        assert_eq!(
            doc.depth,
            match raw.sample {
                photocraft_xcf::Sample::U8 => photocraft_doc::SampleType::U8,
                photocraft_xcf::Sample::U16 => photocraft_doc::SampleType::U16,
                photocraft_xcf::Sample::U32 => photocraft_doc::SampleType::U32,
                photocraft_xcf::Sample::F16 | photocraft_xcf::Sample::F32 => photocraft_doc::SampleType::F32,
                photocraft_xcf::Sample::F64 => photocraft_doc::SampleType::F64,
            }
        );
        assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, raw.precision);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(extent).unwrap(), canonical_native);
        assert_eq!(raw.layers[0].plane.as_ref().unwrap().bytes, canonical_native);
        let state = doc.xcf_state.as_ref().unwrap();
        let node = &state.filter_stacks[0].nodes[0];
        assert_eq!(node.operation.name, row["operation"]);
        assert_eq!(node.operation.stored_version, if node.operation.name == "gegl:exposure" { "1:0" } else { "0:0" });
        assert_eq!(node.composition.as_ref().unwrap().compatibility_epoch, "xcf-22-stock-point-replace-fields-v1");
        assert_eq!(node.enabled, expected["layers"][0]["filters"][0]["visible"].as_bool().unwrap());
        assert_eq!(node.arguments.len(), expected["layers"][0]["filters"][0]["config"].as_object().unwrap().len());
        for argument in &node.arguments {
            let value = &expected["layers"][0]["filters"][0]["config"][&argument.name];
            use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
            match &argument.value {
                FilterValue::Number(ExactNumber::Float32Bits(bits)) => assert_eq!(f64::from(f32::from_bits(*bits)), value.as_f64().unwrap()),
                FilterValue::Bool(v) => assert_eq!(*v, value.as_bool().unwrap()),
                FilterValue::Enum { domain, value: ordinal } => {
                    assert_eq!(domain, "gegl:alien-map/color-model");
                    assert_eq!(*ordinal, if value == "hsl" { 1 } else { 0 });
                }
                _ => panic!("unexpected preserved typed argument"),
            }
        }
        assert_eq!(state.filter_masks.len(), 1);
        assert_eq!(state.filter_masks[0].surface.try_read_interleaved(doc.bounds()).unwrap(), mask.bytes);
        let native = photocraft_io::export(&doc, "exposure-black-level.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("exposure-black-level.pcraft", &native.bytes).unwrap().document;
        assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "exposure-black-level.pcraft"));
        let (stream, _) = photocraft_format::PcraftWriter::new()
            .save_zip_stream(&doc, &Default::default(), Cursor::new(Vec::new()), Default::default(), 64 << 20, &photocraft_raster::Interrupt::NONE)
            .unwrap();
        let streamed = photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(streamed, doc);
        let projection = read(&root, &expected["projection"]).unwrap();
        assert_eq!(projection.len(), (row["canvas"][0].as_u64().unwrap() * row["canvas"][1].as_u64().unwrap()) as usize * 32);
        use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
        assert_eq!(doc.sample_encoding, Some(photocraft_doc::SampleEncoding::Perceptual));
        let encoding = Encoding::Perceptual;
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
            assert_eq!(rendered.px.len(), (row["canvas"][0].as_u64().unwrap() * row["canvas"][1].as_u64().unwrap()) as usize);
            for (pixel, reference) in rendered.px.iter().zip(projection.as_chunks::<32>().0.iter().map(|chunk| chunk.as_slice())) {
                // U32 must reach integer storage before nonlinear display
                // conversion. Do not hide ordering errors by clamping here.
                if doc.depth == photocraft_doc::SampleType::U32 {
                    assert!(pixel[..3].iter().all(|sample| (0.0..=1.0).contains(sample)), "{prefix} U32 storage boundary");
                }
                let stored: [f64; 3] = std::array::from_fn(|i| {
                    if matches!(doc.depth, photocraft_doc::SampleType::U8 | photocraft_doc::SampleType::U16) { pixel[i].clamp(0.0, 1.0) } else { pixel[i] }
                });
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
    assert_eq!(identities.len(), 198);
    println!("198 independent point Replace extent states native/streaming/projection max {}/255", worst * 255.0);
}

#[test]
#[ignore = "local unpublished seven-family six-precision Perceptual original-byte contract"]
fn all198_original_point_perceptual_raw_masks_and_planes_are_bound() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-replace-perceptual-evidence");
    assert_eq!(hash(&fs::read(root.join("batch-manifest.json")).unwrap()), "e6c2dcd48b3aff0ecddb2dacc134a897d83983c38eeacc79898d7a50818576c8");
    let proof: Value = serde_json::from_slice(&fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in proof["records"].as_array().unwrap() {
        let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!(raw.version, 22);
        let owner = &raw.layers[0];
        assert_eq!(owner.mode, 28);
        assert_eq!(owner.offset, (row["offset"][0].as_i64().unwrap() as i32, row["offset"][1].as_i64().unwrap() as i32));
        assert_eq!(owner.plane.as_ref().unwrap().bytes, canonical_source(&read(&root, &row["reopened"]["layers"][0]["native"]).unwrap(), raw.sample));
        assert_eq!(owner.effects.len(), 1);
        let effect = &owner.effects[0];
        assert_eq!(effect.operation, row["operation"]);
        assert_eq!(effect.version.as_deref(), Some(if effect.operation == "gegl:exposure" { "1:0" } else { "0:0" }));
        let mask = effect.mask.as_ref().unwrap();
        assert_eq!(mask.offset, (0, 0));
        assert_eq!((mask.width, mask.height), (raw.width, raw.height));
        assert_eq!(mask.plane.as_ref().unwrap().sample, if raw.sample == photocraft_xcf::Sample::F16 { photocraft_xcf::Sample::F32 } else { raw.sample });
        assert!(mask.plane.as_ref().unwrap().bytes.iter().all(|v| *v == 0));
        println!("{}: {} canvas={}x{} owner={}x{} offset={:?}", row["id"], effect.operation, raw.width, raw.height, owner.width, owner.height, owner.offset);
        count += 1;
    }
    assert_eq!(count, 198);
}
