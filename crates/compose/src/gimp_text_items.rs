//! Bounded exact-authority text regeneration; retained source pixels are evidence.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::{
    ColorMode, Document, PixelFormat, SampleEncoding, SampleType,
    item_identity::ItemRef,
    xcf_items::{EditableSource, SourceItem},
};
use photocraft_raster::{Interrupt, RasterInputError};
use photocraft_text::xcf::{ExactFont, ScaledText, StemRendered, StemText, TextSourceError};

mod underline;
use photocraft_text::xcf::runtime_font_input::PreparedRuntimeFont;
pub use underline::{qualify_underlined_source, qualify_underlined_source_with_authorized_font};

/// Explicit current-source dispatch; old markup-free APIs remain closed.
pub fn qualify_current_source(doc: &Document, source: &SourceItem, ctl: &Interrupt<'_>) -> Result<()> {
    if matches!(&source.source, EditableSource::Text { markup: Some(_), .. }) {
        qualify_underlined_source(doc, source, ctl).map(|_| ())
    } else {
        qualify_layout_source(doc, source, ctl).map(|_| ())
    }
}

pub fn qualify_current_source_with_authorized_font(doc: &Document, source: &SourceItem, font: &PreparedRuntimeFont<'_>, ctl: &Interrupt<'_>) -> Result<()> {
    if matches!(&source.source, EditableSource::Text { markup: Some(_), .. }) {
        qualify_underlined_source_with_authorized_font(doc, source, font, ctl).map(|_| ())
    } else {
        qualify_layout_source_with_authorized_font(doc, source, font, ctl).map(|_| ())
    }
}

fn unsupported() -> CompositionError {
    CompositionError::Unsupported("unqualified exact-font text context")
}
fn text_error(error: TextSourceError) -> CompositionError {
    if error == TextSourceError::Cancelled { RasterInputError::Cancelled.into() } else { unsupported() }
}
pub fn qualify_source<'a>(doc: &Document, source: &'a SourceItem, ctl: &Interrupt<'_>) -> Result<StemText<'a>> {
    let (request, _) = qualify_scaled_source(doc, source, ctl)?;
    if request.size_px != 32.0 {
        return Err(unsupported());
    }
    Ok(StemText { content: request.content, origin: request.origin })
}
pub fn qualify_scaled_source<'a>(
    doc: &Document,
    source: &'a SourceItem,
    ctl: &Interrupt<'_>,
) -> Result<(ScaledText<'a>, photocraft_text::xcf::RectilinearAntialias)> {
    let (request, antialias, color) = qualify_colored_source(doc, source, ctl)?;
    if color.encoding != photocraft_text::xcf::TextColorEncoding::Nonlinear || color.rgba_f32_bits != [1_f32.to_bits(); 4] {
        return Err(unsupported());
    }
    Ok((request, antialias))
}
pub fn qualify_colored_source<'a>(
    doc: &Document,
    source: &'a SourceItem,
    ctl: &Interrupt<'_>,
) -> Result<(ScaledText<'a>, photocraft_text::xcf::RectilinearAntialias, photocraft_text::xcf::TextColor)> {
    if matches!(source.source, EditableSource::Text { current_origin: Some(_), .. }) {
        return Err(unsupported());
    }
    let (request, antialias, color, spacing) = qualify_spaced_colored_source(doc, source, ctl)?;
    if spacing != 0. {
        return Err(unsupported());
    }
    Ok((request, antialias, color))
}
/// Return operative spacing with the qualified source; legacy zero-spacing
/// consumers remain closed through `qualify_colored_source`.
pub fn qualify_spaced_colored_source<'a>(
    doc: &Document,
    source: &'a SourceItem,
    ctl: &Interrupt<'_>,
) -> Result<(ScaledText<'a>, photocraft_text::xcf::RectilinearAntialias, photocraft_text::xcf::TextColor, f32)> {
    let qualified = qualify_layout_source(doc, source, ctl)?;
    let photocraft_text::xcf::RectilinearLineLayout::SingleLine { letter_spacing_px } = qualified.layout else { return Err(unsupported()) };
    Ok((qualified.request, qualified.antialias, qualified.color, letter_spacing_px))
}
/// Explicit layout opt-in; legacy single-line consumers remain closed.
pub fn qualify_layout_source<'a>(doc: &Document, source: &'a SourceItem, ctl: &Interrupt<'_>) -> Result<photocraft_text::xcf::QualifiedRectilinearText<'a>> {
    qualify_layout_source_impl(doc, source, None, ctl)
}
pub fn qualify_layout_source_with_authorized_font<'a>(
    doc: &Document,
    source: &'a SourceItem,
    font: &PreparedRuntimeFont<'_>,
    ctl: &Interrupt<'_>,
) -> Result<photocraft_text::xcf::QualifiedRectilinearText<'a>> {
    qualify_layout_source_impl(doc, source, Some(font.exact_font()), ctl)
}
fn qualify_layout_source_impl<'a>(
    doc: &Document,
    source: &'a SourceItem,
    authorized: Option<&ExactFont<'_>>,
    ctl: &Interrupt<'_>,
) -> Result<photocraft_text::xcf::QualifiedRectilinearText<'a>> {
    let (resource, bounds) = qualify_context(doc, source, ctl)?;
    let qualified =
        photocraft_text::xcf::qualify_placed_layout_colored_rectilinear_source(source, resource, [bounds.x0, bounds.y0], ctl).map_err(text_error)?;
    if !matches!(
        (doc.size.height, qualified.layout),
        (64, photocraft_text::xcf::RectilinearLineLayout::SingleLine { .. }) | (128, photocraft_text::xcf::RectilinearLineLayout::TwoLines { .. })
    ) {
        return Err(unsupported());
    }
    let fallback;
    let font = if let Some(font) = authorized {
        font
    } else {
        fallback = ExactFont::bundled_inter(ctl).map_err(text_error)?;
        &fallback
    };
    photocraft_text::xcf::qualify_layout_rectilinear_geometry(&font, qualified, ctl).map_err(text_error)?;
    doc.working_profile_interpretation.validate_with(doc.icc_profile.as_deref().map(Vec::as_slice), doc.mode).map_err(|_| unsupported())?;
    Ok(qualified)
}

