//! XCF transfer interpretation using ICC primaries and curves without converting source pixels.
use crate::{Builtin, CmsError, ColorSpace, Curve, Profile};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Linear,
    NonlinearProfile,
    Perceptual,
}

/// Effective matrix/TRC interpretation. Original source ICC must be retained separately.
pub fn effective_profile(original: Option<&[u8]>, space: ColorSpace, encoding: Encoding) -> Result<Profile, CmsError> {
    if !matches!(space, ColorSpace::Rgb | ColorSpace::Gray) {
        return Err(CmsError::Unsupported("XCF color model".into()));
    }
    let mut p = match original {
        Some(bytes) => {
            if bytes.len() > 16 << 20 {
                return Err(CmsError::Invalid("XCF ICC exceeds 16 MiB".into()));
            }
            Profile::parse(bytes)?
        }
        None => match space {
            ColorSpace::Gray => Builtin::SGray.profile().clone(),
            _ => Builtin::Srgb.profile().clone(),
        },
    };
    if p.color_space != space {
        return Err(CmsError::Invalid("XCF profile color model mismatch".into()));
    }
    if space == ColorSpace::Rgb && !p.is_matrix_shaper()
        || space == ColorSpace::Gray && p.gray_trc.is_none()
        || p.a2b.iter().any(Option::is_some)
        || p.b2a.iter().any(Option::is_some)
    {
        return Err(CmsError::Unsupported("XCF requires matrix/TRC RGB or TRC gray profile".into()));
    }
    if encoding == Encoding::NonlinearProfile {
        let curves: Vec<&Curve> =
            if space == ColorSpace::Rgb { p.trc.as_ref().map(|t| t.iter().collect()).unwrap_or_default() } else { p.gray_trc.iter().collect() };
        let supported = |c: &Curve| {
            c.is_identity()
                || matches!(c, Curve::Gamma(g) if g.is_finite() && *g > 0.0)
                || (0..=32).all(|i| (c.eval(i as f32 / 32.0) - crate::curve::srgb_trc().eval(i as f32 / 32.0)).abs() < 0.0001)
        };
        if curves.is_empty()
            || curves.iter().any(|c| !supported(c))
            || curves
                .iter()
                .skip(1)
                .any(|c| (0..=32).any(|i| (c.eval(i as f32 / 32.0) - curves.first().map_or(0.0, |f| f.eval(i as f32 / 32.0))).abs() > 0.0001))
        {
            return Err(CmsError::Unsupported("XCF unequal or unsupported profile curves for editable adjustments".into()));
        }
    }
    let curve = match encoding {
        Encoding::Linear => Some(Curve::Identity),
        Encoding::Perceptual => Some(crate::curve::srgb_trc()),
        Encoding::NonlinearProfile => None,
    };
    if let Some(c) = curve {
        if space == ColorSpace::Rgb {
            p.trc = Some([c.clone(), c.clone(), c]);
        } else {
            p.gray_trc = Some(c);
        }
        p.description = format!("{} (XCF {encoding:?} samples)", p.description);
        p = p.with_encoded_bytes();
    }
    Ok(p)
}

/// Same primaries, different working transfer. Alpha is deliberately absent.
pub fn convert_encoding(p: &Profile, from: Encoding, to: Encoding, rgb: [f32; 3]) -> [f32; 3] {
    if from == to {
        return rgb;
    }
    std::array::from_fn(|i| {
        let profile_curve = p.trc.as_ref().and_then(|t| t.get(i)).or(p.gray_trc.as_ref());
        let curve = |encoding| match encoding {
            Encoding::Linear => Curve::Identity,
            Encoding::Perceptual => crate::curve::srgb_trc(),
            Encoding::NonlinearProfile => profile_curve.cloned().unwrap_or_else(crate::curve::srgb_trc),
        };
        curve(to).eval_inverse(curve(from).eval(rgb[i]))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encoding_changes_interpretation_not_source_bytes_or_primaries() {
        let original = Builtin::DisplayP3.profile().to_bytes();
        let linear = effective_profile(Some(&original), ColorSpace::Rgb, Encoding::Linear).unwrap();
        assert_eq!(linear.matrix, Builtin::DisplayP3.profile().matrix);
        assert!(linear.trc.as_ref().unwrap().iter().all(Curve::is_identity));
        assert_eq!(Profile::parse(&original).unwrap(), Profile::parse(&original).unwrap());
        let p = Builtin::Srgb.profile();
        let v = convert_encoding(p, Encoding::NonlinearProfile, Encoding::Linear, [0.5; 3]);
        assert!((v[0] - 0.214041).abs() < 0.00001);
        let back = convert_encoding(p, Encoding::Linear, Encoding::NonlinearProfile, v);
        assert!((back[0] - 0.5).abs() < 0.00001);
        assert!(effective_profile(Some(&original), ColorSpace::Gray, Encoding::Linear).is_err());
        assert!(effective_profile(Some(b"bad"), ColorSpace::Rgb, Encoding::Linear).is_err());
    }
}
