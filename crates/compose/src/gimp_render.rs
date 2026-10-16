//! Checked authoritative f64 document consumer for the currently qualified GIMP slice.
use crate::gimp_composite::{CompositionError, Result, WorkingSpace, coverage, space};
use photocraft_cms::{Profile, WideWork};
use photocraft_color::{ColorMode, blend::BlendMode};
use photocraft_doc::{
    Document, Layer, LayerContent, SampleEncoding,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_geom::Rect;
use photocraft_text::xcf::runtime_font_input::{self, PreparedRuntimeFont};
#[path = "gimp_accounted.rs"]
mod accounted;
use crate::gimp_group_gaussian as group_gaussian;
#[path = "gimp_normal_geometry.rs"]
mod normal_geometry;
#[path = "gimp_normal_ordinary.rs"]
mod normal_ordinary;
#[path = "gimp_plain_box.rs"]
mod plain_box;
#[path = "gimp_plain_brightness_contrast.rs"]
mod plain_brightness_contrast;
#[path = "gimp_plain_gaussian.rs"]
mod plain_gaussian;
#[path = "gimp_plain_median.rs"]
mod plain_median;
#[path = "gimp_plain_motion.rs"]
mod plain_motion;
#[path = "gimp_plain_normal.rs"]
mod plain_normal;
#[path = "gimp_plain_point.rs"]
mod plain_point;
#[path = "gimp_prepare_control.rs"]
mod prepare_control;
#[path = "gimp_text_budgeted.rs"]
mod text_budgeted;
#[path = "gimp_scene_budget.rs"]
pub mod scene_budget;

/// Caller-owned aggregate accounting for ordinary-scene preparation.
/// A successful reservation must remain charged until its returned lease drops.
pub trait PreparationBudget {
    type Lease;
    fn reserve_prepared_bytes(&mut self, bytes: u64) -> Result<Self::Lease>;
    fn consume_preparation_work(&mut self, units: u64) -> Result<()>;
}

/// Keeps preparation and source accounting alive across every rendered band.
/// `source_lease` may borrow an already charged owner. An inert marker is valid
/// only while an external owner retains that source reservation for this lifetime.
pub struct LeasedPreparedRenderer<'a, S, P> {
    renderer: PreparedRenderer<'a>,
    _source_lease: S,
    _prepared_lease: P,
}
impl<'a, S, P> LeasedPreparedRenderer<'a, S, P> {
    pub fn renderer(&self) -> &PreparedRenderer<'a> {
        &self.renderer
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WideBuffer {
    pub rect: Rect,
    pub px: Vec<[f64; 4]>,
}
impl WideBuffer {
    /// Explicit derived display conversion; authoritative evaluation remains in f64.
    pub fn try_display(&self) -> Result<crate::Buffer> {
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(self.px.len()).map_err(|_| CompositionError::Invalid("display allocation denied"))?;
        for pixel in &self.px {
            if pixel.iter().any(|v| !v.is_finite() || !(*v as f32).is_finite()) {
                return Err(CompositionError::Invalid("display conversion exceeds f32 range"));
            }
            pixels.push(pixel.map(|v| v as f32));
        }
        Ok(crate::Buffer { rect: self.rect, px: pixels })
    }
}
struct Prepared<'a> {
    layer: &'a Layer,
    working: usize,
    operator: GimpOperator,
    mode: crate::gimp_blend::GimpBlendMode,
    blend_working: usize,
    children: Vec<Prepared<'a>>,
    pass_through: bool,
    isolated_seed: bool,
    filters: Option<crate::gimp_neighborhood::OrderedStack>,
    mask_state: Option<photocraft_doc::xcf_state::MaskState>,
    floating_extent: Option<Rect>,
    group_gaussian: Option<group_gaussian::GroupGaussian>,
    vector_source: Option<&'a photocraft_doc::xcf_items::SourceItem>,
    text_source: Option<&'a photocraft_doc::xcf_items::SourceItem>,
    finite_normal_source: bool,
}
enum HeldTextFont<'a> {
    Borrowed(&'a PreparedRuntimeFont<'a>),
    Prepared(PreparedRuntimeFont<'a>),
}
impl HeldTextFont<'_> {
    fn handle(&self) -> &PreparedRuntimeFont<'_> {
        match self {
            Self::Borrowed(font) => font,
            Self::Prepared(font) => font,
        }
    }
}
struct RenderJob<'a, 'b> {
    supplied_font: Option<&'a PreparedRuntimeFont<'a>>,
    text_font: Option<HeldTextFont<'a>>,
    font_bytes: u64,
    cache: crate::gimp_neighborhood::TileCache,
    interrupt: &'a photocraft_raster::Interrupt<'b>,
    groups: Vec<(photocraft_doc::LayerId, photocraft_raster::Surface)>,
    doc: &'a Document,
    vectors: Vec<(photocraft_doc::LayerId, crate::gimp_vector_items::RegeneratedVector)>,
    vector_bytes: u64,
    texts: Vec<(photocraft_doc::LayerId, photocraft_text::xcf::StemRendered)>,
    text_bytes: u64,
}
impl RenderJob<'_, '_> {
    fn finish(&mut self, poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<()> {
        self.vectors.clear();
        self.texts.clear();
        self.text_font.take();
        let font_bytes = std::mem::take(&mut self.font_bytes);
        let fonts = if font_bytes == 0 { Ok(()) } else { poll(WideWork::ReleaseBytes(font_bytes)) };
        let texts = poll(WideWork::ReleaseBytes(std::mem::take(&mut self.text_bytes)));
        let vectors = poll(WideWork::ReleaseBytes(std::mem::take(&mut self.vector_bytes)));
        let filters = self.cache.finish(poll);
        vectors.and(texts).and(fonts).and(filters)
    }
}
fn regenerate_vector(
    job: &mut RenderJob<'_, '_>,
    id: photocraft_doc::LayerId,
    source: &photocraft_doc::xcf_items::SourceItem,
    poll: &mut impl FnMut(WideWork) -> Result<()>,
) -> Result<()> {
    if job.vectors.iter().any(|(cached, _)| *cached == id) {
        return Ok(());
    }
    let meter = std::cell::RefCell::new(poll);
    let held = std::cell::Cell::new(0_u64);
    let mut charge = |bytes: usize| {
        let bytes = u64::try_from(bytes).map_err(|_| CompositionError::Invalid("vector resident size"))?;
        let next = held.get().checked_add(bytes).ok_or(CompositionError::Invalid("vector resident overflow"))?;
        let mut callback = meter.try_borrow_mut().map_err(|_| CompositionError::Invalid("vector accounting already borrowed"))?;
        (**callback)(WideWork::ReserveBytes(bytes))?;
        held.set(next);
        Ok(())
    };
    let mut work = |units| {
        let mut callback = meter.try_borrow_mut().map_err(|_| CompositionError::Invalid("vector accounting already borrowed"))?;
        (**callback)(WideWork::SampleUnits(units))
    };
    let output = crate::gimp_vector_items::render_source(job.doc, source, job.interrupt, &mut charge, &mut work);
    job.vector_bytes = job.vector_bytes.checked_add(held.get()).ok_or(CompositionError::Invalid("vector resident overflow"))?;
    job.vectors.try_reserve(1).map_err(|_| CompositionError::Invalid("vector job cache allocation"))?;
    job.vectors.push((id, output?));
    Ok(())
}

fn prepare_text_font<'a, 'b>(job: &mut RenderJob<'a, 'b>, poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<()> {
    if job.text_font.is_some() {
        return Ok(());
    }
    prepare_control::checkpoint(job.interrupt)?;
    if let Some(font) = job.supplied_font {
        let requirements = font.requirements();
        let next = job.font_bytes.checked_add(requirements.peak_bytes).ok_or(CompositionError::Invalid("font resident overflow"))?;
        poll(WideWork::ReserveBytes(requirements.peak_bytes))?;
        job.font_bytes = next;
        poll(WideWork::SampleUnits(requirements.work_units))?;
        prepare_control::checkpoint(job.interrupt)?;
        job.text_font = Some(HeldTextFont::Borrowed(font));
        return Ok(());
    }
    let meter = std::cell::RefCell::new(poll);
    let held = std::cell::Cell::new(0u64);
    let denied = std::cell::RefCell::new(None);
    let output = runtime_font_input::prepare_with_charge(
        runtime_font_input::RuntimeInput { bytes: &photocraft_text::fonts::INTER_REGULAR[..], identity: runtime_font_input::FaceIdentity::inter_regular() },
        runtime_font_input::Limits::default(),
        job.interrupt,
        &mut |bytes| {
            let next = held.get().checked_add(bytes).ok_or(photocraft_text::xcf::TextSourceError::Budget)?;
            let mut callback = meter.try_borrow_mut().map_err(|_| photocraft_text::xcf::TextSourceError::Budget)?;
            (**callback)(WideWork::ReserveBytes(bytes)).map_err(|e| {
                if let Ok(mut failure) = denied.try_borrow_mut() {
                    *failure = Some(e);
                }
                photocraft_text::xcf::TextSourceError::Budget
            })?;
            held.set(next);
            Ok(())
        },
        &mut |units| {
            let mut callback = meter.try_borrow_mut().map_err(|_| photocraft_text::xcf::TextSourceError::Budget)?;
            (**callback)(WideWork::SampleUnits(units)).map_err(|e| {
                if let Ok(mut failure) = denied.try_borrow_mut() {
                    *failure = Some(e);
                }
                photocraft_text::xcf::TextSourceError::Budget
            })
        },
    );
    job.font_bytes = job.font_bytes.checked_add(held.get()).ok_or(CompositionError::Invalid("font resident overflow"))?;
    if let Some(error) = denied.into_inner() {
        return Err(error);
    }
    let font = output.map_err(|error| {
        if error == photocraft_text::xcf::TextSourceError::Cancelled {
            photocraft_raster::RasterInputError::Cancelled.into()
        } else {
            CompositionError::Unsupported("exact runtime text font unavailable")
        }
    })?;
    prepare_control::checkpoint(job.interrupt)?;
    job.text_font = Some(HeldTextFont::Prepared(font));
    Ok(())
}
fn regenerate_text(
    job: &mut RenderJob<'_, '_>,
    id: photocraft_doc::LayerId,
    source: &photocraft_doc::xcf_items::SourceItem,
    poll: &mut impl FnMut(WideWork) -> Result<()>,
) -> Result<()> {
    if job.texts.iter().any(|(cached, _)| *cached == id) {
        return Ok(());
    }
    prepare_text_font(job, poll)?;
    let held = std::cell::Cell::new(0_u64);
    let mut charge = |bytes: u64| {
        let next = held.get().checked_add(bytes).ok_or(CompositionError::Invalid("text resident overflow"))?;
        poll(WideWork::ReserveBytes(bytes))?;
        held.set(next);
        Ok(())
    };
    let font = job.text_font.as_ref().ok_or(CompositionError::Invalid("text font preparation missing"))?.handle();
    let output = crate::gimp_text_items::render_source_with_authorized_font(job.doc, source, font, job.interrupt, &mut charge);
    job.text_bytes = job.text_bytes.checked_add(held.get()).ok_or(CompositionError::Invalid("text resident overflow"))?;
    job.texts.try_reserve(1).map_err(|_| CompositionError::Invalid("text job cache allocation"))?;
    job.texts.push((id, output?));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn prepare<'a>(
    doc: &'a Document,
    layers: &'a [Layer],
    profile: &Profile,
    stored: SampleEncoding,
    spaces: &mut Vec<(GimpSpace, WorkingSpace)>,
    depth: usize,
    interrupt: &photocraft_raster::Interrupt<'_>,
    authorized_font: Option<&PreparedRuntimeFont<'_>>,
) -> Result<Vec<Prepared<'a>>> {
    prepare_control::checkpoint(interrupt)?;
    if depth == 0 {
        normal_geometry::validate(doc, interrupt)?;
    }
    let mut prepared = prepare_control::reserve(layers.len(), depth, interrupt)?;
    for layer in layers {
        prepare_control::checkpoint(interrupt)?;
        if layer.clipped
            || layer.vector_mask.is_some()
            || crate::effects::has_effects(layer)
            || layer.excluded_channels != 0
            || layer.advanced != photocraft_doc::AdvancedBlending::default()
            || layer.blend_if != photocraft_doc::BlendIf::default()
        {
            return Err(CompositionError::Unsupported("clipping, vector masks or advanced effects require additional wide evaluators"));
        }
        coverage(f64::from(layer.opacity))?;
        coverage(f64::from(layer.fill_opacity))?;
        if let Some(mask) = &layer.mask {
            coverage(f64::from(mask.density))?;
            if mask.feather != 0.0 {
                return Err(CompositionError::Unsupported("wide mask feather evaluator"));
            }
            if mask.surface.format().mode != ColorMode::Grayscale {
                return Err(CompositionError::Invalid("mask color model"));
            }
        }
        let item = doc.xcf_state.as_ref().and_then(|state| state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)));
        let vector_source = doc.xcf_state.as_ref().and_then(|state| {
            state.source_items.iter().find(|source| {
                source.owner == ItemRef::Layer(layer.id)
                    && matches!(source.source, photocraft_doc::xcf_items::EditableSource::Vector { source_modified: false, .. })
            })
        });
        let text_source = doc.xcf_state.as_ref().and_then(|state| {
            state.source_items.iter().find(|source| {
                source.owner == ItemRef::Layer(layer.id)
                    && matches!(source.source, photocraft_doc::xcf_items::EditableSource::Text { source_modified: false, .. })
            })
        });
        if let Some(source) = text_source {
            if let Some(font) = authorized_font {
                crate::gimp_text_items::qualify_current_source_with_authorized_font(doc, source, font, interrupt)?;
            } else {
                crate::gimp_text_items::qualify_current_source(doc, source, interrupt)?;
            }
        }
        if let Some(source) = vector_source {
            if layer.mask.is_some() || doc.xcf_state.as_ref().is_some_and(|state| state.filter_stacks.iter().any(|stack| stack.owner == source.owner)) {
                return Err(CompositionError::Unsupported("vector source mask or filter interaction"));
            }
            crate::gimp_vector_items::qualify_source(doc, source, interrupt)?;
        }
        let mut working = layer.import_compositing.map_or(space(stored), |policy| space(policy.composite_encoding));
        let mut operator = GimpOperator::Union;
        let mut mode = crate::gimp_blend::GimpBlendMode::Normal;
        let mut blend_space = working;
        let mut pass_through = layer.blend == BlendMode::PassThrough;
        if let Some(composition) = item.and_then(|item| item.composition.as_ref()) {
            if let Some(required) = photocraft_doc::import_compositing::analytic_normal_encoding(&composition.compatibility_epoch) {
                if doc.sample_encoding != Some(required)
                    || stored != required
                    || !matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::Icc)
                {
                    return Err(CompositionError::Unsupported("analytic Normal requires its epoch-bound current assigned ICC interpretation"));
                }
                let encoding = match required {
                    SampleEncoding::Linear => photocraft_cms::xcf_color::Encoding::Linear,
                    SampleEncoding::NonlinearProfile => photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                    SampleEncoding::Perceptual => photocraft_cms::xcf_color::Encoding::Perceptual,
                };
                photocraft_cms::xcf_color::qualify_effective_analytic_profile(profile, encoding)?;
            }
            if matches!(composition.source_mode, 3 | 4 | 6..=21) && !composition.legacy {
                return Err(CompositionError::Invalid("legacy mode requires explicit legacy policy"));
            }
            if !(composition.source_mode == 61 && composition.pass_through && matches!(layer.content, LayerContent::Group(_))) {
                mode = crate::gimp_blend::mode(composition.source_mode)?;
            }
            let resolved = composition.checked_resolved().map_err(|error| match error.code {
                "composition_unresolved" | "composition_unknown" => CompositionError::Unsupported("unresolved or unknown GIMP composition policy"),
                _ => CompositionError::Invalid("stored GIMP composition policy contradicts its resolved state"),
            })?;
            working = resolved.composite_space;
            blend_space = resolved.blend_space;
            operator = resolved.operator;
            pass_through = composition.pass_through;
        } else if !matches!(layer.blend, BlendMode::Normal | BlendMode::PassThrough) {
            return Err(CompositionError::Unsupported("wide GIMP blend mode evaluator"));
        }
        let finite_normal_source = item
            .and_then(|item| item.composition.as_ref())
            .is_some_and(|policy| policy.compatibility_epoch == photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH);
        if finite_normal_source {
            let implicit = matches!(
                (&doc.working_profile_interpretation, doc.mode),
                (photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. }, ColorMode::Rgb)
                    | (photocraft_doc::WorkingProfileInterpretation::ImplicitSGray { .. }, ColorMode::Grayscale)
            );
            if !implicit
                || doc.depth != photocraft_color::SampleType::F64
                || stored != SampleEncoding::NonlinearProfile
                || (doc.size.width, doc.size.height) != (16, 14)
                || doc.layers.len() != 1
                || depth != 0
                || layer.opacity.to_bits() != 1.0_f32.to_bits()
                || layer.fill_opacity.to_bits() != 1.0_f32.to_bits()
                || layer.mask.is_some()
                || vector_source.is_some()
                || pass_through
                || mode != crate::gimp_blend::GimpBlendMode::Normal
                || operator != GimpOperator::Union
                || working != GimpSpace::LinearRgb
                || blend_space != GimpSpace::LinearRgb
                || item.and_then(|item| item.composition.as_ref()).is_none_or(|policy| {
                    policy.legacy
                        || policy.pass_through
                        || policy.source_mode != 28
                        || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (0, -1, -1)
                })
                || item
                    .and_then(|item| item.raster_bounds)
                    .is_none_or(|r| !matches!((r.width(), r.height(), r.x0, r.y0), (9, 8, -2, 1) | (11, 11, -3, 0) | (8, 8, -1, 1)))
                || !doc.channels.is_empty()
                || doc.xcf_state.as_ref().is_none_or(|state| !state.filter_stacks.is_empty() || state.floating.is_some() || !state.source_items.is_empty())
                || layer.surface().is_none_or(|surface| surface.format().sample != photocraft_color::SampleType::F64 || !surface.format().alpha)
            {
                return Err(CompositionError::Unsupported("finite Normal source context"));
            }
            let extent = item.and_then(|item| item.raster_bounds).ok_or(CompositionError::Invalid("finite source extent"))?;
            let surface = layer.surface().ok_or(CompositionError::Invalid("finite source surface"))?;
            let channels = surface.channels();
            let raw = surface.try_read_interleaved_with(extent, interrupt)?;
            if surface.format() != doc.pixel_format() || channels == 0 {
                return Err(CompositionError::Unsupported("finite Normal source colour domain"));
            }
            for (index, word) in raw.as_chunks::<8>().0.iter().enumerate() {
                if index.is_multiple_of(256) {
                    prepare_control::checkpoint(interrupt)?;
                }
                let value = f64::from_ne_bytes(*word);
                if !value.is_finite() || (index % channels != channels - 1 && !(0.0..=1.0).contains(&value)) {
                    return Err(CompositionError::Unsupported("finite Normal source colour domain"));
                }
            }
            prepare_control::checkpoint(interrupt)?;
        }
        if pass_through && operator != GimpOperator::Union {
            return Err(CompositionError::Unsupported("pass-through group composite operator"));
        }
        let isolated_seed = !pass_through
            && matches!(layer.content, LayerContent::Group(_))
            && item.and_then(|item| item.composition.as_ref()).is_some_and(|policy| {
                policy.source_mode == 28 && policy.compatibility_epoch == photocraft_doc::import_compositing::XCF13_ISOLATED_MULTIPLY_SEED_EPOCH
            });
        let index = if let Some(index) = spaces.iter().position(|(kind, _)| *kind == working) {
            index
        } else {
            spaces.push((working, WorkingSpace::new_with_interrupt(profile, stored, working, interrupt)?));
            spaces.len() - 1
        };
        let blend_working = if mode != crate::gimp_blend::GimpBlendMode::Normal {
            if blend_space == GimpSpace::Lab
                && !matches!(
                    mode,
                    crate::gimp_blend::GimpBlendMode::LchHue
                        | crate::gimp_blend::GimpBlendMode::LchChroma
                        | crate::gimp_blend::GimpBlendMode::LchColor
                        | crate::gimp_blend::GimpBlendMode::LchLightness
                )
            {
                return Err(CompositionError::Unsupported("separable Lab blend space"));
            }
            if let Some(index) = spaces.iter().position(|(kind, _)| *kind == blend_space) {
                index
            } else {
                spaces.push((blend_space, WorkingSpace::new_with_interrupt(profile, stored, blend_space, interrupt)?));
                spaces.len() - 1
            }
        } else {
            index
        };
        let children = match &layer.content {
            LayerContent::Raster(surface) => {
                if !matches!(surface.format().mode, ColorMode::Rgb | ColorMode::Grayscale) || surface.format().mode != doc.mode {
                    return Err(CompositionError::Unsupported("wide raster color model"));
                }
                Vec::new()
            }
            LayerContent::Group(group) => {
                if group.artboard.is_some() {
                    return Err(CompositionError::Unsupported("wide artboard bounds"));
                }
                prepare(doc, &group.children, profile, stored, spaces, depth + 1, interrupt, authorized_font)?
            }
            _ => return Err(CompositionError::Unsupported("wide editable item renderer")),
        };
        let mut group_gaussian = None;
        let filters = if let Some(state) = &doc.xcf_state {
            let mut stacks = state.filter_stacks.iter().filter(|stack| stack.owner == ItemRef::Layer(layer.id));
            let first = stacks.next();
            if stacks.next().is_some() {
                return Err(CompositionError::Invalid("duplicate owner filter stacks"));
            }
            if let Some(stack) = first {
                let extent = item.and_then(|item| item.raster_bounds).unwrap_or(Rect::EMPTY);
                if matches!(layer.content, LayerContent::Group(_)) {
                    group_gaussian = Some(group_gaussian::GroupGaussian::new(doc, layer, state, stack, (extent, profile, stored), interrupt)?);
                    None
                } else {
                    if !matches!(layer.content, LayerContent::Raster(_)) {
                        return Err(CompositionError::Unsupported("point filter non-raster owner"));
                    }
                    let extent = item.and_then(|item| item.raster_bounds).unwrap_or(Rect::EMPTY);
                    let same_depth_selection = || {
                        stack.nodes.iter().all(|node| {
                            node.mask.is_some_and(|item| {
                                state
                                    .filter_masks
                                    .iter()
                                    .any(|mask| ItemRef::Mask(mask.id) == item && mask.owner == node.id && mask.surface.format().sample == doc.depth)
                            })
                        })
                    };
                    if stack.nodes.iter().any(|node| node.operation.name == "gegl:rgb-clip") {
                        let source_policy = item.and_then(|item| item.composition.as_ref());
                        let qualified = doc.size == photocraft_doc::Size::new(8, 6)
                            && extent == Rect::new(0, 0, 8, 6)
                            && doc.mode == ColorMode::Rgb
                            && doc.depth == photocraft_color::SampleType::F32
                            && stored == SampleEncoding::Linear
                            && matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. })
                            && doc.layers.len() == 1
                            && doc.channels.is_empty()
                            && doc.paths.is_empty()
                            && state.source_items.is_empty()
                            && state.filter_stacks.len() == 1
                            && stack.nodes.len() == 1
                            && same_depth_selection()
                            && layer.mask.is_none()
                            && layer.visible
                            && layer.opacity.to_bits() == 1.0_f32.to_bits()
                            && doc.source_color_context.as_ref().is_some_and(|context| context.original_icc.is_none())
                            && profile.to_bytes() == photocraft_cms::Builtin::Srgb.profile().to_bytes()
                            && source_policy.is_some_and(|policy| {
                                policy.source_mode == 28
                                    && policy.stored_blend_space == 0
                                    && policy.stored_composite_space == -1
                                    && policy.stored_operator == -1
                                    && !policy.legacy
                                    && !policy.pass_through
                            })
                            && layer.surface().is_some_and(|surface| {
                                surface.format() == photocraft_color::PixelFormat::RGBA32F
                                    && surface.try_read_interleaved(extent).is_ok_and(|bytes| {
                                        bytes.len() == 768
                                            && bytes.as_chunks::<4>().0.iter().enumerate().all(|(index, bytes)| {
                                                let value = f32::from_ne_bytes(*bytes);
                                                value.is_finite() && (index % 4 != 3 || value.to_bits() == 1.0_f32.to_bits())
                                            })
                                    })
                            });
                        if !qualified {
                            return Err(CompositionError::Unsupported("unqualified HDR RGB Clip raster context"));
                        }
                    }
                    let generic_point = stack.nodes.iter().any(|node| {
                        node.operation.name == "gegl:unpremultiply"
                            || crate::gimp_semi_flatten::claims(node)
                            || crate::gimp_video_degradation::claims_video(node)
                            || crate::gimp_red_eye::claims_geometry(node)
                            || node
                                .composition
                                .as_ref()
                                .is_some_and(|policy| policy.compatibility_epoch == crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH)
                    });
                    if generic_point {
                        qualify_generic_point_context(doc, layer, state, stack, (extent, profile, stored), interrupt)?;
                    }
                    qualify_residual_point_metadata(doc, layer, state, stack, extent, stored, interrupt)?;
                    let precision_value = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gegl:value-invert" && node.operation.schema_epoch == "xcf-22-stock-value-invert-precision-v1"
                        });
                    let precision_brightness = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gegl:brightness-contrast"
                                && node.operation.schema_epoch == crate::gimp_filters::BRIGHTNESS_CONTRAST_PRECISION_EPOCH
                        });
                    let precision_levels = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gegl:levels" && node.operation.schema_epoch == crate::gimp_filters::GEGL_LEVELS_PRECISION_EPOCH
                        });
                    let exposure_geometry = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gegl:exposure" && node.operation.schema_epoch == crate::gimp_filters::EXPOSURE_GEOMETRY_EPOCH
                        });
                    let precision_exposure_black = exposure_geometry
                        || stack.nodes.len() == 1
                            && stack.nodes.first().is_some_and(|node| {
                                node.operation.name == "gegl:exposure" && node.operation.schema_epoch == crate::gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH
                            });
                    let precision_balance_depth = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gimp:color-balance" && node.operation.schema_epoch == crate::gimp_color_balance::DEPTH_EPOCH
                        });
                    let precision_balance = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gimp:color-balance"
                                && matches!(node.operation.schema_epoch.as_str(), crate::gimp_color_balance::EPOCH | crate::gimp_color_balance::DEPTH_EPOCH)
                        });
                    if stack.nodes.iter().any(|node| node.operation.name == "gimp:color-balance") && (!precision_balance || doc.mode != ColorMode::Rgb) {
                        return Err(CompositionError::Unsupported("unqualified Color Balance source context"));
                    }
                    let precision_temperature = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gegl:color-temperature"
                                && matches!(
                                    node.operation.schema_epoch.as_str(),
                                    crate::gimp_filters::COLOR_TEMPERATURE_PRECISION_EPOCH | crate::gimp_filters::COLOR_TEMPERATURE_OFFGRID_EPOCH
                                )
                        });
                    let precision_alien_map = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gegl:alien-map"
                                && matches!(
                                    node.operation.schema_epoch.as_str(),
                                    crate::gimp_filters::ALIEN_MAP_PRECISION_EPOCH | crate::gimp_filters::ALIEN_MAP_HSL_PRECISION_EPOCH
                                )
                        });
                    if stack.nodes.iter().any(|node| node.operation.name == "gegl:alien-map") && (!precision_alien_map || doc.mode != ColorMode::Rgb) {
                        return Err(CompositionError::Unsupported("unqualified Alien Map color/epoch domain"));
                    }
                    let precision_sepia = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gegl:sepia" && node.operation.schema_epoch == crate::gimp_filters::SEPIA_PRECISION_EPOCH
                        });
                    if stack.nodes.iter().any(|node| node.operation.name == "gegl:sepia") && (!precision_sepia || doc.mode != ColorMode::Rgb) {
                        return Err(CompositionError::Unsupported("unqualified Sepia color/epoch domain"));
                    }
                    let precision_saturation = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gegl:saturation"
                                && matches!(
                                    node.operation.schema_epoch.as_str(),
                                    crate::gimp_filters::SATURATION_PRECISION_EPOCH | crate::gimp_filters::SATURATION_RGB_YUV_PRECISION_EPOCH
                                )
                        });
                    if precision_saturation
                        && (doc.mode != ColorMode::Rgb
                            || !matches!(doc.depth, photocraft_color::SampleType::U16 | photocraft_color::SampleType::F32)
                                && stack.nodes.iter().any(|node| {
                                    node.operation.schema_epoch == crate::gimp_filters::SATURATION_RGB_YUV_PRECISION_EPOCH
                                        || node.arguments.iter().any(|argument| {
                                            matches!(&argument.value,
                                                photocraft_doc::xcf_filters::FilterValue::Enum {domain,value:2}
                                                if argument.name=="colorspace" && domain=="GeglSaturationType")
                                        })
                                }))
                    {
                        return Err(CompositionError::Unsupported("unqualified Saturation color/depth domain"));
                    }
                    let precision_desaturate = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gimp:desaturate" && node.operation.schema_epoch == crate::gimp_filters::DESATURATE_PRECISION_EPOCH
                        });
                    if precision_desaturate && doc.mode != ColorMode::Rgb {
                        return Err(CompositionError::Unsupported("unqualified Desaturate color model"));
                    }
                    let precision_saturation_gray = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gegl:saturation" && node.operation.schema_epoch == crate::gimp_filters::SATURATION_GRAY_PRECISION_EPOCH
                        });
                    if precision_saturation_gray && doc.mode != ColorMode::Grayscale {
                        return Err(CompositionError::Unsupported("unqualified Gray Saturation color domain"));
                    }
                    let precision_threshold = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gimp:threshold" && node.operation.schema_epoch == crate::gimp_filters::THRESHOLD_PRECISION_EPOCH
                        });
                    if precision_threshold && doc.mode != ColorMode::Rgb {
                        return Err(CompositionError::Unsupported("unqualified Threshold color domain"));
                    }
                    let precision_threshold_alpha = stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| {
                            node.operation.name == "gimp:threshold-alpha" && node.operation.schema_epoch == crate::gimp_filters::THRESHOLD_ALPHA_PRECISION_EPOCH
                        });
                    let precision_wide = precision_alien_map
                        || precision_sepia
                        || precision_brightness
                        || precision_levels
                        || precision_exposure_black
                        || precision_balance
                        || precision_temperature
                        || precision_desaturate
                        || precision_saturation
                        || precision_saturation_gray
                        || precision_threshold
                        || precision_threshold_alpha;
                    if !generic_point && (precision_value || precision_wide) {
                        let source_policy = item.and_then(|item| item.composition.as_ref());
                        let width = match (doc.depth, stored) {
                            (photocraft_color::SampleType::U8, _) if precision_balance_depth => 1,
                            (photocraft_color::SampleType::U32, _) if precision_balance_depth => 4,
                            (photocraft_color::SampleType::U16, SampleEncoding::Perceptual) if precision_balance_depth => 2,
                            (photocraft_color::SampleType::F32, SampleEncoding::Perceptual) if precision_balance_depth => 4,
                            (photocraft_color::SampleType::F64, SampleEncoding::Perceptual) if precision_balance_depth => 8,
                            (photocraft_color::SampleType::U16, SampleEncoding::NonlinearProfile) => 2,
                            (photocraft_color::SampleType::F32, SampleEncoding::Linear) => 4,
                            (photocraft_color::SampleType::F64, SampleEncoding::NonlinearProfile) => 8,
                            (photocraft_color::SampleType::U16, SampleEncoding::Linear) if precision_wide => 2,
                            (photocraft_color::SampleType::F32, SampleEncoding::NonlinearProfile) if precision_wide => 4,
                            (photocraft_color::SampleType::F64, SampleEncoding::Linear) if precision_wide => 8,
                            _ => 0,
                        };
                        // This epoch records only matching current storage/encoding. Original Half
                        // is exactly widened; it never licenses rounding arbitrary current F32.
                        let source_precision = doc.source_color_context.as_ref().map(|context| context.source_precision);
                        let balance_depth_storage = !precision_balance_depth
                            || matches!(
                                (source_precision, doc.depth, stored),
                                (Some(100), photocraft_color::SampleType::U8, SampleEncoding::Linear)
                                    | (Some(150), photocraft_color::SampleType::U8, SampleEncoding::NonlinearProfile)
                                    | (Some(175), photocraft_color::SampleType::U8, SampleEncoding::Perceptual)
                                    | (Some(300), photocraft_color::SampleType::U32, SampleEncoding::Linear)
                                    | (Some(350), photocraft_color::SampleType::U32, SampleEncoding::NonlinearProfile)
                                    | (Some(375), photocraft_color::SampleType::U32, SampleEncoding::Perceptual)
                                    | (Some(500), photocraft_color::SampleType::F32, SampleEncoding::Linear)
                                    | (Some(550), photocraft_color::SampleType::F32, SampleEncoding::NonlinearProfile)
                                    | (Some(575 | 675), photocraft_color::SampleType::F32, SampleEncoding::Perceptual)
                                    | (Some(275), photocraft_color::SampleType::U16, SampleEncoding::Perceptual)
                                    | (Some(775), photocraft_color::SampleType::F64, SampleEncoding::Perceptual)
                            );
                        let qualified = width != 0
                            && balance_depth_storage
                            && same_depth_selection()
                            && matches!((&doc.working_profile_interpretation, doc.mode),
                                (photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. }, ColorMode::Rgb)
                                | (photocraft_doc::WorkingProfileInterpretation::ImplicitSGray { .. }, ColorMode::Grayscale) if precision_wide || doc.mode == ColorMode::Rgb)
                            && if exposure_geometry {
                                doc.size == photocraft_doc::Size::new(295, 160)
                                    && extent.width() == 257
                                    && extent.height() == 129
                                    && matches!((extent.x0, extent.y0), (-13, 13) | (17, -11))
                            } else if precision_threshold_alpha {
                                // This point operation has no neighborhood/offset dependency.
                                // Keep its full-canvas geometry distinct from the other
                                // independently scoped precision-point source families.
                                doc.size.width > 0
                                    && doc.size.height > 0
                                    && doc.size.width <= 262144
                                    && doc.size.height <= 262144
                                    && u64::from(doc.size.width).checked_mul(u64::from(doc.size.height)).is_some_and(|pixels| pixels <= 1 << 30)
                                    && extent == doc.bounds()
                            } else {
                                doc.size == photocraft_doc::Size::new(8, 6) && extent == Rect::new(0, 0, 8, 6)
                            }
                            && (doc.mode == ColorMode::Rgb || precision_wide && doc.mode == ColorMode::Grayscale)
                            && doc.layers.len() == 1
                            && doc.channels.is_empty()
                            && doc.paths.is_empty()
                            && state.source_items.is_empty()
                            && state.filter_stacks.len() == 1
                            && layer.mask.is_none()
                            && layer.visible
                            && layer.opacity.to_bits() == 1.0_f32.to_bits()
                            && doc.source_color_context.as_ref().is_some_and(|context| context.original_icc.is_none())
                            && profile.to_bytes()
                                == if doc.mode == ColorMode::Grayscale { photocraft_cms::Builtin::SGray } else { photocraft_cms::Builtin::Srgb }
                                    .profile()
                                    .to_bytes()
                            && source_policy.is_some_and(|policy| {
                                policy.source_mode == 28
                                    && policy.stored_blend_space == 0
                                    && policy.stored_composite_space == -1
                                    && policy.stored_operator == -1
                                    && !policy.legacy
                                    && !policy.pass_through
                            })
                            && layer.surface().is_some_and(|surface| {
                                surface.format() == doc.pixel_format()
                                    && surface.try_read_interleaved(extent).is_ok_and(|bytes| {
                                        (if precision_threshold_alpha {
                                            usize::try_from(extent.width())
                                                .ok()
                                                .and_then(|w| w.checked_mul(usize::try_from(extent.height()).ok()?))
                                                .and_then(|n| n.checked_mul(surface.channels()))
                                                .and_then(|n| n.checked_mul(width))
                                                .is_some_and(|expected| bytes.len() == expected)
                                        } else {
                                            bytes.len() == (if exposure_geometry { 257 * 129 } else { 48 }) * surface.channels() * width
                                        })
                                            && bytes.chunks_exact(width).all(|raw| match doc.depth {
                                                photocraft_color::SampleType::U8 | photocraft_color::SampleType::U16 | photocraft_color::SampleType::U32 => {
                                                    true
                                                }
                                                photocraft_color::SampleType::F32 => {
                                                    let Ok(raw) = <[u8; 4]>::try_from(raw) else { return false };
                                                    let value = f32::from_ne_bytes(raw);
                                                    value.is_finite()
                                                        && (0.0..=1.0).contains(&value)
                                                        && (!precision_balance_depth
                                                            || !matches!(source_precision, Some(500 | 550 | 575))
                                                            || photocraft_color::try_round_f32_to_f16_bits(value)
                                                                .is_ok_and(|bits| photocraft_color::widen_f16_to_f32_bits(bits) == value.to_bits()))
                                                }
                                                photocraft_color::SampleType::F64 => {
                                                    let Ok(raw) = <[u8; 8]>::try_from(raw) else { return false };
                                                    let value = f64::from_ne_bytes(raw);
                                                    value.is_finite() && (0.0..=1.0).contains(&value)
                                                }
                                            })
                                    })
                            });
                        if !qualified {
                            return Err(CompositionError::Unsupported("unqualified precision point raster context"));
                        }
                    }
                    if !precision_value
                        && !stack.nodes.iter().any(|node| node.operation.schema_epoch == crate::gimp_filters::MONO_MIXER_SIGNED_PRECISION_EPOCH)
                        && stack.nodes.iter().any(|node| matches!(node.operation.name.as_str(), "gimp:posterize" | "gegl:value-invert" | "gegl:mono-mixer"))
                    {
                        let source_policy = item.and_then(|item| item.composition.as_ref());
                        let qualified = doc.size == photocraft_doc::Size::new(32, 32)
                            && extent == Rect::new(0, 0, 32, 32)
                            && doc.mode == ColorMode::Rgb
                            && doc.depth == photocraft_color::SampleType::U8
                            && stored == SampleEncoding::NonlinearProfile
                            && doc.layers.len() == 1
                            && doc.channels.is_empty()
                            && doc.paths.is_empty()
                            && state.source_items.is_empty()
                            && state.filter_stacks.len() == 1
                            && stack.nodes.len() == 1
                            && layer.mask.is_none()
                            && layer.visible
                            && layer.opacity.to_bits() == 1.0_f32.to_bits()
                            && doc.source_color_context.as_ref().is_some_and(|context| context.original_icc.is_none())
                            && profile.to_bytes() == photocraft_cms::Builtin::Srgb.profile().to_bytes()
                            && source_policy.is_some_and(|policy| {
                                policy.source_mode == 28
                                    && policy.stored_blend_space == 0
                                    && policy.stored_composite_space == -1
                                    && policy.stored_operator == -1
                                    && !policy.legacy
                                    && !policy.pass_through
                            })
                            && layer.surface().is_some_and(|surface| {
                                surface.format() == photocraft_color::PixelFormat::RGBA8
                                    && surface
                                        .try_read_interleaved(extent)
                                        .is_ok_and(|bytes| bytes.len() == 4096 && bytes.as_chunks::<4>().0.iter().all(|pixel| pixel.last() == Some(&255)))
                            });
                        if !qualified {
                            return Err(CompositionError::Unsupported("unqualified scoped point raster context"));
                        }
                    }
                    if stack.nodes.iter().any(|node| node.operation.schema_epoch == "xcf-22-stock-point-stack-edits-v1") {
                        let source_policy = item.and_then(|item| item.composition.as_ref());
                        let qualified = doc.size == photocraft_doc::Size::new(8, 6)
                            && extent == Rect::new(0, 0, 8, 6)
                            && doc.mode == ColorMode::Rgb
                            && doc.depth == photocraft_color::SampleType::U8
                            && stored == SampleEncoding::NonlinearProfile
                            && doc.layers.len() == 1
                            && doc.channels.is_empty()
                            && doc.paths.is_empty()
                            && state.source_items.is_empty()
                            && state.filter_stacks.len() == 1
                            && layer.mask.is_none()
                            && layer.visible
                            && layer.opacity.to_bits() == 1.0_f32.to_bits()
                            && doc.source_color_context.as_ref().is_some_and(|context| context.original_icc.is_none())
                            && profile.to_bytes() == photocraft_cms::Builtin::Srgb.profile().to_bytes()
                            && source_policy.is_some_and(|policy| {
                                policy.source_mode == 28
                                    && policy.stored_blend_space == 0
                                    && policy.stored_composite_space == 2
                                    && policy.stored_operator == 1
                                    && !policy.legacy
                                    && !policy.pass_through
                            })
                            && layer.surface().is_some_and(|surface| {
                                surface.format() == photocraft_color::PixelFormat::RGBA8
                                    && surface.try_read_interleaved(extent).is_ok_and(|bytes| bytes.len() == 192)
                            });
                        if !qualified {
                            return Err(CompositionError::Unsupported("unqualified point stack edit raster context"));
                        }
                    }
                    for node in stack.nodes.iter().filter(|node| node.enabled || generic_point) {
                        crate::gimp_neighborhood::qualify_selection_mask_with_interrupt(
                            state,
                            node,
                            layer.id,
                            if generic_point { doc.bounds() } else { extent },
                            interrupt,
                        )?;
                    }
                    let qualified = extent == Rect::new(0, 0, 96, 80)
                        && doc.mode == ColorMode::Rgb
                        && stored == SampleEncoding::NonlinearProfile
                        && doc.source_color_context.as_ref().is_some_and(|context| context.original_icc.is_none())
                        && profile.to_bytes() == photocraft_cms::Builtin::Srgb.profile().to_bytes()
                        && layer.surface().is_some_and(|surface| surface.format() == photocraft_color::PixelFormat::RGBA8);
                    let finite_gaussian_qualified = extent == Rect::new(0, 0, 16, 12)
                        && doc.size == photocraft_doc::Size::new(16, 12)
                        && doc.mode == ColorMode::Rgb
                        && matches!(doc.depth, photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64)
                        && stored == SampleEncoding::NonlinearProfile
                        && doc.layers.len() == 1
                        && stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| node.operation.schema_epoch == crate::gimp_neighborhood::FINITE_GAUSSIAN_EPOCH)
                        && layer.surface().is_some_and(|surface| surface.format() == doc.pixel_format());
                    let box_qualified = doc.mode == ColorMode::Rgb
                        && stored == SampleEncoding::NonlinearProfile
                        && doc.layers.len() == 1
                        && stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| node.operation.schema_epoch == crate::gimp_neighborhood::BOX_BLUR_EPOCH)
                        && layer.surface().is_some_and(|surface| surface.format() == doc.pixel_format())
                        && ((extent == Rect::new(0, 0, 16, 12)
                            && doc.size == photocraft_doc::Size::new(16, 12)
                            && matches!(doc.depth, photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64))
                            || (extent == Rect::new(0, 0, 32, 32)
                                && doc.size == photocraft_doc::Size::new(32, 32)
                                && doc.depth == photocraft_color::SampleType::U8
                                && doc.source_color_context.as_ref().is_some_and(|context| context.original_icc.is_none())));
                    let opaque_neighborhood_context = extent == Rect::new(0, 0, 32, 32)
                        && doc.size == photocraft_doc::Size::new(32, 32)
                        && doc.mode == ColorMode::Rgb
                        && doc.depth == photocraft_color::SampleType::U8
                        && stored == SampleEncoding::NonlinearProfile
                        && doc.layers.len() == 1
                        && stack.nodes.len() == 1
                        && doc.source_color_context.as_ref().is_some_and(|context| context.original_icc.is_none())
                        && layer.surface().is_some_and(|surface| surface.format() == doc.pixel_format());
                    let median_qualified = opaque_neighborhood_context
                        && stack.nodes.first().is_some_and(|node| node.operation.schema_epoch == crate::gimp_neighborhood::MEDIAN_BLUR_EPOCH);
                    let motion_qualified = opaque_neighborhood_context
                        && stack.nodes.first().is_some_and(|node| node.operation.schema_epoch == crate::gimp_neighborhood::MOTION_BLUR_EPOCH);
                    let pixelize_qualified = opaque_neighborhood_context
                        && stack.nodes.first().is_some_and(|node| node.operation.schema_epoch == crate::gimp_neighborhood::PIXELIZE_EPOCH);
                    let median_color_qualified = extent == Rect::new(0, 0, 16, 12)
                        && doc.size == photocraft_doc::Size::new(16, 12)
                        && doc.mode == ColorMode::Rgb
                        && matches!(doc.depth, photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64)
                        && stored == SampleEncoding::NonlinearProfile
                        && doc.layers.len() == 1
                        && stack.nodes.len() == 1
                        && stack.nodes.first().is_some_and(|node| node.operation.schema_epoch == crate::gimp_neighborhood::MEDIAN_COLOR_EPOCH)
                        && layer.surface().is_some_and(|surface| surface.format() == doc.pixel_format());
                    let median_gray_qualified = extent == Rect::new(0, 0, 16, 12)
                        && doc.size == photocraft_doc::Size::new(16, 12)
                        && doc.mode == ColorMode::Grayscale
                        && matches!(doc.depth, photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64)
                        && stored == SampleEncoding::NonlinearProfile
                        && doc.layers.len() == 1
                        && stack.nodes.len() == 1
                        && doc.source_color_context.as_ref().is_some_and(|context| context.original_icc.is_none())
                        && stack.nodes.first().is_some_and(|node| node.operation.schema_epoch == crate::gimp_neighborhood::MEDIAN_GRAY_EPOCH)
                        && layer.surface().is_some_and(|surface| surface.format() == doc.pixel_format());
                    let curves_qualified = extent == Rect::new(0, 0, 12, 9)
                        && doc.size == photocraft_doc::Size::new(12, 9)
                        && doc.mode == ColorMode::Rgb
                        && stored == SampleEncoding::NonlinearProfile
                        && doc.source_color_context.as_ref().is_some_and(|context| context.original_icc.is_none())
                        && profile.to_bytes() == photocraft_cms::Builtin::Srgb.profile().to_bytes()
                        && layer.surface().is_some_and(|surface| surface.format() == photocraft_color::PixelFormat::RGBA8);
                    let levels_qualified = extent == Rect::new(0, 0, 12, 9)
                        && doc.size == photocraft_doc::Size::new(12, 9)
                        && doc.mode == ColorMode::Rgb
                        && matches!(stored, SampleEncoding::Linear | SampleEncoding::NonlinearProfile)
                        && doc.source_color_context.as_ref().is_some_and(|context| context.original_icc.is_none())
                        && profile.to_bytes() == photocraft_cms::Builtin::Srgb.profile().to_bytes()
                        && layer.surface().is_some_and(|surface| surface.format() == photocraft_color::PixelFormat::RGBA32F);
                    Some(
                        crate::gimp_neighborhood::OrderedStack::new_with_interrupt(
                            stack,
                            profile,
                            stored,
                            state.provenance.format_version,
                            extent,
                            [
                                qualified
                                    || finite_gaussian_qualified
                                    || box_qualified
                                    || median_qualified
                                    || median_color_qualified
                                    || median_gray_qualified
                                    || motion_qualified
                                    || pixelize_qualified,
                                curves_qualified,
                                levels_qualified,
                            ],
                            interrupt,
                        )?
                        .with_current_point_storage(if generic_point { Some(doc.depth) } else { None })
                        .with_filter_output_precision(
                            doc.xcf_state.as_ref().map_or(photocraft_doc::FilterOutputPrecision::Canonical, |state| state.current_filter_output_precision),
                        ),
                    )
                }
            } else {
                None
            }
        } else {
            None
        };
        let mask_state = layer_mask_state(doc, layer)?;
        let floating_extent = if doc.xcf_state.as_ref().and_then(|state| state.floating.as_ref()).is_some_and(|attachment| attachment.layer == layer.id) {
            Some(item.and_then(|item| item.raster_bounds).ok_or(CompositionError::Invalid("floating source extent missing"))?)
        } else {
            None
        };
        prepare_control::checkpoint(interrupt)?;
        prepared.push(Prepared {
            layer,
            working: index,
            operator,
            mode,
            blend_working,
            children,
            pass_through,
            isolated_seed,
            filters,
            mask_state,
            floating_extent,
            group_gaussian,
            vector_source,
            text_source,
            finite_normal_source,
        });
    }
    prepare_control::checkpoint(interrupt)?;
    Ok(prepared)
}

