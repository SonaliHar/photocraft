//! Canvas-wide geometry: Image › Image Rotation (90°, 180°, flips, arbitrary), Canvas Size, Crop,
//! Trim and Image Size move everything in the document, not only pixels. Pixels are remapped by
//! the callers; this module moves the rest: type, shape and smart-object transforms, vector masks,
//! gradient fill angles, effect reference points, artboards, and document-level paths, guides,
//! slices, notes and measurement marks. Layer effect angles are left alone (Photoshop keeps the
//! Global Light when the canvas rotates).

use std::collections::HashMap;
use std::sync::Arc;

use photocraft_doc::{Document, Fill, Layer, LayerContent, TextLayer};
use photocraft_geom::{Affine, Point, Rect};
use photocraft_raster::{Interrupt, Surface};

const EPS: f64 = 1e-9;

fn turn_poll(control: &Interrupt<'_>) -> crate::Result<()> {
    control.check().map_err(|_| crate::EngineError::Cancelled)
}

struct CanvasMetadata {
    extents: HashMap<photocraft_doc::item_identity::ItemRef, Rect>,
    masks: HashMap<photocraft_doc::LayerId, Rect>,
    charged_bytes: u64,
}
impl CanvasMetadata {
    fn new(doc: &Document, limits: photocraft_algo::wide_geometry::GeometryLimits, control: &Interrupt<'_>) -> crate::Result<Self> {
        turn_poll(control)?;
        use photocraft_doc::item_identity::ItemRef;
        let mut result = Self { extents: HashMap::new(), masks: HashMap::new(), charged_bytes: 0 };
        let Some(state) = &doc.xcf_state else {
            return Ok(result);
        };
        if state.items.len() > 100_000 || state.filter_masks.len() > 100_000 {
            return Err(crate::EngineError::Other("Canvas metadata exceeds item limit".into()));
        }
        // Conservative hash-table capacity, entry/control storage and temporary ID index.
        result.charged_bytes = state.items.len() as u64 * 128 + state.filter_masks.len() as u64 * 16;
        if result.charged_bytes > limits.resident_bytes {
            return Err(crate::EngineError::Other("Canvas metadata exceeds memory limit".into()));
        }
        let allocation = || crate::EngineError::Other("Canvas metadata allocation denied".into());
        result.extents.try_reserve(state.items.len()).map_err(|_| allocation())?;
        result.masks.try_reserve(state.items.len()).map_err(|_| allocation())?;
        let mut filter_ids = Vec::new();
        filter_ids.try_reserve_exact(state.filter_masks.len()).map_err(|_| allocation())?;
        filter_ids.extend(state.filter_masks.iter().map(|mask| mask.id));
        turn_poll(control)?;
        filter_ids.sort_unstable();
        for item in &state.items {
            turn_poll(control)?;
            if let Some(bounds) = item.raster_bounds
                && result.extents.insert(item.item, bounds).is_some()
            {
                return Err(crate::EngineError::Other("Ambiguous canvas item bounds".into()));
            }
            let ItemRef::Mask(id) = item.item else {
                continue;
            };
            if filter_ids.binary_search(&id).is_ok() {
                continue;
            }
            if let Some(mask) = &item.mask
                && let ItemRef::Layer(owner) = mask.owner
                && result.masks.insert(owner, mask.bounds).is_some()
            {
                return Err(crate::EngineError::Other("Ambiguous layer-mask bounds".into()));
            }
        }
        Ok(result)
    }
}

fn map_pt(a: &Affine, p: [f64; 2]) -> [f64; 2] {
    let q = a.apply(Point::new(p[0], p[1]));
    [q.x, q.y]
}

/// Bounding box `[x0, y0, x1, y1]` of the four mapped corners of `[x0, y0, x1, y1]`.
fn map_box(a: &Affine, b: [f64; 4]) -> [f64; 4] {
    let c = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(|p| map_pt(a, p));
    c.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |r, p| [r[0].min(p[0]), r[1].min(p[1]), r[2].max(p[0]), r[3].max(p[1])])
}

fn map_rect(a: &Affine, r: Rect) -> Rect {
    if r.is_empty() {
        return r;
    }
    let b = map_box(a, [f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)]);
    let i = |v: f64| v.round().clamp(f64::from(i32::MIN / 2), f64::from(i32::MAX / 2)) as i32;
    Rect::new(i(b[0]), i(b[1]), i(b[2]), i(b[3]))
}

/// A gradient angle (degrees, counter-clockwise on screen) carried through `a`'s linear part.
fn map_angle(a: &Affine, deg: f32) -> f32 {
    let (s, c) = f64::from(deg).to_radians().sin_cos();
    // Direction in y-down document space.
    let (dx, dy) = (c, -s);
    let (nx, ny) = (a.m[0] * dx + a.m[2] * dy, a.m[1] * dx + a.m[3] * dy);
    if nx.abs() < EPS && ny.abs() < EPS {
        return deg;
    }
    let out = (-ny).atan2(nx).to_degrees();
    // Keep whole-degree angles whole (90° turns and flips are exact up to rounding noise).
    let out = if (out - out.round()).abs() < 1e-6 { out.round() } else { out };
    // (−180°, 180°]: −0 from atan2 would otherwise give −180°.
    (if out <= -180.0 { out + 360.0 } else { out }) as f32
}

fn map_fill(a: &Affine, f: &mut Fill) {
    if let Fill::Gradient { angle, offset, .. } = f {
        *angle = map_angle(a, *angle);
        // The centre offset (a fraction of the frame) turns with the canvas: exact for 90° turns
        // and flips, where the frame's sides swap with the axes.
        let (ox, oy) = (f64::from(offset.0), f64::from(offset.1));
        *offset = ((a.m[0] * ox + a.m[2] * oy) as f32, (a.m[1] * ox + a.m[3] * oy) as f32);
    }
}

/// Rebuilds a type layer's PSD `TySh` block for its current transform, keeping its pixels.
fn retag_text(resolution: Option<&photocraft_doc::PhysicalResolution>, t: &mut TextLayer) -> crate::Result<()> {
    if t.psd_raw.is_none() {
        return Ok(());
    }
    let resolution = resolution.ok_or_else(|| crate::EngineError::Other("Missing checked canvas text resolution".into()))?;
    let mut eng = crate::type_cmds::text_engine()?.lock().unwrap_or_else(|e| e.into_inner());
    let layout = eng.layout_with_resolution(t, resolution).map_err(|error| crate::EngineError::Other(error.to_string()))?;
    t.psd_raw = Some(Arc::new(
        photocraft_text::psd::build_tysh_with_resolution(t, resolution, layout.bounds()).map_err(|error| crate::EngineError::Other(error.to_string()))?,
    ));
    Ok(())
}

