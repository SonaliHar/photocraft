//! Cooperative ICC writer. The borrowed count pass and materializing pass use
//! the same recipe; legacy tag order, quantization and omission are preserved.
use super::*;
use crate::control::{Budget, Error, Limits, Result, invalid};
#[derive(Clone, Copy, Debug)]
pub struct EncodeRequirements {
    pub retained_bytes: usize,
    pub encoded_bytes_upper_bound: usize,
    pub peak_resident_bytes: usize,
    pub work_units: u64,
}
fn add(a: usize, b: usize) -> Result<usize> {
    a.checked_add(b).ok_or(Error::Limit("ICC writer length"))
}
fn mul(a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b).ok_or(Error::Limit("ICC writer size"))
}
fn u32n(n: usize) -> Result<u32> {
    u32::try_from(n).map_err(|_| Error::Limit("ICC u32 offset/count"))
}
fn u8n(n: usize) -> Result<u8> {
    u8::try_from(n).map_err(|_| invalid("ICC channel/grid byte"))
}
struct Sink {
    bytes: Option<Vec<u8>>,
    len: usize,
    work: u64,
}
impl Sink {
    fn count() -> Self {
        Self { bytes: None, len: 0, work: 0 }
    }
    fn allocated(size: usize, b: &mut Budget<'_>) -> Result<Self> {
        Ok(Self { bytes: Some(b.reserve(size)?), len: 0, work: 0 })
    }
    fn charge(&mut self, n: u64, b: &mut Budget<'_>) -> Result<()> {
        self.work = self.work.checked_add(n).ok_or(Error::Limit("ICC recipe work"))?;
        b.work(n)
    }
    fn put(&mut self, s: &[u8], b: &mut Budget<'_>) -> Result<()> {
        let end = add(self.len, s.len())?;
        if let Some(v) = &self.bytes
            && end > v.capacity()
        {
            return Err(Error::Limit("ICC counted allocation"));
        }
        for part in s.chunks(1024) {
            self.charge(part.len() as u64, b)?;
            if let Some(v) = &mut self.bytes {
                v.extend_from_slice(part);
            }
        }
        self.len = end;
        Ok(())
    }
    fn pad(&mut self, b: &mut Budget<'_>) -> Result<()> {
        while !self.len.is_multiple_of(4) {
            self.put(&[0], b)?;
        }
        Ok(())
    }
    fn finish(self) -> Result<Vec<u8>> {
        self.bytes.ok_or_else(|| invalid("ICC count sink"))
    }
}
fn curve_valid(c: &Curve, b: &Budget<'_>) -> Result<()> {
    b.poll()?;
    match c {
        Curve::Parametric { kind, .. } if *kind > 4 => Err(invalid("ICC parametric curve kind")),
        Curve::Table(t) if t.len() > 1 << 20 => Err(Error::Limit("ICC curve samples")),
        Curve::Table64(t) if t.len() > 1 << 20 => Err(Error::Limit("ICC curve samples")),
        _ => Ok(()),
    }
}
fn curve(c: &Curve, o: &mut Sink, b: &mut Budget<'_>) -> Result<()> {
    curve_valid(c, b)?;
    match c {
        Curve::Identity => {
            o.put(b"curv\0\0\0\0", b)?;
            o.put(&0u32.to_be_bytes(), b)?;
        }
        Curve::Gamma(g) => {
            o.put(b"para\0\0\0\0", b)?;
            o.put(&[0; 4], b)?;
            o.put(&s15(*g), b)?;
        }
        Curve::Parametric { kind, p } => {
            o.put(b"para\0\0\0\0", b)?;
            o.put(&kind.to_be_bytes(), b)?;
            o.put(&[0; 2], b)?;
            for v in p.iter().take(Curve::param_count(*kind)) {
                o.put(&s15(*v), b)?;
            }
        }
        Curve::Table(t) => {
            o.put(b"curv\0\0\0\0", b)?;
            o.put(&u32n(t.len())?.to_be_bytes(), b)?;
            for v in t {
                o.put(&u16q(*v).to_be_bytes(), b)?;
            }
        }
        Curve::Table64(t) => {
            o.put(b"curv\0\0\0\0", b)?;
            o.put(&u32n(t.len())?.to_be_bytes(), b)?;
            for v in t {
                o.put(&((v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_be_bytes(), b)?;
            }
        }
    }
    Ok(())
}
fn curves(cs: &[Curve], o: &mut Sink, b: &mut Budget<'_>) -> Result<()> {
    for c in cs {
        curve(c, o, b)?;
        o.pad(b)?;
    }
    Ok(())
}
fn sampled(c: &Curve, n: usize, wide: bool, o: &mut Sink, b: &mut Budget<'_>) -> Result<()> {
    curve_valid(c, b)?;
    if n < 2 {
        return Err(invalid("ICC sampled curve length"));
    }
    for i in 0..n {
        o.charge(128, b)?;
        let value = match c {
            Curve::Table(t) if t.len() == n => *t.get(i).ok_or_else(|| invalid("ICC curve index"))?,
            _ => c.eval64(i as f64 / (n - 1) as f64) as f32,
        };
        if wide {
            o.put(&u16q(value).to_be_bytes(), b)?;
        } else {
            o.put(&[(value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8], b)?;
        }
    }
    Ok(())
}
fn clut_shape(c: &Clut, b: &Budget<'_>) -> Result<()> {
    b.poll()?;
    if !(1..=15).contains(&c.inputs) || !(1..=15).contains(&c.outputs) || c.grid.len() != c.inputs {
        return Err(invalid("ICC CLUT dimensions"));
    }
    let mut n = c.outputs;
    for &g in &c.grid {
        if g == 0 || g > 255 {
            return Err(invalid("ICC CLUT grid"));
        }
        n = mul(n, g)?;
    }
    if n != c.data.len() {
        return Err(invalid("ICC CLUT data length"));
    }
    Ok(())
}
#[derive(Clone, Copy)]
enum LutPlan<'a> {
    Mft {
        matrix: Option<&'a [f64]>,
        a: &'a [Curve],
        c: &'a Clut,
        b: &'a [Curve],
        wide: bool,
    },
    Ab {
        a: Option<&'a [Curve]>,
        c: Option<&'a Clut>,
        m: Option<&'a [Curve]>,
        matrix: Option<(&'a [f64], &'a [f64])>,
        b: &'a [Curve],
        inputs: usize,
        outputs: usize,
        a2b: bool,
    },
}
fn lut_plan<'a>(l: &'a crate::profile::Lut, a2b: bool, budget: &Budget<'_>) -> Result<Option<LutPlan<'a>>> {
    budget.poll()?;
    if l.stages.len() > 4096 {
        return Err(Error::Limit("ICC writer stage count"));
    }
    match l.kind {
        LutKind::Lut8 | LutKind::Lut16 => {
            let (matrix, rest) = match l.stages.first() {
                Some(Stage::Matrix { rows: 3, cols: 3, m, .. }) => (Some(m.as_slice()), l.stages.get(1..).ok_or_else(|| invalid("ICC stages"))?),
                _ => (None, l.stages.as_slice()),
            };
            let [Stage::Curves(a), Stage::Clut(c), Stage::Curves(b)] = rest else { return Ok(None) };
            let Some(&g) = c.grid.first() else { return Ok(None) };
            if !c.grid.iter().all(|x| *x == g) {
                return Ok(None);
            }
            clut_shape(c, budget)?;
            if a.len() != c.inputs || b.len() != c.outputs || matrix.is_some_and(|m| m.len() != 9) {
                return Err(invalid("ICC mft curve/matrix shape"));
            }
            Ok(Some(LutPlan::Mft { matrix, a, c, b, wide: matches!(l.kind, LutKind::Lut16) }))
        }
        LutKind::Ab => {
            let mut rest = l.stages.as_slice();
            let (mut a, mut c, mut m, mut matrix) = (None, None, None, None);
            let b;
            if a2b {
                if let [Stage::Curves(ac), Stage::Clut(cl), tail @ ..] = rest {
                    a = Some(ac.as_slice());
                    c = Some(cl);
                    rest = tail;
                }
                match rest {
                    [Stage::Curves(mc), Stage::Matrix { m: mm, offset, .. }, Stage::Curves(bc)] => {
                        m = Some(mc.as_slice());
                        matrix = Some((mm.as_slice(), offset.as_slice()));
                        b = bc.as_slice();
                    }
                    [Stage::Curves(bc)] => b = bc.as_slice(),
                    _ => return Ok(None),
                }
            } else {
                let Some(Stage::Curves(bc)) = rest.first() else { return Ok(None) };
                b = bc.as_slice();
                rest = rest.get(1..).ok_or_else(|| invalid("ICC stages"))?;
                if let [Stage::Matrix { m: mm, offset, .. }, Stage::Curves(mc), tail @ ..] = rest {
                    matrix = Some((mm.as_slice(), offset.as_slice()));
                    m = Some(mc.as_slice());
                    rest = tail;
                }
                if let [Stage::Clut(cl), Stage::Curves(ac)] = rest {
                    c = Some(cl);
                    a = Some(ac.as_slice());
                    rest = &[];
                }
                if !rest.is_empty() {
                    return Ok(None);
                }
            }
            if !(1..=15).contains(&l.inputs) || !(1..=15).contains(&l.outputs) {
                return Err(invalid("ICC mAB channels"));
            }
            let bc = if a2b { l.outputs } else { l.inputs };
            if b.len() != bc {
                return Err(invalid("ICC mAB B curves"));
            }
            if let Some((mm, off)) = matrix
                && (mm.len() != 9 || off.len() != 3 || bc != 3 || m.is_none_or(|cs| cs.len() != 3))
            {
                return Err(invalid("ICC mAB matrix"));
            }
            if let Some(cl) = c {
                clut_shape(cl, budget)?;
                let ac = if a2b { l.inputs } else { l.outputs };
                if a.is_none_or(|cs| cs.len() != ac) || cl.inputs != (if a2b { l.inputs } else { bc }) || cl.outputs != (if a2b { bc } else { l.outputs }) {
                    return Err(invalid("ICC mAB CLUT curves"));
                }
            }
            Ok(Some(LutPlan::Ab { a, c, m, matrix, b, inputs: l.inputs, outputs: l.outputs, a2b }))
        }
    }
}
fn lut(plan: LutPlan<'_>, o: &mut Sink, budget: &mut Budget<'_>) -> Result<()> {
    match plan {
        LutPlan::Mft { matrix, a, c, b, wide } => {
            o.put(if wide { b"mft2\0\0\0\0" } else { b"mft1\0\0\0\0" }, budget)?;
            o.put(&[u8n(c.inputs)?, u8n(c.outputs)?, u8n(*c.grid.first().ok_or_else(|| invalid("ICC grid"))?)?, 0], budget)?;
            let identity = [1., 0., 0., 0., 1., 0., 0., 0., 1.];
            for v in matrix.unwrap_or(&identity) {
                o.put(&s15(*v), budget)?;
            }
            let (ni, no) = if wide { (table_len(a), table_len(b)) } else { (256, 256) };
            if wide {
                o.put(&u16::try_from(ni).map_err(|_| invalid("ICC input curve count"))?.to_be_bytes(), budget)?;
                o.put(&u16::try_from(no).map_err(|_| invalid("ICC output curve count"))?.to_be_bytes(), budget)?;
            }
            for v in a {
                sampled(v, ni, wide, o, budget)?;
            }
            for v in &c.data {
                if wide {
                    o.put(&u16q(*v).to_be_bytes(), budget)?;
                } else {
                    o.put(&[(v.clamp(0., 1.) * 255. + 0.5) as u8], budget)?;
                }
            }
            for v in b {
                sampled(v, no, wide, o, budget)?;
            }
        }
        LutPlan::Ab { a, c, m, matrix, b, inputs, outputs, a2b } => {
            o.put(if a2b { b"mAB \0\0\0\0" } else { b"mBA \0\0\0\0" }, budget)?;
            o.put(&[u8n(inputs)?, u8n(outputs)?, 0, 0], budget)?;
            o.put(&[0; 20], budget)?;
            let mut offsets = [0u32; 5];
            offsets[0] = u32n(o.len)?;
            curves(b, o, budget)?;
            if let Some((mm, off)) = matrix {
                offsets[1] = u32n(o.len)?;
                for v in mm.iter().chain(off) {
                    o.put(&s15(*v), budget)?;
                }
                offsets[2] = u32n(o.len)?;
                curves(m.ok_or_else(|| invalid("ICC M curves"))?, o, budget)?;
            }
            if let Some(c) = c {
                offsets[3] = u32n(o.len)?;
                let mut grid = [0; 16];
                for (slot, &n) in grid.iter_mut().zip(&c.grid) {
                    *slot = u8n(n)?;
                }
                o.put(&grid, budget)?;
                o.put(&[2, 0, 0, 0], budget)?;
                for v in &c.data {
                    o.put(&u16q(*v).to_be_bytes(), budget)?;
                }
                o.pad(budget)?;
                offsets[4] = u32n(o.len)?;
                curves(a.ok_or_else(|| invalid("ICC A curves"))?, o, budget)?;
            }
            if let Some(v) = &mut o.bytes {
                for (k, offset) in offsets.iter().enumerate() {
                    v.get_mut(12 + k * 4..16 + k * 4).ok_or_else(|| invalid("ICC offset patch"))?.copy_from_slice(&offset.to_be_bytes());
                }
            }
        }
    }
    Ok(())
}
#[derive(Clone, Copy)]
enum Payload<'a> {
    Text(&'a str, &'a str),
    Xyz([f64; 3]),
    Chad(&'a crate::math::Mat3),
    Curve(&'a Curve),
    Lut(LutPlan<'a>),
}
fn payload(p: Payload<'_>, o: &mut Sink, b: &mut Budget<'_>) -> Result<()> {
    match p {
        Payload::Text(s, suffix) => {
            let mut count = 0;
            for _ in s.encode_utf16().chain(suffix.encode_utf16()) {
                o.charge(1, b)?;
                count = add(count, 1)?;
            }
            o.put(b"mluc\0\0\0\0", b)?;
            o.put(&1u32.to_be_bytes(), b)?;
            o.put(&12u32.to_be_bytes(), b)?;
            o.put(b"enUS", b)?;
            o.put(&u32n(mul(count, 2)?)?.to_be_bytes(), b)?;
            o.put(&28u32.to_be_bytes(), b)?;
            for u in s.encode_utf16().chain(suffix.encode_utf16()) {
                o.put(&u.to_be_bytes(), b)?;
            }
        }
        Payload::Xyz(v) => {
            o.put(b"XYZ \0\0\0\0", b)?;
            for v in v {
                o.put(&s15(v), b)?;
            }
        }
        Payload::Chad(m) => {
            o.put(b"sf32\0\0\0\0", b)?;
            for v in m.iter().flatten() {
                o.put(&s15(*v), b)?;
            }
        }
        Payload::Curve(c) => curve(c, o, b)?,
        Payload::Lut(l) => lut(l, o, b)?,
    }
    Ok(())
}
#[derive(Clone, Copy)]
struct Tag<'a> {
    sig: [u8; 4],
    payload: Payload<'a>,
    len: usize,
}
struct Plan<'a> {
    tags: [Option<Tag<'a>>; 24],
    count: usize,
    total: usize,
    requirements: EncodeRequirements,
}
fn plan<'a>(p: &'a Profile, suffix: &'a str, override_curve: Option<&'a Curve>, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<Plan<'a>> {
    let mut b = Budget::new(limits, cancelled)?;
    let retained = p.controlled_retained_bytes(&b)?;
    b.retained(retained)?;
    let mut tags = [None; 24];
    let mut count = 0;
    let mut total = 0;
    let mut census_work = 0u64;
    let mut push = |sig: [u8; 4], value: Payload<'a>| -> Result<()> {
        let mut sink = Sink::count();
        payload(value, &mut sink, &mut b)?;
        u32n(sink.len)?;
        census_work = census_work.checked_add(sink.work).ok_or(Error::Limit("ICC census work"))?;
        total = add(total, add(sink.len, 3)? & !3)?;
        *tags.get_mut(count).ok_or(Error::Limit("ICC tag count"))? = Some(Tag { sig, payload: value, len: sink.len });
        count += 1;
        Ok(())
    };
    push(*b"desc", Payload::Text(&p.description, suffix))?;
    push(*b"cprt", Payload::Text(&p.copyright, ""))?;
    push(*b"wtpt", Payload::Xyz(p.white_point))?;
    if let Some(v) = p.black_point {
        push(*b"bkpt", Payload::Xyz(v))?;
    }
    if let Some(v) = &p.chad {
        push(*b"chad", Payload::Chad(v))?;
    }
    if let Some(m) = p.matrix {
        push(*b"rXYZ", Payload::Xyz([m[0][0], m[1][0], m[2][0]]))?;
        push(*b"gXYZ", Payload::Xyz([m[0][1], m[1][1], m[2][1]]))?;
        push(*b"bXYZ", Payload::Xyz([m[0][2], m[1][2], m[2][2]]))?;
    }
    if let Some(c) = override_curve {
        match p.color_space {
            crate::ColorSpace::Rgb => {
                for sig in [*b"rTRC", *b"gTRC", *b"bTRC"] {
                    push(sig, Payload::Curve(c))?;
                }
                if let Some(c) = &p.gray_trc {
                    push(*b"kTRC", Payload::Curve(c))?;
                }
            }
            crate::ColorSpace::Gray => {
                if let Some(cs) = &p.trc {
                    for (sig, c) in [*b"rTRC", *b"gTRC", *b"bTRC"].into_iter().zip(cs) {
                        push(sig, Payload::Curve(c))?;
                    }
                }
                push(*b"kTRC", Payload::Curve(c))?;
            }
            _ => return Err(invalid("ICC transfer override space")),
        }
    } else {
        if let Some(cs) = &p.trc {
            for (sig, c) in [*b"rTRC", *b"gTRC", *b"bTRC"].into_iter().zip(cs) {
                push(sig, Payload::Curve(c))?;
            }
        }
        if let Some(c) = &p.gray_trc {
            push(*b"kTRC", Payload::Curve(c))?;
        }
    }
    // The fixed inventory has no allocations; unsupported legacy layouts are omitted.
    for (ls, names, a2b) in [(&p.a2b, [*b"A2B0", *b"A2B1", *b"A2B2"], true), (&p.b2a, [*b"B2A0", *b"B2A1", *b"B2A2"], false)] {
        for (l, sig) in ls.iter().zip(names) {
            if let Some(l) = l {
                let probe = Budget::new(limits, cancelled)?;
                if let Some(recipe) = lut_plan(l, a2b, &probe)? {
                    push(sig, Payload::Lut(recipe))?;
                }
            }
        }
    }
    drop(push);
    let table = add(4, mul(count, 12)?)?;
    let encoded = add(add(128, table)?, total)?;
    u32n(encoded)?;
    // Payload inventory, packed data, final bytes and worst-case dedup comparisons.
    let allocation = add(mul(total, 3)?, mul(count, std::mem::size_of::<([u8; 4], Vec<u8>)>())?)?;
    let peak = add(retained, add(allocation, add(128, table)?)?)?;
    let work = census_work
        .checked_mul(2)
        .and_then(|v| v.checked_add(mul(total, add(count, 4).ok()?).ok()? as u64))
        .and_then(|v| v.checked_add((128 + table) as u64))
        .ok_or(Error::Limit("ICC work"))?;
    let admission = Budget::new(limits, cancelled)?;
    admission.admit(peak, work)?;
    Ok(Plan {
        tags,
        count,
        total,
        requirements: EncodeRequirements { retained_bytes: retained, encoded_bytes_upper_bound: encoded, peak_resident_bytes: peak, work_units: work },
    })
}
pub(crate) fn transfer_requirements(p: &Profile, suffix: &str, curve: &Curve, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<EncodeRequirements> {
    Ok(plan(p, suffix, Some(curve), limits, cancelled)?.requirements)
}
pub fn requirements(p: &Profile, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<EncodeRequirements> {
    Ok(plan(p, "", None, limits, cancelled)?.requirements)
}
fn equal(a: &[u8], b: &[u8], budget: &mut Budget<'_>) -> Result<bool> {
    if a.len() != b.len() {
        return Ok(false);
    }
    for (x, y) in a.chunks(1024).zip(b.chunks(1024)) {
        budget.work(x.len() as u64)?;
        if x != y {
            return Ok(false);
        }
    }
    Ok(true)
}
pub fn encode(p: &Profile, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    let plan = plan(p, "", None, limits, cancelled)?;
    let mut budget = Budget::new(limits, cancelled)?;
    budget.retained(plan.requirements.retained_bytes)?;
    let mut tags = budget.reserve(plan.count)?;
    for t in plan.tags.iter().flatten() {
        let mut sink = Sink::allocated(t.len, &mut budget)?;
        payload(t.payload, &mut sink, &mut budget)?;
        if sink.len != t.len {
            return Err(invalid("ICC recipe size changed"));
        }
        tags.push((t.sig, sink.finish()?));
    }
    let table = add(4, mul(plan.count, 12)?)?;
    let mut data = Sink::allocated(plan.total, &mut budget)?;
    let mut entries = [([0u8; 4], 0u32, 0u32); 24];
    let mut placed = [(0usize, 0u32, 0u32); 24];
    let mut placed_count = 0;
    for (i, (sig, d)) in tags.iter().enumerate() {
        let mut prior = None;
        for &(j, off, len) in placed.iter().take(placed_count) {
            let candidate = &tags.get(j).ok_or_else(|| invalid("ICC tag lookup"))?.1;
            if equal(candidate, d, &mut budget)? {
                prior = Some((off, len));
                break;
            }
        }
        let (off, len) = match prior {
            Some(v) => v,
            None => {
                let off = u32n(add(add(128, table)?, data.len)?)?;
                data.put(d, &mut budget)?;
                data.pad(&mut budget)?;
                let len = u32n(d.len())?;
                *placed.get_mut(placed_count).ok_or(Error::Limit("ICC placements"))? = (i, off, len);
                placed_count += 1;
                (off, len)
            }
        };
        *entries.get_mut(i).ok_or(Error::Limit("ICC entries"))? = (*sig, off, len);
    }
    let size = add(add(128, table)?, data.len)?;
    let mut o = Sink::allocated(size, &mut budget)?;
    o.put(&u32n(size)?.to_be_bytes(), &mut budget)?;
    o.put(&[0; 4], &mut budget)?;
    o.put(&[4, 0x30, 0, 0], &mut budget)?;
    o.put(&p.class.sig().to_be_bytes(), &mut budget)?;
    o.put(&p.color_space.sig().to_be_bytes(), &mut budget)?;
    o.put(
        match p.pcs {
            Pcs::Xyz => b"XYZ ",
            Pcs::Lab => b"Lab ",
        },
        &mut budget,
    )?;
    for v in [2026u16, 1, 1, 0, 0, 0] {
        o.put(&v.to_be_bytes(), &mut budget)?;
    }
    o.put(b"acsp", &mut budget)?;
    o.put(&[0; 24], &mut budget)?;
    o.put(&(p.rendering_intent as u32).to_be_bytes(), &mut budget)?;
    for v in math::D50 {
        o.put(&s15(v), &mut budget)?;
    }
    o.put(&[0; 20], &mut budget)?;
    while o.len < 128 {
        o.put(&[0], &mut budget)?;
    }
    o.put(&u32n(plan.count)?.to_be_bytes(), &mut budget)?;
    for (sig, off, len) in entries.iter().take(plan.count) {
        o.put(sig, &mut budget)?;
        o.put(&off.to_be_bytes(), &mut budget)?;
        o.put(&len.to_be_bytes(), &mut budget)?;
    }
    let packed = data.finish()?;
    for part in packed.chunks(1024) {
        o.put(part, &mut budget)?;
    }
    budget.poll()?;
    if o.len != size || o.len > plan.requirements.encoded_bytes_upper_bound {
        return Err(invalid("ICC final size"));
    }
    o.finish()
}

#[cfg(test)]
mod controlled_tests {
    use super::*;
    use crate::{
        Builtin, ColorSpace,
        pipeline::Stage,
        profile::{Lut, LutKind},
    };
    use std::cell::Cell;
    #[test]
    fn all_builtin_writer_bytes_equal_legacy() {
        for builtin in [Builtin::Srgb, Builtin::SGray, Builtin::DisplayP3, Builtin::LabD50, Builtin::CoatedCmyk] {
            let p = builtin.profile();
            let actual = encode(p, Limits::default(), &|| false).unwrap();
            assert_eq!(actual, super::super::encode(p));
        }
    }
    #[test]
    fn every_lut_family_and_direction_preserve_bytes_and_omission() {
        for kind in [LutKind::Lut8, LutKind::Lut16, LutKind::Ab] {
            for reverse in [false, true] {
                let mut p = Builtin::Srgb.profile().clone();
                p.description = "Unicode \u{1f642} / repeated tag".into();
                p.copyright = p.description.clone();
                let curves = vec![Curve::Identity; 3];
                let c = Clut::sample(vec![2; 3], 3, |x, y| y.copy_from_slice(x));
                let stages = vec![Stage::Curves(curves.clone()), Stage::Clut(c), Stage::Curves(curves)];
                let l = Lut { kind, inputs: 3, outputs: 3, stages };
                if reverse {
                    p.b2a[0] = Some(l);
                } else {
                    p.a2b[0] = Some(l);
                }
                assert_eq!(encode(&p, Limits::default(), &|| false).unwrap(), super::super::encode(&p));
            }
        }
        let mut p = Builtin::Srgb.profile().clone();
        p.a2b[0] = Some(Lut { kind: LutKind::Ab, inputs: 3, outputs: 3, stages: vec![Stage::Clamp01] });
        assert_eq!(encode(&p, Limits::default(), &|| false).unwrap(), super::super::encode(&p));
    }
    #[test]
    fn table64_text_dedup_and_resource_cancel_do_not_mutate_source() {
        let mut p = Builtin::Srgb.profile().clone();
        p.color_space = ColorSpace::Rgb;
        p.trc = Some(std::array::from_fn(|_| Curve::Table64(vec![0., 0.123456789, 1.])));
        let original = super::super::encode(&p);
        let before = p.clone();
        assert_eq!(encode(&p, Limits::default(), &|| false).unwrap(), original);
        assert!(matches!(encode(&p, Limits { resident_bytes: 1, work_units: 1 }, &|| false), Err(Error::Limit(_))));
        let calls = Cell::new(0);
        assert!(matches!(
            encode(&p, Limits::default(), &|| {
                calls.set(calls.get() + 1);
                calls.get() > 40
            }),
            Err(Error::Cancelled)
        ));
        assert_eq!(p.description, before.description);
        assert_eq!(p.trc, before.trc);
        assert_eq!(p.encoded_bytes(), before.encoded_bytes());
    }
    #[test]
    fn malformed_grid_and_curve_kind_are_errors() {
        let mut p = Builtin::Srgb.profile().clone();
        p.trc = Some(std::array::from_fn(|_| Curve::Parametric { kind: 99, p: [1.; 7] }));
        assert!(encode(&p, Limits::default(), &|| false).is_err());
        let mut c = Clut::sample(vec![2; 3], 3, |x, y| y.copy_from_slice(x));
        c.grid.push(2);
        p.trc = Some(std::array::from_fn(|_| Curve::Identity));
        p.a2b[0] = Some(Lut {
            kind: LutKind::Lut16,
            inputs: 3,
            outputs: 3,
            stages: vec![Stage::Curves(vec![Curve::Identity; 3]), Stage::Clut(c), Stage::Curves(vec![Curve::Identity; 3])],
        });
        assert!(encode(&p, Limits::default(), &|| false).is_err());
    }
}
