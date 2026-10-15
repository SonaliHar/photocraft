//! Closed implicit RGB Semi-Flatten adapter. Literal encoding and native
//! encoding are independent; no source admission follows from this evaluator.
use crate::gimp_composite::{CompositionError, Result, WorkingSpace};
use photocraft_algo::gimp::point::{PointOperation, PreparedPointOperation};
use photocraft_cms::{Builtin, Profile};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    Document, FilterOutputPrecision, LayerId, SampleEncoding,
    import_compositing::GimpSpace,
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterValue},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError};

pub const EPOCH: &str = "xcf22-gimp-semi-flatten-literal-v1";
fn scope() -> CompositionError {
    CompositionError::Unsupported("closed Semi-Flatten recipe")
}
pub(crate) fn claims(node: &FilterNode) -> bool {
    node.operation.name == "gimp:semi-flatten" || node.operation.schema_epoch == EPOCH
}
/// Preserve the source encoding tag and all four raw words, including the
/// inert literal alpha. A public get_rgba conversion is not the stored value.
pub(crate) fn literal_arguments(arguments: &[FilterArgument]) -> Result<([f32; 4], SampleEncoding)> {
    let [argument] = arguments else { return Err(scope()) };
    let FilterValue::Color { components, encoding, profile: None } = &argument.value else { return Err(scope()) };
    if argument.name != "color" || components.len() != 4 || !matches!(encoding, SampleEncoding::Linear | SampleEncoding::NonlinearProfile) {
        return Err(scope());
    }
    let mut color = [0.; 4];
    for (out, value) in color.iter_mut().zip(components) {
        let ExactNumber::Float32Bits(bits) = value else { return Err(scope()) };
        *out = f32::from_bits(*bits);
        if !out.is_finite() || !(0.0..=1.0).contains(out) {
            return Err(scope());
        }
    }
    Ok((color, *encoding))
}
pub(crate) fn canonical(node: &FilterNode, stored: SampleEncoding, version: u32, extent: Rect) -> Result<()> {
    if version != 22
        || node.operation.name != "gimp:semi-flatten"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != EPOCH
        || stored != SampleEncoding::NonlinearProfile
        || extent != Rect::new(0, 0, 8, 6)
        || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
        || !matches!(node.mask, Some(ItemRef::Mask(_)))
        || node.composition.as_ref().is_none_or(|p| p.compatibility_epoch != EPOCH || p.source_mode != 62 || p.legacy || p.pass_through || p.resolved.is_some())
    {
        return Err(scope());
    }
    literal_arguments(&node.arguments)?;
    Ok(())
}
/// Fixed analytic literal preparation has no profile clone or transform owner.
/// The caller prepays three inverse analytic evaluations before construction.
pub(crate) const LITERAL_PREPARATION_WORK: u64 = 3 * 128 + 16;
pub(crate) struct PreparedSemiFlatten {
    working: WorkingSpace,
    operation: PreparedPointOperation,
}
impl PreparedSemiFlatten {
    pub(crate) fn new(node: &FilterNode, profile: &Profile, stored: SampleEncoding, version: u32, extent: Rect, control: &Interrupt<'_>) -> Result<Self> {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        canonical(node, stored, version, extent)?;
        if profile != Builtin::Srgb.profile() {
            return Err(scope());
        }
        let (mut color, encoding) = literal_arguments(&node.arguments)?;
        if encoding == SampleEncoding::Linear {
            let curve = photocraft_cms::curve::srgb_trc();
            for component in color.iter_mut().take(3) {
                control.check().map_err(|_| RasterInputError::Cancelled)?;
                *component = curve.eval_inverse64(f64::from(*component)) as f32;
                if !component.is_finite() || !(0.0..=1.0).contains(component) {
                    return Err(scope());
                }
            }
        }
        let operation = PointOperation::SemiFlatten { color }.prepare()?;
        let working = WorkingSpace::new_with_interrupt(profile, stored, GimpSpace::LinearRgb, control)?;
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        Ok(Self { working, operation })
    }
    pub(crate) fn validate_input(&self, pixel: [f64; 4]) -> Result<()> {
        if pixel.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(scope());
        }
        let color = self.working.convert([pixel[0], pixel[1], pixel[2]], true)?;
        if color.iter().any(|v| !(*v as f32).is_finite() || !(0.0..=1.0).contains(&(*v as f32))) {
            return Err(scope());
        }
        Ok(())
    }
    pub(crate) fn apply(&self, pixel: [f64; 4], enabled: bool) -> Result<[f64; 4]> {
        self.validate_input(pixel)?;
        if !enabled {
            return Ok(pixel);
        }
        // Enabled alpha zero and alpha one still take the measured transfer
        // round trip. Float input alpha is supplied to the dedicated evaluator.
        let color = self.working.convert([pixel[0], pixel[1], pixel[2]], true)?;
        let input = [color[0], color[1], color[2], pixel[3]].map(|v| f64::from(v as f32));
        let mut output = [0.; 4];
        self.operation.try_eval_pixel(&input, 3, &mut output)?;
        let native = self.working.convert([output[0], output[1], output[2]], false)?;
        let result = [native[0], native[1], native[2], output[3]];
        if result.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&result[3]) {
            return Err(scope());
        }
        Ok(result)
    }
}
pub(crate) fn qualify_context(doc: &Document, extent: Rect, node: &FilterNode) -> Result<()> {
    let source = doc.source_color_context.as_ref().ok_or_else(scope)?;
    if doc.mode != ColorMode::Rgb
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || doc.size != photocraft_doc::Size::new(8, 6)
        || extent != doc.bounds()
        || !matches!((doc.depth, source.source_precision), (SampleType::F32, 650) | (SampleType::F64, 750))
        || source.original_icc.is_some()
        || doc.xcf_state.as_ref().is_none_or(|s| s.current_filter_output_precision != FilterOutputPrecision::Canonical)
    {
        return Err(scope());
    }
    canonical(node, SampleEncoding::NonlinearProfile, 22, extent)
}
/// Complete physical zero mask proof, including default and tile padding.
/// The caller independently prepays every repeated scan before entering here.
pub(crate) fn qualify_mask(state: &photocraft_doc::XcfState, node: &FilterNode, owner: LayerId, sample: SampleType, control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    let Some(ItemRef::Mask(id)) = node.mask else { return Err(scope()) };
    let [mask] = state.filter_masks.as_slice() else { return Err(scope()) };
    if state.items.len() != 2
        || mask.id != id
        || mask.owner != node.id
        || mask.bounds != Rect::new(0, 0, 8, 6)
        || mask.surface.format() != PixelFormat::new(ColorMode::Grayscale, sample, false)
        || mask.surface.tile_count() > 1
    {
        return Err(scope());
    }
    let item = state.items.iter().find(|i| i.item == ItemRef::Mask(id)).ok_or_else(scope)?;
    let metadata = item.mask.as_ref().ok_or_else(scope)?;
    if !item.visible
        || metadata.owner != ItemRef::Layer(owner)
        || metadata.bounds != mask.bounds
        || metadata.offset != [0, 0]
        || metadata.outside_f64_bits != 0f64.to_bits()
        || !metadata.enabled
        || metadata.shown
        || metadata.editing
        || metadata.linked
    {
        return Err(scope());
    }
    let word = sample.bytes();
    let check = |bytes: &[u8]| -> Result<()> {
        if bytes.len() % word != 0 {
            return Err(scope());
        }
        for block in bytes.chunks(word * 64) {
            control.check().map_err(|_| RasterInputError::Cancelled)?;
            for raw in block.chunks_exact(word) {
                let value = photocraft_color::try_read_sample_f64(raw, sample, 0).map_err(|_| scope())?;
                if !value.is_finite() || value != 0. {
                    return Err(scope());
                }
            }
        }
        Ok(())
    };
    check(mask.surface.default_bytes())?;
    for (coordinate, tile) in mask.surface.tiles() {
        if coordinate.tx != 0 || coordinate.ty != 0 {
            return Err(scope());
        }
        check(tile.bytes())?;
    }
    Ok(())
}