fn layer_mask_state(doc: &Document, layer: &Layer) -> Result<Option<photocraft_doc::xcf_state::MaskState>> {
    let mask = if let Some(state) = doc.xcf_state.as_ref() {
        let mut masks = state
            .items
            .iter()
            .filter(|item| matches!(item.item, ItemRef::Mask(id) if !state.filter_masks.iter().any(|mask| mask.id == id)))
            .filter_map(|item| item.mask.as_ref())
            .filter(|mask| mask.owner == ItemRef::Layer(layer.id));
        let first = masks.next();
        if masks.next().is_some() {
            return Err(CompositionError::Invalid("duplicate layer mask metadata"));
        }
        first
    } else {
        None
    };
    Ok(mask.cloned())
}

fn mask(layer: &Layer, state: Option<&photocraft_doc::xcf_state::MaskState>, x: i32, y: i32) -> Result<f64> {
    let Some(mask) = layer.mask.as_ref().filter(|mask| mask.enabled) else {
        return Ok(1.0);
    };
    let mut sample = [0.0_f64; 2];
    let count = mask.surface.channels();
    let value = if let Some(state) = state.filter(|state| !state.bounds.contains(x, y)) {
        f64::from_bits(state.outside_f64_bits)
    } else {
        mask.surface.try_read_pixel_f64(x, y, sample.get_mut(..count).ok_or(CompositionError::Invalid("mask channels"))?)?;
        sample[0]
    };
    coverage(value)?;
    Ok(1.0 - f64::from(mask.density) * (1.0 - value))
}
fn stack(
    layers: &[Prepared<'_>],
    spaces: &[(GimpSpace, WorkingSpace)],
    mut backdrop: [f64; 4],
    position: (i32, i32),
    poll: &mut impl FnMut(WideWork) -> Result<()>,
    job: &mut RenderJob<'_, '_>,
    isolated_seed: bool,
) -> Result<[f64; 4]> {
    let (x, y) = position;
    let mut first_visible = true;
    for node in layers {
        if !node.layer.visible {
            continue;
        }
        if node.floating_extent.is_some_and(|extent| !extent.contains(x, y)) {
            continue;
        }
        poll(WideWork::SampleUnits(128))?;
        let working = &spaces.get(node.working).ok_or(CompositionError::Invalid("working space reference"))?.1;
        let blend = &spaces.get(node.blend_working).ok_or(CompositionError::Invalid("blend space reference"))?.1;
        // Authored v13 isolated groups seed the first Multiply raster's coverage.
        // Pass-through stacks retain their real backdrop and original operator.
        let seed = isolated_seed
            && first_visible
            && matches!(node.layer.content, LayerContent::Raster(_))
            && node.mode == crate::gimp_blend::GimpBlendMode::Multiply
            && node.operator == GimpOperator::ClipToBackdrop;
        first_visible = false;
        let operator = if seed { GimpOperator::Union } else { node.operator };
        let compose = |backdrop, source, opacity| match node.mode {
            crate::gimp_blend::GimpBlendMode::Normal if node.finite_normal_source => working.normal_finite_source(backdrop, source, opacity, operator),
            crate::gimp_blend::GimpBlendMode::Normal => working.normal(backdrop, source, opacity, operator),
            mode => working.separable(blend, backdrop, source, opacity, operator, mode),
        };
        let opacity = f64::from(node.layer.opacity) * f64::from(node.layer.fill_opacity) * mask(node.layer, node.mask_state.as_ref(), x, y)?;
        match &node.layer.content {
            LayerContent::Raster(surface) => {
                if let Some(source) = node.vector_source {
                    regenerate_vector(job, node.layer.id, source, poll)?;
                }
                if let Some(source) = node.text_source {
                    regenerate_text(job, node.layer.id, source, poll)?;
                }
                let surface = if node.text_source.is_some() {
                    &job.texts.iter().find(|(id, _)| *id == node.layer.id).ok_or(CompositionError::Invalid("text job cache"))?.1.surface
                } else if node.vector_source.is_some() {
                    &job.vectors.iter().find(|(id, _)| *id == node.layer.id).ok_or(CompositionError::Invalid("vector job cache"))?.1.surface
                } else {
                    surface
                };
                let mut samples = [0.0_f64; 4];
                let format = surface.format();
                surface.try_read_pixel_f64(x, y, samples.get_mut(..format.channels()).ok_or(CompositionError::Invalid("raster channels"))?)?;
                let color = if format.mode == ColorMode::Grayscale { [samples[0]; 3] } else { [samples[0], samples[1], samples[2]] };
                let alpha = if format.alpha { *samples.get(format.mode.color_channels()).ok_or(CompositionError::Invalid("raster alpha"))? } else { 1.0 };
                let source = [color[0], color[1], color[2], alpha];
                let source = if let Some(filters) = &node.filters {
                    if filters.has_neighborhood() {
                        job.cache.sample(filters, surface, node.layer.id, position, poll, job.interrupt)?
                    } else {
                        filters.point_at(source, position, poll)?
                    }
                } else {
                    source
                };
                backdrop = compose(backdrop, source, opacity)?;
            }
            LayerContent::Group(_) => {
                if node.pass_through {
                    let after = stack(&node.children, spaces, backdrop, position, poll, job, false)?;
                    backdrop = working.group_mix(backdrop, after, opacity)?;
                } else {
                    let source = if let Some(filter) = node.group_gaussian.as_ref().filter(|filter| filter.enabled) {
                        if !filter.extent.contains(x, y) {
                            continue;
                        }
                        if !job.groups.iter().any(|(id, _)| *id == node.layer.id) {
                            if !job.groups.is_empty() {
                                return Err(CompositionError::Unsupported("multiple Gaussian group cache domain"));
                            }
                            let mut input = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA64F);
                            let mut samples = Vec::new();
                            let count = usize::try_from(filter.extent.width())
                                .ok()
                                .and_then(|w| usize::try_from(filter.extent.height()).ok().and_then(|h| w.checked_mul(h)))
                                .and_then(|n| n.checked_mul(4))
                                .ok_or(CompositionError::Invalid("Gaussian group input size"))?;
                            samples.try_reserve_exact(count).map_err(|_| CompositionError::Invalid("Gaussian group input allocation"))?;
                            for gy in filter.extent.y0..filter.extent.y1 {
                                for gx in filter.extent.x0..filter.extent.x1 {
                                    let pixel = stack(&node.children, spaces, [0.0; 4], (gx, gy), poll, job, node.isolated_seed)?;
                                    samples.extend_from_slice(&filter.linear(pixel)?);
                                }
                            }
                            input.try_write_region_f64_with(filter.extent, &samples, job.interrupt)?;
                            let filtered = filter.evaluate(&input, poll, job.interrupt)?;
                            job.groups.try_reserve(1).map_err(|_| CompositionError::Invalid("Gaussian group cache allocation"))?;
                            job.groups.push((node.layer.id, filtered));
                        }
                        let surface = &job.groups.iter().find(|(id, _)| *id == node.layer.id).ok_or(CompositionError::Invalid("Gaussian group cache"))?.1;
                        let mut pixel = [0.0; 4];
                        surface.try_read_pixel_f64(x, y, &mut pixel)?;
                        pixel
                    } else {
                        stack(&node.children, spaces, [0.0; 4], position, poll, job, node.isolated_seed)?
                    };
                    backdrop = compose(backdrop, source, opacity)?;
                }
            }
            _ => return Err(CompositionError::Unsupported("wide item renderer")),
        }
    }
    Ok(backdrop)
}

