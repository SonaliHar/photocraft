//! Everyday Photoshop commands: clipboard (Cut/Copy/Copy Merged/Paste/Paste in Place), Layer via
//! Copy/Cut, Merge Visible, Auto Tone/Contrast/Color, Toggle Last State and Transform Again.

use photocraft_color::PixelFormat;
use photocraft_doc::adjust::LevelsChannel;
use photocraft_doc::{Adjustment, Document, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The paste commands' `"target"` param (for `concat!` into their params docs).
macro_rules! paste_target {
    () => {
        r#""target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target (not pixels: the luminosity pasted into that mask or channel)"#
    };
}
pub(crate) use paste_target;

/// Pixels on the session clipboard, with where they came from (for Paste in Place).
#[derive(Clone, Debug)]
pub struct Clip {
    pub surface: Surface,
    pub bounds: Rect,
    pub color: ClipColorContext,
    pub composition: Vec<photocraft_doc::import_compositing::GimpComposition>,
    pub layers: Option<std::sync::Arc<Document>>,
    pub resolution: Option<f32>,
}

impl Clip {
    /// The OS image-clipboard service supplies encoded sRGB RGBA8 without profile metadata.
    pub fn from_external_srgb(surface: Surface, bounds: Rect) -> Result<Self> {
        if surface.format() != PixelFormat::RGBA8 {
            return Err(pixel_error("external image clipboard must be sRGB RGBA8"));
        }
        surface.try_validate_tile_origins().map_err(pixel_error)?;
        Ok(Self {
            surface,
            bounds,
            composition: Vec::new(),
            layers: None,
            resolution: None,
            color: ClipColorContext {
                mode: photocraft_doc::ColorMode::Rgb,
                icc_profile: Some(photocraft_cms::Builtin::Srgb.profile().to_bytes()),
                sample_encoding: Some(photocraft_doc::SampleEncoding::NonlinearProfile),
                interpretation: photocraft_doc::WorkingProfileInterpretation::Icc,
            },
        })
    }
}

/// Clipboard samples retain their current working interpretation independently of source documents.
#[derive(Clone, Debug)]
pub struct ClipColorContext {
    pub mode: photocraft_doc::ColorMode,
    pub icc_profile: Option<std::sync::Arc<Vec<u8>>>,
    pub sample_encoding: Option<photocraft_doc::SampleEncoding>,
    pub interpretation: photocraft_doc::WorkingProfileInterpretation,
}
impl ClipColorContext {
    fn from_document(doc: &Document) -> Result<Self> {
        crate::color_cmds::try_document_profile(doc)?;
        Ok(Self {
            mode: doc.mode,
            icc_profile: doc.icc_profile.clone(),
            sample_encoding: doc.sample_encoding,
            interpretation: doc.working_profile_interpretation.clone(),
        })
    }
    fn profile(&self, mode: photocraft_doc::ColorMode) -> Result<std::sync::Arc<photocraft_cms::Profile>> {
        if mode != self.mode {
            return Err(EngineError::Other("clipboard color context does not match its samples".into()));
        }
        let mut doc = Document::new("Clipboard", photocraft_geom::Size::new(1, 1), mode, photocraft_color::SampleType::F64);
        self.apply(&mut doc);
        crate::color_cmds::try_document_profile(&doc)
    }
    fn apply(&self, doc: &mut Document) {
        doc.icc_profile = self.icc_profile.clone();
        doc.sample_encoding = self.sample_encoding;
        doc.working_profile_interpretation = self.interpretation.clone();
    }
}
/// Ordinary pasted layers cannot install the bounded finite-source-alpha behavior yet.
fn require_ordinary_clip(clip: &Clip) -> Result<()> {
    if clip.composition.iter().any(|policy| policy.compatibility_epoch == photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH) {
        return Err(pixel_error("this clipboard retains a finite-alpha composition policy; pasting it as an ordinary layer is not supported yet"));
    }
    let format = clip.surface.format();
    clip.surface.try_validate_tile_origins().map_err(pixel_error)?;
    if !format.alpha {
        return Ok(());
    }
    let validate = |pixel: &[u8]| -> Result<()> {
        let alpha = photocraft_color::try_read_sample_f64(pixel, format.sample, format.mode.color_channels()).map_err(pixel_error)?;
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err(pixel_error("ordinary clipboard layers require finite alpha between zero and one"));
        }
        Ok(())
    };
    validate(clip.surface.default_bytes())?;
    for (_, tile) in clip.surface.tiles() {
        for pixel in tile.bytes().chunks_exact(format.bytes_per_pixel()) {
            validate(pixel)?;
        }
    }
    Ok(())
}
fn pixel_error(error: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("clipboard pixel conversion: {error}"))
}
fn convert_clip(surface: &Surface, context: &ClipColorContext, target: PixelFormat, profile: &photocraft_cms::Profile) -> Result<Surface> {
    convert_clip_with(surface, context, target, profile, &photocraft_raster::Interrupt::NONE)
}
fn convert_clip_with(
    surface: &Surface,
    context: &ClipColorContext,
    target: PixelFormat,
    profile: &photocraft_cms::Profile,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<Surface> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    surface.try_validate_tile_origins_with(control).map_err(pixel_error)?;
    let source = context.profile(surface.format().mode)?;
    let transform = photocraft_cms::Transform::new(&source, profile, photocraft_cms::Intent::RelativeColorimetric, false).map_err(pixel_error)?;
    if transform.is_identity() && surface.format().mode == target.mode {
        return surface.try_convert_with(target, control).map_err(pixel_error);
    }
    let format = surface.format();
    if transform.inputs() != format.mode.color_channels()
        || transform.outputs() != target.mode.color_channels()
        || transform.inputs() > 16
        || transform.outputs() > 16
    {
        return Err(pixel_error("clipboard profile channels do not match sample models"));
    }
    let intermediate = PixelFormat::new(target.mode, format.sample, true);
    let convert = |input: &[u8], output: &mut [u8]| -> Result<()> {
        let mut colors = [0.0_f64; 16];
        let mut result = [0.0_f64; 16];
        for (index, value) in colors.get_mut(..transform.inputs()).ok_or_else(|| pixel_error("source channel range"))?.iter_mut().enumerate() {
            *value = photocraft_color::try_read_sample_f64(input, format.sample, index).map_err(pixel_error)?;
        }
        transform
            .try_eval_f64(
                colors.get(..transform.inputs()).ok_or_else(|| pixel_error("source color channels"))?,
                result.get_mut(..transform.outputs()).ok_or_else(|| pixel_error("destination color channels"))?,
            )
            .map_err(pixel_error)?;
        for (index, value) in result.get(..transform.outputs()).ok_or_else(|| pixel_error("destination channel range"))?.iter().enumerate() {
            photocraft_color::try_write_sample_f64(output, format.sample, index, *value).map_err(pixel_error)?;
        }
        let destination = transform.outputs().checked_mul(format.sample.bytes()).ok_or_else(|| pixel_error("clipboard alpha offset overflow"))?;
        if format.alpha {
            let source = transform.inputs().checked_mul(format.sample.bytes()).ok_or_else(|| pixel_error("clipboard alpha offset overflow"))?;
            let end = source.checked_add(format.sample.bytes()).ok_or_else(|| pixel_error("clipboard alpha range overflow"))?;
            let destination_end = destination.checked_add(format.sample.bytes()).ok_or_else(|| pixel_error("clipboard alpha range overflow"))?;
            output
                .get_mut(destination..destination_end)
                .ok_or_else(|| pixel_error("destination alpha range"))?
                .copy_from_slice(input.get(source..end).ok_or_else(|| pixel_error("source alpha range"))?);
        } else {
            photocraft_color::try_write_sample_f64(output, format.sample, transform.outputs(), 1.0).map_err(pixel_error)?;
        }
        Ok(())
    };
    let count = surface.tiles().count();
    let tile_pixels =
        usize::try_from(photocraft_geom::TILE_SIZE).ok().and_then(|n| n.checked_mul(n)).ok_or_else(|| pixel_error("clipboard tile size overflow"))?;
    let length = tile_pixels.checked_mul(intermediate.bytes_per_pixel()).ok_or_else(|| pixel_error("clipboard tile length overflow"))?;
    let output_bytes = (count as u64)
        .checked_mul(length as u64)
        .and_then(|n| n.checked_add((count as u64).checked_mul(256)?))
        .ok_or_else(|| pixel_error("clipboard retained allocation overflow"))?;
    let peak = output_bytes
        .checked_add(surface.resident_payload_bytes().map_err(pixel_error)?)
        .and_then(|n| n.checked_add((length as u64).checked_mul(2)?))
        .ok_or_else(|| pixel_error("clipboard resident size overflow"))?;
    if peak > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(pixel_error("clipboard conversion exceeds resident memory limit"));
    }
    let mut default = [0_u8; 136];
    let default = default.get_mut(..intermediate.bytes_per_pixel()).ok_or_else(|| pixel_error("clipboard default channels"))?;
    convert(surface.default_bytes(), default)?;
    let mut staged = Surface::try_with_default_bytes(intermediate, default).map_err(pixel_error)?;
    for (coordinate, tile) in surface.tiles() {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if tile.bytes().len() != tile_pixels.checked_mul(format.bytes_per_pixel()).ok_or_else(|| pixel_error("clipboard source tile length overflow"))? {
            return Err(pixel_error("malformed clipboard tile length"));
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(pixel_error)?;
        bytes.resize(length, 0);
        for (index, (input, output)) in
            tile.bytes().chunks_exact(format.bytes_per_pixel()).zip(bytes.chunks_exact_mut(intermediate.bytes_per_pixel())).enumerate()
        {
            if index % 1024 == 0 {
                control.check().map_err(|_| EngineError::Cancelled)?;
            }
            convert(input, output)?;
        }
        staged.try_write_interleaved_with(coordinate.rect(), &bytes, control).map_err(pixel_error)?;
    }
    staged.try_convert_with(target, control).map_err(pixel_error)
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// Paste and New from Clipboard need only a clipboard: with no document open, they make one.
fn has_clip_only(s: &Session) -> std::result::Result<(), String> {
    s.clipboard.as_ref().map(|_| ()).ok_or_else(|| "the clipboard is empty".into())
}

fn active_layer(s: &Session) -> std::result::Result<&Layer, String> {
    let d = s.active().ok_or("no document open")?;
    d.active_layer.and_then(|id| d.doc.layer(id)).ok_or_else(|| "no active layer".into())
}

/// Cut and Layer via Cut edit the pixels, so they need a pixel layer.
fn has_pixels(s: &Session) -> std::result::Result<(), String> {
    match active_layer(s)?.content {
        LayerContent::Raster(_) => Ok(()),
        LayerContent::Smart(_) => Err("the smart object is not directly editable".into()),
        _ => Err("the active layer has no pixels".into()),
    }
}

fn can_cut(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_clip::native_gray_write_candidate(s) { return Ok(()); }
    has_pixels(s)
}

/// Copy only reads: any layer that shows pixels will do, so a smart object, type or shape layer
/// copies what it shows (Photoshop).
fn has_layer_pixels(s: &Session) -> std::result::Result<(), String> {
    active_layer(s)?.surface().map(|_| ()).ok_or_else(|| "the active layer has no pixels".into())
}

fn has_clip(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    s.clipboard.as_ref().map(|_| ()).ok_or_else(|| "the clipboard is empty".into())
}

fn active_id(s: &Session) -> Result<LayerId> {
    s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into()))
}

