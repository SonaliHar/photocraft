//! Cancellation-aware legacy thumbnail reduction with bounded fallible reduction buffers.
//! Proxy construction and legacy effect preparation retain the documented backend limitations.
use super::*;
use photocraft_raster::{Interrupt, RasterInputError};

/// Same legacy thumbnail values, with cancellation in reduction and encoding loops.
/// Initial metadata cloning and render-band backend work retain interruption limitations.
pub fn thumbnail_with_interrupt(doc: &Document, max_side: u32, control: &Interrupt<'_>) -> Result<Rgba8Image, RasterInputError> {
    let buffer = thumbnail_buffer_with_interrupt(doc, max_side, control)?;
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    rgba8(&buffer, control)
}

/// Checked thumbnail reduction before RGBA8 encoding. This is a display buffer,
/// never an authoritative replacement for current native samples.
pub fn thumbnail_buffer_with_interrupt(doc: &Document, max_side: u32, control: &Interrupt<'_>) -> Result<Buffer, RasterInputError> {
    validate_document(doc, control)?;
    if max_side > 4096 {
        return Err(RasterInputError::LimitExceeded);
    }
    let b = doc.bounds();
    let longest = b.width().max(b.height()).max(1);
    let scale = (max_side as f32 / longest as f32).min(1.0);
    let w = ((b.width() as f32 * scale).round() as u32).max(1);
    let h = ((b.height() as f32 * scale).round() as u32).max(1);
    let k = longest / max_side.max(1).saturating_mul(2);
    let buffer = if k >= 2 && doc.size.area() > PROXY_THUMBNAIL_PIXELS && proxy::proxy_faithful_with_interrupt(doc, control)? {
        let proxy = proxy::try_proxy_document_with_interrupt(doc, k, control)?;
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        reduced(&proxy, proxy.bounds(), w, h, None, 0, control)?
    } else {
        reduced(doc, b, w, h, None, 0, control)?
    };
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    Ok(buffer)
}

/// Checked area-averaged ordinary display reduction, with bounded output and
/// cancellation throughout reduction. Imported GIMP rendering uses its own
/// prepared renderer and is refused here, even for empty output.
pub fn render_reduced_with_interrupt(doc: &Document, w: u32, h: u32, control: &Interrupt<'_>) -> Result<Buffer, RasterInputError> {
    reduced(doc, doc.bounds(), w, h, None, 0, control)
}

/// Checked reduction of an explicit document-plane rectangle, including pixels
/// outside the canvas. Output coordinates start at zero; source data is retained.
pub fn render_reduced_rect_with_interrupt(doc: &Document, rect: Rect, w: u32, h: u32, control: &Interrupt<'_>) -> Result<Buffer, RasterInputError> {
    reduced(doc, rect, w, h, None, 0, control)
}

/// Checked damaged portion of an ordinary reduced display. Missing damage
/// returns an empty buffer only after document and request validation succeed.
pub fn render_reduced_damage_with_interrupt(doc: &Document, w: u32, h: u32, damage: Rect, control: &Interrupt<'_>) -> Result<Buffer, RasterInputError> {
    photocraft_raster::checked_region_byte_len(damage, photocraft_color::PixelFormat::GRAY8)?;
    reduced(doc, doc.bounds(), w, h, Some(damage), 0, control)
}