/// Actual document evaluation with no f32 sample/display round trip. Unimplemented semantics
/// fail before rendering. Reservation for the returned buffer remains caller-owned until drop;
/// failure releases it. The caller may impose lower work/resident/deadline limits via poll.
pub fn try_render(doc: &Document, rect: Rect, poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<WideBuffer> {
    PreparedRenderer::new(doc)?.render(rect, poll)
}

/// Qualified tree/working spaces reused across bounded bands or reduced previews.
pub struct PreparedRenderer<'a> {
    authorized_font: Option<&'a PreparedRuntimeFont<'a>>,
    doc: &'a Document,
    layers: Vec<Prepared<'a>>,
    spaces: Vec<(GimpSpace, WorkingSpace)>,
}

/// Current metadata/layout checks shared with the explicit backed operative
/// recipe. This does not validate a profile, samples, raw tile padding or source
/// correspondence. The resident caller below retains all of those old checks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn qualify_generic_point_storage_metadata(
    doc: &Document,
    layer: &Layer,
    state: &photocraft_doc::XcfState,
    stack: &photocraft_doc::xcf_filters::FilterStack,
    extent: Rect,
    stored: SampleEncoding,
    format: photocraft_color::PixelFormat,
    default_bytes: &[u8],
) -> Result<()> {
    qualify_point_storage_metadata(doc, layer, state, stack, extent, stored, format, default_bytes, PointLayerCoverage::OriginalRecipe)
}

