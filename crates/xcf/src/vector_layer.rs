//! Public XCF24 vector-layer envelope. Resource identities remain inert.
use crate::{
    arguments::{blob, string, u32_value},
    reader::Reader,
    *,
};
fn boolean(r: &mut Reader<'_, '_>, end: u64) -> Result<bool, XcfError> {
    match u32_value(r, end)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(r.error(ErrorKind::Structure, "vector boolean")),
    }
}
fn finite(r: &mut Reader<'_, '_>, end: u64) -> Result<u32, XcfError> {
    let bits = u32_value(r, end)?;
    if !f32::from_bits(bits).is_finite() {
        return Err(r.error(ErrorKind::Structure, "nonfinite vector number"));
    }
    Ok(bits)
}
fn named(r: &mut Reader<'_, '_>, end: u64) -> Result<Option<String>, XcfError> {
    let at = r.pos;
    let n = u32_value(r, end)?;
    if n == 0 {
        return Ok(None);
    }
    r.pos = at;
    Ok(Some(string(r, end)?))
}
fn parse_fill(r: &mut Reader<'_, '_>, end: u64) -> Result<VectorFill, XcfError> {
    let style = u32_value(r, end)?;
    if style > 1 {
        return Err(r.error(ErrorKind::Unsupported, "unknown vector fill style"));
    }
    let antialias = boolean(r, end)?;
    let color =
        if let Some(encoding) = named(r, end)? { Some(StoredColor { encoding, samples: blob(r, end, false)?, icc: blob(r, end, true)? }) } else { None };
    let pattern = if let Some(name) = named(r, end)? { Some(StoredResource { name, collection: string(r, end)?, internal: boolean(r, end)? }) } else { None };
    Ok(VectorFill { style, antialias, color, pattern })
}
pub(crate) fn parse(r: &mut Reader<'_, '_>, end: u64) -> Result<VectorLayer, XcfError> {
    if r.version < 24 {
        return Err(r.error(ErrorKind::Unsupported, "vector layer before XCF24"));
    }
    r.charge(std::mem::size_of::<VectorLayer>())?;
    let source_modified = boolean(r, end)?;
    let path_tattoo = u32_value(r, end)?;
    let filled = boolean(r, end)?;
    let stroked = boolean(r, end)?;
    let fill = parse_fill(r, end)?;
    let stroke_fill = parse_fill(r, end)?;
    let width_bits = finite(r, end)?;
    let cap_style = u32_value(r, end)?;
    let join_style = u32_value(r, end)?;
    let miter_limit_bits = finite(r, end)?;
    if cap_style > 2 || join_style > 2 {
        return Err(r.error(ErrorKind::Unsupported, "unknown vector cap or join style"));
    }
    if f32::from_bits(width_bits) < 0. || f32::from_bits(miter_limit_bits) < 0. {
        return Err(r.error(ErrorKind::Structure, "negative vector stroke dimension"));
    }
    let count = u32_value(r, end)? as usize;
    let bytes = count.checked_mul(4).ok_or_else(|| r.error(ErrorKind::Limit, "vector dash count overflow"))?;
    if count > r.limits.visits || r.pos.checked_add(bytes as u64).is_none_or(|p| p > end) {
        return Err(r.error(ErrorKind::Structure, "vector dash count outside property"));
    }
    r.charge(bytes)?;
    let mut dashes_bits = Vec::new();
    dashes_bits.try_reserve_exact(count).map_err(|_| r.error(ErrorKind::Allocation, "vector dashes"))?;
    for _ in 0..count {
        r.poll()?;
        let bits = finite(r, end)?;
        if f32::from_bits(bits) < 0. {
            return Err(r.error(ErrorKind::Structure, "negative vector dash"));
        }
        dashes_bits.push(bits);
    }
    if r.pos != end {
        return Err(r.error(ErrorKind::Unsupported, "unknown required vector configuration fields"));
    }
    let stroke = VectorStroke { fill: stroke_fill, width_bits, cap_style, join_style, miter_limit_bits, dashes_bits };
    Ok(VectorLayer { source_modified, path_tattoo, filled, stroked, fill, stroke })
}
