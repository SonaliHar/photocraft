//! Ordered stock configuration records. No mathematical execution or defaults.
use crate::{reader::Reader, *};
mod color_balance;
mod brightness_contrast;
mod hue_saturation;
fn fail(r: &Reader<'_, '_>) -> XcfError {
    r.error(ErrorKind::Unsupported, "unqualified required effect configuration shape")
}
fn list(n: &ConfigNode) -> Option<&[ConfigNode]> {
    if let ConfigValue::List(v) = &n.value { Some(v) } else { None }
}
fn atom(n: &ConfigNode) -> Option<&str> {
    if let ConfigValue::Atom(v) = &n.value { Some(v) } else { None }
}
fn number(n: &ConfigNode) -> Option<u64> {
    if let ConfigValue::Number { bits, .. } = n.value { Some(bits) } else { None }
}
fn integer(n: &ConfigNode) -> Option<u32> {
    if let ConfigValue::Number { literal, .. } = &n.value { literal.parse().ok() } else { None }
}
fn copy(r: &mut Reader<'_, '_>, s: &str) -> Result<String, XcfError> {
    r.charge(s.len())?;
    let mut v = String::new();
    v.try_reserve_exact(s.len()).map_err(|_| r.error(ErrorKind::Allocation, "configuration identity"))?;
    v.push_str(s);
    Ok(v)
}
fn field(n: &ConfigNode) -> Option<(&str, &[ConfigNode])> {
    let v = list(n)?;
    Some((atom(v.first()?)?, v.get(1..)?))
}
fn scalar(r: &Reader<'_, '_>, v: &[ConfigNode]) -> Result<u64, XcfError> {
    if v.len() != 1 {
        return Err(fail(r));
    }
    number(v.first().ok_or_else(|| fail(r))?).ok_or_else(|| fail(r))
}
fn named<'a>(r: &Reader<'_, '_>, v: &'a [ConfigNode]) -> Result<&'a str, XcfError> {
    if v.len() != 1 {
        return Err(fail(r));
    }
    atom(v.first().ok_or_else(|| fail(r))?).ok_or_else(|| fail(r))
}
fn flag(r: &Reader<'_, '_>, v: &[ConfigNode]) -> Result<bool, XcfError> {
    match named(r, v)? {
        "yes" => Ok(true),
        "no" => Ok(false),
        _ => Err(fail(r)),
    }
}
fn channel(r: &mut Reader<'_, '_>, v: &[ConfigNode]) -> Result<String, XcfError> {
    let s = named(r, v)?;
    if !matches!(s, "value" | "red" | "green" | "blue" | "alpha") {
        return Err(fail(r));
    }
    copy(r, s)
}
fn numbers(r: &mut Reader<'_, '_>, v: &[ConfigNode]) -> Result<Vec<u64>, XcfError> {
    let count = integer(v.first().ok_or_else(|| fail(r))?).ok_or_else(|| fail(r))? as usize;
    if v.len() != count.checked_add(1).ok_or_else(|| fail(r))? {
        return Err(fail(r));
    }
    r.charge(count.checked_mul(8).ok_or_else(|| fail(r))?)?;
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| r.error(ErrorKind::Allocation, "configuration numbers"))?;
    for n in v.get(1..).ok_or_else(|| fail(r))? {
        r.poll()?;
        out.push(number(n).ok_or_else(|| fail(r))?);
    }
    Ok(out)
}
fn curve(r: &mut Reader<'_, '_>, v: &[ConfigNode]) -> Result<StoredCurve, XcfError> {
    let mut curve_type = None;
    let mut points = None;
    let mut types = None;
    let mut n_samples = None;
    let mut samples = None;
    for n in v {
        r.poll()?;
        let (name, args) = field(n).ok_or_else(|| fail(r))?;
        match name {
            "curve-type" => {
                if curve_type.is_some() {
                    return Err(fail(r));
                }
                let s = named(r, args)?;
                if !matches!(s, "smooth" | "free") {
                    return Err(fail(r));
                }
                curve_type = Some(copy(r, s)?);
            }
            "points" => {
                if points.is_some() {
                    return Err(fail(r));
                }
                points = Some(numbers(r, args)?);
            }
            "point-types" => {
                if types.is_some() {
                    return Err(fail(r));
                }
                let count = integer(args.first().ok_or_else(|| fail(r))?).ok_or_else(|| fail(r))? as usize;
                if args.len() != count.saturating_add(1) {
                    return Err(fail(r));
                }
                r.charge(count * std::mem::size_of::<String>())?;
                let mut out = Vec::new();
                out.try_reserve_exact(count).map_err(|_| r.error(ErrorKind::Allocation, "curve point types"))?;
                for n in args.get(1..).ok_or_else(|| fail(r))? {
                    let s = atom(n).ok_or_else(|| fail(r))?;
                    if !matches!(s, "smooth" | "corner") {
                        return Err(fail(r));
                    }
                    out.push(copy(r, s)?);
                }
                types = Some(out);
            }
            "n-samples" => {
                if n_samples.is_some() || args.len() != 1 {
                    return Err(fail(r));
                }
                n_samples = Some(integer(args.first().ok_or_else(|| fail(r))?).ok_or_else(|| fail(r))?);
            }
            "samples" => {
                if samples.is_some() {
                    return Err(fail(r));
                }
                samples = Some(numbers(r, args)?);
            }
            _ => return Err(fail(r)),
        }
    }
    let points_bits = points.ok_or_else(|| fail(r))?;
    let point_types = types.ok_or_else(|| fail(r))?;
    let n_samples = n_samples.ok_or_else(|| fail(r))?;
    let samples_bits = samples.ok_or_else(|| fail(r))?;
    if !points_bits.len().is_multiple_of(2) || point_types.len() != points_bits.len() / 2 || samples_bits.len() != n_samples as usize {
        return Err(fail(r));
    }
    Ok(StoredCurve { curve_type: curve_type.ok_or_else(|| fail(r))?, points_bits, point_types, n_samples, samples_bits })
}
pub(crate) fn typed(r: &mut Reader<'_, '_>, operation: &str, nodes: &[ConfigNode]) -> Result<Option<TypedConfiguration>, XcfError> {
    if operation == "gimp:hue-saturation" {
        return hue_saturation::typed(r, nodes).map(Some);
    }
    if operation == "gimp:brightness-contrast" {
        return brightness_contrast::typed(r, nodes).map(Some);
    }
    if operation == "gimp:color-balance" {
        return color_balance::typed(r, nodes).map(Some);
    }
    if !matches!(operation, "gimp:levels" | "gimp:curves") {
        return Ok(None);
    }
    let mut time = None;
    let mut trc = None;
    let mut clamp_input = None;
    let mut clamp_output = None;
    let mut current = None;
    let mut channels = Vec::new();
    let mut curves = Vec::new();
    let mut values = [None; 5];
    for node in nodes {
        r.poll()?;
        let (name, args) = field(node).ok_or_else(|| fail(r))?;
        match name {
            "time" => {
                if time.is_some() {
                    return Err(fail(r));
                }
                time = Some(scalar(r, args)?);
            }
            "trc" => {
                if trc.is_some() {
                    return Err(fail(r));
                }
                let s = named(r, args)?;
                if !matches!(s, "linear" | "non-linear" | "perceptual") {
                    return Err(fail(r));
                }
                trc = Some(copy(r, s)?);
            }
            "clamp-input" if operation == "gimp:levels" => {
                if clamp_input.is_some() {
                    return Err(fail(r));
                }
                clamp_input = Some(flag(r, args)?);
            }
            "clamp-output" if operation == "gimp:levels" => {
                if clamp_output.is_some() {
                    return Err(fail(r));
                }
                clamp_output = Some(flag(r, args)?);
            }
            "channel" => {
                if operation == "gimp:levels" {
                    if let Some(channel) = current.take() {
                        push_levels(r, &mut channels, channel, &values)?;
                        values = [None; 5];
                    }
                } else if current.is_some() {
                    return Err(fail(r));
                }
                current = Some(channel(r, args)?);
            }
            "curve" if operation == "gimp:curves" => {
                let channel = current.take().ok_or_else(|| fail(r))?;
                let curve = curve(r, args)?;
                r.charge(std::mem::size_of::<CurveChannel>())?;
                curves.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "curve channels"))?;
                curves.push(CurveChannel { channel, curve });
            }
            "low-input" | "high-input" | "gamma" | "low-output" | "high-output" if operation == "gimp:levels" => {
                if current.is_none() {
                    return Err(fail(r));
                }
                let index = match name {
                    "low-input" => 0,
                    "high-input" => 1,
                    "gamma" => 2,
                    "low-output" => 3,
                    _ => 4,
                };
                let slot = values.get_mut(index).ok_or_else(|| fail(r))?;
                if slot.is_some() {
                    return Err(fail(r));
                }
                *slot = Some(scalar(r, args)?);
            }
            _ => return Err(fail(r)),
        }
    }
    if operation == "gimp:levels" {
        if let Some(channel) = current {
            push_levels(r, &mut channels, channel, &values)?;
        }
        Ok(Some(TypedConfiguration::Levels(LevelsConfiguration { time_bits: time, trc, clamp_input, clamp_output, channels })))
    } else {
        if current.is_some() {
            return Err(fail(r));
        }
        Ok(Some(TypedConfiguration::Curves(CurvesConfiguration { time_bits: time, trc, channels: curves })))
    }
}
fn push_levels(r: &mut Reader<'_, '_>, out: &mut Vec<LevelsChannel>, channel: String, v: &[Option<u64>; 5]) -> Result<(), XcfError> {
    let [Some(low_input_bits), Some(high_input_bits), Some(gamma_bits), Some(low_output_bits), Some(high_output_bits)] = *v else {
        return Err(fail(r));
    };
    r.charge(std::mem::size_of::<LevelsChannel>())?;
    out.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "levels channels"))?;
    out.push(LevelsChannel { channel, low_input_bits, high_input_bits, gamma_bits, low_output_bits, high_output_bits });
    Ok(())
}
