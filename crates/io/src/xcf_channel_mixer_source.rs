//! Exact independently observed Channel Mixer source arguments/envelope.
use photocraft_algo::gimp::point_channel_mixer::ChannelMixer;
use photocraft_xcf::{ArgumentValue, Effect};
pub(crate) fn parameters<P>(effect: &Effect<P>) -> Option<ChannelMixer> {
    if effect.operation != "gegl:channel-mixer" || effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 10 {
        return None;
    }
    let names = [["rr-gain", "rg-gain", "rb-gain"], ["gr-gain", "gg-gain", "gb-gain"], ["br-gain", "bg-gain", "bb-gain"]];
    let mut gains = [[0.; 3]; 3];
    for (row, row_names) in gains.iter_mut().zip(names) {
        for (target, name) in row.iter_mut().zip(row_names) {
            let mut matches = effect.arguments.iter().filter(|a| a.name == name);
            let argument = matches.next()?;
            if matches.next().is_some() || argument.configuration.is_some() || argument.graph.is_some() {
                return None;
            }
            let ArgumentValue::Float32Bits(bits) = argument.value else {
                return None;
            };
            *target = f64::from(f32::from_bits(bits));
        }
    }
    let mut matches = effect.arguments.iter().filter(|a| a.name == "preserve-luminosity");
    let argument = matches.next()?;
    if matches.next().is_some() || argument.configuration.is_some() || argument.graph.is_some() {
        return None;
    }
    let ArgumentValue::Boolean(preserve_luminosity) = argument.value else {
        return None;
    };
    let operation = ChannelMixer { gains, preserve_luminosity };
    operation.try_eval_linear_float(&[0.; 4], &mut [0.; 4]).ok()?;
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
        && (state.blend_space, state.composite_space, state.composite_mode) == (Some(-1150320047), Some(-1150320047), Some(-1150320047))
        && parameters(effect).is_some()
}

#[cfg(all(test, feature = "corpus"))]
mod tests {
    use super::*;
    #[test]
    #[ignore = "independent Channel Mixer source and bounded hostile controls"]
    fn original_channel_sources_reject_changed_schema_work_and_native() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("channel-mixer-precision-evidence");
        let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        let mut rejected = 0;
        let mut positive = 0;
        for row in observations["records"].as_array().unwrap().iter().filter(|r| r["action"] == "source") {
            let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
            let source = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert!(matches!(
                crate::xcf_point_replace_source::classify(&source, &photocraft_raster::Interrupt::NONE, None).unwrap(),
                Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(crate::xcf_filters::PointReplaceFamily::ChannelMixer))
            ));
            assert!(crate::xcf_point_replace_source::classify(&source, &photocraft_raster::Interrupt::cancel_only(&|| true), None).is_err());
            for mutation in 0..16 {
                let mut bad = source.clone();
                let effect = &mut bad.layers[0].effects[0];
                match mutation {
                    0 => effect.version = Some("future".into()),
                    1 => effect.state.blend_space = Some(0),
                    2 => effect.state.composite_space = Some(0),
                    3 => effect.state.composite_mode = Some(0),
                    4 => effect.state.opacity_bits = Some(0.5f32.to_bits()),
                    5 => effect.state.clipping = Some(1),
                    6 => effect.state.region = Some(1),
                    7 => effect.arguments.iter_mut().find(|a| a.name == "rr-gain").unwrap().value = ArgumentValue::Float32Bits(f32::NAN.to_bits()),
                    8 => effect.arguments.iter_mut().find(|a| a.name == "rr-gain").unwrap().value = ArgumentValue::Float32Bits(2.01f32.to_bits()),
                    9 => effect.arguments[0].name = effect.arguments[1].name.clone(),
                    10 => effect.arguments.iter_mut().find(|a| a.name == "preserve-luminosity").unwrap().value = ArgumentValue::Float32Bits(0),
                    11 => effect.arguments.pop().map(|_| ()).unwrap(),
                    12 => bad.width = 9,
                    13 => bad.layers[0].offset = (-1, 0),
                    14 => bad.layers[0].blend_space = 1,
                    _ => bad.icc = vec![1, 2, 3],
                }
                assert!(
                    crate::xcf_point_replace_source::classify(&bad, &photocraft_raster::Interrupt::NONE, None).unwrap().is_none(),
                    "{}/mutation{mutation}",
                    row["id"]
                );
                rejected += 1;
            }
            positive += 1;
        }
        assert_eq!((positive, rejected), (36, 576));
    }
}
