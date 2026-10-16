//! The lettering geometry, and the unit conversions under it.
//!
//! Every number here is a behaviour of **TyperTools 1.4.9** (a Photoshop CEP panel, MIT, © 2017
//! Swirt) read off its published source: how it turns a marquee drawn around a balloon into a text
//! box, how it nudges the type size, and how it keeps a block of type where it was. The port is
//! clean-room — the behaviour and the constants are observations, no code is copied — and each
//! constant has a test, because this is the part a letterer notices drifting.
//!
//! One model difference is worth stating up front. Photoshop stores a layer's type size in the
//! layer's own ruler unit, and TyperTools' default is pixels, so a size of `14` there is 14 device
//! pixels. PhotoCraft's type is sized in **points** (`CharStyle::size_pt`), so every size that
//! crosses the boundary is converted at the document's resolution: `px → pt` is `px / dpi * 72`,
//! which is TyperTools' own conversion. A style library written on a 300 dpi document therefore
//! letters the same *physical* size on a 72 dpi one, which is what a letterer expects.

/// How much the marquee is pulled in before it is used, in document pixels.
///
/// TyperTools contracts the selection by 10 px and then measures it, so the balloon's outline (and
/// the marquee's own soft edge) does not decide the text box.
pub const SELECTION_CONTRACT_PX: f64 = 10.0;

/// A selection smaller than this is not a balloon, in square pixels.
pub const MIN_SELECTION_AREA: f64 = 200.0;

/// A new text box takes this much of the (contracted) selection's width.
pub const BOX_WIDTH_FACTOR: f64 = 0.9;

/// …and this many times its height: the box is deliberately far too tall to begin with, so the text
/// lays out on one line, and is shrunk around the result afterwards.
pub const BOX_HEIGHT_FACTOR: f64 = 15.0;

/// Growing the type grows the box with it: the bottom by this factor, the right by this one.
pub const BOX_GROW_BOTTOM: f64 = 1.12;
pub const BOX_GROW_RIGHT: f64 = 1.06;

/// The size buttons move the type by one document pixel, expressed in points at the document's
/// resolution.
pub const SIZE_STEP_PX: f64 = 1.0;

/// Extra room left under the last line when a box is refitted around its text, in document pixels.
pub const BOX_PAD_PX: f64 = 2.0;

/// The size used when neither a style nor the layer says (points, at 72 dpi).
pub const FALLBACK_SIZE_PT: f64 = 12.0;

/// PhotoCraft's own limits on a type size, from `type.create`'s parameter table. A nudge that would
/// leave them is refused rather than clamped silently.
pub const MIN_SIZE_PT: f64 = 0.1;
pub const MAX_SIZE_PT: f64 = 1296.0;

/// The engine's limits on the character and paragraph values a style can carry, from
/// `crates/engine/src/type_cmds.rs`. A style written elsewhere may hold anything at all; these are
/// what the engine would reject, so a value outside them is brought inside rather than sent and
/// refused — a letterer who typed `-1` for a leading meant "not this", not "error".
pub const LEADING_MIN_PT: f64 = 0.01;
pub const KERN_MIN: f64 = -1000.0;
pub const KERN_MAX: f64 = 10_000.0;
pub const TRACKING_MIN: f64 = -1000.0;
pub const TRACKING_MAX: f64 = 10_000.0;
pub const BASELINE_SHIFT_MAX_PT: f64 = 5000.0;

