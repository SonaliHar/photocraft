//! Opt-in exact stored RGBA-float Color to Alpha binding, not import admission.
//! The enclosing owner retains/charges ALL decoded effect, mask and raw-property
//! backing. A zero ICC framing length is preserved, not interpreted as absent
//! color-space authority or sRGB. Default xcf_filters mapping remains unchanged.
use crate::IoError;
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Argument, ArgumentValue, Effect, RawProperty};
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub output_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { output_bytes: 4096, work_units: 4096 }
    }
}
pub struct StoredColor<'a, P> {
    effect: &'a Effect<P>,
    color: &'a Argument,
    property: &'a RawProperty,
    components: [u32; 4],
    thresholds: [u32; 2],
}
impl<P> StoredColor<'_, P> {
    pub fn original_effect(&self) -> &Effect<P> {
        self.effect
    }
    pub fn original_argument(&self) -> &Argument {
        self.color
    }
    pub fn original_property(&self) -> &RawProperty {
        self.property
    }
    pub fn component_bits(&self) -> [u32; 4] {
        self.components
    }
    pub fn threshold_bits(&self) -> [u32; 2] {
        self.thresholds
    }
    /// Canonical Linear bits only; profile=None means no canonical resource binding,
    /// never proof that the serialized literal has no interpreted profile.
    pub fn canonical_arguments(
        &self,
        limits: Limits,
        c: &Interrupt<'_>,
        reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    ) -> Result<Vec<FilterArgument>, IoError> {
        check_limits(limits)?;
        poll(c)?;
        let minimum = (3 * std::mem::size_of::<FilterArgument>() + 4 * std::mem::size_of::<ExactNumber>() + 64) as u64;
        if minimum > limits.output_bytes {
            return Err(bad("prospective output capacity"));
        }
        reserve(limits.output_bytes, limits.work_units)?;
        let mut used = 0u64;
        let mut args = Vec::new();
        args.try_reserve_exact(3).map_err(|_| bad("argument allocation"))?;
        debit(&mut used, (args.capacity() * std::mem::size_of::<FilterArgument>()) as u64, limits)?;
        let mut components = Vec::new();
        components.try_reserve_exact(4).map_err(|_| bad("component allocation"))?;
        debit(&mut used, (components.capacity() * std::mem::size_of::<ExactNumber>()) as u64, limits)?;
        for bits in self.components {
            poll(c)?;
            components.push(ExactNumber::Float32Bits(bits));
        }
        args.push(FilterArgument {
            name: text("color", &mut used, limits)?,
            value: FilterValue::Color { components, encoding: SampleEncoding::Linear, profile: None },
        });
        for (name, bits) in ["transparency-threshold", "opacity-threshold"].into_iter().zip(self.thresholds) {
            poll(c)?;
            args.push(FilterArgument { name: text(name, &mut used, limits)?, value: FilterValue::Number(ExactNumber::Float32Bits(bits)) });
        }
        poll(c)?;
        Ok(args)
    }
}
fn bad(s: &str) -> IoError {
    IoError::Unsupported(format!("stored Color to Alpha candidate: {s}"))
}
fn poll(c: &Interrupt<'_>) -> Result<(), IoError> {
    c.check().map_err(|_| IoError::Cancelled)
}
fn check_limits(l: Limits) -> Result<(), IoError> {
    let m = Limits::default();
    if l.output_bytes > m.output_bytes || l.work_units > m.work_units || l.work_units < 512 {
        return Err(bad("lower-only limits"));
    }
    Ok(())
}
fn debit(used: &mut u64, n: u64, l: Limits) -> Result<(), IoError> {
    *used = used.checked_add(n).filter(|v| *v <= l.output_bytes).ok_or_else(|| bad("actual output capacity"))?;
    Ok(())
}
fn text(s: &str, used: &mut u64, l: Limits) -> Result<String, IoError> {
    if used.checked_add(s.len() as u64).is_none_or(|n| n > l.output_bytes) {
        return Err(bad("prospective name capacity"));
    }
    let mut out = String::new();
    out.try_reserve_exact(s.len()).map_err(|_| bad("name allocation"))?;
    debit(used, out.capacity() as u64, l)?;
    out.push_str(s);
    Ok(out)
}
fn word(bytes: &[u8], at: &mut usize) -> Result<u32, IoError> {
    let end = at.checked_add(4).ok_or_else(|| bad("raw offset"))?;
    let a: [u8; 4] = bytes.get(*at..end).ok_or_else(|| bad("raw word"))?.try_into().map_err(|_| bad("raw width"))?;
    *at = end;
    Ok(u32::from_be_bytes(a))
}
fn string<'a>(bytes: &'a [u8], at: &mut usize) -> Result<&'a [u8], IoError> {
    let n = word(bytes, at)? as usize;
    let end = at.checked_add(n).ok_or_else(|| bad("raw string offset"))?;
    let s = bytes.get(*at..end).and_then(|v| v.strip_suffix(&[0])).ok_or_else(|| bad("raw string framing"))?;
    *at = end;
    Ok(s)
}
fn property<'a, P>(e: &'a Effect<P>, a: &Argument) -> Result<(&'a RawProperty, usize), IoError> {
    if a.configuration.is_some() || a.graph.is_some() {
        return Err(bad("nested argument"));
    }
    let p = e.properties.get(a.property_index).ok_or_else(|| bad("raw property index"))?;
    if p.tag != 45 || p.payload.len() > 512 || p.payload.len() != p.declared_bytes as usize {
        return Err(bad("raw tag45 bounds"));
    }
    let mut at = 0;
    if string(&p.payload, &mut at)? != a.name.as_bytes() {
        return Err(bad("raw argument name"));
    }
    Ok((p, at))
}
/// Allocation-free observed framing check. This preserves linear literal storage
/// independently of the nonlinear operation/input working-domain interpretation.
pub fn bind<'a, P>(effect: &'a Effect<P>, limits: Limits, c: &Interrupt<'_>) -> Result<StoredColor<'a, P>, IoError> {
    check_limits(limits)?;
    poll(c)?;
    if effect.operation != "gegl:color-to-alpha" || effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 3 || effect.properties.len() > 64 {
        return Err(bad("identity/version/count"));
    }
    let mut color = None;
    let mut thresholds = [None, None];
    for a in &effect.arguments {
        poll(c)?;
        let (p, mut at) = property(effect, a)?;
        let kind = word(&p.payload, &mut at)?;
        match (a.name.as_str(), &a.value) {
            ("color", ArgumentValue::Color { encoding, samples, icc }) if encoding == "RGBA float" && samples.len() == 16 && icc.is_empty() && kind == 8 => {
                if color.is_some() || string(&p.payload, &mut at)? != b"RGBA float" || word(&p.payload, &mut at)? != 16 {
                    return Err(bad("linear color framing"));
                }
                let end = at.checked_add(16).ok_or_else(|| bad("sample offset"))?;
                let raw = p.payload.get(at..end).ok_or_else(|| bad("sample framing"))?;
                if raw != samples.as_slice() {
                    return Err(bad("raw/typed color mismatch"));
                }
                at = end;
                if word(&p.payload, &mut at)? != 0 || at != p.payload.len() {
                    return Err(bad("observed literal zero tail framing"));
                }
                let mut bits = [0u32; 4];
                for (out, chunk) in bits.iter_mut().zip(raw.chunks_exact(4)) {
                    let x: [u8; 4] = chunk.try_into().map_err(|_| bad("sample width"))?;
                    *out = u32::from_le_bytes(x);
                    if !f32::from_bits(*out).is_finite() || !(0.0..=1.0).contains(&f32::from_bits(*out)) {
                        return Err(bad("finite unit color"));
                    }
                }
                color = Some((a, p, bits));
            }
            (name @ ("transparency-threshold" | "opacity-threshold"), ArgumentValue::Float32Bits(bits)) if kind == 3 => {
                let i = usize::from(name == "opacity-threshold");
                if thresholds[i].is_some()
                    || word(&p.payload, &mut at)? != *bits
                    || at != p.payload.len()
                    || !f32::from_bits(*bits).is_finite()
                    || !(0.0..=1.0).contains(&f32::from_bits(*bits))
                {
                    return Err(bad("threshold framing/domain"));
                }
                thresholds[i] = Some(*bits);
            }
            _ => return Err(bad("exact typed arguments")),
        }
    }
    let (color, property, components) = color.ok_or_else(|| bad("missing color"))?;
    let thresholds = [thresholds[0].ok_or_else(|| bad("missing transparency"))?, thresholds[1].ok_or_else(|| bad("missing opacity"))?];
    poll(c)?;
    Ok(StoredColor { effect, color, property, components, thresholds })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn property(name: &str, kind: u32, value: &[u8]) -> RawProperty {
        let mut p = Vec::new();
        p.extend_from_slice(&((name.len() + 1) as u32).to_be_bytes());
        p.extend_from_slice(name.as_bytes());
        p.push(0);
        p.extend_from_slice(&kind.to_be_bytes());
        p.extend_from_slice(value);
        RawProperty { tag: 45, offset: 0, declared_bytes: p.len() as u32, payload: p }
    }
    fn fixture() -> Effect<()> {
        let bits = [0.25f32.to_bits(), 0.5f32.to_bits(), 0.75f32.to_bits(), 1f32.to_bits()];
        let samples: Vec<u8> = bits.into_iter().flat_map(u32::to_le_bytes).collect();
        let mut value = 11u32.to_be_bytes().to_vec();
        value.extend_from_slice(b"RGBA float\0");
        value.extend_from_slice(&16u32.to_be_bytes());
        value.extend_from_slice(&samples);
        value.extend_from_slice(&0u32.to_be_bytes());
        let mut properties = vec![property("color", 8, &value)];
        let mut arguments = vec![Argument {
            name: "color".into(),
            property_index: 0,
            value: ArgumentValue::Color { encoding: "RGBA float".into(), samples, icc: vec![] },
            configuration: None,
            graph: None,
        }];
        for (name, bits) in [("transparency-threshold", 0.125f32.to_bits()), ("opacity-threshold", 0.875f32.to_bits())] {
            let i = properties.len();
            properties.push(property(name, 3, &bits.to_be_bytes()));
            arguments.push(Argument { name: name.into(), property_index: i, value: ArgumentValue::Float32Bits(bits), configuration: None, graph: None });
        }
        Effect {
            name: String::new(),
            icon: String::new(),
            operation: "gegl:color-to-alpha".into(),
            version: Some("0:0".into()),
            properties,
            arguments,
            mask: None,
            state: Default::default(),
        }
    }
    #[test]
    fn linear_bits_raw_tail_preserved_and_forged_typed_data_refuses() {
        let mut e = fixture();
        let before = e.properties[0].payload.clone();
        let b = bind(&e, Limits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(b.component_bits(), [0.25f32.to_bits(), 0.5f32.to_bits(), 0.75f32.to_bits(), 1f32.to_bits()]);
        let args = b.canonical_arguments(Limits::default(), &Interrupt::NONE, &mut |_, _| Ok(())).unwrap();
        assert!(matches!(args[0].value, FilterValue::Color { encoding: SampleEncoding::Linear, .. }));
        assert_eq!(e.properties[0].payload, before);
        if let ArgumentValue::Color { samples, .. } = &mut e.arguments[0].value {
            samples[0] ^= 1;
        }
        assert!(bind(&e, Limits::default(), &Interrupt::NONE).is_err());
    }
    #[test]
    fn limits_and_cancel_never_make_literal_profile_authority() {
        let e = fixture();
        assert!(bind(&e, Limits { work_units: 0, ..Limits::default() }, &Interrupt::NONE).is_err());
        assert!(matches!(bind(&e, Limits::default(), &Interrupt::cancel_only(&|| true)), Err(IoError::Cancelled)));
    }
}