/// Applies `a` to one layer's (and its children's) non-pixel geometry. With `content` false only
/// the parts Free Transform (`transform_layer`) doesn't move are touched: unlinked vector masks,
/// gradient fill angles, effect reference points and artboards.
#[cfg(test)]
pub(crate) fn transform_layer_geometry(l: &mut Layer, a: &Affine, content: bool, dpi: Option<&photocraft_doc::PhysicalResolution>) -> crate::Result<()> {
    if let Some(vm) = l.vector_mask.as_mut()
        && (content || !vm.linked)
    {
        vm.path = vm.path.transform(a);
    }
    if let Some((x, y)) = l.effects.reference {
        let p = map_pt(a, [x, y]);
        l.effects.reference = Some((p[0], p[1]));
    }
    if !content {
        // Free Transform doesn't warp fill caches: drop it, the fill re-renders from its model.
        l.fill_cache = None;
    } else if let Some(fc) = &mut l.fill_cache {
        map_fill(a, &mut fc.fill);
    }
    match &mut l.content {
        LayerContent::Fill(f) => map_fill(a, f),
        LayerContent::Shape(sh) => {
            if let Some(f) = &mut sh.fill {
                map_fill(a, f);
            }
            if content {
                crate::vector_cmds::transform_shape(sh, a);
            }
        }
        LayerContent::Text(t) if content => {
            t.transform = a.mul(&t.transform);
            retag_text(dpi, t)?;
        }
        LayerContent::Smart(sm) if content => crate::smart_cmds::transform_placement(sm, a),
        LayerContent::Group(g) => {
            if let Some(ab) = &mut g.artboard {
                ab.rect = map_rect(a, ab.rect);
            }
            for c in &mut g.children {
                transform_layer_geometry(c, a, content, dpi)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Document-level geometry: paths, notes, Count/Ruler marks, slices and guides. Guides survive
/// axis-aligned maps (and swap orientation under 90° turns); any other map clears them.
#[cfg(test)]
pub(crate) fn transform_doc_marks(doc: &mut Document, a: &Affine) {
    for p in &mut doc.paths {
        p.path = p.path.transform(a);
    }
    if let Some(wp) = &mut doc.work_path {
        *wp = wp.transform(a);
    }
    for n in &mut doc.notes {
        n.position = map_pt(a, n.position);
        n.popup = map_box(a, n.popup);
    }
    for g in &mut doc.measurement.count_groups {
        for p in &mut g.points {
            *p = map_pt(a, *p);
        }
    }
    if let Some(r) = &mut doc.measurement.ruler {
        r.start = map_pt(a, r.start);
        r.end = map_pt(a, r.end);
        if let Some(p) = &mut r.protractor {
            *p = map_pt(a, *p);
        }
    }
    for s in &mut doc.slices.list {
        s.rect = map_rect(a, s.rect);
    }
    let [m0, m1, m2, m3, tx, ty] = a.m;
    let g = &mut doc.guides;
    if m1.abs() < EPS && m2.abs() < EPS {
        g.horizontal.iter_mut().for_each(|y| *y = m3 * *y + ty);
        g.vertical.iter_mut().for_each(|x| *x = m0 * *x + tx);
    } else if m0.abs() < EPS && m3.abs() < EPS {
        // A horizontal guide at y becomes a vertical one at x' = m2·y + tx, and vice versa.
        let v: Vec<f64> = g.horizontal.iter().map(|y| m2 * y + tx).collect();
        let h: Vec<f64> = g.vertical.iter().map(|x| m1 * x + ty).collect();
        g.horizontal = h;
        g.vertical = v;
    } else {
        g.horizontal.clear();
        g.vertical.clear();
    }
}

/// Resolve physical type resolution only when a canvas operation will rebuild PSD type data.
/// The bounded walk finishes before any geometry or cache is changed.
#[cfg(test)]
fn retag_dpi(doc: &Document) -> crate::Result<Option<photocraft_doc::PhysicalResolution>> {
    retag_dpi_with_interrupt(doc, &Interrupt::NONE)
}
fn retag_dpi_with_interrupt(doc: &Document, control: &Interrupt<'_>) -> crate::Result<Option<photocraft_doc::PhysicalResolution>> {
    turn_poll(control)?;
    let allocation = || crate::EngineError::Other("Canvas text preflight allocation denied".into());
    let mut stack = Vec::new();
    stack.try_reserve(1).map_err(|_| allocation())?;
    stack.push((doc.layers.as_slice(), 0usize));
    let mut count = 0usize;
    let mut needs_retag = false;
    while let Some((layers, depth)) = stack.pop() {
        turn_poll(control)?;
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(crate::EngineError::Other("Canvas group depth exceeds limit".into()));
        }
        let Some((layer, remaining)) = layers.split_first() else { continue };
        count = count.saturating_add(1);
        if count > 100_000 {
            return Err(crate::EngineError::Other("Canvas text preflight exceeds layer limit".into()));
        }
        stack.try_reserve(2).map_err(|_| allocation())?;
        if !remaining.is_empty() {
            stack.push((remaining, depth));
        }
        match &layer.content {
            LayerContent::Text(text) => needs_retag |= text.psd_raw.is_some(),
            LayerContent::Group(group) => stack.push((group.children.as_slice(), depth + 1)),
            _ => {}
        }
    }
    turn_poll(control)?;
    if needs_retag { Ok(Some(crate::type_cmds::checked_layout_resolution(doc)?)) } else { Ok(None) }
}

/// Known retained model/maps/COW preflight before snapshot or geometry work.
/// Immutable raster/source Arc payloads are retained once, not multiplied as
/// model clones. This is not an allocator/RSS or regeneration-peak proof.
pub(crate) fn preflight_geometry_with_interrupt(doc: &Document, control: &Interrupt<'_>) -> crate::Result<()> {
    turn_poll(control)?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, control)
        .map_err(|error| if control.cancelled() { crate::EngineError::Cancelled } else { crate::EngineError::Other(error.to_string()) })?
        as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, control)
        .map_err(|error| if control.cancelled() { crate::EngineError::Cancelled } else { crate::EngineError::Other(error.to_string()) })?
        as u64;
    let raster = crate::document_snapshot::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    if model
        .checked_mul(3)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(3)?))
        .is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES)
    {
        return Err(crate::EngineError::Other("Canvas geometry retained/model staging limit".into()));
    }
    turn_poll(control)
}
fn finite_canvas(values: &[f64]) -> crate::Result<()> {
    if values.iter().all(|value| value.is_finite()) { Ok(()) } else { Err(crate::EngineError::Other("Nonfinite canvas geometry".into())) }
}
fn path_with_interrupt(path: &mut photocraft_doc::Path, a: &Affine, control: &Interrupt<'_>) -> crate::Result<()> {
    for subpath in &mut path.subpaths {
        turn_poll(control)?;
        for chunk in subpath.knots.chunks_mut(256) {
            turn_poll(control)?;
            for knot in chunk {
                let next = knot.transform(a);
                finite_canvas(&[next.anchor.x, next.anchor.y, next.in_ctrl.x, next.in_ctrl.y, next.out_ctrl.x, next.out_ctrl.y])?;
                *knot = next;
            }
        }
    }
    turn_poll(control)
}
fn fill_with_interrupt(fill: &mut Fill, a: &Affine, control: &Interrupt<'_>) -> crate::Result<()> {
    turn_poll(control)?;
    map_fill(a, fill);
    if let Fill::Gradient { angle, offset, .. } = fill {
        finite_canvas(&[f64::from(*angle), f64::from(offset.0), f64::from(offset.1)])?;
    }
    turn_poll(control)
}
fn live_shape_finite(live: Option<&photocraft_doc::LiveShape>) -> crate::Result<()> {
    use photocraft_doc::LiveShape;
    match live {
        Some(LiveShape::Rect { rect, radii }) => {
            finite_canvas(rect)?;
            finite_canvas(radii)
        }
        Some(LiveShape::Ellipse { rect }) => finite_canvas(rect),
        Some(LiveShape::Polygon { rect, star_ratio, .. }) => {
            finite_canvas(rect)?;
            finite_canvas(&[*star_ratio])
        }
        Some(LiveShape::Line { from, to, weight }) => {
            finite_canvas(from)?;
            finite_canvas(to)?;
            finite_canvas(&[*weight])
        }
        None => Ok(()),
    }
}
fn layer_geometry_with_interrupt(
    layer: &mut Layer,
    a: &Affine,
    dpi: Option<&photocraft_doc::PhysicalResolution>,
    depth: usize,
    control: &Interrupt<'_>,
) -> crate::Result<()> {
    turn_poll(control)?;
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(crate::EngineError::Other("Canvas group depth exceeds limit".into()));
    }
    if let Some(mask) = &mut layer.vector_mask {
        path_with_interrupt(&mut mask.path, a, control)?;
    }
    if let Some((x, y)) = layer.effects.reference {
        let value = map_pt(a, [x, y]);
        finite_canvas(&value)?;
        layer.effects.reference = Some((value[0], value[1]));
    }
    if let Some(cache) = &mut layer.fill_cache {
        fill_with_interrupt(&mut cache.fill, a, control)?;
    }
    match &mut layer.content {
        LayerContent::Group(group) => {
            if let Some(artboard) = &mut group.artboard {
                let mapped = map_box(a, [artboard.rect.x0 as f64, artboard.rect.y0 as f64, artboard.rect.x1 as f64, artboard.rect.y1 as f64]);
                finite_canvas(&mapped)?;
                artboard.rect = map_rect(a, artboard.rect);
            }
            for child in &mut group.children {
                layer_geometry_with_interrupt(child, a, dpi, depth + 1, control)?;
            }
        }
        LayerContent::Shape(shape) => {
            if let Some(fill) = &mut shape.fill {
                fill_with_interrupt(fill, a, control)?;
            }
            // Preserve the exact shared live-shape math while avoiding its
            // whole Path clone: transform original knots in place, restore them.
            let mut path = std::mem::take(&mut shape.path);
            path_with_interrupt(&mut path, a, control)?;
            crate::vector_cmds::transform_shape(shape, a);
            shape.path = path;
            live_shape_finite(shape.live.as_ref())?;
        }
        LayerContent::Fill(fill) => fill_with_interrupt(fill, a, control)?,
        LayerContent::Text(text) => {
            text.transform = a.mul(&text.transform);
            finite_canvas(&text.transform.m)?;
            // Layout/TySh APIs currently have no internal Interrupt argument.
            turn_poll(control)?;
            retag_text(dpi, text)?;
            turn_poll(control)?;
        }
        LayerContent::Smart(smart) => {
            crate::smart_cmds::transform_placement(smart, a);
            finite_canvas(&smart.transform.m)?;
            if let Some(perspective) = &smart.perspective {
                finite_canvas(perspective)?;
            }
        }
        _ => {}
    }
    turn_poll(control)
}
fn marks_with_interrupt(doc: &mut Document, a: &Affine, control: &Interrupt<'_>) -> crate::Result<()> {
    for path in &mut doc.paths {
        turn_poll(control)?;
        path_with_interrupt(&mut path.path, a, control)?;
    }
    if let Some(path) = &mut doc.work_path {
        path_with_interrupt(path, a, control)?;
    }
    for note in &mut doc.notes {
        turn_poll(control)?;
        let position = map_pt(a, note.position);
        let popup = map_box(a, note.popup);
        finite_canvas(&position)?;
        finite_canvas(&popup)?;
        note.position = position;
        note.popup = popup;
    }
    for group in &mut doc.measurement.count_groups {
        turn_poll(control)?;
        for chunk in group.points.chunks_mut(256) {
            turn_poll(control)?;
            for point in chunk {
                let next = map_pt(a, *point);
                finite_canvas(&next)?;
                *point = next;
            }
        }
    }
    if let Some(ruler) = &mut doc.measurement.ruler {
        let start = map_pt(a, ruler.start);
        let end = map_pt(a, ruler.end);
        finite_canvas(&start)?;
        finite_canvas(&end)?;
        ruler.start = start;
        ruler.end = end;
        if let Some(point) = &mut ruler.protractor {
            let next = map_pt(a, *point);
            finite_canvas(&next)?;
            *point = next;
        }
    }
    for slice in &mut doc.slices.list {
        turn_poll(control)?;
        let mapped = map_box(a, [slice.rect.x0 as f64, slice.rect.y0 as f64, slice.rect.x1 as f64, slice.rect.y1 as f64]);
        finite_canvas(&mapped)?;
        slice.rect = map_rect(a, slice.rect);
    }
    let [m0, m1, m2, m3, tx, ty] = a.m;
    let guides = &mut doc.guides;
    let map_guides = |values: &mut [f64], scale: f64, offset: f64| -> crate::Result<()> {
        for chunk in values.chunks_mut(256) {
            turn_poll(control)?;
            for value in chunk {
                let next = scale * *value + offset;
                if !next.is_finite() {
                    return Err(crate::EngineError::Other("Canvas guide exceeds finite f64 range".into()));
                }
                *value = next;
            }
        }
        Ok(())
    };
    if m1.abs() < EPS && m2.abs() < EPS {
        map_guides(&mut guides.horizontal, m3, ty)?;
        map_guides(&mut guides.vertical, m0, tx)?;
    } else if m0.abs() < EPS && m3.abs() < EPS {
        map_guides(&mut guides.horizontal, m2, tx)?;
        map_guides(&mut guides.vertical, m1, ty)?;
        std::mem::swap(&mut guides.horizontal, &mut guides.vertical);
    } else {
        guides.horizontal.clear();
        guides.vertical.clear();
    }
    turn_poll(control)
}

