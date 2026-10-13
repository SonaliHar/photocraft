//! Saved zero-argument Linear Absolute envelope; current samples are caller-owned.
pub(crate) fn observed_envelope<P>(effect: &photocraft_xcf::Effect<P>) -> bool {
    let s = &effect.state;
    effect.operation == "gegl:absolute"
        && effect.version.as_deref() == Some("0:0")
        && effect.arguments.is_empty()
        && s.mode == Some(62)
        && s.opacity_bits == Some(1f32.to_bits())
        && s.opacity_u8.is_none()
        && s.enabled.is_some()
        && s.clipping == Some(0)
        && s.region == Some(0)
        && (s.blend_space, s.composite_space, s.composite_mode) == (Some(1574142545), Some(1574142545), Some(1574142545))
}

#[cfg(all(test, feature = "corpus"))]
mod tests {
    #[test]
    #[ignore = "independent108 original source states, with36 Linear candidates and altered-source refusals"]
    fn original_absolute_source_precision_fields_and_cancel_are_bounded() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-absolute-evidence");
        let report: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        let mut counts = (0, 0, 0);
        for row in report["records"].as_array().unwrap() {
            let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
            let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let result = crate::xcf_point_replace_source::classify(&original, &photocraft_raster::Interrupt::NONE, None).unwrap();
            if original.encoding != photocraft_xcf::Encoding::Linear {
                assert!(result.is_none());
                counts.1 += 1;
                continue;
            }
            assert!(matches!(result, Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(crate::xcf_filters::PointReplaceFamily::AbsoluteLinear))));
            assert!(crate::xcf_point_replace_source::classify(&original, &photocraft_raster::Interrupt::cancel_only(&|| true), None).is_err());
            for mutation in 0..11 {
                let mut bad = original.clone();
                match mutation {
                    0 => bad.layers[0].effects[0].version = Some("future".into()),
                    1 => bad.layers[0].effects[0].state.blend_space = Some(0),
                    2 => bad.layers[0].effects[0].state.composite_space = Some(0),
                    3 => bad.layers[0].effects[0].state.composite_mode = Some(0),
                    4 => bad.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
                    5 => bad.layers[0].effects[0].state.region = Some(1),
                    6 => bad.layers[0].effects[0].state.clipping = Some(1),
                    7 => bad.layers[0].offset = (1, 0),
                    8 => bad.layers[0].mode = 30,
                    9 => bad.icc = vec![0],
                    _ => bad.layers[0].effects[0].arguments.push(photocraft_xcf::Argument {
                        name: "invented".into(),
                        property_index: 0,
                        value: photocraft_xcf::ArgumentValue::Boolean(true),
                        configuration: None,
                        graph: None,
                    }),
                }
                assert!(
                    crate::xcf_point_replace_source::classify(&bad, &photocraft_raster::Interrupt::NONE, None).unwrap().is_none(),
                    "{}/mutation{mutation}",
                    row["id"]
                );
                counts.2 += 1;
            }
            counts.0 += 1;
        }
        assert_eq!(counts, (36, 72, 396));
    }
}
