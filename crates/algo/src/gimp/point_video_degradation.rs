//! Coordinate-aware RGB monitor-pattern math from independent public responses.
//! This explicit nonlinear Float RGB candidate does not grant source/profile/ROI admission.
use photocraft_raster::RasterInputError;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoDegradation {
    /// Public pattern choice ordinal, 0 through 8.
    pub pattern: u8,
    pub additive: bool,
    pub rotated: bool,
}
impl VideoDegradation {
    /// Allocation-free evaluation at a caller-supplied pattern coordinate.
    /// Coordinate provenance (world versus drawable local) is a consumer obligation.
    /// All errors leave output untouched; source alpha bits are retained.
    pub fn try_eval_nonlinear_float(&self, x: i32, y: i32, input: &[f32], output: &mut [f32]) -> Result<(), RasterInputError> {
        if input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        if input.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(RasterInputError::InvalidSample);
        }
        let (x, y) = if self.rotated { (i64::from(y), i64::from(x)) } else { (i64::from(x), i64::from(y)) };
        let selected = self.selected_channel(x, y)?;
        let source: [f32; 4] = input.try_into().map_err(|_| RasterInputError::LengthMismatch)?;
        let mut result = source;
        for (channel, value) in result.iter_mut().take(3).enumerate() {
            if self.additive {
                if channel == selected {
                    *value = (*value + *value).min(1.0);
                }
            } else if channel != selected {
                *value = 0.0;
            }
        }
        for (destination, value) in output.iter_mut().zip(result) {
            *destination = value;
        }
        Ok(())
    }
    fn selected_channel(&self, x: i64, y: i64) -> Result<usize, RasterInputError> {
        // These small periodic layouts are the operation's observed pattern definition,
        // not source-image or rendered-pixel lookups. Geometry repeats independently
        // in the 16x16 response and 32x24 holdout for every option combination.
        const STAGGERED: [[usize; 2]; 6] = [[0, 1], [0, 2], [1, 2], [1, 0], [2, 0], [2, 1]];
        const HEX: [[usize; 6]; 6] = [[2, 2, 0, 0, 0, 0], [2, 2, 2, 0, 0, 2], [2, 2, 2, 2, 2, 2], [2, 2, 2, 1, 1, 2], [2, 2, 1, 1, 1, 1], [1, 1, 1, 1, 1, 1]];
        const DOTS: [[usize; 5]; 3] = [[0, 1, 2, 0, 0], [1, 1, 1, 2, 0], [0, 1, 2, 2, 2]];
        let channel = match self.pattern {
            0 | 1 | 4 => {
                let px = if self.pattern == 1 { x.div_euclid(2) } else { x };
                let py = if self.pattern == 1 || self.pattern == 4 { y.div_euclid(2) } else { y };
                STAGGERED.get(py.rem_euclid(6) as usize).and_then(|r| r.get(px.rem_euclid(2) as usize)).copied()
            }
            2 => Some(y.rem_euclid(3) as usize),
            3 => Some(y.div_euclid(2).rem_euclid(3) as usize),
            5 => Some((x - y).rem_euclid(3) as usize),
            6 => Some((x.div_euclid(2) - y.div_euclid(2)).rem_euclid(3) as usize),
            7 => HEX.get(y.rem_euclid(6) as usize).and_then(|r| r.get(x.rem_euclid(6) as usize)).map(|c| (*c + y.div_euclid(6).rem_euclid(3) as usize) % 3),
            8 => DOTS.get(y.rem_euclid(3) as usize).and_then(|r| r.get((x - y.div_euclid(3)).rem_euclid(5) as usize)).copied(),
            _ => return Err(RasterInputError::UnsupportedConversion),
        };
        channel.ok_or(RasterInputError::InvalidSample)
    }
}