/// Applies `a` to all non-pixel geometry of the document (layers and document marks).
#[cfg(test)]
pub(crate) fn transform_geometry(doc: &mut Document, a: &Affine) -> crate::Result<()> {
    transform_geometry_with_interrupt(doc, a, &Interrupt::NONE)
}
/// Controlled private-candidate mutation. Callers discard partial candidates on
/// refusal; source/cache/provenance bytes and DPI are not reinterpreted here.
pub(crate) fn transform_geometry_with_interrupt(doc: &mut Document, a: &Affine, control: &Interrupt<'_>) -> crate::Result<()> {
    turn_poll(control)?;
    finite_canvas(&a.m)?;
    preflight_geometry_with_interrupt(doc, control)?;
    let dpi = retag_dpi_with_interrupt(doc, control)?;
    for l in &mut doc.layers {
        layer_geometry_with_interrupt(l, a, dpi.as_ref(), 0, control)?;
    }
    marks_with_interrupt(doc, a, control)
}

/// What to re-render after the geometry moved.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refresh {
    /// Shapes only (their pixels are clipped to the canvas, which may have grown).
    Shapes,
    /// Type, shapes and smart objects (after a resampling map). Smart objects whose source is
    /// unavailable keep their (already mapped) cache.
    All,
}

fn refresh_admission(layers: &[Layer], what: Refresh, format: photocraft_color::PixelFormat, depth: usize, control: &Interrupt<'_>) -> crate::Result<bool> {
    turn_poll(control)?;
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(crate::EngineError::Other("Canvas refresh group depth exceeds limit".into()));
    }
    let wide = |format: photocraft_color::PixelFormat| matches!(format.sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64);
    let mut needed = false;
    for layer in layers {
        turn_poll(control)?;
        let cache = match &layer.content {
            LayerContent::Group(group) => {
                needed |= refresh_admission(&group.children, what, format, depth + 1, control)?;
                continue;
            }
            LayerContent::Shape(shape) => {
                needed = true;
                shape.cache.as_ref()
            }
            LayerContent::Text(text) if what == Refresh::All => {
                needed = true;
                text.cache.as_ref()
            }
            LayerContent::Smart(smart) if what == Refresh::All => {
                needed = true;
                smart.cache.as_ref()
            }
            _ => continue,
        };
        if wide(format) || cache.is_some_and(|surface| wide(surface.format())) {
            return Err(crate::EngineError::Other("Canvas cache regeneration does not support U32/F64 arithmetic".into()));
        }
    }
    turn_poll(control)?;
    Ok(needed)
}
/// Snapshot clone is admitted before allocation; hierarchy/cache boundaries poll.
/// Current text render/TySh, shape render and smart refresh APIs are internally
/// uninterruptible. Their existing math/source behavior is retained and guarded
/// before/after each call; this does not claim deep cancellation or render RSS.
pub(crate) fn refresh_with_interrupt(doc: &mut Document, what: Refresh, control: &Interrupt<'_>) -> crate::Result<()> {
    preflight_geometry_with_interrupt(doc, control)?;
    if !refresh_admission(&doc.layers, what, doc.pixel_format(), 0, control)? {
        return turn_poll(control);
    }
    fn rec(snapshot: &Document, layers: &mut [Layer], what: Refresh, depth: usize, control: &Interrupt<'_>) -> crate::Result<()> {
        turn_poll(control)?;
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(crate::EngineError::Other("Canvas refresh group depth exceeds limit".into()));
        }
        for l in layers {
            turn_poll(control)?;
            match &mut l.content {
                LayerContent::Group(g) => rec(snapshot, &mut g.children, what, depth + 1, control)?,
                LayerContent::Shape(sh) => crate::vector_cmds::refresh_shape(snapshot, sh),
                LayerContent::Text(t) if what == Refresh::All => crate::type_cmds::try_refresh(snapshot, t)?,
                LayerContent::Smart(_) if what == Refresh::All => {
                    #[cfg(target_arch = "wasm32")]
                    crate::type_cmds::text_engine()?;
                    crate::smart_cmds::refresh_layer(snapshot, l)?;
                }
                _ => {}
            }
            turn_poll(control)?;
        }
        turn_poll(control)
    }
    turn_poll(control)?;
    let snapshot = doc.clone();
    turn_poll(control)?;
    rec(&snapshot, &mut doc.layers, what, 0, control)
}

