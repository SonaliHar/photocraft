//! Additive two-pass ICC parser. Legacy parser remains unchanged.
use super::*;
use crate::control::{Budget, Error, Limits, Result, invalid};
#[derive(Clone, Copy, Debug)]
pub struct ParseRequirements {
    pub input_bytes: usize,
    pub model_bytes: usize,
    pub peak_resident_bytes: usize,
    pub work_units: u64,
}
impl Profile {
    /// O(1) upper bound for this parser's requirements census, before inspecting
    /// hostile ICC records. This is an accounting bound, not a header/profile gate.
    /// Two whole-input charges + four named curve tags (<=2L work) + six LUTs
    /// (each <=3L CLUT work + three <=L/2 sequential curve groups) + two text
    /// tags (<=16L) give <=47L. <=1024 tag headers and fixed groups fit65536.
    /// Repeated/shared tag payloads are deliberately counted repeatedly. The
    /// default ceiling is never raised; the actual planner enforces the result.
    pub fn parse_planning_work_ceiling(input_bytes: usize) -> Result<u64> {
        let bound = u64::try_from(input_bytes)
            .map_err(|_| Error::Limit("ICC input work"))?
            .checked_mul(64)
            .and_then(|n| n.checked_add(65536))
            .ok_or(Error::Limit("ICC input work"))?;
        Ok(bound.min(Limits::default().work_units))
    }
}
fn add(a: usize, b: usize) -> Result<usize> {
    a.checked_add(b).ok_or(Error::Limit("ICC size addition"))
}
fn mul(a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b).ok_or(Error::Limit("ICC size product"))
}
fn at(off: usize, index: usize, stride: usize) -> Result<usize> {
    add(off, mul(index, stride)?)
}
struct Census<'a> {
    limits: Limits,
    cancelled: &'a dyn Fn() -> bool,
    model: usize,
    work: u64,
    input: usize,
}
impl Census<'_> {
    fn poll(&self) -> Result<()> {
        if (self.cancelled)() { Err(Error::Cancelled) } else { Ok(()) }
    }
    fn charge(&mut self, bytes: usize, work: usize) -> Result<()> {
        self.poll()?;
        self.model = add(self.model, bytes)?;
        self.work = self.work.checked_add(work as u64).ok_or(Error::Limit("ICC work product"))?;
        if add(mul(self.input, 2)?, self.model)? > self.limits.resident_bytes || self.work > self.limits.work_units {
            return Err(Error::Limit("ICC prospective storage/work"));
        }
        Ok(())
    }
}
struct Header<'a> {
    r: Reader<'a>,
    count: usize,
}
impl<'a> Header<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        let r = Reader { b: bytes };
        if bytes.len() < 132 {
            return Err(CmsError::Truncated.into());
        }
        if r.slice(36, 4)? != b"acsp" {
            return Err(CmsError::BadSignature.into());
        }
        let declared = r.u32(0)? as usize;
        let end = if declared >= 132 && declared <= bytes.len() { declared } else { bytes.len() };
        let r = Reader { b: bytes.get(..end).ok_or(CmsError::Truncated)? };
        let count = r.u32(128)? as usize;
        if count > 1024 {
            return Err(CmsError::Invalid(format!("{count} tags")).into());
        }
        r.slice(132, mul(count, 12)?)?;
        Ok(Self { r, count })
    }
    fn tag(&self, name: &[u8; 4], cancelled: &dyn Fn() -> bool) -> Result<Option<&'a [u8]>> {
        for i in 0..self.count {
            if i % 32 == 0 && (cancelled)() {
                return Err(Error::Cancelled);
            }
            let o = at(132, i, 12)?;
            if self.r.u32(o)? == sig(name) {
                let off = self.r.u32(add(o, 4)?)? as usize;
                let len = self.r.u32(add(o, 8)?)? as usize;
                return Ok(Some(self.r.slice(off, len)?));
            }
        }
        Ok(None)
    }
}
fn curve_cost(d: &[u8], c: &mut Census<'_>) -> Result<usize> {
    let r = Reader { b: d };
    match r.slice(0, 4)? {
        b"curv" => {
            let n = r.u32(8)? as usize;
            if n > 1 << 20 {
                return Err(CmsError::Invalid("curve too long".into()).into());
            }
            let length = add(12, mul(n, 2)?)?;
            match n {
                0 => {}
                1 => {
                    r.u16(12)?;
                }
                _ => {
                    r.slice(12, mul(n, 2)?)?;
                    c.charge(mul(n, 8)?, n)?;
                }
            }
            Ok(length)
        }
        b"para" => {
            let kind = r.u16(8)?;
            if kind > 4 {
                return Err(CmsError::Unsupported(format!("parametric curve {kind}")).into());
            }
            let count = Curve::param_count(kind);
            for i in 0..count {
                r.s15f16(at(12, i, 4)?)?;
            }
            Ok(add(12, mul(count, 4)?)?)
        }
        _ => Err(CmsError::Unsupported("not a tone curve".into()).into()),
    }
}
fn curves_cost(d: &[u8], off: usize, n: usize, c: &mut Census<'_>) -> Result<()> {
    c.charge(mul(n, std::mem::size_of::<Curve>())?, n)?;
    let mut pos = off;
    for _ in 0..n {
        c.poll()?;
        let tail = d.get(pos..).ok_or(CmsError::Truncated)?;
        let used = curve_cost(tail, c)?;
        pos = add(pos, add(used, 3)? & !3)?;
    }
    Ok(())
}
fn clut_cost(r: &Reader<'_>, offset: usize, grid: &[usize], outputs: usize, bps: usize, c: &mut Census<'_>) -> Result<usize> {
    let mut nodes = 1usize;
    for &g in grid {
        c.poll()?;
        if g == 0 {
            return Err(CmsError::Invalid("zero grid points".into()).into());
        }
        nodes = nodes.checked_mul(g).filter(|n| *n <= MAX_CLUT_VALUES).ok_or_else(|| CmsError::Invalid("CLUT too large".into()))?;
    }
    let total = nodes.checked_mul(outputs).filter(|n| *n <= MAX_CLUT_VALUES).ok_or_else(|| CmsError::Invalid("CLUT too large".into()))?;
    r.slice(offset, mul(total, bps)?)?;
    c.charge(add(mul(total, 12)?, mul(grid.len(), 2 * std::mem::size_of::<usize>())?)?, mul(total, 3)?)?;
    Ok(total)
}
fn lut_cost(d: &[u8], a2b: bool, c: &mut Census<'_>) -> Result<()> {
    let r = Reader { b: d };
    let ty = r.slice(0, 4)?;
    let i = r.u8(8)? as usize;
    let o = r.u8(9)? as usize;
    if !(1..=15).contains(&i) || !(1..=15).contains(&o) {
        return Err(CmsError::Invalid("bad LUT dimensions".into()).into());
    }
    c.charge(6 * std::mem::size_of::<Stage>() + 256, 64)?;
    match ty {
        b"mft1" | b"mft2" => {
            let wide = ty == b"mft2";
            let g = r.u8(10)? as usize;
            if g == 0 {
                return Err(CmsError::Invalid("bad lut dimensions".into()).into());
            }
            for k in 0..9 {
                r.s15f16(at(12, k, 4)?)?;
            }
            let (ni, no, mut off) = if wide { (r.u16(48)? as usize, r.u16(50)? as usize, 52) } else { (256, 256, 48) };
            if wide && (!(2..=4096).contains(&ni) || !(2..=4096).contains(&no)) {
                return Err(CmsError::Invalid("bad lut16 table size".into()).into());
            }
            let bps = if wide { 2 } else { 1 };
            let incoming = mul(i, ni)?;
            r.slice(off, mul(incoming, bps)?)?;
            c.charge(add(mul(incoming, 8)?, mul(i, std::mem::size_of::<Curve>())?)?, incoming)?;
            off = add(off, mul(incoming, bps)?)?;
            let grid = [g; 15];
            let total = clut_cost(&r, off, grid.get(..i).ok_or(Error::Limit("ICC grid"))?, o, bps, c)?;
            off = add(off, mul(total, bps)?)?;
            let outgoing = mul(o, no)?;
            r.slice(off, mul(outgoing, bps)?)?;
            c.charge(add(mul(outgoing, 8)?, mul(o, std::mem::size_of::<Curve>())?)?, outgoing)?;
        }
        b"mAB " | b"mBA " => {
            let (ob, om, omc, oc, oa) = (r.u32(12)? as usize, r.u32(16)? as usize, r.u32(20)? as usize, r.u32(24)? as usize, r.u32(28)? as usize);
            let clut = |c: &mut Census<'_>| -> Result<()> {
                let mut grid = [0usize; 15];
                for (k, v) in grid.get_mut(..i).ok_or(Error::Limit("ICC grid"))?.iter_mut().enumerate() {
                    *v = r.u8(add(oc, k)?)? as usize;
                }
                let prec = r.u8(add(oc, 16)?)? as usize;
                if !matches!(prec, 1 | 2) {
                    return Err(CmsError::Invalid(format!("CLUT precision {prec}")).into());
                }
                clut_cost(&r, add(oc, 20)?, grid.get(..i).ok_or(Error::Limit("ICC grid"))?, o, prec, c)?;
                Ok(())
            };
            if ty == b"mAB " {
                if oc != 0 {
                    if oa == 0 {
                        return Err(CmsError::Invalid("lutAtoB with CLUT but no A curves".into()).into());
                    }
                    curves_cost(d, oa, i, c)?;
                    clut(c)?;
                } else if i != o {
                    return Err(CmsError::Invalid("lutAtoB without CLUT changes channel count".into()).into());
                }
                if om != 0 {
                    curves_cost(d, omc, o, c)?;
                    r.slice(om, 48)?;
                }
                if ob == 0 {
                    return Err(CmsError::Invalid("lutAtoB without B curves".into()).into());
                }
                curves_cost(d, ob, o, c)?;
            } else {
                if ob == 0 {
                    return Err(CmsError::Invalid("lutBtoA without B curves".into()).into());
                }
                curves_cost(d, ob, i, c)?;
                if om != 0 {
                    r.slice(om, 48)?;
                    curves_cost(d, omc, i, c)?;
                }
                if oc != 0 {
                    if oa == 0 {
                        return Err(CmsError::Invalid("lutBtoA with CLUT but no A curves".into()).into());
                    }
                    clut(c)?;
                    curves_cost(d, oa, o, c)?;
                } else if i != o {
                    return Err(CmsError::Invalid("lutBtoA without CLUT changes channel count".into()).into());
                }
            }
        }
        other => return Err(CmsError::Unsupported(format!("LUT type {:?}", String::from_utf8_lossy(other))).into()),
    }
    let _ = a2b;
    Ok(())
}
fn text_slice<'a>(d: &'a [u8], cancelled: &dyn Fn() -> bool) -> Result<(&'a [u8], bool)> {
    let r = Reader { b: d };
    let Ok(ty) = r.slice(0, 4) else {
        return Ok((&[], false));
    };
    match ty {
        b"desc" => {
            let n = r.u32(8).unwrap_or(0) as usize;
            Ok((r.slice(12, n).unwrap_or(&[]), false))
        }
        b"text" => Ok((d.get(8..).unwrap_or(&[]), false)),
        b"mluc" => {
            let (Ok(n), Ok(size)) = (r.u32(8), r.u32(12)) else {
                return Ok((&[], true));
            };
            let mut best = None;
            for k in 0..(n as usize).min(256) {
                if (cancelled)() {
                    return Err(Error::Cancelled);
                }
                let Ok(off) = at(16, k, size as usize) else {
                    break;
                };
                let (Ok(lang), Ok(len), Ok(pos)) = (r.u16(off), add(off, 4).and_then(|o| Ok(r.u32(o)?)), add(off, 8).and_then(|o| Ok(r.u32(o)?))) else {
                    break;
                };
                if best.is_none() || lang == u16::from_be_bytes(*b"en") {
                    best = Some((pos as usize, len as usize));
                    if lang == u16::from_be_bytes(*b"en") {
                        break;
                    }
                }
            }
            let Some((off, len)) = best else {
                return Ok((&[], true));
            };
            Ok((r.slice(off, len).unwrap_or(&[]), true))
        }
        _ => Ok((&[], false)),
    }
}
pub(super) fn requirements(bytes: &[u8], limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<ParseRequirements> {
    let _ = Budget::new(limits, cancelled)?;
    let mut c = Census { limits, cancelled, model: 0, work: 0, input: bytes.len() };
    c.charge(4096, bytes.len())?;
    let h = Header::new(bytes)?;
    c.charge(mul(h.count, std::mem::size_of::<(u32, usize, usize)>())?, h.count * 32)?;
    // Required XYZ tags use the legacy semantics; bkpt/chad remain optional.
    for name in [b"wtpt", b"rXYZ", b"gXYZ", b"bXYZ"] {
        if let Some(d) = h.tag(name, cancelled)? {
            if d.len() < 20 || d.get(..4) != Some(b"XYZ ") {
                return Err(CmsError::Invalid(format!("tag {} is not XYZType", String::from_utf8_lossy(name))).into());
            }
            Reader { b: d }.xyz(8)?;
        }
    }
    if let Some(d) = h.tag(b"chad", cancelled)?
        && d.len() >= 44
        && d.get(..4) == Some(b"sf32")
    {
        let r = Reader { b: d };
        for i in 0..9 {
            r.s15f16(at(8, i, 4)?)?;
        }
    }
    // A malformed optional black point is ignored by the legacy reader, but
    // cancellation from its lookup is never converted into an absent record.
    if let Err(Error::Cancelled) = h.tag(b"bkpt", cancelled) {
        return Err(Error::Cancelled);
    }
    for name in [b"rTRC", b"gTRC", b"bTRC", b"kTRC"] {
        if let Some(d) = h.tag(name, cancelled)? {
            curve_cost(d, &mut c)?;
        }
    }
    let device = ColorSpace::from_sig(h.r.u32(16)?).channels();
    let class = ProfileClass::from_sig(h.r.u32(12)?);
    let connection = if class == ProfileClass::DeviceLink { ColorSpace::from_sig(h.r.u32(20)?).channels() } else { 3 };
    for (name, input) in [(b"A2B0", true), (b"A2B1", true), (b"A2B2", true), (b"B2A0", false), (b"B2A1", false), (b"B2A2", false)] {
        if let Some(d) = h.tag(name, cancelled)? {
            lut_cost(d, input, &mut c)?;
            let reader = Reader { b: d };
            let i = reader.u8(8)? as usize;
            let o = reader.u8(9)? as usize;
            let (expected_i, expected_o) = if input { (device, connection) } else { (connection, device) };
            if (expected_i != 0 && i != expected_i) || (expected_o != 0 && o != expected_o) {
                return Err(invalid("LUT channels disagree with profile header"));
            }
        }
    }
    for name in [b"desc", b"cprt"] {
        if let Some(d) = h.tag(name, cancelled)? {
            let (text, _) = text_slice(d, cancelled)?;
            c.charge(mul(text.len(), 6)?, mul(text.len(), 8)?)?;
        }
    }
    c.charge(0, bytes.len())?;
    Ok(ParseRequirements { input_bytes: bytes.len(), model_bytes: c.model, peak_resident_bytes: add(mul(bytes.len(), 2)?, c.model)?, work_units: c.work })
}

fn parse_curve_controlled(d: &[u8], b: &mut Budget<'_>) -> Result<(Curve, usize)> {
    b.poll()?;
    let r = Reader { b: d };
    match r.slice(0, 4)? {
        b"curv" => {
            let n = r.u32(8)? as usize;
            if n > 1 << 20 {
                return Err(CmsError::Invalid("curve too long".into()).into());
            }
            let length = add(12, mul(n, 2)?)?;
            match n {
                0 => Ok((Curve::Identity, length)),
                1 => Ok((Curve::Gamma(f64::from(r.u16(12)?) / 256.0), length)),
                _ => {
                    let raw = r.slice(12, mul(n, 2)?)?;
                    let mut values = b.reserve(n)?;
                    for chunk in raw.chunks(2048) {
                        b.work((chunk.len() / 2) as u64)?;
                        for &bytes in chunk.as_chunks::<2>().0 {
                            values.push(f64::from(u16::from_be_bytes(bytes)) / 65535.0);
                        }
                    }
                    Ok((Curve::Table64(values), length))
                }
            }
        }
        b"para" => {
            let kind = r.u16(8)?;
            if kind > 4 {
                return Err(CmsError::Unsupported(format!("parametric curve {kind}")).into());
            }
            let n = Curve::param_count(kind);
            let mut p = [0f64; 7];
            for (i, value) in p.iter_mut().enumerate().take(n) {
                b.poll()?;
                *value = r.s15f16(at(12, i, 4)?)?;
            }
            Ok((if kind == 0 { Curve::Gamma(p[0]) } else { Curve::Parametric { kind, p } }, add(12, mul(n, 4)?)?))
        }
        _ => Err(CmsError::Unsupported("not a tone curve".into()).into()),
    }
}
fn parse_curves_controlled(d: &[u8], off: usize, n: usize, b: &mut Budget<'_>) -> Result<Vec<Curve>> {
    let mut out = b.reserve(n)?;
    let mut pos = off;
    for _ in 0..n {
        b.poll()?;
        let tail = d.get(pos..).ok_or(CmsError::Truncated)?;
        let (curve, used) = parse_curve_controlled(tail, b)?;
        out.push(curve);
        pos = add(pos, add(used, 3)? & !3)?;
    }
    Ok(out)
}
fn values(raw: &[u8], wide: bool, b: &mut Budget<'_>) -> Result<Vec<f64>> {
    let length = if wide { raw.len() / 2 } else { raw.len() };
    let mut values = b.reserve(length)?;
    if wide {
        for chunk in raw.chunks(2048) {
            b.work((chunk.len() / 2) as u64)?;
            for &bytes in chunk.as_chunks::<2>().0 {
                values.push(f64::from(u16::from_be_bytes(bytes)) / 65535.0);
            }
        }
    } else {
        for chunk in raw.chunks(1024) {
            b.work(chunk.len() as u64)?;
            for &value in chunk {
                values.push(f64::from(value) / 255.0);
            }
        }
    }
    Ok(values)
}
fn tables(r: &Reader<'_>, off: &mut usize, count: usize, length: usize, wide: bool, b: &mut Budget<'_>) -> Result<Vec<Curve>> {
    let mut curves = b.reserve(count)?;
    let bytes = mul(length, if wide { 2 } else { 1 })?;
    for _ in 0..count {
        b.poll()?;
        let raw = r.slice(*off, bytes)?;
        curves.push(Curve::Table64(values(raw, wide, b)?));
        *off = add(*off, bytes)?;
    }
    Ok(curves)
}
fn matrix(r: &Reader<'_>, off: usize, b: &mut Budget<'_>) -> Result<Stage> {
    let mut m = b.reserve(9)?;
    for i in 0..9 {
        m.push(r.s15f16(at(off, i, 4)?)?);
    }
    let mut offset = b.reserve(3)?;
    for i in 0..3 {
        offset.push(r.s15f16(at(add(off, 36)?, i, 4)?)?);
    }
    Ok(Stage::Matrix { rows: 3, cols: 3, m, offset })
}
fn clut(r: &Reader<'_>, off: usize, inputs: usize, outputs: usize, b: &mut Budget<'_>) -> Result<Stage> {
    let mut grid = b.reserve(inputs)?;
    let mut count = outputs;
    for i in 0..inputs {
        b.poll()?;
        let g = r.u8(add(off, i)?)? as usize;
        if g == 0 {
            return Err(CmsError::Invalid("zero grid points".into()).into());
        }
        count = count.checked_mul(g).filter(|n| *n <= MAX_CLUT_VALUES).ok_or_else(|| CmsError::Invalid("CLUT too large".into()))?;
        grid.push(g);
    }
    let precision = r.u8(add(off, 16)?)?;
    if !matches!(precision, 1 | 2) {
        return Err(CmsError::Invalid(format!("CLUT precision {precision}")).into());
    }
    let raw = r.slice(add(off, 20)?, mul(count, precision as usize)?)?;
    let data = values(raw, precision == 2, b)?;
    Ok(Stage::Clut(Clut::from_f64_controlled(grid, outputs, data, b)?))
}
fn parse_lut_controlled(d: &[u8], a2b: bool, b: &mut Budget<'_>) -> Result<Lut> {
    b.poll()?;
    let r = Reader { b: d };
    let ty = r.slice(0, 4)?;
    let i = r.u8(8)? as usize;
    let o = r.u8(9)? as usize;
    if !(1..=15).contains(&i) || !(1..=15).contains(&o) {
        return Err(CmsError::Invalid("bad LUT dimensions".into()).into());
    }
    let mut stages = b.reserve(6)?;
    match ty {
        b"mft1" | b"mft2" => {
            let wide = ty == b"mft2";
            let g = r.u8(10)? as usize;
            if g == 0 {
                return Err(CmsError::Invalid("bad lut dimensions".into()).into());
            }
            let mut m = [[0f64; 3]; 3];
            for (a, row) in m.iter_mut().enumerate() {
                for (k, v) in row.iter_mut().enumerate() {
                    *v = r.s15f16(at(12, a * 3 + k, 4)?)?;
                }
            }
            let (ni, no, mut off) = if wide { (r.u16(48)? as usize, r.u16(50)? as usize, 52) } else { (256, 256, 48) };
            if wide && (!(2..=4096).contains(&ni) || !(2..=4096).contains(&no)) {
                return Err(CmsError::Invalid("bad lut16 table size".into()).into());
            }
            let incoming = tables(&r, &mut off, i, ni, wide, b)?;
            let mut count = o;
            let mut grid = b.reserve(i)?;
            for _ in 0..i {
                count = count.checked_mul(g).filter(|n| *n <= MAX_CLUT_VALUES).ok_or_else(|| CmsError::Invalid("CLUT too large".into()))?;
                grid.push(g);
            }
            let bps = if wide { 2 } else { 1 };
            let raw = r.slice(off, mul(count, bps)?)?;
            let data = values(raw, wide, b)?;
            off = add(off, mul(count, bps)?)?;
            let outgoing = tables(&r, &mut off, o, no, wide, b)?;
            let ident = m.iter().enumerate().all(|(a, row)| row.iter().enumerate().all(|(k, v)| (v - if a == k { 1.0 } else { 0.0 }).abs() < 1e-6));
            if !a2b && i == 3 && !ident {
                let mut flat = b.reserve(9)?;
                for value in m.iter().flatten() {
                    flat.push(*value);
                }
                let mut offset = b.reserve(3)?;
                offset.resize(3, 0.0);
                stages.push(Stage::Matrix { rows: 3, cols: 3, m: flat, offset });
            }
            stages.push(Stage::Curves(incoming));
            stages.push(Stage::Clut(Clut::from_f64_controlled(grid, o, data, b)?));
            stages.push(Stage::Curves(outgoing));
            Ok(Lut { kind: if wide { LutKind::Lut16 } else { LutKind::Lut8 }, inputs: i, outputs: o, stages })
        }
        b"mAB " | b"mBA " => {
            let (ob, om, omc, oc, oa) = (r.u32(12)? as usize, r.u32(16)? as usize, r.u32(20)? as usize, r.u32(24)? as usize, r.u32(28)? as usize);
            if ty == b"mAB " {
                if oc != 0 {
                    if oa == 0 {
                        return Err(CmsError::Invalid("lutAtoB with CLUT but no A curves".into()).into());
                    }
                    stages.push(Stage::Curves(parse_curves_controlled(d, oa, i, b)?));
                    stages.push(clut(&r, oc, i, o, b)?);
                } else if i != o {
                    return Err(CmsError::Invalid("lutAtoB without CLUT changes channel count".into()).into());
                }
                if om != 0 {
                    stages.push(Stage::Curves(parse_curves_controlled(d, omc, o, b)?));
                    stages.push(matrix(&r, om, b)?);
                }
                if ob == 0 {
                    return Err(CmsError::Invalid("lutAtoB without B curves".into()).into());
                }
                stages.push(Stage::Curves(parse_curves_controlled(d, ob, o, b)?));
            } else {
                if ob == 0 {
                    return Err(CmsError::Invalid("lutBtoA without B curves".into()).into());
                }
                stages.push(Stage::Curves(parse_curves_controlled(d, ob, i, b)?));
                if om != 0 {
                    stages.push(matrix(&r, om, b)?);
                    stages.push(Stage::Curves(parse_curves_controlled(d, omc, i, b)?));
                }
                if oc != 0 {
                    if oa == 0 {
                        return Err(CmsError::Invalid("lutBtoA with CLUT but no A curves".into()).into());
                    }
                    stages.push(clut(&r, oc, i, o, b)?);
                    stages.push(Stage::Curves(parse_curves_controlled(d, oa, o, b)?));
                } else if i != o {
                    return Err(CmsError::Invalid("lutBtoA without CLUT changes channel count".into()).into());
                }
            }
            Ok(Lut { kind: LutKind::Ab, inputs: i, outputs: o, stages })
        }
        other => Err(CmsError::Unsupported(format!("LUT type {:?}", String::from_utf8_lossy(other))).into()),
    }
}

fn string_capacity(count: usize, b: &mut Budget<'_>) -> Result<String> {
    b.admit(count, 0)?;
    let mut out = String::new();
    out.try_reserve_exact(count).map_err(|_| Error::Allocation)?;
    b.retained(out.capacity())?;
    Ok(out)
}
fn append(out: &mut String, text: &str, b: &mut Budget<'_>) -> Result<()> {
    let mut start = 0;
    for (i, _) in text.char_indices() {
        if i - start >= 1024 {
            let part = text.get(start..i).ok_or(Error::Limit("ICC text chunk"))?;
            b.work(part.len() as u64)?;
            out.push_str(part);
            start = i;
        }
    }
    let tail = text.get(start..).ok_or(Error::Limit("ICC text chunk"))?;
    b.work(tail.len() as u64)?;
    out.push_str(tail);
    Ok(())
}
fn parse_text_controlled(d: &[u8], b: &mut Budget<'_>) -> Result<String> {
    let (raw, utf16) = text_slice(d, b.cancelled)?;
    let mut text = string_capacity(mul(raw.len(), 3)?, b)?;
    if utf16 {
        let units = raw.as_chunks::<2>().0.iter().copied().map(u16::from_be_bytes);
        for (i, ch) in char::decode_utf16(units).enumerate() {
            if i % 256 == 0 {
                b.poll()?;
            }
            let ch = ch.unwrap_or(char::REPLACEMENT_CHARACTER);
            let mut word = [0u8; 4];
            append(&mut text, ch.encode_utf8(&mut word), b)?;
        }
    } else {
        let mut end = raw.len();
        for (i, &value) in raw.iter().enumerate() {
            if i % 1024 == 0 {
                b.poll()?;
            }
            if value == 0 {
                end = i;
                break;
            }
        }
        let mut remaining = raw.get(..end).ok_or(Error::Limit("ICC text slice"))?;
        while !remaining.is_empty() {
            b.poll()?;
            let take = remaining.len().min(4096);
            let chunk = remaining.get(..take).ok_or(Error::Limit("ICC UTF8 chunk"))?;
            match std::str::from_utf8(chunk) {
                Ok(valid) => {
                    append(&mut text, valid, b)?;
                    remaining = remaining.get(take..).ok_or(Error::Limit("ICC UTF8 suffix"))?;
                }
                Err(error) => {
                    let good = error.valid_up_to();
                    let prefix =
                        std::str::from_utf8(chunk.get(..good).ok_or(Error::Limit("ICC UTF8 prefix"))?).map_err(|_| invalid("ICC valid UTF8 prefix"))?;
                    append(&mut text, prefix, b)?;
                    if error.error_len().is_none() && take < remaining.len() {
                        if good == 0 {
                            return Err(invalid("ICC UTF8 chunk made no progress"));
                        }
                        remaining = remaining.get(good..).ok_or(Error::Limit("ICC UTF8 suffix"))?;
                        continue;
                    }
                    append(&mut text, "�", b)?;
                    let skip = add(good, error.error_len().unwrap_or(remaining.len() - good))?;
                    remaining = remaining.get(skip..).ok_or(Error::Limit("ICC UTF8 suffix"))?;
                }
            }
        }
    }
    let mut tail = text.len();
    if utf16 {
        while text.as_bytes().get(tail.saturating_sub(1)) == Some(&0) && tail > 0 {
            if tail % 1024 == 0 {
                b.poll()?;
            }
            tail -= 1;
        }
    }
    let text = text.get(..tail).ok_or(Error::Limit("ICC text boundary"))?;
    let mut start = None;
    let mut end = 0;
    for (i, ch) in text.char_indices() {
        b.work(1)?;
        if !ch.is_whitespace() {
            if start.is_none() {
                start = Some(i);
            }
            end = add(i, ch.len_utf8())?;
        }
    }
    let Some(start) = start else {
        return Ok(String::new());
    };
    let trimmed = text.get(start..end).ok_or(Error::Limit("ICC trimmed text"))?;
    let mut output = string_capacity(trimmed.len(), b)?;
    append(&mut output, trimmed, b)?;
    Ok(output)
}
fn parse_model(bytes: &[u8], budget: &mut Budget<'_>) -> Result<Profile> {
    let r = Reader { b: bytes };
    if bytes.len() < 132 {
        return Err(CmsError::Truncated.into());
    }
    if r.slice(36, 4)? != b"acsp" {
        return Err(CmsError::BadSignature.into());
    }
    let declared = r.u32(0)? as usize;
    // Some writers pad or truncate; trust the smaller of the two.
    let end = if declared >= 132 && declared <= bytes.len() { declared } else { bytes.len() };
    let r = Reader { b: bytes.get(..end).ok_or(CmsError::Truncated)? };
    let version = (r.u8(8)?, r.u8(9)?);
    let class = ProfileClass::from_sig(r.u32(12)?);
    let color_space = ColorSpace::from_sig(r.u32(16)?);
    let pcs_sig = r.u32(20)?;
    let pcs = match &pcs_sig.to_be_bytes() {
        b"Lab " => Pcs::Lab,
        _ => Pcs::Xyz,
    };
    let rendering_intent = Intent::from_u32(r.u32(64)? & 0xFFFF).unwrap_or(Intent::Perceptual);
    let count = r.u32(128)? as usize;
    if count > 1024 {
        return Err(CmsError::Invalid(format!("{count} tags")).into());
    }
    let mut tags: Vec<(u32, usize, usize)> = budget.reserve(count)?;
    for i in 0..count {
        budget.poll()?;
        let o = 132 + i * 12;
        let (s, off, len) = (r.u32(o)?, r.u32(o + 4)? as usize, r.u32(o + 8)? as usize);
        tags.push((s, off, len));
    }
    let find = |s: &[u8; 4]| tags.iter().find(|t| t.0 == sig(s)).map(|t| (t.1, t.2));
    let tag_data = |s: &[u8; 4]| -> Result<Option<&[u8]>> {
        match find(s) {
            Some((off, len)) => Ok(Some(r.slice(off, len)?)),
            None => Ok(None),
        }
    };

    let xyz_tag = |s: &[u8; 4]| -> Result<Option<[f64; 3]>> {
        match tag_data(s)? {
            Some(d) if d.len() >= 20 && d.get(..4) == Some(b"XYZ ") => Ok(Some(Reader { b: d }.xyz(8)?)),
            Some(_) => Err(CmsError::Invalid(format!("tag {} is not XYZType", String::from_utf8_lossy(s))).into()),
            None => Ok(None),
        }
    };
    let mut curve_tag = |s: &[u8; 4]| -> Result<Option<Curve>> {
        match tag_data(s)? {
            Some(d) => Ok(Some(parse_curve_controlled(d, budget)?.0)),
            None => Ok(None),
        }
    };

    let white_point = xyz_tag(b"wtpt")?.unwrap_or(math::D50);
    let black_point = xyz_tag(b"bkpt").ok().flatten();
    let chad = match tag_data(b"chad")? {
        Some(d) if d.len() >= 44 && d.get(..4) == Some(b"sf32") => {
            let rd = Reader { b: d };
            let mut m = [[0.0; 3]; 3];
            for (i, row) in m.iter_mut().enumerate() {
                for (j, v) in row.iter_mut().enumerate() {
                    *v = rd.s15f16(8 + (i * 3 + j) * 4)?;
                }
            }
            Some(m)
        }
        _ => None,
    };
    let matrix = match (xyz_tag(b"rXYZ")?, xyz_tag(b"gXYZ")?, xyz_tag(b"bXYZ")?) {
        (Some(rr), Some(g), Some(b)) => Some([[rr[0], g[0], b[0]], [rr[1], g[1], b[1]], [rr[2], g[2], b[2]]]),
        _ => None,
    };
    let trc = match (curve_tag(b"rTRC")?, curve_tag(b"gTRC")?, curve_tag(b"bTRC")?) {
        (Some(a), Some(b), Some(c)) => Some([a, b, c]),
        _ => None,
    };
    let gray_trc = curve_tag(b"kTRC")?;
    let mut lut = |s: &[u8; 4], a2b: bool| -> Result<Option<Lut>> {
        match tag_data(s)? {
            Some(d) => parse_lut_controlled(d, a2b, budget).map(Some),
            None => Ok(None),
        }
    };
    let a2b = [lut(b"A2B0", true)?, lut(b"A2B1", true)?, lut(b"A2B2", true)?];
    let b2a = [lut(b"B2A0", false)?, lut(b"B2A1", false)?, lut(b"B2A2", false)?];
    let description = match tag_data(b"desc")? {
        Some(d) => parse_text_controlled(d, budget)?,
        None => String::new(),
    };
    let copyright = match tag_data(b"cprt")? {
        Some(d) => parse_text_controlled(d, budget)?,
        None => String::new(),
    };

    let p = Profile {
        version,
        class,
        color_space,
        pcs,
        rendering_intent,
        description,
        copyright,
        white_point,
        black_point,
        chad,
        matrix,
        trc,
        gray_trc,
        a2b,
        b2a,
        bytes: None,
        hash: Default::default(),
    };
    // Validate channel counts of LUTs against the header. A DeviceLink connects two device
    // spaces: its header PCS field holds the output colour space (ICC.1:2010 §7.2.7), so a
    // CMYK→CMYK link has a 4→4 AToB0.
    let dev = p.color_space.channels();
    let conn = if p.class == ProfileClass::DeviceLink { ColorSpace::from_sig(pcs_sig).channels() } else { 3 };
    for l in p.a2b.iter().flatten() {
        budget.poll()?;
        if dev != 0 && l.inputs != dev || conn != 0 && l.outputs != conn {
            return Err(CmsError::Invalid(format!("AToB tag has {}→{} channels for a {:?} profile", l.inputs, l.outputs, p.color_space)).into());
        }
    }
    for l in p.b2a.iter().flatten() {
        budget.poll()?;
        if conn != 0 && l.inputs != conn || dev != 0 && l.outputs != dev {
            return Err(CmsError::Invalid(format!("BToA tag has {}→{} channels for a {:?} profile", l.inputs, l.outputs, p.color_space)).into());
        }
    }
    Ok(p)
}

pub(super) fn parse(bytes: &[u8], limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<Profile> {
    let required = requirements(bytes, limits, cancelled)?;
    let mut b = Budget::new(limits, cancelled)?;
    b.retained(required.input_bytes)?;
    let mut profile = parse_model(bytes, &mut b)?;
    profile.bytes = Some(Arc::new(b.copy(bytes)?));
    b.poll()?;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn word(out: &mut Vec<u8>, offset: usize, value: u32) {
        if out.len() < offset + 4 {
            out.resize(offset + 4, 0);
        }
        out[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }
    fn icc(tags: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![0u8; 132 + tags.len() * 12];
        word(&mut out, 8, 0x04300000);
        out[12..16].copy_from_slice(b"scnr");
        out[16..20].copy_from_slice(b"GRAY");
        out[20..24].copy_from_slice(b"XYZ ");
        out[36..40].copy_from_slice(b"acsp");
        word(&mut out, 64, 1);
        word(&mut out, 128, tags.len() as u32);
        for (i, (name, data)) in tags.iter().enumerate() {
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
            let offset = out.len();
            out.extend_from_slice(data);
            let row = 132 + i * 12;
            out[row..row + 4].copy_from_slice(name);
            word(&mut out, row + 4, offset as u32);
            word(&mut out, row + 8, data.len() as u32);
        }
        let length = out.len();
        word(&mut out, 0, length as u32);
        out
    }
    fn identity() -> Vec<u8> {
        let mut out = vec![0; 12];
        out[..4].copy_from_slice(b"curv");
        out
    }
    fn mft(wide: bool) -> Vec<u8> {
        let mut out = vec![0u8; if wide { 52 } else { 48 }];
        out[..4].copy_from_slice(if wide { b"mft2" } else { b"mft1" });
        out[8] = 1;
        out[9] = 3;
        out[10] = 2;
        for i in [0, 4, 8] {
            word(&mut out, 12 + i * 4, 65536);
        }
        if wide {
            out[48..50].copy_from_slice(&2u16.to_be_bytes());
            out[50..52].copy_from_slice(&2u16.to_be_bytes());
            for value in [0u16, u16::MAX] {
                out.extend(value.to_be_bytes());
            }
            for value in [0u16, 0, 0, u16::MAX, u16::MAX, u16::MAX] {
                out.extend(value.to_be_bytes());
            }
            for _ in 0..3 {
                out.extend(0u16.to_be_bytes());
                out.extend(u16::MAX.to_be_bytes());
            }
        } else {
            out.extend(0..=255u8);
            out.extend([0, 0, 0, 255, 255, 255]);
            for _ in 0..3 {
                out.extend(0..=255u8);
            }
        }
        out
    }
    fn ab(reverse: bool) -> Vec<u8> {
        let mut out = vec![0u8; 108];
        out[..4].copy_from_slice(if reverse { b"mBA " } else { b"mAB " });
        out[8] = if reverse { 3 } else { 1 };
        out[9] = if reverse { 1 } else { 3 };
        word(&mut out, 12, 32);
        word(&mut out, 24, 68);
        word(&mut out, 28, 96);
        for i in 0..3 {
            out[32 + i * 12..44 + i * 12].copy_from_slice(&identity());
        }
        out[68] = 2;
        if reverse {
            out[69] = 2;
            out[70] = 2;
        }
        out[84] = 1;
        if reverse {
            out[88..96].copy_from_slice(&[0, 36, 72, 108, 144, 180, 216, 255]);
        } else {
            out[88..94].copy_from_slice(&[0, 0, 0, 255, 255, 255]);
        }
        out[96..108].copy_from_slice(&identity());
        out
    }
    #[test]
    fn old_parser_oracle_matches_all_existing_lut_families_and_raw_tail() {
        for (name, tag) in [(mft(false), *b"A2B0"), (mft(true), *b"A2B0"), (ab(false), *b"A2B0"), (ab(true), *b"B2A0")] {
            let mut raw = icc(&[(tag, name)]);
            raw.extend(b"retained original tail");
            let old = Profile::parse(&raw).unwrap();
            let controlled = parse(&raw, Limits::default(), &|| false).unwrap();
            assert_eq!(controlled, old);
            assert_eq!(controlled.encoded_bytes(), Some(raw.as_slice()));
        }
        for builtin in [crate::Builtin::Srgb, crate::Builtin::SGray, crate::Builtin::CoatedCmyk] {
            let raw = builtin.profile().to_bytes();
            assert_eq!(parse(&raw, Limits::default(), &|| false).unwrap(), Profile::parse(&raw).unwrap());
        }
    }
    #[test]
    fn text_first_tag_optional_fallback_and_parametric_zero_are_preserved() {
        let mut gamma = vec![0u8; 16];
        gamma[..4].copy_from_slice(b"para");
        word(&mut gamma, 12, 131072);
        let text = |value: &[u8]| {
            let mut out = b"text\0\0\0\0".to_vec();
            out.extend(value);
            out
        };
        let raw = icc(&[
            (*b"desc", text(b"  first \xff name \0ignored")),
            (*b"desc", text(b"second")),
            (*b"cprt", text(" © λ ".as_bytes())),
            (*b"kTRC", gamma),
            (*b"bkpt", vec![0; 4]),
            (*b"chad", vec![0; 4]),
        ]);
        let old = Profile::parse(&raw).unwrap();
        let controlled = parse(&raw, Limits::default(), &|| false).unwrap();
        assert_eq!(old, controlled);
        assert!(matches!(controlled.gray_trc, Some(Curve::Gamma(2.0))));
        assert_eq!(controlled.description, "first � name");
    }
    #[test]
    fn budget_cancellation_and_malformed_records_never_publish_or_mutate_input() {
        let mut raw = icc(&[(*b"A2B0", mft(true))]);
        let before = raw.clone();
        assert!(matches!(parse(&raw, Limits { resident_bytes: 1, work_units: 1 }, &|| false), Err(Error::Limit(_))));
        assert!(matches!(parse(&raw, Limits::default(), &|| true), Err(Error::Cancelled)));
        assert_eq!(raw, before);
        word(&mut raw, 128, 1025);
        assert!(matches!(requirements(&raw, Limits::default(), &|| false), Err(Error::Cms(_))));
        let raw = icc(&[(*b"A2B0", mft(false))]);
        let ticks = std::cell::Cell::new(0);
        let cancelled = || {
            let n = ticks.get();
            ticks.set(n + 1);
            n > 30
        };
        assert!(matches!(parse(&raw, Limits::default(), &cancelled), Err(Error::Cancelled)));
    }
}

#[cfg(test)]
mod planning_ceiling_tests {
    use super::*;
    #[test]
    fn original_parser_requirements_fit_input_derived_ceiling_without_header_policy_change() {
        for builtin in [crate::Builtin::Srgb, crate::Builtin::SGray, crate::Builtin::CoatedCmyk, crate::Builtin::LabD50] {
            let bytes = builtin.profile().to_bytes();
            let before = bytes.clone();
            let ceiling = Profile::parse_planning_work_ceiling(bytes.len()).unwrap();
            let lower = Limits { work_units: ceiling, ..Limits::default() };
            let a = requirements(&bytes, Limits::default(), &|| false).unwrap();
            let z = requirements(&bytes, lower, &|| false).unwrap();
            assert_eq!(a.work_units, z.work_units);
            assert_eq!(a.model_bytes, z.model_bytes);
            assert!(z.work_units <= ceiling);
            assert_eq!(bytes, before);
        }
    }
    #[test]
    fn lower_work_and_cancellation_are_still_actual_planner_errors() {
        let bytes = crate::Builtin::Srgb.profile().to_bytes();
        assert!(requirements(&bytes, Limits { work_units: 0, ..Limits::default() }, &|| false).is_err());
        assert!(matches!(
            requirements(&bytes, Limits { work_units: Profile::parse_planning_work_ceiling(bytes.len()).unwrap(), ..Limits::default() }, &|| true),
            Err(Error::Cancelled)
        ));
        assert_eq!(Profile::parse_planning_work_ceiling(16 << 20).unwrap(), Limits::default().work_units);
    }
}