fn validate_document(doc: &Document, control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    if doc.xcf_state.is_some() {
        return Err(RasterInputError::UnsupportedConversion);
    }
    if doc.size.width > 262144 || doc.size.height > 262144 || doc.size.area() > (1 << 30) {
        return Err(RasterInputError::LimitExceeded);
    }
    proxy::validate_surfaces_with_interrupt(doc, control)
}
fn reduced(doc: &Document, b: Rect, w: u32, h: u32, damage: Option<Rect>, band_rows: i32, control: &Interrupt<'_>) -> Result<Buffer, RasterInputError> {
    validate_document(doc, control)?;
    if w > 4096 || h > 4096 {
        return Err(RasterInputError::LimitExceeded);
    }
    photocraft_raster::checked_region_byte_len(b, photocraft_color::PixelFormat::GRAY8)?;
    let (fw, fh) = (b.width() as usize, b.height() as usize);
    let (w, h) = (w.clamp(1, b.width().max(1)) as usize, h.clamp(1, b.height().max(1)) as usize);
    let full = Rect::from_xywh(0, 0, w as u32, h as u32);
    if fw == 0 || fh == 0 {
        return Ok(Buffer::transparent(if damage.is_some() { Rect::EMPTY } else { full }));
    }
    // Output column / row of each document column / row: output pixel t covers [t·f/n, (t+1)·f/n).
    let span = |t: usize, f: usize, n: usize| (t * f / n, ((t + 1) * f / n).max(t * f / n + 1).min(f));
    // The output pixel whose span holds document column / row `x` (spans are contiguous: n <= f).
    let cell = |x: usize, f: usize, n: usize| ((x + 1) * n).saturating_sub(1) / f;
    // Output pixels to produce and the document area they cover.
    let out = match damage {
        None => full,
        Some(d) => {
            let d = d.intersect(&b);
            if d.is_empty() {
                return Ok(Buffer::transparent(Rect::EMPTY));
            }
            let (x0, x1) = ((d.x0 - b.x0) as usize, (d.x1 - b.x0) as usize);
            let (y0, y1) = ((d.y0 - b.y0) as usize, (d.y1 - b.y0) as usize);
            Rect::new(cell(x0, fw, w) as i32, cell(y0, fh, h) as i32, cell(x1 - 1, fw, w) as i32 + 1, cell(y1 - 1, fh, h) as i32 + 1)
        }
    };
    let (ox0, oy0, ow, oh) = (out.x0 as usize, out.y0 as usize, out.width() as usize, out.height() as usize);
    let (sx0, sx1) = (span(ox0, fw, w).0, span(ox0 + ow - 1, fw, w).1);
    let (sy0, sy1) = (span(oy0, fh, h).0, span(oy0 + oh - 1, fh, h).1);
    let src = Rect::new(b.x0 + sx0 as i32, b.y0 + sy0 as i32, b.x0 + sx1 as i32, b.y0 + sy1 as i32);
    // Preflight known rendered buffers too; pattern/effect preparation retains its
    // separate limits. The renderer itself is still noninterruptible inside a band.
    let rows = if (w, h) == (fw, fh) { src.height() as usize } else { (band_rows_for(src.width(), band_rows) as usize).min(src.height() as usize) };
    let band_bytes = (src.width() as usize).checked_mul(rows).and_then(|n| n.checked_mul(32)).ok_or(RasterInputError::LimitExceeded)?;
    let reduction_bytes = fw
        .checked_add(fh)
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| ow.checked_mul(oh).and_then(|pixels| pixels.checked_mul(20)).and_then(|bytes| n.checked_add(bytes)))
        .ok_or(RasterInputError::LimitExceeded)?;
    let known_peak =
        band_bytes.checked_add(reduction_bytes).and_then(|n| n.checked_add(pattern::PREPARED_PATTERN_BYTES)).ok_or(RasterInputError::LimitExceeded)?;
    if known_peak as u64 > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded);
    }
    if (w, h) == (fw, fh) {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let mut px = render_with_interrupt(doc, src, control)?;
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        px.rect = out;
        return Ok(px);
    }
    let (cols, rows) = (cell_map(fw, w, control)?, cell_map(fh, h, control)?);
    let cols = &cols[sx0..sx1];
    let mut acc = zeros::<[f32; 4]>(ow.checked_mul(oh).ok_or(RasterInputError::LimitExceeded)?, control)?;
    render_bands_with_interrupt(doc, src, band_rows, control, |band| -> Result<(), RasterInputError> {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let (by0, by1) = ((band.rect.y0 - b.y0) as usize, (band.rect.y1 - b.y0) as usize);
        let (t0, t1) = (rows[by0] as usize, rows[by1 - 1] as usize + 1);
        let sw = sx1 - sx0;
        // Each output row sums its source rows in this band (in order, top to bottom).
        let sum_row = |t: usize, out: &mut [[f32; 4]]| -> Result<(), RasterInputError> {
            let (a, z) = span(t, fh, h);
            for y in a.max(by0)..z.min(by1) {
                control.check().map_err(|_| RasterInputError::Cancelled)?;
                let src = band.px.get((y - by0) * sw..(y - by0 + 1) * sw).ok_or(RasterInputError::InvalidBounds)?;
                for (index, (p, &c)) in src.iter().zip(cols).enumerate() {
                    if index.is_multiple_of(4096) {
                        control.check().map_err(|_| RasterInputError::Cancelled)?;
                    }
                    let column = (c as usize).checked_sub(ox0).ok_or(RasterInputError::InvalidBounds)?;
                    let o = out.get_mut(column).ok_or(RasterInputError::InvalidBounds)?;
                    for i in 0..3 {
                        o[i] += p[i] * p[3];
                    }
                    o[3] += p[3];
                }
            }
            Ok(())
        };
        let part = &mut acc[(t0 - oy0) * ow..(t1 - oy0) * ow];
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            part.par_chunks_mut(ow).enumerate().try_for_each(|(i, out)| sum_row(t0 + i, out))?;
        }
        #[cfg(target_arch = "wasm32")]
        for (i, out) in part.chunks_mut(ow).enumerate() {
            sum_row(t0 + i, out)?;
        }
        Ok(())
    })?;
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    for (i, p) in acc.iter_mut().enumerate() {
        if i.is_multiple_of(4096) {
            control.check().map_err(|_| RasterInputError::Cancelled)?;
        }
        let ((x0, x1), (y0, y1)) = (span(ox0 + i % ow, fw, w), span(oy0 + i / ow, fh, h));
        let n = ((y1 - y0) * (x1 - x0)).max(1) as f32;
        let a = p[3];
        *p = if a > 0.0 { [p[0] / a, p[1] / a, p[2] / a, a / n] } else { [0.0; 4] };
    }
    Ok(Buffer { rect: out, px: acc })
}