/// Image › Image Rotation by a right angle or a flip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Turn {
    FlipHorizontal,
    FlipVertical,
    Rotate180,
    Cw90,
    Ccw90,
}

impl Turn {
    /// Pixel map `(x, y) → (x', y')` (pixel indices) for a `w × h` canvas.
    #[cfg(test)]
    pub(crate) fn pixel_map(self, w: i32, h: i32) -> impl Fn(i32, i32) -> (i32, i32) {
        move |x, y| match self {
            Turn::FlipHorizontal => (w - 1 - x, y),
            Turn::FlipVertical => (x, h - 1 - y),
            Turn::Rotate180 => (w - 1 - x, h - 1 - y),
            Turn::Cw90 => (h - 1 - y, x),
            Turn::Ccw90 => (y, w - 1 - x),
        }
    }
    /// The same map in continuous (pixel-edge) coordinates.
    pub(crate) fn affine(self, w: f64, h: f64) -> Affine {
        Affine {
            m: match self {
                Turn::FlipHorizontal => [-1.0, 0.0, 0.0, 1.0, w, 0.0],
                Turn::FlipVertical => [1.0, 0.0, 0.0, -1.0, 0.0, h],
                Turn::Rotate180 => [-1.0, 0.0, 0.0, -1.0, w, h],
                Turn::Cw90 => [0.0, 1.0, -1.0, 0.0, h, 0.0],
                Turn::Ccw90 => [0.0, -1.0, 1.0, 0.0, 0.0, w],
            },
        }
    }
    fn swaps(self) -> bool {
        matches!(self, Turn::Cw90 | Turn::Ccw90)
    }
}

