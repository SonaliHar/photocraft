//! Editable import of admitted XCF rendering semantics. Nothing executes GIMP resources.
use crate::{ImportOrigin, ImportResult, IoError};
use photocraft_doc::{
    AlphaChannel, BlendMode, ColorMode, ColorTable, Document, ImportCompositing, IndexedSource, Layer, LayerContent, LayerMask, PixelFormat, Rect,
    SampleEncoding, SampleType, Size, SourceColorContext,
};
use photocraft_raster::{Interrupt, Surface};
use photocraft_xcf::{DecodeOptions, Drawable, Encoding, ErrorKind, Limits, Model, Plane, Sample, XcfError, XcfFile};
use std::sync::Arc;

/// Common import contract, including cancellation and a conservative shared resident budget.
pub fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult, IoError> {
    import_with_limits(name, bytes, ctl, Limits::default())
}
/// Same admission rules with tighter budgets, useful for untrusted-input tests and callers.
pub fn import_with_limits(name: &str, bytes: &[u8], ctl: &Interrupt<'_>, mut limits: Limits) -> Result<ImportResult, IoError> {
    limits.group_depth = limits.group_depth.min(photocraft_doc::MAX_GROUP_DEPTH);
    let cancelled = || ctl.cancelled();
    let progress = |v| ctl.progress(v * 0.7);
    let options = DecodeOptions { limits, cancelled: Some(&cancelled), progress: Some(&progress) };
    let file = match photocraft_xcf::decode(bytes, &options) {
        Err(e) if e.kind == ErrorKind::Cancelled => return Err(IoError::Cancelled),
        other => other?,
    };
    map(name, file, ctl, options.limits.resident_bytes.min(Limits::default().resident_bytes)).map(|(result, _)| result)
}
/// Development measurement of the same parser/adapter, without a different import path.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct ImportStats {
    pub decode: std::time::Duration,
    pub mapping: std::time::Duration,
    /// Conservatively charged peak, including borrowed input and simultaneous staging/output.
    pub peak_charged_bytes: usize,
}
#[cfg(not(target_arch = "wasm32"))]
pub fn import_with_stats(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<(ImportResult, ImportStats), IoError> {
    let cancelled = || ctl.cancelled();
    let progress = |v| ctl.progress(v * 0.7);
    let options = DecodeOptions { cancelled: Some(&cancelled), progress: Some(&progress), ..Default::default() };
    let before = std::time::Instant::now();
    let file = match photocraft_xcf::decode(bytes, &options) {
        Err(e) if e.kind == ErrorKind::Cancelled => return Err(IoError::Cancelled),
        other => other?,
    };
    let decode = before.elapsed();
    let before = std::time::Instant::now();
    let (result, peak_charged_bytes) = map(name, file, ctl, options.limits.resident_bytes)?;
    Ok((result, ImportStats { decode, mapping: before.elapsed(), peak_charged_bytes }))
}
struct Budget {
    used: usize,
    max: usize,
    peak: usize,
}
impl Budget {
    fn release(&mut self, n: usize) {
        self.used = self.used.saturating_sub(n);
    }
    fn charge(&mut self, n: usize) -> Result<(), IoError> {
        self.used = self.used.checked_add(n).ok_or_else(|| IoError::Unsupported("XCF mapping memory overflow".into()))?;
        self.peak = self.peak.max(self.used);
        if self.used > self.max {
            return Err(IoError::Xcf(XcfError {
                kind: ErrorKind::Limit,
                offset: 0,
                item: String::new(),
                message: "XCF mapping/provenance resident memory budget exceeded".into(),
            }));
        }
        Ok(())
    }
    fn bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
        self.charge(n)?;
        let mut v = Vec::new();
        v.try_reserve_exact(n).map_err(|_| IoError::Unsupported("XCF mapping allocation failed".into()))?;
        v.resize(n, 0);
        Ok(v)
    }
}
fn unsupported(d: &Drawable, reason: impl std::fmt::Display) -> IoError {
    IoError::Unsupported(format!("XCF item '{}': {reason}", short(&d.name)))
}
fn short(s: &str) -> String {
    s.chars().take(80).collect()
}
fn encoding(e: Encoding) -> SampleEncoding {
    match e {
        Encoding::Linear => SampleEncoding::Linear,
        Encoding::NonlinearProfile => SampleEncoding::NonlinearProfile,
        Encoding::Perceptual => SampleEncoding::Perceptual,
    }
}
fn working(d: &Drawable, n: i32) -> Result<SampleEncoding, IoError> {
    match n.checked_abs() {
        Some(0) if matches!(d.mode, 0 | 28 | 61) && d.composite_space != 0 => working(d, d.composite_space),
        Some(1) => Ok(SampleEncoding::Linear),
        Some(2) => Ok(SampleEncoding::NonlinearProfile),
        Some(4) => Ok(SampleEncoding::Perceptual),
        _ => Err(unsupported(d, format!("unsupported blend/composite space {n}"))),
    }
}
fn admit(d: &Drawable) -> Result<(), IoError> {
    if let Some(reason) = d.unsupported.first() {
        return Err(unsupported(d, reason));
    }
    if !(matches!(d.mode, 0 | 28) || d.group && d.mode == 61) {
        return Err(unsupported(d, format!("blend mode {} is unsupported; no verified stored-raster fallback", d.mode)));
    }
    if d.composite_mode.checked_abs() != Some(1) {
        return Err(unsupported(d, format!("composite mode {} is unsupported", d.composite_mode)));
    }
    working(d, d.composite_space)?;
    working(d, d.blend_space)?;
    if let Some(m) = &d.mask {
        if !m.unsupported.is_empty() {
            return Err(unsupported(m, "unsupported layer mask semantics"));
        }
        if m.width != d.width || m.height != d.height {
            return Err(unsupported(d, "mask/layer dimensions differ"));
        }
    }
    Ok(())
}
fn bounds(d: &Drawable) -> Result<Rect, IoError> {
    let x1 = d
        .offset
        .0
        .checked_add(i32::try_from(d.width).map_err(|_| unsupported(d, "width exceeds coordinates"))?)
        .ok_or_else(|| unsupported(d, "horizontal bounds overflow"))?;
    let y1 = d
        .offset
        .1
        .checked_add(i32::try_from(d.height).map_err(|_| unsupported(d, "height exceeds coordinates"))?)
        .ok_or_else(|| unsupported(d, "vertical bounds overflow"))?;
    let r = Rect::new(d.offset.0, d.offset.1, x1, y1);
    if [r.x0, r.x1, r.y0, r.y1].iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(v)) {
        return Err(unsupported(d, "offsets exceed safe raster coordinates"));
    }
    Ok(r)
}
fn depth(s: Sample) -> SampleType {
    match s {
        Sample::U8 => SampleType::U8,
        Sample::U16 => SampleType::U16,
        Sample::F16 | Sample::F32 => SampleType::F32,
    }
}
fn surface(b: &mut Budget, plane: &Plane, r: Rect, mode: ColorMode, alpha: bool, bytes: &[u8], ctl: &Interrupt) -> Result<Surface, IoError> {
    let fmt = PixelFormat::new(mode, depth(plane.sample), alpha);
    let ts = photocraft_geom::TILE_SIZE as usize;
    // Includes padded tiles, tile/tree overhead, and the ingestion API's temporary tile buffer.
    let cols = (r.x1 - 1).div_euclid(ts as i32) - r.x0.div_euclid(ts as i32) + 1;
    let rows = (r.y1 - 1).div_euclid(ts as i32) - r.y0.div_euclid(ts as i32) + 1;
    let tiles = (cols as usize).checked_mul(rows as usize).ok_or_else(|| IoError::Unsupported("XCF tile count overflow".into()))?;
    let tile_bytes =
        ts.checked_mul(ts).and_then(|n| n.checked_mul(fmt.bytes_per_pixel())).ok_or_else(|| IoError::Unsupported("XCF tile bytes overflow".into()))?;
    b.charge(
        tiles.checked_mul(tile_bytes + 128).and_then(|n| n.checked_add(tile_bytes)).ok_or_else(|| IoError::Unsupported("XCF resident tile overflow".into()))?,
    )?;
    Surface::try_from_interleaved_with(fmt, r, bytes, ctl).map_err(|e| {
        if matches!(e, photocraft_raster::RasterInputError::Cancelled) {
            IoError::Cancelled
        } else {
            IoError::Unsupported(format!("XCF raster ingestion: {e}"))
        }
    })
}
fn raster(d: &Drawable, plane: &Plane, palette: &[[u8; 3]], b: &mut Budget, ctl: &Interrupt) -> Result<(Surface, Option<IndexedSource>), IoError> {
    let r = bounds(d)?;
    let mode = match d.model {
        Model::Rgb => ColorMode::Rgb,
        Model::Gray => ColorMode::Grayscale,
        Model::Indexed => ColorMode::Rgb,
    };
    if d.model == Model::Indexed {
        let n = (d.width as usize).checked_mul(d.height as usize).ok_or_else(|| unsupported(d, "indexed plane overflow"))?;
        let mut expanded = b.bytes(n.checked_mul(4).ok_or_else(|| unsupported(d, "expanded indexed size overflow"))?)?;
        let mut indices = b.bytes(n)?;
        let mut alpha = if d.alpha { Some(b.bytes(n)?) } else { None };
        for (i, source) in plane.bytes.chunks_exact(plane.channels).enumerate() {
            if i.is_multiple_of(4096) {
                ctl.check().map_err(|_| IoError::Cancelled)?;
            }
            let index = source.first().copied().ok_or_else(|| unsupported(d, "missing palette index"))?;
            let color = palette.get(index as usize).ok_or_else(|| unsupported(d, format!("palette index {index} out of range")))?;
            let output = expanded.get_mut(i * 4..i * 4 + 4).ok_or_else(|| unsupported(d, "indexed output bounds"))?;
            output.get_mut(..3).ok_or_else(|| unsupported(d, "indexed RGB"))?.copy_from_slice(color);
            let coverage = if d.alpha { source.get(1).copied().ok_or_else(|| unsupported(d, "missing indexed alpha"))? } else { 255 };
            *output.get_mut(3).ok_or_else(|| unsupported(d, "indexed alpha output"))? = coverage;
            *indices.get_mut(i).ok_or_else(|| unsupported(d, "index sidecar bounds"))? = index;
            if let Some(a) = alpha.as_mut() {
                *a.get_mut(i).ok_or_else(|| unsupported(d, "alpha sidecar bounds"))? = coverage;
            }
        }
        let s = surface(b, plane, r, mode, true, &expanded, ctl)?;
        b.charge(palette.len() * 3 + (256 * 256 * 4))?;
        let side = IndexedSource::new_with_interrupt(Arc::new(indices), alpha.map(Arc::new), palette.to_vec(), r, &s, ctl)
            .map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { unsupported(d, e) })?;
        let temporary = expanded.len();
        drop(expanded);
        b.release(temporary + (256 * 256 * 4));
        Ok((s, Some(side)))
    } else if d.alpha {
        Ok((surface(b, plane, r, mode, true, &plane.bytes, ctl)?, None))
    } else {
        let source_stride = plane.channels * plane.sample.source_bytes();
        let target_stride = (plane.channels + 1) * plane.sample.source_bytes();
        let n = plane.bytes.len() / source_stride;
        let mut expanded = b.bytes(n.checked_mul(target_stride).ok_or_else(|| unsupported(d, "opaque plane overflow"))?)?;
        for (i, (src, dst)) in plane.bytes.chunks_exact(source_stride).zip(expanded.chunks_exact_mut(target_stride)).enumerate() {
            if i.is_multiple_of(4096) {
                ctl.check().map_err(|_| IoError::Cancelled)?;
            }
            dst.get_mut(..source_stride).ok_or_else(|| unsupported(d, "opaque output color"))?.copy_from_slice(src);
            let a = dst.get_mut(source_stride..).ok_or_else(|| unsupported(d, "opaque alpha"))?;
            match plane.sample {
                Sample::U8 => a.fill(255),
                Sample::U16 => a.copy_from_slice(&u16::MAX.to_ne_bytes()),
                Sample::F16 | Sample::F32 => a.copy_from_slice(&1.0f32.to_ne_bytes()),
            }
        }
        let s = surface(b, plane, r, mode, true, &expanded, ctl)?;
        let temporary = expanded.len();
        drop(expanded);
        b.release(temporary);
        Ok((s, None))
    }
}
fn insert(layers: &mut Vec<Layer>, path: &[u32], layer: Layer) -> Result<(), IoError> {
    let Some((&first, rest)) = path.split_first() else {
        return Err(IoError::Unsupported("XCF empty layer path".into()));
    };
    if rest.is_empty() {
        if first as usize != layers.len() {
            return Err(IoError::Unsupported("XCF duplicate/noncontiguous item path".into()));
        }
        layers.try_reserve_exact(1).map_err(|_| IoError::Unsupported("XCF group allocation failed".into()))?;
        layers.push(layer);
        return Ok(());
    }
    let parent = layers.get_mut(first as usize).ok_or_else(|| IoError::Unsupported("XCF item path references missing parent".into()))?;
    let LayerContent::Group(g) = &mut parent.content else {
        return Err(IoError::Unsupported("XCF item path parent is not a group".into()));
    };
    insert(&mut g.children, rest, layer)
}
fn reverse(layers: &mut [Layer]) {
    layers.reverse();
    for l in layers {
        if let LayerContent::Group(g) = &mut l.content {
            reverse(&mut g.children);
        }
    }
}
fn map(name: &str, mut file: XcfFile, ctl: &Interrupt<'_>, resident: usize) -> Result<(ImportResult, usize), IoError> {
    if file.dpi.0 != file.dpi.1 {
        return Err(IoError::Unsupported(format!("XCF unequal horizontal/vertical DPI ({}, {})", file.dpi.0, file.dpi.1)));
    }
    let mut budget = Budget { used: file.charged_bytes, max: resident, peak: file.charged_bytes };
    budget.charge(
        file.icc.len().checked_mul(4).and_then(|n| n.checked_add(1 << 20)).ok_or_else(|| IoError::Unsupported("XCF ICC allocation overflow".into()))?,
    )?;
    let space = if file.model == Model::Gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb };
    let ce = match file.encoding {
        Encoding::Linear => photocraft_cms::xcf_color::Encoding::Linear,
        Encoding::NonlinearProfile => photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        Encoding::Perceptual => photocraft_cms::xcf_color::Encoding::Perceptual,
    };
    let effective = photocraft_cms::xcf_color::effective_profile(if file.icc.is_empty() { None } else { Some(&file.icc) }, space, ce)
        .map_err(|e| IoError::Unsupported(format!("XCF source profile/encoding: {e}")))?;
    let mode = match file.model {
        Model::Rgb => ColorMode::Rgb,
        Model::Gray => ColorMode::Grayscale,
        Model::Indexed => ColorMode::Indexed,
    };
    let mut doc = Document::new(name, Size::new(file.width, file.height), mode, depth(file.sample));
    doc.sample_encoding = Some(encoding(file.encoding));
    doc.source_color_context =
        Some(SourceColorContext { original_icc: if file.icc.is_empty() { None } else { Some(Arc::new(file.icc.clone())) }, source_precision: file.precision });
    doc.icc_profile = Some(effective.to_bytes());
    doc.resolution_dpi = file.dpi.0;
    if mode == ColorMode::Indexed {
        budget.charge(file.palette.len() * 3)?;
        doc.color_table = Some(ColorTable { colors: file.palette.clone(), transparent: None });
    }
    let mut warnings = file.diagnostics;
    if file.sample == Sample::F16 {
        warnings.push("XCF finite F16 samples widened exactly to F32; original precision retained".into());
    }
    let layer_total = file.layers.len();
    for (i, drawable) in file.layers.drain(..).enumerate() {
        let d = &drawable;
        ctl.check().map_err(|_| IoError::Cancelled)?;
        admit(d)?;
        budget.charge(1024 + d.name.len() + d.path.len() * 4 + d.diagnostics.iter().map(|v| v.len() + std::mem::size_of::<String>()).sum::<usize>())?;
        let (mut l, side) = if d.group {
            (Layer::group(&d.name, Vec::new()), None)
        } else {
            let p = d.plane.as_ref().ok_or_else(|| unsupported(d, "missing raster pixels"))?;
            let (s, side) = raster(d, p, &file.palette, &mut budget, ctl)?;
            (Layer::new(&d.name, LayerContent::Raster(s)), side)
        };
        l.indexed_source = side;
        l.opacity = d.opacity;
        l.visible = d.visible;
        l.locks.transparency = d.lock_alpha;
        l.locks.pixels = d.lock_content;
        l.locks.position = d.lock_position;
        let pass = d.group && d.mode == 61;
        l.blend = if pass { BlendMode::PassThrough } else { BlendMode::Normal };
        l.import_compositing =
            Some(ImportCompositing { blend_encoding: working(d, d.blend_space)?, composite_encoding: working(d, d.composite_space)?, pass_through: pass });
        if d.lock_visibility {
            warnings.push(format!("{}: visibility lock omitted; visibility remains editable", short(&d.name)));
        }
        warnings.extend(d.diagnostics.iter().cloned());
        if let Some(m) = &d.mask {
            let p = m.plane.as_ref().ok_or_else(|| unsupported(m, "missing mask plane"))?;
            let r = bounds(d)?;
            let s = surface(&mut budget, p, r, ColorMode::Grayscale, false, &p.bytes, ctl)?;
            l.mask = Some(LayerMask { surface: s, enabled: d.mask_enabled, linked: true, density: 1.0, feather: 0.0 });
        }
        let path = if d.path.is_empty() { vec![doc.layers.len() as u32] } else { d.path.clone() };
        if path.len() > photocraft_doc::MAX_GROUP_DEPTH + 1 {
            return Err(unsupported(d, "group nesting exceeds document cap"));
        }
        insert(&mut doc.layers, &path, l)?;
        let released = d.plane.as_ref().map_or(0, |p| p.bytes.len()) + d.mask.as_ref().and_then(|m| m.plane.as_ref()).map_or(0, |p| p.bytes.len());
        drop(drawable);
        budget.release(released);
        ctl.progress(0.7 + 0.25 * ((i + 1) as f32) / (layer_total.max(1) as f32));
    }
    reverse(&mut doc.layers);
    for d in file.channels {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if !d.unsupported.is_empty() {
            return Err(unsupported(&d, "unsupported saved channel semantics"));
        }
        let p = d.plane.as_ref().ok_or_else(|| unsupported(&d, "missing channel plane"))?;
        let s = surface(&mut budget, p, bounds(&d)?, ColorMode::Grayscale, false, &p.bytes, ctl)?;
        if d.selection {
            if doc.selection.is_some() {
                return Err(unsupported(&d, "duplicate selection channels"));
            }
            doc.selection = Some(s);
        } else {
            let mut c = AlphaChannel::new(&d.name, s);
            c.opacity = d.opacity;
            doc.channels.push(c);
            warnings.push(format!("{}: channel samples preserved; GIMP channel display/editing state omitted", short(&d.name)));
        }
    }
    for g in file.guides {
        if (g.position as f32) as i32 != g.position {
            warnings.push(format!("Guide at {} omitted; position is not exactly representable", g.position));
            continue;
        }
        if g.horizontal {
            doc.guides.horizontal.push(g.position as f32);
        } else {
            doc.guides.vertical.push(g.position as f32);
        }
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(1.0);
    Ok((ImportResult { document: doc, warnings, origin: ImportOrigin::xcf() }, budget.peak))
}
