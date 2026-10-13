use crate::{reader::Reader, *};
pub(crate) fn parse<P>(r: &mut Reader<'_, '_>, f: &mut XcfFile<P>, mut drawable: Option<&mut Drawable<P>>) -> Result<(), XcfError> {
    // FLOAT_OPACITY is authoritative over compatibility OPACITY regardless of
    // property order; both original payloads remain retained below.
    let mut float_opacity = false;
    let mut singleton_seen = 0u64;
    let mut icc_seen = false;
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
        let mut end = start.checked_add(n as u64).ok_or_else(|| r.error(ErrorKind::Structure, "property offset overflow"))?;
        // Public XCF specification documents incorrect USER_UNIT lengths in
        // historical and current writers. Its versioned fields delimit it.
        let unreliable_length = tag == 24 && drawable.is_none() || tag == 48 && drawable.is_some();
        if !unreliable_length && end > r.len() {
            return Err(r.error(ErrorKind::Truncated, "property payload"));
        }
        if tag == 0 {
            if n != 0 {
                return Err(r.error(ErrorKind::Structure, "end property must be empty"));
            }
            break;
        }
        let singleton = if drawable.is_some() { matches!(tag, 2..=16 | 20 | 26 | 28..=38 | 42 | 47..=49) } else { matches!(tag, 1 | 17..=20 | 22..=25) };
        if singleton {
            unique(r, &mut singleton_seen, tag)?;
        }
        // Includes bounded formatted diagnostics (two 80-character Unicode names),
        // vector slots/capacity, and interpreted property state before allocation.
        r.charge(2048)?;
        if tag == 21 {
            while r.pos < end {
                r.poll()?;
                let name = r.string()?;
                let flags = r.u32()?;
                let size = r.u32()? as usize;
                if size > r.limits.property_bytes || r.pos.checked_add(size as u64).is_none_or(|p| p > end) {
                    return Err(r.error(ErrorKind::Structure, "parasite outside property"));
                }
                if name == "icc-profile" && drawable.is_none() {
                    if size > r.limits.profile_bytes {
                        return Err(r.error(ErrorKind::Limit, "ICC profile byte limit"));
                    }
                    if icc_seen {
                        return Err(r.error(ErrorKind::Structure, "duplicate ICC profile"));
                    }
                    icc_seen = true;
                    let mut icc = r.vec(size)?;
                    icc.copy_from_slice(&r.take(size)?);
                    f.icc = icc;
                } else {
                    let data = r.take(size)?;
                    if let Some(d) = drawable.as_deref_mut() {
                        if name == "gimp-text-layer" || name == "gimp-text-layer-xcf" {
                            if d.text.is_some() {
                                return Err(r.error(ErrorKind::Structure, "duplicate text configuration"));
                            }
                            d.text = Some(crate::text::parse(r, &data, flags)?);
                            d.unsupported.push("editable GIMP text layer".into());
                        } else {
                            d.diagnostics.push(format!(
                                "{}: parasite {} retained as source metadata; GIMP metadata editing unavailable",
                                short(&d.name),
                                short(&name)
                            ));
                        }
                    } else {
                        f.diagnostics.push(format!("Image parasite {} retained as source metadata; GIMP metadata editing unavailable", short(&name)));
                    }
                }
            }
        } else if let Some(d) = drawable.as_deref_mut() {
            match tag {
                2 | 3 => {
                    if n != 0 {
                        return Err(r.error(ErrorKind::Structure, "active item property length"));
                    }
                    d.active = true;
                }
                20 => d.tattoo = Some(r.u32()?),
                31 => d.group_flags = r.u32()?,
                34 => d.color_tag = Some(r.u32()?),
                41 => {
                    r.charge(std::mem::size_of::<u32>())?;
                    d.item_sets.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "item set membership"))?;
                    d.item_sets.push(r.u32()?);
                }
                4 => {
                    if n != 0 {
                        return Err(r.error(ErrorKind::Structure, "selection property length"));
                    }
                    d.selection = true;
                }
                5 => {
                    d.unsupported.push("floating selection".into());
                    d.floating_attachment = Some(r.pointer()?);
                }
                6 => {
                    let v = r.u32()?;
                    if v > 255 {
                        return Err(r.error(ErrorKind::Structure, "opacity exceeds 255"));
                    }
                    if !float_opacity {
                        d.opacity = v as f32 / 255.0;
                    }
                }
                7 => d.mode = r.i32()?,
                8 => d.visible = boolean(r)?,
                9 => {
                    d.legacy_linked = boolean(r)?;
                }
                10 => d.lock_alpha = boolean(r)?,
                11 => d.mask_enabled = boolean(r)?,
                12 => d.mask_editing = boolean(r)?,
                13 => {
                    d.mask_shown = boolean(r)?;
                    if d.mask_shown {
                        d.unsupported.push("show-mask display mode".into());
                    }
                }
                14 => d.show_masked = boolean(r)?,
                16 => {
                    let b: [u8; 3] = r.take(3)?.as_ref().try_into().map_err(|_| r.error(ErrorKind::Structure, "channel color"))?;
                    if d.channel_color.is_none() {
                        d.channel_color = Some(b.map(|v| v as f32 / 255.0));
                    }
                }
                38 => {
                    let color = [r.float()?, r.float()?, r.float()?];
                    if color.iter().any(|v| !(0.0..=1.0).contains(v)) {
                        return Err(r.error(ErrorKind::Structure, "channel color outside 0..1"));
                    }
                    d.channel_color = Some(color);
                }
                15 => d.offset = (r.i32()?, r.i32()?),
                26 => {
                    d.text_flags = r.u32()?;
                    if d.text_flags & !3 != 0 {
                        d.unsupported.push("unknown required text layer flags".into());
                    }
                    d.unsupported.push("editable GIMP text layer".into());
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
                    float_opacity = true;
                }
                35 => d.composite_mode = r.i32()?,
                36 => d.composite_space = r.i32()?,
                37 => d.blend_space = r.i32()?,
                42 => d.lock_visibility = boolean(r)?,
                47 => {
                    d.unsupported.push("editable GIMP vector layer".into());
                    if d.vector.is_some() {
                        return Err(r.error(ErrorKind::Structure, "duplicate vector layer configuration"));
                    }
                    d.vector = Some(crate::vector_layer::parse(r, end)?);
                }
                48 => {
                    if r.version < 25 {
                        return Err(r.error(ErrorKind::Unsupported, "linked layer before XCF25"));
                    }
                    d.unsupported.push("linked layer/transform".into());
                    let flags = r.u32()?;
                    if flags & !3 != 0 {
                        d.unsupported.push(format!("unknown linked-layer flags 0x{:08x}", flags & !3));
                    }
                    let path = r.string()?;
                    let width = r.u32()?;
                    let height = r.u32()?;
                    end = r.pos;
                    actual_metadata(r, start, end, n)?;
                    d.link = Some(LinkedLayer { flags, path, width, height });
                }
                49 => {
                    if r.version < 25 {
                        return Err(r.error(ErrorKind::Unsupported, "layer transform before XCF25"));
                    }
                    d.unsupported.push("linked layer/transform".into());
                    let offset = (r.i32()?, r.i32()?);
                    let interpolation = r.u32()?;
                    if interpolation > 4 {
                        return Err(r.error(ErrorKind::Unsupported, "unknown linked-layer interpolation"));
                    }
                    let mut matrix = [0.0; 9];
                    for coefficient in &mut matrix {
                        *coefficient = r.float()?;
                    }
                    d.transform = Some(LayerTransform { offset, interpolation, matrix });
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
                        end = start.checked_add(expected as u64).ok_or_else(|| r.error(ErrorKind::Structure, "palette offset overflow"))?;
                    } else if n != expected {
                        return Err(r.error(ErrorKind::Structure, "palette payload length"));
                    }
                    r.charge(count * 3)?;
                    f.palette.try_reserve_exact(count).map_err(|_| r.error(ErrorKind::Allocation, "palette"))?;
                    for _ in 0..count {
                        let b = r.take(3)?;
                        let a: [u8; 3] = b.as_ref().try_into().map_err(|_| r.error(ErrorKind::Structure, "palette entry"))?;
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
                        // Off-canvas negative guides became meaningful in XCF15.
                        // Historical negative entries are ignored semantically,
                        // while the whole original property is retained below.
                        if f.version >= 15 || position >= 0 {
                            f.guides.push(Guide { position, horizontal });
                        }
                    }
                }
                19 => {
                    f.dpi = (r.float()?, r.float()?);
                    if f.dpi.0 <= 0.0 || f.dpi.1 <= 0.0 {
                        return Err(r.error(ErrorKind::Structure, "nonpositive resolution"));
                    }
                }
                20 => {
                    f.tattoo = Some(r.u32()?);
                }
                40 => {
                    let item_type = r.u32()?;
                    let method = r.u32()?;
                    let name = r.string()?;
                    if item_type > 2 || !matches!(method, 0..=2 | u32::MAX) {
                        return Err(r.error(ErrorKind::Unsupported, "unknown item set type or search method"));
                    }
                    r.charge(std::mem::size_of::<ItemSet>())?;
                    f.item_sets.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "item set declarations"))?;
                    f.item_sets.push(ItemSet { item_type, method, name });
                }
                22 => {
                    f.unit = match r.u32()? {
                        1 => Unit::Inch,
                        2 => Unit::Millimeter,
                        3 => Unit::Point,
                        4 => Unit::Pica,
                        _ => return Err(r.error(ErrorKind::Unsupported, "unknown physical unit identifier")),
                    };
                }
                24 => {
                    let factor = r.float()?;
                    if factor <= 0.0 {
                        return Err(r.error(ErrorKind::Structure, "nonpositive custom unit factor"));
                    }
                    let digits = r.u32()?;
                    let name = r.string()?;
                    let symbol = r.string()?;
                    let abbreviation = r.string()?;
                    let (singular, plural) = if f.version < 21 { (Some(r.string()?), Some(r.string()?)) } else { (None, None) };
                    end = r.pos;
                    actual_metadata(r, start, end, n)?;
                    f.unit = Unit::Custom(CustomUnit { factor, digits, name, symbol, abbreviation, singular, plural });
                }
                25 => crate::paths::legacy_vectors(r, f, end)?,
                23 => crate::paths::legacy_paths(r, f, end)?,
                _ => {
                    f.diagnostics.push(format!("Image property {tag} omitted; GIMP editing metadata unavailable"));
                    r.pos = end;
                }
            }
        }
        if r.pos != end {
            return Err(r.error(ErrorKind::Structure, format!("invalid length of property {tag}")));
        }
        let property = r.retain_property(tag, start, end, n as u32)?;
        let out = if let Some(drawable) = drawable.as_deref_mut() { &mut drawable.raw_properties } else { &mut f.raw_properties };
        out.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "retained property list"))?;
        out.push(property);
    }
    Ok(())
}
fn actual_metadata(r: &mut Reader<'_, '_>, start: u64, end: u64, declared: usize) -> Result<(), XcfError> {
    let actual = usize::try_from(end.checked_sub(start).ok_or_else(|| r.error(ErrorKind::Structure, "property offset"))?)
        .map_err(|_| r.error(ErrorKind::Limit, "property width"))?;
    if actual > r.limits.property_bytes {
        return Err(r.error(ErrorKind::Limit, "actual property byte limit"));
    }
    r.metadata = r.metadata.checked_sub(declared).and_then(|v| v.checked_add(actual)).ok_or_else(|| r.error(ErrorKind::Limit, "actual metadata overflow"))?;
    if r.metadata > r.limits.metadata_bytes {
        return Err(r.error(ErrorKind::Limit, "aggregate actual metadata limit"));
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

/// The format defines repeated collection properties separately. A duplicate
/// singleton has no unambiguous semantic authority, so do not silently win last.
pub(crate) fn unique(r: &Reader<'_, '_>, seen: &mut u64, tag: u32) -> Result<(), XcfError> {
    let bit = 1u64.checked_shl(tag).ok_or_else(|| r.error(ErrorKind::Structure, "singleton tag width"))?;
    if *seen & bit != 0 {
        return Err(r.error(ErrorKind::Structure, format!("duplicate singleton property {tag}")));
    }
    *seen |= bit;
    Ok(())
}

fn short(s: &str) -> String {
    s.chars().take(80).collect()
}
