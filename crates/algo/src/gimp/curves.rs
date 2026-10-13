//! Checked evaluation of explicitly stored curve sample tables.
//! Smooth point regeneration is deliberately a separate, unqualified operation.
use photocraft_raster::RasterInputError;

/// Evaluation of retained computed samples, independent of editable point geometry.
/// The caller retains and validates the actual stored curve type and points separately;
/// this does not regenerate SMOOTH samples or turn a SMOOTH curve into a FREE curve.
#[derive(Clone, Copy, Debug)]
pub struct StoredCurvesConfiguration<'a> {
    pub tables: [&'a [f64]; 5],
}
impl<'a> StoredCurvesConfiguration<'a> {
    pub fn prepare(self) -> Result<PreparedCurveTables<'a>, RasterInputError> {
        FreeCurvesConfiguration { channels: self.tables.map(|samples| FreeCurve { samples }) }.prepare()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FreeCurve<'a> {
    pub samples: &'a [f64],
}
impl FreeCurve<'_> {
    pub fn validate(self) -> Result<(), RasterInputError> {
        if self.samples.len() != 256 || self.samples.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        Ok(())
    }
    fn eval(self, input: f64) -> Result<f64, RasterInputError> {
        if !input.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
        let position = input.clamp(0.0, 1.0) * 255.0;
        let low = position.floor() as usize;
        let a = self.samples.get(low).copied().ok_or(RasterInputError::LengthMismatch)?;
        let b = self.samples.get(low.saturating_add(1)).copied().unwrap_or(a);
        let fraction = position - low as f64;
        Ok(a + (b - a) * fraction)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FreeCurvesConfiguration<'a> {
    /// Value, red, green, blue, alpha; each is an exact stored 256-sample curve.
    pub channels: [FreeCurve<'a>; 5],
}
impl<'a> FreeCurvesConfiguration<'a> {
    pub fn prepare(self) -> Result<PreparedFreeCurves<'a>, RasterInputError> {
        for curve in self.channels {
            curve.validate()?;
        }
        Ok(PreparedCurveTables { channels: self.channels })
    }
    pub fn try_eval_pixel(self, input: &[f64], color_channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
        self.prepare()?.try_eval_pixel(input, color_channels, output)
    }
}

/// Immutable borrowed tables validated once, without allocation or per-pixel table scans.
#[derive(Clone, Copy, Debug)]
pub struct PreparedCurveTables<'a> {
    channels: [FreeCurve<'a>; 5],
}
/// Compatibility name for the FREE-only checked facade.
pub type PreparedFreeCurves<'a> = PreparedCurveTables<'a>;

/// Fixed-size owned immutable tables for cached renderers. No self-referential borrow
/// or per-pixel validation; the 10,240 payload bytes need caller cache accounting.
#[derive(Clone, Debug)]
pub struct OwnedPreparedCurveTables {
    tables: [[f64; 256]; 5],
}
impl OwnedPreparedCurveTables {
    /// Fixed owned tables, validated once without intermediate heap vectors.
    pub fn try_from_tables(tables: [[f64; 256]; 5]) -> Result<Self, RasterInputError> {
        for samples in &tables { FreeCurve { samples }.validate()?; }
        Ok(Self { tables })
    }

    pub fn try_new(tables: [Vec<f64>; 5]) -> Result<Self, RasterInputError> {
        let [value, red, green, blue, alpha] = tables;
        Ok(Self { tables: [copy_table(&value)?, copy_table(&red)?, copy_table(&green)?, copy_table(&blue)?, copy_table(&alpha)?] })
    }
    pub const fn payload_bytes(&self) -> usize {
        5 * 256 * 8
    }
    pub fn try_eval_pixel(&self, input: &[f64], color_channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
        PreparedCurveTables { channels: self.tables.each_ref().map(|samples| FreeCurve { samples }) }.try_eval_pixel(input, color_channels, output)
    }
}
fn copy_table(source: &[f64]) -> Result<[f64; 256], RasterInputError> {
    FreeCurve { samples: source }.validate()?;
    let mut table = [0.0; 256];
    table.copy_from_slice(source);
    Ok(table)
}
impl PreparedCurveTables<'_> {
    pub fn try_eval_pixel(self, input: &[f64], color_channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
        if !matches!(color_channels, 1 | 3) || input.len() != output.len() || !(color_channels..=color_channels + 1).contains(&input.len()) {
            return Err(RasterInputError::LengthMismatch);
        }
        let mut scratch = [0.0; 4];
        let values = scratch.get_mut(..input.len()).ok_or(RasterInputError::LengthMismatch)?;
        values.copy_from_slice(input);
        let value = self.channels.first().copied().ok_or(RasterInputError::LengthMismatch)?;
        for (index, sample) in values.iter_mut().take(color_channels).enumerate() {
            if color_channels == 3 {
                *sample = self.channels.get(index + 1).copied().ok_or(RasterInputError::LengthMismatch)?.eval(*sample)?;
            }
            *sample = value.eval(*sample)?;
        }
        if let Some(alpha) = values.get_mut(color_channels) {
            *alpha = self.channels.get(4).copied().ok_or(RasterInputError::LengthMismatch)?.eval(*alpha)?;
        }
        output.copy_from_slice(values);
        Ok(())
    }
}
