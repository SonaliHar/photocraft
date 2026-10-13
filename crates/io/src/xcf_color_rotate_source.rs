//! Exact observed Perceptual Color Rotate wire, without allocations or reads.
use photocraft_algo::gimp::point_color_rotate::{ColorRotate, GrayMode, HueArc};
use photocraft_xcf::{ArgumentValue, Effect};
pub(crate) fn parameters<P>(effect: &Effect<P>) -> Option<ColorRotate> {
    if effect.operation != "gegl:color-rotate" || effect.version.as_deref() != Some("0:0") {
        return None;
    }
    if effect.arguments.len() != 10 {
        return None;
    }
    let mut result = ColorRotate {
        source: HueArc { from_degrees: 0., to_degrees: 90., clockwise: false },
        destination: HueArc { from_degrees: 0., to_degrees: 90., clockwise: false },
        gray_threshold: 0.,
        gray_mode: GrayMode::TreatAs,
        gray_hue_degrees: 0.,
        gray_saturation: 0.,
    };
    let number = |value: &ArgumentValue| -> Option<f32> {
        match value {
            ArgumentValue::Float32Bits(bits) => Some(f32::from_bits(*bits)),
            _ => None,
        }
    };
    let boolean = |value: &ArgumentValue| -> Option<bool> {
        match value {
            ArgumentValue::Boolean(value) => Some(*value),
            _ => None,
        }
    };
    let mut seen = 0u16;
    for argument in &effect.arguments {
        if argument.configuration.is_some() || argument.graph.is_some() {
            return None;
        }
        let bit = match argument.name.as_str() {
            "src-clockwise" => 1,
            "src-from" => 2,
            "src-to" => 4,
            "dest-clockwise" => 8,
            "dest-from" => 16,
            "dest-to" => 32,
            "threshold" => 64,
            "gray-mode" => 128,
            "hue" => 256,
            "saturation" => 512,
            _ => return None,
        };
        if seen & bit != 0 {
            return None;
        }
        seen |= bit;
        match argument.name.as_str() {
            "src-clockwise" => result.source.clockwise = boolean(&argument.value)?,
            "src-from" => result.source.from_degrees = number(&argument.value)?,
            "src-to" => result.source.to_degrees = number(&argument.value)?,
            "dest-clockwise" => result.destination.clockwise = boolean(&argument.value)?,
            "dest-from" => result.destination.from_degrees = number(&argument.value)?,
            "dest-to" => result.destination.to_degrees = number(&argument.value)?,
            "threshold" => result.gray_threshold = number(&argument.value)?,
            "gray-mode" => {
                result.gray_mode = match &argument.value {
                    ArgumentValue::Enum(0) => GrayMode::TreatAs,
                    ArgumentValue::Enum(1) => GrayMode::ChangeTo,
                    _ => return None,
                }
            }
            "hue" => result.gray_hue_degrees = number(&argument.value)?,
            "saturation" => result.gray_saturation = number(&argument.value)?,
            _ => return None,
        }
    }
    result.validate().ok()?;
    photocraft_compose::gimp_color_rotate::witnessed_source_parameters(result).then_some(result)
}
pub(crate) fn observed_envelope<P>(effect: &Effect<P>) -> bool {
    let Some(parameters) = parameters(effect) else { return false };
    let Some(raw) = photocraft_compose::gimp_color_rotate::witnessed_source_triplet(parameters) else { return false };
    let s = &effect.state;
    s.mode == Some(62)
        && s.opacity_bits == Some(1f32.to_bits())
        && s.opacity_u8.is_none()
        && s.enabled.is_some()
        && s.clipping == Some(0)
        && s.region == Some(0)
        && (s.blend_space, s.composite_space, s.composite_mode) == (Some(raw), Some(raw), Some(raw))
}

#[cfg(all(test, feature = "corpus"))]
mod tests {
    use super::*;
    #[test]
    #[ignore = "frozen original Color Rotate source and hostile admission"]
    fn original_six_and_altered_source_controls() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/color-rotate-perceptual-evidence");
        let rows: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        let mut count = 0;
        for row in rows["records"].as_array().unwrap() {
            let original = photocraft_xcf::decode(&std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap(), &Default::default()).unwrap();
            assert!(matches!(
                crate::xcf_point_replace_source::classify(&original, &photocraft_raster::Interrupt::NONE, None).unwrap(),
                Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(crate::xcf_filters::PointReplaceFamily::ColorRotate))
            ));
            assert!(crate::xcf_point_replace_source::classify(&original, &photocraft_raster::Interrupt::cancel_only(&|| true), None).is_err());
            for mutation in 0..21 {
                let mut bad = original.clone();
                let e = &mut bad.layers[0].effects[0];
                match mutation {
                    0 => e.version = Some("future".into()),
                    1 => e.state.blend_space = Some(0),
                    2 => e.state.composite_space = Some(0),
                    3 => e.state.composite_mode = Some(0),
                    4 => e.arguments[0].value = ArgumentValue::Boolean(true),
                    5 => e.arguments[1].value = ArgumentValue::Float32Bits(200f32.to_bits()),
                    6 => e.arguments[1].value = ArgumentValue::Float32Bits(f32::NAN.to_bits()),
                    7 => e.arguments[7].value = ArgumentValue::Enum(2),
                    8 => e.arguments.push(e.arguments[0].clone()),
                    9 => e.state.opacity_bits = Some(0.5f32.to_bits()),
                    10 => e.state.clipping = Some(1),
                    11 => bad.layers[0].offset = (1, 0),
                    12 => bad.precision = 650,
                    13 => bad.encoding = photocraft_xcf::Encoding::NonlinearProfile,
                    14 => bad.icc = vec![1],
                    15 => e.arguments[6].value = ArgumentValue::Float32Bits((-0.1f32).to_bits()),
                    16 => e.arguments[1].value = ArgumentValue::Float32Bits(16f32.to_bits()),
                    17 => e.arguments[2].value = ArgumentValue::Float32Bits(121f32.to_bits()),
                    18 => e.arguments[6].value = ArgumentValue::Float32Bits(0.25f32.to_bits()),
                    19 => e.arguments[7].value = ArgumentValue::Enum(1),
                    _ => e.arguments[8].value = ArgumentValue::Float32Bits(46f32.to_bits()),
                }
                assert!(
                    crate::xcf_point_replace_source::classify(&bad, &photocraft_raster::Interrupt::NONE, None).unwrap().is_none(),
                    "{}/mutation{mutation}",
                    row["id"]
                );
                count += 1;
            }
        }
        assert_eq!(count, 126);
    }
}