/// Shared immutable source/owner/cache/current-raster and profile context.
fn qualify_context<'a>(
    doc: &'a Document,
    source: &SourceItem,
    ctl: &Interrupt<'_>,
) -> Result<(&'a photocraft_doc::xcf_resources::ResourceReference, photocraft_geom::Rect)> {
    ctl.check().map_err(|_| RasterInputError::Cancelled)?;
    let state = doc.xcf_state.as_ref().ok_or_else(unsupported)?;
    let ItemRef::Layer(id) = source.owner else { return Err(unsupported()) };
    let layer = doc.layer(id).ok_or_else(unsupported)?;
    let EditableSource::Text { font, source_modified: false, .. } = source.source else { return Err(unsupported()) };
    let mut resources = state.resources.iter().filter(|resource| resource.id == font);
    let resource = resources.next().ok_or_else(unsupported)?;
    let mut owners = state.items.iter().filter(|item| item.item == source.owner);
    let owner = owners.next().ok_or_else(unsupported)?;
    let bounds = owner.raster_bounds.ok_or_else(unsupported)?;
    let cache = source.cache.as_ref().ok_or_else(unsupported)?;
    if resources.next().is_some()
        || owners.next().is_some()
        || state.provenance.format_version != 19
        || !matches!((doc.size.width, doc.size.height), (128, 64) | (128, 128))
        || doc.mode != ColorMode::Rgb
        || doc.depth != SampleType::U8
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || !matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. })
        || doc.source_color_context.as_ref().is_none_or(|context| context.source_precision != 150 || context.original_icc.is_some())
        || doc.layers.len() != 1
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || state.source_items.len() != 1
        || state.resources.len() != 1
        || !state.filter_stacks.is_empty()
        || layer.mask.is_some()
        || !layer.visible
        || layer.opacity.to_bits() != 1_f32.to_bits()
        || layer.fill_opacity.to_bits() != 1_f32.to_bits()
        || !matches!((bounds.x0, bounds.y0), (8, 4) | (11, 2) | (-5, 7))
        || bounds.height() != cache.height
        || bounds.width() != cache.width
        || !match doc.size.height {
            64 => matches!(cache.height, 20 | 30 | 39 | 49),
            128 => bounds.x0 == 8 && bounds.y0 == 4 && matches!(cache.height, 40 | 42 | 44 | 78 | 80 | 82 | 98 | 100 | 102),
            _ => false,
        }
        || !(1..=512).contains(&cache.width)
        || cache.format != PixelFormat::RGBA8
        || owner.composition.as_ref().is_none_or(|policy| {
            policy.source_mode != 28
                || policy.legacy
                || policy.pass_through
                || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (0, -1, -1)
        })
    {
        return Err(unsupported());
    }
    cache.validate_with(ctl).map_err(|error| if error.code == "cancelled" { RasterInputError::Cancelled.into() } else { unsupported() })?;
    let surface = layer.surface().ok_or_else(unsupported)?;
    let min_tx = bounds.x0.div_euclid(256);
    let min_ty = bounds.y0.div_euclid(256);
    let max_tx = bounds.x1.checked_sub(1).and_then(|edge| edge.div_euclid(256).checked_add(1)).ok_or_else(unsupported)?;
    let max_ty = bounds.y1.checked_sub(1).and_then(|edge| edge.div_euclid(256).checked_add(1)).ok_or_else(unsupported)?;
    let tile_count =
        max_tx.checked_sub(min_tx).and_then(|columns| max_ty.checked_sub(min_ty).and_then(|rows| columns.checked_mul(rows))).ok_or_else(unsupported)?;
    if !(1..=3).contains(&tile_count) {
        return Err(unsupported());
    }
    let scan_x0 = min_tx.checked_mul(256).ok_or_else(unsupported)?;
    let scan_y0 = min_ty.checked_mul(256).ok_or_else(unsupported)?;
    let scan_x1 = max_tx.checked_mul(256).ok_or_else(unsupported)?;
    let scan_y1 = max_ty.checked_mul(256).ok_or_else(unsupported)?;
    if surface.format() != PixelFormat::RGBA8
        || surface.default_bytes() != [0, 0, 0, 0]
        || surface.tiles().any(|(coordinate, _)| coordinate.ty < min_ty || coordinate.ty >= max_ty || coordinate.tx < min_tx || coordinate.tx >= max_tx)
    {
        return Err(unsupported());
    }
    // Raster edits must not disappear behind an authoritative source evaluator.
    // At most three bounded tiles contain the original cache and transparent exterior.
    let width = usize::try_from(cache.width).map_err(|_| unsupported())?;
    let mut pixel = [0_u8; 4];
    for y in scan_y0..scan_y1 {
        for x in scan_x0..scan_x1 {
            if x % 64 == 0 {
                ctl.check().map_err(|_| RasterInputError::Cancelled)?;
            }
            surface.try_read_pixel_bytes(x, y, &mut pixel)?;
            let expected = if bounds.contains(x, y) {
                let row = usize::try_from(y - bounds.y0).map_err(|_| unsupported())?;
                let column = usize::try_from(x - bounds.x0).map_err(|_| unsupported())?;
                let offset = row.checked_mul(width).and_then(|n| n.checked_add(column)).and_then(|n| n.checked_mul(4)).ok_or_else(unsupported)?;
                cache.bytes.get(offset..offset.checked_add(4).ok_or_else(unsupported)?).ok_or_else(unsupported)?
            } else {
                &[0, 0, 0, 0]
            };
            if pixel.as_slice() != expected {
                return Err(unsupported());
            }
        }
    }
    doc.working_profile_interpretation.validate_with(doc.icc_profile.as_deref().map(Vec::as_slice), doc.mode).map_err(|_| unsupported())?;
    Ok((resource, bounds))
}

