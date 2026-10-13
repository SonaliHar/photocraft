//! Editable source state and source-cache relationships; regeneration is an explicit command.
mod remap;

use crate::item_identity::{ItemRef, PathId, ResourceId};
use serde::{Deserialize, Serialize};
pub(crate) fn checked_payload_hash(bytes: &[u8], control: &photocraft_raster::Interrupt<'_>) -> Result<blake3::Hash, crate::XcfValidationError> {
    let mut hash = blake3::Hasher::new();
    if control.cancelled() {
        return Err(crate::XcfValidationError::new("cancelled", "payload validation cancelled"));
    }
    for chunk in bytes.chunks(32 << 10) {
        if control.cancelled() {
            return Err(crate::XcfValidationError::new("cancelled", "payload validation cancelled"));
        }
        hash.update(chunk);
    }
    Ok(hash.finalize())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCache {
    /// Immutable owned cache bytes; cloning retains the original allocation.
    pub bytes: std::sync::Arc<Vec<u8>>,
    pub blake3: String,
    pub width: u32,
    pub height: u32,
    pub format: crate::PixelFormat,
}
impl SourceCache {
    /// Exact uncompressed cache length, bounded before any native blob allocation.
    pub fn checked_byte_len(&self) -> Result<usize, crate::XcfValidationError> {
        Self::checked_descriptor_len(self.width, self.height, self.format)
    }
    pub fn checked_descriptor_len(width: u32, height: u32, format: crate::PixelFormat) -> Result<usize, crate::XcfValidationError> {
        let invalid = || crate::XcfValidationError::new("cache_bounds", "invalid source cache extent or byte count");
        let count = usize::try_from(width)
            .ok()
            .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
            .and_then(|n| n.checked_mul(format.bytes_per_pixel()))
            .ok_or_else(invalid)?;
        if width == 0 || height == 0 || width > 262144 || height > 262144 || count == 0 || count > 64 << 20 {
            return Err(invalid());
        }
        Ok(count)
    }
    pub fn validate(&self) -> Result<(), crate::XcfValidationError> {
        self.validate_with(&photocraft_raster::Interrupt::NONE)
    }
    /// Cache samples remain raw, including nonfinite payloads and signed zero.
    pub fn validate_with(&self, control: &photocraft_raster::Interrupt<'_>) -> Result<(), crate::XcfValidationError> {
        if self.checked_byte_len()? != self.bytes.len() {
            return Err(crate::XcfValidationError::new("cache_bounds", "invalid source cache byte count"));
        }
        if self.blake3.len() != 64 || !self.blake3.bytes().all(|v| v.is_ascii_hexdigit()) {
            return Err(crate::XcfValidationError::new("payload_hash", "invalid source cache hash"));
        }
        if crate::source_cache_hash::checked_digest(&self.bytes, control)?.to_hex().as_str() != self.blake3 {
            return Err(crate::XcfValidationError::new("payload_hash", "source cache hash mismatch"));
        }
        control.check().map_err(|_| crate::XcfValidationError::new("cancelled", "source cache validation cancelled"))?;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransformState {
    pub matrix_f64_bits: [u64; 9],
    pub interpolation: i32,
    pub pre_offset: [i32; 2],
}
/// Operative linked geometry, separate from the immutable imported transform/cache.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkedCurrentState {
    pub transform: Option<TransformState>,
    pub bounds: crate::Rect,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditableSource {
    Text {
        content: String,
        markup: Option<String>,
        font: ResourceId,
        direction: i32,
        source_modified: bool,
        configuration: Vec<crate::xcf_filters::FilterArgument>,
        /// PhotoCraft's current placement; absent uses the retained drawable origin.
        /// This does not change the original GIMP AST, transform or raster cache.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_origin: Option<[i32; 2]>,
    },
    Vector {
        path: PathId,
        source_modified: bool,
        configuration: Vec<crate::xcf_filters::FilterArgument>,
    },
    Linked {
        resource: ResourceId,
        last_verified_hash: Option<String>,
        #[serde(default)]
        configuration: Vec<crate::xcf_filters::FilterArgument>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current: Option<LinkedCurrentState>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceItem {
    pub owner: ItemRef,
    pub source: EditableSource,
    pub transform: Option<TransformState>,
    pub cache: Option<SourceCache>,
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    fn cache() -> SourceCache {
        let bytes = [0x7ff8_0000_0000_0042_u64, (-0.0_f64).to_bits(), (0.5 + 2.0_f64.powi(-40)).to_bits(), 1e-100_f64.to_bits()]
            .into_iter()
            .flat_map(u64::to_ne_bytes)
            .collect::<Vec<_>>();
        SourceCache {
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            bytes: bytes.into(),
            width: 4,
            height: 1,
            format: crate::PixelFormat::new(crate::ColorMode::Grayscale, crate::SampleType::F64, false),
        }
    }
    #[test]
    fn source_cache_preserves_nonfinite_signed_zero_and_low_bits_and_checks_hash() {
        let original = cache();
        original.validate().unwrap();
        let restored: SourceCache = serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert_eq!(restored, original);
        let mut corrupt = original.clone();
        std::sync::Arc::make_mut(&mut corrupt.bytes)[0] ^= 1;
        assert!(corrupt.validate().is_err());
        corrupt = original.clone();
        corrupt.width = u32::MAX;
        assert!(corrupt.validate().is_err());
        corrupt = original;
        std::sync::Arc::make_mut(&mut corrupt.bytes).pop();
        assert!(corrupt.validate().is_err());
    }
    #[test]
    fn source_cache_hashing_is_cancellable_and_descriptor_caps_before_allocation() {
        assert!(cache().validate_with(&photocraft_raster::Interrupt::cancel_only(&|| true)).is_err());
        assert!(SourceCache::checked_descriptor_len(262144, 262144, crate::PixelFormat::RGBA64F).is_err());
        assert!(SourceCache::checked_descriptor_len(0, 1, crate::PixelFormat::RGBA8).is_err());
    }
}
