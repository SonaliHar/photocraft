//! Independently observed finite isolated Normal group source family.
//! The group projection is never substituted for its editable children.
use super::*;

pub(super) const EPOCH: &str = "own-accounted-isolated-normal-group-v1";

pub(super) fn qualifies(file: &XcfFile, ctl: &Interrupt<'_>) -> Result<bool, IoError> {
    qualifies_with(file, ctl, None)
}

pub(super) fn qualifies_with<P: ExactParsedPlane>(
    file: &XcfFile<P>,
    ctl: &Interrupt<'_>,
    budget: Option<&photocraft_xcf::Budget<'_>>,
) -> Result<bool, IoError> {
    // GIMP writes its minimum required XCF version: these authored masked
    // groups are v13 and the otherwise identical no-mask controls are v12.
    let source_version = if file.layers.first().is_some_and(|group| group.mask.is_some()) { 13 } else { 12 };
    if file.version != source_version
        || !matches!(file.model, Model::Rgb | Model::Gray)
        || !matches!((file.precision, file.sample), (250, Sample::U16) | (650, Sample::F32) | (750, Sample::F64))
        || file.encoding != Encoding::NonlinearProfile
        || file.width != 17
        || file.height != 13
        || file.layers.len() != 4
        || !file.channels.is_empty()
        || !file.paths.is_empty()
        || !file.palette.is_empty()
        || !super::staged_accounted_group::profile(file, ctl, budget)?
    {
        return Ok(false);
    }
    let channels = if file.model == Model::Gray { 2 } else { 4 };
    for (index, d) in file.layers.iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if d.model != file.model
            || !d.alpha
            || d.mode != 28
            || (d.blend_space, d.composite_space, d.composite_mode) != (0, 1, 1)
            || !d.effects.is_empty()
            || !d.unsupported.is_empty()
            || d.text.is_some()
            || d.vector.is_some()
            || d.link.is_some()
            || d.transform.is_some()
            || d.floating_attachment.is_some()
            // The parser validates and skips the optional group hierarchy.
            // Only leaf planes are authoritative editable pixel storage.
            || if d.group { d.plane.is_some() } else { !plane_matches_with(d, file.sample, channels, ctl, budget)? }
        {
            return Ok(false);
        }
        let shape = match index {
            0 => d.group && d.path.is_empty() && (d.width, d.height, d.offset) == (16, 12, (-3, -2)) && d.opacity.to_bits() == 0.625f32.to_bits(),
            1 => !d.group && d.path == [0, 0] && (d.width, d.height, d.offset) == (9, 7, (4, 3)) && d.opacity.to_bits() == 0.3125f32.to_bits(),
            2 => !d.group && d.path == [0, 1] && (d.width, d.height, d.offset) == (11, 9, (-3, -2)) && d.opacity.to_bits() == 1f32.to_bits(),
            3 => !d.group && d.path.is_empty() && (d.width, d.height, d.offset) == (17, 13, (0, 0)) && d.opacity.to_bits() == 1f32.to_bits(),
            _ => false,
        };
        if !shape || index != 0 && (!d.visible || d.mask.is_some()) {
            return Ok(false);
        }
        if index == 0 {
            if d.mask_shown || d.mask_editing {
                return Ok(false);
            }
            if let Some(mask) = &d.mask
                && (mask.group
                    || mask.model != Model::Gray
                    || mask.alpha
                    || (mask.width, mask.height, mask.offset) != (16, 12, (0, 0))
                    || !mask.unsupported.is_empty()
                    || !mask.effects.is_empty()
                    || mask.mask.is_some()
                    || mask.text.is_some()
                    || mask.vector.is_some()
                    || mask.link.is_some()
                    || mask.transform.is_some()
                    || mask.floating_attachment.is_some()
                    || !plane_matches_with(mask, file.sample, 1, ctl, budget)?)
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

// The exact old plane shape/unit predicate, using bounded native reads for both
// eager and staged storage. No conversion or group projection is substituted.
fn plane_matches_with<P: ExactParsedPlane>(
    d: &Drawable<P>,
    sample: Sample,
    channels: usize,
    ctl: &Interrupt<'_>,
    budget: Option<&photocraft_xcf::Budget<'_>>,
) -> Result<bool, IoError> {
    let Some(plane) = d.plane.as_ref() else { return Ok(false) };
    let bytes = if sample == Sample::F16 { 4 } else { sample.source_bytes() };
    let expected = u64::from(d.width).checked_mul(u64::from(d.height)).and_then(|n| n.checked_mul(channels as u64)).and_then(|n| n.checked_mul(bytes as u64));
    let desc = plane.descriptor();
    if desc.sample != if sample == Sample::F16 { Sample::F32 } else { sample }
        || desc.channels != channels
        || desc.width != d.width
        || desc.height != d.height
        || plane.native_byte_len() != expected
    {
        return Ok(false);
    }
    let mut valid = true;
    plane.visit_native(ctl, &mut |block| {
        if let Some(budget) = budget {
            super::staged_accounted_group::pay(budget, block.len() as u64)?;
        }
        for raw in block.chunks_exact(bytes) {
            if matches!(sample, Sample::U8 | Sample::U16 | Sample::U32) {
                continue;
            }
            let value = if sample == Sample::F64 {
                f64::from_ne_bytes(raw.try_into().map_err(|_| IoError::Unsupported("group native F64 word".into()))?)
            } else {
                f64::from(f32::from_ne_bytes(raw.try_into().map_err(|_| IoError::Unsupported("group native F32 word".into()))?))
            };
            valid &= value.is_finite() && (0.0..=1.0).contains(&value) && (sample != Sample::F16 || exact_half_unit_f32(value as f32));
        }
        Ok(())
    })?;
    Ok(valid)
}

#[cfg(all(test, feature = "corpus"))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires reviewed unpublished full XCF corpus"]
    fn accounted_group_epoch_refuses_unobserved_source_contexts() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("full corpus path"));
        let bytes = std::fs::read(root.join("accounted-normal-group-evidence/rgb-f32-visible-mask.xcf")).unwrap();
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert!(qualifies(&file, &Interrupt::NONE).unwrap());
        for change in 0..19 {
            let mut invalid = file.clone();
            match change {
                0 => invalid.version = 23,
                1 => invalid.encoding = Encoding::Linear,
                2 => invalid.precision = 675,
                3 => invalid.icc.clear(),
                4 => invalid.layers[0].mode = 61,
                5 => invalid.layers[0].composite_mode = 2,
                6 => invalid.layers[1].offset = (5, 3),
                7 => invalid.layers[1].path = vec![0, 1],
                8 => invalid.layers[0].opacity = f32::NAN,
                9 => invalid.layers[1].visible = false,
                10 => invalid.layers[0].mask_shown = true,
                11 => invalid.layers[0].mask_editing = true,
                12 => invalid.layers[0].mask.as_mut().unwrap().offset = (1, 0),
                13 => invalid.layers[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
                14 => invalid.layers[1].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&2f32.to_ne_bytes()),
                15 => invalid.layers[3].opacity = 0.5,
                16 => invalid.layers[2].group = true,
                17 => invalid.version = 12,
                _ => invalid.layers[0].mask = None,
            }
            assert!(!qualifies(&invalid, &Interrupt::NONE).unwrap(), "change {change}");
        }
        let mut renamed = file.clone();
        renamed.layers[0].name = "Renamed observed group".into();
        assert!(qualifies(&renamed, &Interrupt::NONE).unwrap(), "names confer no admission authority");
        let cancelled = || true;
        assert!(matches!(qualifies(&file, &Interrupt::cancel_only(&cancelled)), Err(IoError::Cancelled)));
        assert_eq!(std::fs::read(root.join("accounted-normal-group-evidence/rgb-f32-visible-mask.xcf")).unwrap(), bytes);
    }
}
