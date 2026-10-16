//! Edit › Free Transform / Transform: projective transforms of layers (and the selection).

use photocraft_algo::transform::{Homography, Interp};
use photocraft_color::PixelFormat;
use photocraft_doc::{Document, Layer, LayerContent, LayerId, Locks};
use photocraft_geom::{Affine, Rect};
use photocraft_raster::{Interrupt, Surface};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};
mod xcf_cubic;

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    s.active().and_then(|d| d.active_layer).map(|_| ()).ok_or_else(|| "no active layer".into())
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "edit.transform".into(), msg: msg.into() }
}

/// Document-space bounds a transform of `layer` starts from (what Free Transform frames).
/// Content scans are cached per tile: snapping asks for every layer's bounds per Move drag.
pub fn transform_bounds(doc: &Document, layer: &Layer) -> Rect {
    if let Some(source) = doc.xcf_state.as_ref().and_then(|state| {
        state.source_items.iter().find(|source| {
            source.owner == photocraft_doc::item_identity::ItemRef::Layer(layer.id)
                && matches!(source.source, photocraft_doc::xcf_items::EditableSource::Text { .. })
        })
    }) {
        let mut resident = 0_u64;
        let mut charge = |bytes| {
            resident = resident.checked_add(bytes).ok_or(photocraft_compose::gimp_composite::CompositionError::Invalid("text geometry resident overflow"))?;
            if resident > 8 << 20 {
                return Err(photocraft_compose::gimp_composite::CompositionError::Invalid("text geometry resident limit"));
            }
            Ok(())
        };
        return photocraft_compose::gimp_text_items::current_source_layout_bounds(doc, source, &photocraft_raster::Interrupt::NONE, &mut charge)
            .unwrap_or(Rect::EMPTY);
    }
    if let Some(bounds) = doc
        .xcf_state
        .as_ref()
        .and_then(|state| state.items.iter().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(layer.id)))
        .and_then(|item| item.raster_bounds)
    {
        return bounds;
    }
    let content = match &layer.content {
        LayerContent::Group(g) => g.children.iter().map(|l| transform_bounds(doc, l)).fold(Rect::EMPTY, |a, b| a.union(&b)),
        _ => layer.surface().map_or(Rect::EMPTY, photocraft_compose::bounds::content_bounds),
    };
    let content =
        if content.is_empty() { layer.mask.as_ref().map_or(Rect::EMPTY, |m| photocraft_compose::bounds::content_bounds(&m.surface)) } else { content };
    match &doc.selection {
        Some(sel) if !layer.is_group() => content.intersect(&sel.content_bounds()),
        _ => content,
    }
}

/// The box Free Transform opens on `layer`, and the Move tool's transform controls show (#5297).
/// With a selection that limits the transform it is the selection's own bounds, as in Photoshop:
/// the box and the marching ants are one rectangle, however little of the selection the layer
/// fills. Empty, like [`transform_bounds`], when the selection holds none of the layer.
pub fn transform_frame(doc: &Document, layer: &Layer) -> Rect {
    let b = transform_bounds(doc, layer);
    if moves_whole(layer) { b } else { framed_by_selection(doc, b) }
}

/// `bounds` (what a selection left of a transform's content), or the selection's own bounds.
fn framed_by_selection(doc: &Document, bounds: Rect) -> Rect {
    match &doc.selection {
        Some(sel) if !bounds.is_empty() => sel.content_bounds(),
        _ => bounds,
    }
}

/// A group moves all its descendants, so its frame must not clip raster children to the selection.
fn whole_layer_bounds(layer: &Layer) -> Rect {
    let content = match &layer.content {
        LayerContent::Group(g) => g.children.iter().map(whole_layer_bounds).fold(Rect::EMPTY, |a, b| a.union(&b)),
        _ => layer.surface().map_or(Rect::EMPTY, photocraft_compose::bounds::content_bounds),
    };
    if content.is_empty() { layer.mask.as_ref().map_or(Rect::EMPTY, |m| photocraft_compose::bounds::content_bounds(&m.surface)) } else { content }
}

/// Warp a surface whose pixels outside the content read as `default` (masks, selections):
/// warp the content with alpha, then flatten back onto the default value.
fn check_narrow_numeric(s: &Surface) -> Result<()> {
    if matches!(s.format().sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {
        return Err(bad("wide numeric-mask interpolation is not qualified"));
    }
    s.try_convert(s.format()).map_err(|e| bad(e.to_string()))?;
    photocraft_raster::checked_region_byte_len(s.content_bounds(), s.format()).map_err(|e| bad(e.to_string()))?;
    if s.channels() != 1 {
        return Err(bad("numeric-mask warp requires one channel"));
    }
    Ok(())
}

pub(crate) fn try_warp_gray(s: &Surface, h: &Homography, interp: Interp) -> Result<Surface> {
    check_narrow_numeric(s)?;
    let default = s.default_pixel().first().copied().unwrap_or(0.0);
    let fmt = s.format();
    let src = s.content_bounds();
    let mut out = Surface::with_default(fmt, &[default]);
    if src.is_empty() {
        return Ok(out);
    }
    // Content-only copy (default 0 + alpha) so the "outside" is transparent during the warp.
    let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
    let mut tmp = Surface::new(with_alpha);
    let v = s.read_region(src);
    tmp.write_region(src, &v.iter().flat_map(|g| [*g, 1.0]).collect::<Vec<f32>>());
    let w = photocraft_algo::transform::try_warp_surface(&tmp, src, h, interp).map_err(|e| bad(e.to_string()))?;
    // Clear the old content region, then composite the warped content over it. These are written as
    // two sparse regions rather than one dense `src ∪ warped` block: a transform that moves the
    // content far away (e.g. a huge translation) would otherwise allocate a buffer spanning both and
    // hang/OOM. `Surface` is tile-sparse, so distant regions cost only their own tiles.
    let old: Vec<f32> = vec![default; src.width() as usize * src.height() as usize];
    out.write_region(src, &old);
    let b = w.content_bounds();
    if !b.is_empty() {
        let px = w.read_region(b);
        let flat: Vec<f32> = px.as_chunks::<2>().0.iter().map(|p| p[0] * p[1] + default * (1.0 - p[1])).collect();
        out.write_region(b, &flat);
    }
    out.prune();
    Ok(out)
}

/// Split a single-channel surface whose outside reads as its default (a mask, a channel) by a
/// selection: (the selected values as grey + alpha = selection, the surface with the selected
/// values replaced by the default). `None` for surfaces that aren't a single grey channel.
pub fn split_gray_selected(s: &Surface, sel: &Surface) -> Option<(Surface, Surface)> {
    let fmt = s.format();
    if fmt.alpha || fmt.channels() != 1 {
        return None;
    }
    let default = s.default_pixel().first().copied().unwrap_or(0.0);
    let mut rest = s.clone();
    let mut lifted = Surface::new(PixelFormat::new(fmt.mode, fmt.sample, true));
    // Everything selected moves, including default-valued parts outside the painted content, so
    // the moved region stays aligned with the moved pixels.
    let area = sel.content_bounds();
    if area.is_empty() {
        return Some((lifted, rest));
    }
    let v = s.read_region(area);
    let w = area.width() as usize;
    let mut lp = Vec::with_capacity(v.len() * 2);
    let mut rp = Vec::with_capacity(v.len());
    for (i, g) in v.iter().enumerate() {
        let (x, y) = (area.x0 + (i % w) as i32, area.y0 + (i / w) as i32);
        let k = sel.sample_channel(x, y, 0);
        lp.extend([*g, k]);
        rp.push(g * (1.0 - k) + default * k);
    }
    lifted.write_region(area, &lp);
    lifted.prune();
    rest.write_region(area, &rp);
    rest.prune();
    Some((lifted, rest))
}

/// [`warp_gray`] limited to a selection: only the selected values move (the vacated area reads as
/// the default), as the selected pixels of the layer do.
/// Checked selected numeric-channel split; leaves source bytes untouched on error.
pub fn try_split_gray_selected(s: &Surface, sel: &Surface) -> Result<Option<(Surface, Surface)>> {
    check_narrow_numeric(s)?;
    check_narrow_numeric(sel)?;
    photocraft_raster::checked_region_byte_len(sel.content_bounds(), s.format()).map_err(|e| bad(e.to_string()))?;
    Ok(split_gray_selected(s, sel))
}

pub(crate) fn try_warp_gray_selected(s: &Surface, sel: &Surface, h: &Homography, interp: Interp) -> Result<Surface> {
    let Some((lifted, mut out)) = try_split_gray_selected(s, sel)? else { return try_warp_gray(s, h, interp) };
    let src = lifted.content_bounds();
    if src.is_empty() {
        return Ok(out);
    }
    let w = photocraft_algo::transform::try_warp_surface(&lifted, src, h, interp).map_err(|e| bad(e.to_string()))?;
    let b = w.content_bounds();
    if !b.is_empty() {
        let moved = w.read_region(b);
        let under = out.read_region(b);
        let flat: Vec<f32> = moved.as_chunks::<2>().0.iter().zip(&under).map(|(p, u)| p[0] * p[1] + u * (1.0 - p[1])).collect();
        out.write_region(b, &flat);
    }
    out.prune();
    Ok(out)
}

/// The surface Free Transform moves by itself when the params target one (`"target"`): an
/// unlinked layer mask, an alpha channel or the Quick Mask. `None` means the layer, with its
/// linked masks (a targeted *linked* mask moves together with its layer, as in Photoshop).
pub fn lone_target<'a>(doc: &'a Document, layer: Option<LayerId>, p: &Value) -> Result<Option<&'a Surface>> {
    use crate::channel_cmds::Target;
    match crate::channel_cmds::target_of(p) {
        Target::Pixels => Ok(None),
        Target::Mask => {
            let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            let m = l.mask.as_ref().ok_or_else(|| EngineError::Other("layer has no mask".into()))?;
            Ok((!m.linked).then_some(&m.surface))
        }
        Target::Alpha(i) => doc
            .channels
            .get(i)
            .map(|c| Some(&c.surface))
            .ok_or_else(|| EngineError::Other(format!("no alpha channel {i} (document has {})", doc.channels.len()))),
        Target::QuickMask => doc.quick_mask.as_ref().map(|c| Some(&c.surface)).ok_or_else(|| EngineError::Other("not in Quick Mask mode".into())),
    }
}

