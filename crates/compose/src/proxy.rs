//! Downsampled proxy documents: every pixel surface scaled down by an integer factor `k`, so a
//! composite of the proxy approximates the document's composite at 1/k resolution for ~1/k² of
//! the work. Used for interactive previews and for thumbnails of large documents (navigator,
//! channel thumbnails, histograms, file previews), where a full-resolution composite of a 200 MP
//! document would cost seconds and gigabytes.

use photocraft_doc::{Document, Layer, LayerContent, Size, SmartObject};
use photocraft_geom::{Affine, Rect};
use photocraft_raster::Surface;

/// Nearest-neighbour downsample of a surface by integer factor `k` (document coordinates / k).
pub fn downsample(s: &Surface, k: u32) -> Surface {
    let mut out = Surface::with_default(s.format(), &s.default_pixel());
    let b = s.content_bounds();
    if b.is_empty() || k <= 1 {
        return if k <= 1 { s.clone() } else { out };
    }
    // Surfaces span at most ±2^30 px; larger factors reduce everything to one pixel anyway.
    let k = k.min(1 << 20) as i32;
    let bpp = s.format().bytes_per_pixel();
    let (x0, x1) = (b.x0.div_euclid(k), b.x1.div_euclid(k) + i32::from(b.x1.rem_euclid(k) != 0));
    let (y0, y1) = (b.y0.div_euclid(k), b.y1.div_euclid(k) + i32::from(b.y1.rem_euclid(k) != 0));
    let w = x1.saturating_sub(x0).max(0) as usize;
    let mut row = vec![0u8; w * bpp];
    for oy in y0..y1 {
        let src = s.to_interleaved(Rect::new(x0.saturating_mul(k), oy.saturating_mul(k), x1.saturating_mul(k), oy.saturating_mul(k).saturating_add(1)));
        for (i, px) in row.chunks_exact_mut(bpp).enumerate() {
            // Saturated edges (a surface at the i32 limits) can make `src` shorter than expected.
            if let Some(from) = (i * k as usize * bpp).checked_add(bpp).and_then(|end| src.get(i * k as usize * bpp..end)) {
                px.copy_from_slice(from);
            }
        }
        out.write_interleaved(Rect::new(x0, oy, x1, oy + 1), &row);
    }
    out
}

/// Checked raw nearest downsample. Authoritative defaults and sampled pixels are
/// copied bitwise, including wide and nonfinite floating payloads. Known source,
/// destination tile payloads and staging are capped; allocator bookkeeping is not
/// a process-RSS guarantee. The source is never changed on cancellation/error.
pub fn try_downsample_with_interrupt(s: &Surface, k: u32, control: &photocraft_raster::Interrupt<'_>) -> Result<Surface, photocraft_raster::RasterInputError> {
    try_downsample_region_with_interrupt(s, k, None, control)
}
/// Render-only nearest samples inside a requested derived extent. No persistence
/// surface is replaced by this view; masks with feathered outside support must
/// use the full-extent helper. Defaults and all retained samples remain bitwise.
pub(crate) fn try_downsample_render_extent(
    s: &Surface,
    k: u32,
    extent: Rect,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<Surface, photocraft_raster::RasterInputError> {
    if !(2..=1 << 20).contains(&k) {
        return Err(photocraft_raster::RasterInputError::InvalidBounds);
    }
    photocraft_raster::checked_region_byte_len(extent, s.format())?;
    try_downsample_region_with_interrupt(s, k, Some(extent), control)
}
fn try_downsample_region_with_interrupt(
    s: &Surface,
    k: u32,
    extent: Option<Rect>,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<Surface, photocraft_raster::RasterInputError> {
    use photocraft_raster::{MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
    control.check().map_err(|_| E::Cancelled)?;
    if k == 0 {
        return Err(E::InvalidBounds);
    }
    s.try_validate_tile_origins_with(control)?;
    let mut out = Surface::try_with_default_bytes(s.format(), s.default_bytes())?;
    let tile_bytes = 256_u64.checked_mul(256).and_then(|n| n.checked_mul(s.format().bytes_per_pixel() as u64)).ok_or(E::LimitExceeded)?;
    if k == 1 {
        let source = s.resident_payload_bytes()?;
        if source.checked_mul(2).and_then(|n| n.checked_add(tile_bytes * 2)).is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES) {
            return Err(E::LimitExceeded);
        }
        for (coord, tile) in s.tiles() {
            control.check().map_err(|_| E::Cancelled)?;
            out.try_write_interleaved_with(coord.rect(), tile.bytes(), control)?;
        }
        control.check().map_err(|_| E::Cancelled)?;
        return Ok(out);
    }
    let b = s.content_bounds();
    if b.is_empty() {
        return Ok(out);
    }
    let divisor = i64::from(k.min(1 << 20));
    let edge = |value: i32, ceil: bool| -> Result<i32, E> {
        let value = i64::from(value);
        i32::try_from(value.div_euclid(divisor) + i64::from(ceil && value.rem_euclid(divisor) != 0)).map_err(|_| E::InvalidBounds)
    };
    let bounds = Rect::new(edge(b.x0, false)?, edge(b.y0, false)?, edge(b.x1, true)?, edge(b.y1, true)?);
    let bounds = extent.map_or(bounds, |extent| bounds.intersect(&extent));
    if bounds.is_empty() {
        return Ok(out);
    }
    photocraft_raster::checked_region_byte_len(bounds, s.format())?;
    let columns = (i64::from(bounds.x1) - 1).div_euclid(256) - i64::from(bounds.x0).div_euclid(256) + 1;
    let rows = (i64::from(bounds.y1) - 1).div_euclid(256) - i64::from(bounds.y0).div_euclid(256) + 1;
    let tiles = u64::try_from(columns.checked_mul(rows).ok_or(E::LimitExceeded)?).map_err(|_| E::LimitExceeded)?;
    let peak = tiles
        .checked_mul(tile_bytes.checked_add(128).ok_or(E::LimitExceeded)?)
        .and_then(|n| n.checked_add(tile_bytes * 2))
        .and_then(|n| n.checked_add(s.resident_payload_bytes().ok()?))
        .ok_or(E::LimitExceeded)?;
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(E::LimitExceeded);
    }
    let bpp = s.format().bytes_per_pixel();
    for coord in bounds.tiles() {
        control.check().map_err(|_| E::Cancelled)?;
        let region = coord.rect().intersect(&bounds);
        let len = photocraft_raster::checked_region_byte_len(region, s.format())?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).map_err(|_| E::AllocationFailed)?;
        if bytes.capacity() as u64 > tile_bytes {
            return Err(E::LimitExceeded);
        }
        bytes.resize(len, 0);
        let mut cursor = 0usize;
        for y in region.y0..region.y1 {
            control.check().map_err(|_| E::Cancelled)?;
            let sy = i32::try_from(i64::from(y).checked_mul(divisor).ok_or(E::InvalidBounds)?).map_err(|_| E::InvalidBounds)?;
            for x in region.x0..region.x1 {
                if (cursor / bpp).is_multiple_of(4096) {
                    control.check().map_err(|_| E::Cancelled)?;
                }
                let sx = i32::try_from(i64::from(x).checked_mul(divisor).ok_or(E::InvalidBounds)?).map_err(|_| E::InvalidBounds)?;
                let end = cursor.checked_add(bpp).ok_or(E::LimitExceeded)?;
                s.try_read_pixel_bytes(sx, sy, bytes.get_mut(cursor..end).ok_or(E::LengthMismatch)?)?;
                cursor = end;
            }
        }
        out.try_write_interleaved_with(region, &bytes, control)?;
    }
    control.check().map_err(|_| E::Cancelled)?;
    Ok(out)
}

