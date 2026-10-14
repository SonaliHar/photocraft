//! Controlled ordinary Glow coverage, preserving legacy range/order.
use super::{Glow, GlowSource, GlowTechnique, Map, box_geom, spread_split};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
const MAX_WORK: u64 = 8_000_000_000;
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
pub(super) fn validate(g: &Glow) -> Result<(), E> {
    let (r, blur) = spread_split(g.size, g.spread);
    super::controlled_contour::requirements(&g.contour, Some(g.range), false)?;
    if !g.size.is_finite()
        || !g.spread.is_finite()
        || !r.is_finite()
        || !blur.is_finite()
        || !g.range.is_finite()
        || g.technique == GlowTechnique::Precise && (!(g.size * g.spread).is_finite() || !(g.size + 1.0 - g.size * g.spread).is_finite())
        || !g.noise.is_finite()
        || !(0.0..=1.0).contains(&g.noise)
    {
        return Err(E::UnsupportedConversion);
    }
    Ok(())
}
pub(super) fn contour_scratch_bytes(g: &Glow) -> Result<usize, E> {
    Ok(super::controlled_contour::requirements(&g.contour, Some(g.range), false)?.peak_bytes)
}
pub(super) fn work_units(w: usize, h: usize, g: &Glow) -> Result<u64, E> {
    validate(g)?;
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    let contour_work = super::controlled_contour::requirements(&g.contour, Some(g.range), false)?.work_units;
    if g.technique == GlowTechnique::Precise {
        let work = super::controlled_euclidean::work_units(w, h)?
            .checked_add((n as u64).checked_mul(64).ok_or(E::LimitExceeded)?)
            .and_then(|n| n.checked_add(contour_work))
            .ok_or(E::LimitExceeded)?;
        if work > MAX_WORK {
            return Err(E::LimitExceeded);
        }
        return Ok(work);
    }
    let (_, blur) = spread_split(g.size, g.spread);
    let (r, _, _) = box_geom(blur);
    let lines = (w as u64).checked_add(h as u64).and_then(|n| n.checked_mul(u64::try_from(r).ok()?)).and_then(|n| n.checked_mul(16)).ok_or(E::LimitExceeded)?;
    let work = (n as u64).checked_mul(320).and_then(|n| n.checked_add(lines)).and_then(|n| n.checked_add(contour_work)).ok_or(E::LimitExceeded)?;
    if work > MAX_WORK {
        return Err(E::LimitExceeded);
    }
    Ok(work)
}
/// Peak private owners for one checked glow; retained input/result maps are
/// charged separately by the caller's full enabled-stack envelope.
pub(super) fn scratch_bytes(w: usize, h: usize, g: &Glow) -> Result<usize, E> {
    validate(g)?;
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    if g.technique == GlowTechnique::Precise {
        return super::controlled_euclidean::scratch_bytes(w, h)?.checked_add(contour_scratch_bytes(g)?).ok_or(E::LimitExceeded);
    }
    let (_, blur) = spread_split(g.size, g.spread);
    let (r, _, _) = box_geom(blur);
    let line = w.max(h).checked_add(usize::try_from(r).map_err(|_| E::LimitExceeded)?.checked_mul(2).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
    n.checked_mul(28)
        .and_then(|n| line.checked_mul(24).and_then(|v| n.checked_add(v)))
        .and_then(|n| n.checked_add(contour_scratch_bytes(g).ok()?))
        .ok_or(E::LimitExceeded)
}
fn filled(n: usize, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    poll(c)?;
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if out.capacity() != n {
        return Err(E::LimitExceeded);
    }
    while out.len() < n {
        poll(c)?;
        out.resize(out.len().saturating_add(4096).min(n), 0.0);
    }
    Ok(out)
}
pub(super) fn build(shape: &Map, g: &Glow, inner: bool, origin: (i32, i32), budget: usize, c: &Interrupt<'_>) -> Result<Map, E> {
    poll(c)?;
    validate(g)?;
    let n = shape.w.checked_mul(shape.h).ok_or(E::LimitExceeded)?;
    if n != shape.v.len() {
        return Err(E::LengthMismatch);
    }
    work_units(shape.w, shape.h, g)?;
    let peak = scratch_bytes(shape.w, shape.h, g)?;
    if peak > budget.min(64 << 20) || peak as u64 > MAX_CHECKED_RASTER_BYTES {
        return Err(E::LimitExceeded);
    }
    let mut map = match g.technique {
        GlowTechnique::Precise => {
            let distance = super::controlled_euclidean::distance(shape, inner, budget, c)?;
            let solid = g.size * g.spread;
            let soft = (g.size + 1.0 - solid).max(1e-3);
            if !solid.is_finite() || !soft.is_finite() {
                return Err(E::UnsupportedConversion);
            }
            let mut out = filled(n, c)?;
            for (i, (v, d)) in out.iter_mut().zip(distance).enumerate() {
                if i.is_multiple_of(1024) {
                    poll(c)?;
                }
                let d = d + 0.5;
                *v = if d <= solid { 1.0 } else { 0.5 * (1.0 - (d - solid) / soft).clamp(0.0, 1.0) };
            }
            Map { w: shape.w, h: shape.h, v: out }
        }
        GlowTechnique::Softer => {
            let (radius, blur) = spread_split(g.size, g.spread);
            let mut values = filled(n, c)?;
            for (i, (o, a)) in values.iter_mut().zip(&shape.v).enumerate() {
                if i.is_multiple_of(1024) {
                    poll(c)?;
                }
                if !a.is_finite() {
                    return Err(E::InvalidSample);
                }
                *o = if inner { 1.0 - a } else { *a };
            }
            let source = Map { w: shape.w, h: shape.h, v: values };
            let grown = if radius > 0.0 {
                let grown = super::controlled_distance::dilate(&source, radius, budget, c)?;
                drop(source);
                grown
            } else {
                source
            };
            let map = super::checked_prepare::try_tent(&grown, budget, blur, c)?;
            drop(grown);
            map
        }
    };
    let lut = super::controlled_contour::prepare(&g.contour, Some(g.range), false, budget, c)?;
    super::controlled_contour::apply(&mut map, lut.as_deref(), c)?;
    drop(lut);
    if inner && g.source == GlowSource::Center {
        for (i, v) in map.v.iter_mut().enumerate() {
            if i.is_multiple_of(1024) {
                poll(c)?;
            }
            *v = 1.0 - *v;
        }
    }
    super::checked_prepare::checked_noise(&mut map, g.noise, origin, c)?;
    if inner {
        for (i, (v, a)) in map.v.iter_mut().zip(&shape.v).enumerate() {
            if i.is_multiple_of(1024) {
                poll(c)?;
            }
            *v *= a;
            if !v.is_finite() {
                return Err(E::InvalidSample);
            }
        }
    }
    for values in map.v.chunks(4096) {
        poll(c)?;
        if values.iter().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
    }
    poll(c)?;
    Ok(map)
}
#[cfg(test)]
mod tests {
    use super::super::Contour;
    use super::*;
    fn glow() -> Glow {
        Glow {
            common: photocraft_doc::FxCommon::new(photocraft_doc::BlendMode::Screen, 1.0),
            paint: photocraft_doc::FxPaint::Color(photocraft_doc::Color::WHITE),
            technique: GlowTechnique::Softer,
            spread: 0.3,
            size: 7.25,
            contour: Contour::Linear,
            anti_alias: false,
            range: 0.5,
            jitter: 0.0,
            noise: 0.17,
            source: GlowSource::Edge,
        }
    }
    #[test]
    fn exact_softer_glow_range_center_noise_and_alpha_order() {
        for (w, h) in [(0, 0), (1, 1), (1, 19), (27, 1), (31, 23)] {
            let mut shape = Map::new(w, h, 0.0);
            for (i, v) in shape.v.iter_mut().enumerate() {
                *v = ((i * 73) % 101) as f32 / 100.0;
            }
            for range in [0.0, 0.5, 0.998, 0.999, 1.0] {
                for inner in [false, true] {
                    for source in [GlowSource::Edge, GlowSource::Center] {
                        let mut g = glow();
                        if range == 0.5 {
                            g.contour = Contour::Custom {
                                name: String::new(),
                                points: vec![
                                    photocraft_doc::adjust::CurvePoint { input: 0.0, output: 0.0 },
                                    photocraft_doc::adjust::CurvePoint { input: 0.45, output: 0.85 },
                                    photocraft_doc::adjust::CurvePoint { input: 1.0, output: 0.2 },
                                ],
                                corners: vec![false, true, false],
                            };
                        }
                        g.range = range;
                        g.source = source;
                        let expected = super::super::glow_map(&shape, &g, inner, (-7, 13));
                        let got = build(&shape, &g, inner, (-7, 13), 64 << 20, &Interrupt::NONE).unwrap();
                        assert_eq!(got.v.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.v.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
                    }
                }
            }
        }
    }
    #[test]
    fn malformed_glow_and_private_failure() {
        let shape = Map::new(64, 64, 0.5);
        let before = shape.v.clone();
        let mut g = glow();
        g.range = f32::NAN;
        assert!(matches!(build(&shape, &g, false, (0, 0), 64 << 20, &Interrupt::NONE), Err(E::UnsupportedConversion)));
        g.range = 0.5;
        assert!(matches!(build(&shape, &g, false, (0, 0), 1, &Interrupt::NONE), Err(E::LimitExceeded)));
        use std::sync::atomic::{AtomicUsize, Ordering};
        let ticks = AtomicUsize::new(0);
        let cancel = || ticks.fetch_add(1, Ordering::Relaxed) > 40;
        assert!(matches!(build(&shape, &g, true, (0, 0), 64 << 20, &Interrupt::cancel_only(&cancel)), Err(E::Cancelled)));
        assert_eq!(shape.v, before);
    }
    #[test]
    fn exact_precise_glow_keeps_nearest_alpha_range_center_and_noise() {
        for (w, h) in [(0, 0), (1, 1), (1, 19), (27, 1), (31, 23)] {
            let mut shape = Map::new(w, h, 0.0);
            for (i, a) in shape.v.iter_mut().enumerate() {
                *a = ((i * 73) % 101) as f32 / 100.0;
            }
            for range in [0.5, 0.999, 1.0] {
                for inner in [false, true] {
                    for source in [GlowSource::Edge, GlowSource::Center] {
                        let mut g = glow();
                        if range == 0.5 {
                            g.contour = Contour::Custom {
                                name: String::new(),
                                points: vec![
                                    photocraft_doc::adjust::CurvePoint { input: 0.0, output: 0.0 },
                                    photocraft_doc::adjust::CurvePoint { input: 0.45, output: 0.85 },
                                    photocraft_doc::adjust::CurvePoint { input: 1.0, output: 0.2 },
                                ],
                                corners: vec![false, true, false],
                            };
                        }
                        g.technique = GlowTechnique::Precise;
                        g.range = range;
                        g.source = source;
                        let expected = super::super::glow_map(&shape, &g, inner, (-7, 13));
                        let got = build(&shape, &g, inner, (-7, 13), 64 << 20, &Interrupt::NONE).unwrap();
                        assert_eq!(got.v.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.v.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
                    }
                }
            }
        }
    }
}
