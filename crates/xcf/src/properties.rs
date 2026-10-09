use crate::{reader::Reader, *};
pub(crate) fn parse(r: &mut Reader<'_, '_>, f: &mut XcfFile, mut drawable: Option<&mut Drawable>) -> Result<(), XcfError> {
    loop {
        r.poll()?;
        r.props += 1;
        if r.props > r.limits.properties {
            return Err(r.error(ErrorKind::Limit, "property count limit"));
        }
        let tag = r.u32()?;
        let n = r.u32()? as usize;
        if n > r.limits.property_bytes {
            return Err(r.error(ErrorKind::Limit, "property byte limit"));
        }
        r.metadata = r.metadata.checked_add(n).ok_or_else(|| r.error(ErrorKind::Limit, "metadata overflow"))?;
        if r.metadata > r.limits.metadata_bytes {
            return Err(r.error(ErrorKind::Limit, "aggregate metadata limit"));
        }
        let start = r.pos;
        let mut end = start.checked_add(n).ok_or_else(|| r.error(ErrorKind::Structure, "property offset overflow"))?;
        if r.bytes.get(start..end).is_none() {
            return Err(r.error(ErrorKind::Truncated, "property payload"));
        }
        if tag == 0 {
            if n != 0 {
                return Err(r.error(ErrorKind::Structure, "end property must be empty"));
            }
            break;
        }
        // Includes bounded formatted diagnostics (two 80-character Unicode names),
        // vector slots/capacity, and interpreted property state before allocation.
        r.charge(2048)?;
        if tag == 21 {
            while r.pos < end {
                r.poll()?;
                let name = r.string()?;
                let _flags = r.u32()?;
                let size = r.u32()? as usize;
                if size > r.limits.property_bytes || r.pos.checked_add(size).is_none_or(|p| p > end) {
                    return Err(r.error(ErrorKind::Structure, "parasite outside property"));
                }
                if name == "icc-profile" && drawable.is_none() {
                    if size > r.limits.profile_bytes {
                        return Err(r.error(ErrorKind::Limit, "ICC profile byte limit"));
                    }
                    if !f.icc.is_empty() {
                        return Err(r.error(ErrorKind::Structure, "duplicate ICC profile"));
                    }
                    let mut icc = r.vec(size)?;
                    icc.copy_from_slice(r.take(size)?);
                    f.icc = icc;
                } else {
                    r.take(size)?;
                    if let Some(d) = drawable.as_deref_mut() {
                        if name == "gimp-text-layer" || name == "gimp-text-layer-xcf" {
                            d.unsupported.push("editable GIMP text layer".into());
                        } else {
                            d.diagnostics.push(format!("{}: parasite {} omitted; GIMP metadata editing unavailable", short(&d.name), short(&name)));
                        }
                    } else {
                        f.diagnostics.push(format!("Image parasite {} omitted; GIMP metadata editing unavailable", short(&name)));
                    }
                }
            }
        } else if let Some(d) = drawable.as_deref_mut() {
            match tag {
                2 | 3 | 20 | 31 | 34 | 41 => {
                    r.pos = end;
                }
                4 => {
                    if n != 0 {
                        return Err(r.error(ErrorKind::Structure, "selection property length"));
                    }
                    d.selection = true;
                }
                5 => {
                    d.unsupported.push("floating selection".into());
                    r.pos = end;
                }
                6 => {
                    let v = r.u32()?;
                    if v > 255 {
                        return Err(r.error(ErrorKind::Structure, "opacity exceeds 255"));
                    }
                    d.opacity = v as f32 / 255.0;
                }
                7 => d.mode = r.i32()?,
                8 => d.visible = boolean(r)?,
                9 => {
                    let linked = boolean(r)?;
                    if linked {
                        d.diagnostics.push(format!("{}: linked-item editing state omitted", short(&d.name)));
                    }
                }
                10 => d.lock_alpha = boolean(r)?,
                11 => d.mask_enabled = boolean(r)?,
                12 => {
                    r.pos = end;
                }
                13 => {
                    if boolean(r)? {
                        d.unsupported.push("show-mask display mode".into());
                    }
                }
                14 | 16 | 38 => {
                    r.pos = end;
                }
                15 => d.offset = (r.i32()?, r.i32()?),
                26 => {
                    d.unsupported.push("editable GIMP text layer".into());
                    r.pos = end;
                }
                28 => d.lock_content = boolean(r)?,
                29 => {
                    if n != 0 {
                        return Err(r.error(ErrorKind::Structure, "group item property length"));
                    }
                    d.group = true;
                }
                30 => {
                    if !n.is_multiple_of(4) || n / 4 > r.limits.group_depth.saturating_add(1) {
                        return Err(r.error(ErrorKind::Limit, "invalid/excessive group path depth"));
                    }
                    r.charge(n)?;
                    d.path.try_reserve_exact(n / 4).map_err(|_| r.error(ErrorKind::Allocation, "group path"))?;
                    while r.pos < end {
                        d.path.push(r.u32()?);
                    }
                }
                32 => d.lock_position = boolean(r)?,
                33 => {
                    d.opacity = r.float()?;
                    if !(0.0..=1.0).contains(&d.opacity) {
                        return Err(r.error(ErrorKind::Structure, "opacity outside 0..1"));
                    }
                }
                35 => d.composite_mode = r.i32()?,
                36 => d.composite_space = r.i32()?,
                37 => d.blend_space = r.i32()?,
                42 => d.lock_visibility = boolean(r)?,
                47 => {
                    d.unsupported.push("editable GIMP vector layer".into());
                    r.pos = end;
                }
                48 | 49 => {
                    d.unsupported.push("linked layer/transform".into());
                    r.pos = end;
                }
                _ => {
                    d.unsupported.push(format!("unknown layer property {tag}"));
                    r.pos = end;
                }
            }
        } else {
            match tag {
                1 => {
                    let count = r.u32()? as usize;
                    if count == 0 || count > 256 {
                        return Err(r.error(ErrorKind::Structure, "invalid palette length"));
                    }
                    let expected =
                        count.checked_mul(3).and_then(|n| n.checked_add(4)).ok_or_else(|| r.error(ErrorKind::Structure, "palette length overflow"))?;
                    // Ancient writers stored n+4 while writing 3*n+4 bytes.
                    if n == count + 4 && n != expected {
                        end = start.checked_add(expected).ok_or_else(|| r.error(ErrorKind::Structure, "palette offset overflow"))?;
                    } else if n != expected {
                        return Err(r.error(ErrorKind::Structure, "palette payload length"));
                    }
                    r.charge(count * 3)?;
                    f.palette.try_reserve_exact(count).map_err(|_| r.error(ErrorKind::Allocation, "palette"))?;
                    for _ in 0..count {
                        let b = r.take(3)?;
                        let a: [u8; 3] = b.try_into().map_err(|_| r.error(ErrorKind::Structure, "palette entry"))?;
                        f.palette.push(a);
                    }
                }
                17 => {
                    let b = r.take(1)?.first().copied().ok_or_else(|| r.error(ErrorKind::Truncated, "compression"))?;
                    f.compression = match b {
                        0 => Compression::None,
                        1 => Compression::Rle,
                        2 => Compression::Zlib,
                        _ => return Err(r.error(ErrorKind::Compression, "unsupported compression")),
                    };
                }
                18 => {
                    if !n.is_multiple_of(5) {
                        return Err(r.error(ErrorKind::Structure, "guide property length"));
                    }
                    r.charge((n / 5) * std::mem::size_of::<Guide>())?;
                    f.guides.try_reserve_exact(n / 5).map_err(|_| r.error(ErrorKind::Allocation, "guides"))?;
                    while r.pos < end {
                        let position = r.i32()?;
                        let orientation = r.take(1)?.first().copied().ok_or_else(|| r.error(ErrorKind::Truncated, "guide orientation"))?;
                        let horizontal = match orientation {
                            1 => true,
                            2 => false,
                            _ => return Err(r.error(ErrorKind::Structure, "invalid guide orientation")),
                        };
                        f.guides.push(Guide { position, horizontal });
                    }
                }
                19 => {
                    f.dpi = (r.float()?, r.float()?);
                    if f.dpi.0 <= 0.0 || f.dpi.1 <= 0.0 {
                        return Err(r.error(ErrorKind::Structure, "nonpositive resolution"));
                    }
                }
                20 | 22 => {
                    r.pos = end;
                }
                23 | 25 => {
                    f.diagnostics.push("GIMP paths omitted; path editing unavailable".into());
                    r.pos = end;
                }
                _ => {
                    f.diagnostics.push(format!("Image property {tag} omitted; GIMP editing metadata unavailable"));
                    r.pos = end;
                }
            }
        }
        if r.pos != end {
            return Err(r.error(ErrorKind::Structure, format!("invalid length of property {tag}")));
        }
    }
    Ok(())
}
fn boolean(r: &mut Reader<'_, '_>) -> Result<bool, XcfError> {
    match r.u32()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(r.error(ErrorKind::Structure, "boolean property must be 0 or 1")),
    }
}

fn short(s: &str) -> String {
    s.chars().take(80).collect()
}