/// The pixels of `src` inside the selection (or all of them), as a straight-alpha surface.
fn lift(src: &Surface, sel: Option<&Surface>, canvas: Rect, color: ClipColorContext) -> Result<Clip> {
    let fmt = src.format();
    if fmt.channels() == 0 || fmt.bytes_per_pixel() == 0 {
        return Err(pixel_error("clipboard source has no channels"));
    }
    src.try_validate_tile_origins().map_err(pixel_error)?;
    if let Some(mask) = sel {
        mask.try_validate_tile_origins().map_err(pixel_error)?;
    }
    let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
    let area = sel.map_or_else(|| src.content_bounds().intersect(&canvas), |mask| mask.content_bounds().intersect(&canvas));
    let mut out = Surface::new(with_alpha);
    if area.is_empty() {
        return Ok(Clip { surface: out, bounds: Rect::EMPTY, color, composition: Vec::new(), layers: None, resolution: None });
    }
    for tile in area.tiles() {
        let rect = tile.rect().intersect(&area);
        let source = src.try_read_interleaved(rect).map_err(pixel_error)?;
        let mut bytes = if fmt.alpha {
            source
        } else {
            let length =
                (source.len() / fmt.bytes_per_pixel()).checked_mul(with_alpha.bytes_per_pixel()).ok_or_else(|| pixel_error("clipboard alpha size overflow"))?;
            let mut output = Vec::new();
            output.try_reserve_exact(length).map_err(pixel_error)?;
            for pixel in source.chunks_exact(fmt.bytes_per_pixel()) {
                let start = output.len();
                output.extend_from_slice(pixel);
                let length = output.len().checked_add(fmt.sample.bytes()).ok_or_else(|| pixel_error("clipboard alpha length overflow"))?;
                output.resize(length, 0);
                photocraft_color::try_write_sample_f64(
                    output.get_mut(start..).ok_or_else(|| pixel_error("clipboard alpha pixel range"))?,
                    fmt.sample,
                    fmt.channels(),
                    1.0,
                )
                .map_err(pixel_error)?;
            }
            output
        };
        if let Some(mask) = sel {
            let values = mask.try_read_region_f64(rect).map_err(pixel_error)?;
            for (pixel, coverage) in bytes.chunks_exact_mut(with_alpha.bytes_per_pixel()).zip(values.chunks_exact(mask.channels())) {
                let coverage = *coverage.first().ok_or_else(|| pixel_error("missing mask sample"))?;
                if !coverage.is_finite() || !(0.0..=1.0).contains(&coverage) {
                    return Err(pixel_error("invalid selection coverage"));
                }
                // Boolean coverage is an encoded identity/zero-alpha operation,
                // not sample arithmetic: retain F64 NaN payloads, signed zero
                // and U32 low bits without decoding/re-encoding source alpha.
                if coverage == 1.0 {
                    continue;
                }
                let index = with_alpha.channels() - 1;
                if coverage == 0.0 {
                    let range = photocraft_color::pixel_channel_byte_range(with_alpha, 0, index, pixel.len()).map_err(pixel_error)?;
                    pixel.get_mut(range).ok_or_else(|| pixel_error("clipboard alpha pixel range"))?.fill(0);
                    continue;
                }
                if matches!(fmt.sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {
                    return Err(pixel_error("fractional selection copy for authoritative U32/F64 samples is unsupported"));
                }
                let value = photocraft_color::try_read_sample_f64(pixel, fmt.sample, index).map_err(pixel_error)?;
                photocraft_color::try_write_sample_f64(pixel, fmt.sample, index, value * coverage).map_err(pixel_error)?;
            }
        }
        out.try_write_interleaved(rect, &bytes).map_err(pixel_error)?;
    }
    out.prune();
    let bounds = out.content_bounds();
    Ok(Clip { surface: out, bounds, color, composition: Vec::new(), layers: None, resolution: None })
}

/// Merged composite of the visible document as a surface in the document's format.
fn merged_surface(doc: &Document) -> Result<Surface> {
    let fmt = doc.pixel_format();
    if doc.xcf_state.is_some() || matches!(fmt.sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {
        let renderer = photocraft_compose::gimp_render::PreparedRenderer::new(doc).map_err(pixel_error)?;
        let mut surface = Surface::new(PixelFormat::new(fmt.mode, fmt.sample, true));
        for tile in doc.bounds().tiles() {
            let rect = tile.rect().intersect(&doc.bounds());
            let buffer = renderer.render(rect, &mut |_| Ok(())).map_err(pixel_error)?;
            let mut samples = Vec::new();
            let count = buffer.px.len().checked_mul(surface.channels()).ok_or_else(|| pixel_error("merged sample count overflow"))?;
            samples.try_reserve_exact(count).map_err(pixel_error)?;
            for pixel in buffer.px {
                if fmt.mode == photocraft_doc::ColorMode::Grayscale {
                    samples.extend_from_slice(&[pixel[0], pixel[3]]);
                } else {
                    samples.extend_from_slice(&pixel);
                }
            }
            surface.try_write_region_f64(rect, &samples).map_err(pixel_error)?;
        }
        return Ok(surface);
    }
    Ok(photocraft_compose::flatten_to_surface(doc, PixelFormat::new(fmt.mode, fmt.sample, true), None)?)
}

fn prepare_copy(s: &Session, merged: bool) -> Result<Clip> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let color = ClipColorContext::from_document(&d.doc)?;
    let mut clip = if merged {
        lift(&merged_surface(&d.doc)?,d.doc.selection.as_ref(),canvas,color)?
    } else {
        let id=d.active_layer.ok_or_else(||pixel_error("no active layer"))?;
        let several=multi_copy_ids(d)?;
        let mut layers=if !several.is_empty() {Some(whole_layers(d,&several)?)} else if takes_whole(d,id)? {Some(whole_layer(d,id)?)}else {None};
        if let Some(copies)=&mut layers {copy_linked_sources(&d.doc,copies)?;}
        let mut clip=match (d.doc.layer(id).and_then(Layer::surface).filter(|_|several.is_empty()),&layers) {
            (Some(surface),_)=>lift(surface,d.doc.selection.as_ref(),canvas,color)?,
            (None,Some(copies))=>{
                let mut shown=copies.clone();
                for layer in &mut shown.layers {layer.visible=true;}
                lift(&merged_surface(&shown)?,None,canvas,color)?
            }
            _=>return Err(pixel_error("the active layer has no pixels")),
        };
        clip.layers=layers.map(std::sync::Arc::new);clip
    };
    if let Some(state) = &d.doc.xcf_state {
        clip.composition = state
            .items
            .iter()
            .filter(|item| merged || Some(item.item) == d.active_layer.map(photocraft_doc::item_identity::ItemRef::Layer))
            .filter_map(|item| item.composition.clone())
            .collect();
    }
    if clip.bounds.is_empty() {
        return Err(EngineError::Other("Could not copy: the selected area is empty".into()));
    }
    clip.resolution = Some(d.doc.resolution_dpi);
    Ok(clip)
}

fn copy(s: &mut Session, merged: bool) -> Result<Value> {
    let clip = prepare_copy(s, merged)?;
    let b = clip.bounds;
    s.clipboard = Some(clip);
    Ok(json!({"bounds": [b.x0, b.y0, b.width(), b.height()]}))
}

fn clear_selected(doc: &mut Document, id: LayerId, background: [f32; 4]) -> Result<()> {
    let sel = doc.selection.clone();
    let canvas = doc.bounds();
    let area = sel.as_ref().map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    clear_area(doc, id, area, sel.as_ref(), background)
}

/// Caller join before a document edit or Cut clipboard preparation.
pub(crate) fn preflight_clear_selected(doc: &Document, id: LayerId) -> Result<()> {
    let area = doc.selection.as_ref().map_or(doc.bounds(), |mask| mask.content_bounds().intersect(&doc.bounds()));
    preflight_clear_area(doc, id, area)
}

/// Validate the caller's actual chosen extent (Clear may extend off canvas).
pub(crate) fn preflight_clear_area(doc: &Document, id: LayerId, area: Rect) -> Result<()> {
    let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let Some(surface) = layer.surface() else { return Ok(()) };
    if matches!(surface.format().sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {
        if doc.is_background(id) {
            return Err(pixel_error("authoritative U32/F64 Background fill-clear is unsupported"));
        }
        crate::pixels::preflight_clear_surface(surface, area, doc.selection.as_ref(), &photocraft_raster::Interrupt::NONE)?;
    }
    Ok(())
}

/// Cut / Clear on a layer: makes the selected pixels transparent. The Background can't hold
/// transparency, so there the area is filled with the background colour instead (Photoshop).
pub(crate) fn clear_area(doc: &mut Document, id: LayerId, area: Rect, sel: Option<&Surface>, background: [f32; 4]) -> Result<()> {
    let bg = doc.is_background(id);
    let source = doc.layer(id).and_then(Layer::surface);
    let wide = source.is_some_and(|surface| matches!(surface.format().sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64));
    if wide && bg {
        return Err(pixel_error("authoritative U32/F64 Background fill-clear is unsupported"));
    }
    if wide && let Some(source) = source {
        crate::pixels::preflight_clear_surface(source, area, sel, &photocraft_raster::Interrupt::NONE)?;
    }
    let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
    if bg {
        crate::pixels::try_fill_surface(surf, area, background, sel, true)?;
    } else {
        crate::pixels::try_clear_surface(surf, area, sel, &photocraft_raster::Interrupt::NONE)?;
    }
    if !wide {
        surf.prune();
    }
    Ok(())
}

/// Paste as a new layer (or into the targeted mask or channel: [`paste_to_target`]). `in_place` keeps the original position; otherwise the pixels are centred
/// on `center` (the view centre from the UI) or the canvas, unless they already overlap the canvas.
fn paste(s: &mut Session, p: &Value, in_place: bool) -> Result<Value> {
    if let Some(result) = crate::channel_clip::try_paste_native_gray(s, p, in_place, None, "Paste", &photocraft_raster::Interrupt::NONE)? {
        return Ok(result);
    }
    let clip = s.clipboard.clone().ok_or(EngineError::Other("the clipboard is empty".into()))?;
    require_ordinary_clip(&clip)?;
    let Some(d) = s.active() else {
        // Nothing open to paste into: the clipboard becomes a document of its own (#368).
        return new_from_clipboard(s);
    };
    if crate::channel_cmds::target_of(p) != crate::channel_cmds::Target::Pixels {
        return paste_to_target(s, p, in_place, None, "Paste");
    }
    if let Some(channel)=crate::channel_clip::paste_color_target(s,p) {return crate::channel_clip::paste(s,p,channel,in_place,None,"Paste");}
    let canvas = d.doc.bounds();
    let fmt = d.doc.pixel_format();
    let (dx, dy) = paste_offset(&clip, canvas, p, in_place)?;
    if let Some(layers) = clip.layers.as_deref().filter(|_| crate::layer_copy_cmds::layered(d.doc.mode)) {
        return paste_layers(s, layers, dx, dy);
    }
    let moved = shifted(&clip.surface, dx, dy)?;
    let target = PixelFormat::new(fmt.mode, fmt.sample, true);
    let target_profile = crate::color_cmds::try_document_profile(&d.doc)?;
    let surf = convert_clip(&moved, &clip.color, target, &target_profile)?;
    let id = s.edit("Paste", |doc, active| {
        let mut l = Layer::raster(doc.next_layer_name("Layer"), target);
        *crate::pixels_mut(&mut l)? = surf;
        let id = doc.insert_above(*active, l);
        *active = Some(id);
        doc.selection = None;
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "offset": [dx, dy]}))
}

/// How far a paste moves the clipboard pixels: not at all in place (or when they lie on the
/// canvas and no `center` is given), else onto `center` (the view centre from the UI) or the
/// canvas centre.
pub(crate) fn paste_offset(clip: &Clip, canvas: Rect, p: &Value, in_place: bool) -> Result<(i32, i32)> {
    if in_place || (clip.bounds.intersect(&canvas) == clip.bounds && p.get("center").is_none()) {
        return Ok((0, 0));
    }
    let center = p.get("center").and_then(Value::as_array).filter(|values| values.len() >= 2).map(|values| {
        (
            values.first().and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0),
            values.get(1).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0),
        )
    });
    let (cx, cy) = center.unwrap_or(((f64::from(canvas.x0) + f64::from(canvas.x1)) / 2.0, (f64::from(canvas.y0) + f64::from(canvas.y1)) / 2.0));
    let b = clip.bounds;
    let dx = (cx - (f64::from(b.x0) + f64::from(b.x1)) / 2.0).round();
    let dy = (cy - (f64::from(b.y0) + f64::from(b.y1)) / 2.0).round();
    if !dx.is_finite() || !dy.is_finite() || dx < f64::from(i32::MIN) || dx > f64::from(i32::MAX) || dy < f64::from(i32::MIN) || dy > f64::from(i32::MAX) {
        return Err(pixel_error("paste offset exceeds supported integer coordinates"));
    }
    Ok((dx as i32, dy as i32))
}

pub(crate) fn shifted(surface: &Surface, dx: i32, dy: i32) -> Result<Surface> {
    surface.try_validate_tile_origins().map_err(pixel_error)?;
    if dx == 0 && dy == 0 {
        surface.try_validate_tile_origins().map_err(pixel_error)?;
        Ok(surface.clone())
    } else {
        photocraft_algo::wide::try_translate(surface, surface.content_bounds(), dx, dy, &photocraft_raster::Interrupt::NONE).map_err(pixel_error)
    }
}

/// Paste into the targeted layer mask, alpha channel or Quick Mask (`"target"`) instead of as a
/// new layer (Photoshop, #1035): the pasted pixels' luminosity, placed as a paste places them,
/// over what was there. Their transparency, and `limit` (Paste Into / Outside), let it show
/// through. One history step.
pub(crate) fn paste_to_target(s: &mut Session, p: &Value, in_place: bool, limit: Option<&Surface>, label: &str) -> Result<Value> {
    if let Some(result) = crate::channel_clip::try_paste_native_gray(s, p, in_place, limit, label, &photocraft_raster::Interrupt::NONE)? {
        return Ok(result);
    }
    let clip = s.clipboard.clone().ok_or(EngineError::Other("the clipboard is empty".into()))?;
    require_ordinary_clip(&clip)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let layer = d.active_layer;
    let (dx, dy) = paste_offset(&clip, canvas, p, in_place)?;
    let moved = shifted(&clip.surface, dx, dy)?;
    let document_profile = crate::color_cmds::try_document_profile(&d.doc)?;
    let gray_profile = if d.doc.mode == photocraft_doc::ColorMode::Grayscale {
        document_profile
    } else {
        std::sync::Arc::new(photocraft_cms::Builtin::SGray.profile().clone())
    };
    let target = PixelFormat::new(photocraft_doc::ColorMode::Grayscale, moved.format().sample, true);
    let src = convert_clip(&moved, &clip.color, target, &gray_profile)?;
    let area = src.content_bounds().intersect(&canvas);
    s.edit(label, |doc, _| {
        let (surf, _) = crate::channel_cmds::target_surface(doc, layer, p)?;
        if surf.format().mode != photocraft_doc::ColorMode::Grayscale || surf.channels() != 1 {
            return Err(pixel_error("luminosity target must be a single coverage channel"));
        }
        let mut staged = surf.clone();
        for tile in area.tiles() {
            let rect = tile.rect().intersect(&area);
            let samples = src.try_read_region_f64(rect).map_err(pixel_error)?;
            let selection = limit.map(|mask| mask.try_read_region_f64(rect)).transpose().map_err(pixel_error)?;
            let mut raw = staged.try_read_interleaved(rect).map_err(pixel_error)?;
            let channels = limit.map_or(1, Surface::channels);
            for (index, (pixel, destination)) in samples.as_chunks::<2>().0.iter().zip(raw.chunks_exact_mut(surf.format().bytes_per_pixel())).enumerate() {
                let selection = if let Some(values) = &selection {
                    *values
                        .get(index.checked_mul(channels).ok_or_else(|| pixel_error("selection offset overflow"))?)
                        .ok_or_else(|| pixel_error("missing selection coverage"))?
                } else {
                    1.0
                };
                let alpha = pixel[1];
                if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) || !selection.is_finite() || !(0.0..=1.0).contains(&selection) {
                    return Err(pixel_error("invalid paste coverage"));
                }
                let coverage = alpha * selection;
                if coverage == 0.0 {
                    continue;
                }
                let previous = photocraft_color::try_read_sample_f64(destination, surf.format().sample, 0).map_err(pixel_error)?;
                let luminosity = pixel[0].clamp(0.0, 1.0);
                let value = previous * (1.0 - coverage) + luminosity * coverage;
                if !value.is_finite() {
                    return Err(pixel_error("nonfinite pasted luminosity"));
                }
                photocraft_color::try_write_sample_f64(destination, surf.format().sample, 0, value).map_err(pixel_error)?;
            }
            staged.try_write_interleaved(rect, &raw).map_err(pixel_error)?;
        }
        staged.prune();
        *surf = staged;
        doc.selection = None;
        Ok(())
    })?;
    Ok(json!({"target": p.get("target"), "offset": [dx, dy]}))
}