/// [`lone_target`], mutably.
pub fn lone_target_mut<'a>(doc: &'a mut Document, layer: Option<LayerId>, p: &Value) -> Result<Option<&'a mut Surface>> {
    if lone_target(doc, layer, p)?.is_none() {
        return Ok(None);
    }
    crate::channel_cmds::target_surface(doc, layer, p).map(|(s, _)| Some(s))
}

/// The frame Free Transform starts from on a lone target: its painted content (the whole canvas
/// when nothing is painted), within the selection.
pub fn target_bounds(doc: &Document, surf: &Surface) -> Rect {
    let content = surf.content_bounds();
    let content = if content.is_empty() { doc.bounds() } else { content };
    match &doc.selection {
        Some(sel) => content.intersect(&sel.content_bounds()),
        None => content,
    }
}

/// The box Free Transform opens on a lone target: like [`transform_frame`], the selection's own
/// bounds when there is one (#5297).
pub fn target_frame(doc: &Document, surf: &Surface) -> Rect {
    framed_by_selection(doc, target_bounds(doc, surf))
}

/// The locks `id` inherits from the groups around it.
pub(crate) fn group_locks(doc: &Document, id: LayerId) -> Result<Locks> {
    if doc.xcf_state.is_some() {
        crate::xcf_item_locks::require_layer_subtree(doc, id, crate::xcf_item_locks::EditKind::Position)?;
    }
    let path = doc.path_of(id).ok_or(EngineError::NoLayer(id))?;
    let (_, parent) = path.split_last().ok_or(EngineError::NoLayer(id))?;
    Ok(doc.locks_at(parent))
}

/// The locks a transform of `l` meets: its own and `group`'s (those it inherits). The Background's
/// position lock doesn't count: Photoshop turns it into a normal layer first.
fn transform_locks(l: &Layer, group: Locks, background: bool) -> Locks {
    let mut own = l.locks;
    if background {
        own.position = false;
        own.transparency = false;
    }
    own.union(group)
}

/// Would a transform of the layer `id` be refused because it, a group around it, or (for a group)
/// a layer inside it is locked in place? The same rule [`transform_layer`] applies at commit, so
/// Free Transform can refuse before its box opens, as Photoshop does.
pub fn locked_for_transform(doc: &Document, id: LayerId) -> bool {
    let Some(l) = doc.layer(id) else { return false };
    // An explicit stack rather than recursion: documents can nest deeply.
    let mut stack = vec![(l, group_locks(doc, id), doc.is_background(id))];
    while let Some((l, group, background)) = stack.pop() {
        let locks = transform_locks(l, group, background);
        if locks.position || locks.all {
            return true;
        }
        if let LayerContent::Group(g) = &l.content {
            stack.extend(g.children.iter().map(|c| (c, locks, false)));
        }
    }
    false
}

/// The largest result (px) a single Free Transform warp may build: 2³⁰, the bound Puppet Warp uses
/// too ([`photocraft_algo::puppet::MAX_WARP_PIXELS`]). Only the size of the result counts, not
/// where it lands, so moving a layer far off the canvas stays allowed. A scale past it is refused
/// rather than clipped (#3773): the warp would otherwise fill every tile of a result up to 2²¹ px
/// on a side before anything could fail.
pub const MAX_TRANSFORM_PIXELS: u64 = 1 << 30;

/// Tests use a far lower budget, so proving a refusal never comes near allocating the real one.
#[cfg(not(test))]
const BUDGET: u64 = MAX_TRANSFORM_PIXELS;
#[cfg(test)]
const BUDGET: u64 = 1 << 20;

/// Refuse a warp of the content inside `src` by `h` whose result would exceed the pixel budget,
/// before anything is built. `what` names the surface in the message.
pub(crate) fn check_budget(src: Rect, h: &Homography, what: &str) -> Result<()> {
    let Some(b) = warp_output_bounds(src, h) else { return Ok(()) };
    let (w, ht) = (u64::from(b.width()), u64::from(b.height()));
    if w.saturating_mul(ht) > BUDGET {
        let detail = format!("the transformed {what} would be {w} x {ht} px, more than the {BUDGET} px limit");
        return Err(crate::allocation::error("transforming pixels", &detail));
    }
    Ok(())
}