/// Every pixel surface a canvas turn moves: content, rendered caches (type, shape, smart object,
/// fill), masks (layer, smart filter), alpha channels, Quick Mask and the selection.
fn remap_pixels(doc: &mut Document, turn: Turn, control: &Interrupt<'_>) -> crate::Result<()> {
    let (width, height) = (i64::from(doc.size.width), i64::from(doc.size.height));
    let re = |surface: &mut Surface| -> crate::Result<()> {
        turn_poll(control)?;
        let bounds = surface.content_bounds();
        photocraft_raster::checked_region_byte_len(bounds, surface.format()).map_err(|error| crate::EngineError::Other(error.to_string()))?;
        let mut output = Surface::with_default(surface.format(), &surface.default_pixel());
        let channels = surface.channels();
        if channels == 0 {
            return Err(crate::EngineError::Other("Canvas sample layout has no channels".into()));
        }
        // Preserve the legacy f32 sample/default interpretation, with bounded
        // tile scratch and row polling instead of one full-surface read.
        for tile in bounds.tiles() {
            turn_poll(control)?;
            let area = tile.rect().intersect(&bounds);
            let band = surface.read_region(area);
            let row_width = usize::try_from(area.width()).map_err(|_| crate::EngineError::Other("Canvas row exceeds coordinates".into()))?;
            if row_width == 0 {
                continue;
            }
            for (index, pixel) in band.chunks_exact(channels).enumerate() {
                if index.is_multiple_of(row_width) {
                    turn_poll(control)?;
                }
                let x = i64::from(area.x0) + i64::try_from(index % row_width).map_err(|_| crate::EngineError::Other("Canvas x coordinate overflow".into()))?;
                let y = i64::from(area.y0) + i64::try_from(index / row_width).map_err(|_| crate::EngineError::Other("Canvas y coordinate overflow".into()))?;
                let (nx, ny) = match turn {
                    Turn::FlipHorizontal => (width - 1 - x, y),
                    Turn::FlipVertical => (x, height - 1 - y),
                    Turn::Rotate180 => (width - 1 - x, height - 1 - y),
                    Turn::Cw90 => (height - 1 - y, x),
                    Turn::Ccw90 => (y, width - 1 - x),
                };
                let nx = i32::try_from(nx).map_err(|_| crate::EngineError::Other("Canvas mapped x exceeds coordinates".into()))?;
                let ny = i32::try_from(ny).map_err(|_| crate::EngineError::Other("Canvas mapped y exceeds coordinates".into()))?;
                output.write_pixel(nx, ny, pixel);
            }
        }
        turn_poll(control)?;
        *surface = output;
        Ok(())
    };
    fn rec(layers: &mut [Layer], re: &dyn Fn(&mut Surface) -> crate::Result<()>, control: &Interrupt<'_>) -> crate::Result<()> {
        for l in layers {
            turn_poll(control)?;
            if let Some(m) = &mut l.mask {
                re(std::sync::Arc::make_mut(&mut m.surface))?;
            }
            if let Some(fc) = &mut l.fill_cache {
                re(&mut fc.surface)?;
            }
            match &mut l.content {
                LayerContent::Raster(s) => re(s)?,
                LayerContent::Text(t) => {
                    if let Some(cache) = &mut t.cache {
                        re(cache)?;
                    }
                }
                LayerContent::Shape(sh) => {
                    if let Some(cache) = &mut sh.cache {
                        re(cache)?;
                    }
                }
                LayerContent::Smart(sm) => {
                    if let Some(cache) = &mut sm.cache {
                        re(cache)?;
                    }
                    if let Some(m) = &mut sm.filter_mask {
                        re(std::sync::Arc::make_mut(&mut m.surface))?;
                    }
                }
                LayerContent::Group(g) => rec(&mut g.children, re, control)?,
                _ => {}
            }
        }
        Ok(())
    }
    rec(&mut doc.layers, &re, control)?;
    for ch in doc.channels.iter_mut().chain(doc.quick_mask.as_mut()) {
        re(&mut ch.surface)?;
    }
    if let Some(selection) = &mut doc.selection {
        re(selection)?;
    }
    turn_poll(control)
}

/// Turns or flips the whole canvas. Pixels (and rendered caches) are remapped exactly, so nothing
/// is re-rendered; vector geometry follows through the matching affine map.
#[cfg(test)]
pub(crate) fn turn_canvas(doc: &mut Document, turn: Turn) -> crate::Result<()> {
    turn_canvas_with_interrupt(doc, turn, &Interrupt::NONE)
}

/// Controlled staging seam. Callers publish through a single Session edit; an
/// error/cancellation discards its staged document rather than partially commit.
pub(crate) fn turn_canvas_with_interrupt(doc: &mut Document, turn: Turn, control: &Interrupt<'_>) -> crate::Result<()> {
    turn_poll(control)?;
    // The document's physical axes turn with a quarter-turn. Prepare the
    // canonical metadata before touching pixels; immutable source DPI stays put.
    let rotated_resolution = if turn.swaps() {
        doc.physical_resolution
            .as_ref()
            .map(|resolution| {
                let mut rotated = resolution.clone();
                std::mem::swap(&mut rotated.x_dpi, &mut rotated.y_dpi);
                rotated.validate().map_err(|error| crate::EngineError::Other(format!("Canvas physical resolution: {error}")))?;
                Ok::<_, crate::EngineError>(rotated)
            })
            .transpose()?
    } else {
        None
    };
    retag_dpi_with_interrupt(doc, control)?;
    turn_poll(control)?;
    let w = i32::try_from(doc.size.width).map_err(|_| crate::EngineError::Other("Canvas width exceeds safe coordinates".into()))?;
    let h = i32::try_from(doc.size.height).map_err(|_| crate::EngineError::Other("Canvas height exceeds safe coordinates".into()))?;
    let _ = (w, h); // Validated dimensions also bound the legacy i64 pixel map.
    let a = turn.affine(f64::from(doc.size.width), f64::from(doc.size.height));
    // The legacy band path still writes destination tiles: charge their full
    // payload before any allocations, including thin/extremely sparse extents.
    preflight_checked_turn_with_interrupt(doc, &a, photocraft_algo::wide_geometry::GeometryLimits::default(), control)?;
    if matches!(doc.depth, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64) || doc.xcf_state.is_some() {
        remap_checked_pixels(doc, turn, &a, control)?;
    } else {
        remap_pixels(doc, turn, control)?;
    }
    turn_poll(control)?;
    if turn.swaps() {
        doc.size = photocraft_doc::Size::new(doc.size.height, doc.size.width);
    }
    transform_geometry_with_interrupt(doc, &a, control)?;
    turn_poll(control)?;
    if let Some(resolution) = rotated_resolution {
        doc.set_physical_resolution(resolution).map_err(|error| crate::EngineError::Other(format!("Canvas physical resolution: {error}")))?;
    }
    turn_poll(control)
}

