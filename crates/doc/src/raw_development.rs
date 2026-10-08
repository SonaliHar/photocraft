//! Persisted sensor-development settings, separate from pixel Camera Raw filters.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawDevelopment {
    pub version: u32,
    pub exposure: f64,
    pub white_balance: RawWhiteBalance,
    pub demosaic: RawDemosaic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum RawWhiteBalance {
    #[default]
    AsShot,
    Auto,
    Multipliers {
        rgb: [f64; 3],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RawDemosaic {
    Bilinear,
    Mhc,
    #[default]
    Ahd,
}

impl Default for RawDevelopment {
    fn default() -> Self {
        Self { version: 1, exposure: 0.0, white_balance: RawWhiteBalance::default(), demosaic: RawDemosaic::default() }
    }
}

impl RawDevelopment {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("unsupported RAW development version".into());
        }
        if !self.exposure.is_finite() || !(-10.0..=10.0).contains(&self.exposure) {
            return Err("RAW exposure must be finite and between -10 and 10 EV".into());
        }
        if let RawWhiteBalance::Multipliers { rgb } = &self.white_balance
            && rgb.iter().any(|v| !v.is_finite() || !(0.01..=100.0).contains(v))
        {
            return Err("RAW white-balance multipliers must be finite and between 0.01 and 100".into());
        }
        Ok(())
    }
}