/// [`check_budget`] for every warp [`transform_layer`] would make of `l` (for a group, of every
/// layer inside it), so a transform that doesn't fit is refused before any layer is built and a
/// group that partly fits isn't half-transformed.
fn check_layer_budget(doc_sel: Option<&Surface>, l: &Layer, h: &Homography) -> Result<()> {
    if h.is_identity() {
        return Ok(());
    }
    // An explicit stack rather than recursion: documents can nest deeply.
    let mut stack = vec![(l, doc_sel)];
    while let Some((l, sel)) = stack.pop() {
        match &l.content {
            LayerContent::Group(g) => stack.extend(g.children.iter().map(|c| (c, None))),
            LayerContent::Text(_) | LayerContent::Shape(_) => {}
            LayerContent::Smart(sm) => {
                if let Some(c) = &sm.cache {
                    check_budget(c.content_bounds(), h, "smart object")?;
                }
                if let Some(m) = &sm.filter_mask {
                    check_budget(m.surface.content_bounds(), h, "smart filter mask")?;
                }
            }
            _ => {
                if let Some(surf) = l.surface() {
                    // With a selection only the lifted pixels are warped (`split_selected`).
                    let src = match sel {
                        Some(sel) => surf.content_bounds().intersect(&sel.content_bounds()),
                        None => surf.content_bounds(),
                    };
                    check_budget(src, h, &format!("layer \"{}\"", l.name))?;
                }
            }
        }
        if let Some(m) = &l.mask
            && m.linked
        {
            // With a selection only its region of the mask moves (`warp_gray_selected`).
            let src = match sel.filter(|_| !moves_whole(l)) {
                Some(sel) => sel.content_bounds(),
                None => m.surface.content_bounds(),
            };
            check_budget(src, h, "layer mask")?;
        }
    }
    Ok(())
}

/// `group` holds the locks `l` inherits from the groups around it ([`group_locks`]). A Background
/// is turned into a normal layer by the caller first ([`crate::extra_cmds::background_to_layer`]).
/// Refused, with `l` untouched, when a result would exceed [`MAX_TRANSFORM_PIXELS`].
pub(crate) fn transform_layer(doc_sel: Option<&Surface>, group: Locks, l: &mut Layer, h: &Homography, affine: Option<Affine>, interp: Interp) -> Result<()> {
    transform_layer_with_extent(doc_sel, group, l, h, (affine, interp, None), &Interrupt::NONE)
}

fn transform_layer_with_extent(
    doc_sel: Option<&Surface>,
    group: Locks,
    l: &mut Layer,
    h: &Homography,
    options: (Option<Affine>, Interp, Option<Rect>),
    control: &Interrupt<'_>,
) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    let (affine, interp, declared) = options;
    // Photoshop turns the Background into a normal layer before transforming it.
    if l.locks.position && l.name == "Background" {
        l.locks.position = false;
        l.locks.transparency = false;
        l.name = "Layer 0".into();
    }
    let locks = l.locks.union(group);
    if locks.position || locks.all {
        return Err(EngineError::Other(format!("layer \"{}\" is locked", l.name)));
    }
    // With a selection only the selected pixels move, and so only the same region of a linked
    // mask (#205). Groups, type, shapes and smart objects move whole.
    let mask_sel = doc_sel.filter(|_| !matches!(l.content, LayerContent::Group(_) | LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Smart(_)));
    match &mut l.content {
        LayerContent::Group(g) => {
            for c in g.children.iter_mut() {
                transform_layer_with_extent(None, locks, c, h, (affine, interp, None), control)?;
            }
        }
        LayerContent::Text(t) => {
            let Some(a) = affine else {
                return Err(EngineError::Other("Distort and Perspective need rasterized type (Type › Rasterize Type Layer)".into()));
            };
            t.transform = a.mul(&t.transform);
        }
        LayerContent::Shape(sh) => {
            let Some(a) = affine else {
                return Err(EngineError::Other("Distort and Perspective need a rasterized shape (Layer › Rasterize › Shape)".into()));
            };
            crate::vector_cmds::transform_shape(sh, &a);
        }
        LayerContent::Smart(sm) => {
            // Smart objects keep the transform and re-render from their source afterwards
            // (`refresh_text`), so repeated transforms don't degrade the pixels. Distort and
            // Perspective keep the full projective map, so the fourth corner survives re-renders.
            match affine {
                Some(a) => crate::smart_cmds::transform_placement(sm, &a),
                None => crate::smart_cmds::set_placement(sm, h.mul(&crate::smart_cmds::placement(sm))),
            }
            // Fallback appearance for sources that can't be re-rendered.
            if let Some(c) = &mut sm.cache {
                let src = c.content_bounds();
                *c = controlled_warp(c, src, h, interp, control)?;
            }
            if let Some(m) = &mut sm.filter_mask {
                m.surface = std::sync::Arc::new(try_warp_gray(&m.surface, h, interp)?);
            }
        }
        _ => {
            if let Some(surf) = l.surface_mut() {
                *surf = match doc_sel {
                    // Only the selected pixels move: lift them, clear them, warp and paste back. The
                    // lifted pixels' own bounds are the edge a stretch repeats (#3773).
                    Some(sel) => {
                        let (lifted, mut rest) = split_selected(surf, sel)?;
                        let moved = warp_surface_with(&lifted, lifted.content_bounds(), h, interp, ctl)?.ok_or(EngineError::Cancelled)?;
                        composite_over(&mut rest, &moved);
                        rest.prune();
                        rest
                    }
                    None => {
                        let src = surf.content_bounds();
                        warp_surface_with(surf, src, h, interp, ctl)?.ok_or(EngineError::Cancelled)?
                    }
                };
            }
        }
    }
    if let Some(m) = l.mask.as_mut()
        && m.linked
    {
        m.surface = std::sync::Arc::new(match mask_sel {
            Some(sel) => try_warp_gray_selected(&m.surface, sel, h, interp)?,
            None => try_warp_gray(&m.surface, h, interp)?,
        });
    }
    if let Some(vm) = l.vector_mask.as_mut()
        && vm.linked
        && let Some(a) = affine
    {
        vm.path = vm.path.transform(&a);
    }
    Ok(())
}

fn controlled_warp(src: &Surface, bounds: Rect, h: &Homography, interp: Interp, control: &Interrupt<'_>) -> Result<Surface> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    let result = if interp == Interp::Nearest && matches!(src.format().mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale) {
        photocraft_algo::transform::try_warp_nearest_surface_with_limits(src, bounds, h, photocraft_algo::wide_geometry::GeometryLimits::default(), control)
    } else if matches!(src.format().sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {
        photocraft_algo::transform::try_warp_surface_with_limits(src, bounds, h, interp, photocraft_algo::wide_geometry::GeometryLimits::default(), control)
    } else {
        photocraft_algo::transform::try_warp_surface(src, bounds, h, interp)
    };
    control.check().map_err(|_| EngineError::Cancelled)?;
    result.map_err(|e| EngineError::Other(e.to_string()))
}