/// An axis-aligned rectangle in document pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    /// From `[x0, y0, x1, y1]`, as `type.info`'s `bounds` and the engine's content bounds are.
    pub fn from_bounds(b: [f64; 4]) -> Rect {
        Rect { x: b[0], y: b[1], w: b[2] - b[0], h: b[3] - b[1] }
    }

    /// From `[x, y, w, h]`, as `doc.info`'s `selectionBounds` is.
    pub fn from_xywh(b: [f64; 4]) -> Rect {
        Rect { x: b[0], y: b[1], w: b[2], h: b[3] }
    }

    pub fn centre(self) -> [f64; 2] {
        [self.x + self.w / 2.0, self.y + self.h / 2.0]
    }

    pub fn area(self) -> f64 {
        self.w * self.h
    }

    /// Pulled in by `by` on every side; never past nothing.
    pub fn contracted(self, by: f64) -> Rect {
        let w = (self.w - 2.0 * by).max(0.0);
        let h = (self.h - 2.0 * by).max(0.0);
        Rect { x: self.x + by, y: self.y + by, w, h }
    }

    /// Grown for bigger type: the right and bottom edges move, the origin does not.
    pub fn grown(self) -> Rect {
        Rect { x: self.x, y: self.y, w: self.w * BOX_GROW_RIGHT, h: self.h * BOX_GROW_BOTTOM }
    }

    /// The box a new block of type is given, in the contracted selection.
    pub fn box_for(self) -> Rect {
        Rect { x: self.x, y: self.y, w: self.w * BOX_WIDTH_FACTOR, h: self.h * BOX_HEIGHT_FACTOR }
    }

    /// The same box, shrunk around text that laid out `content_h` pixels tall, leaving room for the
    /// size plus [`BOX_PAD_PX`] under it.
    pub fn refitted(self, content_h: f64, size_px: f64) -> Rect {
        Rect { x: self.x, y: self.y, w: self.w, h: (content_h + size_px + BOX_PAD_PX).max(1.0) }
    }

    pub fn as_box(self) -> [f64; 4] {
        [self.x, self.y, self.w.max(1.0), self.h.max(1.0)]
    }
}

/// Why an action could not be carried out on the current selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Nothing is selected.
    NoSelection,
    /// Something is selected, but it is too small to hold lettering.
    SmallSelection,
}

impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Refusal::NoSelection => "Select the balloon first: draw a marquee around it, then Create.",
            Refusal::SmallSelection => "That selection is too small to letter: it has to be at least 200 px² after the 10 px inset.",
        }
    }
}

/// The marquee as TyperTools uses it: nothing selected, or too small, is a refusal with a reason.
pub fn checked_selection(bounds: Option<[f64; 4]>) -> Result<Rect, Refusal> {
    let selection = Rect::from_xywh(bounds.ok_or(Refusal::NoSelection)?);
    let contracted = selection.contracted(SELECTION_CONTRACT_PX);
    if contracted.area() < MIN_SELECTION_AREA {
        return Err(Refusal::SmallSelection);
    }
    Ok(contracted)
}

/// Pixels to points, at the document's resolution: `px / dpi * 72`.
pub fn px_to_pt(px: f64, dpi: f64) -> f64 {
    if !(dpi.is_finite() && dpi > 0.0) {
        return px;
    }
    px / dpi * 72.0
}

/// Points to pixels, at the document's resolution.
pub fn pt_to_px(pt: f64, dpi: f64) -> f64 {
    if !(dpi.is_finite() && dpi > 0.0) {
        return pt;
    }
    pt * dpi / 72.0
}

/// One press of a size button: one document pixel, in points.
pub fn size_step_pt(dpi: f64) -> f64 {
    px_to_pt(SIZE_STEP_PX, dpi)
}

/// A length in one of Photoshop's `typeUnit`s, in points. PhotoCraft's type is sized in points, so
/// a style library written in any unit can be applied at the size its author meant.
pub fn length_to_pt(value: f64, unit: Option<&str>, dpi: f64) -> f64 {
    match unit {
        Some("pixelsUnit") => px_to_pt(value, dpi),
        Some("mmUnit") => value * 72.0 / 25.4,
        Some("inchesUnit") => value * 72.0,
        Some("picasUnit") => value * 12.0,
        // `pointsUnit`, anything unknown, and no unit at all: points, which is PhotoCraft's own unit
        // and what a Photoshop layer is measured in when its ruler is set to points.
        _ => value,
    }
}

/// Points in the unit a style library stores, the inverse of [`length_to_pt`].
pub fn pt_to_length(pt: f64, unit: Option<&str>, dpi: f64) -> f64 {
    match unit {
        Some("pixelsUnit") => pt_to_px(pt, dpi),
        Some("mmUnit") => pt * 25.4 / 72.0,
        Some("inchesUnit") => pt / 72.0,
        Some("picasUnit") => pt / 12.0,
        _ => pt,
    }
}

