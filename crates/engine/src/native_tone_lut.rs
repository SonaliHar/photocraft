//! Shared exact native interpolation of original typed tone LUT words.
pub(crate) const TABLE_LEN: usize = 4096;
pub(crate) const TABLE_SCALE: i128 = 1i128 << 52;
fn bad(message: impl Into<String>) -> String {
    message.into()
}
pub(crate) fn exact_word(output: f32) -> std::result::Result<u64, String> {
    let exact = f64::from(output) * TABLE_SCALE as f64;
    if !output.is_finite() || !(0.0..=1.0).contains(&output) || exact.fract() != 0.0 {
        return Err(bad("tone LUT word lies outside the exact2^-52 rational unit domain"));
    }
    Ok(exact as u64)
}
fn table_pair(table: &[u64], index: usize) -> std::result::Result<(i128, i128), String> {
    if table.len() != TABLE_LEN {
        return Err(bad("native tone table length"));
    }
    let next = index.checked_add(1).ok_or_else(|| bad("native tone table index"))?.min(TABLE_LEN - 1);
    let lower = *table.get(index).ok_or_else(|| bad("native tone table index"))?;
    let upper = *table.get(next).ok_or_else(|| bad("native tone table index"))?;
    if lower > TABLE_SCALE as u64 || upper > TABLE_SCALE as u64 {
        return Err(bad("native tone referenced word exceeds unit domain"));
    }
    Ok((i128::from(lower), i128::from(upper)))
}
pub(crate) fn interpolate_u32(table: &[u64], before: u32, percent: u8, selection: u32, selection_max: u32) -> std::result::Result<u32, String> {
    if percent > 100 || selection_max == 0 || selection > selection_max {
        return Err(bad("native tone coverage domain"));
    }
    let max = i128::from(u32::MAX);
    let position = i128::from(before) * (TABLE_LEN - 1) as i128;
    let index = usize::try_from(position / max).map_err(|_| bad("native tone table index"))?;
    let remainder = position % max;
    let (lower, upper) = table_pair(table, index)?;
    // Every table word <=2^52; native position <=2^44. The final blend
    // numerator is <2^125 even with maximum U32 coverage and percent 100.
    let mapped = lower * (max - remainder) + upper * remainder;
    let coverage = i128::from(percent) * i128::from(selection);
    let total = 100 * i128::from(selection_max);
    let numerator = i128::from(before) * TABLE_SCALE * (total - coverage) + mapped * coverage;
    let denominator = TABLE_SCALE * total;
    let rounded = (numerator + denominator / 2) / denominator;
    u32::try_from(rounded).map_err(|_| bad("native tone result domain"))
}
pub(crate) fn interpolate_f64(table: &[u64], before: f64, weight: f64) -> std::result::Result<f64, String> {
    if !weight.is_finite() || !(0.0..=1.0).contains(&weight) {
        return Err(bad("native tone requires finite unit coverage"));
    }
    if weight == 0.0 {
        if table.len() != TABLE_LEN {
            return Err(bad("native tone table length"));
        }
        return Ok(before);
    }
    if !before.is_finite() || !(0.0..=1.0).contains(&before) {
        return Err(bad("native tone requires finite unit samples"));
    }
    let position = before * (TABLE_LEN - 1) as f64;
    let index = position.floor() as usize;
    let fraction = position - index as f64;
    let (lower, upper) = table_pair(table, index)?;
    let lower = lower as f64 / TABLE_SCALE as f64;
    let upper = upper as f64 / TABLE_SCALE as f64;
    let left = lower * (1.0 - fraction);
    let right = upper * fraction;
    let mapped = left + right;
    let output = if weight == 1.0 {
        mapped
    } else {
        let original = before * (1.0 - weight);
        let edited = mapped * weight;
        original + edited
    };
    if !output.is_finite() {
        return Err(bad("native tone result is nonfinite"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn referenced_word_and_length_guards_precede_exact_arithmetic() {
        assert!(interpolate_u32(&[0], 0, 100, 1, 1).is_err());
        let mut table = vec![0; TABLE_LEN];
        table[0] = u64::MAX;
        assert!(interpolate_u32(&table, 0, 100, 1, 1).is_err());
        assert!(interpolate_f64(&table, 0.0, 1.0).is_err());
        let negative_zero = -0.0f64;
        assert_eq!(interpolate_f64(&table, negative_zero, 0.0).unwrap().to_bits(), negative_zero.to_bits());
        let nan = f64::from_bits(0x7ff8_0000_0000_0042);
        assert_eq!(interpolate_f64(&table, nan, 0.0).unwrap().to_bits(), nan.to_bits());
    }
}