/// A new document the size of the clipboard image, holding it as its one layer, in the pixel
/// format it was copied in (#368).
fn new_from_clipboard(s: &mut Session) -> Result<Value> {
    let clip = s.clipboard.clone().ok_or(EngineError::Other("the clipboard is empty".into()))?;
    require_ordinary_clip(&clip)?;
    let b = clip.bounds;
    let (w, h) = (b.width(), b.height());
    if w == 0 || h == 0 {
        return Err(EngineError::Other("the clipboard image has no size".into()));
    }
    let (Some(dx), Some(dy)) = (b.x0.checked_neg(), b.y0.checked_neg()) else {
        return Err(EngineError::Other("the clipboard origin is outside the coordinate range for a new document".into()));
    };
    let fmt = clip.surface.format();
    let target = PixelFormat::new(fmt.mode, fmt.sample, true);
    let moved = if dx == 0 && dy == 0 { clip.surface } else { photocraft_algo::resample::translate_surface(&clip.surface, dx, dy) };
    let mut doc = Document::new("Untitled", photocraft_geom::Size::new(w, h), fmt.mode, fmt.sample);
    clip.color.apply(&mut doc);
    let mut l = Layer::raster(doc.next_layer_name("Layer"), target);
    *crate::pixels_mut(&mut l)? =
        if moved.format() == target { moved } else { moved.try_convert(target).map_err(|e| crate::EngineError::Other(format!("pixel conversion: {e}")))? };
    doc.layers.push(l);
    let i = s.add_document(doc, None)?;
    Ok(json!({"document": i, "width": w, "height": h}))
}

fn layer_via(s: &mut Session, cut: bool) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = active_id(s)?;
    let has_sel = d.doc.selection.is_some();
    if !has_sel && !cut {
        // No selection: Layer via Copy duplicates the whole layer (Photoshop).
        let nid = s.edit("Layer Via Copy", |doc, active| {
            let src = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            let mut dup = src.duplicate();
            dup.name = doc.copy_name(&src.name);
            dup.locks = Default::default();
            let nid = doc.insert_above(Some(id), dup);
            *active = Some(nid);
            Ok(nid)
        })?;
        return Ok(json!({"layer": nid.0}));
    }
    let surf = d.doc.layer(id).and_then(Layer::surface).ok_or(EngineError::Other("the active layer has no pixels".into()))?;
    let clip = lift(surf, d.doc.selection.as_ref(), d.doc.bounds(), ClipColorContext::from_document(&d.doc)?)?;
    if clip.bounds.is_empty() {
        return Err(EngineError::Other("Could not complete the command: the selected area is empty".into()));
    }
    let bg = s.tools.background;
    let label = if cut { "Layer Via Cut" } else { "Layer Via Copy" };
    let nid = s.edit(label, |doc, active| {
        if cut {
            clear_selected(doc, id, bg)?;
        }
        let mut l = Layer::raster(doc.next_layer_name("Layer"), clip.surface.format());
        *crate::pixels_mut(&mut l)? = clip.surface;
        let nid = doc.insert_above(Some(id), l);
        *active = Some(nid);
        Ok(nid)
    })?;
    Ok(json!({"layer": nid.0}))
}