fn shrink_layer(l: &mut Layer, k: u32) {
    if let Some(m) = &mut l.mask {
        m.surface = std::sync::Arc::new(downsample(&m.surface, k));
    }
    if let Some(fc) = &mut l.fill_cache {
        fc.surface = downsample(&fc.surface, k);
    }
    match &mut l.content {
        LayerContent::Raster(s) => *s = downsample(s, k),
        LayerContent::Group(g) => {
            if let Some(ab) = &mut g.artboard {
                let divisor = i64::from(k);
                // Pixel x samples source x*k: both half-open edges round up, including negatives.
                let edge = |v: i32| {
                    let v = i64::from(v);
                    (v.div_euclid(divisor) + i64::from(v.rem_euclid(divisor) != 0)) as i32
                };
                ab.rect = Rect::new(edge(ab.rect.x0), edge(ab.rect.y0), edge(ab.rect.x1), edge(ab.rect.y1));
            }
            for c in &mut g.children {
                shrink_layer(c, k);
            }
        }
        LayerContent::Text(t) => t.cache = t.cache.as_ref().map(|s| downsample(s, k)),
        LayerContent::Shape(sh) => sh.cache = sh.cache.as_ref().map(|s| downsample(s, k)),
        LayerContent::Smart(so) => shrink_smart(so, k),
        LayerContent::Adjustment(_) | LayerContent::Fill(_) => {}
    }
}

/// Scales a smart object with its proxy: the cache and filter mask like any surface, and the
/// placement too. A preview that adds a smart filter re-renders the source through the placement;
/// left in full-size document coordinates, that render came out `k` times too large on the proxy
/// canvas, so the Curves preview on a smart object zoomed in (#5766).
fn shrink_smart(so: &mut SmartObject, k: u32) {
    so.cache = so.cache.as_ref().map(|s| downsample(s, k));
    if let Some(m) = &mut so.filter_mask {
        m.surface = downsample(&m.surface, k);
    }
    let s = 1.0 / f64::from(k);
    so.transform = Affine::scale(s).mul(&so.transform);
    // Row-major source → document map: scaling the document side scales its first two rows.
    if let Some(h) = &mut so.perspective {
        for v in h.iter_mut().take(6) {
            *v *= s;
        }
    }
}

/// A copy of `doc` scaled down by `k` (same layer ids, so adjustments can be swapped in). The
/// selection scales with it, so a filter previewed on the proxy stays inside the (feathered)
/// selection exactly as the full-size result does.
pub fn proxy_document(doc: &Document, k: u32) -> Result<Document, photocraft_raster::RasterInputError> {
    validate_surfaces_with_interrupt(doc, &photocraft_raster::Interrupt::NONE)?;
    Ok(proxy_document_resident(doc, k))
}

pub(crate) fn proxy_document_resident(doc: &Document, k: u32) -> Document {
    let mut p = doc.clone();
    if k <= 1 {
        return p;
    }
    p.size = Size::new(doc.size.width.div_ceil(k), doc.size.height.div_ceil(k));
    for l in &mut p.layers {
        shrink_layer(l, k);
    }
    // Spot channels are printed under Multichannel composites.
    for c in &mut p.channels {
        c.surface = downsample(&c.surface, k);
    }
    p.selection = p.selection.as_ref().map(|s| downsample(s, k));
    if let Some(mask) = &mut p.quick_mask {
        mask.surface = downsample(&mask.surface, k);
    }
    p
}

