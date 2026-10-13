//! Clean-room layouts from the public XCF specification, with original float bits.
use crate::{reader::Reader, *};
fn boolean(r: &mut Reader<'_, '_>) -> Result<bool, XcfError> {
    match r.u32()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(r.error(ErrorKind::Structure, "path boolean")),
    }
}
fn count(r: &mut Reader<'_, '_>, n: usize, bytes: usize, end: u64) -> Result<(), XcfError> {
    r.poll()?;
    let total = n.checked_mul(bytes).ok_or_else(|| r.error(ErrorKind::Limit, "path count overflow"))?;
    if n > r.limits.visits || r.pos.checked_add(total as u64).is_none_or(|v| v > end) {
        return Err(r.error(ErrorKind::Structure, "path count outside payload"));
    }
    Ok(())
}
pub(crate) fn append<P>(r: &mut Reader<'_, '_>, f: &mut XcfFile<P>, p: VectorPath) -> Result<(), XcfError> {
    if f.paths.len() >= r.limits.drawables {
        return Err(r.error(ErrorKind::Limit, "path count limit"));
    }
    r.charge(std::mem::size_of::<VectorPath>())?;
    f.paths.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "paths"))?;
    f.paths.push(p);
    Ok(())
}
fn parasite(r: &mut Reader<'_, '_>, p: &mut VectorPath, end: u64) -> Result<(), XcfError> {
    let name = r.string()?;
    let flags = r.u32()?;
    let n = r.u32()? as usize;
    if n > r.limits.property_bytes {
        return Err(r.error(ErrorKind::Limit, "path parasite size"));
    }
    count(r, n, 1, end)?;
    let mut bytes = r.vec(n)?;
    bytes.copy_from_slice(&r.take(n)?);
    r.charge(std::mem::size_of::<PathParasite>())?;
    p.parasites.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "path parasites"))?;
    p.parasites.push(PathParasite { name, flags, bytes });
    Ok(())
}
fn strokes(r: &mut Reader<'_, '_>, p: &mut VectorPath, k: usize, end: u64) -> Result<(), XcfError> {
    count(r, k, 16, end)?;
    for _ in 0..k {
        r.poll()?;
        if r.u32()? != 1 {
            return Err(r.error(ErrorKind::Unsupported, "unknown path stroke type"));
        }
        let closed = boolean(r)?;
        let nf = r.u32()? as usize;
        let np = r.u32()? as usize;
        if !(2..=6).contains(&nf) {
            return Err(r.error(ErrorKind::Structure, "path component count"));
        }
        count(r, np, (nf + 1) * 4, end)?;
        r.charge(
            std::mem::size_of::<PathStroke>()
                + np.checked_mul(std::mem::size_of::<PathPoint>() + nf * 4).ok_or_else(|| r.error(ErrorKind::Limit, "path allocation size"))?,
        )?;
        let mut points = Vec::new();
        points.try_reserve_exact(np).map_err(|_| r.error(ErrorKind::Allocation, "path points"))?;
        for _ in 0..np {
            r.poll()?;
            let kind = r.u32()?;
            if kind > 1 {
                return Err(r.error(ErrorKind::Unsupported, "unknown path control point"));
            }
            let mut components = Vec::new();
            components.try_reserve_exact(nf).map_err(|_| r.error(ErrorKind::Allocation, "path components"))?;
            for _ in 0..nf {
                components.push(r.float()?.to_bits());
            }
            points.push(PathPoint { kind, components, legacy_integer_coordinates: None });
        }
        p.strokes.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "path strokes"))?;
        p.strokes.push(PathStroke { closed, points });
    }
    Ok(())
}
pub(crate) fn modern(r: &mut Reader<'_, '_>, offset: u64) -> Result<VectorPath, XcfError> {
    r.enter(offset)?;
    let name = r.string()?;
    let n = r.u32()? as usize;
    if n > r.limits.property_bytes {
        return Err(r.error(ErrorKind::Limit, "path payload size"));
    }
    let end = r.pos.checked_add(n as u64).ok_or_else(|| r.error(ErrorKind::Structure, "path extent"))?;
    if end > r.len() {
        return Err(r.error(ErrorKind::Truncated, "path payload"));
    }
    let mut p = VectorPath { record_offset: Some(offset), name, ..Default::default() };
    let mut singleton_seen = 0u64;
    loop {
        r.poll()?;
        r.props += 1;
        if r.props > r.limits.properties {
            return Err(r.error(ErrorKind::Limit, "path property count"));
        }
        let tag = r.u32()?;
        let n = r.u32()? as usize;
        let start = r.pos;
        let pe = start.checked_add(n as u64).ok_or_else(|| r.error(ErrorKind::Structure, "path property extent"))?;
        if pe > end || n > r.limits.property_bytes {
            return Err(r.error(ErrorKind::Structure, "path property outside payload"));
        }
        if tag == 0 {
            if n != 0 {
                return Err(r.error(ErrorKind::Structure, "path END payload"));
            }
            break;
        }
        if matches!(tag, 8 | 9 | 20 | 28 | 32 | 34 | 42 | 43) {
            crate::properties::unique(r, &mut singleton_seen, tag)?;
        }
        r.metadata = r.metadata.checked_add(n).ok_or_else(|| r.error(ErrorKind::Limit, "path metadata overflow"))?;
        if r.metadata > r.limits.metadata_bytes {
            return Err(r.error(ErrorKind::Limit, "path metadata limit"));
        }
        match tag {
            8 => p.visible = boolean(r)?,
            9 => p.legacy_linked = boolean(r)?,
            20 => p.tattoo = Some(r.u32()?),
            28 => p.lock_content = boolean(r)?,
            32 => p.lock_position = boolean(r)?,
            42 => p.lock_visibility = boolean(r)?,
            34 => p.color_tag = Some(r.u32()?),
            43 => {
                if n != 0 {
                    return Err(r.error(ErrorKind::Structure, "selected path payload"));
                }
                p.selected = true;
            }
            41 => {
                r.charge(4)?;
                p.item_sets.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "path item sets"))?;
                p.item_sets.push(r.u32()?);
            }
            21 => {
                while r.pos < pe {
                    parasite(r, &mut p, pe)?;
                }
            }
            _ => {
                r.charge(128)?;
                p.unsupported.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "path unsupported fields"))?;
                p.unsupported.push(format!("unknown path property {tag}"));
                r.pos = pe;
            }
        }
        if r.pos != pe {
            return Err(r.error(ErrorKind::Structure, "path property length"));
        }
        let raw = r.retain_property(tag, start, pe, n as u32)?;
        p.properties.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "path properties"))?;
        p.properties.push(raw);
    }
    if r.u32()? != 1 {
        return Err(r.error(ErrorKind::Unsupported, "unknown path payload version"));
    }
    let k = r.u32()? as usize;
    strokes(r, &mut p, k, end)?;
    if r.pos != end {
        return Err(r.error(ErrorKind::Structure, "path payload length"));
    }
    let size = usize::try_from(end.checked_sub(offset).ok_or_else(|| r.error(ErrorKind::Structure, "path raw extent"))?)
        .map_err(|_| r.error(ErrorKind::Limit, "path width"))?;
    if size > r.limits.metadata_bytes {
        return Err(r.error(ErrorKind::Limit, "path raw record limit"));
    }
    r.metadata = r.metadata.checked_add(size).ok_or_else(|| r.error(ErrorKind::Limit, "path raw metadata overflow"))?;
    if r.metadata > r.limits.metadata_bytes {
        return Err(r.error(ErrorKind::Limit, "aggregate path raw metadata limit"));
    }
    let mut raw = r.vec(size)?;
    r.read_at(offset, &mut raw)?;
    p.raw_record = raw;
    Ok(p)
}
pub(crate) fn legacy_vectors<P>(r: &mut Reader<'_, '_>, f: &mut XcfFile<P>, end: u64) -> Result<(), XcfError> {
    if r.u32()? != 1 {
        return Err(r.error(ErrorKind::Unsupported, "unknown legacy vectors version"));
    }
    let active = r.u32()? as usize;
    let n = r.u32()? as usize;
    count(r, n, 24, end)?;
    if n > r.limits.drawables {
        return Err(r.error(ErrorKind::Limit, "legacy path count"));
    }
    for i in 0..n {
        r.poll()?;
        let name = r.string()?;
        let tattoo = Some(r.u32()?);
        let visible = boolean(r)?;
        let legacy_linked = boolean(r)?;
        let m = r.u32()? as usize;
        let k = r.u32()? as usize;
        let mut p = VectorPath { name, tattoo, visible, legacy_linked, selected: i == active, ..Default::default() };
        count(r, m, 12, end)?;
        for _ in 0..m {
            parasite(r, &mut p, end)?;
        }
        strokes(r, &mut p, k, end)?;
        append(r, f, p)?;
    }
    Ok(())
}
pub(crate) fn legacy_paths<P>(r: &mut Reader<'_, '_>, f: &mut XcfFile<P>, end: u64) -> Result<(), XcfError> {
    let active = r.u32()? as usize;
    let n = r.u32()? as usize;
    count(r, n, 21, end)?;
    if n > r.limits.drawables {
        return Err(r.error(ErrorKind::Limit, "legacy path count"));
    }
    for i in 0..n {
        r.poll()?;
        let name = r.string()?;
        let legacy_linked = boolean(r)?;
        let state = r.take(1)?.first().copied().ok_or_else(|| r.error(ErrorKind::Truncated, "legacy path state"))?;
        if !matches!(state, 2 | 4) {
            return Err(r.error(ErrorKind::Unsupported, "unknown legacy path state"));
        }
        let closed = boolean(r)?;
        let np = r.u32()? as usize;
        let version = r.u32()?;
        if !(1..=3).contains(&version) {
            return Err(r.error(ErrorKind::Unsupported, "unknown legacy path version"));
        }
        if version >= 2 {
            let _dummy = r.u32()?;
        }
        let tattoo = if version == 3 { Some(r.u32()?) } else { None };
        count(r, np, 12, end)?;
        r.charge(np.checked_mul(std::mem::size_of::<PathPoint>() + 8).ok_or_else(|| r.error(ErrorKind::Limit, "legacy point allocation"))?)?;
        let mut points = Vec::new();
        points.try_reserve_exact(np).map_err(|_| r.error(ErrorKind::Allocation, "legacy path points"))?;
        for _ in 0..np {
            r.poll()?;
            let kind = r.u32()?;
            if kind > 2 {
                return Err(r.error(ErrorKind::Unsupported, "unknown legacy point type"));
            }
            let integers = if version == 1 { Some([r.i32()?, r.i32()?]) } else { None };
            let xy = if let Some([x, y]) = integers { [x as f32, y as f32] } else { [r.float()?, r.float()?] };
            let mut components = Vec::new();
            components.try_reserve_exact(2).map_err(|_| r.error(ErrorKind::Allocation, "legacy point components"))?;
            components.extend(xy.map(f32::to_bits));
            points.push(PathPoint { kind, legacy_integer_coordinates: integers, components });
        }
        let mut p =
            VectorPath { legacy_state: Some(state), legacy_format: Some(version), name, legacy_linked, tattoo, selected: i == active, ..Default::default() };
        if np > 0 {
            r.charge(std::mem::size_of::<PathStroke>())?;
            p.strokes.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "legacy strokes"))?;
            p.strokes.push(PathStroke { closed, points });
        }
        append(r, f, p)?;
    }
    Ok(())
}