fn merge_visible(s: &mut Session) -> Result<Value> {
    s.edit("Merge Visible", |doc, active| {
        let visible: Vec<usize> = doc.layers.iter().enumerate().filter(|(_, l)| l.visible).map(|(i, _)| i).collect();
        if visible.len() < 2 && !visible.iter().any(|i| doc.layers[*i].is_group()) {
            return Err(EngineError::Other("there are not enough visible layers to merge".into()));
        }
        let lowest = visible[0];
        let base = doc.layers[lowest].clone();
        let is_background = base.name == "Background" && base.locks.transparency;
        // Composite of visible layers only (hidden ones stay where they are).
        let mut solo = doc.clone();
        solo.layers.retain(|l| l.visible);
        let buf = photocraft_compose::flatten(&solo)?;
        let buf = if is_background { buf.over_background([1.0, 1.0, 1.0]) } else { buf };
        let fmt = doc.pixel_format();
        let fmt = if is_background { fmt } else { PixelFormat::new(fmt.mode, fmt.sample, true) };
        let data: Vec<f32> = buf.px.iter().flat_map(|p| photocraft_raster::from_rgba(&fmt, *p)).collect();
        let mut merged = Layer::raster(base.name.clone(), fmt);
        merged.locks = base.locks;
        let surf = crate::pixels_mut(&mut merged)?;
        surf.write_region(doc.bounds(), &data);
        surf.prune();
        let mid = merged.id;
        let mut out = Vec::with_capacity(doc.layers.len());
        for (i, l) in doc.layers.drain(..).enumerate() {
            if i == lowest {
                out.push(merged.clone());
            } else if !l.visible {
                out.push(l);
            }
        }
        doc.layers = out;
        *active = Some(mid);
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Histogram of a pixel layer (8-bit bins per channel: R, G, B) inside the selection.
fn histograms(surf: &Surface, area: Rect) -> [[u64; 256]; 3] {
    let mut h = [[0u64; 256]; 3];
    let step = ((area.width() as u64 * area.height() as u64) / 4_000_000).max(1) as usize; // sample big images
    let mut row = vec![[0u8; 4]; area.width() as usize];
    for y in (area.y0..area.y1).step_by(step.isqrt().max(1)) {
        surf.read_rgba8_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
        for p in row.iter().step_by(step.isqrt().max(1)) {
            if p[3] == 0 {
                continue;
            }
            for c in 0..3 {
                h[c][p[c] as usize] += 1;
            }
        }
    }
    h
}

/// Black and white points with `clip` (fraction) clipped at each end.
fn clip_points(h: &[u64; 256], clip: f64) -> (f32, f32) {
    let total: u64 = h.iter().sum();
    if total == 0 {
        return (0.0, 1.0);
    }
    let lim = (total as f64 * clip) as u64;
    let (mut acc, mut lo) = (0u64, 0usize);
    while lo < 254 && acc + h[lo] <= lim {
        acc += h[lo];
        lo += 1;
    }
    let (mut acc, mut hi) = (0u64, 255usize);
    while hi > lo + 1 && acc + h[hi] <= lim {
        acc += h[hi];
        hi -= 1;
    }
    (lo as f32 / 255.0, hi as f32 / 255.0)
}

fn median(h: &[u64; 256]) -> f32 {
    let total: u64 = h.iter().sum();
    let mut acc = 0;
    for (i, v) in h.iter().enumerate() {
        acc += v;
        if acc * 2 >= total {
            return i as f32 / 255.0;
        }
    }
    0.5
}

/// Image › Auto Tone / Auto Contrast / Auto Color (0.1% clipping, Photoshop's defaults).
fn auto_adjust(s: &mut Session, kind: &str) -> Result<Value> {
    let id = active_id(s)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let surf = d.doc.layer(id).and_then(Layer::surface).ok_or(EngineError::Other("the active layer has no pixels".into()))?;
    let area = d.doc.selection.as_ref().map_or(d.doc.bounds(), |m| m.content_bounds()).intersect(&d.doc.bounds());
    let h = histograms(surf, area);
    let clip = 0.001;
    let mut per = [LevelsChannel::default(), LevelsChannel::default(), LevelsChannel::default()];
    match kind {
        "contrast" => {
            // Same points for all channels (preserves colour), from the combined histogram.
            let mut all = [0u64; 256];
            for c in &h {
                for (a, v) in all.iter_mut().zip(c) {
                    *a += v;
                }
            }
            let (lo, hi) = clip_points(&all, clip);
            for p in &mut per {
                (p.in_black, p.in_white) = (lo, hi);
            }
        }
        _ => {
            for (c, p) in per.iter_mut().enumerate() {
                (p.in_black, p.in_white) = clip_points(&h[c], clip);
            }
            if kind == "color" {
                // Neutralise midtones: move each channel's median onto the average median.
                let meds: Vec<f32> = (0..3).map(|c| median(&h[c])).collect();
                let target = meds.iter().sum::<f32>() / 3.0;
                for (c, p) in per.iter_mut().enumerate() {
                    let t = ((meds[c] - p.in_black) / (p.in_white - p.in_black).max(1e-3)).clamp(0.02, 0.98);
                    let want = ((target - p.in_black) / (p.in_white - p.in_black).max(1e-3)).clamp(0.02, 0.98);
                    // out = t^(1/g) = want  →  g = ln t / ln want
                    p.gamma = (t.ln() / want.ln()).clamp(0.1, 9.99);
                }
            }
        }
    }
    let adj = Adjustment::Levels { master: LevelsChannel::default(), per_channel: per, space: Default::default(), black: LevelsChannel::default() };
    let label = match kind {
        "contrast" => "Auto Contrast",
        "color" => "Auto Color",
        _ => "Auto Tone",
    };
    s.edit(label, |doc, _| {
        let sel = doc.selection.clone();
        let mode = doc.mode;
        let transfer = photocraft_compose::adjust::Transfer::for_doc(doc);
        let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
        crate::pixels::adjust_surface_with_transfer(surf, &adj, sel.as_ref(), mode, transfer)?;
        Ok(())
    })?;
    Ok(Value::Null)
}

fn toggle_last_state(s: &mut Session) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let redo = st.history.can_redo();
    Ok(json!(if redo { s.redo() } else { s.undo() }))
}

/// Edit › Transform › Again: replay the last `edit.transform` on the active layer.
fn transform_again(s: &mut Session) -> Result<Value> {
    let p = transform_again_params(s)?;
    s.execute("edit.transform", p)
}

/// Transform Again on a copy (⌥⇧⌘T, no menu item): duplicates the active layer and repeats the
/// last transform on the copy, as one history step, so pressing it again steps and repeats
/// (#352). The transform is worked out before anything changes, so a refusal leaves no copy.
fn transform_again_copy(s: &mut Session) -> Result<Value> {
    let p = transform_again_params(s)?;
    let id = active_id(s)?;
    s.execute("layer.duplicate", json!({"layer": id.0}))?;
    match s.execute("edit.transform", p) {
        Ok(r) => {
            let st = s.active_mut().ok_or(EngineError::NoDocument)?;
            st.history.purge_last(&crate::read_control_none())?;
            st.history.set_current_label("Transform Again", &crate::read_control_none())?;
            Ok(r)
        }
        Err(e) => {
            // The copy (e.g. of a layer whose position is locked) couldn't be transformed: take
            // it back, leaving nothing to redo.
            s.undo();
            if let Some(st) = s.active_mut() {
                st.history.clear_redo(&crate::read_control_none())?;
            }
            Err(e)
        }
    }
}

/// The `edit.transform` params that repeat the last transform on the active layer.
fn transform_again_params(s: &Session) -> Result<Value> {
    let (_, last) =
        s.journal.iter().rev().find(|(id, _)| id == "edit.transform").cloned().ok_or(EngineError::Other("there is no transform to repeat".into()))?;
    let (rect, quad) = (last.get("rect").cloned(), last.get("quad").cloned());
    let mut p = json!({"interpolation": last.get("interpolation").cloned().unwrap_or(json!("bicubic"))});
    match (rect, quad, last.get("matrix").cloned()) {
        (Some(r), Some(q), _) => {
            let v = |x: &Value| -> Vec<f64> { x.as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default() };
            let r = v(&r);
            let q: Vec<Vec<f64>> = q.as_array().map(|a| a.iter().map(v).collect()).unwrap_or_default();
            if r.len() != 4 || q.len() != 4 || q.iter().any(|row| row.len() < 2) {
                return Err(EngineError::Other("the last transform can't be repeated".into()));
            }
            let h = photocraft_algo::transform::Homography::rect_to_quad(
                [r[0], r[1], r[2], r[3]],
                [[q[0][0], q[0][1]], [q[1][0], q[1][1]], [q[2][0], q[2][1]], [q[3][0], q[3][1]]],
            )
            .ok_or(EngineError::Other("the last transform can't be repeated".into()))?;
            let d = s.active().ok_or(EngineError::NoDocument)?;
            let l = d.doc.layer(active_id(s)?).ok_or(EngineError::Other("no active layer".into()))?;
            let b = crate::transform_cmds::transform_bounds(&d.doc, l);
            if b.is_empty() {
                return Err(EngineError::Other("nothing to transform".into()));
            }
            let nr = [b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64];
            let corners = [(nr[0], nr[1]), (nr[2], nr[1]), (nr[2], nr[3]), (nr[0], nr[3])].map(|(x, y)| {
                let (a, b) = h.apply(x, y);
                [a, b]
            });
            p["rect"] = json!(nr);
            p["quad"] = json!(corners);
        }
        (_, _, Some(m)) => p["matrix"] = m,
        _ => return Err(EngineError::Other("the last transform can't be repeated".into())),
    }
    Ok(p)
}

/// Guides further than this from the canvas origin are refused: far past any document (300000 px)
/// yet well within f32 precision.
const MAX_GUIDE_PX: f64 = 1_000_000.0;

/// A guide `position` in document pixels: a number of pixels, or text with a unit as in Photoshop's
/// New Guide dialog ("50%", "2.5 cm", "12 pt"). Percent is of the document's width for a vertical
/// guide and its height for a horizontal one; points and picas follow Units & Rulers › Point Size.
/// `None` when there is no `position`.
fn guide_position(s: &Session, p: &Value, vertical: bool) -> Result<Option<f64>> {
    let Some(v) = p.get("position") else { return Ok(None) };
    let px = match (v.as_f64(), v.as_str()) {
        (Some(n), _) => n,
        (None, Some(text)) => {
            let (n, unit) = crate::prefs::Unit::parse_length(text).ok_or_else(|| {
                EngineError::Other(format!("`position` {text:?} is not a length: give a number of pixels, or a number with px, in, cm, mm, pt, pc or %"))
            })?;
            let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
            let extent = if vertical { doc.size.width } else { doc.size.height };
            unit.unwrap_or(crate::prefs::Unit::Pixels).to_px(n, doc.resolution_dpi as f64, extent as f64, s.prefs().units_and_rulers.point_size.per_inch())
        }
        _ => return Err(EngineError::Other("`position` must be a number of pixels or text with a unit".into())),
    };
    if !px.is_finite() || px.abs() > MAX_GUIDE_PX {
        return Err(EngineError::Other(format!("`position` is out of range (at most {MAX_GUIDE_PX} px from the canvas origin)")));
    }
    Ok(Some(px))
}

/// Photoshop's guide colours, in its order, by the names `view.newGuide` takes: New Guide's and
/// New Guide Layout's Color lists. Cyan (the default guide colour in Preferences, so a guide
/// dragged from a ruler reads as Cyan), Magenta and Yellow are the values New Guide Layout first
/// shipped with; the rest approximate Photoshop's on screen, rounded to web-safe steps.
pub const GUIDE_COLORS: [(&str, [u8; 3]); 9] = [
    ("lightBlue", [0x33, 0x66, 0xff]),
    ("lightRed", [0xff, 0x33, 0x33]),
    ("green", [0x33, 0xff, 0x33]),
    ("mediumBlue", [0x33, 0x33, 0xff]),
    ("yellow", [0xff, 0xcc, 0x00]),
    ("magenta", [0xff, 0x00, 0xff]),
    ("cyan", [0x4a, 0xff, 0xff]),
    ("lightGray", [0xe7, 0xe7, 0xe7]),
    ("black", [0x00, 0x00, 0x00]),
];

/// A guide `color`: one of [`GUIDE_COLORS`]' names or `"#rrggbb"`. `None` when there is no `color`
/// (the guide uses Preferences › Guides, Grid & Slices › Guide colour).
fn guide_color(p: &Value) -> Result<Option<[u8; 3]>> {
    let Some(v) = p.get("color") else { return Ok(None) };
    let text = v.as_str().ok_or(EngineError::Other("`color` must be a colour name or \"#rrggbb\"".into()))?;
    GUIDE_COLORS.iter().find(|(name, ..)| name.eq_ignore_ascii_case(text)).map(|(.., rgb)| *rgb).or_else(|| crate::prefs::parse_hex(text)).map(Some).ok_or_else(
        || EngineError::Other(format!("`color` {text:?} is not a guide colour: use \"#rrggbb\" or one of {}", GUIDE_COLORS.map(|c| c.0).join(", "))),
    )
}

/// Edit Selected Guides (the guide context menu): change one guide's orientation, position and
/// colour in one history step. A guide that keeps its orientation keeps its index; one that turns
/// goes to the end of the other list. Without `color` it keeps its own colour, or none.
fn edit_guide(s: &mut Session, p: &Value) -> Result<Value> {
    let orientation = |key: &str| match p.get(key).map(|v| v.as_str()) {
        None => Ok(None),
        Some(Some("horizontal")) => Ok(Some(false)),
        Some(Some("vertical")) => Ok(Some(true)),
        Some(_) => Err(EngineError::Other(format!("`{key}` must be \"horizontal\" or \"vertical\""))),
    };
    let from = orientation("orientation")?.unwrap_or(false);
    let to = orientation("newOrientation")?.unwrap_or(from);
    let index = p.get("index").and_then(Value::as_u64).ok_or(EngineError::Other("missing `index`".into()))?;
    let pos = guide_position(s, p, to)?;
    let color = guide_color(p)?;
    s.edit("Edit Guide", |doc, _| {
        let guides = &mut doc.guides;
        let list = if from { &guides.vertical } else { &guides.horizontal };
        let i = usize::try_from(index).ok().filter(|i| *i < list.len()).ok_or(EngineError::Other("no such guide".into()))?;
        let old = list[i];
        let at = pos.unwrap_or(old);
        let rgb = color.or_else(|| guides.color(from, old));
        if from == to {
            let list = if from { &mut guides.vertical } else { &mut guides.horizontal };
            list[i] = at;
        } else {
            (if from { &mut guides.vertical } else { &mut guides.horizontal }).remove(i);
            (if to { &mut guides.vertical } else { &mut guides.horizontal }).push(at);
        }
        guides.prune_colors();
        if rgb.is_some() {
            guides.set_color(to, at, rgb);
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// View › New Guide / guide moves (undoable, like Photoshop's "New Guide"/"Move Guide" states).
mod guide_controlled;

pub(crate) fn guide_dispatch_preflight(s: &Session, p: &Value, op: &str, background: bool) -> Result<()> {
    guide_controlled::dispatch_preflight(s, p, op, background)
}

fn guide_cmd(s: &mut Session, p: &Value, op: &str) -> Result<Value> {
    guide_controlled::run(s, p, op)
}

/// Farthest the ruler zero point may sit from the image, in document pixels.
const MAX_RULER_ORIGIN: f64 = 1.0e7;

/// Move the rulers' zero point (`view.setRulerOrigin`), or put it back at the image's top-left
/// (`view.resetRulerOrigin`, `p` = None). Undoable like the guides.
fn ruler_origin_cmd(s: &mut Session, p: Option<&Value>) -> Result<Value> {
    let origin = match p {
        None => [0.0, 0.0],
        Some(p) => {
            let axis = |k: &str| {
                p.get(k)
                    .and_then(Value::as_f64)
                    .filter(|v| v.is_finite() && v.abs() <= MAX_RULER_ORIGIN)
                    .ok_or_else(|| EngineError::Other(format!("`{k}` must be a number of pixels within ±{MAX_RULER_ORIGIN}")))
            };
            [axis("x")?, axis("y")?]
        }
    };
    let label = if p.is_some() { "Set Ruler Origin" } else { "Reset Ruler Origin" };
    s.edit(label, |doc, _| {
        doc.ruler_origin = origin;
        Ok(())
    })?;
    Ok(json!({"x": origin[0], "y": origin[1]}))
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $sc:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: $sc, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    vec![
        spec!("edit.cut", "Cut", &["Edit"], Some("Cmd+X"), "{}", can_cut, |s, p| {
            if let Some(result) = crate::channel_clip::try_cut_native_gray(s, p, &photocraft_raster::Interrupt::NONE)? {
                return Ok(result);
            }
            // Refuse a locked layer before copying, so a refused Cut leaves the clipboard alone.
            let id = active_id(s)?;
            crate::commands::check_pixels_unlocked(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
            if let Some(channel)=crate::channel_clip::paste_color_target(s,p) {return crate::channel_clip::cut(s,id,channel);}
            preflight_clear_selected(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
            let clip = prepare_copy(s, false)?;
            let b = clip.bounds;
            let r = json!({"bounds": [b.x0, b.y0, b.width(), b.height()]});
            let bg = s.tools.background;
            s.edit("Cut Pixels", |doc, _| clear_selected(doc, id, bg))?;
            s.clipboard = Some(clip);
            Ok(r)
        }),
        spec!("edit.copy", "Copy", &["Edit"], Some("Cmd+C"), "{}", can_copy, |s, p| {
            if let Some(result) = crate::channel_clip::try_copy_native_gray(s, p, &photocraft_raster::Interrupt::NONE)? {
                return Ok(result);
            }
            if crate::commands::is_mask_target(p) {return copy_mask(s);}
            match crate::channel_clip::copy(s)? {Some(result)=>Ok(result),None=>copy(s,false)}
        }),
        spec!("edit.copyMerged", "Copy Merged", &["Edit"], Some("Cmd+Shift+C"), "{}", has_doc, |s, _| copy(s, true)),
        spec!(
            "edit.paste",
            "Paste",
            &["Edit"],
            Some("Cmd+V"),
            concat!(
                r#"{"center":[x,y]? (view centre; default keeps the position when it overlaps the canvas),"#,
                paste_target!(),
                "} (with no document open: a new document from the clipboard)"
            ),
            has_clip_only,
            |s, p| paste(s, p, false)
        ),
        spec!(
            "file.newFromClipboard",
            "New from Clipboard",
            &["File"],
            None,
            "{} (a new document the size of the clipboard image, holding it as one layer)",
            has_clip_only,
            |s, _| new_from_clipboard(s)
        ),
        spec!(
            "edit.pasteSpecial.pasteInPlace",
            "Paste in Place",
            &["Edit", "Paste Special"],
            Some("Cmd+Shift+V"),
            concat!("{", paste_target!(), "}"),
            has_clip,
            |s, p| paste(s, p, true)
        ),
        spec!("layer.new.layerViaCopy", "Layer via Copy", &["Layer", "New"], Some("Cmd+J"), "{}", has_doc, |s, _| layer_via(s, false)),
        spec!("layer.new.layerViaCut", "Layer via Cut", &["Layer", "New"], Some("Cmd+Shift+J"), "{}", has_pixels, |s, _| layer_via(s, true)),
        spec!("layer.mergeVisible", "Merge Visible", &["Layer"], Some("Cmd+Shift+E"), "{}", has_doc, |s, _| merge_visible(s)),
        spec!("image.autoTone", "Auto Tone", &["Image"], Some("Cmd+Shift+L"), "{}", has_pixels, |s, _| auto_adjust(s, "tone")),
        spec!("image.autoContrast", "Auto Contrast", &["Image"], Some("Cmd+Alt+Shift+L"), "{}", has_pixels, |s, _| auto_adjust(s, "contrast")),
        spec!("image.autoColor", "Auto Color", &["Image"], Some("Cmd+Shift+B"), "{}", has_pixels, |s, _| auto_adjust(s, "color")),
        spec!("edit.toggleLastState", "Toggle Last State", &["Edit"], Some("Cmd+Alt+Z"), "{}", has_doc, |s, _| toggle_last_state(s)),
        spec!("edit.transform.again", "Again", &["Edit", "Transform"], Some("Cmd+Shift+T"), "{}", has_doc, |s, _| transform_again(s)),
        spec!(
            "edit.transform.againCopy",
            "Transform Again on a Copy",
            &[],
            Some("Cmd+Alt+Shift+T"),
            "{} (duplicates the active layer and repeats the last transform on the copy: step and repeat)",
            has_doc,
            |s, _| transform_again_copy(s)
        ),
        spec!(
            "view.newGuide",
            "New Guide…",
            &["View"],
            None,
            r##"{"orientation":"horizontal|vertical","position":finite number|unit-bearing string (px/in/cm/mm/pt/pc/%),"unit":"px|in|mm|pt|pc|document"="px","documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?} (vertical uses X DPI; horizontal uses Y DPI; native f64 position storage)"##,
            has_doc,
            |s, p| guide_cmd(s, p, "new")
        ),
        spec!(
            "view.moveGuide",
            "Move Guide",
            &[],
            None,
            r##"{"orientation":"horizontal|vertical","index":n,"position":finite number|unit-bearing string (px/in/cm/mm/pt/pc/%),"unit":"px|in|mm|pt|pc|document"="px","documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?} (per-axis DPI; native f64 position storage)"##,
            has_doc,
            |s, p| guide_cmd(s, p, "move")
        ),
        spec!(
            "view.editGuide",
            "Edit Guide",
            &[],
            None,
            r##"{"orientation":"horizontal|vertical","index":n,"newOrientation":"horizontal|vertical"?,"position":px or "<number><unit>"?,"color":name or "#rrggbb"?} (Edit Selected Guides: units and colours as for view.newGuide; % is of the new orientation's extent; whatever is left out stays as it was)"##,
            has_doc,
            edit_guide
        ),
        spec!("view.deleteGuide", "Delete Guide", &[], None, r##"{"orientation":"horizontal|vertical","index":n}"##, has_doc, |s, p| guide_cmd(s, p, "delete")),
        spec!("view.clearGuides", "Clear Guides", &["View"], None, "{}", has_doc, |s, p| guide_cmd(s, p, "clear")),
        spec!(
            "view.setRulerOrigin",
            "Set Ruler Origin",
            &[],
            None,
            r##"{"x":px,"y":px} (the rulers' zero point in document pixels; the rulers and Info panel measure from it)"##,
            has_doc,
            |s, p| ruler_origin_cmd(s, Some(p))
        ),
        spec!("view.resetRulerOrigin", "Reset Ruler Origin", &[], None, "{} (zero point back at the image's top-left)", has_doc, |s, _| ruler_origin_cmd(
            s, None
        )),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::default();
        s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 50, 30), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s
    }

    #[test]
    fn new_clipboard_document_rejects_an_unrepresentable_origin() {
        let mut s = Session::new();
        let mut surface = Surface::new(PixelFormat::RGBA8);
        surface.write_pixel(i32::MIN, 0, &[1.0, 0.0, 0.0, 1.0]);
        s.clipboard = Some(Clip { surface, bounds: Rect::from_xywh(i32::MIN, 0, 1, 1), layers: None, resolution: None });
        let result = new_from_clipboard(&mut s);
        assert!(result.unwrap_err().to_string().contains("clipboard origin"));
        assert!(s.execute("file.newFromClipboard", json!({})).is_err());
        assert!(s.documents().is_empty());
        assert_eq!(s.clipboard.as_ref().unwrap().surface.rgba(i32::MIN, 0), [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn injected_clear_buffer_failure_leaves_the_document_unchanged() {
        let mut s = session();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let before = st.doc.layer(id).unwrap().surface().unwrap().clone();
        let revision = st.revision;

        crate::allocation::fail_next_for_test();
        let result = s.execute("edit.clear", json!({}));

        assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for erasing pixels")));
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(id).unwrap().surface(), Some(&before));
        assert_eq!(st.revision, revision);
    }

    #[test]
    fn injected_copy_buffer_failure_does_not_change_document_or_clipboard() {
        let mut s = session();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let before = st.doc.layer(id).unwrap().surface().unwrap().clone();
        let revision = st.revision;

        crate::allocation::fail_next_for_test();
        let result = s.execute("edit.copy", json!({}));

        assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for copying pixels")));
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(id).unwrap().surface(), Some(&before));
        assert_eq!(st.revision, revision);
        assert!(s.clipboard.is_none());
    }

    #[test]
    fn injected_fill_result_reservation_failure_keeps_pixels_and_history() {
        let mut s = session();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let before = st.doc.layer(id).unwrap().surface().unwrap().clone();
        let revision = st.revision;
        let history_len = st.history.past_len();

        crate::allocation::fail_next_for_test();
        let result = s.execute("edit.fill", json!({"color": "#00ff00"}));

        assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for blending pixels")));
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(id).unwrap().surface(), Some(&before));
        assert_eq!(st.revision, revision);
        assert_eq!(st.history.past_len(), history_len);
    }

    #[test]
    fn injected_paste_failure_does_not_change_document_or_clipboard() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 8, "height": 8})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        let before_clip = s.clipboard.clone();
        let st = s.active().unwrap();
        let before_doc = (*st.doc).clone();
        let revision = st.revision;
        let history_len = st.history.past_len();

        crate::allocation::fail_next_for_test();
        let result = s.execute("edit.paste", json!({}));

        assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for pasting pixels")));
        let st = s.active().unwrap();
        assert_eq!(*st.doc, before_doc);
        assert_eq!(st.revision, revision);
        assert_eq!(st.history.past_len(), history_len);
        let after_clip = s.clipboard.as_ref().expect("clipboard remains available");
        let before_clip = before_clip.as_ref().expect("clipboard was populated");
        assert_eq!(after_clip.bounds, before_clip.bounds);
        assert_eq!(after_clip.surface, before_clip.surface);
    }

    #[test]
    fn injected_transform_failure_keeps_pixels_and_history() {
        let mut s = session();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let before = st.doc.layer(id).unwrap().surface().unwrap().clone();
        let revision = st.revision;
        let history_len = st.history.past_len();

        crate::allocation::fail_next_for_test();
        let result = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 20, 30]}));

        assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for transforming pixels")));
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(id).unwrap().surface(), Some(&before));
        assert_eq!(st.revision, revision);
        assert_eq!(st.history.past_len(), history_len);
    }

    fn active_bounds(s: &Session) -> Rect {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds()
    }

    /// #368: the clipboard as a document of its own, from New from Clipboard or a Paste with
    /// nothing open, at every depth.
    fn clipboard_float_document(mode: photocraft_doc::ColorMode, profile: photocraft_cms::Builtin, samples: &[f64]) -> Document {
        let mut doc = Document::new("Own clipboard", photocraft_geom::Size::new(2, 2), mode, photocraft_color::SampleType::F64);
        doc.icc_profile = Some(profile.profile().to_bytes());
        doc.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
        let mut layer = Layer::raster("Literal pixels", doc.pixel_format());
        layer.surface_mut().unwrap().try_write_region_f64(Rect::new(0, 0, 1, 1), samples).unwrap();
        doc.layers.push(layer);
        doc
    }
    #[test]
    fn clipboard_retains_assigned_profile_and_converts_crossmode_and_same_mode() {
        use photocraft_cms::{Builtin, Intent, Transform};
        use photocraft_doc::ColorMode;
        let mut s = Session::default();
        let samples = [0.5 + 2.0_f64.powi(-40), 0.2, 0.7, 1.0];
        s.add_document(clipboard_float_document(ColorMode::Rgb, Builtin::DisplayP3, &samples), None).unwrap();
        copy(&mut s, false).unwrap();
        let context = s.clipboard.as_ref().unwrap().color.clone();
        let raw = s.clipboard.as_ref().unwrap().surface.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap();
        assert_eq!(raw, samples.iter().flat_map(|v| v.to_ne_bytes()).collect::<Vec<_>>());
        for (mode, destination) in [(ColorMode::Grayscale, Builtin::GrayGamma22), (ColorMode::Rgb, Builtin::AdobeRgbCompat)] {
            let doc = clipboard_float_document(mode, destination, if mode == ColorMode::Grayscale { &[0.0, 1.0] } else { &[0.0, 0.0, 0.0, 1.0] });
            let target = crate::color_cmds::try_document_profile(&doc).unwrap();
            let source = context.profile(ColorMode::Rgb).unwrap();
            let transform = Transform::new(&source, &target, Intent::RelativeColorimetric, false).unwrap();
            let mut expected = [0.0; 3];
            transform.try_eval_f64(&samples[..3], &mut expected[..mode.color_channels()]).unwrap();
            s.add_document(doc, None).unwrap();
            paste(&mut s, &json!({}), true).unwrap();
            let st = s.active().unwrap();
            let surface = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
            let actual = surface.try_read_region_f64(Rect::new(0, 0, 1, 1)).unwrap();
            for (a, b) in actual[..mode.color_channels()].iter().zip(expected) {
                assert!((*a - b).abs() < 1e-12);
            }
            assert_eq!(actual.last(), Some(&1.0));
        }
        let before = s.active().unwrap().doc.as_ref().clone();
        let history = s.active().unwrap().history.past_len();
        s.clipboard.as_mut().unwrap().color.icc_profile = Some(std::sync::Arc::new(vec![0; 128]));
        assert!(paste(&mut s, &json!({}), true).is_err());
        assert_eq!(*s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    #[test]
    #[ignore = "24MP release clipboard benchmark"]
    fn clipboard_checked_24mp() {
        let area = Rect::new(0, 0, 6000, 4000);
        let mut doc = Document::new("Own24MP", photocraft_geom::Size::new(6000, 4000), photocraft_doc::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let raw: Vec<_> = [128_u8, 64, 32, 255].repeat(24_000_000);
        let mut layer = Layer::raster("Own uniform pixels", doc.pixel_format());
        *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), area, &raw).unwrap();
        doc.layers.push(layer);
        drop(raw);
        let mut s = Session::new();
        s.add_document(doc, None).unwrap();
        let start = std::time::Instant::now();
        copy(&mut s, false).unwrap();
        println!("24MP checked exact-native copy {:?}", start.elapsed());
        s.add_document(
            Document::new("Gray target", photocraft_geom::Size::new(6000, 4000), photocraft_doc::ColorMode::Grayscale, photocraft_color::SampleType::U8),
            None,
        )
        .unwrap();
        let start = std::time::Instant::now();
        paste(&mut s, &json!({}), true).unwrap();
        println!("24MP checked RGB-to-Gray CMS paste {:?}", start.elapsed());
    }
    #[test]
    fn clipboard_checked_conversion_rejects_nonfinite_narrowing_and_cancels_atomically() {
        use photocraft_cms::Builtin;
        use photocraft_doc::{ColorMode, SampleType};
        let area = Rect::new(0, 0, 1, 1);
        let doc = Document::new("Float32 source", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
        let color = ClipColorContext::from_document(&doc).unwrap();
        for first in [f32::MAX, f32::NAN] {
            let raw: Vec<_> = [first, 0.2, 0.7, 1.0].iter().flat_map(|v| v.to_ne_bytes()).collect();
            let source = Surface::try_from_interleaved(PixelFormat::RGBA32F, area, &raw).unwrap();
            let before = source.clone();
            assert!(convert_clip(&source, &color, PixelFormat::RGBA32F, Builtin::LinearSrgb.profile()).is_err());
            assert_eq!(source, before);
        }
        let doc = clipboard_float_document(ColorMode::Rgb, Builtin::Srgb, &[0.5, 0.2, 0.7, 1.0]);
        let color = ClipColorContext::from_document(&doc).unwrap();
        let payload = f64::from_bits(0x7ff8_0000_0000_1234);
        let raw: Vec<_> = [0.5, 0.2, 0.7, payload].iter().flat_map(|v| v.to_ne_bytes()).collect();
        let source = Surface::try_from_interleaved(PixelFormat::RGBA64F, area, &raw).unwrap();
        let before = source.clone();
        let gray = PixelFormat::new(photocraft_color::ColorMode::Grayscale, SampleType::F64, true);
        let converted = convert_clip(&source, &color, gray, Builtin::GrayGamma22.profile()).unwrap();
        assert_eq!(converted.try_read_interleaved(area).unwrap().get(8..16).unwrap(), payload.to_ne_bytes());
        let cancelled = photocraft_raster::Interrupt::cancel_only(&|| true);
        assert!(matches!(convert_clip_with(&source, &color, gray, Builtin::GrayGamma22.profile(), &cancelled), Err(EngineError::Cancelled)));
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 10;
        assert!(convert_clip_with(&source, &color, gray, Builtin::GrayGamma22.profile(), &photocraft_raster::Interrupt::cancel_only(&cancel)).is_err());
        assert!(polls.load(std::sync::atomic::Ordering::Relaxed) > 10);
        assert_eq!(source, before);
    }
    #[test]
    fn clipboard_integer_codes_and_selection_keep_color_samples_exact() {
        use photocraft_doc::{ColorMode, SampleType};
        let area = Rect::new(0, 0, 1, 1);
        let mut doc = Document::new("U32 clipboard", photocraft_geom::Size::new(2, 2), ColorMode::Rgb, SampleType::U32);
        let codes = [2_147_483_648_u32, 1_610_612_736, 4_294_967_294, u32::MAX];
        let raw: Vec<_> = codes.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let mut layer = Layer::raster("Own integer markers", doc.pixel_format());
        *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), area, &raw).unwrap();
        doc.layers.push(layer);
        let mut s = Session::new();
        s.add_document(doc, None).unwrap();
        copy(&mut s, false).unwrap();
        assert_eq!(s.clipboard.as_ref().unwrap().surface.try_read_interleaved(area).unwrap(), raw);
        paste(&mut s, &json!({}), true).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().try_read_interleaved(area).unwrap(), raw);
        let colors = [0.5 + 2.0_f64.powi(-40), -0.0, 0.7, 1.0];
        let mut doc = clipboard_float_document(ColorMode::Rgb, photocraft_cms::Builtin::Srgb, &colors);
        let mut selection = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Grayscale, SampleType::F64, false));
        let coverage = 0.375 + 2.0_f64.powi(-40);
        selection.try_write_region_f64(area, &[coverage]).unwrap();
        doc.selection = Some(selection);
        s.add_document(doc, None).unwrap();
        copy(&mut s, false).unwrap();
        let raw = s.clipboard.as_ref().unwrap().surface.try_read_interleaved(area).unwrap();
        let expected: Vec<_> = [colors[0], colors[1], colors[2], coverage].iter().flat_map(|v| v.to_ne_bytes()).collect();
        assert_eq!(raw, expected);
    }
    #[test]
    fn clipboard_implicit_encoding_new_document_and_merged_lowbits_are_retained() {
        use photocraft_cms::Builtin;
        use photocraft_doc::{ColorMode, SampleEncoding, WorkingProfileInterpretation};
        let samples = [0.5 + 2.0_f64.powi(-40), -0.0, 0.7, 1.0];
        let mut doc = clipboard_float_document(ColorMode::Rgb, Builtin::LinearSrgb, &samples);
        doc.sample_encoding = Some(SampleEncoding::Linear);
        doc.icc_profile = Some(
            photocraft_cms::xcf_color::effective_profile(None, photocraft_cms::ColorSpace::Rgb, photocraft_cms::xcf_color::Encoding::Linear)
                .unwrap()
                .to_bytes(),
        );
        doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
        let mut s = Session::default();
        s.add_document(doc, None).unwrap();
        for merged in [false, true] {
            copy(&mut s, merged).unwrap();
            let clip = s.clipboard.as_ref().unwrap();
            assert_eq!(clip.surface.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), samples.iter().flat_map(|v| v.to_ne_bytes()).collect::<Vec<_>>());
            assert_eq!(clip.color.sample_encoding, Some(SampleEncoding::Linear));
        }
        let color = s.clipboard.as_ref().unwrap().color.clone();
        new_from_clipboard(&mut s).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.icc_profile, color.icc_profile);
        assert_eq!(doc.sample_encoding, color.sample_encoding);
        assert_eq!(doc.working_profile_interpretation, color.interpretation);
        assert_eq!(
            doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(),
            samples.iter().flat_map(|v| v.to_ne_bytes()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn checked_paste_quantization_and_model_error_preserve_history() {
        let mut s = session();
        let value = (128.5_f64 - 1e-8) / 255.0;
        let raw: Vec<_> = [value, 0.25, 0.5, 1.0].iter().flat_map(|v| v.to_ne_bytes()).collect();
        let area = Rect::new(0, 0, 1, 1);
        s.clipboard = Some(Clip {
            surface: Surface::try_from_interleaved(PixelFormat::RGBA64F, area, &raw).unwrap(),
            bounds: area,
            composition: Vec::new(),
            layers: None,
            resolution: None,
            color: ClipColorContext::from_document(&s.active().unwrap().doc).unwrap(),
        });
        let before = s.active().unwrap().doc.as_ref().clone();
        s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
        let st = s.active().unwrap();
        let surface = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        assert_eq!(surface.try_read_interleaved(area).unwrap()[0], 128);
        s.undo();
        assert_eq!(*s.active().unwrap().doc, before);
        s.clipboard = Some(Clip {
            surface: Surface::new(PixelFormat::GRAYA8),
            bounds: area,
            composition: Vec::new(),
            layers: None,
            resolution: None,
            color: ClipColorContext::from_document(&s.active().unwrap().doc).unwrap(),
        });
        let history = s.active().unwrap().history.past_len();
        assert!(s.execute("edit.pasteSpecial.pasteInPlace", json!({})).is_err());
        assert_eq!(*s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }

    #[test]
    fn new_from_clipboard_and_paste_with_no_document() {
        for depth in [8, 16, 32] {
            let mut s = Session::default();
            assert!(!s.is_enabled("file.newFromClipboard") && !s.is_enabled("edit.paste"), "nothing copied yet");
            assert!(s.execute("file.newFromClipboard", json!({})).is_err());
            s.execute("file.new", json!({"width": 100, "height": 100, "depth": depth})).unwrap();
            s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
            s.execute("select.rect", json!({"x": 10, "y": 20, "width": 30, "height": 15})).unwrap();
            s.execute("edit.copy", json!({})).unwrap();
            // With a document open: a second document, sized to the copy, its pixels at the origin.
            let r = s.execute("file.newFromClipboard", json!({})).unwrap();
            assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(30), Some(15)), "{r}");
            assert_eq!(s.documents().len(), 2);
            let d = &s.active().unwrap().doc;
            assert_eq!((d.size.width, d.size.height, d.layers.len()), (30, 15, 1));
            assert_eq!(d.pixel_format().sample, s.documents()[0].doc.pixel_format().sample, "{depth}-bit kept");
            assert_eq!(active_bounds(&s), Rect::new(0, 0, 30, 15));
            let px = d.layers[0].surface().unwrap().rgba(0, 0);
            assert!(px[0] > 0.99 && px[1] < 0.01 && px[3] > 0.99, "{px:?}");
            // With nothing open, Paste makes the document too.
            while s.active().is_some() {
                s.execute("file.close", json!({})).unwrap();
            }
            assert!(s.is_enabled("edit.paste") && !s.is_enabled("edit.pasteSpecial.pasteInPlace"));
            s.execute("edit.paste", json!({})).unwrap();
            assert_eq!(s.documents().len(), 1);
            assert_eq!(active_bounds(&s), Rect::new(0, 0, 30, 15));
            assert!(s.execute("edit.pasteSpecial.pasteInPlace", json!({})).is_ok(), "into the new document");
        }
    }

    #[test]
    fn copy_paste_in_place_and_centred() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(10, 10, 30, 30));
        assert!(s.active().unwrap().doc.selection.is_none());
        s.execute("edit.paste", json!({"center": [70, 70]})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(60, 60, 80, 80));
        assert_eq!(s.active().unwrap().doc.layers.len(), 4);
    }

    #[test]
    fn paste_with_malformed_center_does_not_panic() {
        // A short/empty `center` array must not panic (Rule 9) — it falls back to the canvas centre.
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        for center in [json!([5]), json!([]), json!("nope"), json!([1, 2, 3])] {
            // Fresh clipboard each time is unnecessary; copy persists. Must return Ok, never panic.
            assert!(s.execute("edit.paste", json!({ "center": center })).is_ok(), "center={center}");
        }
    }

    #[test]
    fn copy_paste_between_documents() {
        // Copy in one tab, paste into another: the clipboard is shared across documents.
        let mut s = session(); // doc A (100x100): red rect at (10,10)-(50,30)
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        // Open a second document (a new tab), which becomes active.
        s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        assert_eq!(s.active_index().unwrap(), 1, "the new document is the active tab");
        let before = s.active().unwrap().doc.layers.len();
        s.execute("edit.paste", json!({})).unwrap();
        assert_eq!(s.active_index().unwrap(), 1, "paste stays in the second tab");
        let d = &s.active().unwrap().doc;
        assert_eq!(d.layers.len(), before + 1, "pasted layer added to the second document");
        let pasted = d.layers.last().unwrap().surface().unwrap();
        let cb = pasted.content_bounds();
        let (cx, cy) = (cb.x0 + cb.width() as i32 / 2, cb.y0 + cb.height() as i32 / 2);
        let mut px = [[0.0f32; 4]; 1];
        pasted.read_rgba_into(Rect::new(cx, cy, cx + 1, cy + 1), &mut px);
        assert!(px[0][0] > 0.9 && px[0][1] < 0.1 && px[0][2] < 0.1, "pasted content is red: {:?}", px[0]);
    }

    /// #1035: a `depth`-bit document with a blue-grey square copied from layer `src` (at
    /// (5,5)-(15,15)) and an active layer `m` above it with a hide-all mask.
    fn mask_target_session(depth: u32) -> (Session, LayerId, LayerId) {
        let mut s = Session::default();
        s.execute("file.new", json!({"width": 40, "height": 40, "depth": depth})).unwrap();
        let src = LayerId(s.execute("layer.new.layer", json!({"name": "src"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#3366cc"})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let m = LayerId(s.execute("layer.new.layer", json!({"name": "m"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("layer.layerMask.hideAll", json!({})).unwrap();
        (s, src, m)
    }

    /// The luminosity of `src`'s pixel at (`x`, `y`): what a paste into a mask writes there.
    fn luminosity(s: &Session, src: LayerId, x: i32, y: i32) -> f32 {
        let p = s.active().unwrap().doc.layer(src).unwrap().surface().unwrap().rgba(x, y);
        let source = crate::color_cmds::try_document_profile(&s.active().unwrap().doc).unwrap();
        let t = photocraft_cms::Transform::new(&source, photocraft_cms::Builtin::SGray.profile(), photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
        let mut output = [0.0_f64];
        t.try_eval_f64(&[f64::from(p[0]), f64::from(p[1]), f64::from(p[2])], &mut output).unwrap();
        output[0] as f32
    }

    fn mask_at(s: &Session, id: LayerId, x: i32, y: i32) -> f32 {
        s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().value(x, y)
    }

    #[test]
    fn paste_into_a_targeted_mask_writes_the_luminosity_in_one_step() {
        for depth in [8, 16, 32] {
            let (mut s, src, m) = mask_target_session(depth);
            let lum = luminosity(&s, src, 10, 10);
            assert!(lum > 0.05 && lum < 0.95, "{depth}-bit: a mid grey ({lum})");
            let (layers, steps) = (s.active().unwrap().doc.layer_count(), s.active().unwrap().history.past_len());
            s.execute("edit.pasteSpecial.pasteInPlace", json!({"target": "mask"})).unwrap();
            let st = s.active().unwrap();
            assert_eq!(st.doc.layer_count(), layers, "{depth}-bit: no new layer");
            assert_eq!(st.history.past_len(), steps + 1, "{depth}-bit: one history step");
            assert_eq!(st.active_layer, Some(m));
            assert!((mask_at(&s, m, 10, 10) - lum).abs() < 2.0 / 255.0, "{depth}-bit: {} vs {lum}", mask_at(&s, m, 10, 10));
            assert_eq!(mask_at(&s, m, 20, 20), 0.0, "{depth}-bit: the mask outside the paste is unchanged");
            assert!(st.doc.layer(m).unwrap().surface().unwrap().content_bounds().is_empty(), "{depth}-bit: the pixels are untouched");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(mask_at(&s, m, 10, 10), 0.0, "{depth}-bit: undo restores the mask");
        }
    }

    #[test]
    fn paste_follows_the_mask_view_and_is_placed_like_a_paste() {
        let (mut s, src, m) = mask_target_session(8);
        let lum = luminosity(&s, src, 10, 10);
        // ⌥-click the mask thumbnail: the canvas shows the mask, and pasting targets it.
        s.execute(crate::mask_view_cmds::ID, json!({"layer": m.0, "mode": "gray"})).unwrap();
        let r = s.execute("edit.paste", json!({"center": [30, 30]})).unwrap();
        assert_eq!(r["offset"], json!([20, 20]));
        assert!((mask_at(&s, m, 30, 30) - lum).abs() < 2.0 / 255.0);
        assert_eq!(mask_at(&s, m, 10, 10), 0.0);
        assert_eq!(s.active().unwrap().doc.layers.len(), 3, "Background, src, m");
    }

    #[test]
    fn paste_into_and_outside_a_targeted_mask_keep_to_the_selection() {
        let (mut s, src, m) = mask_target_session(16);
        let lum = luminosity(&s, src, 10, 10);
        // The paste is centred on the selection, so lands in place: only the selection takes it.
        s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
        s.execute("edit.pasteSpecial.pasteInto", json!({"target": "mask"})).unwrap();
        assert!((mask_at(&s, m, 9, 9) - lum).abs() < 2.0 / 255.0);
        assert_eq!(mask_at(&s, m, 7, 7), 0.0, "pasted, but outside the selection");
        assert!(s.active().unwrap().doc.selection.is_none());
        assert_eq!(s.active().unwrap().doc.layers.len(), 3, "no new layer");
        s.execute("layer.layerMask.hideAll", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
        s.execute("edit.pasteSpecial.pasteOutside", json!({"target": "mask"})).unwrap();
        assert_eq!(mask_at(&s, m, 9, 9), 0.0, "Paste Outside leaves the selection alone");
        assert!((mask_at(&s, m, 7, 7) - lum).abs() < 2.0 / 255.0);
    }

    #[test]
    fn paste_follows_a_targeted_alpha_channel() {
        let (mut s, src, _) = mask_target_session(8);
        let lum = luminosity(&s, src, 10, 10);
        s.execute("channel.new", json!({})).unwrap();
        s.execute("edit.pasteSpecial.pasteInPlace", json!({"target": {"channel": 0}})).unwrap();
        let c = &s.active().unwrap().doc.channels[0].surface;
        assert!((c.sample_channel(10, 10, 0) - lum).abs() < 2.0 / 255.0);
        assert_eq!(s.active().unwrap().doc.layers.len(), 3, "no new layer");
    }

    #[test]
    fn paste_into_a_missing_mask_or_channel_is_an_error() {
        let (mut s, _, m) = mask_target_session(8);
        s.edit("drop mask", |doc, _| {
            doc.layer_mut(m).unwrap().mask = None;
            Ok(())
        })
        .unwrap();
        let layers = s.active().unwrap().doc.layer_count();
        for target in [json!("mask"), json!({"channel": 7}), json!("quickMask")] {
            for id in ["edit.paste", "edit.pasteSpecial.pasteInPlace"] {
                assert!(s.execute(id, json!({ "target": target })).is_err(), "{id} {target}");
            }
        }
        assert_eq!(s.active().unwrap().doc.layer_count(), layers, "nothing pasted as a layer");
    }

    #[test]
    fn paste_between_different_color_modes() {
        // Copy from an RGB document, paste into a Grayscale one: the clipboard converts to the
        // target document's format (a realistic between-tabs case that must not panic).
        let mut s = session(); // RGB, red rect
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        s.execute("file.new", json!({"width": 120, "height": 120, "mode": "gray"})).unwrap();
        s.execute("edit.paste", json!({})).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!(d.mode, photocraft_doc::ColorMode::Grayscale);
        let pasted = d.layers.last().unwrap().surface().unwrap();
        assert_eq!(pasted.format().mode, photocraft_color::ColorMode::Grayscale, "pasted layer is grayscale");
        assert!(!pasted.content_bounds().is_empty(), "pasted content exists");
    }

    #[test]
    fn transform_scale_grows_content() {
        let mut s = session(); // red rect (10,10)-(50,30): 40x20
        s.execute("edit.transform", json!({"matrix": [2, 0, 0, 2, 0, 0]})).unwrap();
        let d = &s.active().unwrap().doc;
        let cb = d.layers.last().unwrap().surface().unwrap().content_bounds();
        // Scaled ~2x about the origin (≈ 80x40 at ≈ (20,20)); allow ~1px bilinear edge bleed.
        assert!((cb.x0 - 20).abs() <= 2 && (cb.y0 - 20).abs() <= 2, "origin ~ (20,20): {cb:?}");
        assert!((cb.width() as i32 - 80).abs() <= 3 && (cb.height() as i32 - 40).abs() <= 3, "size ~ 80x40: {cb:?}");
    }

    #[test]
    fn cut_and_clear_on_the_background_fill_with_the_background_colour() {
        // The Background keeps its transparency lock: cutting fills with the background colour.
        let fresh = || {
            let mut s = Session::default();
            s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
            s.tools.background = [1.0, 0.0, 0.0, 1.0];
            s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
            s
        };
        let px = |s: &Session, i: usize, x: i32, y: i32| s.active().unwrap().doc.layers[i].surface().unwrap().pixel(x, y);
        for cmd in ["edit.cut", "edit.clear", "layer.new.layerViaCut"] {
            let mut s = fresh();
            s.execute(cmd, json!({})).unwrap();
            let d = &s.active().unwrap().doc;
            assert!(crate::extra_cmds::is_background(&d.layers[0]), "{cmd}: still the Background");
            assert_eq!(px(&s, 0, 8, 8), vec![1.0, 0.0, 0.0, 1.0], "{cmd}: cut area is the background colour");
            assert_eq!(px(&s, 0, 30, 20), vec![1.0, 1.0, 1.0, 1.0], "{cmd}: outside untouched");
        }
        // Layer via Cut still lifts the original pixels onto the new layer.
        let mut s = fresh();
        s.execute("layer.new.layerViaCut", json!({})).unwrap();
        assert_eq!(px(&s, 1, 8, 8), vec![1.0, 1.0, 1.0, 1.0]);
        // Once it's a normal layer, cutting makes transparency again.
        let mut s = fresh();
        s.execute("layer.new.layerFromBackground", json!({})).unwrap();
        s.execute("edit.clear", json!({})).unwrap();
        assert_eq!(px(&s, 0, 8, 8)[3], 0.0);
    }

    #[test]
    fn layer_via_copy_of_a_background_keeps_colour_and_partial_selection() {
        // #668: lift reads only the selected area now; the result must be what converting the
        // whole layer gave: the colour, with the selection's coverage as alpha.
        for depth in [8, 16, 32] {
            let mut s = Session::default();
            s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth, "background": "#336699"})).unwrap();
            s.edit("partial selection", |doc, _| {
                let mut m = photocraft_raster::Surface::new(PixelFormat::GRAY8);
                m.fill_rect(Rect::new(5, 5, 15, 15), &[1.0]);
                m.fill_rect(Rect::new(15, 5, 20, 15), &[0.5]);
                doc.selection = Some(m);
                Ok(())
            })
            .unwrap();
            let id = s.execute("layer.new.layerViaCopy", json!({})).unwrap()["layer"].as_u64().unwrap();
            let d = s.active().unwrap();
            let surf = d.doc.layer(LayerId(id)).unwrap().surface().unwrap();
            assert!(surf.format().alpha, "@{depth}");
            let full = surf.read_region(Rect::new(6, 6, 7, 7));
            let half = surf.read_region(Rect::new(16, 6, 17, 7));
            for (got, want) in full.iter().zip([0.2, 0.4, 0.6, 1.0]) {
                assert!((got - want).abs() < 0.01, "@{depth}: {full:?}");
            }
            assert!((half[3] - 0.5).abs() < 0.01 && (half[0] - 0.2).abs() < 0.01, "@{depth}: {half:?}");
            assert_eq!(surf.content_bounds(), Rect::new(5, 5, 20, 15), "@{depth}");
        }
    }

    #[test]
    fn cut_clears_and_layer_via_copy_cut() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 20})).unwrap();
        s.execute("edit.cut", json!({})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(20, 10, 50, 30));
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 20})).unwrap();
        s.execute("layer.new.layerViaCut", json!({})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(10, 10, 20, 30));
        let d = &s.active().unwrap().doc;
        assert_eq!(d.layers[1].surface().unwrap().content_bounds(), Rect::new(20, 10, 50, 30));
        // Without a selection ⌘J duplicates the layer.
        let mut s = session();
        s.execute("layer.new.layerViaCopy", json!({})).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!(d.layers.len(), 3);
        assert!(d.layers[2].name.ends_with("copy"));
    }

    fn active_content(s: &Session) -> LayerContent {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().content.clone()
    }

    /// #541: a smart object copies what it shows, with or without a selection, and pastes as
    /// pixels. Cut edits pixels, so it's refused; Layer via Copy and Duplicate Layer copy the layer.
    #[test]
    fn copy_a_smart_object_layer() {
        let mut s = session();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        assert!(s.is_enabled("edit.copy"));
        assert_eq!(s.execute("edit.copy", json!({})).unwrap()["bounds"], json!([10, 10, 40, 20]), "no selection: the whole layer");
        s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
        assert!(matches!(active_content(&s), LayerContent::Raster(_)));
        assert_eq!(active_bounds(&s), Rect::new(10, 10, 50, 30));
        assert_eq!(
            s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().pixel(20, 20),
            vec![1.0, 0.0, 0.0, 1.0]
        );
        s.undo();
        assert!(matches!(active_content(&s), LayerContent::Smart(_)));
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
        assert_eq!(s.execute("edit.copy", json!({})).unwrap()["bounds"], json!([10, 10, 10, 10]), "only the selected part");
        let cut = s.execute("edit.cut", json!({})).unwrap_err().to_string();
        assert!(cut.contains("not directly editable"), "{cut}");
        assert!(matches!(active_content(&s), LayerContent::Smart(_)));
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("layer.new.layerViaCopy", json!({})).unwrap();
        assert!(matches!(active_content(&s), LayerContent::Smart(_)), "⌘J without a selection duplicates the smart object");
        s.execute("layer.duplicate", json!({})).unwrap();
        assert!(matches!(active_content(&s), LayerContent::Smart(_)));
        assert_eq!(s.active().unwrap().doc.layers.len(), 4);
    }

    /// Type and shape layers copy their rendered pixels too; groups and adjustments have none.
    #[test]
    fn copy_reads_any_layer_that_shows_pixels() {
        let mut s = session();
        s.execute("type.create", json!({"text": "Hi", "size": 30, "x": 20, "y": 60})).unwrap();
        assert!(!s.is_enabled("edit.cut"));
        assert!(s.execute("edit.copy", json!({})).is_ok());
        s.execute("layer.groupLayers", json!({})).unwrap();
        assert!(!s.is_enabled("edit.copy"));
    }

    #[test]
    fn merge_visible_keeps_hidden_layers() {
        let mut s = session();
        s.execute("layer.new.layer", json!({})).unwrap();
        let hidden = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.setProps", json!({"layer": hidden.0, "visible": false})).unwrap();
        s.execute("layer.mergeVisible", json!({})).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!(d.layers.len(), 2);
        assert_eq!(d.layers[0].name, "Background");
        assert_eq!(d.layers[1].id, hidden);
        assert_eq!(d.layers[0].surface().unwrap().pixel(20, 20)[..3], [1.0, 0.0, 0.0]);
    }

    #[test]
    fn auto_tone_stretches_a_low_contrast_layer() {
        let mut s = Session::default();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        s.edit("grey ramp", |doc, _| {
            let surf = doc.layers[0].surface_mut().unwrap();
            for x in 0..64 {
                let v = 0.3 + 0.4 * x as f32 / 63.0;
                surf.fill_rect(Rect::new(x, 0, x + 1, 64), &[v, v * 0.9, v * 0.8, 1.0]);
            }
            Ok(())
        })
        .unwrap();
        for cmd in ["image.autoTone", "image.autoContrast", "image.autoColor"] {
            s.execute(cmd, json!({})).unwrap();
            let d = &s.active().unwrap().doc;
            let (a, b) = (d.layers[0].surface().unwrap().pixel(0, 0), d.layers[0].surface().unwrap().pixel(63, 0));
            // Auto Contrast keeps colour (shared points): only the extreme channels reach 0 and 1.
            let (lo, hi) = if cmd == "image.autoContrast" { (a[2], b[0]) } else { (a[0], b[0]) };
            assert!(lo < 0.05 && hi > 0.95, "{cmd}: {a:?} {b:?}");
            s.undo();
        }
    }

    /// #352: ⌥⇧⌘T steps and repeats: each press adds a copy one transform further on, and each
    /// is one undo step.
    #[test]
    fn transform_again_on_a_copy_steps_and_repeats() {
        let mut s = session();
        assert!(s.execute("edit.transform.againCopy", json!({})).is_err(), "nothing to repeat yet");
        assert_eq!(s.active().unwrap().doc.layers.len(), 2, "a refusal leaves no copy");
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 10, 0]})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        for (i, x) in [30, 40].into_iter().enumerate() {
            s.execute("edit.transform.againCopy", json!({})).unwrap();
            let d = &s.active().unwrap().doc;
            assert_eq!(d.layers.len(), 3 + i);
            assert_eq!(active_bounds(&s), Rect::new(x, 10, x + 40, 30), "the copy moved one step further");
            assert_eq!(s.active().unwrap().history.past_len(), steps + 1 + i, "one history step per press");
        }
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Transform Again"));
        // The original and the first copy stay where they were.
        let d = &s.active().unwrap().doc;
        let b = |i: usize| d.layers[i].surface().unwrap().content_bounds();
        assert_eq!((b(1), b(2)), (Rect::new(20, 10, 60, 30), Rect::new(30, 10, 70, 30)));
        // Undo removes the copy and its move together.
        s.undo();
        assert_eq!(s.active().unwrap().doc.layers.len(), 3);
        assert_eq!(active_bounds(&s), Rect::new(30, 10, 70, 30));
        // A copy that can't move (position locked) is taken back.
        let id = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.setProps", json!({"layer": id.0, "locks": {"position": true}})).unwrap();
        let layers = s.active().unwrap().doc.layers.len();
        assert!(s.execute("edit.transform.againCopy", json!({})).unwrap_err().to_string().contains("locked"));
        assert_eq!(s.active().unwrap().doc.layers.len(), layers);
        assert!(!s.active().unwrap().history.can_redo(), "nothing to redo");
    }

    #[test]
    fn toggle_last_state_and_transform_again() {
        let mut s = session();
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 10, 0]})).unwrap();
        s.execute("edit.transform.again", json!({})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(30, 10, 70, 30));
        s.execute("edit.toggleLastState", json!({})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(20, 10, 60, 30));
        s.execute("edit.toggleLastState", json!({})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(30, 10, 70, 30));
    }

    #[test]
    fn guides_are_undoable() {
        let mut s = session();
        s.execute("view.newGuide", json!({"orientation": "vertical", "position": 40})).unwrap();
        s.execute("view.newGuide", json!({"orientation": "horizontal", "position": 12.5})).unwrap();
        s.execute("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": 60})).unwrap();
        let g = &s.active().unwrap().doc.guides;
        assert_eq!((g.vertical.clone(), g.horizontal.clone()), (vec![60.0], vec![12.5]));
        s.execute("view.deleteGuide", json!({"orientation": "horizontal", "index": 0})).unwrap();
        s.undo();
        assert_eq!(s.active().unwrap().doc.guides.horizontal, vec![12.5]);
        s.execute("view.clearGuides", json!({})).unwrap();
        assert!(s.active().unwrap().doc.guides.vertical.is_empty());
        assert!(s.execute("view.moveGuide", json!({"orientation": "vertical", "index": 3, "position": 1})).is_err());
    }

    /// #2955: the rulers' zero point is set and reset by command, undoable, and bad params are
    /// refused without touching the document.
    #[test]
    fn ruler_origin_is_set_reset_and_undoable() {
        let mut s = session();
        assert_eq!(s.active().unwrap().doc.ruler_origin, [0.0, 0.0]);
        assert_eq!(s.execute("view.setRulerOrigin", json!({"x": 40, "y": 12.5})).unwrap(), json!({"x": 40.0, "y": 12.5}));
        assert_eq!(s.active().unwrap().doc.ruler_origin, [40.0, 12.5]);
        s.execute("view.resetRulerOrigin", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.ruler_origin, [0.0, 0.0]);
        s.undo();
        assert_eq!(s.active().unwrap().doc.ruler_origin, [40.0, 12.5]);
        for bad in [json!({}), json!({"x": 1}), json!({"x": "a", "y": 2}), json!({"x": 1e300, "y": 0}), json!({"x": -2e7, "y": 0}), json!(null), json!([1, 2])]
        {
            assert!(s.execute("view.setRulerOrigin", bad.clone()).is_err(), "{bad} is refused");
        }
        assert_eq!(s.active().unwrap().doc.ruler_origin, [40.0, 12.5]);
        let mut empty = Session::new();
        assert!(empty.execute("view.setRulerOrigin", json!({"x": 1, "y": 2})).is_err());
        assert!(empty.execute("view.resetRulerOrigin", json!({})).is_err());
    }

    /// The last guide added in `orientation` for `position`.
    fn new_guide(s: &mut Session, orientation: &str, position: Value) -> f64 {
        s.execute("view.newGuide", json!({"orientation": orientation, "position": position})).unwrap();
        let g = &s.active().unwrap().doc.guides;
        *if orientation == "vertical" { &g.vertical } else { &g.horizontal }.last().unwrap()
    }

    #[test]
    fn guide_positions_take_photoshop_units() {
        let mut s = Session::new();
        // 4 × 2 inches at 100 ppi.
        s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 100})).unwrap();
        // Percent follows the guide's own axis: the width for a vertical guide, the height for a horizontal one.
        assert_eq!(new_guide(&mut s, "vertical", json!("50%")), 200.0);
        assert_eq!(new_guide(&mut s, "horizontal", json!("50%")), 100.0);
        for one_inch in ["1in", "1 in", "2.54cm", "25.4 mm", "72pt", "6pc", "100px", "100", "1,0 inch"] {
            assert!((new_guide(&mut s, "vertical", json!(one_inch)) - 100.0).abs() < 1e-3, "{one_inch}");
        }
        // Fractional and negative positions are kept; a plain number is pixels, as before.
        assert_eq!(new_guide(&mut s, "horizontal", json!("12.5px")), 12.5);
        assert_eq!(new_guide(&mut s, "vertical", json!("-10px")), -10.0);
        assert_eq!(new_guide(&mut s, "vertical", json!(40)), 40.0);
        // Points and picas follow Units & Rulers › Point Size.
        s.execute("prefs.set", json!({"path": "unitsAndRulers.pointSize", "value": "traditional"})).unwrap();
        assert!((new_guide(&mut s, "vertical", json!("72.27pt")) - 100.0).abs() < 1e-3);
        // A guide moved with a unit lands where a new one would.
        s.execute("view.moveGuide", json!({"orientation": "horizontal", "index": 0, "position": "25%"})).unwrap();
        assert_eq!(s.active().unwrap().doc.guides.horizontal[0], 50.0);
    }

    #[test]
    fn new_guides_keep_their_own_colour_through_moves_deletes_and_undo() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
        let color = |s: &Session, vertical: bool, p: f64| s.active().unwrap().doc.guides.color(vertical, p);
        // Without a colour a guide uses the preference colour, as ruler-drag guides always did.
        s.execute("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
        assert_eq!(color(&s, true, 10.0), None);
        assert!(s.active().unwrap().doc.guides.colors.is_empty());
        s.execute("view.newGuide", json!({"orientation": "vertical", "position": 20, "color": "magenta"})).unwrap();
        s.execute("view.newGuide", json!({"orientation": "horizontal", "position": 20, "color": "#123456"})).unwrap();
        s.execute("view.newGuide", json!({"orientation": "vertical", "position": 30, "color": "LightBlue"})).unwrap();
        assert_eq!(color(&s, true, 20.0), Some([0xff, 0x00, 0xff]));
        assert_eq!(color(&s, false, 20.0), Some([0x12, 0x34, 0x56]), "the same position across is a different guide");
        assert_eq!(color(&s, true, 30.0), Some([0x33, 0x66, 0xff]), "names are case-insensitive");
        // Moving takes the colour along.
        s.execute("view.moveGuide", json!({"orientation": "vertical", "index": 1, "position": 25})).unwrap();
        assert_eq!((color(&s, true, 20.0), color(&s, true, 25.0)), (None, Some([0xff, 0x00, 0xff])));
        // Undo restores it where it was.
        s.undo();
        assert_eq!((color(&s, true, 20.0), color(&s, true, 25.0)), (Some([0xff, 0x00, 0xff]), None));
        // Deleting drops it; clearing drops them all.
        s.execute("view.deleteGuide", json!({"orientation": "vertical", "index": 2})).unwrap();
        assert_eq!(color(&s, true, 30.0), None);
        assert_eq!(s.active().unwrap().doc.guides.colors.len(), 2);
        s.execute("view.clearGuides", json!({})).unwrap();
        assert!(s.active().unwrap().doc.guides.colors.is_empty());
    }

    #[test]
    fn moving_one_of_two_coloured_guides_at_one_position_leaves_the_other_its_colour() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
        for _ in 0..2 {
            s.execute("view.newGuide", json!({"orientation": "vertical", "position": 50, "color": "green"})).unwrap();
        }
        s.execute("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": 60})).unwrap();
        let g = &s.active().unwrap().doc.guides;
        assert_eq!((g.color(true, 50.0), g.color(true, 60.0)), (Some([0x33, 0xff, 0x33]), Some([0x33, 0xff, 0x33])));
    }

    #[test]
    fn cyan_is_the_default_guide_colour_so_ruler_guides_read_as_cyan() {
        let default = crate::prefs::Preferences::default().guides_grid_and_slices.guide_color;
        let cyan = GUIDE_COLORS.iter().find(|(name, ..)| *name == "cyan").map(|(.., rgb)| *rgb);
        assert_eq!(cyan, crate::prefs::parse_hex(&default));
    }

    #[test]
    fn bad_guide_colours_are_refused_without_a_guide() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
        let undo_before = s.active().unwrap().history.can_undo();
        for bad in [json!("puce"), json!("#12345"), json!("#gggggg"), json!(""), json!(7), json!(null), json!(["cyan"])] {
            assert!(s.execute("view.newGuide", json!({"orientation": "vertical", "position": 10, "color": bad.clone()})).is_err(), "{bad}");
        }
        let g = &s.active().unwrap().doc.guides;
        assert!(g.vertical.is_empty() && g.colors.is_empty());
        assert_eq!(s.active().unwrap().history.can_undo(), undo_before);
    }

    #[test]
    fn editing_a_guide_changes_what_is_given_in_one_undoable_step() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
        s.execute("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
        s.execute("view.newGuide", json!({"orientation": "vertical", "position": 20, "color": "magenta"})).unwrap();
        s.execute("view.newGuide", json!({"orientation": "vertical", "position": 30})).unwrap();
        let guides = |s: &Session| s.active().unwrap().doc.guides.clone();
        // Position only: the guide keeps its index and its own colour.
        s.execute("view.editGuide", json!({"orientation": "vertical", "index": 1, "position": "50%"})).unwrap();
        let g = guides(&s);
        assert_eq!(g.vertical, vec![10.0, 200.0, 30.0]);
        assert_eq!((g.color(true, 20.0), g.color(true, 200.0)), (None, Some([0xff, 0x00, 0xff])));
        // Colour only: an uncoloured guide gets one; the position stays.
        s.execute("view.editGuide", json!({"orientation": "vertical", "index": 0, "color": "#123456"})).unwrap();
        assert_eq!(guides(&s).color(true, 10.0), Some([0x12, 0x34, 0x56]));
        // Turning it: % is of the new orientation's extent, and the colour goes along.
        s.execute("view.editGuide", json!({"orientation": "vertical", "index": 1, "newOrientation": "horizontal", "position": "25%"})).unwrap();
        let g = guides(&s);
        assert_eq!((g.vertical.clone(), g.horizontal.clone()), (vec![10.0, 30.0], vec![50.0]));
        assert_eq!((g.color(true, 200.0), g.color(false, 50.0)), (None, Some([0xff, 0x00, 0xff])));
        // One step each: undo restores the vertical guide with its colour.
        s.undo();
        let g = guides(&s);
        assert_eq!((g.vertical.clone(), g.horizontal.is_empty()), (vec![10.0, 200.0, 30.0], true));
        assert_eq!(g.color(true, 200.0), Some([0xff, 0x00, 0xff]));
    }

    #[test]
    fn bad_guide_edits_are_refused_without_a_history_step() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
        s.execute("view.newGuide", json!({"orientation": "vertical", "position": 10, "color": "green"})).unwrap();
        let before = s.active().unwrap().doc.guides.clone();
        for bad in [
            json!({}),
            json!({"orientation": "vertical"}),
            json!({"orientation": "vertical", "index": 1}),
            json!({"orientation": "horizontal", "index": 0}),
            json!({"orientation": "vertical", "index": -1}),
            json!({"orientation": "vertical", "index": "0"}),
            json!({"orientation": "vertical", "index": u64::MAX}),
            json!({"orientation": "diagonal", "index": 0}),
            json!({"orientation": "vertical", "index": 0, "newOrientation": 3}),
            json!({"orientation": "vertical", "index": 0, "position": "far"}),
            json!({"orientation": "vertical", "index": 0, "position": 1e12}),
            json!({"orientation": "vertical", "index": 0, "color": "puce"}),
        ] {
            assert!(s.execute("view.editGuide", bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(s.active().unwrap().doc.guides, before);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("New Guide"));
    }

    #[test]
    fn bad_guide_positions_are_refused_without_a_guide() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 100})).unwrap();
        let undo_before = s.active().unwrap().history.can_undo();
        for bad in [
            json!("abc"),
            json!("50 furlongs"),
            json!(""),
            json!("%"),
            json!("NaN"),
            json!("inf"),
            json!("1e400"),
            json!("2000000px"),
            json!(1e300),
            json!(true),
            json!([1]),
        ] {
            assert!(s.execute("view.newGuide", json!({"orientation": "vertical", "position": bad.clone()})).is_err(), "{bad}");
        }
        assert!(s.execute("view.newGuide", json!({"orientation": "vertical"})).is_err(), "no position");
        let g = &s.active().unwrap().doc.guides;
        assert!(g.vertical.is_empty() && g.horizontal.is_empty());
        assert_eq!(s.active().unwrap().history.can_undo(), undo_before, "a refused guide records no history step");
    }

    /// Guides keep their exact place (#2918): 1 cm at 300 ppi through New Guide and .pcraft, and
    /// a 1/32 px PSD position past 2^19 px, where f32 has no 1/32 steps left.
    #[test]
    fn guides_keep_sub_pixel_positions_through_pcraft_and_psd() {
        let (cm, far) = (300.0 / 2.54, 600_000.031_25);
        let mut s = session();
        s.execute("view.newGuide", json!({"orientation": "horizontal", "position": cm})).unwrap();
        s.execute("view.newGuide", json!({"orientation": "vertical", "position": far})).unwrap();
        fn wide(v: impl Into<f64>) -> f64 {
            v.into()
        }
        let exact = |g: &photocraft_doc::Guides| (wide(g.horizontal[0]), wide(g.vertical[0]));
        let doc = &s.active().unwrap().doc;
        assert_eq!(exact(&doc.guides), (cm, far));
        let pcraft = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
        assert_eq!(exact(&photocraft_format::load_from_bytes(&pcraft).unwrap().guides), (cm, far));
        let psd = photocraft_io::export(doc, "psd", &Default::default()).unwrap().bytes;
        let back = photocraft_io::import("g.psd", &psd).unwrap().document.guides;
        assert_eq!(wide(back.vertical[0]), far, "1032 stores 1/32 px");
    }

    #[test]
    fn guide_physical_units_use_independent_axes_and_preserve_redo_on_refusal() {
        let mut s = session();
        s.edit("Own axes", |document, _| {
            document
                .set_physical_resolution(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: photocraft_doc::PhysicalUnit::Inch })
                .map_err(|error| EngineError::Other(error.to_string()))?;
            Ok(())
        })
        .unwrap();
        s.execute("view.newGuide", json!({"orientation":"vertical","position":1.0,"unit":"in"})).unwrap();
        s.execute("view.newGuide", json!({"orientation":"horizontal","position":25.4,"unit":"mm"})).unwrap();
        assert_eq!(s.active().unwrap().doc.guides.vertical, vec![144.0]);
        assert_eq!(s.active().unwrap().doc.guides.horizontal, vec![288.0]);
        s.execute("view.moveGuide", json!({"orientation":"vertical","index":0,"position":72.0,"unit":"pt"})).unwrap();
        assert_eq!(s.active().unwrap().doc.guides.vertical, vec![144.0]);
        s.execute("view.moveGuide", json!({"orientation":"horizontal","index":0,"position":-0.5,"unit":"document"})).unwrap();
        assert_eq!(s.active().unwrap().doc.guides.horizontal, vec![-144.0]);
        assert!(s.undo());
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        for parameters in [json!({"position":f64::MAX}), json!({"position":"wrong"}), json!({"position":1.0,"unit":"unknown"})] {
            assert!(s.execute("view.newGuide", parameters).is_err());
            assert_eq!(s.active().unwrap().doc, before);
            assert_eq!(s.active().unwrap().revision, revision);
        }
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc.guides.horizontal, vec![-144.0]);
    }

    #[test]
    fn captured_guide_target_refuses_an_intervening_edit_without_touching_guides_or_redo() {
        let mut s = session();
        let document = s.active().unwrap().doc.id.0.to_string();
        let revision = s.active().unwrap().revision.to_string();
        s.edit("Own intervening edit", |document, _| {
            document.name = "Changed name".into();
            Ok(())
        })
        .unwrap();
        assert!(s.undo());
        let before = s.active().unwrap().doc.clone();
        assert!(
            s.execute("view.newGuide", json!({"documentId":document,"expectedRevision":revision,"orientation":"vertical","position":1,"unit":"in"})).is_err()
        );
        assert_eq!(s.active().unwrap().doc, before);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc.name, "Changed name");
    }
}