/// Charge the complete staged canvas before authoring its first replacement.
#[cfg(test)]
fn preflight_checked_turn(doc: &Document, affine: &Affine, limits: photocraft_algo::wide_geometry::GeometryLimits) -> crate::Result<()> {
    preflight_checked_turn_with_interrupt(doc, affine, limits, &Interrupt::NONE)
}
fn preflight_checked_turn_with_interrupt(
    doc: &Document,
    affine: &Affine,
    limits: photocraft_algo::wide_geometry::GeometryLimits,
    control: &Interrupt<'_>,
) -> crate::Result<()> {
    turn_poll(control)?;
    use photocraft_doc::item_identity::ItemRef;
    let fail = || crate::EngineError::Other("Canvas geometry exceeds aggregate memory, work or coordinate limits".into());
    let checked_bounds = |bounds: Rect| -> crate::Result<Rect> {
        if bounds.is_empty() {
            return Ok(bounds);
        }
        let values = map_box(affine, [f64::from(bounds.x0), f64::from(bounds.y0), f64::from(bounds.x1), f64::from(bounds.y1)]);
        if values.iter().any(|n| !n.is_finite() || *n < f64::from(i32::MIN / 2) || *n > f64::from(i32::MAX / 2)) {
            return Err(fail());
        }
        Ok(Rect::new(values[0] as i32, values[1] as i32, values[2] as i32, values[3] as i32))
    };
    let state = doc.xcf_state.as_ref();
    let metadata = CanvasMetadata::new(doc, limits, control)?;
    let mut resident = metadata.charged_bytes;
    let mut work = 0u64;
    let mut scratch = 0u64;
    let mut charge = |surface: &Surface, bounds: Option<Rect>| -> crate::Result<()> {
        turn_poll(control)?;
        let bounds = bounds.unwrap_or_else(|| surface.content_bounds());
        let destination = checked_bounds(bounds)?;
        let input = surface.resident_payload_bytes().map_err(|_| fail())?;
        let bytes = photocraft_raster::checked_region_byte_len(bounds, surface.format()).map_err(|_| fail())? as u64;
        let tile_bytes = (256u64 * 256).checked_mul(surface.format().bytes_per_pixel() as u64).ok_or_else(fail)?;
        let tiles = if destination.is_empty() {
            0
        } else {
            let cols = i64::from((destination.x1 - 1).div_euclid(256)) - i64::from(destination.x0.div_euclid(256)) + 1;
            let rows = i64::from((destination.y1 - 1).div_euclid(256)) - i64::from(destination.y0.div_euclid(256)) + 1;
            u64::try_from(cols).ok().and_then(|cols| u64::try_from(rows).ok().and_then(|rows| cols.checked_mul(rows))).ok_or_else(fail)?
        };
        resident = resident.checked_add(input).and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes)?)).ok_or_else(fail)?;
        work = work.checked_add(bytes).ok_or_else(fail)?;
        scratch = scratch.max(tile_bytes.checked_mul(6).ok_or_else(fail)?);
        if resident.checked_add(scratch).is_none_or(|n| n > limits.resident_bytes) || work > limits.work_units {
            return Err(fail());
        }
        Ok(())
    };
    fn visit(
        layers: &[Layer],
        depth: usize,
        metadata: &CanvasMetadata,
        charge: &mut dyn FnMut(&Surface, Option<Rect>) -> crate::Result<()>,
        control: &Interrupt<'_>,
    ) -> crate::Result<()> {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(crate::EngineError::Other("Canvas group depth exceeds limit".into()));
        }
        for layer in layers {
            turn_poll(control)?;
            let bounds = metadata.extents.get(&ItemRef::Layer(layer.id)).copied();
            if let Some(mask) = &layer.mask {
                let bounds = metadata.masks.get(&layer.id).copied();
                charge(&mask.surface, bounds)?;
            }
            if let Some(cache) = &layer.fill_cache {
                charge(&cache.surface, None)?;
            }
            match &layer.content {
                LayerContent::Raster(surface) => charge(surface, bounds)?,
                LayerContent::Text(text) => {
                    if let Some(cache) = &text.cache {
                        charge(cache, None)?;
                    }
                }
                LayerContent::Shape(shape) if shape.cache.is_some() => {
                    if let Some(cache) = &shape.cache {
                        charge(cache, None)?;
                    }
                }
                LayerContent::Smart(smart) => {
                    if let Some(cache) = &smart.cache {
                        charge(cache, None)?;
                    }
                    if let Some(mask) = &smart.filter_mask {
                        charge(&mask.surface, None)?;
                    }
                }
                LayerContent::Group(group) => visit(&group.children, depth + 1, metadata, charge, control)?,
                _ => {}
            }
        }
        Ok(())
    }
    visit(&doc.layers, 0, &metadata, &mut charge, control)?;
    for channel in doc.channels.iter().chain(doc.quick_mask.as_ref()) {
        let bounds = metadata.extents.get(&ItemRef::Channel(channel.id)).copied();
        charge(&channel.surface, bounds)?;
    }
    if let Some(selection) = &doc.selection {
        charge(selection, Some(doc.bounds()))?;
    }
    if let Some(state) = state {
        for mask in &state.filter_masks {
            charge(&mask.surface, Some(mask.bounds))?;
        }
        for item in &state.items {
            turn_poll(control)?;
            if let Some(bounds) = item.raster_bounds {
                checked_bounds(bounds)?;
            }
            if let Some(mask) = &item.mask {
                checked_bounds(mask.bounds)?;
            }
        }
        for item in &state.editable_indexed {
            turn_poll(control)?;
            let bytes = item.raster.indices().len() as u64 + item.raster.alpha().map_or(0, |a| a.len() as u64);
            resident = resident.checked_add(bytes.checked_mul(2).ok_or_else(fail)?).ok_or_else(fail)?;
            work = work.checked_add(bytes).ok_or_else(fail)?;
        }
    }
    if resident.checked_add(scratch).is_none_or(|n| n > limits.resident_bytes) || work > limits.work_units {
        return Err(fail());
    }
    turn_poll(control)
}

