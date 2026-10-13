//! Explicit observed underline source domain; common immutable guards are reused.
use super::*;
use photocraft_text::xcf::{QualifiedUnderlinedText, underline::prepare_underlined_with_charge};
pub fn qualify_underlined_source<'a>(doc: &Document, source: &'a SourceItem, ctl: &Interrupt<'_>) -> Result<QualifiedUnderlinedText<'a>> {
    let (resource, bounds) = qualify_context(doc, source, ctl)?;
    let cache = source.cache.as_ref().ok_or_else(unsupported)?;
    if (doc.size.width, doc.size.height) != (128, 64) || (bounds.x0, bounds.y0) != (8, 4) || !matches!(cache.height, 20 | 39) {
        return Err(unsupported());
    }
    let qualified = photocraft_text::xcf::qualify_underlined_source(source, resource, [8, 4], ctl).map_err(text_error)?;
    let font = ExactFont::bundled_inter(ctl).map_err(text_error)?;
    // Stack-only outline eligibility; actual shaping/preparation is charged below.
    photocraft_text::xcf::qualify_scaled_rectilinear_geometry(&font, qualified.request, ctl).map_err(text_error)?;
    Ok(qualified)
}
fn prepare<'a>(
    doc: &Document,
    source: &'a SourceItem,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<(photocraft_text::xcf::PreparedRectilinear, QualifiedUnderlinedText<'a>)> {
    let q = qualify_underlined_source(doc, source, ctl)?;
    charge(u64::try_from(photocraft_text::fonts::INTER_REGULAR.len()).map_err(|_| unsupported())?)?;
    let font = ExactFont::bundled_inter(ctl).map_err(text_error)?;
    let mut denied = None;
    let mut resident = |n| {
        charge(n).map_err(|e| {
            denied = Some(e);
            TextSourceError::Budget
        })
    };
    // The current compositor seam supplies only resident accounting. Bound
    // kernel work separately here; this is not aggregate caller work accounting.
    let mut work = 0_u64;
    let prepared = prepare_underlined_with_charge(&font, q, 8 << 20, ctl, &mut resident, &mut |n| {
        work = work.checked_add(n).ok_or(TextSourceError::Budget)?;
        if work > 1_u64 << 36 { Err(TextSourceError::Budget) } else { Ok(()) }
    });
    if let Some(e) = denied {
        return Err(e);
    }
    Ok((prepared.map_err(text_error)?, q))
}
pub(super) fn layout_bounds(
    doc: &Document,
    source: &SourceItem,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<photocraft_geom::Rect> {
    prepare(doc, source, ctl, charge).map(|(p, _)| p.layout_rect())
}
pub(super) fn render(doc: &Document, source: &SourceItem, ctl: &Interrupt<'_>, charge: &mut dyn FnMut(u64) -> Result<()>) -> Result<StemRendered> {
    let (p, q) = prepare(doc, source, ctl, charge)?;
    let mut denied = None;
    let output = p.render_mode_with_charge(q.antialias, 8 << 20, ctl, &mut |n| {
        charge(n).map_err(|e| {
            denied = Some(e);
            TextSourceError::Budget
        })
    });
    if let Some(e) = denied {
        return Err(e);
    }
    output.map_err(text_error)
}
