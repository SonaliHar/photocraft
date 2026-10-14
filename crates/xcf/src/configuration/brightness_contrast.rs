//! Exact stock GimpConfig storage, not GEGL brightness/contrast mathematics.
use super::{fail, field, scalar};
use crate::{BrightnessContrastConfiguration, ConfigNode, TypedConfiguration, XcfError, reader::Reader};

fn assign(slot: &mut Option<u64>, bits: u64) -> bool {
    if slot.is_some() {
        false
    } else {
        *slot = Some(bits);
        true
    }
}
fn valid(time: u64, brightness: u64, contrast: u64) -> bool {
    f64::from_bits(time).is_finite()
        && [brightness, contrast].into_iter().all(|v| {
            let n = f64::from_bits(v);
            n.is_finite() && (-1.0..=1.0).contains(&n)
        })
}
pub(super) fn typed(r: &mut Reader<'_, '_>, nodes: &[ConfigNode]) -> Result<TypedConfiguration, XcfError> {
    if nodes.len() != 3 {
        return Err(fail(r));
    }
    let (mut time, mut brightness, mut contrast) = (None, None, None);
    for node in nodes {
        r.poll()?;
        let (name, args) = field(node).ok_or_else(|| fail(r))?;
        let bits = scalar(r, args)?;
        let slot = match name {
            "time" => &mut time,
            "brightness" => &mut brightness,
            "contrast" => &mut contrast,
            _ => return Err(fail(r)),
        };
        if !assign(slot, bits) {
            return Err(fail(r));
        }
    }
    let (Some(time_bits), Some(brightness_bits), Some(contrast_bits)) = (time, brightness, contrast) else {
        return Err(fail(r));
    };
    if !valid(time_bits, brightness_bits, contrast_bits) {
        return Err(fail(r));
    }
    r.charge(std::mem::size_of::<BrightnessContrastConfiguration>())?;
    r.poll()?;
    Ok(TypedConfiguration::BrightnessContrast(BrightnessContrastConfiguration { time_bits: Some(time_bits), brightness_bits, contrast_bits }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_bits_are_not_defaulted_or_rewritten() {
        let bits = (-0f64).to_bits();
        let mut slot = None;
        assert!(assign(&mut slot, bits));
        assert_eq!(slot, Some(bits));
        assert!(!assign(&mut slot, 0));
        assert!(valid(0, bits, 0.125f64.to_bits()));
    }
    #[test]
    fn hostile_fields_refuse_instead_of_clamping() {
        assert!(!valid(0, 1.0000001f64.to_bits(), 0));
        assert!(!valid(0, 0, f64::INFINITY.to_bits()));
        assert!(!valid(f64::NAN.to_bits(), 0, 0));
    }
}