fn multi_copy_ids(d: &crate::DocState) -> Result<Vec<LayerId>> {
    if d.doc.selection.is_some() || !crate::layer_copy_cmds::layered(d.doc.mode) { return Ok(Vec::new()); }
    let selected=d.selected_layers()?;
    if selected.len()<2 { return Ok(Vec::new()); }
    Ok(crate::layer_multi_cmds::top_level(&d.doc,&selected))
}
fn whole_layers(d:&crate::DocState,ids:&[LayerId])->Result<Document> {
    let mut copies=scratch_doc(&d.doc);
    copies.layers.try_reserve(ids.len()).map_err(|_|pixel_error("not enough memory to copy layers"))?;
    for id in ids {
        let mut layer=d.doc.layer(*id).ok_or(EngineError::NoLayer(*id))?.clone();
        if d.doc.is_background(*id) {layer.name="Layer 0".into();layer.locks=Default::default();}
        copies.layers.push(layer);
    }
    Ok(copies)
}
fn can_copy(s:&Session)->std::result::Result<(),String> {
    if crate::channel_clip::native_gray_copy_candidate(s) { return Ok(()); }
    if crate::channel_clip::copies_alpha_channel(s) {return Ok(());}
    let d=s.active().ok_or("no document open")?;
    if !multi_copy_ids(d).map_err(|error|error.to_string())?.is_empty() {return Ok(());}
    has_layer_pixels(s).or_else(|error| {
        let id=d.active_layer.ok_or(error.clone())?;
        if takes_whole(d,id).map_err(|error|error.to_string())? {Ok(())}else {Err(error)}
    })
}
fn copy_linked_sources(source: &Document, copies: &mut Document) -> Result<()> {
    fn collect<'a>(layers: &'a [Layer], depth: usize, paths: &mut std::collections::HashSet<&'a str>) -> Result<()> {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(EngineError::Other("can't copy linked smart objects from layers nested too deeply".into()));
        }
        for layer in layers {
            if let LayerContent::Smart(sm) = &layer.content
                && let photocraft_doc::SmartSource::Linked { path } = &sm.source
            {
                paths.try_reserve(1).map_err(|_| EngineError::Other("not enough memory to copy smart-object source IDs".into()))?;
                paths.insert(path);
            }
            if let Some(children) = layer.children().filter(|children| !children.is_empty()) {
                collect(children, depth + 1, paths)?;
            }
        }
        Ok(())
    }
    let mut paths = std::collections::HashSet::new();
    collect(&copies.layers, 0, &mut paths)?;
    if paths.is_empty() {
        return Ok(());
    }
    for (signature, key, data) in &source.metadata.psd_global_blocks {
        if !matches!(key, b"lnk2" | b"lnk3" | b"lnkD" | b"lnkE") {
            continue;
        }
        let selected = photocraft_io::linked::select_items(data, &|uuid, _| paths.contains(uuid))
            .map_err(|e| EngineError::Other(format!("Could not copy smart-object contents: {e}; the clipboard was not changed")))?;
        let selected = selected.map(std::sync::Arc::new).unwrap_or_else(|| data.clone());
        if !selected.is_empty() {
            copies.metadata.psd_global_blocks.try_reserve(1).map_err(|_| EngineError::Other("not enough memory to copy smart-object source records".into()))?;
            copies.metadata.psd_global_blocks.push((*signature, *key, selected));
        }
    }
    Ok(())
}

