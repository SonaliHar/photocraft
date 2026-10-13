#[cfg(test)]
mod tests {
    //! Independent saved Color Overlay envelopes, with hostile near-neighbor controls.
    use photocraft_raster::Interrupt;
    use sha2::{Digest, Sha256};

    fn hash(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn corpus() -> (std::path::PathBuf, serde_json::Value) {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-color-overlay-evidence");
        let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(hash(&bytes), "828c1da757d0706cd6eb502699c74c0c2856745d3603c6ab6d1dd56d70df9831");
        let census: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let descriptor = census["artifacts"].as_array().unwrap().iter().find(|entry| entry["path"] == "observations.json").unwrap();
        let bytes = std::fs::read(root.join("observations.json")).unwrap();
        assert_eq!(hash(&bytes), descriptor["sha256"]);
        (root, serde_json::from_slice(&bytes).unwrap())
    }

    #[test]
    #[ignore = "reviewed unpublished 360 Color Overlay original saved controls"]
    fn color_overlay_source_classifies_all_independent_saved_controls() {
        let (root, observations) = corpus();
        let rows = observations["records"].as_array().unwrap();
        assert_eq!(rows.len(), 360);
        for row in rows {
            let bytes = std::fs::read(root.join(row["source"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&bytes), row["sha256"]);
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let owner = &file.layers[0];
            let effect = &owner.effects[0];
            assert_eq!((file.version, file.width, file.height), (22, 8, 6));
            assert_eq!((owner.width, owner.height, owner.offset), (8, 6, (0, 0)));
            assert_eq!((owner.mode, owner.blend_space, owner.composite_space, owner.composite_mode), (28, 0, -1, -1));
            assert_eq!(effect.operation, "gegl:color-overlay");
            assert_eq!(effect.version.as_deref(), Some("0:0"));
            assert_eq!(
                (effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode),
                (Some(1435337297), Some(1435337297), Some(1435337297))
            );
            assert_eq!((effect.state.mode, effect.state.clipping, effect.state.region), (Some(62), Some(0), Some(0)));
            assert_eq!(effect.state.enabled, row["enabled"].as_bool());
            assert_eq!(effect.arguments.len(), 2);
            assert!(
                matches!(
                    super::super::classify(&file, &Interrupt::NONE, None).unwrap(),
                    Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(crate::xcf_filters::PointReplaceFamily::ColorOverlay))
                ),
                "{}",
                row["id"]
            );
        }
        println!("360 independent Color Overlay sources classified; RGB/Gray, six depths, three transfers, both srgb values, five states");
    }

    #[test]
    #[ignore = "reviewed unpublished Color Overlay source near-neighbor controls"]
    fn color_overlay_source_rejects_altered_context_and_refunds_cancel_work() {
        use photocraft_xcf::{ArgumentValue, Model};
        let (root, observations) = corpus();
        let mut seen = std::collections::BTreeSet::new();
        for row in observations["records"].as_array().unwrap() {
            if row["variant"] != "source" || row["srgb"] != false {
                continue;
            }
            let path = root.join(row["source"].as_str().unwrap());
            let bytes = std::fs::read(&path).unwrap();
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            if !seen.insert((file.precision, format!("{:?}", file.model))) {
                continue;
            }
            assert!(matches!(
                super::super::classify(&file, &Interrupt::NONE, None).unwrap(),
                Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(crate::xcf_filters::PointReplaceFamily::ColorOverlay))
            ));
            for change in 0..25 {
                let mut bad = file.clone();
                let owner = &mut bad.layers[0];
                match change {
                    0 => bad.version = 23,
                    1 => bad.icc = vec![1],
                    2 => bad.model = Model::Indexed,
                    3 => bad.width += 1,
                    4 => owner.offset.0 = 1,
                    5 => owner.mode = 30,
                    6 => owner.opacity = 0.5,
                    7 => owner.blend_space = 2,
                    8 => owner.effects[0].state.blend_space = Some(0),
                    9 => owner.effects[0].state.composite_space = Some(87327313),
                    10 => owner.effects[0].state.composite_mode = Some(-305954223),
                    11 => owner.effects[0].state.mode = Some(28),
                    12 => owner.effects[0].state.clipping = Some(1),
                    13 => owner.effects[0].state.region = Some(1),
                    14 => owner.effects[0].state.enabled = None,
                    15 => owner.effects[0].version = Some("1:0".into()),
                    16 => {
                        let duplicate = owner.effects[0].arguments[0].clone();
                        owner.effects[0].arguments.push(duplicate);
                    }
                    17 => owner.effects[0].arguments[0].name = "unknown".into(),
                    18 => owner.effects[0].arguments[0].value = ArgumentValue::Boolean(false),
                    19 => owner.plane.as_mut().unwrap().width += 1,
                    20 => owner.effects[0].mask.as_mut().unwrap().offset = (1, 0),
                    21 => owner.effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1,
                    22 => owner.effects[0].mask.as_mut().unwrap().mask_shown = true,
                    23 => owner.effects[0].mask = None,
                    _ => {
                        let duplicate = owner.effects[0].clone();
                        owner.effects.push(duplicate);
                    }
                }
                assert!(super::super::classify(&bad, &Interrupt::NONE, None).unwrap().is_none(), "{} change{change}", row["id"]);
            }
            let cancel = || true;
            assert!(matches!(super::super::classify(&file, &Interrupt::cancel_only(&cancel), None), Err(crate::IoError::Cancelled)));
            let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 0)]).unwrap();
            let budget = photocraft_xcf::Budget::new(limits, None);
            assert!(
                matches!(super::super::classify(&file, &Interrupt::NONE, Some(&budget)), Err(crate::IoError::Xcf(error)) if error.kind == photocraft_xcf::ErrorKind::Limit)
            );
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
        assert_eq!(seen.len(), 36);
    }
}
