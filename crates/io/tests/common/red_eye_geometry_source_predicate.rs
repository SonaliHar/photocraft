//! Historical design refusal: resized OLD DTOs are not fresh source evidence.
#[path = "red_eye_geometry_packet.rs"]
mod packet;

#[test]
#[ignore = "actual six originals plus isolated hostile source DTO mutations"]
fn actual_geometry_source_hostiles_and_cancellation_are_refused() {
    use photocraft_xcf::{ArgumentValue, Encoding, Model, Sample};
    let (root, records) = packet::fresh_records().unwrap();
    let mut negatives = 0;
    for row in records {
        let bytes = packet::fresh_artifact(&root, &row["saved"]).unwrap();
        let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert!(crate::xcf_point_replace_source::classify(&original, &photocraft_raster::Interrupt::NONE, None).unwrap().is_some());
        assert!(matches!(
            crate::xcf_point_replace_source::classify(&original, &photocraft_raster::Interrupt::cancel_only(&|| true), None),
            Err(crate::IoError::Cancelled)
        ));
        let limits =
            photocraft_xcf::ResourceLimits::for_policy(photocraft_xcf::ResourcePolicy::NativeStandard).unwrap().lowered(&[("sampleWorkUnits", 0)]).unwrap();
        let budget = photocraft_xcf::Budget::new(limits, None);
        assert!(
            matches!(crate::xcf_point_replace_source::classify(&original, &photocraft_raster::Interrupt::NONE, Some(&budget)), Err(crate::IoError::Xcf(error)) if error.kind == photocraft_xcf::ErrorKind::Limit)
        );
        for mutation in 0..54 {
            let mut bad = original.clone();
            match mutation {
                0 => bad.version = 21,
                1 => bad.width += 1,
                2 => bad.height += 1,
                3 => bad.precision = 600,
                4 => bad.sample = Sample::F64,
                5 => bad.encoding = Encoding::Linear,
                6 => bad.model = Model::Gray,
                7 => bad.icc = vec![1],
                8 => bad.layers[0].width += 1,
                9 => bad.layers[0].height += 1,
                10 => bad.layers[0].offset.0 += 1,
                11 => bad.layers[0].offset.1 += 1,
                12 => bad.layers[0].unsupported.push("unobserved owner semantics".into()),
                13 => bad.layers[0].effects[0].version = Some("future".into()),
                14 => bad.layers[0].effects[0].arguments[0].value = ArgumentValue::Boolean(true),
                15 => bad.layers[0].effects[0].arguments[0].value = ArgumentValue::Float32Bits(f32::NAN.to_bits()),
                16 => bad.layers[0].effects[0].arguments[0].value = ArgumentValue::Float32Bits(f32::INFINITY.to_bits()),
                17 => bad.layers[0].effects[0].arguments.clear(),
                18 => {
                    let arg = bad.layers[0].effects[0].arguments[0].clone();
                    bad.layers[0].effects[0].arguments.push(arg);
                }
                19 => bad.layers[0].effects[0].arguments[0].property_index = 7,
                20 => bad.layers[0].effects[0].properties[0].tag = 999,
                21 => {
                    bad.layers[0].effects[0].properties.pop();
                }
                22 => bad.layers[0].effects[0].properties.swap(0, 1),
                23 => bad.layers[0].effects[0].properties[0].declared_bytes = 3,
                24 => {
                    bad.layers[0].effects[0].properties[0].payload.pop();
                }
                25 => bad.layers[0].effects[0].state.blend_space = Some(0),
                26 => bad.layers[0].effects[0].state.composite_space = Some(0),
                27 => bad.layers[0].effects[0].state.composite_mode = Some(0),
                28 => bad.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
                29 => bad.layers[0].effects[0].state.clipping = Some(1),
                30 => bad.layers[0].effects[0].state.region = Some(1),
                31 => bad.layers[0].effects[0].mask = None,
                32..=48 => {
                    let mask = bad.layers[0].effects[0].mask.as_mut().unwrap();
                    match mutation {
                        32 => mask.offset.0 = 1,
                        33 => mask.width += 1,
                        34 => mask.model = Model::Rgb,
                        35 => mask.alpha = true,
                        36 => mask.path.push(0),
                        37 => mask.text_flags = 1,
                        38 => mask.opacity = 1.0,
                        39 => mask.mask_enabled = false,
                        40 => mask.mask_shown = true,
                        41 => mask.mask_editing = true,
                        42 => mask.legacy_linked = true,
                        43 => mask.mode = 28,
                        44 => mask.unsupported.push("unobserved mask semantics".into()),
                        45 => mask.plane.as_mut().unwrap().sample = Sample::U8,
                        46 => mask.plane.as_mut().unwrap().bytes[0..4].copy_from_slice(&(-0.0f32).to_ne_bytes()),
                        47 => mask.plane.as_mut().unwrap().bytes[0..4].copy_from_slice(&1.0f32.to_ne_bytes()),
                        _ => mask.mask = Some(Box::default()),
                    }
                }
                49 => bad.layers[0].plane.as_mut().unwrap().bytes[0..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
                50 => bad.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&1.001f32.to_ne_bytes()),
                51 => bad.layers[0].text_flags = 1,
                52 => bad.layers[0].group_flags = 1,
                _ => bad.layers[0].effects[0].mask.as_mut().unwrap().group_flags = 1,
            }
            assert!(
                crate::xcf_point_replace_source::classify(&bad, &photocraft_raster::Interrupt::NONE, None).unwrap().is_none(),
                "{}/mutation{mutation}",
                row["id"]
            );
            negatives += 1;
        }
        assert_eq!(packet::fresh_artifact(&root, &row["saved"]).unwrap(), bytes);
    }
    assert_eq!(negatives, 324);
    eprintln!("6 actual source classifiers; 324 isolated DTO hostiles and 6 cancellation refusals");
}

