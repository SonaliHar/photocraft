//! Explicit external-font fixed-box text experiment candidate.
//! This never admits a retained source into the default renderer, resolves a
//! font locator, changes a document cache, or authorizes embedding font bytes.
use photocraft_color::PixelFormat;
use photocraft_raster::Interrupt;
use photocraft_text::xcf::fixed_box_candidate::{self, Alignment, Limits, Request, Requirements};
use photocraft_text::xcf::{ExactFont, StemRendered, TextSourceError};

/// The six observed states used this precise authoring envelope. A caller must
/// supply its own authorized external ExactFont; identifiers do not grant reads.
#[derive(Clone, Copy)]
pub struct Binding<'a> {
    pub requested_output_format: PixelFormat,
    pub author_version: &'a str,
    pub profile_identity: &'a str,
    pub content: &'a str,
    pub origin: [i32; 2],
    pub alignment: Alignment,
    pub size_px: f64,
    pub box_pixels: [u32; 2],
    pub left_to_right: bool,
    pub antialias: bool,
    pub hint_none: bool,
    pub white_nonlinear_rgba8: bool,
}

/// These declarations describe the candidate boundary, not source-equivalence
/// qualification or a profile conversion. Color ingress is deliberately absent.
pub fn requirements(binding: Binding<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<Requirements, TextSourceError> {
    control.check().map_err(|_| TextSourceError::Cancelled)?;
    if binding.requested_output_format != PixelFormat::RGBA8
        || binding.author_version != "3.2.6"
        || binding.profile_identity.is_empty()
        || binding.profile_identity.len() > 128
        || binding.profile_identity.bytes().any(|b| b == 0)
        || binding.size_px.to_bits() != 32f64.to_bits()
        || binding.box_pixels != [96, 80]
        || !binding.left_to_right
        || binding.antialias
        || !binding.hint_none
        || !binding.white_nonlinear_rgba8
    {
        return Err(TextSourceError::Unsupported("unobserved fixed-box candidate boundary"));
    }
    fixed_box_candidate::requirements(
        Request { requested_output_format: binding.requested_output_format, content: binding.content, origin: binding.origin, alignment: binding.alignment },
        limits,
    )
}

/// Caller-bound candidate output only. Original source DTOs, runtime grants and
/// painted source caches remain untouched. Profile identity is an inert witness,
/// not a default sRGB assumption or an ICC transform authorization.
pub fn render_with_charge(
    font: &ExactFont<'_>,
    binding: Binding<'_>,
    limits: Limits,
    control: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
) -> Result<StemRendered, TextSourceError> {
    requirements(binding, limits, control)?;
    fixed_box_candidate::render_with_charge(
        font,
        Request { requested_output_format: binding.requested_output_format, content: binding.content, origin: binding.origin, alignment: binding.alignment },
        limits,
        control,
        charge,
    )
}

use photocraft_doc::{item_identity::ItemRef, xcf_items::SourceItem, xcf_resources::ResourceReference};
#[derive(Clone, Copy)]
pub struct SourceContext<'a> {
    pub requested_output_format: PixelFormat,
    pub author_version: &'a str,
    pub profile_identity: &'a str,
    /// Retained drawable placement supplied by the caller, never cache-derived.
    pub original_origin: [i32; 2],
}
pub struct BoundRetainedSource<'a> {
    original: &'a SourceItem,
    binding: Binding<'a>,
}
/// Strictly bind the observed AST and canonical layout without changing default
/// source admission. The font parameter is existing external runtime authority.
pub fn bind_retained_source<'a>(
    item: &'a SourceItem,
    target: ItemRef,
    resource: &ResourceReference,
    font: &ExactFont<'_>,
    context: SourceContext<'a>,
    control: &Interrupt<'_>,
) -> Result<BoundRetainedSource<'a>, TextSourceError> {
    let request = fixed_box_candidate::bind_retained_source(item, target, resource, font, context.original_origin, context.requested_output_format, control)?;
    let binding = Binding {
        requested_output_format: context.requested_output_format,
        author_version: context.author_version,
        profile_identity: context.profile_identity,
        content: request.content,
        origin: request.origin,
        alignment: request.alignment,
        size_px: 32.,
        box_pixels: [96, 80],
        left_to_right: true,
        antialias: false,
        hint_none: true,
        white_nonlinear_rgba8: true,
    };
    requirements(binding, Limits::default(), control)?;
    Ok(BoundRetainedSource { original: item, binding })
}
impl<'a> BoundRetainedSource<'a> {
    pub fn original_source(&self) -> &'a SourceItem {
        self.original
    }
    pub fn requirements(&self, limits: Limits, control: &Interrupt<'_>) -> Result<Requirements, TextSourceError> {
        requirements(self.binding, limits, control)
    }
    pub fn render_with_charge(
        &self,
        font: &ExactFont<'_>,
        limits: Limits,
        control: &Interrupt<'_>,
        charge: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
    ) -> Result<StemRendered, TextSourceError> {
        render_with_charge(font, self.binding, limits, control, charge)
    }
}
