//! The three witnessed RGB/F32 Linear singleton source envelopes. Other
//! precisions, encodings, models and owner geometries remain separate domains.
use super::*;

pub(crate) const EPOCH: &str = "xcf-22-stock-unpremultiply-rgb-f32-linear-v1";

pub(crate) fn observed_envelope<P>(effect: &photocraft_xcf::Effect<P>) -> bool {
    let s = &effect.state;
    let tags = [7, 8, 33, 35, 36, 37, 44, 46];
    effect.operation == "gegl:unpremultiply"
        && effect.version.as_deref() == Some("0:0")
        && effect.arguments.is_empty()
        && s.mode == Some(62)
        && s.opacity_bits == Some(1f32.to_bits())
        && s.opacity_u8.is_none()
        && s.enabled.is_some()
        && s.clipping == Some(0)
        && s.region == Some(0)
        && (s.blend_space, s.composite_space, s.composite_mode) == (Some(-1877441967), Some(-1877441967), Some(-1877441967))
        && effect.properties.len() == tags.len()
        && tags.iter().all(|tag| effect.properties.iter().filter(|p| p.tag == *tag && p.declared_bytes == 4 && p.payload.len() == 4).count() == 1)
}

pub(super) fn qualifies(file: &XcfFile, ctl: &Interrupt<'_>, shared: Option<&photocraft_xcf::Budget<'_>>) -> Result<bool, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if file.version != 22
        || file.precision != 600
        || file.model != Model::Rgb
        || file.sample != Sample::F32
        || file.encoding != Encoding::Linear
        || (file.width, file.height) != (8, 6)
        || !file.icc.is_empty()
        || file.layers.len() != 1
        || !file.channels.is_empty()
        || !file.paths.is_empty()
        || !file.palette.is_empty()
    {
        return Ok(false);
    }
    let Some(owner) = file.layers.first() else { return Ok(false) };
    if owner.group
        || owner.path.len() > 1
        || !owner.visible
        || !owner.alpha
        || owner.model != Model::Rgb
        || (owner.width, owner.height, owner.offset) != (8, 6, (0, 0))
        || owner.mode != 28
        || (owner.blend_space, owner.composite_space, owner.composite_mode) != (0, -1, -1)
        || owner.opacity.to_bits() != 1f32.to_bits()
        || owner.mask.is_some()
        || owner.text.is_some()
        || owner.vector.is_some()
        || owner.link.is_some()
        || owner.transform.is_some()
        || owner.floating_attachment.is_some()
        || owner.effects.len() != 1
    {
        return Ok(false);
    }
    let Some(effect) = owner.effects.first() else { return Ok(false) };
    if !observed_envelope(effect) {
        return Ok(false);
    }
    let Some(mask) = &effect.mask else { return Ok(false) };
    if mask.model != Model::Gray
        || mask.alpha
        || mask.group
        || (mask.width, mask.height, mask.offset) != (8, 6, (0, 0))
        || !mask.visible
        || !mask.mask_enabled
        || mask.mask_shown
        || mask.mask_editing
        || mask.legacy_linked
        || (mask.mode, mask.blend_space, mask.composite_space, mask.composite_mode) != (0, 2, 2, 1)
        || !mask.effects.is_empty()
        || !mask.unsupported.is_empty()
        || mask.mask.is_some()
        || mask.text.is_some()
        || mask.vector.is_some()
        || mask.link.is_some()
        || mask.transform.is_some()
        || mask.floating_attachment.is_some()
    {
        return Ok(false);
    }
    let Some(mask_plane) = &mask.plane else { return Ok(false) };
    if (mask_plane.width, mask_plane.height, mask_plane.channels, mask_plane.sample) != (8, 6, 1, Sample::F32)
        || mask_plane.bytes.len() != 192
        || mask_plane.bytes.iter().any(|b| *b != 0)
    {
        return Ok(false);
    }
    let Some(plane) = &owner.plane else { return Ok(false) };
    if (plane.width, plane.height, plane.channels, plane.sample) != (8, 6, 4, Sample::F32) || plane.bytes.len() != 768 {
        return Ok(false);
    }
    if let Some(shared) = shared {
        shared.consume_work(960, "Unpremultiply exact source admission").map_err(accounting_error)?;
    }
    for raw in plane.bytes.as_chunks::<16>().0 {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let channels: &[[u8; 4]; 4] = raw.as_chunks::<4>().0.try_into().map_err(|_| IoError::Unsupported("Unpremultiply native stride".into()))?;
        let rgba = (*channels).map(f32::from_ne_bytes);
        let [r, g, b, a] = rgba;
        if rgba.iter().any(|v| !v.is_finite())
            || !(0.0..=1.0).contains(&a)
            || effect.state.enabled == Some(true) && a != 0.0 && [r, g, b].iter().any(|c| !(*c / a).is_finite())
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(all(test, feature = "corpus"))]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    #[ignore = "independent3 source envelopes with hostile metadata/native/mask/context mutations"]
    fn actual_sources_refuse_unwitnessed_fields_without_substitution() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-unpremultiply-evidence");
        let packet = std::fs::read(root.join("observations.json")).unwrap();
        assert_eq!(
            Sha256::digest(&packet).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "f40f522c4e8b7e7bab1e00b4e2d5fd2729c90062fbc36cb93172f362cd406f3d"
        );
        let report: serde_json::Value = serde_json::from_slice(&packet).unwrap();
        let mut count = 0;
        for row in report["records"].as_array().unwrap().iter().filter(|r| r["model"] == "RGB" && r["precision"] == "FLOAT_LINEAR") {
            let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
            assert_eq!(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), row["sha256"]);
            let source = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert!(qualifies(&source, &Interrupt::NONE, None).unwrap());
            assert!(matches!(qualifies(&source, &Interrupt::cancel_only(&|| true), None), Err(IoError::Cancelled)));
            for mutation in 0..34 {
                let mut bad = source.clone();
                match mutation {
                    0 => bad.version = 23,
                    1 => bad.precision = 650,
                    2 => bad.sample = Sample::F64,
                    3 => bad.encoding = Encoding::Perceptual,
                    4 => bad.model = Model::Gray,
                    5 => bad.icc = vec![0],
                    6 => bad.width = 9,
                    7 => bad.layers[0].offset = (1, 0),
                    8 => bad.layers[0].mode = 30,
                    9 => bad.layers[0].composite_mode = 1,
                    10 => bad.layers[0].visible = false,
                    11 => bad.layers[0].effects[0].version = Some("future".into()),
                    12 => bad.layers[0].effects[0].state.mode = Some(28),
                    13 => bad.layers[0].effects[0].state.blend_space = Some(1574142545),
                    14 => bad.layers[0].effects[0].state.composite_space = Some(1574142545),
                    15 => bad.layers[0].effects[0].state.composite_mode = Some(1574142545),
                    16 => bad.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
                    17 => bad.layers[0].effects[0].state.clipping = Some(1),
                    18 => bad.layers[0].effects[0].state.region = Some(1),
                    19 => bad.layers[0].effects[0].properties[0].tag = 999,
                    20 => bad.layers[0].effects[0].properties[0].declared_bytes = 8,
                    21 => bad.layers[0].effects[0].mask = None,
                    22 => bad.layers[0].effects[0].mask.as_mut().unwrap().mask_shown = true,
                    23 => bad.layers[0].effects[0].mask.as_mut().unwrap().mask_enabled = false,
                    24 => bad.layers[0].effects[0].mask.as_mut().unwrap().offset = (1, 0),
                    25 => bad.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[..4].copy_from_slice(&(-0f32).to_ne_bytes()),
                    26 => bad.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().sample = Sample::F64,
                    27 => bad.layers[0].effects[0].mask.as_mut().unwrap().legacy_linked = true,
                    28 => bad.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
                    29 => bad.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::INFINITY.to_ne_bytes()),
                    30 => bad.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&2f32.to_ne_bytes()),
                    31 => bad.layers[0].plane.as_mut().unwrap().bytes.truncate(767),
                    32 => {
                        bad.layers[0].effects[0].state.enabled = Some(true);
                        bad.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::MAX.to_ne_bytes());
                        bad.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&0.5f32.to_ne_bytes());
                    }
                    _ => bad.layers[0].effects[0].arguments.push(photocraft_xcf::Argument {
                        name: "invented".into(),
                        property_index: 0,
                        value: ArgumentValue::Boolean(true),
                        configuration: None,
                        graph: None,
                    }),
                }
                assert!(!qualifies(&bad, &Interrupt::NONE, None).unwrap(), "{}/mutation{mutation}", row["id"]);
                assert!(super::super::map("hostile Unpremultiply", bad, &Interrupt::NONE, 128 << 20).is_err(), "{}/publication{mutation}", row["id"]);
                count += 1;
            }
            assert_eq!(std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap(), bytes);
        }
        assert_eq!(count, 102);
    }
}