fn takes_whole(d: &crate::DocState, id: LayerId) -> Result<bool> {
    Ok(d.doc.selection.is_none()
        && d.selected_layers()?.len() <= 1
        && crate::layer_copy_cmds::layered(d.doc.mode)
        && d.doc.layer(id).is_some_and(|l| matches!(l.content, LayerContent::Smart(_) | LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Group(_))))
}
fn whole_layer(d: &crate::DocState, id: LayerId) -> Result<Document> {
    if !takes_whole(d,id)? { return Err(pixel_error("the active layer cannot be copied as a whole layer")); }
    let layer=d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let mut copies=scratch_doc(&d.doc);
    copies.layers.push(layer.clone());
    Ok(copies)
}


fn copy_mask(s: &mut Session) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let id = d.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
    let mask = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?.mask.as_ref().ok_or_else(|| EngineError::Other("the active layer has no layer mask".into()))?;
    if matches!(mask.surface.format().sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {return Err(pixel_error("authoritative U32/F64 mask copying is unsupported"));}
    let sel = d.doc.selection.as_ref();
    let area = sel.map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    if area.is_empty() {
        return Err(EngineError::Other("Could not copy: the selected area is empty".into()));
    }
    let n = mask.surface.format().channels().max(1);
    let values = crate::allocation::read_region(&mask.surface, area, "copying the mask")?;
    let len = values
        .len()
        .checked_div(n)
        .and_then(|pixels| pixels.checked_mul(2))
        .ok_or_else(|| EngineError::Other("not enough memory for copying the mask (requested size overflowed); the document was not changed".into()))?;
    let cover = sel.map(|m| (m.read_region(area), m.format().channels().max(1)));
    let mut px = crate::allocation::filled(len, 0.0, "copying the mask")?;
    for (i, p) in values.chunks_exact(n).enumerate() {
        px[i * 2] = p.first().copied().unwrap_or(0.0);
        px[i * 2 + 1] = cover.as_ref().map_or(1.0, |(m, mk)| m.get(i * mk).copied().unwrap_or(0.0));
    }
    let fmt = PixelFormat::new(photocraft_color::ColorMode::Grayscale, mask.surface.format().sample, true);
    let mut surface = Surface::new(fmt);
    surface
        .try_write_region(area, &px)
        .map_err(|e| EngineError::Other(format!("not enough memory while copying the mask ({e}); the document was not changed")))?;
    surface.prune();
    let bounds = surface.content_bounds();
    if bounds.is_empty() {
        return Err(EngineError::Other("Could not copy: the selected area is empty".into()));
    }
    let b = bounds;
    let resolution = Some(d.doc.resolution_dpi);
    s.clipboard = Some(Clip { surface, bounds: b, layers: None, resolution, color: ClipColorContext::from_document(&d.doc)?, composition: Vec::new() });
    Ok(json!({"bounds": [b.x0, b.y0, b.width(), b.height()], "target": "mask"}))
}