// Only genuine class-validated CURRENT callers may use dynamic layer coverage.
// This changes neither the native filter output nor original-source eligibility.
pub(crate) fn qualify_current_point_layer_coverage_metadata(
    doc: &Document, layer: &Layer, state: &photocraft_doc::XcfState,
    stack: &photocraft_doc::xcf_filters::FilterStack, extent: Rect,
    stored: SampleEncoding, format: photocraft_color::PixelFormat, default_bytes: &[u8],
) -> Result<()> {
    qualify_point_storage_metadata(doc, layer, state, stack, extent, stored, format, default_bytes, PointLayerCoverage::Current)
}
// Closed second geometry bundle; no source, precision or operation grant.
pub(crate) fn residual_origin_node(node: &photocraft_doc::xcf_filters::FilterNode) -> bool {
    match node.operation.name.as_str() {
        "gimp:desaturate" => node.operation.schema_epoch == crate::gimp_filters::DESATURATE_PRECISION_EPOCH,
        "gegl:saturation" => matches!(node.operation.schema_epoch.as_str(), crate::gimp_filters::SATURATION_PRECISION_EPOCH | crate::gimp_filters::SATURATION_GRAY_PRECISION_EPOCH | crate::gimp_filters::SATURATION_RGB_YUV_PRECISION_EPOCH),
        "gegl:value-invert" => node.operation.schema_epoch == crate::gimp_filters::VALUE_INVERT_PRECISION_EPOCH,
        "gimp:color-balance" => matches!(node.operation.schema_epoch.as_str(), crate::gimp_color_balance::EPOCH | crate::gimp_color_balance::DEPTH_EPOCH),
        _ => false,
    }
}
pub(crate) fn residual_origin_extent(extent: Rect) -> bool {
    extent.x0 == 0 && extent.y0 == 0 && !extent.is_empty()
        && extent.width() <= 262144 && extent.height() <= 262144
        && u64::from(extent.width()).checked_mul(u64::from(extent.height())).is_some_and(|pixels| pixels <= 1 << 30)
}
/// Join plain Document canonical geometry to the actual native descriptor before
/// profile construction or physical mask traversal. L1 custody is not assumed.
pub(crate) fn qualify_residual_origin_context(doc: &Document, layer: &Layer, state: &photocraft_doc::XcfState, stack: &photocraft_doc::xcf_filters::FilterStack, extent: Rect) -> Result<()> {
    if !stack.nodes.first().is_some_and(residual_origin_node) { return Ok(()) }
    let [actual_layer] = doc.layers.as_slice() else { return Err(CompositionError::Unsupported("residual origin singleton layer")) };
    let [actual_stack] = state.filter_stacks.as_slice() else { return Err(CompositionError::Unsupported("residual origin singleton stack")) };
    if state.items.len() != 2 || state.filter_masks.len() != 1 || stack.nodes.len() != 1 || actual_stack.nodes.len() != 1
        || actual_layer.id != layer.id || stack.owner != ItemRef::Layer(layer.id) || actual_stack.owner != stack.owner
        || actual_stack.nodes.first().map(|node| node.id) != stack.nodes.first().map(|node| node.id)
        || !residual_origin_extent(extent) || extent != doc.bounds() || extent.size() != doc.size
        || state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).and_then(|item| item.raster_bounds) != Some(extent)
    { return Err(CompositionError::Unsupported("residual checked origin canonical/native extent")); }
    Ok(())
}
/// Metadata portion of the existing scoped precision-point/current raster gates.
/// Native input validation stays with the caller's real controlled reader.
/// These identities retain their own epochs and never use PointReplace aliases.
pub(crate) fn qualify_residual_point_metadata(
    doc:&Document, layer:&Layer, state:&photocraft_doc::XcfState,
    stack:&photocraft_doc::xcf_filters::FilterStack, extent:Rect,
    stored:SampleEncoding, interrupt:&photocraft_raster::Interrupt<'_>,
)->Result<()> {
    interrupt.check().map_err(|_|photocraft_raster::RasterInputError::Cancelled)?;
    qualify_residual_origin_context(doc, layer, state, stack, extent)?;
    let [node]=stack.nodes.as_slice() else { return Ok(()) };
    let desaturate=node.operation.name=="gimp:desaturate" && node.operation.schema_epoch==crate::gimp_filters::DESATURATE_PRECISION_EPOCH;
    let saturation=node.operation.name=="gegl:saturation" && matches!(node.operation.schema_epoch.as_str(),crate::gimp_filters::SATURATION_PRECISION_EPOCH|crate::gimp_filters::SATURATION_GRAY_PRECISION_EPOCH|crate::gimp_filters::SATURATION_RGB_YUV_PRECISION_EPOCH);
    let value=node.operation.name=="gegl:value-invert" && node.operation.schema_epoch==crate::gimp_filters::VALUE_INVERT_PRECISION_EPOCH;
    let posterize=node.operation.name=="gimp:posterize" && node.operation.schema_epoch=="xcf-22-stock-point-replace";
    if !(desaturate||saturation||value||posterize) { return Ok(()) }
    let gray=saturation && node.operation.schema_epoch==crate::gimp_filters::SATURATION_GRAY_PRECISION_EPOCH;
    let yuv=saturation && !gray && (node.operation.schema_epoch==crate::gimp_filters::SATURATION_RGB_YUV_PRECISION_EPOCH || node.arguments.iter().any(|arg|arg.name=="colorspace" && matches!(&arg.value,photocraft_doc::xcf_filters::FilterValue::Enum{domain,value:2} if domain=="GeglSaturationType")));
    let precision=if value { matches!((doc.depth,stored),
        (photocraft_color::SampleType::U16,SampleEncoding::NonlinearProfile)|
        (photocraft_color::SampleType::F32,SampleEncoding::Linear)|
        (photocraft_color::SampleType::F64,SampleEncoding::NonlinearProfile))
    } else if posterize { doc.depth==photocraft_color::SampleType::U8 && stored==SampleEncoding::NonlinearProfile
    } else { matches!(doc.depth,photocraft_color::SampleType::U16|photocraft_color::SampleType::F32|photocraft_color::SampleType::F64) && matches!(stored,SampleEncoding::Linear|SampleEncoding::NonlinearProfile) };
    if (posterize && (doc.size!=photocraft_doc::Size::new(32,32) || extent!=Rect::new(0,0,32,32)))
        || doc.mode!=if gray {ColorMode::Grayscale} else {ColorMode::Rgb}
        || !precision || (yuv && !matches!(doc.depth,photocraft_color::SampleType::U16|photocraft_color::SampleType::F32))
        || doc.layers.len()!=1 || !doc.channels.is_empty() || !doc.paths.is_empty() || !state.source_items.is_empty()
        || state.filter_stacks.len()!=1 || layer.mask.is_some() || !layer.visible || layer.opacity.to_bits()!=1f32.to_bits()
        || doc.source_color_context.as_ref().is_none_or(|context|context.original_icc.is_some())
        || state.items.iter().find(|item|item.item==ItemRef::Layer(layer.id)).and_then(|item|item.composition.as_ref()).is_none_or(|policy|policy.source_mode!=28 || (policy.stored_blend_space,policy.stored_composite_space,policy.stored_operator)!=(0,-1,-1) || policy.legacy || policy.pass_through)
    { return Err(CompositionError::Unsupported("unqualified residual point current raster context")); }
    interrupt.check().map_err(|_|photocraft_raster::RasterInputError::Cancelled)?;
    Ok(())
}
pub(crate) fn qualify_current_point_optional_mask_metadata(doc:&Document,layer:&Layer,state:&photocraft_doc::XcfState,stack:&photocraft_doc::xcf_filters::FilterStack,extent:Rect,stored:SampleEncoding,format:photocraft_color::PixelFormat,default_bytes:&[u8])->Result<()> {
    qualify_point_storage_metadata(doc,layer,state,stack,extent,stored,format,default_bytes,PointLayerCoverage::CurrentOptionalMask)
}
#[derive(Clone, Copy)]
enum PointLayerCoverage { OriginalRecipe, Current, CurrentOptionalMask }
fn qualify_point_storage_metadata(
    doc: &Document,
    layer: &Layer,
    state: &photocraft_doc::XcfState,
    stack: &photocraft_doc::xcf_filters::FilterStack,
    extent: Rect,
    stored: SampleEncoding,
    format: photocraft_color::PixelFormat,
    default_bytes: &[u8],
    coverage_policy: PointLayerCoverage,
) -> Result<()> {
    let item = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id));
    let bounds_valid = |bounds: Rect| {
        !bounds.is_empty()
            && bounds.width() <= 262144
            && bounds.height() <= 262144
            && [bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|value| (-(1i32 << 29)..=(1i32 << 29)).contains(value))
    };
    if !bounds_valid(doc.bounds())
        || !bounds_valid(extent)
        || doc.layers.len() != 1
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || !state.source_items.is_empty()
        || state.floating.is_some()
        || state.filter_stacks.len() != 1
        || stack.nodes.len() != 1
        || layer.mask.is_some()
        || (matches!(coverage_policy, PointLayerCoverage::OriginalRecipe) && !layer.visible)
        || (matches!(coverage_policy, PointLayerCoverage::OriginalRecipe) && layer.opacity.to_bits() != 1f32.to_bits())
        || (matches!(coverage_policy, PointLayerCoverage::Current|PointLayerCoverage::CurrentOptionalMask)
            && (!layer.opacity.is_finite() || !(0.0..=1.0).contains(&layer.opacity) || item.is_none_or(|item| item.visible != layer.visible)))
        || !matches!(doc.depth, photocraft_color::SampleType::U8 | photocraft_color::SampleType::U16 | photocraft_color::SampleType::U32 | photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64)
        || !matches!(stored, SampleEncoding::Linear | SampleEncoding::NonlinearProfile | SampleEncoding::Perceptual)
        || format != doc.pixel_format()
        || !format.alpha
        || default_bytes.iter().any(|byte| *byte != 0)
        || item.and_then(|item| item.composition.as_ref()).is_none_or(|policy| {
            policy.source_mode != 28
                || policy.legacy
                || policy.pass_through
                || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (0, -1, -1)
        })
        || stack.nodes.iter().any(|node| {
            match node.mask {
                None=>!matches!(coverage_policy,PointLayerCoverage::CurrentOptionalMask),
                Some(reference)=>!state.filter_masks.iter().any(|mask|ItemRef::Mask(mask.id)==reference && mask.owner==node.id && mask.surface.format().sample==doc.depth),
            }
        })
    {
        return Err(CompositionError::Unsupported("unqualified generic point current raster context"));
    }
    Ok(())
}

