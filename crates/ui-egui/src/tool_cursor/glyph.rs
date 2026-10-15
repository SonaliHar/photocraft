//! Original procedural cursor silhouettes, drawn from simple geometric primitives.
//! No toolbar SVG or third-party cursor artwork is used. MIT OR Apache-2.0.

use egui::{Vec2, vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Glyph {
    Pen,
    Eyedropper,
    Pixel,
}

/// Signed distance to a convex polygon; vertices run clockwise in screen coordinates.
fn polygon(x: f32, y: f32, points: &[Vec2]) -> f32 {
    let mut distance = f32::INFINITY;
    let mut inside = true;
    for (i, &a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        distance = distance.min(super::seg_distance(x, y, a, b));
        inside &= (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x) >= 0.0;
    }
    if inside { -distance } else { distance }
}

pub(super) fn coverage(kind: Glyph, x: f32, y: f32, scale: f32) -> (f32, f32) {
    let (edge, crease) = match kind {
        Glyph::Pen => {
            // A tapered nib whose point is the hotspot, followed by a short barrel.
            let nib = polygon(x, y, &[vec2(0.0, 0.0), vec2(10.0, 3.0), vec2(3.0, 10.0)]);
            let barrel = polygon(x, y, &[vec2(9.0, 2.0), vec2(15.0, 8.0), vec2(8.0, 15.0), vec2(2.0, 9.0)]);
            (nib.min(barrel), super::seg_distance(x, y, vec2(0.0, 0.0), vec2(6.0, 6.0)))
        }
        Glyph::Eyedropper => {
            // Glass tube, collar, and bulb. The tube's lower end is the sampled pixel.
            let tube = polygon(x, y, &[vec2(0.0, 0.0), vec2(2.0, -5.0), vec2(10.0, -13.0), vec2(13.0, -10.0), vec2(5.0, -2.0)]);
            let collar = super::hand::capsule(x, y, [8.0, -15.0], [15.0, -8.0], 1.2);
            let bulb = super::hand::capsule(x, y, [12.0, -12.0], [15.0, -15.0], 2.4);
            (tube.min(collar).min(bulb), f32::INFINITY)
        }
        Glyph::Pixel => ((x.abs() - 1.5).max(y.abs() - 1.5), f32::INFINITY),
    };
    let (fill, _) = super::silhouette(edge, crease, 0.55, scale);
    (fill, (1.5 * scale + 0.5 - edge * scale).clamp(0.0, 1.0))
}