fn paste_layers(s: &mut Session, layers: &Document, dx: i32, dy: i32) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let mut copies = layers.clone();
    for l in &mut copies.layers {
        *l = l.duplicate();
    }
    crate::layer_copy_cmds::adapt_copies(s, &mut copies, &d.doc)?;
    crate::layer_copy_cmds::translate_copies(&mut copies, dx, dy)?;
    let patterns: Vec<_> = copies.patterns.iter().filter(|pat| !d.doc.patterns.iter().any(|q| q.id == pat.id)).cloned().collect();
    // A copied smart object may name a PSD-embedded linked file; land its `lnk` block too (#3584).
    let mut linked = Vec::new();
    for (signature, key, data) in copies.metadata.psd_global_blocks.iter().filter(|(_, k, _)| matches!(k, b"lnk2" | b"lnk3" | b"lnkD" | b"lnkE")) {
        if d.doc.metadata.psd_global_blocks.iter().any(|(_, dk, dd)| dk == key && std::sync::Arc::ptr_eq(dd, data)) {
            continue;
        }
        // The clipboard may contain a subset of a destination block. Compare complete records,
        // so copying one layer in the same document does not duplicate large embedded files.
        let missing = photocraft_io::linked::select_items(data, &|_, raw| {
            !d.doc.metadata.psd_global_blocks.iter().any(|(_, dk, dd)| dk == key && photocraft_io::linked::contains_item(dd, raw))
        })
        .map_err(|e| EngineError::Other(format!("Could not paste smart-object contents: {e}; the document was not changed")))?;
        let missing = missing.map(std::sync::Arc::new).unwrap_or_else(|| data.clone());
        if !missing.is_empty() {
            linked.try_reserve(1).map_err(|_| EngineError::Other("not enough memory to paste smart-object source records".into()))?;
            linked.push((*signature, *key, missing));
        }
    }
    let id = s.edit("Paste", |doc, active| {
        doc.patterns.extend(patterns);
        doc.metadata.psd_global_blocks.extend(linked);
        for l in copies.layers {
            *active = Some(doc.insert_above(*active, l));
        }
        doc.selection = None;
        active.ok_or(EngineError::Other("nothing to paste".into()))
    })?;
    Ok(json!({"layer": id.0, "offset": [dx, dy]}))
}

