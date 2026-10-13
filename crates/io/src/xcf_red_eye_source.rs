//! Exact saved Red Eye schema; source pixels, profile and empty mask are caller-owned.
use photocraft_xcf::{ArgumentValue, Effect};
#[cfg(all(test, feature = "corpus"))]
#[path = "../tests/common/red_eye_geometry_source_predicate.rs"]
mod geometry_source_tests;
pub(crate) fn threshold<P>(effect: &Effect<P>) -> Option<f64> {
    if effect.operation != "gegl:red-eye-removal" || effect.version.as_deref() != Some("0:0") {
        return None;
    }
    let [argument] = effect.arguments.as_slice() else { return None };
    if argument.name != "threshold" || argument.configuration.is_some() || argument.graph.is_some() {
        return None;
    }
    let ArgumentValue::Float32Bits(bits) = argument.value else { return None };
    let value = f64::from(f32::from_bits(bits));
    (value.is_finite() && (0.0..=0.8).contains(&value)).then_some(value)
}
pub(crate) fn observed_envelope<P>(effect: &Effect<P>) -> bool {
    let state = &effect.state;
    threshold(effect).is_some()
        && state.mode == Some(62)
        && state.opacity_bits == Some(1f32.to_bits())
        && state.opacity_u8.is_none()
        && state.enabled.is_some()
        && state.clipping == Some(0)
        && state.region == Some(0)
        && (state.blend_space, state.composite_space, state.composite_mode) == (Some(127500881), Some(127500881), Some(127500881))
}
pub(crate) fn observed_epoch<P>(effect: &Effect<P>) -> Option<&'static str> {
    if observed_envelope(effect) {
        return Some(photocraft_compose::gimp_red_eye::RED_EYE_PRECISION_EPOCH);
    }
    threshold(effect)?;
    let layout = [(8, 4), (33, 4), (7, 4), (37, 4), (36, 4), (35, 4), (46, 4), (44, 4), (45, 22)];
    if effect.properties.len() != layout.len()
        || effect.properties.iter().zip(layout).any(|(property, (tag, bytes))| {
            property.tag != tag || property.declared_bytes != bytes || property.payload.len() != bytes as usize
        })
        || effect.arguments.first().is_none_or(|argument| argument.property_index != 8)
    {
        return None;
    }
    let state = &effect.state;
    (state.mode == Some(62) && state.opacity_bits == Some(1f32.to_bits()) && state.opacity_u8.is_none() && state.enabled.is_some()
        && state.clipping == Some(0) && state.region == Some(0)
        && (state.blend_space, state.composite_space, state.composite_mode) == (Some(-487751087), Some(-487751087), Some(-487751087)))
        .then_some(photocraft_compose::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH)
}

#[cfg(all(test, feature = "corpus"))]
mod tests {
    use super::*;
    #[test]
    #[ignore = "independent local108 Red Eye originals and altered-source admission controls"]
    fn original_red_eye_source_and_altered_fields_are_bounded() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("red-eye-removal-precision-evidence");
        let rows: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        let mut positive = 0;
        let mut negatives = 0;
        for row in rows["records"].as_array().unwrap().iter().filter(|r| r["action"] == "source") {
            let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
            let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert!(matches!(
                crate::xcf_point_replace_source::classify(&original, &photocraft_raster::Interrupt::NONE, None).unwrap(),
                Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(crate::xcf_filters::PointReplaceFamily::RedEye))
            ));
            assert!(crate::xcf_point_replace_source::classify(&original, &photocraft_raster::Interrupt::cancel_only(&|| true), None).is_err());
            for mutation in 0..12 {
                let mut bad = original.clone();
                let effect = &mut bad.layers[0].effects[0];
                match mutation {
                    0 => effect.version = Some("future".into()),
                    1 => effect.state.blend_space = Some(0),
                    2 => effect.state.composite_space = Some(0),
                    3 => effect.state.composite_mode = Some(0),
                    4 => effect.state.opacity_bits = Some(0.5f32.to_bits()),
                    5 => effect.state.clipping = Some(1),
                    6 => effect.state.region = Some(1),
                    7 => effect.arguments[0].value = ArgumentValue::Float32Bits(f32::NAN.to_bits()),
                    8 => effect.arguments[0].value = ArgumentValue::Float32Bits(0.8f32.to_bits()),
                    9 => effect.arguments[0].name = "unknown".into(),
                    10 => effect.arguments.push(effect.arguments[0].clone()),
                    _ => bad.layers[0].offset = (1, 0),
                }
                assert!(
                    crate::xcf_point_replace_source::classify(&bad, &photocraft_raster::Interrupt::NONE, None).unwrap().is_none(),
                    "{}/mutation{mutation}",
                    row["id"]
                );
                negatives += 1;
            }
            positive += 1;
        }
        assert_eq!((positive, negatives), (36, 432));
    }
}
