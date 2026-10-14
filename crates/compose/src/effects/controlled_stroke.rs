//! Existing raster Stroke distance choices and gradient frames, controlled.
use super::{Effect, FxPaint, INSIDE_EPS, Layer, MAX_REACH, Map, stroke_outside_exact, stroke_widths};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError as E};
#[derive(Clone, Copy, Default)]
pub(super) struct Plan {
    pub present: bool,
    pub narrow: bool,
    pub exact: bool,
    pub strokes: usize,
    pub gradients: usize,
}
impl Plan {
    pub fn fields(self) -> usize {
        usize::from(self.present) + usize::from(self.narrow) + usize::from(self.exact)
    }
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
pub(super) fn plan(layer: &Layer, c: &Interrupt<'_>) -> Result<Plan, E> {
    let mut p = Plan::default();
    for e in &layer.effects.items {
        poll(c)?;
        if !e.enabled() {
            continue;
        }
        let Effect::Stroke(st) = e else {
            continue;
        };
        if !matches!(layer.content, photocraft_doc::LayerContent::Raster(_)) || !st.size.is_finite() {
            return Err(E::UnsupportedConversion);
        }
        p.present = true;
        p.strokes = p.strokes.checked_add(1).filter(|n| *n <= 64).ok_or(E::LimitExceeded)?;
        let width = stroke_widths(st).1;
        if width > 0.0 {
            if stroke_outside_exact(width) {
                p.exact = true;
            } else {
                p.narrow = true;
            }
        }
        if matches!(st.paint, FxPaint::Gradient(_)) {
            p.gradients = p.gradients.checked_add(1).ok_or(E::LimitExceeded)?;
        }
    }
    Ok(p)
}
pub(super) fn retained_bytes(w: usize, h: usize, p: Plan) -> Result<usize, E> {
    w.checked_mul(h)
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| n.checked_mul(p.fields()))
        .and_then(|n| p.gradients.checked_mul(std::mem::size_of::<(u32, Rect)>()).and_then(|v| n.checked_add(v)))
        .ok_or(E::LimitExceeded)
}
pub(super) fn scratch_bytes(w: usize, h: usize, p: Plan) -> Result<usize, E> {
    if !p.present {
        return Ok(0);
    }
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    // Coverage/start/distance/nearest/final owners, plus exact EDT line scratch.
    let mut bytes = n.checked_mul(28).ok_or(E::LimitExceeded)?;
    if p.exact {
        bytes = bytes.max(super::controlled_euclidean::scratch_bytes(w, h)?);
    }
    Ok(bytes)
}
pub(super) fn work_units(w: usize, h: usize, p: Plan) -> Result<u64, E> {
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)? as u64;
    let factor = if p.present { 320u64 } else { 0 };
    let mut work = n.checked_mul(factor).ok_or(E::LimitExceeded)?;
    if p.narrow {
        work = work.checked_add(n.checked_mul(192).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
    }
    if p.exact {
        work = work.checked_add(super::controlled_euclidean::work_units(w, h)?).ok_or(E::LimitExceeded)?;
    }
    work = work
        .checked_add(n.checked_mul(p.gradients as u64).and_then(|n| n.checked_mul(8)).ok_or(E::LimitExceeded)?)
        .and_then(|n| n.checked_add((p.strokes as u64).checked_mul(p.strokes as u64)?.checked_mul(4)?))
        .ok_or(E::LimitExceeded)?;
    if work > 8_000_000_000 {
        return Err(E::LimitExceeded);
    }
    Ok(work)
}
pub(super) struct Fields {
    pub din: Option<Vec<f32>>,
    pub dout: Option<Vec<f32>>,
    pub dout_exact: Option<Vec<f32>>,
    pub frames: Vec<(u32, Rect)>,
}
fn frame(shape: &Map, width: f32, rect: Rect, c: &Interrupt<'_>) -> Result<Rect, E> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..shape.h {
        poll(c)?;
        for x in 0..shape.w {
            if x.is_multiple_of(1024) {
                poll(c)?;
            }
            let a = *shape.v.get(y * shape.w + x).ok_or(E::LengthMismatch)?;
            if a > INSIDE_EPS {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 == usize::MAX {
        return Ok(rect);
    }
    let grow = (width.min(MAX_REACH).ceil().max(1.0) as i32 - 1).max(0);
    let coord = |base: i32, value: usize| i32::try_from(i64::from(base) + i64::try_from(value).map_err(|_| E::InvalidBounds)?).map_err(|_| E::InvalidBounds);
    Ok(Rect::new(coord(rect.x0, x0)?, coord(rect.y0, y0)?, coord(rect.x0, x1)?, coord(rect.y0, y1)?).inflate(grow))
}
pub(super) fn build(layer: &Layer, shape: &Map, rect: Rect, budget: usize, c: &Interrupt<'_>) -> Result<Fields, E> {
    poll(c)?;
    let p = plan(layer, c)?;
    if !p.present {
        return Ok(Fields { din: None, dout: None, dout_exact: None, frames: Vec::new() });
    }
    let n = shape.w.checked_mul(shape.h).ok_or(E::LimitExceeded)?;
    if n != shape.v.len() {
        return Err(E::LengthMismatch);
    }
    let peak = scratch_bytes(shape.w, shape.h, p)?.checked_add(retained_bytes(shape.w, shape.h, p)?).ok_or(E::LimitExceeded)?;
    work_units(shape.w, shape.h, p)?;
    if peak > budget.min(64 << 20) {
        return Err(E::LimitExceeded);
    }
    let din = if p.present { Some(super::controlled_distance::field(shape, true, budget, c)?) } else { None };
    let dout = if p.narrow { Some(super::controlled_distance::field(shape, false, budget, c)?) } else { None };
    let dout_exact = if p.exact { Some(super::controlled_euclidean::distance(shape, false, budget, c)?) } else { None };
    let mut frames = Vec::new();
    frames.try_reserve_exact(p.gradients).map_err(|_| E::AllocationFailed)?;
    if frames.capacity() != p.gradients {
        return Err(E::LimitExceeded);
    }
    for e in &layer.effects.items {
        poll(c)?;
        if !e.enabled() {
            continue;
        }
        let Effect::Stroke(st) = e else {
            continue;
        };
        if !matches!(st.paint, FxPaint::Gradient(_)) {
            continue;
        }
        let width = stroke_widths(st).1;
        let key = width.to_bits();
        let mut found = false;
        for (entry, _) in &frames {
            poll(c)?;
            if *entry == key {
                found = true;
                break;
            }
        }
        if !found {
            if frames.len() >= frames.capacity() {
                return Err(E::LimitExceeded);
            }
            frames.push((key, frame(shape, width, rect, c)?));
        }
    }
    poll(c)?;
    Ok(Fields { din, dout, dout_exact, frames })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raster_stroke_fields_keep_thresholds_and_raw_words() {
        for size in [0.0, 19.5, 20.0, 40.0] {
            for position in [super::super::StrokePosition::Inside, super::super::StrokePosition::Outside, super::super::StrokePosition::Center] {
                let st = photocraft_doc::StrokeFx {
                    common: photocraft_doc::FxCommon::new(photocraft_doc::BlendMode::Normal, 1.0),
                    size,
                    position,
                    paint: FxPaint::Color(photocraft_doc::Color::WHITE),
                };
                let mut layer =
                    Layer::new("stroke draft", photocraft_doc::LayerContent::Raster(photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8)));
                layer.effects.items.push(Effect::Stroke(st));
                let rect = Rect::new(-7, 13, 24, 36);
                let mut shape = Map::new(31, 23, 0.0);
                for (i, a) in shape.v.iter_mut().enumerate() {
                    *a = ((i * 73) % 101) as f32 / 100.0;
                }
                let expected = super::super::build_maps(
                    &layer,
                    shape.v.clone(),
                    rect,
                    &photocraft_doc::GlobalLight::default(),
                    &super::super::TextureCtx { rect, patterns: &[], anchor: (0.0, 0.0) },
                );
                let got = build(&layer, &shape, rect, 64 << 20, &Interrupt::NONE).unwrap();
                for (a, b) in [(&got.din, &expected.din), (&got.dout, &expected.dout), (&got.dout_exact, &expected.dout_exact)] {
                    assert_eq!(
                        a.as_ref().map(|v| v.iter().map(|v| v.to_bits()).collect::<Vec<_>>()),
                        b.as_ref().map(|v| v.iter().map(|v| v.to_bits()).collect::<Vec<_>>())
                    );
                }
            }
        }
    }
    #[test]
    fn private_stroke_refusal_and_cancel_preserve_input() {
        let mut layer = Layer::raster("stroke", photocraft_color::PixelFormat::RGBA8);
        layer.effects.items.push(Effect::Stroke(photocraft_doc::StrokeFx {
            common: photocraft_doc::FxCommon::new(photocraft_doc::BlendMode::Normal, 1.0),
            size: 40.0,
            position: super::super::StrokePosition::Outside,
            paint: FxPaint::Color(photocraft_doc::Color::WHITE),
        }));
        let shape = Map::new(64, 64, 0.5);
        let before = shape.v.clone();
        assert!(matches!(build(&layer, &shape, Rect::new(0, 0, 64, 64), 1, &Interrupt::NONE), Err(E::LimitExceeded)));
        assert!(matches!(build(&layer, &shape, Rect::new(0, 0, 64, 64), 64 << 20, &Interrupt::cancel_only(&|| true)), Err(E::Cancelled)));
        assert_eq!(shape.v, before);
    }
    #[test]
    fn exact_gradient_stroke_frame_and_retained_field_charge() {
        let rect = Rect::new(-7, 13, 24, 36);
        let mut shape = Map::new(31, 23, 0.0);
        for y in 4..18 {
            for x in 3..26 {
                shape.v[y * 31 + x] = 0.75;
            }
        }
        for width in [0.0, 1.0, 3.5, 19.5, 20.0, 41.0] {
            assert_eq!(frame(&shape, width, rect, &Interrupt::NONE).unwrap(), super::super::stroke_frame(&shape, width, rect));
        }
        let mut layer = Layer::raster("shared field ledger", photocraft_color::PixelFormat::RGBA8);
        for size in [3.0, 40.0] {
            layer.effects.items.push(Effect::Stroke(photocraft_doc::StrokeFx {
                common: photocraft_doc::FxCommon::new(photocraft_doc::BlendMode::Normal, 1.0),
                size,
                position: super::super::StrokePosition::Outside,
                paint: FxPaint::Color(photocraft_doc::Color::WHITE),
            }));
        }
        let p = plan(&layer, &Interrupt::NONE).unwrap();
        assert_eq!(p.fields(), 3);
        let retained = super::super::checked_effect_map_retained_bytes(&layer, rect, &Interrupt::NONE).unwrap();
        assert_eq!(retained, 31 * 23 * 4 * 4 + 2 * 128 + 1024);
    }
}