fn remap_checked_pixels(doc: &mut Document, turn: Turn, affine: &Affine, control: &Interrupt<'_>) -> crate::Result<()> {
    turn_poll(control)?;
    use photocraft_algo::wide_geometry::{GeometryLimits, OrthogonalTransform, try_transform_orthogonal};
    use photocraft_doc::item_identity::ItemRef;
    let operation = match turn {
        Turn::FlipHorizontal => OrthogonalTransform::FlipHorizontal,
        Turn::FlipVertical => OrthogonalTransform::FlipVertical,
        Turn::Rotate180 => OrthogonalTransform::Rotate180,
        Turn::Cw90 => OrthogonalTransform::RotateClockwise,
        Turn::Ccw90 => OrthogonalTransform::RotateCounterclockwise,
    };
    let metadata = CanvasMetadata::new(doc, GeometryLimits::default(), control)?;
    let mut indexed = crate::xcf_indexed_geometry::transform_orthogonal(doc, doc.bounds(), operation, GeometryLimits::default(), control)
        .map_err(|error| if control.cancelled() { crate::EngineError::Cancelled } else { error })?;
    indexed.sort_unstable_by_key(|(owner, _)| *owner);
    let transform = |surface: &mut Surface, declared: Option<Rect>| -> crate::Result<()> {
        turn_poll(control)?;
        let bounds = declared.unwrap_or_else(|| surface.content_bounds());
        if bounds.is_empty() {
            return Ok(());
        }
        let destination = map_rect(affine, bounds);
        *surface = try_transform_orthogonal(surface, bounds, (destination.x0, destination.y0), operation, GeometryLimits::default(), control)
            .map_err(|e| {
                if matches!(e, photocraft_raster::RasterInputError::Cancelled) {
                    crate::EngineError::Cancelled
                } else {
                    crate::EngineError::Other(format!("Cannot turn canvas pixels: {e}"))
                }
            })?
            .0;
        Ok(())
    };
    fn rec_checked(
        layers: &mut [Layer],
        metadata: &CanvasMetadata,
        indexed: &[(photocraft_doc::LayerId, Rect)],
        transform: &dyn Fn(&mut Surface, Option<Rect>) -> crate::Result<()>,
        control: &Interrupt<'_>,
    ) -> crate::Result<()> {
        for layer in layers {
            turn_poll(control)?;
            let declared = metadata.extents.get(&ItemRef::Layer(layer.id)).copied();
            if let Some(mask) = &mut layer.mask {
                transform(std::sync::Arc::make_mut(&mut mask.surface), metadata.masks.get(&layer.id).copied())?;
            }
            if let Some(cache) = &mut layer.fill_cache {
                transform(&mut cache.surface, None)?;
            }
            match &mut layer.content {
                LayerContent::Raster(surface) if indexed.binary_search_by_key(&layer.id, |(owner, _)| *owner).is_err() => transform(surface, declared)?,
                LayerContent::Text(text) => {
                    if let Some(cache) = &mut text.cache {
                        transform(cache, None)?;
                    }
                }
                LayerContent::Shape(shape) => {
                    if let Some(cache) = &mut shape.cache {
                        transform(cache, None)?;
                    }
                }
                LayerContent::Smart(smart) => {
                    if let Some(cache) = &mut smart.cache {
                        transform(cache, None)?;
                    }
                    if let Some(mask) = &mut smart.filter_mask {
                        transform(std::sync::Arc::make_mut(&mut mask.surface), None)?;
                    }
                }
                LayerContent::Group(group) => rec_checked(&mut group.children, metadata, indexed, transform, control)?,
                _ => {}
            }
        }
        Ok(())
    }
    rec_checked(&mut doc.layers, &metadata, &indexed, &transform, control)?;
    for channel in doc.channels.iter_mut().chain(doc.quick_mask.as_mut()) {
        let declared = metadata.extents.get(&ItemRef::Channel(channel.id)).copied();
        transform(&mut channel.surface, declared)?;
    }
    if let Some(selection) = &mut doc.selection {
        transform(selection, Some(Rect::from_xywh(0, 0, doc.size.width, doc.size.height)))?;
    }
    if let Some(state) = &mut doc.xcf_state {
        for mask in &mut state.filter_masks {
            transform(&mut mask.surface, Some(mask.bounds))?;
            mask.bounds = map_rect(affine, mask.bounds);
        }
        for item in &mut state.items {
            turn_poll(control)?;
            if let Some(bounds) = &mut item.raster_bounds {
                *bounds = map_rect(affine, *bounds);
            }
            if let Some(mask) = &mut item.mask {
                mask.bounds = map_rect(affine, mask.bounds);
                mask.offset = [mask.bounds.x0, mask.bounds.y0];
            }
        }
    }
    turn_poll(control)
}

#[cfg(test)]
mod tests;