/// Split a layer surface by a selection: (selected pixels, everything else), both with alpha.
/// Checked selected split. Wide selection compositing requires a qualified f64
/// evaluator and is explicitly rejected before any authoritative samples change.
pub fn try_split_selected(surf: &Surface, sel: &Surface) -> Result<(Surface, Surface)> {
    check_narrow_numeric(sel)?;
    if matches!(surf.format().sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {
        return Err(crate::EngineError::Other("wide selected transform is not qualified".into()));
    }
    let fmt = surf.format();
    let prepared = surf.try_convert(PixelFormat::new(fmt.mode, fmt.sample, true)).map_err(|e| crate::EngineError::Other(format!("pixel conversion: {e}")))?;
    Ok(split_selected(&prepared, sel))
}

/// Legacy presentation split; authoritative command callers use `try_split_selected`.
pub fn split_selected(surf: &Surface, sel: &Surface) -> (Surface, Surface) {
    let fmt = surf.format();
    let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
    let mut lifted = Surface::new(with_alpha);
    let mut rest = surf.convert(with_alpha);
    let src = surf.content_bounds();
    if src.is_empty() {
        return (lifted, rest);
    }
    let n = with_alpha.channels();
    let px = rest.read_region(src);
    let mut lp = px.clone();
    let mut rp = px;
    let w = src.width() as usize;
    for (i, (l, r)) in lp.chunks_exact_mut(n).zip(rp.chunks_exact_mut(n)).enumerate() {
        let (x, y) = (src.x0 + (i % w) as i32, src.y0 + (i / w) as i32);
        let k = sel.sample_channel(x, y, 0);
        l[n - 1] *= k;
        r[n - 1] *= 1.0 - k;
        // Likewise an unselected pixel leaves nothing in the lifted surface, so its bounds are the
        // selected pixels' (what a stretch repeats at the edge, #3773).
        if l[n - 1] <= 0.0 {
            l.fill(0.0);
        }
        if r[n - 1] <= 0.0 {
            r.iter_mut().zip(&rest_clear).for_each(|(d, c)| *d = *c);
        }
    }
    lifted.write_region(src, &lp);
    lifted.prune();
    rest.write_region(src, &rp);
    (lifted, rest)
}

pub(crate) fn refresh_text(doc: &Document, l: &mut Layer) -> Result<()> {
    match &mut l.content {
        LayerContent::Text(t) => crate::type_cmds::try_refresh(doc, t)?,
        LayerContent::Shape(sh) => crate::vector_cmds::refresh_shape(doc, sh),
        LayerContent::Smart(_) => {
            // Unavailable sources keep the warped cache.
            #[cfg(target_arch = "wasm32")]
            crate::type_cmds::text_engine()?;
            let _ = crate::smart_cmds::refresh_layer(doc, l);
        }
        LayerContent::Group(g) => {
            for c in &mut g.children {
                refresh_text(doc, c)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Normal-blend `top` over `dst` (both straight alpha, same format).
pub(crate) fn composite_over(dst: &mut Surface, top: &Surface) {
    let b = top.content_bounds();
    if b.is_empty() {
        return;
    }
    let n = dst.format().channels();
    let a = n - 1;
    let t = top.read_region(b);
    let mut d = dst.read_region(b);
    for (dp, tp) in d.chunks_exact_mut(n).zip(t.chunks_exact(n)) {
        let (ta, da) = (tp[a], dp[a]);
        let oa = ta + da * (1.0 - ta);
        if oa > 0.0 {
            for c in 0..a {
                dp[c] = (tp[c] * ta + dp[c] * da * (1.0 - ta)) / oa;
            }
        }
        dp[a] = oa;
    }
    dst.write_region(b, &d);
}

fn quad_param(p: &Value) -> Option<[[f64; 2]; 4]> {
    let a = p.get("quad")?.as_array()?;
    if a.len() != 4 {
        return None;
    }
    let mut q = [[0.0; 2]; 4];
    for (i, v) in a.iter().enumerate() {
        let c = v.as_array()?;
        q[i] = [c.first()?.as_f64()?, c.get(1)?.as_f64()?];
    }
    Some(q)
}

fn canonical_raster_transform_bounds(doc: &Document, id: LayerId) -> Result<Option<Rect>> {
    use photocraft_doc::item_identity::ItemRef;
    let Some(state) = &doc.xcf_state else { return Ok(None) };
    let owner = ItemRef::Layer(id);
    let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if !matches!(layer.content, LayerContent::Raster(_))
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || doc.selection.is_some()
        || state.source_items.iter().any(|source| source.owner == owner)
        || state.editable_indexed.iter().any(|indexed| indexed.owner == id)
        || state.filter_stacks.iter().any(|stack| stack.owner == owner && !stack.nodes.is_empty())
        || state.floating.as_ref().is_some_and(|floating| floating.layer == id || floating.target == owner)
    {
        return Err(bad("canonical transform currently requires an ordinary Raster without selection, masks, indexed/source/filter or floating relationships"));
    }
    let bounds =
        state.items.iter().find(|item| item.item == owner).and_then(|item| item.raster_bounds).ok_or_else(|| bad("canonical raster has no declared extent"))?;
    Ok(Some(bounds))
}

fn transform(s: &mut Session, p: &Value) -> Result<Value> {
    let params = with_panel_target(s, p);
    transform_with_interrupt(s, &params, &Interrupt::NONE)
}

/// Controlled transform staging. Command registry dispatch currently uses no token.
/// Wide checked sampling polls this token; legacy narrow stages check at stage boundaries.
pub fn transform_with_interrupt(s: &mut Session, p: &Value, control: &Interrupt<'_>) -> Result<Value> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = match p.get("layer").and_then(Value::as_u64) {
        Some(v) => Some(LayerId(v)),
        None => st.active_layer,
    };
    let lone = lone_target(&st.doc, id, p)?;
    if lone.is_some() && st.doc.xcf_state.is_some() {
        return Err(bad("canonical mask/channel transforms require separately qualified geometry"));
    }
    let id = match (id, lone.is_some() && crate::channel_cmds::is_channel_target(p)) {
        (_, true) => None,
        (Some(id), false) => Some(id),
        (None, false) => return Err(EngineError::Other("no active layer".into())),
    };
    let rect = match p.get("rect").and_then(Value::as_array) {
        Some(r) if r.len() == 4 => {
            let v: Vec<f64> = r.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect();
            [v[0], v[1], v[2], v[3]]
        }
        _ => {
            let b = match (lone, id) {
                (Some(surf), _) => target_bounds(&st.doc, surf),
                (None, Some(id)) => transform_bounds(&st.doc, st.doc.layer(id).ok_or(EngineError::NoLayer(id))?),
                (None, None) => Rect::EMPTY,
            };
            if b.is_empty() {
                return Err(EngineError::Other("nothing to transform".into()));
            }
            [b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64]
        }
    };
    let h = if let Some(q) = quad_param(p) {
        Homography::rect_to_quad(rect, q).ok_or_else(|| bad("degenerate quad"))?
    } else if let Some(m) = p.get("matrix").and_then(Value::as_array) {
        let v: Vec<f64> = m.iter().filter_map(Value::as_f64).collect();
        if v.len() != 6 {
            return Err(bad("matrix must be [a, b, c, d, e, f]"));
        }
        // Affine [a b c d e f] maps (x, y) → (a·x + c·y + e, b·x + d·y + f).
        Homography([v[0], v[2], v[4], v[1], v[3], v[5], 0.0, 0.0, 1.0])
    } else {
        relative_matrix(p, rect)?.ok_or_else(|| bad("pass quad, matrix or relative scale/rotate/move parameters"))?
    };
    if h.inverse().is_none() {
        return Err(bad("the transform collapses the layer"));
    }
    let m = h.0;
    let affine =
        (m[6].abs() < 1e-12 && m[7].abs() < 1e-12).then(|| Affine { m: [m[0] / m[8], m[3] / m[8], m[1] / m[8], m[4] / m[8], m[2] / m[8], m[5] / m[8]] });
    let interp = Interp::parse(p.get("interpolation").and_then(Value::as_str).unwrap_or("bicubic"));
    let canonical = if lone.is_none() { id.map(|id| canonical_raster_transform_bounds(&st.doc, id)).transpose()?.flatten() } else { None };
    let moved_extent = canonical.map(|bounds| photocraft_algo::transform::try_warp_bounds(bounds, &h).map_err(|e| bad(e.to_string()))).transpose()?;
    let linear_sampling = p.get("interpolation").and_then(Value::as_str) == Some("gimpLinear");
    let scoped_cubic = linear_sampling || p.get("interpolation").and_then(Value::as_str) == Some("gimpCubic");
    let staged_cubic = if scoped_cubic {
        if lone.is_some() {
            return Err(bad("Cubic compatibility does not qualify numeric targets"));
        }
        let id = id.ok_or_else(|| bad("Cubic compatibility needs a raster owner"))?;
        let bounds = canonical.ok_or_else(|| bad("Cubic compatibility needs a canonical declared Raster extent"))?;
        let layer = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let locks = layer.locks.union(group_locks(&st.doc, id)?);
        if locks.position || locks.all {
            return Err(bad("Cubic owner position is locked"));
        }
        let LayerContent::Raster(source) = &layer.content else {
            return Err(bad("Cubic compatibility requires an ordinary Raster"));
        };
        Some(xcf_cubic::warp(&st.doc, source, bounds, &h, linear_sampling, control)?)
    } else {
        None
    };
    let lone = lone.is_some();
    s.edit_with_interrupt("Free Transform", control, |doc, _| {
        let sel = doc.selection.clone();
        if lone {
            // A targeted unlinked mask, alpha channel or Quick Mask transforms by itself.
            let (surf, _) = crate::channel_cmds::target_surface(doc, id, p)?;
            *surf = match &sel {
                Some(sel) => try_warp_gray_selected(surf, sel, &h, interp)?,
                None => try_warp_gray(surf, &h, interp)?,
            };
            if let Some(sel) = &doc.selection {
                doc.selection = Some(try_warp_gray(sel, &h, Interp::Bilinear)?).filter(|s| !s.content_bounds().is_empty());
            }
            return Ok(());
        }
        let id = id.ok_or_else(|| EngineError::Other("no active layer".into()))?;
        let is_group = doc.layer(id).is_some_and(Layer::is_group);
        let group = group_locks(doc, id)?;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if let Some(surface) = staged_cubic {
            *l.surface_mut().ok_or_else(|| bad("Cubic raster owner changed"))? = surface;
        } else {
            transform_layer_with_extent(if is_group { None } else { sel.as_ref() }, group, l, &h, (affine, interp, canonical), control)?;
        }
        if let Some(bounds) = moved_extent {
            let item = doc
                .xcf_state
                .as_mut()
                .and_then(|state| state.items.iter_mut().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(id)))
                .ok_or_else(|| bad("missing canonical raster owner"))?;
            item.raster_bounds = Some(bounds);
            if scoped_cubic {
                xcf_cubic::stamp_finite_normal_if_needed(doc, id, bounds, control)?;
            }
        }
        // Type layers re-render from their new transform.
        let snapshot = doc.clone();
        if let Some(l) = doc.layer_mut(id) {
            refresh_text(&snapshot, l)?;
        }
        // The selection outline moves with the pixels.
        if let Some(sel) = &doc.selection {
            doc.selection = Some(try_warp_gray(sel, &h, Interp::Bilinear)?).filter(|s| !s.content_bounds().is_empty());
        }
        Ok(())
    })?;
    Ok(json!({"rect": rect}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "edit.transform",
        label: "Free Transform",
        menu: &[],
        shortcut: None,
        params: r##"{"layer":id?,"rect":[x0,y0,x1,y1]? (source frame; default = layer content ∩ selection),"quad":[[x,y]×4]? (where the frame's corners go, clockwise from top-left),"matrix":[a,b,c,d,e,f]? (affine alternative),"interpolation":"bicubic|bilinear|nearest|gimpCubic|gimpLinear"="bicubic" (gimpCubic/gimpLinear = independently qualified bounded implicit RGB/Gray translation and explicit reference geometry only),"target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target (an unlinked mask, an alpha channel or the Quick Mask transforms alone; a linked mask moves with its layer)}"##,
        enabled: has_layer,
        journal: true,
        run: transform,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            let l = doc.layer_mut(active.unwrap()).unwrap();
            l.surface_mut().unwrap().fill_rect(Rect::new(10, 10, 30, 20), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s
    }

    fn active_bounds(s: &Session) -> Rect {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds()
    }

    #[test]
    fn scale_via_quad_and_undo() {
        let mut s = session();
        s.execute("edit.transform", json!({"quad": [[10, 10], [50, 10], [50, 30], [10, 30]]})).unwrap();
        let b = active_bounds(&s);
        assert!(b.width().abs_diff(40) <= 2 && b.height().abs_diff(20) <= 2, "{b:?}");
        s.undo();
        assert_eq!(active_bounds(&s), Rect::new(10, 10, 30, 20));
    }

    #[test]
    fn affine_matrix_and_perspective() {
        let mut s = session();
        // Translate by (40, 50).
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 40, 50]})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(50, 60, 70, 70));
        // Perspective: top edge narrower than the bottom.
        s.execute("edit.transform", json!({"quad": [[55, 60], [65, 60], [75, 70], [45, 70]]})).unwrap();
        let b = active_bounds(&s);
        assert!(b.x0 <= 46 && b.x1 >= 74, "{b:?}");
        assert!(s.execute("edit.transform", json!({"quad": [[0, 0], [0, 0], [0, 0], [0, 0]]})).is_err());
    }

    #[test]
    fn background_becomes_layer_0() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
        s.execute("edit.transform", json!({"quad": [[0, 0], [20, 0], [20, 20], [0, 20]]})).unwrap();
        let st = s.active().unwrap();
        let l = &st.doc.layers[0];
        assert_eq!(l.name, "Layer 0");
        assert!(l.surface().unwrap().format().alpha);
        assert_eq!(l.surface().unwrap().pixel(30, 30)[3], 0.0, "revealed area is transparent");
    }

    #[test]
    fn with_selection_only_selected_pixels_move() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 50]})).unwrap();
        let st = s.active().unwrap();
        let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        assert_eq!(surf.pixel(15, 15)[3], 0.0, "lifted area is cleared");
        assert_eq!(surf.pixel(25, 15)[3], 1.0, "unselected part stays");
        assert_eq!(surf.pixel(15, 65)[3], 1.0, "moved pixels land");
        let sel = st.doc.selection.as_ref().unwrap().content_bounds();
        assert_eq!((sel.y0, sel.y1), (60, 70), "selection moves too");
    }

    #[test]
    fn type_layers_transform_as_vectors() {
        let mut s = session();
        let r = s.execute("type.create", json!({"x": 10, "y": 50, "text": "Hi", "size": 20})).unwrap();
        let id = r["layer"].as_u64().unwrap();
        let w0 = active_bounds(&s).width();
        s.execute("edit.transform", json!({"layer": id, "matrix": [2, 0, 0, 2, -10, -50]})).unwrap();
        let w1 = active_bounds(&s).width();
        assert!(w1 as f32 > w0 as f32 * 1.8, "{w0} → {w1}");
        let st = s.active().unwrap();
        let LayerContent::Text(t) = &st.doc.layer(LayerId(id)).unwrap().content else { panic!() };
        assert!((t.transform.m[0] - 2.0).abs() < 1e-9);
        // Perspective on type is refused, like Photoshop.
        assert!(s.execute("edit.transform", json!({"layer": id, "quad": [[0, 0], [10, 0], [12, 10], [-2, 10]]})).is_err());
    }

    #[test]
    fn shapes_move_and_transform_as_vectors() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 20], "fill": "#ff0000"})).unwrap();
        s.execute("layer.translate", json!({"dx": 30, "dy": 5})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(40, 15, 80, 35));
        s.execute("edit.transform", json!({"matrix": [2, 0, 0, 2, -40, -15]})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(40, 15, 120, 55));
        assert!(s.execute("edit.transform", json!({"quad": [[40, 15], [120, 15], [130, 55], [30, 55]]})).is_err());
    }

    #[test]
    fn type_moves_with_the_move_command() {
        let mut s = session();
        s.execute("type.create", json!({"x": 10, "y": 50, "text": "Hi", "size": 20})).unwrap();
        let b0 = active_bounds(&s);
        s.execute("layer.translate", json!({"dx": 20, "dy": 10})).unwrap();
        let b1 = active_bounds(&s);
        assert_eq!((b1.x0 - b0.x0, b1.y0 - b0.y0), (20, 10));
    }

    // ---- Layer masks and targets (#205) ----

    /// `session()` plus a reveal-all mask with `hide` painted black (linked unless `linked` is false).
    fn masked(hide: &[Rect], linked: bool) -> Session {
        let mut s = session();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.edit("mask", |doc, active| {
            let m = doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap();
            for r in hide {
                std::sync::Arc::make_mut(&mut m.surface).fill_rect(*r, &[0.0]);
            }
            m.linked = linked;
            Ok(())
        })
        .unwrap();
        s
    }

    fn mask_at(s: &Session, x: i32, y: i32) -> f32 {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, y, 0)
    }

    fn alpha_at(s: &Session, x: i32, y: i32) -> f32 {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)[3]
    }

    #[test]
    fn linked_raster_mask_moves_with_the_pixels() {
        let mut s = masked(&[Rect::new(10, 10, 20, 20)], true);
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 40, 50]})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(50, 60, 70, 70));
        assert!(mask_at(&s, 55, 65) < 0.01, "the hidden half moved with the pixels");
        assert!(mask_at(&s, 65, 65) > 0.99);
        assert!(mask_at(&s, 15, 15) > 0.99, "nothing hidden left behind");
    }

    #[test]
    fn linked_mask_with_a_selection_moves_only_the_selected_region() {
        // The selection covers the left half of the red rect; the mask hides its left quarter
        // (selected) and its right quarter (not selected).
        let mut s = masked(&[Rect::new(10, 10, 15, 20), Rect::new(25, 10, 30, 20)], true);
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 50]})).unwrap();
        // Moved pixels and their mask line up.
        assert_eq!(alpha_at(&s, 12, 65), 1.0);
        assert!(mask_at(&s, 12, 65) < 0.01, "selected hidden quarter moved");
        assert!(mask_at(&s, 17, 65) > 0.99, "selected visible quarter moved");
        // The vacated area reads as revealed (default), with transparent pixels.
        assert_eq!(alpha_at(&s, 12, 15), 0.0);
        assert!(mask_at(&s, 12, 15) > 0.99);
        // The unselected part of the mask stays, aligned with the unselected pixels.
        assert_eq!(alpha_at(&s, 27, 15), 1.0);
        assert!(mask_at(&s, 27, 15) < 0.01, "unselected mask stays put");
        assert!(mask_at(&s, 27, 65) > 0.99, "and isn't copied along");
        // Every pixel's mask value is where it was relative to the pixel.
        for (x, y) in [(11, 61), (14, 69), (15, 61), (19, 69)] {
            let want = if x < 15 { 0.0 } else { 1.0 };
            assert!((mask_at(&s, x, y) - want).abs() < 0.01, "({x},{y})");
        }
    }

    #[test]
    fn unlinked_mask_stays_put() {
        let mut s = masked(&[Rect::new(10, 10, 20, 20)], false);
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 40, 50]})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(50, 60, 70, 70));
        assert!(mask_at(&s, 15, 15) < 0.01);
        assert!(mask_at(&s, 55, 65) > 0.99);
        // With a selection too.
        let mut s = masked(&[Rect::new(10, 10, 20, 20)], false);
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 50]})).unwrap();
        assert!(mask_at(&s, 15, 15) < 0.01 && mask_at(&s, 15, 65) > 0.99);
    }

    #[test]
    fn targeted_unlinked_mask_transforms_alone() {
        let mut s = masked(&[Rect::new(10, 10, 20, 20)], false);
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 40, 50], "target": "mask"})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(10, 10, 30, 20), "pixels untouched");
        assert!(mask_at(&s, 55, 65) < 0.01, "mask moved");
        assert!(mask_at(&s, 15, 15) > 0.99, "vacated area reads as the default");
        s.undo();
        assert!(mask_at(&s, 15, 15) < 0.01, "one undo step");
        // With a selection, only its part of the mask.
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 5, "height": 10})).unwrap();
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 50], "target": "mask"})).unwrap();
        assert!(mask_at(&s, 12, 65) < 0.01 && mask_at(&s, 12, 15) > 0.99);
        assert!(mask_at(&s, 17, 15) < 0.01, "unselected mask stays");
        assert_eq!(active_bounds(&s), Rect::new(10, 10, 30, 20), "pixels untouched");
    }

    #[test]
    fn targeted_linked_mask_moves_with_its_layer() {
        // Photoshop: a linked mask can't move without its layer.
        let mut s = masked(&[Rect::new(10, 10, 20, 20)], true);
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 40, 50], "target": "mask"})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(50, 60, 70, 70));
        assert!(mask_at(&s, 55, 65) < 0.01);
    }

    #[test]
    fn targeted_alpha_channel_transforms_alone() {
        let mut s = session();
        s.execute("channel.new", json!({})).unwrap();
        s.edit("paint channel", |doc, _| {
            doc.channels[0].surface.fill_rect(Rect::new(10, 10, 20, 20), &[1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 40, 50], "target": {"channel": 0}})).unwrap();
        let st = s.active().unwrap();
        let ch = &st.doc.channels[0].surface;
        assert!(ch.sample_channel(55, 65, 0) > 0.99 && ch.sample_channel(15, 15, 0) < 0.01);
        assert_eq!(active_bounds(&s), Rect::new(10, 10, 30, 20), "layer untouched");
    }

    #[test]
    fn transform_targets_fail_gracefully() {
        let mut s = session();
        let m = json!([1, 0, 0, 1, 5, 5]);
        assert!(s.execute("edit.transform", json!({"matrix": m, "target": "mask"})).is_err(), "no mask");
        assert!(s.execute("edit.transform", json!({"matrix": m, "target": {"channel": 9}})).is_err());
        assert!(s.execute("edit.transform", json!({"matrix": m, "target": "quickMask"})).is_err());
        // Unknown target shapes fall back to the pixels.
        s.execute("edit.transform", json!({"matrix": m, "target": 7})).unwrap();
        s.execute("edit.transform", json!({"matrix": m, "target": {"channel": "x"}})).unwrap();
        // Selections that miss the mask entirely transform nothing, without panicking.
        let mut s = masked(&[Rect::new(10, 10, 20, 20)], false);
        s.execute("select.rect", json!({"x": 80, "y": 80, "width": 5, "height": 5})).unwrap();
        let _ = s.execute("edit.transform", json!({"matrix": [1e9, 0, 0, 1e9, 0, 0], "target": "mask"}));
        let _ = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 1e12, 0], "target": "mask"}));
    }

    #[test]
    fn split_gray_selected_rejects_non_gray() {
        let s = Surface::new(PixelFormat::RGBA8);
        let mut sel = Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 4, 4), &[1.0]);
        assert!(matches!(split_gray_selected(&s, &sel), Ok(None)));
    }

    /// #2630: type can't be partly transformed, so a selection doesn't narrow the frame: Free
    /// Transform (and the frame-based presets) act on the whole type layer.
    #[test]
    fn a_selection_does_not_narrow_a_type_layers_frame() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 120})).unwrap();
        s.execute("type.create", json!({"x": 20, "y": 60, "text": "Hello", "size": 36})).unwrap();
        let whole = active_bounds(&s);
        s.execute("select.rect", json!({"x": whole.x0, "y": whole.y0, "width": whole.width() / 2, "height": whole.height()})).unwrap();
        let st = s.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert_eq!(transform_bounds(&st.doc, l), whole, "the frame is the whole type layer");
        // Corners of the frame moved 10 px right: a pure translation of the whole text.
        let q = [[whole.x0 + 10, whole.y0], [whole.x1 + 10, whole.y0], [whole.x1 + 10, whole.y1], [whole.x0 + 10, whole.y1]];
        s.execute("edit.transform", json!({"quad": q})).unwrap();
        let moved = active_bounds(&s);
        assert!(moved.x0.abs_diff(whole.x0 + 10) <= 1 && moved.width().abs_diff(whole.width()) <= 1, "{whole:?} -> {moved:?}");
        // Rotate 180° turns it about its own centre, not the selection's.
        s.execute("edit.transform.rotate180", json!({})).unwrap();
        let turned = active_bounds(&s);
        assert!(turned.x0.abs_diff(moved.x0) <= 2 && turned.x1.abs_diff(moved.x1) <= 2, "{moved:?} -> {turned:?}");
        // A pixel layer still frames only the selected pixels.
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 5, "height": 10})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(transform_bounds(&st.doc, st.doc.layer(st.active_layer.unwrap()).unwrap()), Rect::new(10, 10, 15, 20));
    }

    /// #5297: the box Free Transform opens with a selection is the selection's own rectangle,
    /// not the part of it the layer fills; type still moves whole, and a selection that holds none
    /// of the layer leaves nothing to transform.
    #[test]
    fn the_frame_is_the_selections_own_bounds() {
        let mut s = session();
        let frame = |s: &Session| {
            let st = s.active().unwrap();
            let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
            (transform_frame(&st.doc, l), transform_bounds(&st.doc, l))
        };
        let content = Rect::new(10, 10, 30, 20);
        assert_eq!(frame(&s), (content, content), "no selection: the content");
        s.execute("select.rect", json!({"x": 5, "y": 5, "width": 15, "height": 40})).unwrap();
        assert_eq!(frame(&s), (Rect::new(5, 5, 20, 45), Rect::new(10, 10, 20, 20)));
        s.execute("select.rect", json!({"x": 60, "y": 60, "width": 10, "height": 10})).unwrap();
        assert!(frame(&s).0.is_empty(), "nothing of the layer is selected");

        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 120})).unwrap();
        s.execute("type.create", json!({"x": 20, "y": 60, "text": "Hello", "size": 36})).unwrap();
        let whole = active_bounds(&s);
        s.execute("select.rect", json!({"x": whole.x0, "y": whole.y0, "width": whole.width() / 2, "height": whole.height()})).unwrap();
        assert_eq!(frame(&s), (whole, whole), "type moves whole");
    }

    #[test]
    fn group_transform_ignores_selections_inside_its_children() {
        group_transform_with_selection(Rect::new(10, 10, 15, 15));
    }

    #[test]
    fn group_transform_ignores_selections_outside_its_children() {
        group_transform_with_selection(Rect::new(70, 70, 80, 80));
    }

    #[test]
    fn group_transform_presets_rotate_about_the_whole_group() {
        for depth in [8, 16, 32] {
            for x in [10, 70] {
                let mut s = masked_depth(&[Rect::new(10, 10, 15, 20)], true, depth);
                let child = s.active().unwrap().active_layer.unwrap();
                s.execute("layer.groupLayers", json!({})).unwrap();
                s.execute("select.rect", json!({"x": x, "y": 10, "width": 5, "height": 5})).unwrap();
                s.execute("edit.transform.rotate180", json!({})).unwrap();
                let layer = s.active().unwrap().doc.layer(child).unwrap();
                assert_eq!(layer.surface().unwrap().content_bounds(), Rect::new(10, 10, 30, 20), "rotate about the group's centre, not the selection's");
                let mask = &layer.mask.as_ref().unwrap().surface;
                assert_eq!(mask.sample_channel(27, 15, 0), 0.0);
                assert_eq!(mask.sample_channel(12, 15, 0), 1.0);
            }
        }
    }

    fn group_transform_with_selection(selection: Rect) {
        for depth in [8, 16, 32] {
            for nested in [false, true] {
                let mut s = masked_depth(&[Rect::new(10, 10, 15, 20)], true, depth);
                let child = s.active().unwrap().active_layer.unwrap();
                s.execute("layer.groupLayers", json!({})).unwrap();
                if nested {
                    s.execute("layer.groupLayers", json!({})).unwrap();
                }
                s.execute("select.rect", json!({"x": selection.x0, "y": selection.y0, "width": selection.width(), "height": selection.height()})).unwrap();
                let steps = s.active().unwrap().history.entries().len();
                let result = s.execute("edit.transform", json!({"quad": [[10, 10], [50, 10], [50, 30], [10, 30]], "interpolation": "nearest"})).unwrap();
                assert_eq!(result["rect"], json!([10.0, 10.0, 30.0, 20.0]), "frame must include the whole group (nested={nested})");
                let st = s.active().unwrap();
                let layer = st.doc.layer(child).unwrap();
                assert_eq!(layer.surface().unwrap().content_bounds(), Rect::new(10, 10, 50, 30));
                let mask = &layer.mask.as_ref().unwrap().surface;
                assert_eq!(mask.sample_channel(19, 20, 0), 0.0, "the linked mask scales with its complete layer");
                assert_eq!(mask.sample_channel(21, 20, 0), 1.0);
                assert_eq!(st.history.entries().len(), steps + 1);
                assert!(s.undo());
                let st = s.active().unwrap();
                let layer = st.doc.layer(child).unwrap();
                assert_eq!(layer.surface().unwrap().content_bounds(), Rect::new(10, 10, 30, 20));
                assert_eq!(layer.mask.as_ref().unwrap().surface.sample_channel(19, 15, 0), 1.0);
                assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), selection);
                assert!(s.redo());
                assert_eq!(s.active().unwrap().doc.layer(child).unwrap().surface().unwrap().content_bounds(), Rect::new(10, 10, 50, 30));
            }
        }
    }

    #[test]
    fn feathered_selection_identity_transform_preserves_pixels() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "background": "transparent"})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.rect", json!({"x": 4, "y": 4, "width": 8, "height": 8})).unwrap();
        s.execute("select.modify.feather", json!({"radius": 1})).unwrap();

        let mut before_px = Vec::new();
        for y in 0..16 {
            for x in 0..16 {
                before_px.push(s.execute("document.pixel", json!({"x": x, "y": y})).unwrap());
            }
        }

        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 0], "interpolation": "nearest"})).unwrap();

        let mut after_px = Vec::new();
        for y in 0..16 {
            for x in 0..16 {
                after_px.push(s.execute("document.pixel", json!({"x": x, "y": y})).unwrap());
            }
        }

        assert_eq!(before_px, after_px, "feathered selection identity transform must preserve all pixels");
        let p4_8 = s.execute("document.pixel", json!({"x": 4, "y": 8})).unwrap();
        assert_eq!(p4_8, json!([1.0, 0.0, 0.0, 1.0]), "(4,8) must remain opaque red");

        // Undo restores original document.
        s.execute("edit.undo", json!({})).unwrap();
        let undo_p4_8 = s.execute("document.pixel", json!({"x": 4, "y": 8})).unwrap();
        assert_eq!(undo_p4_8, json!([1.0, 0.0, 0.0, 1.0]));

        // Locked layer still refuses transform even if identity.
        s.execute("layer.setProps", json!({"locks": {"position": true}})).unwrap();
        assert!(s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 0]})).is_err(), "locked layer refuses identity transform");
    }

    /// A 1 x 8 px column on a 256 x 256 canvas (#3773).
    fn column_session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 256, "height": 256})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            let l = doc.layer_mut(active.unwrap()).unwrap();
            l.surface_mut().unwrap().fill_rect(Rect::new(100, 100, 101, 108), &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s
    }

    fn opaque_green(s: &Session, x: i32, y: i32) -> bool {
        let st = s.active().unwrap();
        let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        (surf.sample_channel(x, y, 1) - 1.0).abs() < 1e-3 && (surf.sample_channel(x, y, 3) - 1.0).abs() < 1e-3
    }

    /// The "pixel stretch" (#3773): scales far past 1000% are taken, and the stretched column stays
    /// solid along its whole length.
    #[test]
    fn a_one_pixel_column_stretches_5000_percent() {
        let mut s = column_session();
        s.execute("edit.transform", json!({"quad": [[100, 100], [150, 100], [150, 108], [100, 108]]})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(100, 100, 150, 108));
        for x in [100, 101, 125, 149] {
            for y in 100..108 {
                assert!(opaque_green(&s, x, y), "({x}, {y})");
            }
        }
    }

    /// The technique as done in Photoshop: select a 1 px column of an image and stretch it across.
    #[test]
    fn a_selected_column_stretches_solid_over_the_image() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 256, "height": 256})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            surf.fill_rect(Rect::new(0, 0, 256, 256), &[1.0, 0.0, 0.0, 1.0]);
            surf.fill_rect(Rect::new(10, 0, 11, 256), &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 0, "width": 1, "height": 256})).unwrap();
        s.execute("edit.transform", json!({"quad": [[10, 0], [250, 0], [250, 256], [10, 256]]})).unwrap();
        for x in [10, 11, 100, 200, 249] {
            assert!(opaque_green(&s, x, 128), "x = {x}");
        }
        let st = s.active().unwrap();
        let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        assert!(surf.sample_channel(250, 128, 0) > 0.999 && surf.sample_channel(9, 128, 0) > 0.999, "the image either side is untouched");
    }

    /// Free Transform keeps pixels moved off the canvas, as Photoshop does: an 8 x 8 layer moved
    /// three canvas widths right and back comes back whole.
    #[test]
    fn a_layer_moved_far_off_the_canvas_and_back_keeps_its_pixels() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for y in 0..8 {
                for x in 0..8 {
                    surf.fill_rect(Rect::new(20 + x, 20 + y, 21 + x, 21 + y), &[x as f32 / 8.0, y as f32 / 8.0, 0.5, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
        let pixels = |s: &Session| {
            let st = s.active().unwrap();
            let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
            (0..8 * 8 * 4).map(|i| surf.sample_channel(20 + i / 4 % 8, 20 + i / 32, (i % 4) as usize)).collect::<Vec<_>>()
        };
        let before = pixels(&s);
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 192, 0]})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(212, 20, 220, 28));
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, -192, 0]})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(20, 20, 28, 28));
        assert_eq!(pixels(&s), before);
    }

    #[test]
    fn relative_params_scale_and_rotate_about_the_frame_centre() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 20, "y": 30, "width": 40, "height": 20})).unwrap();
        s.execute("edit.fill", json!({"contents": "black"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        // 50% about the centre (40, 40), then 10 px right: the content ends at x 40..60, y 35..45.
        let r = s.execute("edit.transform", json!({"scaleX": 50, "scaleY": 50, "dx": 10})).unwrap();
        let rect: Vec<f64> = serde_json::from_value(r["rect"].clone()).unwrap();
        assert_eq!(rect, [20.0, 30.0, 60.0, 50.0], "the reported frame is the source frame");
        let bounds = |s: &Session| {
            let d = s.active().unwrap();
            let b = transform_bounds(&d.doc, d.doc.layer(d.active_layer.unwrap()).unwrap());
            (b.x0, b.y0, b.x1, b.y1)
        };
        assert_eq!(bounds(&s), (40, 35, 60, 45));
        // A quarter turn about the centre (50, 40) swaps the frame's sides.
        s.execute("edit.transform", json!({"rotate": 90})).unwrap();
        assert_eq!(bounds(&s), (45, 30, 55, 50));
        for bad in [json!({"scaleX": 0}), json!({"rotate": "x"}), json!({"dx": f64::NAN})] {
            assert!(s.execute("edit.transform", bad.clone()).is_err(), "{bad}");
        }
    }
}

pub(crate) fn move_gray_selected(
    s: &Surface,
    sel: &Surface,
    warp: &dyn Fn(&Surface, Rect) -> Result<Surface>,
    whole: &dyn Fn(&Surface) -> Result<Surface>,
) -> Result<Surface> {
    let Some((lifted, mut out)) = try_split_gray_selected(s, sel)? else { return whole(s) };
    let src = lifted.content_bounds();
    if src.is_empty() {
        return Ok(out);
    }
    let w = warp(&lifted, src)?;
    let b = w.content_bounds();
    if !b.is_empty() {
        let moved = crate::allocation::read_region(&w, b, "compositing transformed mask pixels")?;
        let under = crate::allocation::read_region(&out, b, "compositing transformed mask pixels")?;
        let mut flat = crate::allocation::filled(under.len(), 0.0, "compositing transformed mask pixels")?;
        for (i, (p, u)) in moved.as_chunks::<2>().0.iter().zip(&under).enumerate() {
            flat[i] = p[0] * p[1] + u * (1.0 - p[1]);
        }
        out.try_write_region(b, &flat)?;
    }
    out.prune();
    Ok(out)
}

fn transform_locks(l: &Layer, group: Locks, background: bool) -> Locks {
    let mut own = l.locks;
    if background {
        own.position = false;
        own.transparency = false;
    }
    own.union(group)
}

pub fn locked_for_transform(doc: &Document, id: LayerId) -> bool {
    let Some(l) = doc.layer(id) else { return false };
    // An explicit stack rather than recursion: documents can nest deeply.
    let mut stack = vec![(l, group_locks(doc, id).unwrap_or(Locks { all: true, ..Default::default() }), doc.is_background(id))];
    while let Some((l, group, background)) = stack.pop() {
        let locks = transform_locks(l, group, background);
        if locks.position || locks.all {
            return true;
        }
        if let LayerContent::Group(g) = &l.content {
            stack.extend(g.children.iter().map(|c| (c, locks, false)));
        }
    }
    false
}

fn with_panel_target(s: &Session, p: &Value) -> Value {
    let mut p = if p.is_object() { p.clone() } else { json!({}) };
    if p.get("target").is_none()
        && p.get("layer").is_none()
        && let (Some(m), Some(Value::Object(t))) = (p.as_object_mut(), crate::channel_cmds::targeted_channel(s))
    {
        m.extend(t);
    }
    p
}

pub const MAX_TRANSFORM_PIXELS: u64 = 1 << 30;

pub(crate) fn warp_gray(source: &Surface, h: &Homography, interp: Interp) -> Result<Surface> {
    try_warp_gray(source, h, interp)
}

pub fn moves_whole(layer: &Layer) -> bool {
    matches!(layer.content, LayerContent::Group(_) | LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Smart(_))
}

fn relative_matrix(p: &Value, rect: [f64; 4]) -> Result<Option<Homography>> {
    let num = |k: &str| -> Result<Option<f64>> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_f64().filter(|f| f.is_finite()).map(Some).ok_or_else(|| bad(format!("`{k}` must be a finite number"))),
        }
    };
    let (sx, sy, rot, dx, dy) = (num("scaleX")?, num("scaleY")?, num("rotate")?, num("dx")?, num("dy")?);
    if sx.is_none() && sy.is_none() && rot.is_none() && dx.is_none() && dy.is_none() {
        return Ok(None);
    }
    let (sx, sy) = (sx.unwrap_or(100.0) / 100.0, sy.unwrap_or(100.0) / 100.0);
    if sx.abs() < 1e-6 || sy.abs() < 1e-6 {
        return Err(bad("`scaleX` and `scaleY` must not be 0"));
    }
    let (cx, cy) = ((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0);
    let (sin, cos) = rot.unwrap_or(0.0).to_radians().sin_cos();
    // (x, y) → translate(-c) → scale → rotate → translate(c + d).
    let (a, b, c, d) = (sx * cos, sx * sin, -sy * sin, sy * cos);
    let e = cx + dx.unwrap_or(0.0) - a * cx - c * cy;
    let f = cy + dy.unwrap_or(0.0) - b * cx - d * cy;
    Ok(Some(Homography([a, c, e, b, d, f, 0.0, 0.0, 1.0])))
}
