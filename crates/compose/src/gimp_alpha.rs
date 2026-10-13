//! Deterministic public alpha arithmetic; source alpha already includes opacity.
//! Merge conserves summed premultiplied color when coverage sums to <=1;
//! Split subtracts coverage while preserving backdrop color. No Photoshop blend changes.
use super::{CompositionError, GimpBlendMode, Result};
pub(crate) fn replace(mode: GimpBlendMode, b: [f64; 4], s: [f64; 4]) -> Result<[f64; 4]> {
    if b.iter().chain(s.iter()).any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&b[3]) || !(0.0..=1.0).contains(&s[3]) {
        return Err(CompositionError::Invalid("invalid alpha composition input"));
    }
    if s[3] == 0.0 {
        return Ok(b);
    }
    let (alpha, back_weight, source_weight) = match mode {
        GimpBlendMode::Erase => (b[3] * (1.0 - s[3]), b[3] * (1.0 - s[3]), 0.0),
        GimpBlendMode::Merge => ((b[3] + s[3]).min(1.0), b[3].min(1.0 - s[3]), s[3]),
        GimpBlendMode::Split => ((b[3] - s[3]).max(0.0), 0.0, 0.0),
        _ => return Err(CompositionError::Unsupported("not a deterministic alpha mode")),
    };
    if alpha == 0.0 {
        return Ok([0.0; 4]);
    }
    if matches!(mode, GimpBlendMode::Erase | GimpBlendMode::Split) {
        return Ok([b[0], b[1], b[2], alpha]);
    }
    let mut out = [0.0; 4];
    for (i, v) in out.iter_mut().take(3).enumerate() {
        *v = (b[i] * back_weight + s[i] * source_weight) / alpha;
    }
    out[3] = alpha;
    if out.iter().any(|v| !v.is_finite()) {
        return Err(CompositionError::Invalid("alpha composition arithmetic overflow"));
    }
    Ok(out)
}
