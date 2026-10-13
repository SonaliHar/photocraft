#![cfg(feature = "corpus")]
//! Synthetic refusal controls are distinct from future independent owner witnesses.
#[path = "common/red_eye_geometry_packet.rs"]
mod packet;

#[test]
#[ignore = "six independent full-owner originals and separate profile-matched canvas references"]
fn actual_six_canvas_projection_and_png_survive_legacy_and_zip64() {
    use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
    let (root, records) = packet::fresh_records().unwrap();
    let png_index = packet::fresh_png_index(&root).unwrap();
    let mut count = 0;
    let mut maximum = 0.0f64;
    for row in records {
        let id = row["id"].as_str().unwrap();
        let bytes = packet::fresh_artifact(&root, &row["saved"]).unwrap();
        let doc = photocraft_io::import("actual-geometry.xcf", &bytes).unwrap().document;
        let native = packet::fresh_artifact(&root, &row["current"]["owner"]["native"]).unwrap();
        let baked = packet::fresh_artifact(&root, &row["baked"]["after"]["owner"]["native"]).unwrap();
        let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let working_profile = implicit_source_profile(doc.icc_profile.as_ref().unwrap(), photocraft_cms::ColorSpace::Rgb, Encoding::NonlinearProfile).unwrap();
        let candidate = photocraft_compose::gimp_red_eye::RedEyeCandidate::new(
            &working_profile,
            doc.sample_encoding.unwrap(),
            photocraft_compose::gimp_red_eye::typed_threshold(&node.arguments).unwrap(),
        )
        .unwrap();
        let mut whole_owner = Vec::with_capacity(native.len());
        for pixel in native.as_chunks::<16>().0.iter() {
            let input: Vec<f64> = pixel.as_chunks::<4>().0.iter().map(|value| f64::from(f32::from_ne_bytes(*value))).collect();
            let output = candidate.apply_with(&input, node.enabled, &photocraft_raster::Interrupt::NONE, &|_| Ok(())).unwrap();
            whole_owner.extend(output.into_iter().flat_map(|value| (value as f32).to_ne_bytes()));
        }
        assert_eq!(whole_owner, baked, "{id}: whole-owner baked candidate includes offcanvas and alpha-zero hidden RGB");
        let extent = photocraft_geom::Rect::from_xywh(
            row["geometry"][3][0].as_i64().unwrap() as i32,
            row["geometry"][3][1].as_i64().unwrap() as i32,
            row["geometry"][2][0].as_u64().unwrap() as u32,
            row["geometry"][2][1].as_u64().unwrap() as u32,
        );
        let projection = packet::fresh_artifact(&root, &row["current"]["projection"]["formats"]["R'G'B'A double"]).unwrap();
        assert!(row["current"]["profiles"]["assigned"].is_null());
        let png_row = png_index["records"].as_array().unwrap().iter().find(|record| record["id"] == id).unwrap();
        assert_eq!(png_row["source_png"], row["png"]);
        packet::fresh_artifact(&root, &row["png"]).unwrap();
        let png = packet::bounded(&root.join(format!("{id}.png-decoded.bin")), 4 << 20).unwrap();
        assert_eq!(packet::hash(&png), png_row["observation"]["decoded_sha256"]);
        assert_eq!(png.len() as u64, png_row["observation"]["decoded_bytes"].as_u64().unwrap());
        packet::fresh_profile_png_contract(&root, &row["current"], png_row).unwrap();
        let legacy = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&legacy).unwrap();
        let (stream, _) = photocraft_format::PcraftWriter::new()
            .save_zip_stream(&doc, &Default::default(), std::io::Cursor::new(Vec::new()), Default::default(), 64 << 20, &photocraft_raster::Interrupt::NONE)
            .unwrap();
        let streamed = photocraft_format::load_from_stream(stream, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
        for candidate in [&doc, &reopened, &streamed] {
            assert_eq!(candidate.icc_profile, Some(photocraft_cms::Builtin::Srgb.profile().to_bytes()));
            assert_eq!(candidate, &doc);
            assert_eq!(candidate.layers[0].surface().unwrap().try_read_interleaved(extent).unwrap(), native, "{id}: offcanvas/hidden native RGB");
            let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(projection.len(), rendered.px.len() * 32);
            assert_eq!(png.len(), rendered.px.len() * 8);
            let profile =
                implicit_source_profile(candidate.icc_profile.as_ref().unwrap(), photocraft_cms::ColorSpace::Rgb, Encoding::NonlinearProfile).unwrap();
            for ((pixel, reference), png) in rendered.px.iter().zip(projection.as_chunks::<32>().0.iter()).zip(png.as_chunks::<8>().0.iter()) {
                let mut color = [0.0; 3];
                convert_encoding_f64(&profile, Encoding::NonlinearProfile, Encoding::NonlinearProfile, &pixel[..3], &mut color).unwrap();
                let alpha = f64::from_le_bytes(reference[24..32].try_into().unwrap());
                let png_alpha = f64::from(u16::from_be_bytes(png[6..8].try_into().unwrap())) / 65535.0;
                assert!((pixel[3] - alpha).abs() <= 1.0 / 65535.0, "{id}: projection alpha");
                assert!((pixel[3] - png_alpha).abs() <= 1.0 / 65535.0, "{id}: PNG alpha");
                for i in 0..3 {
                    let expected = f64::from_le_bytes(reference[i * 8..i * 8 + 8].try_into().unwrap());
                    let png_color = f64::from(u16::from_be_bytes(png[i * 2..i * 2 + 2].try_into().unwrap())) / 65535.0;
                    for delta in [(color[i] * pixel[3] - expected * alpha).abs(), (color[i] * pixel[3] - png_color * png_alpha).abs()] {
                        maximum = maximum.max(delta);
                        assert!(delta <= 2.0 / 255.0, "{id}: canvas premultiplied delta {delta}");
                    }
                }
            }
        }
        assert_eq!(packet::fresh_artifact(&root, &row["saved"]).unwrap(), bytes);
        count += 1;
    }
    assert_eq!(count, 6);
    eprintln!("6 originals/18 native continuations: premultiplied display max {maximum}; native hidden RGB unchanged");
}

