#[cfg(test)]
mod tests {
    //! Source-only hostile DTO controls derived from immutable independently authored source bytes.
    use crate::xcf_filters::{NeighborhoodDomain, PointReplaceFamily};
    use photocraft_raster::Interrupt;
    use photocraft_xcf::{ArgumentValue, XcfFile};
    fn source() -> XcfFile {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
        let bytes = std::fs::read(root.join("point-alpha-clip-value-domain-nonlinear-controls/rgb-float_non_linear.unit.ordinary.xcf")).unwrap();
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
        let obs: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("point-alpha-clip-value-domain-nonlinear-controls/observations.json")).unwrap()).unwrap();
        let row = obs["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb-float_non_linear.unit.ordinary").unwrap();
        assert_eq!(hash, row["sha256"]);
        photocraft_xcf::decode(&bytes, &Default::default()).unwrap()
    }
    fn set(file: &mut XcfFile, name: &str, value: ArgumentValue) {
        file.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == name).unwrap().value = value;
    }
    fn finite(file: &XcfFile) -> bool {
        crate::xcf_point_replace_source::classify(file, &Interrupt::NONE, None).unwrap()
            == Some(NeighborhoodDomain::PointReplace(PointReplaceFamily::AlphaClipFinite))
    }
    #[test]
    #[ignore = "requires reviewed unpublished full XCF corpus"]
    fn finite_source_refuses_nonunit_output_before_import_and_retains_noop_source_bits() {
        let original = source();
        assert!(finite(&original));
        let source_bytes = original.layers[0].plane.as_ref().unwrap().bytes.clone();
        for (name, threshold, other) in [("low-limit", 1.125f32, "clip-high"), ("high-limit", -0.125f32, "clip-low"), ("low-limit", f32::MAX, "clip-high")] {
            let mut changed = original.clone();
            set(&mut changed, name, ArgumentValue::Float32Bits(threshold.to_bits()));
            set(&mut changed, other, ArgumentValue::Boolean(false));
            assert!(!finite(&changed), "computed nonunit {name} {threshold}");
            changed.layers[0].effects[0].state.enabled = Some(false);
            assert!(finite(&changed), "disabled finite identity {name} {threshold}");
            assert_eq!(changed.layers[0].plane.as_ref().unwrap().bytes, source_bytes);
            changed.layers[0].effects[0].state.enabled = Some(true);
            set(&mut changed, "clip-low", ArgumentValue::Boolean(false));
            set(&mut changed, "clip-high", ArgumentValue::Boolean(false));
            assert!(finite(&changed), "neither finite identity {name} {threshold}");
            assert_eq!(changed.layers[0].plane.as_ref().unwrap().bytes, source_bytes);
        }
    }
    #[test]
    #[ignore = "requires reviewed unpublished full XCF corpus"]
    fn finite_source_nonfinite_unit_alpha_and_tuple_context_are_not_normalized() {
        let original = source();
        for alpha in [-0.125f32, 1.125, f32::MAX, f32::NAN, f32::INFINITY] {
            let mut changed = original.clone();
            changed.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&alpha.to_ne_bytes());
            assert!(crate::xcf_point_replace_source::classify(&changed, &Interrupt::NONE, None).unwrap().is_none());
        }
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut changed = original.clone();
            set(&mut changed, "low-limit", ArgumentValue::Float32Bits(v.to_bits()));
            changed.layers[0].effects[0].state.enabled = Some(false);
            assert!(!finite(&changed));
        }
        for field in 0..3 {
            let mut changed = original.clone();
            let state = &mut changed.layers[0].effects[0].state;
            match field {
                0 => state.blend_space = Some(-1710980527),
                1 => state.composite_space = Some(-1710980527),
                _ => state.composite_mode = Some(-1710980527),
            }
            assert!(crate::xcf_point_replace_source::classify(&changed, &Interrupt::NONE, None).unwrap().is_none());
        }
        assert_eq!(
            (
                original.layers[0].effects[0].state.blend_space,
                original.layers[0].effects[0].state.composite_space,
                original.layers[0].effects[0].state.composite_mode
            ),
            (Some(-305954223), Some(-305954223), Some(-305954223))
        );
        assert!(crate::xcf_point_replace_source::classify(&original, &Interrupt::cancel_only(&|| true), None).is_err());
    }
    #[test]
    #[ignore = "requires reviewed unpublished full XCF corpus"]
    fn finite_source_typed_metadata_and_zero_work_are_closed_before_publication() {
        let original = source();
        for mutation in 0..8 {
            let mut changed = original.clone();
            let effect = &mut changed.layers[0].effects[0];
            match mutation {
                0 => effect.version = None,
                1 => effect.version = Some("0:1".into()),
                2 => {
                    effect.arguments.push(effect.arguments[0].clone());
                }
                3 => effect.arguments[1].name = "clip-low".into(),
                4 => effect.arguments[1].value = ArgumentValue::Float64Bits(0.25f64.to_bits()),
                5 => effect.arguments[0].value = ArgumentValue::Integer(1),
                6 => effect.arguments[0].configuration = Some(photocraft_xcf::EffectConfiguration { nodes: vec![], typed: None }),
                _ => effect.arguments[0].graph = Some(photocraft_xcf::StoredGraph { entries: vec![], nodes: 0, edges: 0 }),
            }
            effect.state.enabled = Some(false);
            assert!(!finite(&changed), "metadata mutation {mutation}");
        }
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 0)]).unwrap();
        let budget = photocraft_xcf::Budget::new(limits, None);
        assert!(
            matches!(crate::xcf_point_replace_source::classify(&original, &Interrupt::NONE, Some(&budget)), Err(crate::IoError::Xcf(error)) if error.kind == photocraft_xcf::ErrorKind::Limit)
        );
    }
}
