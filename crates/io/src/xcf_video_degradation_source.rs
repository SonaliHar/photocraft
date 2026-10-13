//! Exact saved 16x16 nonlinear Float Video source/edit/disabled envelope.
use photocraft_algo::gimp::point_video_degradation::VideoDegradation;
use photocraft_xcf::{ArgumentValue, Effect};
pub(crate) fn parameters<P>(effect: &Effect<P>) -> Option<VideoDegradation> {
    if effect.operation != "gegl:video-degradation" || effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 3 {
        return None;
    }
    let (mut pattern, mut additive, mut rotated) = (None, None, None);
    for argument in &effect.arguments {
        if argument.configuration.is_some() || argument.graph.is_some() {
            return None;
        }
        match (argument.name.as_str(), &argument.value) {
            ("pattern", ArgumentValue::Enum(value)) if pattern.is_none() => pattern = Some(u8::try_from(*value).ok()?),
            ("additive", ArgumentValue::Boolean(value)) if additive.is_none() => additive = Some(*value),
            ("rotated", ArgumentValue::Boolean(value)) if rotated.is_none() => rotated = Some(*value),
            _ => return None,
        }
    }
    let operation = VideoDegradation { pattern: pattern?, additive: additive?, rotated: rotated? };
    photocraft_algo::gimp::registry::video_degradation_member(effect.version.as_deref(), operation).ok()?;
    Some(operation)
}
pub(crate) fn observed_envelope<P>(effect: &Effect<P>) -> bool {
    let Some(operation) = parameters(effect) else {
        return false;
    };
    let source = operation == (VideoDegradation { pattern: 5, additive: false, rotated: false });
    let edited = operation == (VideoDegradation { pattern: 2, additive: true, rotated: true });
    let state = &effect.state;
    (source && state.enabled.is_some() || edited && state.enabled == Some(true))
        && state.mode == Some(62)
        && state.opacity_bits == Some(1f32.to_bits())
        && state.opacity_u8.is_none()
        && state.region == Some(0)
        && state.clipping == Some(0)
        && (state.blend_space, state.composite_space, state.composite_mode) == (Some(-1877441967), Some(-1877441967), Some(-1877441967))
}
pub(crate) fn observed_epoch<P>(effect: &Effect<P>) -> Option<&'static str> {
    if observed_envelope(effect) {
        return Some(photocraft_compose::gimp_video_degradation::VIDEO_DEGRADATION_EPOCH);
    }
    parameters(effect)?;
    // The new packet carries exactly eight state fields and these three
    // typed argument properties. Unknown or repeated raw fields do not gain
    // this epoch's authority; all original payload bytes remain untouched.
    let layout = [(8, 4), (33, 4), (7, 4), (37, 4), (36, 4), (35, 4), (46, 4), (44, 4), (45, 20), (45, 21), (45, 20)];
    if effect.properties.len() != layout.len()
        || effect.properties.iter().zip(layout).any(|(property, (tag, width))| {
            property.tag != tag || property.declared_bytes != width || property.payload.len() != width as usize
        })
        || effect.arguments.iter().any(|argument| {
            let index = match argument.name.as_str() {
                "pattern" => 8,
                "additive" => 9,
                "rotated" => 10,
                _ => return true,
            };
            argument.property_index != index
        })
    {
        return None;
    }
    let state = &effect.state;
    (state.enabled.is_some()
        && state.mode == Some(62)
        && state.opacity_bits == Some(1f32.to_bits())
        && state.opacity_u8.is_none()
        && state.region == Some(0)
        && state.clipping == Some(0)
        && (state.blend_space, state.composite_space, state.composite_mode) == (Some(2026996305), Some(2026996305), Some(2026996305)))
        .then_some(photocraft_compose::gimp_video_degradation::VIDEO_DEGRADATION_OPTIONS_EPOCH)
}
#[cfg(all(test, feature = "corpus"))]
#[path = "../tests/common/video_options_source_predicate.rs"]
mod options_tests;
#[cfg(all(test, feature = "corpus"))]
mod tests {
    #[test]
    #[ignore = "independent saved Video source and closed hostile source predicate"]
    fn saved_sources_and_mutations_are_closed() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/video-degradation-edit-evidence");
        for name in ["source", "edit", "disabled"] {
            let file = photocraft_xcf::decode(&std::fs::read(root.join(format!("{name}.xcf"))).unwrap(), &Default::default()).unwrap();
            assert!(matches!(
                crate::xcf_point_replace_source::classify(&file, &photocraft_raster::Interrupt::NONE, None).unwrap(),
                Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(crate::xcf_filters::PointReplaceFamily::VideoDegradation))
            ));
            assert!(crate::xcf_point_replace_source::classify(&file, &photocraft_raster::Interrupt::cancel_only(&|| true), None).is_err());
            for mutation in 0..18 {
                let mut bad = file.clone();
                match mutation {
                    0 => bad.version = 21,
                    1 => bad.precision = 675,
                    2 => bad.encoding = photocraft_xcf::Encoding::Linear,
                    3 => bad.model = photocraft_xcf::Model::Gray,
                    4 => bad.width = 17,
                    5 => bad.layers[0].offset = (2, 1),
                    6 => bad.layers[0].effects[0].version = Some("future".into()),
                    7 => bad.layers[0].effects[0].state.enabled = None,
                    8 => bad.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
                    9 => bad.layers[0].effects[0].state.blend_space = Some(0),
                    10 => bad.layers[0].effects[0].mask = None,
                    11 => bad.layers[0].effects[0].arguments.clear(),
                    12 => {
                        let duplicate = bad.layers[0].effects[0].arguments[0].clone();
                        bad.layers[0].effects[0].arguments[1] = duplicate;
                    }
                    13 => bad.layers[0].effects[0].arguments[0].name = "unknown".into(),
                    14 => bad.layers[0].effects[0].state.region = Some(1),
                    15 => bad.layers[0].group = true,
                    16 => bad.layers[0].visible = false,
                    17 => bad.layers[0].effects[0].state.clipping = Some(1),
                    _ => panic!("unexpected synthetic mutation ordinal"),
                }
                assert!(
                    crate::xcf_point_replace_source::classify(&bad, &photocraft_raster::Interrupt::NONE, None).unwrap().is_none(),
                    "{name} mutation{mutation}"
                );
            }
        }
    }
}