fn scratch_doc(doc: &Document) -> Document {
    let mut copies = Document::new("", doc.size, doc.mode, doc.depth);
    copies.icc_profile = doc.icc_profile.clone();
    copies.resolution_dpi = doc.resolution_dpi;
    copies.patterns = doc.patterns.clone();
    copies
}

pub const GUIDE_COLORS: [(&str, [u8; 3]); 9] = [
    ("lightBlue", [0x33, 0x66, 0xff]),
    ("lightRed", [0xff, 0x33, 0x33]),
    ("green", [0x33, 0xff, 0x33]),
    ("mediumBlue", [0x33, 0x33, 0xff]),
    ("yellow", [0xff, 0xcc, 0x00]),
    ("magenta", [0xff, 0x00, 0xff]),
    ("cyan", [0x4a, 0xff, 0xff]),
    ("lightGray", [0xe7, 0xe7, 0xe7]),
    ("black", [0x00, 0x00, 0x00]),
];

/// A guide `color`: one of [`GUIDE_COLORS`]' names or `"#rrggbb"`. `None` when there is no `color`
/// (the guide uses Preferences › Guides, Grid & Slices › Guide colour).
fn guide_color(p: &Value) -> Result<Option<[u8; 3]>> {
    let Some(v) = p.get("color") else { return Ok(None) };
    let text = v.as_str().ok_or(EngineError::Other("`color` must be a colour name or \"#rrggbb\"".into()))?;
    GUIDE_COLORS.iter().find(|(name, ..)| name.eq_ignore_ascii_case(text)).map(|(.., rgb)| *rgb).or_else(|| crate::prefs::parse_hex(text)).map(Some).ok_or_else(
        || EngineError::Other(format!("`color` {text:?} is not a guide colour: use \"#rrggbb\" or one of {}", GUIDE_COLORS.map(|c| c.0).join(", "))),
    )
}
