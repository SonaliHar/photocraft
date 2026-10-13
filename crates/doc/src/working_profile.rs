//! Current working-profile interpretation, separate from immutable import provenance.
use crate::{ColorMode, XcfValidationError};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum WorkingProfileInterpretation {
    #[default]
    Icc,
    ImplicitSrgb {
        icc_blake3: String,
    },
    ImplicitSGray {
        icc_blake3: String,
    },
}
impl WorkingProfileInterpretation {
    pub fn implicit_srgb(icc: &[u8]) -> Self {
        Self::ImplicitSrgb { icc_blake3: blake3::hash(icc).to_hex().to_string() }
    }
    pub fn implicit_sgray(icc: &[u8]) -> Self {
        Self::ImplicitSGray { icc_blake3: blake3::hash(icc).to_hex().to_string() }
    }
    /// Checks identity/model binding only. CMS additionally verifies the implicit profile definition.
    /// No marker is inferred from ICC bytes or immutable source provenance.
    pub fn validate_with(&self, icc: Option<&[u8]>, mode: ColorMode) -> Result<(), XcfValidationError> {
        self.validate_with_interrupt(icc, mode, &photocraft_raster::Interrupt::NONE)
    }
    pub fn validate_with_interrupt(&self, icc: Option<&[u8]>, mode: ColorMode, control: &photocraft_raster::Interrupt<'_>) -> Result<(), XcfValidationError> {
        let (hash, model_valid) = match self {
            Self::Icc => return Ok(()),
            Self::ImplicitSrgb { icc_blake3 } => (icc_blake3, matches!(mode, ColorMode::Rgb | ColorMode::Indexed)),
            Self::ImplicitSGray { icc_blake3 } => (icc_blake3, mode == ColorMode::Grayscale),
        };
        let icc = icc.ok_or_else(|| XcfValidationError::new("working_profile_identity", "implicit working profile requires current ICC bytes"))?;
        if !model_valid
            || icc.is_empty()
            || icc.len() > 64 << 20
            || hash.len() != 64
            || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            || crate::xcf_items::checked_payload_hash(icc, control)?.to_hex().as_str() != hash
        {
            return Err(XcfValidationError::new("working_profile_identity", "stale or incompatible implicit working profile identity"));
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn implicit_working_marker_is_bound_to_current_profile_and_model_only() {
        let bytes = b"own current identity, not an ICC semantic admission";
        let marker = WorkingProfileInterpretation::implicit_srgb(bytes);
        marker.validate_with(Some(bytes), ColorMode::Rgb).unwrap();
        marker.validate_with(Some(bytes), ColorMode::Indexed).unwrap();
        assert!(marker.validate_with(Some(b"changed"), ColorMode::Rgb).is_err());
        assert!(marker.validate_with(None, ColorMode::Rgb).is_err());
        assert!(marker.validate_with(Some(bytes), ColorMode::Grayscale).is_err());
        let gray = WorkingProfileInterpretation::implicit_sgray(bytes);
        gray.validate_with(Some(bytes), ColorMode::Grayscale).unwrap();
        assert!(gray.validate_with(Some(bytes), ColorMode::Rgb).is_err());
        WorkingProfileInterpretation::Icc.validate_with(None, ColorMode::Cmyk).unwrap();
        assert!(WorkingProfileInterpretation::implicit_srgb(&[]).validate_with(Some(&[]), ColorMode::Rgb).is_err());
    }
    #[test]
    fn unknown_working_interpretation_or_required_fields_do_not_silently_default() {
        assert!(serde_json::from_str::<WorkingProfileInterpretation>(r#"{"UnknownTransfer":{}}"#).is_err());
        assert!(
            serde_json::from_str::<WorkingProfileInterpretation>(
                r#"{"ImplicitSrgb":{"icc_blake3":"0000000000000000000000000000000000000000000000000000000000000000","required_transfer":"unknown"}}"#
            )
            .is_err()
        );
    }
}