/// Bounded validation before legacy tree walkers or proxy construction.
pub(crate) fn validate_surfaces_with_interrupt(doc: &Document, control: &photocraft_raster::Interrupt<'_>) -> Result<(), photocraft_raster::RasterInputError> {
    use photocraft_raster::RasterInputError as E;
    fn layer(l: &Layer, depth: usize, count: &mut usize, control: &photocraft_raster::Interrupt<'_>) -> Result<(), E> {
        control.check().map_err(|_| E::Cancelled)?;
        *count = count.checked_add(1).ok_or(E::LimitExceeded)?;
        if depth >= 100 || *count > 100000 {
            return Err(E::LimitExceeded);
        }
        if let Some(surface) = l.surface() {
            surface.try_validate_tile_origins_with(control)?;
        }
        if let Some(mask) = &l.mask {
            mask.surface.try_validate_tile_origins_with(control)?;
        }
        if let Some(cache) = &l.fill_cache {
            cache.surface.try_validate_tile_origins_with(control)?;
        }
        if let Some(children) = l.children() {
            for child in children {
                layer(child, depth + 1, count, control)?;
            }
        }
        Ok(())
    }
    if doc.has_backed_rasters()? {
        return Err(E::UnsupportedConversion);
    }
    let mut count = 0;
    for entry in &doc.layers {
        layer(entry, 0, &mut count, control)?;
    }
    if doc.channels.len() > 100000 || doc.patterns.len() > 100000 {
        return Err(E::LimitExceeded);
    }
    for channel in doc.channels.iter().chain(doc.quick_mask.iter()) {
        channel.surface.try_validate_tile_origins_with(control)?;
    }
    for pattern in &doc.patterns {
        pattern.surface.try_validate_tile_origins_with(control)?;
    }
    if let Some(selection) = &doc.selection {
        selection.try_validate_tile_origins_with(control)?;
    }
    Ok(())
}
/// Same proxy-eligibility decision as the legacy helper without allocating paths.
pub(crate) fn proxy_faithful_with_interrupt(doc: &Document, control: &photocraft_raster::Interrupt<'_>) -> Result<bool, photocraft_raster::RasterInputError> {
    fn faithful(layer: &Layer, control: &photocraft_raster::Interrupt<'_>) -> Result<bool, photocraft_raster::RasterInputError> {
        control.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        if crate::effects::has_effects(layer) || layer.vector_mask.as_ref().is_some_and(|mask| mask.enabled) {
            return Ok(false);
        }
        if let Some(children) = layer.children() {
            for child in children {
                if !faithful(child, control)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    validate_surfaces_with_interrupt(doc, control)?;
    for layer in &doc.layers {
        if !faithful(layer, control)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Cancellation-aware proxy construction with cumulative known raster-payload
/// accounting. Canonical imported item geometry is unsupported; use its prepared
/// renderer instead. Ordinary metadata is retained verbatim. The initial document clone remains
/// noninterruptible; callers must additionally charge its non-raster model heap.
pub fn try_proxy_document_with_interrupt(
    doc: &Document,
    k: u32,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<Document, photocraft_raster::RasterInputError> {
    use photocraft_raster::RasterInputError as E;
    control.check().map_err(|_| E::Cancelled)?;
    // Canonical item/effect geometry requires its own qualified scaled view.
    // Retaining XcfState while changing surfaces would desynchronize current bounds.
    if doc.xcf_state.is_some() {
        return Err(E::UnsupportedConversion);
    }
    if k == 0 || k > (1 << 20) {
        return Err(E::InvalidBounds);
    }
    validate_surfaces_with_interrupt(doc, control)?;
    let mut charge = 0_u64;
    add_charge(&mut charge, doc.name.capacity() as u64)?;
    add_charge(&mut charge, (doc.layers.capacity() as u64).checked_mul(std::mem::size_of::<Layer>() as u64).ok_or(E::LimitExceeded)?)?;
    let mut layers = 0_usize;
    let mut staging = 0_u64;
    for layer in &doc.layers {
        charge_layer(layer, k, 0, &mut layers, &mut charge, &mut staging, control)?;
    }
    for channel in &doc.channels {
        charge_surface(&channel.surface, k, &mut charge, &mut staging, control)?;
    }
    if let Some(selection) = &doc.selection {
        charge_surface(selection, k, &mut charge, &mut staging, control)?;
    }
    // These surfaces are retained by Document::clone without resampling.
    for channel in doc.quick_mask.iter() {
        add_charge(&mut charge, channel.surface.resident_payload_bytes()?)?;
    }
    for pattern in &doc.patterns {
        add_charge(&mut charge, pattern.surface.resident_payload_bytes()?)?;
    }
    add_charge(&mut charge, staging)?;
    control.check().map_err(|_| E::Cancelled)?;
    let mut proxy = doc.clone();
    control.check().map_err(|_| E::Cancelled)?;
    if k == 1 {
        return Ok(proxy);
    }
    proxy.size = Size::new(doc.size.width.div_ceil(k), doc.size.height.div_ceil(k));
    for layer in &mut proxy.layers {
        shrink_layer_with_interrupt(layer, k, control)?;
    }
    for channel in &mut proxy.channels {
        channel.surface = try_downsample_with_interrupt(&channel.surface, k, control)?;
    }
    if let Some(selection) = &proxy.selection {
        proxy.selection = Some(try_downsample_with_interrupt(selection, k, control)?);
    }
    control.check().map_err(|_| E::Cancelled)?;
    Ok(proxy)
}
fn add_charge(total: &mut u64, bytes: u64) -> Result<(), photocraft_raster::RasterInputError> {
    use photocraft_raster::{MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
    *total = total.checked_add(bytes).ok_or(E::LimitExceeded)?;
    if *total > MAX_CHECKED_RASTER_BYTES {
        return Err(E::LimitExceeded);
    }
    Ok(())
}
fn charge_surface(
    surface: &Surface,
    k: u32,
    total: &mut u64,
    staging: &mut u64,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<(), photocraft_raster::RasterInputError> {
    use photocraft_raster::RasterInputError as E;
    surface.try_validate_tile_origins_with(control)?;
    add_charge(total, surface.resident_payload_bytes()?)?;
    let tile = 256_u64 * 256 * surface.format().bytes_per_pixel() as u64;
    *staging = (*staging).max(tile.checked_mul(2).ok_or(E::LimitExceeded)?);
    let bounds = surface.content_bounds();
    if bounds.is_empty() {
        return add_charge(total, surface.default_bytes().len() as u64);
    }
    if k == 1 {
        return add_charge(total, surface.resident_payload_bytes()?);
    }
    let divisor = i64::from(k);
    let x0 = i64::from(bounds.x0).div_euclid(divisor);
    let y0 = i64::from(bounds.y0).div_euclid(divisor);
    let x1 = i64::from(bounds.x1).div_euclid(divisor) + i64::from(i64::from(bounds.x1).rem_euclid(divisor) != 0);
    let y1 = i64::from(bounds.y1).div_euclid(divisor) + i64::from(i64::from(bounds.y1).rem_euclid(divisor) != 0);
    let width = x1 - x0;
    let height = y1 - y0;
    if width > 262144 || height > 262144 {
        return Err(E::LimitExceeded);
    }
    let columns = (x1 - 1).div_euclid(256) - x0.div_euclid(256) + 1;
    let rows = (y1 - 1).div_euclid(256) - y0.div_euclid(256) + 1;
    let tiles = u64::try_from(columns.checked_mul(rows).ok_or(E::LimitExceeded)?).map_err(|_| E::LimitExceeded)?;
    add_charge(total, tiles.checked_mul(tile.checked_add(128).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?)
}
fn charge_layer(
    layer: &Layer,
    k: u32,
    depth: usize,
    count: &mut usize,
    total: &mut u64,
    staging: &mut u64,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<(), photocraft_raster::RasterInputError> {
    use photocraft_raster::RasterInputError as E;
    control.check().map_err(|_| E::Cancelled)?;
    *count = count.checked_add(1).ok_or(E::LimitExceeded)?;
    if depth >= 100 || *count > 100000 {
        return Err(E::LimitExceeded);
    }
    add_charge(total, layer.name.capacity() as u64)?;
    if let Some(surface) = layer.surface() {
        charge_surface(surface, k, total, staging, control)?;
    }
    if let Some(mask) = &layer.mask {
        // Immutable source and generated proxy mask Arc owners are separate allocations.
        add_charge(total, ((std::mem::size_of::<Surface>() + 128) * 2) as u64)?;
        charge_surface(&mask.surface, k, total, staging, control)?;
    }
    if let Some(cache) = &layer.fill_cache {
        charge_surface(&cache.surface, k, total, staging, control)?;
    }
    if let LayerContent::Group(group) = &layer.content {
        add_charge(total, (group.children.capacity() as u64).checked_mul(std::mem::size_of::<Layer>() as u64).ok_or(E::LimitExceeded)?)?;
        for child in &group.children {
            charge_layer(child, k, depth + 1, count, total, staging, control)?;
        }
    }
    Ok(())
}
fn shrink_layer_with_interrupt(layer: &mut Layer, k: u32, control: &photocraft_raster::Interrupt<'_>) -> Result<(), photocraft_raster::RasterInputError> {
    use photocraft_raster::RasterInputError as E;
    control.check().map_err(|_| E::Cancelled)?;
    if let Some(mask) = &mut layer.mask {
        mask.surface = std::sync::Arc::new(try_downsample_with_interrupt(&mask.surface, k, control)?);
    }
    if let Some(cache) = &mut layer.fill_cache {
        cache.surface = try_downsample_with_interrupt(&cache.surface, k, control)?;
    }
    match &mut layer.content {
        LayerContent::Raster(surface) => *surface = try_downsample_with_interrupt(surface, k, control)?,
        LayerContent::Group(group) => {
            if let Some(artboard) = &mut group.artboard {
                let divisor = i64::from(k);
                let edge = |v: i32| (i64::from(v).div_euclid(divisor) + i64::from(i64::from(v).rem_euclid(divisor) != 0)) as i32;
                artboard.rect = Rect::new(edge(artboard.rect.x0), edge(artboard.rect.y0), edge(artboard.rect.x1), edge(artboard.rect.y1));
            }
            for child in &mut group.children {
                shrink_layer_with_interrupt(child, k, control)?;
            }
        }
        LayerContent::Text(text) => {
            if let Some(cache) = &text.cache {
                text.cache = Some(try_downsample_with_interrupt(cache, k, control)?);
            }
        }
        LayerContent::Shape(shape) => {
            if let Some(cache) = &shape.cache {
                shape.cache = Some(try_downsample_with_interrupt(cache, k, control)?);
            }
        }
        LayerContent::Smart(smart) => {
            if let Some(cache) = &smart.cache {
                smart.cache = Some(try_downsample_with_interrupt(cache, k, control)?);
            }
        }
        LayerContent::BackedRaster(_) => return Err(photocraft_raster::RasterInputError::UnsupportedConversion),
        LayerContent::Adjustment(_) | LayerContent::Fill(_) => {}
    }
    Ok(())
}

/// Whether a proxy composite is a faithful reduction of `doc`: layer effects (whose sizes are in
/// document pixels) and nonempty vector masks (paths in document coordinates) would not scale
/// with it. Empty vector masks have constant coverage, including inversion and density.
/// Nonempty paths can still anchor fill layouts even when their mask density is zero.
pub fn proxy_faithful(doc: &Document) -> bool {
    if doc.has_backed_rasters().unwrap_or(true) {
        return false;
    }
    doc.walk().iter().all(|(_, _, l)| !crate::effects::has_effects(l) && l.vector_mask.as_ref().is_none_or(|v| !v.enabled || v.path.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
    use photocraft_doc::vector::{Knot, Path, Subpath, VectorMask};
    use photocraft_doc::{Effect, Fill, GradientStyle, LayerMask};

    fn two_colors(size: Size, depth: SampleType) -> Document {
        let mut doc = Document::new("mask", size, ColorMode::Rgb, depth);
        let format = doc.pixel_format();
        for (name, color) in [("blue", [0.0, 0.0, 1.0, 1.0]), ("red", [1.0, 0.0, 0.0, 1.0])] {
            doc.layers.push(Layer::new(name, LayerContent::Raster(Surface::with_default(format, &color))));
        }
        doc
    }

    fn assert_same_pixels(actual: &crate::Buffer, expected: &crate::Buffer) {
        assert_same_pixels_within(actual, expected, 1e-6);
    }

    fn assert_same_pixels_within(actual: &crate::Buffer, expected: &crate::Buffer, tolerance: f32) {
        assert_eq!(actual.rect, expected.rect);
        assert_eq!(actual.px.len(), expected.px.len());
        for (a, b) in actual.px.iter().zip(&expected.px) {
            assert!(a.iter().zip(b).all(|(a, b)| (a - b).abs() <= tolerance), "{a:?} != {b:?}");
        }
    }

    fn u16_coverage(value: f32) -> f32 {
        // Feathered masks use the existing U16 combined-mask surface, including its default.
        (value.clamp(0.0, 1.0) * 65535.0 + 0.5).floor() / 65535.0
    }

    fn opacity_reference(doc: &Document, coverage: f32) -> Document {
        let mut reference = doc.clone();
        reference.layers[1].vector_mask = None;
        reference.layers[1].opacity *= coverage;
        reference
    }

    #[test]
    fn render_extent_preserves_sample_bits_and_skips_unrendered_tile_columns() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32, SampleType::U32, SampleType::F64] {
            let format = PixelFormat::new(ColorMode::Rgb, sample, true);
            let mut source = Surface::new(format);
            source.fill_rect(Rect::new(-513, -9, 39, 27), &[0.73, 0.18, 0.51, 0.0]);
            let before = source.clone();
            let extent = Rect::new(0, 0, 11, 9);
            let full = try_downsample_with_interrupt(&source, 2, &photocraft_raster::Interrupt::NONE).unwrap();
            let view = try_downsample_render_extent(&source, 2, extent, &photocraft_raster::Interrupt::NONE).unwrap();
            assert_eq!(view.tile_count(), 1);
            assert!(full.tile_count() > view.tile_count());
            assert_eq!(view.default_bytes(), source.default_bytes());
            let (mut a, mut b) = (vec![0; format.bytes_per_pixel()], vec![0; format.bytes_per_pixel()]);
            for y in extent.y0..extent.y1 {
                for x in extent.x0..extent.x1 {
                    view.try_read_pixel_bytes(x, y, &mut a).unwrap();
                    full.try_read_pixel_bytes(x, y, &mut b).unwrap();
                    assert_eq!(a, b);
                }
            }
            assert_eq!(source, before);
            let cancel = || true;
            assert!(matches!(
                try_downsample_render_extent(&source, 2, extent, &photocraft_raster::Interrupt::cancel_only(&cancel)),
                Err(photocraft_raster::RasterInputError::Cancelled)
            ));
            assert!(try_downsample_render_extent(&source, 1, extent, &photocraft_raster::Interrupt::NONE).is_err());
        }
    }
    #[test]
    fn downsample_picks_every_kth_pixel() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 8, 8), &[1.0, 0.0, 0.0, 1.0]);
        s.write_pixel(4, 4, &[0.0, 0.0, 1.0, 1.0]);
        let d = downsample(&s, 4);
        assert_eq!(d.pixel(0, 0), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(d.pixel(1, 1), vec![0.0, 0.0, 1.0, 1.0]);
        assert_eq!(d.pixel(2, 2)[3], 0.0);
    }

    #[test]
    fn downsample_handles_negative_coordinates() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(-8, -8, 0, 0), &[0.0, 1.0, 0.0, 1.0]);
        let d = downsample(&s, 4);
        assert_eq!(d.pixel(-1, -1), vec![0.0, 1.0, 0.0, 1.0]);
        assert_eq!(d.pixel(0, 0)[3], 0.0);
    }

    #[test]
    fn proxy_keeps_structure_and_ids() {
        let doc = Document::with_background("d", Size::new(4000, 3000), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let p = proxy_document(&doc, 4).unwrap();
        assert_eq!((p.size.width, p.size.height), (1000, 750));
        assert_eq!(p.layers[0].id, doc.layers[0].id);
        assert_eq!(p.layers[0].surface().unwrap().pixel(999, 749), vec![1.0; 4]);
        assert!(proxy_faithful(&doc));
    }

    /// #5766: a smart object re-rendered on the proxy (a smart filter preview) goes through its
    /// placement, which must shrink with the canvas, cache and filter mask.
    #[test]
    fn proxy_scales_smart_object_placement() {
        use photocraft_doc::SmartSource;
        let mut doc = Document::new("smart", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
        let mut cache = Surface::new(doc.pixel_format());
        cache.fill_rect(Rect::new(16, 16, 32, 48), &[0.0, 0.0, 0.0, 1.0]);
        let source = SmartSource::Embedded { file_name: "s.png".into(), bytes: std::sync::Arc::new(Vec::new()) };
        let mut sm = SmartObject::new(source, Affine::translate(16.0, 16.0).mul(&Affine::scale(2.0)), Some(cache));
        let mut mask = LayerMask::hide_all();
        mask.surface.fill_rect(Rect::new(16, 16, 32, 32), &[1.0]);
        sm.filter_mask = Some(mask);
        sm.perspective = Some([2.0, 0.0, 16.0, 0.0, 2.0, 16.0, 0.0, 0.0, 1.0]);
        doc.layers.push(Layer::new("smart", LayerContent::Smart(sm)));
        let p = proxy_document(&doc, 4);
        let LayerContent::Smart(sm) = &p.layers[0].content else { panic!("a smart object") };
        assert_eq!(sm.transform, Affine { m: [0.5, 0.0, 0.0, 0.5, 4.0, 4.0] });
        assert_eq!(sm.perspective, Some([0.5, 0.0, 4.0, 0.0, 0.5, 4.0, 0.0, 0.0, 1.0]));
        assert_eq!(sm.cache.as_ref().unwrap().content_bounds(), Rect::new(4, 4, 8, 12));
        let mask = &sm.filter_mask.as_ref().unwrap().surface;
        assert_eq!((mask.pixel(4, 4)[0], mask.pixel(8, 8)[0]), (1.0, 0.0));
        // The document keeps its full-size placement.
        let LayerContent::Smart(orig) = &doc.layers[0].content else { panic!("a smart object") };
        assert_eq!(orig.transform.m[4], 16.0);
    }

    #[test]
    fn proxy_scales_the_selection() {
        let mut doc = Document::with_background("d", Size::new(64, 64), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut sel = Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 32, 64), &[1.0]);
        doc.selection = Some(sel);
        let p = proxy_document(&doc, 4).unwrap();
        let s = p.selection.as_ref().expect("selection kept");
        assert_eq!((s.pixel(7, 15)[0], s.pixel(8, 0)[0]), (1.0, 0.0));
        assert!(proxy_document(&Document::new("n", Size::new(8, 8), ColorMode::Rgb, SampleType::U8), 4).unwrap().selection.is_none());
    }
    #[test]
    fn checked_downsample_matches_legacy_at_negative_origins() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat { mode: ColorMode::Rgb, sample, alpha: true };
            let mut source = Surface::new(format);
            source.fill_rect(Rect::new(-9, -7, 15, 19), &[0.2, 0.6, 0.8, 0.5]);
            source.write_pixel(-4, 4, &[0.9, 0.1, 0.3, 0.0]);
            for k in [1, 2, 4, 17] {
                assert_eq!(try_downsample_with_interrupt(&source, k, &photocraft_raster::Interrupt::NONE).unwrap(), downsample(&source, k));
            }
        }
    }
    #[test]
    fn checked_downsample_retains_wide_defaults_and_hidden_bits() {
        for sample in [SampleType::U32, SampleType::F64] {
            let format = PixelFormat { mode: ColorMode::Grayscale, sample, alpha: true };
            let default = if sample == SampleType::U32 {
                [0x80000001_u32.to_ne_bytes(), 0_u32.to_ne_bytes()].concat()
            } else {
                [f64::from_bits(0x7ff8000000000042).to_ne_bytes(), (-0.0_f64).to_ne_bytes()].concat()
            };
            let raw = if sample == SampleType::U32 {
                [0x12345679_u32.to_ne_bytes(), 0_u32.to_ne_bytes()].concat()
            } else {
                [f64::from_bits(0x3fd0000000000001).to_ne_bytes(), (-0.0_f64).to_ne_bytes()].concat()
            };
            let mut source = Surface::try_with_default_bytes(format, &default).unwrap();
            source.try_write_interleaved_with(Rect::new(-4, 4, -3, 5), &raw, &photocraft_raster::Interrupt::NONE).unwrap();
            let before = source.clone();
            let output = try_downsample_with_interrupt(&source, 4, &photocraft_raster::Interrupt::NONE).unwrap();
            assert_eq!(output.default_bytes(), default);
            let mut pixel = vec![0; raw.len()];
            output.try_read_pixel_bytes(-1, 1, &mut pixel).unwrap();
            assert_eq!(pixel, raw);
            output.try_read_pixel_bytes(1000, 1000, &mut pixel).unwrap();
            assert_eq!(pixel, default);
            assert_eq!(source, before);
            assert_eq!(try_downsample_with_interrupt(&source, 1, &photocraft_raster::Interrupt::NONE).unwrap(), source);
        }
    }
    #[test]
    fn checked_downsample_cancels_and_rejects_hostile_bounds_atomically() {
        use photocraft_raster::{Interrupt, RasterInputError};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut source = Surface::new(PixelFormat::RGBA8);
        source.fill_rect(Rect::new(-256, -256, 256, 256), &[0.3, 0.6, 0.9, 1.0]);
        let before = source.clone();
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 10;
        assert!(matches!(try_downsample_with_interrupt(&source, 2, &Interrupt::cancel_only(&cancel)), Err(RasterInputError::Cancelled)));
        assert_eq!(source, before);
        assert!(matches!(try_downsample_with_interrupt(&source, 0, &Interrupt::NONE), Err(RasterInputError::InvalidBounds)));
        source.tile_mut(photocraft_geom::TileCoord::new(i32::MAX, 0));
        assert!(matches!(try_downsample_with_interrupt(&source, 4, &Interrupt::NONE), Err(RasterInputError::InvalidBounds)));
        let mut sparse = Surface::new(PixelFormat::RGBA8);
        sparse.write_pixel(0, 0, &[1.0; 4]);
        sparse.write_pixel(524000, 524000, &[1.0; 4]);
        assert!(matches!(try_downsample_with_interrupt(&sparse, 2, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
    }
    #[test]
    fn checked_document_proxy_preserves_structure_and_legacy_pixels() {
        use photocraft_raster::Interrupt;
        let mut doc = Document::with_background("own checked proxy", Size::new(513, 321), ColorMode::Rgb, SampleType::U16, Color::WHITE);
        let layer = doc.layers.remove(0);
        doc.layers.push(Layer::group("own group", vec![layer]));
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.fill_rect(Rect::new(-4, -4, 128, 160), &[0.5]);
        doc.selection = Some(selection);
        let before = doc.clone();
        for k in [1, 2, 17] {
            assert_eq!(try_proxy_document_with_interrupt(&doc, k, &Interrupt::NONE).unwrap(), proxy_document(&doc, k).unwrap());
        }
        assert_eq!(doc, before);
    }
    #[test]
    fn checked_document_proxy_rejects_combined_payload_and_late_cancel() {
        use photocraft_raster::{Interrupt, RasterInputError};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut doc = Document::new("own combined", Size::new(20000, 20000), ColorMode::Rgb, SampleType::U8);
        let mut surface = Surface::new(PixelFormat::RGBA8);
        surface.write_pixel(0, 0, &[1.0; 4]);
        surface.write_pixel(19999, 19999, &[1.0; 4]);
        for _ in 0..6 {
            doc.layers.push(Layer::new("own sparse", LayerContent::Raster(surface.clone())));
        }
        let before = doc.clone();
        assert!(matches!(try_proxy_document_with_interrupt(&doc, 2, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        assert_eq!(doc, before);
        let doc = Document::with_background("own cancelled clone", Size::new(512, 512), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let before = doc.clone();
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 20;
        assert!(matches!(try_proxy_document_with_interrupt(&doc, 2, &Interrupt::cancel_only(&cancel)), Err(RasterInputError::Cancelled)));
        assert_eq!(doc, before);
    }
    #[test]
    fn checked_proxy_rejects_unscaled_canonical_item_state() {
        let mut doc = Document::new("own canonical state", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
        doc.xcf_state = Some(Default::default());
        let before = doc.clone();
        assert!(matches!(
            try_proxy_document_with_interrupt(&doc, 2, &photocraft_raster::Interrupt::NONE),
            Err(photocraft_raster::RasterInputError::UnsupportedConversion)
        ));
        assert_eq!(doc, before);
    }

    #[test]
    fn proxy_scales_quick_mask_at_every_depth() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = Document::new("mask", Size::new(16, 12), ColorMode::Rgb, depth);
            let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
            surface.fill_rect(Rect::new(0, 0, 8, 12), &[1.0]);
            let mut mask = photocraft_doc::AlphaChannel::new("Quick Mask", surface);
            mask.opacity = 0.25;
            doc.quick_mask = Some(mask);
            for k in [1, 2, 4] {
                let proxy = proxy_document(&doc, k);
                let mask = proxy.quick_mask.as_ref().unwrap();
                assert_eq!(mask.surface.pixel((8 / k - 1) as i32, 0), vec![1.0]);
                assert_eq!(mask.surface.pixel((8 / k) as i32, 0), vec![0.0], "depth {depth:?}, factor {k}");
                assert_eq!(mask.opacity, 0.25);
                assert_eq!(mask.surface.format().sample, depth);
            }
            assert_eq!(doc.quick_mask.as_ref().unwrap().surface.pixel(7, 0), vec![1.0], "source stays intact");
        }
    }

    #[test]
    fn proxy_accepts_empty_paths_but_keeps_geometry_and_effect_guards() {
        let mut doc = two_colors(Size::new(32, 16), SampleType::F32);
        doc.layers[1].vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::default()])));
        assert!(proxy_faithful(&doc));
        // A lone knot has no fillable segments, but still carries document coordinates.
        doc.layers[1].vector_mask.as_mut().unwrap().path.subpaths[0].knots.push(Knot::corner(12.0, 8.0));
        assert!(!proxy_faithful(&doc));
        doc.layers[1].vector_mask.as_mut().unwrap().enabled = false;
        assert!(proxy_faithful(&doc));
        doc.layers[1].vector_mask = Some(VectorMask::new(Path::default()));
        doc.layers[1].effects.items.push(Effect::default_drop_shadow());
        doc.layers[1].effects.enabled = true;
        assert!(!proxy_faithful(&doc));
        doc.layers[1].effects.items[0].set_enabled(false);
        assert!(proxy_faithful(&doc));
    }

    #[test]
    fn empty_vector_masks_keep_constant_coverage_at_every_depth() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = two_colors(Size::new(32, 16), depth);
            for path in [Path::default(), Path::new(vec![Subpath::default()])] {
                for inverted in [false, true] {
                    for density in [0.0, 0.25, 1.0] {
                        for feather in [0.0, 4.0] {
                            let mut mask = VectorMask::new(path.clone());
                            mask.path.inverted = inverted;
                            mask.density = density;
                            mask.feather = feather;
                            doc.layers[1].vector_mask = Some(mask);
                            assert!(proxy_faithful(&doc));
                            let coverage = if inverted { 1.0 - density } else { 1.0 };
                            let coverage = if feather > 0.0 { u16_coverage(coverage) } else { coverage };
                            let expected = [coverage, 0.0, 1.0 - coverage, 1.0];
                            let full = crate::render(&doc, doc.bounds());
                            for pixel in &full.px {
                                assert!(pixel.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-6), "{depth:?} {inverted} {density} {feather}: {pixel:?}");
                            }
                            let proxy = proxy_document(&doc, 4);
                            assert_eq!(proxy.layers[1].vector_mask, doc.layers[1].vector_mask);
                            let reference = proxy_document(&opacity_reference(&doc, coverage), 4);
                            assert_same_pixels(&crate::render(&proxy, proxy.bounds()), &crate::render(&reference, reference.bounds()));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn empty_vector_mask_multiplies_a_feathered_pixel_mask_on_the_proxy() {
        let mut doc = two_colors(Size::new(32, 16), SampleType::F32);
        let mut surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.25]);
        surface.fill_rect(Rect::new(0, 0, 16, 16), &[0.75]);
        doc.layers[1].mask = Some(LayerMask { surface, density: 0.5, feather: 2.0, ..LayerMask::reveal_all() });
        let mut mask = VectorMask::new(Path { inverted: true, ..Path::default() });
        mask.density = 0.75;
        mask.feather = 8.0;
        doc.layers[1].vector_mask = Some(mask);
        assert!(proxy_faithful(&doc));
        let reference = opacity_reference(&doc, 0.25);
        for k in [1, 2, 4] {
            let (actual, expected) = (proxy_document(&doc, k), proxy_document(&reference, k));
            // Q(0.25 * pixel) versus 0.25 * Q(pixel): U16 rounding differs by at most
            // 0.625 / 65535. Allow one U16 level plus a small float arithmetic allowance.
            assert_same_pixels_within(&crate::render(&actual, actual.bounds()), &crate::render(&expected, expected.bounds()), 1.0 / 65535.0 + 1e-6);
        }
    }

    #[test]
    fn empty_vector_masks_keep_the_pixel_mask_gradient_frame() {
        let mut doc = two_colors(Size::new(32, 16), SampleType::F32);
        doc.layers[1].content = LayerContent::Fill(Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], 0.0, 1.0, GradientStyle::Linear, false));
        let mut pixel = LayerMask::hide_all();
        pixel.surface.fill_rect(Rect::new(8, 4, 24, 12), &[1.0]);
        doc.layers[1].mask = Some(pixel);
        for inverted in [false, true] {
            let mut mask = VectorMask::new(Path { inverted, ..Path::default() });
            mask.density = 0.5;
            mask.feather = 3.0;
            doc.layers[1].vector_mask = Some(mask);
            assert!(proxy_faithful(&doc));
            assert_eq!(crate::fill_frame(&doc.layers[1], doc.bounds()), Rect::new(8, 4, 24, 12));
            let actual = proxy_document(&doc, 4);
            assert_eq!(crate::fill_frame(&actual.layers[1], actual.bounds()), Rect::new(2, 1, 6, 3));
            let reference = proxy_document(&opacity_reference(&doc, u16_coverage(if inverted { 0.5 } else { 1.0 })), 4);
            assert_same_pixels(&crate::render(&actual, actual.bounds()), &crate::render(&reference, reference.bounds()));
        }
    }

    #[test]
    fn zero_density_geometry_still_blocks_the_proxy_and_anchors_a_gradient() {
        let mut doc = Document::new("gradient", Size::new(64, 32), ColorMode::Rgb, SampleType::F32);
        let fill = Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], 0.0, 1.0, GradientStyle::Linear, false);
        let mut layer = Layer::new("gradient", LayerContent::Fill(fill));
        let path = Path::new(vec![Subpath::polygon(&[(16.0, 8.0), (48.0, 8.0), (48.0, 24.0), (16.0, 24.0)])]);
        let mut mask = VectorMask::new(path);
        mask.density = 0.0;
        layer.vector_mask = Some(mask);
        doc.layers.push(layer);
        assert!(!proxy_faithful(&doc));
        assert_eq!(crate::fill_frame(&doc.layers[0], doc.bounds()), Rect::new(16, 8, 48, 24));
        // Density zero reveals all pixels, but the unscaled path would still move the fill.
        let forced_proxy = proxy_document(&doc, 4);
        let exact = crate::render_reduced(&doc, 16, 8);
        let wrong = crate::render(&forced_proxy, forced_proxy.bounds());
        assert!(exact.px.iter().zip(&wrong.px).any(|(a, b)| (a[0] - b[0]).abs() > 0.1));
    }

    #[test]
    fn large_thumbnail_keeps_inverted_empty_mask_coverage() {
        // Default-only surfaces exercise the large-document branch without allocating 17 MP.
        let mut doc = two_colors(Size::new(4200, 4200), SampleType::F32);
        let mut mask = VectorMask::new(Path { inverted: true, ..Path::default() });
        mask.density = 0.75;
        mask.feather = 8.0;
        doc.layers[1].vector_mask = Some(mask);
        assert!(doc.size.area() > crate::PROXY_THUMBNAIL_PIXELS);
        assert!(proxy_faithful(&doc));
        let actual = crate::thumbnail_buffer(&doc, 4);
        assert_eq!(actual.rect, Rect::new(0, 0, 4, 4));
        let coverage = u16_coverage(0.25);
        assert_eq!(actual.px, vec![[coverage, 0.0, 1.0 - coverage, 1.0]; 16]);
    }
}