#[test]
#[ignore = "six independently observed TEMP owner-geometry originals"]
fn actual_six_geometry_sources_require_the_observed_family() {
    let (root, records) = packet::fresh_records().unwrap();
    let mut refused = Vec::new();
    for row in records {
        let id = row["id"].as_str().unwrap();
        let bytes = packet::fresh_artifact(&root, &row["saved"]).unwrap();
        let source = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let native = packet::fresh_artifact(&root, &row["current"]["owner"]["native"]).unwrap();
        assert_eq!(source.layers[0].plane.as_ref().unwrap().bytes, native, "{id}");
        if crate::xcf_point_replace_source::classify(&source, &photocraft_raster::Interrupt::NONE, None).unwrap().is_none() {
            eprintln!("ACTUAL CLASSIFIER RED {id}: observed family not admitted");
            refused.push(id.to_owned());
        }
    }
    assert!(refused.is_empty(), "actual original classifiers refused {refused:?}");
}

#[test]
#[ignore = "pre-witness synthetic Red Eye source-domain refusal, not new qualification"]
fn synthetic_geometry_source_dtos_have_no_preincrement_admission() {
    let original = photocraft_xcf::decode(&packet::old_source().unwrap(), &Default::default()).unwrap();
    assert!(crate::xcf_point_replace_source::classify(&original, &photocraft_raster::Interrupt::NONE, None).unwrap().is_some());
    let old = crate::import("old-control.xcf", &packet::old_source().unwrap()).unwrap();
    let node = &old.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.operation.schema_epoch, packet::OLD_EPOCH);
    assert_ne!(node.operation.schema_epoch, packet::NEW_EPOCH);
    assert_eq!(node.composition.as_ref().unwrap().compatibility_epoch, packet::OLD_POLICY);
    for geometry in packet::GEOMETRIES {
        for enabled in [true, false] {
            let synthetic = packet::synthetic_decoded(geometry, enabled).unwrap();
            let result = crate::xcf_point_replace_source::classify(&synthetic, &photocraft_raster::Interrupt::NONE, None).unwrap();
            assert!(result.is_none(), "synthetic OLD source-domain {geometry:?} enabled={enabled} unexpectedly admitted");
            eprintln!(
                "DESIGN REFUSAL {} {:?}/{:?}/{:?} enabled={enabled}: no admitted family; OLD tuple is no NEW evidence",
                geometry.id, geometry.canvas, geometry.owner, geometry.offset
            );
        }
    }
}