pub(crate) struct PointCurrentFamilies {
    channel_mixer: bool,
    mono_signed: bool,
    color_overlay: bool,
    color_rotate: bool,
    unpremultiply: bool,
    absolute: bool,
    red_eye: bool,
    component_extract: bool,
    alpha_clip: bool,
}
/// The exact existing current family extent/precision/output predicates. No
/// Surface/native samples, source correspondence or runtime tag are supplied.
/// The caller must prepay its bounded singleton metadata comparisons.
pub(crate) fn qualify_point_family_context(
    doc:&Document,layer:&Layer,state:&photocraft_doc::XcfState,stack:&photocraft_doc::xcf_filters::FilterStack,
    extent:Rect,profile:&Profile,stored:SampleEncoding,interrupt:&photocraft_raster::Interrupt<'_>,
)->Result<PointCurrentFamilies> {
    interrupt.check().map_err(|_|photocraft_raster::RasterInputError::Cancelled)?;
    if let Some(node) = stack.nodes.iter().find(|node| crate::gimp_semi_flatten::claims(node)) {
        if stack.nodes.len() != 1 { return Err(CompositionError::Unsupported("Semi-Flatten current singleton")); }
        crate::gimp_semi_flatten::qualify_context(doc, extent, node)?;
    }
    let channel_mixer = stack.nodes.first().is_some_and(|node| {
        node.operation.name == "gegl:channel-mixer"
            && node.operation.schema_epoch == crate::gimp_filters::CHANNEL_MIXER_PRECISION_EPOCH
            && node.operation.stored_version == "0:0"
    });
    let mono_signed = stack.nodes.first().is_some_and(|node| {
        node.operation.name == "gegl:mono-mixer"
            && node.operation.schema_epoch == crate::gimp_filters::MONO_MIXER_SIGNED_PRECISION_EPOCH
            && node.operation.stored_version == "0:0"
    });
    let color_overlay = stack.nodes.first().is_some_and(|node| {
        node.operation.name == "gegl:color-overlay"
            && node.operation.schema_epoch == crate::gimp_filters::COLOR_OVERLAY_PRECISION_EPOCH
            && node.operation.stored_version == "0:0"
    });
    let color_rotate = stack.nodes.first().is_some_and(|node| {
        node.operation.name == "gegl:color-rotate" && node.operation.schema_epoch == crate::gimp_color_rotate::COLOR_ROTATE_PERCEPTUAL_EPOCH
    });
    let video = stack.nodes.iter().any(crate::gimp_video_degradation::claims_video);
    if stack.nodes.iter().any(crate::gimp_video_degradation::claims_options_epoch)
        && state.current_filter_output_precision != photocraft_doc::FilterOutputPrecision::Canonical
    {
        return Err(CompositionError::Unsupported("Video options canonical current output precision"));
    }
    let unpremultiply = stack.nodes.iter().any(|node| node.operation.name == "gegl:unpremultiply");
    if unpremultiply
        && (doc.mode != ColorMode::Rgb
            || doc.depth != photocraft_color::SampleType::F32
            || stored != SampleEncoding::Linear
            || doc.size != photocraft_doc::Size::new(8, 6)
            || extent != Rect::new(0, 0, 8, 6)
            || state.current_filter_output_precision != photocraft_doc::FilterOutputPrecision::Canonical)
    {
        return Err(CompositionError::Unsupported("Unpremultiply exact current RGB Float Linear extent/output"));
    }
    if video
        && (doc.size != photocraft_doc::Size::new(16, 16)
            || extent != Rect::new(0, 0, 16, 16)
            || doc.mode != ColorMode::Rgb
            || doc.depth != photocraft_color::SampleType::F32
            || stored != SampleEncoding::NonlinearProfile)
    {
        return Err(CompositionError::Unsupported("Video Degradation exact current Float nonlinear extent"));
    }
    if color_rotate
        && (doc.size != photocraft_doc::Size::new(32, 16)
            || extent != Rect::new(0, 0, 32, 16)
            || doc.depth != photocraft_color::SampleType::F32
            || stored != SampleEncoding::Perceptual)
    {
        return Err(CompositionError::Unsupported("Color Rotate current Perceptual extent/storage"));
    }
    let absolute = stack.nodes.first().is_some_and(|node| {
        node.operation.name == "gegl:absolute"
            && node.operation.schema_epoch == crate::gimp_filters::ABSOLUTE_LINEAR_PRECISION_EPOCH
            && node.operation.stored_version == "0:0"
    });
    if absolute && stored != SampleEncoding::Linear {
        return Err(CompositionError::Unsupported("unqualified Absolute Linear current extent/encoding"));
    }
    let red_eye = stack.nodes.first().is_some_and(|node| {
        node.operation.name == "gegl:red-eye-removal"
            && matches!(node.operation.schema_epoch.as_str(), crate::gimp_filters::RED_EYE_PRECISION_EPOCH | crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH)
            && node.operation.stored_version == "0:0"
    });
    if red_eye {
        let item=state.items.iter().find(|item|item.item==ItemRef::Layer(layer.id));
        let node = stack.nodes.first().ok_or(CompositionError::Invalid("Red Eye current node"))?;
        let dimensions = i64::from(extent.x1)
            .checked_sub(i64::from(extent.x0))
            .and_then(|width| u32::try_from(width).ok())
            .zip(i64::from(extent.y1).checked_sub(i64::from(extent.y0)).and_then(|height| u32::try_from(height).ok()));
        if !dimensions.is_some_and(|owner| {
            crate::gimp_red_eye::qualified_owner_geometry(&node.operation.schema_epoch, (doc.size.width, doc.size.height), owner, (extent.x0, extent.y0))
        }) {
            return Err(CompositionError::Unsupported("unqualified Red Eye current extent"));
        }
        if crate::gimp_red_eye::claims_geometry(node) {
            crate::gimp_red_eye::canonical_geometry(node, profile, stored, state.provenance.format_version)?;
            if doc.mode != ColorMode::Rgb
                || doc.depth != photocraft_color::SampleType::F32
                || state.current_filter_output_precision != photocraft_doc::FilterOutputPrecision::Canonical
                || item.is_none_or(|item| item.source_flags != 0)
            {
                return Err(CompositionError::Unsupported("Red Eye owner geometry RGB Float current precision"));
            }
        }
    }
    let component_extract = stack.nodes.first().is_some_and(|node| {
        node.operation.name == "gegl:component-extract"
            && matches!(
                node.operation.schema_epoch.as_str(),
                crate::gimp_filters::COMPONENT_EXTRACT_PRECISION_EPOCH
                    | crate::gimp_filters::COMPONENT_EXTRACT_CMYK_PRECISION_EPOCH
                    | crate::gimp_filters::COMPONENT_EXTRACT_YCBCR_PRECISION_EPOCH
                    | crate::gimp_filters::COMPONENT_EXTRACT_GEOMETRY_EPOCH
            )
            && node.operation.stored_version == "0:0"
    });
    let component_geometry = stack.nodes.first().is_some_and(|node| node.operation.schema_epoch == crate::gimp_filters::COMPONENT_EXTRACT_GEOMETRY_EPOCH);
    if component_geometry
        && (doc.depth != photocraft_color::SampleType::F32
            || stored != SampleEncoding::NonlinearProfile
            || state.current_filter_output_precision != photocraft_doc::FilterOutputPrecision::Canonical
            || !matches!(
                (doc.size.width, doc.size.height, extent),
                (37, 29, Rect { x0: -7, y0: 5, x1: 12, y1: 18 }) | (259, 17, Rect { x0: -3, y0: -4, x1: 259, y1: 5 })
            ))
    {
        return Err(CompositionError::Unsupported("unqualified Component Extract current geometry"));
    }
    let alpha_clip = stack.nodes.first().is_some_and(|node| {
        node.operation.name == "gegl:alpha-clip"
            && matches!(
                node.operation.schema_epoch.as_str(),
                crate::gimp_filters::ALPHA_CLIP_PRECISION_EPOCH | crate::gimp_filters::ALPHA_CLIP_FINITE_PRECISION_EPOCH
            )
            && node.operation.stored_version == "0:0"
    });
    let alpha_clip_finite = stack.nodes.first().is_some_and(|node| node.operation.schema_epoch == crate::gimp_filters::ALPHA_CLIP_FINITE_PRECISION_EPOCH);
    if alpha_clip_finite
        && (!matches!(doc.depth, photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64)
            || state.current_filter_output_precision != photocraft_doc::FilterOutputPrecision::Canonical)
    {
        return Err(CompositionError::Unsupported("unqualified finite Alpha Clip current precision"));
    }
    // Only the nine precision domains share this origin geometry policy.
    // Component's separately witnessed offset epoch and all other families keep
    // their existing geometry. Plain Document callers cannot assume L1 custody.
    if absolute || channel_mixer || mono_signed || color_overlay || (component_extract && !component_geometry) || alpha_clip {
        let [actual_layer] = doc.layers.as_slice() else {
            return Err(CompositionError::Unsupported("point origin singleton layer"));
        };
        let [actual_stack] = state.filter_stacks.as_slice() else {
            return Err(CompositionError::Unsupported("point origin singleton stack"));
        };
        if state.items.len() != 2
            || state.filter_masks.len() != 1
            || stack.nodes.len() != 1
            || actual_stack.nodes.len() != 1
            || actual_layer.id != layer.id
            || stack.owner != ItemRef::Layer(layer.id)
            || actual_stack.owner != stack.owner
            || actual_stack.nodes.first().map(|node| node.id) != stack.nodes.first().map(|node| node.id)
            || extent.x0 != 0
            || extent.y0 != 0
            || extent.is_empty()
            || extent.width() > 262144
            || extent.height() > 262144
            || u64::from(extent.width()).checked_mul(u64::from(extent.height())).is_none_or(|pixels| pixels > 1 << 30)
            || extent != doc.bounds()
            || extent.size() != doc.size
            || state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).and_then(|item| item.raster_bounds) != Some(extent)
        {
            return Err(CompositionError::Unsupported("point checked origin canonical/native extent"));
        }
    }
    interrupt.check().map_err(|_|photocraft_raster::RasterInputError::Cancelled)?;
    Ok(PointCurrentFamilies {channel_mixer,mono_signed,color_overlay,color_rotate,unpremultiply,absolute,red_eye,component_extract,alpha_clip})
}

