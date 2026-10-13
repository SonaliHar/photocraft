#![cfg(feature = "corpus")]
//! Independently authored Component Extract offset/extent controls, separate from8x6 epochs.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
fn data(root: &Path, descriptor: &Value) -> Vec<u8> {
    let encoded = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
    assert!(encoded.len() <= 2 << 20);
    assert_eq!(hash(&encoded), descriptor["sha256"].as_str().unwrap());
    let limit = descriptor["decoded_bytes"].as_u64().unwrap();
    assert!(limit <= 2 << 20);
    let mut output = Vec::new();
    flate2::read::ZlibDecoder::new(encoded.as_slice()).take(limit + 1).read_to_end(&mut output).unwrap();
    assert_eq!(output.len() as u64, limit);
    assert_eq!(hash(&output), descriptor["decoded_sha256"].as_str().unwrap());
    output
}
#[test]
#[ignore = "local unpublished132 independent Component Extract geometry originals"]
fn original_component_offset_extent_native_projection_and_streaming_reopen() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-component-extract-geometry-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "fe13672593245e18d8224faec99c18a3bfcd10cd5860427f60494e5d29396e88");
    let observations: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let rows = observations["records"].as_array().unwrap();
    assert_eq!(rows.len(), 132);
    let mut maximum = 0f64;
    for row in rows {
        let path = root.join(row["source"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(hash(&bytes), row["sha256"].as_str().unwrap());
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!(raw.sample, photocraft_xcf::Sample::F32);
        assert_eq!(raw.encoding, photocraft_xcf::Encoding::NonlinearProfile);
        assert_eq!(raw.layers.len(), 1);
        let layer = &raw.layers[0];
        assert_eq!(layer.mode, 28);
        assert_eq!((layer.blend_space, layer.composite_space, layer.composite_mode), (0, -1, -1));
        assert_eq!(layer.effects.len(), 1);
        let effect = &layer.effects[0];
        assert_eq!(effect.operation, "gegl:component-extract");
        assert_eq!(effect.version.as_deref(), Some("0:0"));
        assert_eq!(effect.arguments.len(), 3);
        assert_eq!(effect.state.mode, Some(62));
        assert_eq!(effect.state.clipping, Some(0));
        assert_eq!(effect.state.region, Some(0));
        assert_eq!((effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode), (Some(87327313), Some(87327313), Some(87327313)));
        let mask = effect.mask.as_ref().unwrap();
        assert_eq!((mask.width, mask.height, mask.offset), (raw.width, raw.height, (0, 0)));
        let mask_plane = mask.plane.as_ref().unwrap();
        assert_eq!(mask_plane.sample, photocraft_xcf::Sample::F32);
        assert_eq!(mask_plane.channels, 1);
        assert!(mask_plane.bytes.iter().all(|byte| *byte == 0));
        let expected = &row["current"];
        let extent = photocraft_geom::Rect::from_xywh(layer.offset.0, layer.offset.1, layer.width, layer.height);
        let native = data(&root, &expected["layers"][0]["native"]);
        assert_eq!(layer.plane.as_ref().unwrap().bytes, native);
        let doc = photocraft_io::import("component-geometry.xcf", &bytes).unwrap().document;
        doc.validate_xcf_state().unwrap();
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(extent).unwrap(), native);
        let state = doc.xcf_state.as_ref().unwrap();
        let node = &state.filter_stacks[0].nodes[0];
        assert_eq!(node.operation.schema_epoch, "xcf-22-stock-component-extract-geometry-v1");
        assert_eq!(node.enabled, row["enabled"].as_bool().unwrap());
        assert_eq!(state.current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
        assert_eq!(state.filter_masks[0].surface.try_read_interleaved(doc.bounds()).unwrap(), mask_plane.bytes);
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened, doc);
        let (stream, _) = photocraft_format::PcraftWriter::new()
            .save_zip_stream(&doc, &Default::default(), std::io::Cursor::new(Vec::new()), Default::default(), 64 << 20, &photocraft_raster::Interrupt::NONE)
            .unwrap();
        let streamed = photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(streamed, doc);
        let projection = data(&root, &expected["projection"]);
        for candidate in [&doc, &reopened, &streamed] {
            let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(rendered.px.len() * 32, projection.len());
            for (index, (pixel, reference)) in rendered.px.iter().zip(projection.as_chunks::<32>().0.iter().map(|chunk| chunk.as_slice())).enumerate() {
                let alpha = f64::from_le_bytes(reference[24..32].try_into().unwrap());
                assert!((pixel[3] - alpha).abs() <= 1. / 65535., "{} pixel{index} actual {} reference {alpha}", row["id"], pixel[3]);
                let x = index % raw.width as usize;
                let y = index / raw.width as usize;
                if node.enabled && extent.contains(x as i32, y as i32) {
                    assert_eq!(pixel[3], 1.);
                }
                for channel in 0..3 {
                    let expected = f64::from_le_bytes(reference[channel * 8..channel * 8 + 8].try_into().unwrap());
                    let delta = (pixel[channel] * pixel[3] - expected * alpha).abs();
                    maximum = maximum.max(delta);
                    assert!(delta <= 2. / 255., "{} {} /255", row["id"], delta * 255.);
                }
            }
        }
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    println!("132 Component geometry wholeDoc/native/legacy/ZIP64/live controls max {}/255", maximum * 255.);
}
