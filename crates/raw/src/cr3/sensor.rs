//! File-derived Bayer geometry, levels and white balance; no camera matrix table.
use super::{CrxTrack, Metadata, adaptive, craw, lossless, metadata, sensor_track, u16_at};
use crate::tiff::{Tiff, tag};
use crate::{BlackLevels, Cfa, Limits, RawError, RawFormat, Rect, Sensor};

fn rectangle(bytes: &[u8], at: usize, width: usize, height: usize, trailing_rows: usize) -> Result<Rect, RawError> {
    let read = |offset| u16_at(bytes, offset).map(usize::from).ok_or_else(|| RawError::malformed("missing CR3 IAD1 image borders"));
    let (x, y, right, bottom) = (read(at)?, read(at + 2)?, read(at + 4)?, read(at + 6)?);
    if x > right || y > bottom || right >= width || y >= height || bottom >= height.saturating_add(trailing_rows) {
        return Err(RawError::malformed("CR3 image borders outside sensor"));
    }
    Ok(Rect::new(x, y, right - x + 1, bottom.min(height - 1) - y + 1))
}
fn geometry(track: &CrxTrack<'_>) -> Result<(Rect, Rect), RawError> {
    let b = track.dimensions;
    if b.get(..4) != Some(&[0; 4])
        || u16_at(b, 4).map(u32::from) != Some(track.width)
        || u16_at(b, 6).map(u32::from) != Some(track.height)
        || u16_at(b, 8) != Some(1)
        || u16_at(b, 10) != Some(2)
        || u16_at(b, 12) != Some(1)
        || u16_at(b, 14) != Some(0)
    {
        return Err(RawError::unsupported("CR3 IAD1 geometry variant"));
    }
    let crop = rectangle(b, 16, track.width as usize, track.height as usize, 0)?;
    // The R5 adaptive APS-C original advertises four trailing masked rows
    // absent from its stored raster. Bound only this v0x200 readout padding;
    // the photographic crop and horizontal bounds remain strictly validated.
    let active = rectangle(b, 40, track.width as usize, track.height as usize, if track.version == 0x200 { 7 } else { 0 })?;
    if crop.intersect(&active) != crop {
        return Err(RawError::malformed("CR3 default crop outside active sensor"));
    }
    Ok((active, crop))
}
struct Levels {
    wb: [f64; 3],
    black: Vec<f32>,
    white: f32,
}
fn bayer_black(colors: &[u8], levels: &[f32]) -> Result<Vec<f32>, RawError> {
    let red_row = colors.iter().position(|v| *v == 0).map(|p| p / 2).ok_or_else(|| RawError::malformed("missing red CFA site"))?;
    colors
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let at = match c {
                0 => 0,
                2 => 3,
                _ => {
                    if i / 2 == red_row {
                        1
                    } else {
                        2
                    }
                }
            };
            levels.get(at).copied().ok_or_else(|| RawError::malformed("missing CR3 black level"))
        })
        .collect()
}
fn levels(meta: &Metadata<'_>, bits: u8) -> Option<Levels> {
    let t = meta.timed_maker.or(meta.blocks.get(2).copied().flatten()).and_then(Tiff::new)?;
    let ifd = t.ifd_at(t.first_ifd, 0)?;
    let e = ifd.get(0x4001)?;
    if e.typ != 3 {
        return None;
    }
    let v = t.uints(e);
    // Public ExifTool ColorData9–12 word offsets. Version 66 was measured
    // on the CC0 R6 Mark III sample to retain version 64's layout.
    let (wb, black, white) = match *v.first()? {
        16..=19 => (71, 329, 797),
        32 | 33 => (85, 343, 811),
        34 | 48 => (105, 363, 641),
        64..=66 => (105, 383, 661),
        _ => return None,
    };
    let l = v.get(wb..wb + 4)?;
    let [r, g1, g2, b] = [l.first()?, l.get(1)?, l.get(2)?, l.get(3)?].map(|v| f64::from(*v));
    let g = (g1 + g2) / 2.;
    if !(256.0..16384.).contains(&g) || (g1 - g2).abs() > g * 0.05 || !(0.25..8.).contains(&(r / g)) || !(0.25..8.).contains(&(b / g)) {
        return None;
    }
    let full = (1u32.checked_shl(u32::from(bits))?) - 1;
    let white = *v.get(white)?;
    let black = v.get(black..black + 4)?;
    if white == 0 || white > full || black.iter().any(|b| *b >= white) {
        return None;
    }
    // ColorData orders RGGB, whereas a Bayer pattern can be shifted.
    Some(Levels { wb: [r / g, 1., b / g], black: black.iter().map(|v| *v as f32).collect(), white: white as f32 })
}
pub(crate) fn decode(raw: &[u8], limits: &Limits) -> Result<Sensor, RawError> {
    let track = sensor_track(raw, limits)?;
    if track.encoding != 0 || !matches!((track.version, track.wavelet_levels), (0x100, 0 | 3) | (0x200, 3)) {
        return Err(RawError::unsupported("CRX coding variant (lossless, legacy or adaptive three-level C-RAW Bayer is decoded)"));
    }
    let (active, crop) = geometry(&track)?;
    let data = if track.wavelet_levels == 0 {
        lossless::samples(&track, limits)?
    } else if track.version == 0x200 {
        adaptive::samples(&track, limits)?
    } else {
        craw::samples(&track, limits)?
    };
    let meta = metadata(raw);
    let mut warnings = Vec::new();
    let colors = match track.cfa_layout {
        0 => vec![0, 1, 1, 2],
        1 => vec![1, 0, 2, 1],
        2 => vec![1, 2, 0, 1],
        3 => vec![2, 1, 1, 0],
        _ => return Err(RawError::unsupported("CR3 CFA layout")),
    };
    let (black, white, camera_wb) = if let Some(l) = levels(&meta, track.bits) {
        let values = bayer_black(&colors, &l.black)?;
        (BlackLevels { rows: 2, cols: 2, values, delta_h: Vec::new(), delta_v: Vec::new() }, l.white, Some(l.wb))
    } else {
        warnings.push("CR3 ColorData layout or levels unavailable; estimated masked black and clipping levels, automatic white balance".into());
        let black = crate::cr2::masked_black(&data, track.width as usize, Rect::new(0, active.y, active.x.min(128), active.height))
            .map_or_else(|| BlackLevels::uniform(0.), |v| BlackLevels { rows: 2, cols: 2, values: v.to_vec(), delta_h: Vec::new(), delta_v: Vec::new() });
        (black, crate::cr2::clip_level(&data, u32::from(track.bits)), None)
    };
    let root = meta.blocks.first().copied().flatten().and_then(Tiff::new);
    let ifd = root.and_then(|t| t.ifd_at(t.first_ifd, 0));
    let model = root.and_then(|t| ifd.as_ref().and_then(|i| t.tag_ascii(i, tag::MODEL)));
    let color = model.as_deref().and_then(super::calibration::camera).unwrap_or_default();
    if !color.calibrations.is_empty() {
        warnings.push("CR3 camera colour uses a model-specific RGB response calibration (single illuminant; no camera picture-style look)".into());
    }
    Ok(Sensor {
        format: RawFormat::Cr3,
        make: root.and_then(|t| ifd.as_ref().and_then(|i| t.tag_ascii(i, tag::MAKE))),
        model,
        width: track.width as usize,
        height: track.height as usize,
        samples: 1,
        data,
        cfa: Some(Cfa { width: 2, height: 2, colors, origin_x: 0, origin_y: 0 }),
        linearization: None,
        black: crate::cr2::realign_black(black, active),
        white: [white; 3],
        active,
        crop,
        color,
        camera_wb,
        orientation: super::orientation(raw).unwrap_or(1),
        baseline_exposure: 0.,
        gain_maps: Vec::new(),
        tone_curve: Vec::new(),
        black_point: 0.0,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn black_levels_follow_the_red_and_blue_rows_in_every_cfa_phase() {
        let levels = [100., 200., 300., 400.];
        for (cfa, want) in [
            ([0, 1, 1, 2], [100., 200., 300., 400.]),
            ([1, 0, 2, 1], [200., 100., 400., 300.]),
            ([1, 2, 0, 1], [300., 400., 100., 200.]),
            ([2, 1, 1, 0], [400., 300., 200., 100.]),
        ] {
            assert_eq!(bayer_black(&cfa, &levels).unwrap(), want);
        }
        assert!(bayer_black(&[1, 1, 1, 1], &levels).is_err());
        assert!(bayer_black(&[0, 1, 1, 2], &[]).is_err());
    }
}