/// Current structural/profile semantics, independent of inert original provenance.
fn qualify_generic_point_context(
    doc: &Document,
    layer: &Layer,
    state: &photocraft_doc::XcfState,
    stack: &photocraft_doc::xcf_filters::FilterStack,
    context: (Rect, &Profile, SampleEncoding),
    interrupt: &photocraft_raster::Interrupt<'_>,
) -> Result<()> {
    let (extent, profile, stored) = context;
    let Some(surface) = layer.surface() else { return Err(CompositionError::Unsupported("generic point raster owner")) };
    let profile_kind = match doc.mode {
        ColorMode::Rgb => photocraft_cms::Builtin::Srgb,
        ColorMode::Grayscale => photocraft_cms::Builtin::SGray,
        _ => return Err(CompositionError::Unsupported("generic point color model")),
    };
    let implicit = matches!(
        (&doc.working_profile_interpretation, doc.mode),
        (photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. }, ColorMode::Rgb)
            | (photocraft_doc::WorkingProfileInterpretation::ImplicitSGray { .. }, ColorMode::Grayscale)
    );
    let PointCurrentFamilies {channel_mixer,mono_signed,color_overlay,color_rotate,unpremultiply,absolute,red_eye,component_extract,alpha_clip}=qualify_point_family_context(doc,layer,state,stack,extent,profile,stored,interrupt)?;
    qualify_generic_point_storage_metadata(doc, layer, state, stack, extent, stored, surface.format(), surface.default_bytes())?;
    if !implicit || profile.to_bytes() != profile_kind.profile().to_bytes() {
        return Err(CompositionError::Unsupported("unqualified generic point current raster context"));
    }
    let format = surface.format();
    let pixel_bytes = format.bytes_per_pixel();
    let literal_working = stack.nodes.first().is_some_and(|node| {
        (node.operation.name == "gegl:color-exchange" && node.operation.schema_epoch == crate::gimp_filters::COLOR_EXCHANGE_PRECISION_EPOCH)
            || (node.operation.name == "gegl:color-to-alpha" && node.operation.schema_epoch == crate::gimp_filters::COLOR_TO_ALPHA_PRECISION_EPOCH)
            || component_extract
            || color_overlay
            || channel_mixer
            || mono_signed
            || red_eye
            || absolute
            || unpremultiply
            || color_rotate
            || alpha_clip
    });
    // Public authored conversion controls distinguish Alpha Clip's Gray/RGB
    // boundary. Gray uses linear RGB at every stored encoding; RGB uses
    // nonlinear RGB only for nonlinear storage. Other measured literal-color
    // operations retain their nonlinear RGB boundary.
    let working_space = if absolute
        || unpremultiply
        || channel_mixer
        || mono_signed
        || alpha_clip && (doc.mode == ColorMode::Grayscale || stored != SampleEncoding::NonlinearProfile)
    {
        GimpSpace::LinearRgb
    } else {
        GimpSpace::ProfileRgb
    };
    let working = if literal_working { Some(WorkingSpace::new_with_interrupt(profile, stored, working_space, interrupt)?) } else { None };
    let channel = if channel_mixer {
        let node = stack.nodes.first().ok_or(CompositionError::Invalid("Channel Mixer node"))?;
        Some(crate::gimp_channel_mixer::ChannelMixerCandidate::new(profile, stored, crate::gimp_channel_mixer::typed_arguments(&node.arguments)?)?)
    } else {
        None
    };
    let mono = if mono_signed {
        let node = stack.nodes.first().ok_or(CompositionError::Invalid("expanded Mono Mixer node"))?;
        Some(crate::gimp_mono_mixer::MonoMixerCandidate::new(profile, stored, crate::gimp_mono_mixer::typed_arguments(&node.arguments)?)?)
    } else {
        None
    };
    let overlay = if color_overlay {
        let node = stack.nodes.first().ok_or(CompositionError::Invalid("Color Overlay node"))?;
        let (color, srgb) = crate::gimp_color_overlay::literal_arguments(&node.arguments)?;
        Some(crate::gimp_color_overlay::ColorOverlayCandidate::new(profile, stored, color, srgb)?)
    } else {
        None
    };
    let rotate = if color_rotate {
        let node = stack.nodes.first().ok_or(CompositionError::Invalid("Color Rotate node"))?;
        Some(crate::gimp_color_rotate::ColorRotateCandidate::new(profile, stored, crate::gimp_color_rotate::typed_arguments(&node.arguments)?)?)
    } else {
        None
    };
    let absolute = if absolute { Some(crate::gimp_absolute::AbsoluteCandidate::new(profile, stored)?) } else { None };
    let unpremultiply = if unpremultiply {
        let node = stack.nodes.first().ok_or(CompositionError::Invalid("Unpremultiply node"))?;
        if stack.nodes.len() != 1 {
            return Err(CompositionError::Unsupported("Unpremultiply singleton current context"));
        }
        Some((crate::gimp_unpremultiply::canonical_candidate(node, profile, stored, extent, state.provenance.format_version)?, node.enabled))
    } else {
        None
    };
    let red_eye = if red_eye {
        let node = stack.nodes.first().ok_or(CompositionError::Invalid("Red Eye node"))?;
        Some(crate::gimp_red_eye::RedEyeCandidate::new(profile, stored, crate::gimp_red_eye::typed_threshold(&node.arguments)?)?)
    } else {
        None
    };
    for (coordinate, tile) in surface.tiles() {
        let x = coordinate.tx.checked_mul(256).ok_or(CompositionError::Invalid("generic point tile coordinate"))?;
        let y = coordinate.ty.checked_mul(256).ok_or(CompositionError::Invalid("generic point tile coordinate"))?;
        if tile.bytes().len() != 65536usize.checked_mul(pixel_bytes).ok_or(CompositionError::Invalid("generic point tile bytes"))? {
            return Err(CompositionError::Invalid("generic point tile length"));
        }
        for (index, pixel) in tile.bytes().chunks_exact(pixel_bytes).enumerate() {
            if index % 64 == 0 {
                interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
            }
            let px = x.checked_add((index % 256) as i32).ok_or(CompositionError::Invalid("generic point tile coordinate"))?;
            let py = y.checked_add((index / 256) as i32).ok_or(CompositionError::Invalid("generic point tile coordinate"))?;
            if !extent.contains(px, py) {
                if pixel.iter().any(|byte| *byte != 0) {
                    return Err(CompositionError::Unsupported("generic point pixels outside declared drawable"));
                }
                continue;
            }
            let mut native = [0f64; 4];
            for channel in 0..format.channels() {
                let value = photocraft_color::try_read_sample_f64(pixel, format.sample, channel)
                    .map_err(|_| CompositionError::Invalid("generic point native sample"))?;
                if !value.is_finite() || ((!literal_working || channel + 1 == format.channels()) && !(0.0..=1.0).contains(&value)) {
                    return Err(CompositionError::Unsupported("generic point source unit samples"));
                }
                *native.get_mut(channel).ok_or(CompositionError::Invalid("generic point sample count"))? = value;
            }
            if let Some((candidate, enabled)) = &unpremultiply {
                candidate.apply_with(&native, *enabled, interrupt, &|_| Ok(()))?;
            } else if let Some(channel) = &channel {
                let pixel = native.get(..format.channels()).ok_or(CompositionError::Invalid("Channel Mixer native channels"))?;
                if channel.working_input(pixel)?.iter().take(3).any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
                    return Err(CompositionError::Unsupported("Channel Mixer current linear Float unit input"));
                }
                channel.apply_with_current_storage(pixel, stack.nodes.first().is_some_and(|node| node.enabled), doc.depth, interrupt, &|_| Ok(()))?;
            } else if let Some(mono) = &mono {
                let pixel = native.get(..format.channels()).ok_or(CompositionError::Invalid("Mono Mixer native channels"))?;
                if mono.working_input(pixel)?.iter().take(3).any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
                    return Err(CompositionError::Unsupported("Mono Mixer current linear Float unit input"));
                }
                mono.apply_with(pixel, stack.nodes.first().is_some_and(|node| node.enabled), interrupt, &|_| Ok(()))?;
            } else if let Some(overlay) = &overlay {
                overlay.working_input(native.get(..format.channels()).ok_or(CompositionError::Invalid("Color Overlay native channels"))?)?;
            } else if let Some(rotate) = &rotate {
                rotate.working_input(native.get(..format.channels()).ok_or(CompositionError::Invalid("Color Rotate native channels"))?)?;
            } else if let Some(absolute) = &absolute {
                absolute.working_input(native.get(..format.channels()).ok_or(CompositionError::Invalid("Absolute native channels"))?)?;
            } else if let Some(red_eye) = &red_eye {
                red_eye.working_input(native.get(..format.channels()).ok_or(CompositionError::Invalid("Red Eye native channels"))?)?;
            } else if let Some(working) = &working {
                let color = if doc.mode == ColorMode::Grayscale { [native[0]; 3] } else { [native[0], native[1], native[2]] };
                // Check the measured Float32 working boundary. Retain source
                // bits without a native clamp or epsilon allowance.
                if working.convert(color, true)?.iter().any(|value| {
                    let value = *value as f32;
                    !value.is_finite() || !(0.0..=1.0).contains(&value)
                }) {
                    return Err(CompositionError::Unsupported("literal-color working samples outside unit domain"));
                }
            }
        }
    }
    Ok(())
}
fn qualify_floating(doc: &Document, state: &photocraft_doc::xcf_state::XcfState, interrupt: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    let Some(attachment) = &state.floating else { return Ok(()) };
    if doc.layers.len() != 2 || !state.source_items.is_empty() || !state.filter_stacks.is_empty() {
        return Err(CompositionError::Unsupported("floating attachment tree/source/filter domain"));
    }
    state.validate_with(doc, interrupt).map_err(|error| {
        if error.code == "cancelled" {
            photocraft_raster::RasterInputError::Cancelled.into()
        } else {
            CompositionError::Invalid("invalid floating attachment state")
        }
    })?;
    let target = doc.layers.first().ok_or(CompositionError::Invalid("floating target missing"))?;
    let floating = doc.layers.get(1).ok_or(CompositionError::Invalid("floating source missing"))?;
    if attachment.target != ItemRef::Layer(target.id) || attachment.layer != floating.id {
        return Err(CompositionError::Unsupported("floating attachment requires immediate raster target"));
    }
    let mut extents = Vec::new();
    for layer in [target, floating] {
        if !layer.visible
            || layer.opacity != 1.0
            || layer.fill_opacity != 1.0
            || layer.mask.is_some()
            || layer.vector_mask.is_some()
            || !layer.surface().is_some_and(|surface| surface.format() == doc.pixel_format())
        {
            return Err(CompositionError::Unsupported("floating raster visibility/format/mask/opacity domain"));
        }
        let item =
            state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).ok_or(CompositionError::Invalid("floating raster metadata missing"))?;
        let policy = item.composition.as_ref().ok_or(CompositionError::Unsupported("floating Normal policy missing"))?;
        if policy.source_mode != 28
            || policy.legacy
            || policy.pass_through
            || !policy.resolved.as_ref().is_some_and(|resolved| resolved.composite_space == GimpSpace::ProfileRgb && resolved.operator == GimpOperator::Union)
        {
            return Err(CompositionError::Unsupported("floating Normal Profile/Union policy required"));
        }
        extents.push(item.raster_bounds.ok_or(CompositionError::Unsupported("floating declared extent missing"))?);
    }
    let target_extent = extents.first().ok_or(CompositionError::Invalid("floating target extent"))?;
    let floating_extent = extents.get(1).ok_or(CompositionError::Invalid("floating source extent"))?;
    if *target_extent != doc.bounds() || floating_extent.intersect(target_extent) != *floating_extent {
        return Err(CompositionError::Unsupported("floating outside-target clipping evaluator"));
    }
    Ok(())
}
impl<'a> PreparedRenderer<'a> {
    /// The immutable document snapshot whose current semantics were prepared.
    pub fn document(&self) -> &'a Document {
        self.doc
    }
    /// Whether this current policy requires explicit aggregate preparation/source
    /// leases rather than the compatibility constructor.
    pub fn requires_preparation_budget(doc: &Document) -> bool {
        normal_ordinary::applies(doc)
    }
    pub fn new(doc: &'a Document) -> Result<Self> {
        Self::new_with_interrupt(doc, &photocraft_raster::Interrupt::NONE)
    }
    /// Source evidence validation shares the caller's cancellation/deadline
    /// checks, including every bounded chunk of retained-cache hashing.
    pub fn new_with_interrupt(doc: &'a Document, interrupt: &photocraft_raster::Interrupt<'_>) -> Result<Self> {
        if normal_ordinary::applies(doc) {
            return Err(CompositionError::Unsupported("ordinary Normal scene requires retained job preparation accounting"));
        }
        Self::new_internal(doc, interrupt)
    }
    /// Prepares the qualified ordinary Normal scene with caller-owned source and
    /// preparation leases. Output-band allocations keep their existing separate
    /// render callbacks. Failure drops both supplied/created leases atomically.
    pub fn new_budgeted<B: PreparationBudget, S>(
        doc: &'a Document,
        interrupt: &photocraft_raster::Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<LeasedPreparedRenderer<'a, S, B::Lease>> {
        interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        if !normal_ordinary::applies(doc) || doc.layers.len() != 2 {
            return Err(CompositionError::Unsupported("budgeted preparation requires qualified ordinary Normal policy"));
        }
        let icc = doc.icc_profile.as_ref().ok_or(CompositionError::Invalid("missing current ordinary ICC"))?;
        if !(132..=65536).contains(&icc.len()) {
            return Err(CompositionError::Invalid("ordinary current ICC preparation bound"));
        }
        // Two raster nodes, at most two matrix/analytic-curve working spaces,
        // parsed ICC tags and cloned pipeline definitions. The semantic class
        // excludes CLUT/table payloads; 128x original ICC plus64KiB deliberately
        // bounds transient and retained parser/transform allocations together.
        let prepared_bytes = u64::try_from(icc.len())
            .map_err(|_| CompositionError::Invalid("ordinary ICC size overflow"))?
            .checked_mul(128)
            .and_then(|n| n.checked_add(65536))
            .ok_or(CompositionError::Invalid("ordinary preparation size overflow"))?;
        let lease = budget.reserve_prepared_bytes(prepared_bytes)?;
        budget.consume_preparation_work(icc.len() as u64)?;
        let profile = photocraft_cms::xcf_color::parse_bounded_profile(
            icc,
            if doc.mode == ColorMode::Rgb { photocraft_cms::ColorSpace::Rgb } else { photocraft_cms::ColorSpace::Gray },
        )?;
        photocraft_cms::xcf_color::qualify_analytic_profile(&profile)?;
        normal_ordinary::validate(doc, interrupt, budget)?;
        budget.consume_preparation_work(icc.len() as u64)?;
        let renderer = Self::new_internal(doc, interrupt)?;
        Ok(LeasedPreparedRenderer { renderer, _source_lease: source_lease, _prepared_lease: lease })
    }
    /// Nonallocating accounting-class membership, not parameter/sample validation.
    /// Malformed members must fail in `new_accounted` without a legacy fallback.
    pub fn supports_accounted_scalar_preparation(doc: &Document) -> bool {
        accounted::supports(doc)
    }
    /// Additive caller-accounted preparation of already-supported flat implicit
    /// analytic rasters/masks and scalar point plans. This does not certify
    /// runtime cache/output accounting or widen source/operation qualification.
    /// Unsupported accounting complexity is rejected before profile allocation.
    pub fn new_accounted<B: PreparationBudget, S>(
        doc: &'a Document,
        interrupt: &photocraft_raster::Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<LeasedPreparedRenderer<'a, S, B::Lease>> {
        let bytes = accounted::bytes(doc, interrupt, budget)?;
        let lease = budget.reserve_prepared_bytes(bytes)?;
        accounted::validate(doc, interrupt, budget)?;
        let renderer = Self::new_internal(doc, interrupt)?;
        accounted::validate_capacity(&renderer, bytes)?;
        Ok(LeasedPreparedRenderer { renderer, _source_lease: source_lease, _prepared_lease: lease })
    }
    /// Structural selection for the additive plain Normal preparation route.
    /// This does not validate samples, bindings or bounds: malformed members
    /// must reach the fallible constructor rather than a compatibility fallback.
    pub fn supports_plain_normal_budgeted(doc: &Document) -> bool {
        plain_normal::supports(doc)
    }
    /// Prepares one plain current Normal raster on the caller's existing ledger.
    /// The supplied source token must retain an already charged source/model
    /// owner. Both leases survive every output; output allocations are metered
    /// independently by the render callback on that same ledger.
    pub fn new_plain_normal_budgeted<B: PreparationBudget, S>(
        doc: &'a Document,
        interrupt: &photocraft_raster::Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<LeasedPreparedRenderer<'a, S, B::Lease>> {
        interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        if !plain_normal::supports(doc) {
            return Err(CompositionError::Unsupported("budgeted single plain Normal current state"));
        }
        let icc = doc.icc_profile.as_ref().ok_or(CompositionError::Invalid("missing current implicit ICC"))?;
        if !(132..=65536).contains(&icc.len()) {
            return Err(CompositionError::Invalid("plain Normal ICC preparation bound"));
        }
        // Only implicit analytic RGB/Gray and one prepared node/working space
        // are admitted. Reserve before CURRENT profile parsing or allocation.
        let bytes = u64::try_from(icc.len())
            .map_err(|_| CompositionError::Invalid("plain ICC size"))?
            .checked_mul(128)
            .and_then(|n| n.checked_add(65536))
            .ok_or(CompositionError::Invalid("plain preparation size overflow"))?;
        let lease = budget.reserve_prepared_bytes(bytes)?;
        plain_normal::validate(doc, interrupt, budget)?;
        for chunk in icc.chunks(4096) {
            interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
            // Current binding and analytical working-profile construction both
            // inspect these bytes; account for both passes before publication.
            budget.consume_preparation_work((chunk.len() as u64).checked_mul(2).ok_or(CompositionError::Invalid("plain ICC work overflow"))?)?;
        }
        let renderer = Self::new_internal(doc, interrupt)?;
        Ok(LeasedPreparedRenderer { renderer, _source_lease: source_lease, _prepared_lease: lease })
    }
    /// Select the first caller-budgeted point family: captured singleton Exposure
    /// RGB U8/Nonlinear 32x32 with an existing closed canonical Replace policy.
    /// Samples, arguments and active current bindings remain fallible validation.
    pub fn supports_plain_point_budgeted(doc: &Document) -> bool {
        plain_point::supports(doc)
    }
    /// Retains the caller's charged source/model and preparation leases across
    /// both outputs. This singleton uses direct point evaluation, not a tile
    /// cache; only returned output buffers emit render reservations.
    pub fn new_plain_point_budgeted<B: PreparationBudget, S>(
        doc: &'a Document,
        interrupt: &photocraft_raster::Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<LeasedPreparedRenderer<'a, S, B::Lease>> {
        interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        if !plain_point::supports(doc) {
            return Err(CompositionError::Unsupported("budgeted singleton Exposure current state"));
        }
        let icc = doc.icc_profile.as_ref().ok_or(CompositionError::Invalid("missing current implicit ICC"))?;
        if !(132..=65536).contains(&icc.len()) {
            return Err(CompositionError::Invalid("point ICC preparation bound"));
        }
        let lease = budget.reserve_prepared_bytes(plain_point::preparation_bytes(icc.len())?)?;
        plain_point::validate(doc, interrupt, budget)?;
        for chunk in icc.chunks(4096) {
            interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
            budget.consume_preparation_work((chunk.len() as u64).checked_mul(2).ok_or(CompositionError::Invalid("point ICC work overflow"))?)?;
        }
        let renderer = Self::new_internal(doc, interrupt)?;
        Ok(LeasedPreparedRenderer { renderer, _source_lease: source_lease, _prepared_lease: lease })
    }
    /// Select only captured Box Blur RGB/U8 nonlinear32x32 singleton state.
    pub fn supports_plain_box_budgeted(doc: &Document) -> bool {
        plain_box::supports(doc)
    }
    /// Retains charged source/preparation leases across both outputs. Rendering
    /// emits a distinct actual32MiB cache/scratch reservation in addition to its
    /// output buffer; the caller must retain/refund each role on the same ledger.
    pub fn new_plain_box_budgeted<B: PreparationBudget, S>(
        doc: &'a Document,
        interrupt: &photocraft_raster::Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<LeasedPreparedRenderer<'a, S, B::Lease>> {
        interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        if !plain_box::supports(doc) {
            return Err(CompositionError::Unsupported("budgeted singleton Box Blur current state"));
        }
        let icc = doc.icc_profile.as_ref().ok_or(CompositionError::Invalid("missing current implicit ICC"))?;
        if !(132..=65536).contains(&icc.len()) {
            return Err(CompositionError::Invalid("Box ICC preparation bound"));
        }
        let lease = budget.reserve_prepared_bytes(plain_box::preparation_bytes(icc.len())?)?;
        plain_box::validate(doc, interrupt, budget)?;
        for chunk in icc.chunks(4096) {
            interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
            budget.consume_preparation_work((chunk.len() as u64).checked_mul(2).ok_or(CompositionError::Invalid("Box ICC work overflow"))?)?;
        }
        let renderer = Self::new_internal(doc, interrupt)?;
        Ok(LeasedPreparedRenderer { renderer, _source_lease: source_lease, _prepared_lease: lease })
    }
    /// Select only the measured singleton Linear Motion Blur source/edit domain.
    pub fn supports_plain_motion_budgeted(doc: &Document) -> bool {
        plain_motion::supports(doc)
    }
    /// Retains charged source/preparation leases across both outputs. Rendering
    /// emits a distinct actual32MiB cache/scratch reservation in addition to its
    /// output buffer; the caller must retain/refund each role on the same ledger.
    pub fn new_plain_motion_budgeted<B: PreparationBudget, S>(
        doc: &'a Document,
        interrupt: &photocraft_raster::Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<LeasedPreparedRenderer<'a, S, B::Lease>> {
        interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        if !plain_motion::supports(doc) {
            return Err(CompositionError::Unsupported("budgeted singleton Linear Motion Blur current state"));
        }
        let icc = doc.icc_profile.as_ref().ok_or(CompositionError::Invalid("missing current implicit ICC"))?;
        if !(132..=65536).contains(&icc.len()) {
            return Err(CompositionError::Invalid("Motion ICC preparation bound"));
        }
        let lease = budget.reserve_prepared_bytes(plain_motion::preparation_bytes(icc.len())?)?;
        plain_motion::validate(doc, interrupt, budget)?;
        for chunk in icc.chunks(4096) {
            interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
            budget.consume_preparation_work((chunk.len() as u64).checked_mul(2).ok_or(CompositionError::Invalid("Motion ICC work overflow"))?)?;
        }
        let renderer = Self::new_internal(doc, interrupt)?;
        Ok(LeasedPreparedRenderer { renderer, _source_lease: source_lease, _prepared_lease: lease })
    }
    /// Select only the measured singleton quantized Median Blur source/edit domain.
    pub fn supports_plain_median_budgeted(doc: &Document) -> bool {
        plain_median::supports(doc)
    }
    /// Retains charged source/preparation leases across both outputs. Rendering
    /// emits a distinct actual32MiB cache/scratch reservation in addition to its
    /// output buffer; the caller must retain/refund each role on the same ledger.
    pub fn new_plain_median_budgeted<B: PreparationBudget, S>(
        doc: &'a Document,
        interrupt: &photocraft_raster::Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<LeasedPreparedRenderer<'a, S, B::Lease>> {
        interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        if !plain_median::supports(doc) {
            return Err(CompositionError::Unsupported("budgeted singleton quantized Median Blur current state"));
        }
        let icc = doc.icc_profile.as_ref().ok_or(CompositionError::Invalid("missing current implicit ICC"))?;
        if !(132..=65536).contains(&icc.len()) {
            return Err(CompositionError::Invalid("Median ICC preparation bound"));
        }
        let lease = budget.reserve_prepared_bytes(plain_median::preparation_bytes(icc.len())?)?;
        plain_median::validate(doc, interrupt, budget)?;
        for chunk in icc.chunks(4096) {
            interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
            budget.consume_preparation_work((chunk.len() as u64).checked_mul(2).ok_or(CompositionError::Invalid("Median ICC work overflow"))?)?;
        }
        let renderer = Self::new_internal(doc, interrupt)?;
        Ok(LeasedPreparedRenderer { renderer, _source_lease: source_lease, _prepared_lease: lease })
    }
    /// Select only the measured singleton Gaussian Blur source/edit domain.
    pub fn supports_plain_gaussian_budgeted(doc: &Document) -> bool {
        plain_gaussian::supports(doc)
    }
    /// Retains charged source/preparation leases across both outputs. Rendering
    /// emits a distinct actual32MiB cache/scratch reservation in addition to its
    /// output buffer; the caller must retain/refund each role on the same ledger.
    pub fn new_plain_gaussian_budgeted<B: PreparationBudget, S>(
        doc: &'a Document,
        interrupt: &photocraft_raster::Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<LeasedPreparedRenderer<'a, S, B::Lease>> {
        interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        if !plain_gaussian::supports(doc) {
            return Err(CompositionError::Unsupported("budgeted singleton Gaussian Blur current state"));
        }
        let icc = doc.icc_profile.as_ref().ok_or(CompositionError::Invalid("missing current implicit ICC"))?;
        if !(132..=65536).contains(&icc.len()) {
            return Err(CompositionError::Invalid("Gaussian ICC preparation bound"));
        }
        let lease = budget.reserve_prepared_bytes(plain_gaussian::preparation_bytes(icc.len())?)?;
        plain_gaussian::validate(doc, interrupt, budget)?;
        for chunk in icc.chunks(4096) {
            interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
            budget.consume_preparation_work((chunk.len() as u64).checked_mul(2).ok_or(CompositionError::Invalid("Gaussian ICC work overflow"))?)?;
        }
        let renderer = Self::new_internal(doc, interrupt)?;
        Ok(LeasedPreparedRenderer { renderer, _source_lease: source_lease, _prepared_lease: lease })
    }
    /// Select only the measured singleton Brightness/Contrast source/edit domain.
    pub fn supports_plain_brightness_contrast_budgeted(doc: &Document) -> bool {
        plain_brightness_contrast::supports(doc)
    }
    /// Retains charged source/preparation leases across both outputs. Rendering
    /// uses direct point evaluation, with only the returned output reservation.
    /// No neighborhood cache is introduced by this preparation bridge.
    pub fn new_plain_brightness_contrast_budgeted<B: PreparationBudget, S>(
        doc: &'a Document,
        interrupt: &photocraft_raster::Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<LeasedPreparedRenderer<'a, S, B::Lease>> {
        interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        if !plain_brightness_contrast::supports(doc) {
            return Err(CompositionError::Unsupported("budgeted singleton Brightness/Contrast current state"));
        }
        let icc = doc.icc_profile.as_ref().ok_or(CompositionError::Invalid("missing current implicit ICC"))?;
        if !(132..=65536).contains(&icc.len()) {
            return Err(CompositionError::Invalid("Brightness Contrast ICC preparation bound"));
        }
        let lease = budget.reserve_prepared_bytes(plain_brightness_contrast::preparation_bytes(icc.len())?)?;
        plain_brightness_contrast::validate(doc, interrupt, budget)?;
        for chunk in icc.chunks(4096) {
            interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
            budget.consume_preparation_work((chunk.len() as u64).checked_mul(2).ok_or(CompositionError::Invalid("Brightness Contrast ICC work overflow"))?)?;
        }
        let renderer = Self::new_internal(doc, interrupt)?;
        Ok(LeasedPreparedRenderer { renderer, _source_lease: source_lease, _prepared_lease: lease })
    }
    /// Caller-held runtime font; no stored grant or new source qualification.
    pub fn new_with_authorized_font(doc: &'a Document, font: &'a PreparedRuntimeFont<'a>, interrupt: &photocraft_raster::Interrupt<'_>) -> Result<Self> {
        if normal_ordinary::applies(doc) {
            return Err(CompositionError::Unsupported("ordinary Normal scene requires retained job preparation accounting"));
        }
        Self::new_internal_with_authorized_font(doc, Some(font), interrupt)
    }
    /// Cheap accounting-class membership only; constructor checks all source,
    /// geometry, profile and current-raster relationships without widening them.
    pub fn supports_authorized_text_budgeted(doc: &Document) -> bool {
        text_budgeted::supports(doc)
    }
    /// Existing v19 exact-font text preparation on the caller's source/decoder
    /// ledger. Font authority is borrowed for this renderer's lifetime; no
    /// resource payload, locator permission or default source gate is changed.
    pub fn new_budgeted_with_authorized_font<B: PreparationBudget, S>(
        doc: &'a Document,
        font: &'a PreparedRuntimeFont<'a>,
        interrupt: &photocraft_raster::Interrupt<'_>,
        budget: &mut B,
        source_lease: S,
    ) -> Result<LeasedPreparedRenderer<'a, S, B::Lease>> {
        // The helper's fixed header allowance must cover the actual generic
        // source/preparation lease carriers, not merely their common small form.
        if std::mem::size_of::<LeasedPreparedRenderer<'a, S, B::Lease>>() > 4096 {
            return Err(CompositionError::Unsupported("text preparation lease carrier exceeds reserved header"));
        }
        let lease = text_budgeted::reserve(doc, font, budget, interrupt)?;
        text_budgeted::validate_text(doc, font, budget, interrupt)?;
        let renderer = Self::new_internal_with_authorized_font(doc, Some(font), interrupt)?;
        // Exactly one raster node and its same-profile working space. Refuse an
        // unexpected allocator capacity before returning a leased preparation.
        if renderer.layers.capacity() > 2 || renderer.spaces.capacity() > 4 {
            return Err(CompositionError::Invalid("text prepared capacity exceeds reserved headers"));
        }
        interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        Ok(LeasedPreparedRenderer { renderer, _source_lease: source_lease, _prepared_lease: lease })
    }
    fn new_internal(doc: &'a Document, interrupt: &photocraft_raster::Interrupt<'_>) -> Result<Self> {
        Self::new_internal_with_authorized_font(doc, None, interrupt)
    }
    fn new_internal_with_authorized_font(
        doc: &'a Document,
        authorized_font: Option<&'a PreparedRuntimeFont<'a>>,
        interrupt: &photocraft_raster::Interrupt<'_>,
    ) -> Result<Self> {
        interrupt.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        if doc.has_backed_rasters()? {
            return Err(CompositionError::Unsupported("fallible native storage requires the bounded backed renderer"));
        }
        if !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale) {
            return Err(CompositionError::Unsupported("wide document color model"));
        }
        if let Some(state) = &doc.xcf_state {
            if state.floating.is_some() {
                qualify_floating(doc, state, interrupt)?;
            }
            if !state.source_items.is_empty() {
                let mut linked_count = 0usize;
                let mut linked_pixels = 0u64;
                for source in &state.source_items {
                    if matches!(source.source, photocraft_doc::xcf_items::EditableSource::Linked { .. }) {
                        linked_count = linked_count.checked_add(1).ok_or(CompositionError::Invalid("linked preparation count overflow"))?;
                        let cache = source.cache.as_ref().ok_or(CompositionError::Unsupported("linked preparation requires retained source cache"))?;
                        linked_pixels = linked_pixels
                            .checked_add(u64::from(cache.width) * u64::from(cache.height))
                            .ok_or(CompositionError::Invalid("linked preparation pixel overflow"))?;
                        if linked_count > 8 || linked_pixels > 4_000_000 {
                            return Err(CompositionError::Unsupported("linked preparation exceeds bounded eight-owner/four-million-pixel policy"));
                        }
                    }
                }
                // Modified sources select the current raster. Unmodified sources
                // must pass their specialized regeneration qualifiers below.
                // Validate immutable evidence and relationships at preparation.
                state.validate_with(doc, interrupt).map_err(|error| {
                    if error.code == "cancelled" {
                        photocraft_raster::RasterInputError::Cancelled.into()
                    } else {
                        CompositionError::Invalid("invalid modified source state")
                    }
                })?;
                for source in &state.source_items {
                    let linked = matches!(source.source, photocraft_doc::xcf_items::EditableSource::Linked { .. });
                    if linked {
                        crate::gimp_link_items::qualify_display_source(doc, source, interrupt)?;
                    }
                    if source.cache.is_none()
                        || (!linked
                            && (source.transform.is_some()
                                || !matches!(
                                    source.source,
                                    photocraft_doc::xcf_items::EditableSource::Text { source_modified: true, .. }
                                        | photocraft_doc::xcf_items::EditableSource::Text { source_modified: false, .. }
                                        | photocraft_doc::xcf_items::EditableSource::Vector { source_modified: true, .. }
                                        | photocraft_doc::xcf_items::EditableSource::Vector { source_modified: false, .. }
                                )))
                    {
                        return Err(CompositionError::Unsupported("source regeneration or linked resource evaluator"));
                    }
                    let ItemRef::Layer(id) = source.owner else {
                        return Err(CompositionError::Invalid("modified source owner"));
                    };
                    if !doc.layer(id).is_some_and(|layer| matches!(layer.content, LayerContent::Raster(_))) {
                        return Err(CompositionError::Unsupported("modified source requires a current raster"));
                    }
                }
            }
        }
        if let Some(state) = &doc.xcf_state {
            if !doc.channels.is_empty() {
                state.validate_with(doc, interrupt).map_err(|error| {
                    if error.code == "cancelled" {
                        photocraft_raster::RasterInputError::Cancelled.into()
                    } else {
                        CompositionError::Invalid("invalid imported channel state")
                    }
                })?;
                for channel in &doc.channels {
                    let item = state
                        .items
                        .iter()
                        .find(|item| item.item == ItemRef::Channel(channel.id))
                        .ok_or(CompositionError::Invalid("imported channel metadata missing"))?;
                    if item.visible || channel.spot.is_some() {
                        return Err(CompositionError::Unsupported("visible or spot channel appearance evaluator"));
                    }
                }
            }
            for stack in &state.filter_stacks {
                let ItemRef::Layer(id) = stack.owner else {
                    return Err(CompositionError::Unsupported("point filter non-layer owner"));
                };
                if doc.layer(id).is_none() {
                    return Err(CompositionError::Invalid("point filter owner missing"));
                }
            }
        }
        let stored = doc.sample_encoding.ok_or(CompositionError::Invalid("missing source sample encoding"))?;
        // Current interpretation selects current ICC bytes. Immutable source ICC
        // cannot validate a later assignment or supply stale source primaries.
        // A bound implicit marker recovers its analytical nonlinear basis even
        // when current samples use a derived linear/perceptual working profile.
        doc.working_profile_interpretation
            .validate_with(doc.icc_profile.as_deref().map(Vec::as_slice), doc.mode)
            .map_err(|_| CompositionError::Invalid("stale or incompatible active working profile identity"))?;
        let profile = if !matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::Icc) {
            photocraft_cms::xcf_color::implicit_source_profile(
                doc.icc_profile.as_deref().map(Vec::as_slice).ok_or(CompositionError::Invalid("missing implicit working profile ICC"))?,
                if doc.mode == ColorMode::Rgb { photocraft_cms::ColorSpace::Rgb } else { photocraft_cms::ColorSpace::Gray },
                crate::gimp_composite::encoding(stored),
            )?
        } else {
            photocraft_cms::xcf_color::parse_bounded_profile(
                doc.icc_profile.as_deref().map(Vec::as_slice).ok_or(CompositionError::Invalid("missing active ICC profile"))?,
                if doc.mode == ColorMode::Rgb { photocraft_cms::ColorSpace::Rgb } else { photocraft_cms::ColorSpace::Gray },
            )?
        };
        let mut spaces = Vec::new();
        let layers = prepare(doc, &doc.layers, &profile, stored, &mut spaces, 0, interrupt, authorized_font)?;
        prepare_control::checkpoint(interrupt)?;
        Ok(Self { doc, layers, spaces, authorized_font })
    }
    pub fn render(&self, rect: Rect, poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<WideBuffer> {
        self.render_with_interrupt(rect, poll, &photocraft_raster::Interrupt::NONE)
    }
    /// Job cancellation/deadline token is checked within neighborhood kernels too.
    pub fn render_with_interrupt(
        &self,
        rect: Rect,
        poll: &mut impl FnMut(WideWork) -> Result<()>,
        interrupt: &photocraft_raster::Interrupt<'_>,
    ) -> Result<WideBuffer> {
        poll(WideWork::SampleUnits(0))?;
        let width = u64::try_from(i64::from(rect.x1) - i64::from(rect.x0)).map_err(|_| CompositionError::Invalid("render bounds"))?;
        let height = u64::try_from(i64::from(rect.y1) - i64::from(rect.y0)).map_err(|_| CompositionError::Invalid("render bounds"))?;
        let pixels = width.checked_mul(height).ok_or(CompositionError::Invalid("render size overflow"))?;
        let resident = pixels.checked_mul(32).ok_or(CompositionError::Invalid("render size overflow"))?;
        if width > 262144 || height > 262144 || resident > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
            return Err(CompositionError::Invalid("render resident/side limit"));
        }
        poll(WideWork::ReserveBytes(resident))?;
        let mut job = RenderJob {
            cache: crate::gimp_neighborhood::TileCache::new(),
            interrupt,
            groups: Vec::new(),
            doc: self.doc,
            vectors: Vec::new(),
            vector_bytes: 0,
            texts: Vec::new(),
            text_bytes: 0,
            supplied_font: self.authorized_font,
            text_font: None,
            font_bytes: 0,
        };
        let mut result = (|| {
            let mut px = Vec::new();
            px.try_reserve_exact(usize::try_from(pixels).map_err(|_| CompositionError::Invalid("render size"))?)
                .map_err(|_| CompositionError::Invalid("wide render allocation denied"))?;
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    if interrupt.cancelled() {
                        return Err(photocraft_raster::RasterInputError::Cancelled.into());
                    }
                    px.push(stack(&self.layers, &self.spaces, [0.0; 4], (x, y), poll, &mut job, false)?);
                }
            }
            poll(WideWork::SampleUnits(0))?;
            Ok(WideBuffer { rect, px })
        })();
        if let Err(error) = job.finish(poll) {
            result = Err(error);
        }
        if result.is_err() {
            let _ = poll(WideWork::ReleaseBytes(resident));
        }
        result
    }

    /// Derived display reduction with authoritative f64 per-source-pixel composition and
    /// premultiplied averaging. No full-resolution composite is allocated.
    pub fn render_reduced(&self, w: u32, h: u32, damage: Option<Rect>, poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<crate::Buffer> {
        self.render_reduced_with_interrupt(w, h, damage, poll, &photocraft_raster::Interrupt::NONE)
    }
    pub fn render_reduced_with_interrupt(
        &self,
        w: u32,
        h: u32,
        damage: Option<Rect>,
        poll: &mut impl FnMut(WideWork) -> Result<()>,
        interrupt: &photocraft_raster::Interrupt<'_>,
    ) -> Result<crate::Buffer> {
        poll(WideWork::SampleUnits(0))?;
        let fw = u64::from(self.doc.size.width);
        let fh = u64::from(self.doc.size.height);
        if fw == 0 || fh == 0 || fw > 262144 || fh > 262144 || w == 0 || h == 0 || u64::from(w) > fw || u64::from(h) > fh {
            return Err(CompositionError::Invalid("display reduction dimensions"));
        }
        let (w, h) = (u64::from(w), u64::from(h));
        let cell = |x: u64, f: u64, n: u64| ((x + 1) * n - 1) / f;
        let span = |t: u64, f: u64, n: u64| (t * f / n, ((t + 1) * f / n).max(t * f / n + 1).min(f));
        let bounds = self.doc.bounds();
        let out = if let Some(d) = damage {
            let d = d.intersect(&bounds);
            if d.is_empty() {
                return Ok(crate::Buffer { rect: Rect::EMPTY, px: Vec::new() });
            }
            Rect::new(
                cell(d.x0 as u64, fw, w) as i32,
                cell(d.y0 as u64, fh, h) as i32,
                cell((d.x1 - 1) as u64, fw, w) as i32 + 1,
                cell((d.y1 - 1) as u64, fh, h) as i32 + 1,
            )
        } else {
            Rect::from_xywh(0, 0, w as u32, h as u32)
        };
        let count = u64::from(out.width()).checked_mul(u64::from(out.height())).ok_or(CompositionError::Invalid("display reduction size"))?;
        let reserved = count.checked_mul(16).ok_or(CompositionError::Invalid("display reduction size"))?;
        if reserved > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
            return Err(CompositionError::Invalid("display reduction resident limit"));
        }
        poll(WideWork::ReserveBytes(reserved))?;
        let mut job = RenderJob {
            cache: crate::gimp_neighborhood::TileCache::new(),
            interrupt,
            groups: Vec::new(),
            doc: self.doc,
            vectors: Vec::new(),
            vector_bytes: 0,
            texts: Vec::new(),
            text_bytes: 0,
            supplied_font: self.authorized_font,
            text_font: None,
            font_bytes: 0,
        };
        let mut result = (|| {
            let mut px = Vec::new();
            px.try_reserve_exact(usize::try_from(count).map_err(|_| CompositionError::Invalid("display reduction size"))?)
                .map_err(|_| CompositionError::Invalid("display reduction allocation denied"))?;
            for oy in out.y0..out.y1 {
                for ox in out.x0..out.x1 {
                    let (x0, x1) = span(ox as u64, fw, w);
                    let (y0, y1) = span(oy as u64, fh, h);
                    let mut acc = [0.0; 4];
                    for y in y0..y1 {
                        for x in x0..x1 {
                            if interrupt.cancelled() {
                                return Err(photocraft_raster::RasterInputError::Cancelled.into());
                            }
                            let p = stack(&self.layers, &self.spaces, [0.0; 4], (x as i32, y as i32), poll, &mut job, false)?;
                            for i in 0..3 {
                                acc[i] += p[i] * p[3];
                            }
                            acc[3] += p[3];
                        }
                    }
                    let n = ((x1 - x0) * (y1 - y0)) as f64;
                    let alpha = acc[3];
                    let pixel = if alpha > 0.0 { [acc[0] / alpha, acc[1] / alpha, acc[2] / alpha, alpha / n] } else { [0.0; 4] };
                    if pixel.iter().any(|v| !v.is_finite() || !(*v as f32).is_finite()) {
                        return Err(CompositionError::Invalid("display conversion exceeds f32 range"));
                    }
                    px.push(pixel.map(|v| v as f32));
                }
            }
            poll(WideWork::SampleUnits(0))?;
            Ok(crate::Buffer { rect: out, px })
        })();
        if let Err(error) = job.finish(poll) {
            result = Err(error);
        }
        if result.is_err() {
            let _ = poll(WideWork::ReleaseBytes(reserved));
        }
        result
    }
}