#[test]
#[ignore = "six independently authored TEMP Red Eye owner geometry controls"]
fn actual_six_owner_geometry_originals_import_with_whole_native_owner() {
    let (root, records) = packet::fresh_records().unwrap();
    let mut refusals = Vec::new();
    for row in records {
        let id = row["id"].as_str().unwrap();
        let source = packet::fresh_artifact(&root, &row["saved"]).unwrap();
        let expected = packet::fresh_artifact(&root, &row["current"]["owner"]["native"]).unwrap();
        let decoded = photocraft_xcf::decode(&source, &Default::default()).unwrap();
        assert_eq!(decoded.layers[0].plane.as_ref().unwrap().bytes, expected, "{id}");
        match photocraft_io::import("actual-red-eye-geometry.xcf", &source) {
            Ok(imported) => {
                let doc = imported.document;
                let layer = &doc.layers[0];
                let offset = &row["geometry"][3];
                let size = &row["geometry"][2];
                let extent = photocraft_geom::Rect::from_xywh(
                    offset[0].as_i64().unwrap() as i32,
                    offset[1].as_i64().unwrap() as i32,
                    size[0].as_u64().unwrap() as u32,
                    size[1].as_u64().unwrap() as u32,
                );
                assert_eq!(layer.surface().unwrap().try_read_interleaved(extent).unwrap(), expected, "{id}: full offcanvas owner");
                let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
                assert_eq!(node.operation.schema_epoch, packet::NEW_EPOCH, "{id}");
            }
            Err(error) => {
                eprintln!("ACTUAL IMPORT RED {id}: {error}");
                refusals.push(id.to_owned());
            }
        }
    }
    assert!(refusals.is_empty(), "six witnessed originals require new domain: {refusals:?}");
}

#[test]
#[ignore = "old hash-bound Red Eye control with synthetic canvas declaration changes"]
fn changed_canvas_with_old_owner_and_mask_is_refused_after_real_decode() {
    let original = packet::old_source().unwrap();
    let imported = photocraft_io::import("old-red-eye-control.xcf", &original).unwrap();
    let node = &imported.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.operation.schema_epoch, packet::OLD_EPOCH);
    assert_ne!(node.operation.schema_epoch, packet::NEW_EPOCH);
    assert_eq!(node.composition.as_ref().unwrap().compatibility_epoch, packet::OLD_POLICY);
    for geometry in packet::GEOMETRIES {
        let mut altered = original.clone();
        // XCF22 fixed header: signature14, canvas width/height big-endian.
        assert_eq!(altered.get(..14), Some(b"gimp xcf v022\0".as_slice()));
        altered.get_mut(14..18).unwrap().copy_from_slice(&geometry.canvas.0.to_be_bytes());
        altered.get_mut(18..22).unwrap().copy_from_slice(&geometry.canvas.1.to_be_bytes());
        let decoded = photocraft_xcf::decode(&altered, &Default::default()).unwrap();
        assert_eq!((decoded.width, decoded.height), geometry.canvas);
        assert_eq!((decoded.layers[0].width, decoded.layers[0].height), (8, 6));
        assert_eq!(
            decoded.layers[0].plane.as_ref().unwrap().bytes,
            photocraft_xcf::decode(&original, &Default::default()).unwrap().layers[0].plane.as_ref().unwrap().bytes
        );
        assert!(matches!(photocraft_io::import("synthetic-canvas-only.xcf", &altered), Err(photocraft_io::IoError::Unsupported(_))), "{}", geometry.id);
        // A decoded DTO reshaped to the proposed whole-owner rectangle is still
        // synthetic; it is inspected here, never imported as a GIMP witness.
        let synthetic = packet::synthetic_decoded(geometry, true).unwrap();
        assert_eq!((synthetic.layers[0].width, synthetic.layers[0].height), geometry.owner);
        assert_eq!(synthetic.layers[0].offset, geometry.offset);
        assert_eq!(synthetic.layers[0].plane.as_ref().unwrap().bytes, packet::synthetic_native(geometry.owner).unwrap());
    }
}