/// How far to move a block so its centre lands on `target`, from where it is now.
pub fn centre_offset(current: Rect, target: [f64; 2]) -> [f64; 2] {
    let c = current.centre();
    [target[0] - c[0], target[1] - c[1]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_is_inset_ten_pixels_and_a_small_one_is_refused() {
        // A 200 × 160 marquee: contracted to 180 × 140 = 25 200 px², and the box takes 90 % of the
        // width — the marble-and-balloon case TyperTools was written for.
        let r = checked_selection(Some([100.0, 50.0, 200.0, 160.0])).unwrap();
        assert_eq!(r, Rect::new(110.0, 60.0, 180.0, 140.0));
        assert_eq!(r.box_for(), Rect::new(110.0, 60.0, 162.0, 2100.0));

        assert_eq!(checked_selection(None), Err(Refusal::NoSelection));
        // 30 × 30 contracts to 10 × 10 = 100 px², under the 200 px² floor.
        assert_eq!(checked_selection(Some([0.0, 0.0, 30.0, 30.0])), Err(Refusal::SmallSelection));
        // A long thin selection is not a balloon either: 400 × 5 contracts to 380 × 0 = nothing.
        assert_eq!(checked_selection(Some([0.0, 0.0, 400.0, 5.0])), Err(Refusal::SmallSelection));
    }

    #[test]
    fn a_box_is_shrunk_around_its_text() {
        let b = Rect::new(0.0, 0.0, 180.0, 2100.0);
        // Two lines 20 px tall, 14 pt type at 300 dpi (58.33 px), plus the 2 px pad.
        let fit = b.refitted(40.0, pt_to_px(14.0, 300.0));
        assert_eq!(fit.w, 180.0, "the width the box was given is kept");
        assert!((fit.h - (40.0 + 58.333_336 + 2.0)).abs() < 0.001, "{fit:?}");
        assert_eq!(b.as_box()[3], 2100.0);
    }

    #[test]
    fn growing_the_type_grows_the_box_from_its_origin() {
        let grown = Rect::new(10.0, 20.0, 100.0, 50.0).grown();
        assert_eq!(grown.x, 10.0);
        assert_eq!(grown.y, 20.0);
        assert!((grown.w - 106.0).abs() < 1e-9, "right × 1.06");
        assert!((grown.h - 56.0).abs() < 1e-9, "bottom × 1.12");
    }

    #[test]
    fn the_size_step_is_one_document_pixel() {
        // 72 dpi: a pixel is a point. 300 dpi: it is a quarter of one.
        assert!((size_step_pt(72.0) - 1.0).abs() < 1e-9);
        assert!((size_step_pt(300.0) - 0.24).abs() < 1e-9);
        // A nonsense resolution must not produce a nonsense size.
        assert_eq!(size_step_pt(0.0), SIZE_STEP_PX);
    }

    #[test]
    fn units_round_trip_through_points() {
        for unit in [Some("pixelsUnit"), Some("pointsUnit"), Some("mmUnit"), Some("inchesUnit"), Some("picasUnit"), None] {
            let pt = length_to_pt(24.0, unit, 300.0);
            assert!((pt_to_length(pt, unit, 300.0) - 24.0).abs() < 1e-9, "{unit:?}");
        }
        // Photoshop's own pixel case, at 300 dpi: 300 px is 72 pt — an inch.
        assert!((length_to_pt(300.0, Some("pixelsUnit"), 300.0) - 72.0).abs() < 1e-9);
        // 25.4 mm is an inch; so are 6 picas of 12 pt each.
        assert!((length_to_pt(25.4, Some("mmUnit"), 300.0) - 72.0).abs() < 1e-9);
        assert!((length_to_pt(6.0, Some("picasUnit"), 300.0) - 72.0).abs() < 1e-9);
    }

    #[test]
    fn a_block_is_moved_by_the_difference_between_two_centres() {
        let current = Rect::new(0.0, 0.0, 10.0, 20.0);
        assert_eq!(centre_offset(current, [100.0, 100.0]), [95.0, 90.0]);
        assert_eq!(centre_offset(current, current.centre()), [0.0, 0.0]);
    }
}