#[cfg(test)]
mod runtime_text_font_drafts {
    use super::*;
    fn doc() -> Document {
        Document::new("Own runtime font draft", photocraft_geom::Size { width: 1, height: 1 }, ColorMode::Rgb, photocraft_color::SampleType::U8)
    }
    fn job<'a, 'b>(doc: &'a Document, control: &'a photocraft_raster::Interrupt<'b>, font: Option<&'a PreparedRuntimeFont<'a>>) -> RenderJob<'a, 'b> {
        RenderJob {
            cache: crate::gimp_neighborhood::TileCache::new(),
            interrupt: control,
            groups: Vec::new(),
            doc,
            vectors: Vec::new(),
            vector_bytes: 0,
            texts: Vec::new(),
            text_bytes: 0,
            supplied_font: font,
            text_font: None,
            font_bytes: 0,
        }
    }
    #[test]
    fn prepared_font_is_shared_and_released_once_per_job() {
        let doc = doc();
        let control = photocraft_raster::Interrupt::NONE;
        let font = runtime_font_input::prepare_with_charge(
            runtime_font_input::RuntimeInput { bytes: &photocraft_text::fonts::INTER_REGULAR[..], identity: runtime_font_input::FaceIdentity::inter_regular() },
            runtime_font_input::Limits::default(),
            &control,
            &mut |_| Ok(()),
            &mut |_| Ok(()),
        )
        .unwrap();
        let mut job = job(&doc, &control, Some(&font));
        let mut reserved = 0u64;
        let mut work = 0u64;
        let mut meter = |event| {
            match event {
                WideWork::ReserveBytes(n) => reserved += n,
                WideWork::SampleUnits(n) => work += n,
                _ => {}
            }
            Ok(())
        };
        prepare_text_font(&mut job, &mut meter).unwrap();
        prepare_text_font(&mut job, &mut meter).unwrap();
        drop(meter);
        assert_eq!(reserved, font.requirements().peak_bytes);
        assert_eq!(work, font.requirements().work_units);
        assert!(std::ptr::eq(job.text_font.as_ref().unwrap().handle(), &font));
        let mut released = 0;
        job.finish(&mut |event| {
            if let WideWork::ReleaseBytes(n) = event {
                released += n;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(released, reserved);
        assert!(job.text_font.is_none());
        assert_eq!(job.font_bytes, 0);
    }
    #[test]
    fn work_failure_after_font_reserve_keeps_no_handle_and_releases_lease() {
        let doc = doc();
        let control = photocraft_raster::Interrupt::NONE;
        let mut job = job(&doc, &control, None);
        let mut reserved = 0;
        assert!(
            prepare_text_font(&mut job, &mut |event| {
                match event {
                    WideWork::ReserveBytes(n) => reserved += n,
                    WideWork::SampleUnits(n) if n > 0 => return Err(CompositionError::Invalid("Own denied font work")),
                    _ => {}
                }
                Ok(())
            })
            .is_err()
        );
        assert!(job.text_font.is_none());
        assert!(reserved > 0);
        let mut released = 0;
        job.finish(&mut |event| {
            if let WideWork::ReleaseBytes(n) = event {
                released += n;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(reserved, released);
    }
    #[test]
    fn pre_cancelled_font_preparation_does_not_charge_or_publish() {
        let doc = doc();
        let signal = || true;
        let control = photocraft_raster::Interrupt::cancel_only(&signal);
        let mut job = job(&doc, &control, None);
        let mut calls = 0;
        assert!(
            prepare_text_font(&mut job, &mut |_| {
                calls += 1;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(calls, 0);
        assert!(job.text_font.is_none());
        assert_eq!(job.font_bytes, 0);
        assert!(doc.xcf_state.is_none());
    }
}
