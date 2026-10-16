//! Sealed mixed-root Exposure source adapter. No old singleton policy is reused.
use photocraft_xcf::{ArgumentValue, Effect};
/// Retain exact authored F32 words; public gdouble metadata grants no F64 storage.
pub(crate) fn parameters<P>(effect: &Effect<P>) -> Option<[u32; 2]> {
    if effect.operation != "gegl:exposure" || effect.version.as_deref() != Some("1:0") || effect.arguments.len() != 2 {
        return None;
    }
    let mut exposure = None;
    let mut black = None;
    for argument in &effect.arguments {
        if argument.configuration.is_some() || argument.graph.is_some() {
            return None;
        }
        let ArgumentValue::Float32Bits(bits) = &argument.value else {
            return None;
        };
        let slot = match argument.name.as_str() {
            "exposure" => &mut exposure,
            "black-level" => &mut black,
            _ => return None,
        };
        if slot.replace(*bits).is_some() {
            return None;
        }
    }
    let words = [exposure?, black?];
    // Existing scalar validation checks finite stops/exp2 and the promoted f64
    // black domain exactly. In particular stored F32 +/-0.1 is not normalized.
    photocraft_compose::ExposureParameters::from_words(words[0], words[1]).ok()?;
    Some(words)
}
/// Declared bounded full-canvas-origin source geometry policy, independently
/// observed at8x6 and130x65. Actual owner/canvas equality and complete native,
/// physical mask/default/profile validation remain separate mandatory proofs.
pub(crate) fn source_geometry(width: u32, height: u32) -> bool {
    width != 0 && height != 0 && width <= 262144 && height <= 262144 && u64::from(width).checked_mul(u64::from(height)).is_some_and(|pixels| pixels <= 1 << 30)
}