fn zeros<T: Default + Clone>(len: usize, control: &Interrupt<'_>) -> Result<Vec<T>, RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    let bytes = len.checked_mul(std::mem::size_of::<T>()).ok_or(RasterInputError::LimitExceeded)?;
    if bytes as u64 > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut output = Vec::new();
    output.try_reserve_exact(len).map_err(|_| RasterInputError::AllocationFailed)?;
    if output.capacity().checked_mul(std::mem::size_of::<T>()).is_none_or(|n| n as u64 > photocraft_raster::MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    while output.len() < len {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let end = output.len().saturating_add(4096).min(len);
        output.resize(end, T::default());
    }
    Ok(output)
}
fn cell_map(f: usize, n: usize, control: &Interrupt<'_>) -> Result<Vec<u32>, RasterInputError> {
    if n == 0 || n > f || f > 262144 {
        return Err(RasterInputError::InvalidBounds);
    }
    let mut map = zeros::<u32>(f, control)?;
    for t in 0..n {
        let a = t.checked_mul(f).ok_or(RasterInputError::LimitExceeded)? / n;
        let z = (t + 1).checked_mul(f).ok_or(RasterInputError::LimitExceeded)? / n;
        let range = map.get_mut(a..z.max(a + 1).min(f)).ok_or(RasterInputError::InvalidBounds)?;
        for chunk in range.chunks_mut(4096) {
            control.check().map_err(|_| RasterInputError::Cancelled)?;
            chunk.fill(t as u32);
        }
    }
    Ok(map)
}
fn rgba8(buffer: &Buffer, control: &Interrupt<'_>) -> Result<Rgba8Image, RasterInputError> {
    let count = (buffer.rect.width() as usize).checked_mul(buffer.rect.height() as usize).ok_or(RasterInputError::LimitExceeded)?;
    if buffer.px.len() != count {
        return Err(RasterInputError::LengthMismatch);
    }
    let mut pixels = zeros::<u8>(count.checked_mul(4).ok_or(RasterInputError::LimitExceeded)?, control)?;
    for (index, (output, pixel)) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(&buffer.px).enumerate() {
        if index.is_multiple_of(4096) {
            control.check().map_err(|_| RasterInputError::Cancelled)?;
        }
        for (dst, value) in output.iter_mut().zip(pixel) {
            *dst = (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    Ok(Rgba8Image { width: buffer.rect.width(), height: buffer.rect.height(), pixels })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn reduction_allocations_reject_absurd_counts_before_allocation() {
        assert_eq!(zeros::<[f32; 4]>(usize::MAX, &Interrupt::NONE), Err(RasterInputError::LimitExceeded));
        assert_eq!(cell_map(1, 0, &Interrupt::NONE), Err(RasterInputError::InvalidBounds));
        let doc = Document::new("own rejected band", Size::new(262144, 4096), ColorMode::Rgb, SampleType::U8);
        assert!(matches!(reduced(&doc, doc.bounds(), 1, 1, None, 0, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
    }
    #[test]
    fn rgba_conversion_cancels_mid_loop_and_matches_legacy() {
        let buffer = Buffer { rect: Rect::from_xywh(0, 0, 16384, 1), px: vec![[0.2, -0.0, 1.5, 0.6]; 16384] };
        assert_eq!(rgba8(&buffer, &Interrupt::NONE).unwrap(), buffer.to_rgba8());
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 18;
        assert!(matches!(rgba8(&buffer, &Interrupt::cancel_only(&cancel)), Err(RasterInputError::Cancelled)));
        assert_eq!(buffer.px[0], [0.2, -0.0, 1.5, 0.6]);
    }
    #[test]
    fn inner_cell_map_polls_during_long_single_cell_without_output() {
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 2;
        assert!(matches!(cell_map(262144, 1, &Interrupt::cancel_only(&cancel)), Err(RasterInputError::Cancelled)));
        assert!(polls.load(Ordering::Relaxed) >= 3);
    }

    #[test]
    fn ordinary_preview_preserves_legacy_values() {
        let doc = Document::with_background("own", Size::new(513, 321), ColorMode::Rgb, SampleType::U16, Color::rgba(0.2, 0.6, 0.8, 1.0));
        for side in [1, 17, 256, 1024] {
            assert_eq!(thumbnail_with_interrupt(&doc, side, &Interrupt::NONE).unwrap(), thumbnail(&doc, side).unwrap());
        }
    }
    #[test]
    fn faithful_proxy_preview_preserves_legacy_values() {
        let doc = Document::new("own proxy", Size::new(6000, 3000), ColorMode::Rgb, SampleType::U8);
        assert_eq!(thumbnail_with_interrupt(&doc, 256, &Interrupt::NONE).unwrap(), thumbnail(&doc, 256).unwrap());
    }
    #[test]
    fn late_cancel_discards_preview_without_changing_source() {
        let doc = Document::new("own", Size::new(512, 512), ColorMode::Rgb, SampleType::U8);
        let before = doc.clone();
        let count = AtomicUsize::new(0);
        let cancel = || count.fetch_add(1, Ordering::Relaxed) >= 2;
        assert_eq!(thumbnail_with_interrupt(&doc, 256, &Interrupt::cancel_only(&cancel)), Err(RasterInputError::Cancelled));
        assert_eq!(doc, before);
        assert!(count.load(Ordering::Relaxed) > 2);
    }
    #[test]
    fn checked_real_proxy_consumer_matches_legacy_with_mask_and_selection() {
        use photocraft_color::PixelFormat;
        use photocraft_doc::{Layer, LayerContent, LayerMask};
        let mut doc = Document::new("own real proxy", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U16);
        let mut source = Surface::with_default(PixelFormat { mode: ColorMode::Rgb, sample: SampleType::U16, alpha: true }, &[0.2, 0.6, 0.8, 1.0]);
        source.fill_rect(Rect::new(-256, -256, 512, 512), &[0.9, 0.1, 0.3, 0.5]);
        let mut layer = Layer::new("own source", LayerContent::Raster(source));
        let mut mask = LayerMask::reveal_all();
        std::sync::Arc::make_mut(&mut mask.surface).fill_rect(Rect::new(0, 0, 256, 256), &[0.0]);
        layer.mask = Some(mask);
        doc.layers.push(Layer::group("own group", vec![layer]));
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.fill_rect(Rect::new(0, 0, 256, 256), &[0.5]);
        doc.selection = Some(selection);
        let before = doc.clone();
        assert_eq!(thumbnail_with_interrupt(&doc, 256, &Interrupt::NONE).unwrap(), thumbnail(&doc, 256).unwrap());
        assert_eq!(doc, before);
    }
    #[test]
    fn thumbnail_bounds_nested_tree_before_legacy_walkers() {
        use photocraft_doc::Layer;
        let mut layer = Layer::raster("own leaf", photocraft_color::PixelFormat::RGBA8);
        for _ in 0..101 {
            layer = Layer::group("own nested", vec![layer]);
        }
        let mut doc = Document::new("own too deep", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
        doc.layers.push(layer);
        let before = doc.clone();
        assert!(matches!(thumbnail_with_interrupt(&doc, 32, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        assert_eq!(doc, before);
    }

    #[test]
    fn empty_imported_thumbnail_refuses_unqualified_rendering_before_empty_success() {
        let mut doc = Document::new("own unsupported empty preview", Size::new(0, 0), ColorMode::Rgb, SampleType::U8);
        doc.xcf_state = Some(Default::default());
        let before = doc.clone();
        assert_eq!(thumbnail_with_interrupt(&doc, 16, &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion));
        assert_eq!(doc, before);
    }
}
