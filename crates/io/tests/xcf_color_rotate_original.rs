#![cfg(feature = "corpus")]
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|n| format!("{n:02x}")).collect()
}
fn originals() -> (PathBuf, Value) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/color-rotate-perceptual-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "8b338d140af0add90f7390e7fc2db4f0adef9dac24f45151ff50140dd66bb59c");
    let census: Value = serde_json::from_slice(&manifest).unwrap();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    let entry = census["files"].as_array().unwrap().iter().find(|n| n["file"] == "observations.json").unwrap();
    assert_eq!(hash(&bytes), entry["sha256"].as_str().unwrap());
    (root, serde_json::from_slice(&bytes).unwrap())
}
#[test]
#[ignore = "independent native Perceptual source schema before admission"]
fn color_rotate_saved_wire_and_native_six() {
    let (root, observations) = originals();
    for row in observations["records"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"].as_str().unwrap());
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!((file.version, file.precision, file.width, file.height), (22, 675, 32, 16));
        assert_eq!(file.layers.len(), 1);
        let owner = &file.layers[0];
        let effect = &owner.effects[0];
        assert_eq!(owner.offset, (0, 0));
        assert_eq!(effect.operation, "gegl:color-rotate");
        assert_eq!(effect.version.as_deref(), Some("0:0"));
        eprintln!(
            "{} owner=({},{},{},{}) filter=({:?},{:?},{:?},{:?}) enabled={:?} region={:?} clipping={:?} args={:?}",
            row["id"],
            owner.mode,
            owner.blend_space,
            owner.composite_space,
            owner.composite_mode,
            effect.state.mode,
            effect.state.blend_space,
            effect.state.composite_space,
            effect.state.composite_mode,
            effect.state.enabled,
            effect.state.region,
            effect.state.clipping,
            effect.arguments
        );
        let mask = effect.mask.as_ref().unwrap();
        eprintln!(
            "mask size{}x{} offset{:?} mode{} policy{}/{}/{} enabled{} shown{} editing{} linked{} visible{}",
            mask.width,
            mask.height,
            mask.offset,
            mask.mode,
            mask.blend_space,
            mask.composite_space,
            mask.composite_mode,
            mask.mask_enabled,
            mask.mask_shown,
            mask.mask_editing,
            mask.legacy_linked,
            mask.visible
        );
        assert_eq!((mask.width, mask.height, mask.offset), (32, 16, (0, 0)));
        assert!(mask.plane.as_ref().unwrap().bytes.iter().all(|n| *n == 0));
        let descriptor = &row["current"]["layers"][0]["native"];
        let native = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&native), descriptor["sha256"].as_str().unwrap());
        assert_eq!(owner.plane.as_ref().unwrap().bytes, native);
    }
}
#[test]
#[ignore = "independent Perceptual source import runtime gate"]
fn color_rotate_original_six_import() {
    let (root, observations) = originals();
    for row in observations["records"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"].as_str().unwrap());
        let imported = photocraft_io::import("independent-color-rotate.xcf", &bytes).unwrap();
        imported.document.validate_xcf_state().unwrap();
    }
}

#[test]
#[ignore = "independent six Perceptual native/legacy/ZIP64/live projections"]
fn color_rotate_native_reopen_projection_six() {
    let (root, observations) = originals();
    native_roundtrip_projection(&root, &observations);
}
fn native_roundtrip_projection(root: &std::path::Path, observations: &Value) {
    use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
    let mut maximum = 0f64;
    for row in observations["records"].as_array().unwrap() {
        let source = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&source), row["sha256"]);
        let doc = photocraft_io::import("color-rotate.xcf", &source).unwrap().document;
        let descriptor = &row["current"]["layers"][0]["native"];
        let native = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&native), descriptor["sha256"]);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), native);
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened, doc);
        let (stream, _) = photocraft_format::PcraftWriter::new()
            .save_zip_stream(&doc, &Default::default(), std::io::Cursor::new(Vec::new()), Default::default(), 64 << 20, &photocraft_raster::Interrupt::NONE)
            .unwrap();
        let streamed = photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(streamed, doc);
        let gray = doc.mode == photocraft_doc::ColorMode::Grayscale;
        let channels = if gray { 1 } else { 3 };
        let profile = implicit_source_profile(
            doc.icc_profile.as_ref().unwrap(),
            if gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb },
            Encoding::Perceptual,
        )
        .unwrap();
        let descriptor = &row["current"]["projection"];
        let projection = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&projection), descriptor["sha256"]);
        assert_eq!(projection.len(), 512 * 32);
        for candidate in [&doc, &reopened, &streamed] {
            let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(rendered.px.len(), 512);
            for (pixel, reference) in rendered.px.iter().zip(projection.as_chunks::<32>().0.iter().map(|chunk| chunk.as_slice())) {
                let mut color = [0.; 3];
                convert_encoding_f64(&profile, Encoding::Perceptual, Encoding::NonlinearProfile, &pixel[..channels], &mut color[..channels]).unwrap();
                if gray {
                    color = [color[0]; 3];
                }
                let alpha = f64::from_le_bytes(reference[24..32].try_into().unwrap());
                assert!((pixel[3] - alpha).abs() <= 1. / 65535.);
                for (i, value) in color.iter().enumerate() {
                    let expected = f64::from_le_bytes(reference[i * 8..i * 8 + 8].try_into().unwrap());
                    let delta = (value * pixel[3] - expected * alpha).abs();
                    maximum = maximum.max(delta);
                    assert!(delta <= 2. / 255., "{} delta{}", row["id"], delta * 255.);
                }
            }
        }
    }
    println!("six Color Rotate native/reopen/ZIP64/projected max {}/255", maximum * 255.);
}

#[test]
#[ignore = "independent fractional originals raw and typed source admission"]
fn color_rotate_boundary_eight_raw_and_typed_admission() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/color-rotate-perceptual-boundary-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "72e93210100f51ce988929d0ba9010f817b7ca5c1e69cb4f66caaae0eeacc9ac");
    let observations: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let rows = observations["records"].as_array().unwrap();
    assert_eq!(rows.len(), 8);
    for row in rows {
        let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"].as_str().unwrap());
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!((file.version, file.precision, file.width, file.height), (22, 675, 32, 16));
        let effect = &file.layers[0].effects[0];
        assert_eq!(effect.operation, "gegl:color-rotate");
        assert_eq!(effect.version.as_deref(), Some("0:0"));
        assert_eq!(effect.arguments.len(), 10);
        eprintln!("{} raw={:?}/{:?}/{:?}", row["id"], effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode);
        let original = std::fs::read(root.join(row["current"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(file.layers[0].plane.as_ref().unwrap().bytes, original);
        let imported = photocraft_io::import("boundary.xcf", &bytes).unwrap().document;
        assert_eq!(imported.layers[0].surface().unwrap().try_read_interleaved(imported.bounds()).unwrap(), original);
    }
}

#[test]
#[ignore = "independent eight fractional-boundary Perceptual native/live/legacy/ZIP64 controls"]
fn color_rotate_boundary_eight_native_pipeline() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/color-rotate-perceptual-boundary-evidence");
    let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "72e93210100f51ce988929d0ba9010f817b7ca5c1e69cb4f66caaae0eeacc9ac");
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), manifest["files"].as_array().unwrap().iter().find(|r| r["file"] == "observations.json").unwrap()["sha256"]);
    let observations: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(observations["records"].as_array().unwrap().len(), 8);
    native_roundtrip_projection(&root, &observations);
}