// Controlled metadata/refresh regression drafts, intentionally unrun.
#[cfg(test)]
mod controlled_geometry_drafts {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn document() -> Document {
        let mut doc = Document::new("Own geometry", photocraft_doc::Size::new(4, 3), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U16);
        doc.layers.push(Layer::raster("Own unchanged raster", doc.pixel_format()));
        doc.set_physical_resolution(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: photocraft_doc::PhysicalUnit::Inch }).unwrap();
        let mut path = photocraft_doc::NamedPath::new(
            "Own path",
            photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[(0.25, 0.5), (2.0, 0.5), (1.0, 2.0)])]),
        );
        path.psd_raw = Some(Arc::new(vec![1, 2, 3]));
        doc.paths.push(path);
        doc.guides.horizontal = vec![0.5, 2.5];
        doc.guides.vertical = vec![1.5];
        doc
    }
    #[test]
    fn controlled_geometry_matches_existing_math_and_preserves_raw_dpi_and_pixels() {
        let original = document();
        let before_pixels = original.layers[0].surface().unwrap().clone();
        let raw = original.paths[0].psd_raw.clone();
        for affine in [Affine::translate(-3.0, 5.0), Affine { m: [0.0, 1.0, -1.0, 0.0, 3.0, 0.0] }] {
            let mut expected = original.clone();
            let dpi = retag_dpi(&expected).unwrap();
            for layer in &mut expected.layers {
                transform_layer_geometry(layer, &affine, true, dpi.as_ref()).unwrap();
            }
            transform_doc_marks(&mut expected, &affine);
            let mut actual = original.clone();
            transform_geometry_with_interrupt(&mut actual, &affine, &Interrupt::NONE).unwrap();
            assert_eq!(actual, expected);
            assert_eq!(actual.physical_resolution, original.physical_resolution);
            assert_eq!(actual.paths[0].psd_raw, raw);
            assert_eq!(actual.layers[0].surface().unwrap(), &before_pixels);
        }
    }
    #[test]
    fn late_metadata_cancel_inside_session_keeps_native_and_real_redo() {
        use std::sync::atomic::AtomicBool;
        let mut doc = document();
        doc.work_path = Some(photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&vec![(0.5, 1.0); 64])]));
        let mut session = crate::Session::new();
        session.open_document(doc, None).unwrap();
        session
            .edit("Own redo seed", |doc, _| {
                doc.layers[0].name.push_str(" edited");
                Ok(())
            })
            .unwrap();
        let redo_document = session.active().unwrap().doc.clone();
        session.execute("edit.undo", serde_json::json!({})).unwrap();
        let before = session.active().unwrap().doc.clone();
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let revision = session.active().unwrap().revision;
        let past = session.active().unwrap().history.entry_labels().map(str::to_owned).collect::<Vec<_>>();
        let redo = session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>();

        // Probe the helper separately, then cancel its final poll inside the
        // actual Session callback. Session admission cannot consume this signal.
        let probe_polls = AtomicUsize::new(0);
        let probe_signal = || {
            probe_polls.fetch_add(1, Ordering::Relaxed);
            false
        };
        let mut probe = (*before).clone();
        transform_geometry_with_interrupt(&mut probe, &Affine::translate(1.0, 0.0), &Interrupt::cancel_only(&probe_signal)).unwrap();
        let cancel_at = probe_polls.load(Ordering::Relaxed).checked_sub(1).unwrap();
        assert!(cancel_at > 8);
        let entered = AtomicBool::new(false);
        let polls = AtomicUsize::new(0);
        let cancel = || entered.load(Ordering::Relaxed) && polls.fetch_add(1, Ordering::Relaxed) >= cancel_at;
        let control = Interrupt::cancel_only(&cancel);
        assert!(matches!(
            session.edit_with_interrupt("Own cancelled metadata", &control, |doc, _| {
                entered.store(true, Ordering::Relaxed);
                transform_geometry_with_interrupt(doc, &Affine::translate(1.0, 0.0), &control)
            }),
            Err(crate::EngineError::Cancelled)
        ));
        assert!(entered.load(Ordering::Relaxed));
        assert_eq!(polls.load(Ordering::Relaxed), cancel_at + 1);
        let active = session.active().unwrap();
        assert!(active.doc.shares_state(&before));
        assert_eq!(active.revision, revision);
        assert_eq!(active.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(), past);
        assert_eq!(active.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
        assert!(active.history.can_redo());
        assert_eq!(photocraft_format::save_to_bytes(&active.doc, &Default::default()).unwrap(), native);
        session.execute("edit.redo", serde_json::json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&redo_document));
    }
    #[test]
    fn oversized_geometry_metadata_keeps_native_preparation_limit() {
        let mut doc = document();
        doc.work_path = Some(photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&vec![(0.5, 1.0); 1024])]));
        let before = doc.clone();
        let error = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap_err();
        assert!(matches!(error, photocraft_format::FormatError::LimitExceeded(_)));
        assert!(error.to_string().contains("native preparation JSON bytes"));
        assert_eq!(doc, before);
    }
    #[test]
    fn invalid_affine_and_early_refresh_cancel_do_not_mutate_document() {
        let mut doc = document();
        let original = doc.clone();
        assert!(transform_geometry_with_interrupt(&mut doc, &Affine { m: [f64::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0] }, &Interrupt::NONE).is_err());
        assert_eq!(doc, original);
        let cancel = || true;
        assert!(matches!(refresh_with_interrupt(&mut doc, Refresh::All, &Interrupt::cancel_only(&cancel)), Err(crate::EngineError::Cancelled)));
        assert_eq!(doc, original);
        refresh_with_interrupt(&mut doc, Refresh::All, &Interrupt::NONE).unwrap();
        assert_eq!(doc, original);
    }
}

#[cfg(test)]
mod controlled_turn_drafts {
    use super::*;
    #[test]
    fn cancelled_turn_entry_preserves_native_document_and_real_redo() {
        let mut session = crate::Session::new();
        // The public new-document API keeps its upstream 8/16/32 contract.
        assert!(session.execute("file.new", serde_json::json!({"width":2,"height":1,"depth":64,"background":"transparent"})).is_err());
        assert!(session.documents().is_empty());
        let mut document = Document::new("Own F64 turn", photocraft_doc::Size::new(2, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F64);
        document.layers.push(Layer::raster("Own F64 raster", document.pixel_format()));
        session.open_document(document, None).unwrap();
        session
            .edit("Own redo", |document, _| {
                document.layers[0].name.push_str(" redo");
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", serde_json::json!({})).unwrap();
        let before = session.active().unwrap().doc.clone();
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let revision = session.active().unwrap().revision;
        let cancelled = || true;
        let control = Interrupt::new(&cancelled, &|_| {});
        assert!(matches!(
            session.edit_with_interrupt("Own controlled flip", &control, |document, _| turn_canvas_with_interrupt(document, Turn::FlipHorizontal, &control)),
            Err(crate::EngineError::Cancelled)
        ));
        let active = session.active().unwrap();
        assert!(active.doc.shares_state(&before));
        assert_eq!(active.revision, revision);
        assert!(active.history.can_redo());
        assert_eq!(photocraft_format::save_to_bytes(&active.doc, &Default::default()).unwrap(), native);
    }
}
