//! Clean-room typed envelope parsing, independent of operation execution.
use crate::{reader::Reader, *};
fn take<'a, 'o>(r: &mut Reader<'a, 'o>, end: u64, n: usize) -> Result<crate::reader::ReadBytes<'a, 'o, 'o>, XcfError> {
    if r.pos.checked_add(n as u64).is_none_or(|p| p > end) {
        return Err(r.error(ErrorKind::Structure, "argument outside property envelope"));
    }
    r.take(n)
}
pub(crate) fn u32_value(r: &mut Reader<'_, '_>, end: u64) -> Result<u32, XcfError> {
    let data: [u8; 4] = take(r, end, 4)?.as_ref().try_into().map_err(|_| r.error(ErrorKind::Structure, "argument integer width"))?;
    Ok(u32::from_be_bytes(data))
}
pub(crate) fn string(r: &mut Reader<'_, '_>, end: u64) -> Result<String, XcfError> {
    let pos = r.pos;
    let n = u32_value(r, end)? as usize;
    if r.pos.checked_add(n as u64).is_none_or(|p| p > end) {
        return Err(r.error(ErrorKind::Structure, "argument string outside property"));
    }
    r.pos = pos;
    r.string()
}
pub(crate) fn blob(r: &mut Reader<'_, '_>, end: u64, profile: bool) -> Result<Vec<u8>, XcfError> {
    let n = u32_value(r, end)? as usize;
    if n > if profile { r.limits.profile_bytes } else { r.limits.property_bytes } {
        return Err(r.error(ErrorKind::Limit, "argument color/profile byte limit"));
    }
    let data = take(r, end, n)?;
    let mut out = r.vec(n)?;
    for (dst, src) in out.chunks_mut(65536).zip(data.chunks(65536)) {
        r.poll()?;
        dst.copy_from_slice(src);
    }
    Ok(out)
}
pub(crate) fn parse(r: &mut Reader<'_, '_>, end: u64, property_index: usize) -> Result<Argument, XcfError> {
    let name = string(r, end)?;
    let kind = u32_value(r, end)?;
    let value = match kind {
        1 => ArgumentValue::Integer(u32_value(r, end)? as i32),
        2 => ArgumentValue::Boolean(match u32_value(r, end)? {
            0 => false,
            1 => true,
            _ => return Err(r.error(ErrorKind::Structure, "argument boolean outside 0..1")),
        }),
        3 => match end.checked_sub(r.pos) {
            Some(4) => ArgumentValue::Float32Bits(u32_value(r, end)?),
            Some(8) => {
                let data: [u8; 8] = take(r, end, 8)?.as_ref().try_into().map_err(|_| r.error(ErrorKind::Structure, "argument float width"))?;
                ArgumentValue::Float64Bits(u64::from_be_bytes(data))
            }
            _ => return Err(r.error(ErrorKind::Structure, "argument float width must be four or eight")),
        },
        4 => ArgumentValue::String(string(r, end)?),
        5 => ArgumentValue::Enum(u32_value(r, end)? as i32),
        6 => ArgumentValue::Config(string(r, end)?),
        7 => ArgumentValue::Unsigned(u32_value(r, end)?),
        8 => {
            // Since GIMP3.2, NULL GeglColor is exactly a zero STRING length;
            // there are no sample/profile fields after that marker. The outer
            // XCF header can remain22 because effects use that structural epoch.
            let at = r.pos;
            let encoding_bytes = u32_value(r, end)?;
            if encoding_bytes == 0 {
                ArgumentValue::NullColor
            } else {
                r.pos = at;
                ArgumentValue::Color { encoding: string(r, end)?, samples: blob(r, end, false)?, icc: blob(r, end, true)? }
            }
        }
        unknown => {
            r.pos = end;
            ArgumentValue::Unknown(unknown)
        }
    };
    if r.pos != end {
        return Err(r.error(ErrorKind::Structure, "trailing argument value bytes"));
    }
    let configuration =
        if let ArgumentValue::Config(data) = &value { Some(EffectConfiguration { nodes: crate::text::tree(r, data.as_bytes())?, typed: None }) } else { None };
    Ok(Argument { name, property_index, value, configuration, graph: None })
}
