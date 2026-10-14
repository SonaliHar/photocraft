//! Model colour transforms independently fitted from controlled RGB response probes.
//! No proprietary profiles, external matrix tables or decoder implementation are copied.
//! These single-illuminant estimates preserve neutral white; see data/calibration/README.md.
use crate::{Calibration, ColorInfo, color};
const DATA: &str = include_str!("../../data/calibration/canon.csv");
pub(super) fn camera(model: &str) -> Option<ColorInfo> {
    let line = DATA.lines().filter(|v| !v.starts_with('#')).find(|v| v.split(',').next().is_some_and(|name| name.eq_ignore_ascii_case(model.trim())))?;
    let mut values = line.split(',').skip(1);
    let mut numbers = [0.0; 12];
    for number in &mut numbers {
        *number = values.next()?.parse::<f64>().ok()?;
    }
    if values.next().is_some() || numbers.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let wb = [numbers[0], numbers[1], numbers[2]];
    if wb.iter().any(|v| *v <= 0.0) {
        return None;
    }
    let forward = [[numbers[3], numbers[4], numbers[5]], [numbers[6], numbers[7], numbers[8]], [numbers[9], numbers[10], numbers[11]]];
    let white = color::apply(&forward, [1.0; 3]);
    if white.iter().zip(color::D50).any(|(a, b)| (a - b).abs() > 1e-3) {
        return None;
    }
    let inverse = color::invert(&forward)?;
    let color_matrix = std::array::from_fn(|r| inverse[r].map(|v| v / wb[r]));
    Some(ColorInfo {
        calibrations: vec![Calibration { temperature: 5003.0, color_matrix, forward_matrix: Some(forward), camera_calibration: color::IDENTITY }],
        ..Default::default()
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_measured_model_has_a_finite_invertible_neutral_preserving_transform() {
        for row in DATA.lines().filter(|v| !v.starts_with('#') && !v.is_empty()) {
            let mut fields = row.split(',');
            let name = fields.next().unwrap();
            let wb: Vec<_> = fields.take(3).map(|v| v.parse::<f64>().unwrap()).collect();
            let profile = camera(name).unwrap();
            let matrix = profile.balanced_to_xyz_d50([1.0 / wb[0], 1.0 / wb[1], 1.0 / wb[2]]).unwrap();
            assert!(matrix.iter().flatten().all(|v| v.is_finite()));
            assert!(color::invert(&matrix).is_some());
            let white = color::apply(&matrix, [1.0; 3]);
            assert!(white.iter().zip(color::D50).all(|(a, b)| (a - b).abs() < 1e-3));
            assert_eq!(camera(&name.to_uppercase()), Some(profile));
        }
        assert!(camera("Canon EOS R unknown").is_none());
        assert!(camera("Nikon EOS R").is_none());
    }
    #[test]
    fn eos_r_camera_channels_are_not_treated_as_srgb_primaries() {
        let c = camera("Canon EOS R").unwrap();
        let matrix = c.balanced_to_xyz_d50([1.0; 3]).unwrap();
        let generic = color::srgb_to_xyz_d50();
        assert!((matrix[0][0] - generic[0][0]).abs() > 0.1);
        assert!((matrix[2][1] - generic[2][1]).abs() > 0.1);
    }
}
