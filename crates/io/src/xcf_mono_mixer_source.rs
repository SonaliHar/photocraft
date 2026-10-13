//! Exact independently observed expanded Mono Mixer arguments/envelope.
//! Structural, mask, profile and bounded native scans remain caller-owned.
use photocraft_algo::gimp::point_mono_mixer::MonoMixer;
use photocraft_xcf::{ArgumentValue, Effect};

pub(crate) fn parameters<P>(effect: &Effect<P>) -> Option<MonoMixer> {
    if effect.operation != "gegl:mono-mixer" || effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 4 {
        return None;
    }
    let (mut red, mut green, mut blue, mut preserve) = (None, None, None, None);
    for argument in &effect.arguments {
        if argument.configuration.is_some() || argument.graph.is_some() {
            return None;
        }
        match (argument.name.as_str(), &argument.value) {
            ("red", ArgumentValue::Float32Bits(bits)) if red.is_none() => red = Some(f64::from(f32::from_bits(*bits))),
            ("green", ArgumentValue::Float32Bits(bits)) if green.is_none() => green = Some(f64::from(f32::from_bits(*bits))),
            ("blue", ArgumentValue::Float32Bits(bits)) if blue.is_none() => blue = Some(f64::from(f32::from_bits(*bits))),
            ("preserve-luminosity", ArgumentValue::Boolean(value)) if preserve.is_none() => preserve = Some(*value),
            _ => return None,
        }
    }
    let operation = MonoMixer { red: red?, green: green?, blue: blue?, preserve_luminosity: preserve? };
    operation.validate().ok()?;
    Some(operation)
}

pub(crate) fn observed_envelope<P>(effect: &Effect<P>) -> bool {
    let state = &effect.state;
    state.mode == Some(62)
        && state.opacity_bits == Some(1f32.to_bits())
        && state.opacity_u8.is_none()
        && state.clipping == Some(0)
        && state.region == Some(0)
        && state.enabled.is_some()
        && (state.blend_space, state.composite_space, state.composite_mode) == (Some(1799848529), Some(1799848529), Some(1799848529))
        && parameters(effect).is_some()
}

#[cfg(all(test, feature = "corpus"))]
mod tests {
    use super::*;
    #[test]
    #[ignore = "local independently authored expanded Mono source classification controls"]
    fn observed_signed_sources_reject_changed_typed_envelopes_and_cancel() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("mono-mixer-signed-precision-evidence");
        let records: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        let mut rejected = 0;
        for row in records["records"].as_array().unwrap().iter().filter(|r| r["action"] == "source") {
            let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
            let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert!(crate::xcf_point_replace_source::classify(&original, &photocraft_raster::Interrupt::NONE, None).unwrap().is_some());
            assert!(crate::xcf_point_replace_source::classify(&original, &photocraft_raster::Interrupt::cancel_only(&|| true), None).is_err());
            for mutation in 0..10 {
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
                    7 => effect.arguments.iter_mut().find(|a| a.name == "red").unwrap().value = ArgumentValue::Float32Bits(f32::NAN.to_bits()),
                    8 => effect.arguments.iter_mut().find(|a| a.name == "green").unwrap().value = ArgumentValue::Float32Bits(5.125f32.to_bits()),
                    _ => effect.arguments[0].name = effect.arguments[1].name.clone(),
                }
                assert!(
                    crate::xcf_point_replace_source::classify(&bad, &photocraft_raster::Interrupt::NONE, None).unwrap().is_none(),
                    "{}/mutation{mutation}",
                    row["id"]
                );
                rejected += 1;
            }
        }
        assert_eq!(rejected, 360);
    }
}