/// Current layout geometry without replacing the original source raster cache.
/// The caller accounts temporary shaping/font preparation until this call returns.
pub fn current_source_layout_bounds(
    doc: &Document,
    source: &SourceItem,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<photocraft_geom::Rect> {
    current_source_layout_bounds_impl(doc, source, None, ctl, charge)
}
pub fn current_source_layout_bounds_with_authorized_font(
    doc: &Document,
    source: &SourceItem,
    font: &PreparedRuntimeFont<'_>,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<photocraft_geom::Rect> {
    current_source_layout_bounds_impl(doc, source, Some(font.exact_font()), ctl, charge)
}
fn current_source_layout_bounds_impl(
    doc: &Document,
    source: &SourceItem,
    authorized: Option<&ExactFont<'_>>,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<photocraft_geom::Rect> {
    if matches!(&source.source, EditableSource::Text { markup: Some(_), .. }) {
        return if authorized.is_some() {
            underline::layout_bounds_with_font(doc, source, authorized, ctl, charge)
        } else {
            underline::layout_bounds(doc, source, ctl, charge)
        };
    }
    if authorized.is_none() {
        charge(u64::try_from(photocraft_text::fonts::INTER_REGULAR.len()).map_err(|_| unsupported())?)?;
    }
    let qualified = qualify_layout_source_impl(doc, source, authorized, ctl)?;
    let fallback;
    let font = if let Some(font) = authorized {
        font
    } else {
        fallback = ExactFont::bundled_inter(ctl).map_err(text_error)?;
        &fallback
    };
    let mut denied = None;
    let mut account = |bytes| {
        charge(bytes).map_err(|error| {
            denied = Some(error);
            TextSourceError::Budget
        })
    };
    let prepared = photocraft_text::xcf::prepare_qualified_rectilinear_with_charge(&font, qualified, 8 << 20, ctl, &mut account);
    if let Some(error) = denied {
        return Err(error);
    }
    prepared.map(|prepared| prepared.layout_rect()).map_err(text_error)
}

pub fn render_source(doc: &Document, source: &SourceItem, ctl: &Interrupt<'_>, charge: &mut dyn FnMut(u64) -> Result<()>) -> Result<StemRendered> {
    render_source_impl(doc, source, None, ctl, charge)
}
pub fn render_source_with_authorized_font(
    doc: &Document,
    source: &SourceItem,
    font: &PreparedRuntimeFont<'_>,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<StemRendered> {
    render_source_impl(doc, source, Some(font.exact_font()), ctl, charge)
}
fn render_source_impl(
    doc: &Document,
    source: &SourceItem,
    authorized: Option<&ExactFont<'_>>,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<StemRendered> {
    if matches!(&source.source, EditableSource::Text { markup: Some(_), .. }) {
        return if authorized.is_some() {
            underline::render_with_font(doc, source, authorized, ctl, charge)
        } else {
            underline::render(doc, source, ctl, charge)
        };
    }
    let qualified = qualify_layout_source_impl(doc, source, authorized, ctl)?;
    let color = qualified.color;
    // Borrowed compiled authority is charged conservatively for this job too.
    if authorized.is_none() {
        charge(u64::try_from(photocraft_text::fonts::INTER_REGULAR.len()).map_err(|_| unsupported())?)?;
    }
    // Stored encoding is operative data. Use the bound current implicit profile
    // through CMS; neither the original raster nor a guessed sRGB curve selects it.
    let bound = doc.icc_profile.as_ref().ok_or_else(unsupported)?;
    charge(u64::try_from(bound.len()).ok().and_then(|n| n.checked_mul(256)).and_then(|n| n.checked_add(65536)).ok_or_else(unsupported)?)?;
    let output_profile =
        photocraft_cms::xcf_color::effective_implicit_profile(bound, photocraft_cms::ColorSpace::Rgb, photocraft_cms::xcf_color::Encoding::NonlinearProfile)?;
    let encoding = match color.encoding {
        photocraft_text::xcf::TextColorEncoding::Linear => photocraft_cms::xcf_color::Encoding::Linear,
        photocraft_text::xcf::TextColorEncoding::Nonlinear => photocraft_cms::xcf_color::Encoding::NonlinearProfile,
    };
    let input_profile = photocraft_cms::xcf_color::effective_profile_from_definition(&output_profile, encoding)?;
    let transform = photocraft_cms::Transform::new(&input_profile, &output_profile, photocraft_cms::Intent::RelativeColorimetric, false)?;
    let rgba = color.rgba_f32_bits.map(f32::from_bits);
    let mut converted = [0_f64; 3];
    let input = [f64::from(rgba[0]), f64::from(rgba[1]), f64::from(rgba[2])];
    transform.try_eval_f64(&input, &mut converted)?;
    // Independent alpha-only saved edits retain stored alpha while rendering an
    // opaque glyph fill. Coverage still comes from the measured glyph rasterizer.
    let encoded = [converted[0].clamp(0.0, 1.0) as f32, converted[1].clamp(0.0, 1.0) as f32, converted[2].clamp(0.0, 1.0) as f32, 1.0];
    let fallback;
    let font = if let Some(font) = authorized {
        font
    } else {
        fallback = ExactFont::bundled_inter(ctl).map_err(text_error)?;
        &fallback
    };
    let mut denied = None;
    let mut render_charge = |bytes| {
        charge(bytes).map_err(|error| {
            denied = Some(error);
            TextSourceError::Budget
        })
    };
    let output = photocraft_text::xcf::prepare_qualified_rectilinear_with_charge(&font, qualified, 8 << 20, ctl, &mut render_charge)
        .and_then(|prepared| prepared.render_color_with_charge(qualified.antialias, encoded, 8 << 20, ctl, &mut render_charge));
    if let Some(error) = denied {
        return Err(error);
    }
    output.map_err(text_error)
}
