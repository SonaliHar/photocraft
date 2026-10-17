//! Closed native unit blend equations; no profile or encoded-color interpretation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Blend {
    Normal,
    Multiply,
    Screen,
}
pub(crate) fn u32_blend(destination: u32, source: u32, blend: Blend, percent: u8, selection: u32, selection_max: u32) -> Result<u32, &'static str> {
    if percent > 100 || selection_max == 0 || selection > selection_max {
        return Err("invalid native unit blend weight");
    }
    if percent == 0 || selection == 0 {
        return Ok(destination);
    }
    let max = i128::from(u32::MAX);
    let d = i128::from(destination);
    let s = i128::from(source);
    let full = max.checked_mul(max).ok_or("unit blend overflow")?;
    let blended = match blend {
        Blend::Normal => s.checked_mul(max),
        Blend::Multiply => d.checked_mul(s),
        Blend::Screen => max.checked_sub(d).and_then(|a| max.checked_sub(s).and_then(|b| a.checked_mul(b))).and_then(|n| full.checked_sub(n)),
    }
    .ok_or("unit blend overflow")?;
    let delta = blended.checked_sub(d.checked_mul(max).ok_or("unit blend overflow")?).ok_or("unit blend overflow")?;
    let denominator = 100i128.checked_mul(max).and_then(|n| n.checked_mul(i128::from(selection_max))).ok_or("unit blend overflow")?;
    let numerator = d
        .checked_mul(denominator)
        .and_then(|n| delta.checked_mul(i128::from(percent)).and_then(|a| a.checked_mul(i128::from(selection))).and_then(|a| n.checked_add(a)))
        .ok_or("unit blend overflow")?;
    if numerator < 0 {
        return Err("native blend escaped unit range");
    }
    let rounded = numerator.checked_add(denominator / 2).ok_or("unit blend overflow")? / denominator;
    u32::try_from(rounded).map_err(|_| "native blend escaped unit range")
}
pub(crate) fn f64_blend(destination: f64, source: f64, blend: Blend, opacity: f64, selection: f64) -> Result<f64, &'static str> {
    if [destination, source, opacity, selection].iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err("native blend requires finite unit samples");
    }
    let weight = opacity * selection;
    if weight == 0.0 {
        return Ok(destination);
    }
    let blended = match blend {
        Blend::Normal => source,
        Blend::Multiply => destination * source,
        Blend::Screen => {
            let a = 1.0 - destination;
            let b = 1.0 - source;
            let product = a * b;
            1.0 - product
        }
    };
    let delta = blended - destination;
    let weighted = delta * weight;
    let result = destination + weighted;
    if !result.is_finite() || !(0.0..=1.0).contains(&result) {
        return Err("native blend escaped unit range");
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_u32_final_rounding_covers_extrema_fractional_selection_and_identity() {
        for mode in [Blend::Multiply, Blend::Screen] {
            for d in [0, 1, 123_456_789, u32::MAX - 1, u32::MAX] {
                for s in [0, 1, 2_000_000_001, u32::MAX] {
                    let result = u32_blend(d, s, mode, 37, 65_534, 65_535).unwrap();
                    let max = i128::from(u32::MAX);
                    let a = i128::from(d);
                    let b = i128::from(s);
                    let den = 100 * max * 65_535;
                    let blend = if mode == Blend::Multiply { a * b } else { max * max - (max - a) * (max - b) };
                    let expected = (a * den + (blend - a * max) * 37 * 65_534 + den / 2) / den;
                    assert_eq!(i128::from(result), expected);
                    assert_eq!(u32_blend(d, s, mode, 0, 1, 1).unwrap(), d);
                }
            }
        }
        assert_eq!(u32_blend(u32::MAX, u32::MAX, Blend::Multiply, 100, u32::MAX, u32::MAX).unwrap(), u32::MAX);
    }
    #[test]
    fn f64_equations_keep_low_words_and_zero_weight_signed_zero() {
        let d = f64::from_bits(0.5f64.to_bits() + 7);
        let s = f64::from_bits(0.25f64.to_bits() + 3);
        for mode in [Blend::Multiply, Blend::Screen] {
            let blended = if mode == Blend::Multiply { d * s } else { 1.0 - (1.0 - d) * (1.0 - s) };
            assert_eq!(f64_blend(d, s, mode, 0.37, 0.75).unwrap().to_bits(), (d + (blended - d) * (0.37 * 0.75)).to_bits());
        }
        assert_eq!(f64_blend(-0.0, 0.5, Blend::Screen, 0.0, 1.0).unwrap().to_bits(), (-0.0f64).to_bits());
        assert!(f64_blend(f64::NAN, s, Blend::Multiply, 1.0, 1.0).is_err());
    }
}
