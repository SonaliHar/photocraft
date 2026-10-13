//! Checked physical dimensions for the actual asset-generation route.
//! Public scalar parsers retain their original grammar and compatibility.

use super::{AssetDefault, AssetSpec, Document, Result, bad, parse_asset_name, parse_defaults};

const COMMAND: &str = "file.generate.imageAssets";

pub(super) fn axes(doc: &Document) -> Result<(f64, f64)> {
    let legacy = photocraft_doc::PhysicalResolution::square(f64::from(doc.resolution_dpi));
    let resolution = doc.physical_resolution.as_ref().unwrap_or(&legacy);
    resolution.validate().map_err(|error| bad(COMMAND, error.to_string()))?;
    Ok((resolution.x_dpi, resolution.y_dpi))
}

fn numbers(scale: Option<f64>, width: Option<f64>, height: Option<f64>) -> Result<()> {
    // Positive inputs may underflow to zero; the existing final one-pixel
    // quantization is preserved. Nonfinite dimensions cannot enter JSON.
    if [scale, width, height].into_iter().flatten().any(|value| !value.is_finite() || value < 0.0) {
        return Err(bad(COMMAND, "asset physical dimensions or scale are not finite"));
    }
    Ok(())
}

pub(super) fn names(name: &str, axes: (f64, f64)) -> Result<Vec<AssetSpec>> {
    let mut horizontal = parse_asset_name(name, axes.0);
    let vertical = parse_asset_name(name, axes.1);
    if horizontal.len() != vertical.len() {
        return Err(bad(COMMAND, "asset size grammar differs between resolution axes"));
    }
    for (spec, y) in horizontal.iter_mut().zip(vertical) {
        spec.height = y.height;
        numbers(spec.scale, spec.width, spec.height)?;
    }
    Ok(horizontal)
}

pub(super) fn defaults(name: &str, axes: (f64, f64)) -> Result<Option<Vec<AssetDefault>>> {
    let Some(mut horizontal) = parse_defaults(name, axes.0) else { return Ok(None) };
    let vertical = parse_defaults(name, axes.1).ok_or_else(|| bad(COMMAND, "asset default grammar differs between resolution axes"))?;
    if horizontal.len() != vertical.len() {
        return Err(bad(COMMAND, "asset default count differs between resolution axes"));
    }
    for (variant, y) in horizontal.iter_mut().zip(vertical) {
        variant.height = y.height;
        numbers(variant.scale, variant.width, variant.height)?;
    }
    Ok(Some(horizontal))
}

fn pair(width: f64, height: f64) -> Result<(f64, f64)> {
    numbers(None, Some(width), Some(height))?;
    Ok((width, height))
}

pub(super) fn dimensions(base: (f64, f64), spec: &AssetSpec, variant: &AssetDefault) -> Result<(f64, f64)> {
    let (w0, h0) = base;
    if !w0.is_finite() || !h0.is_finite() || w0 <= 0.0 || h0 <= 0.0 {
        return Err(bad(COMMAND, "asset source dimensions must be positive and finite"));
    }
    let scale = spec.scale.unwrap_or(1.0);
    let (mut width, mut height) = pair(w0 * scale, h0 * scale)?;
    (width, height) = match (spec.width, spec.height) {
        (Some(w), Some(h)) => pair(w, h)?,
        (Some(w), None) => pair(w, h0 * w / w0)?,
        (None, Some(h)) => pair(w0 * h / h0, h)?,
        _ => (width, height),
    };
    let scale = variant.scale.unwrap_or(1.0);
    (width, height) = pair(width * scale, height * scale)?;
    (width, height) = match (variant.width, variant.height) {
        (Some(w), Some(h)) => pair(w, h)?,
        (Some(w), None) if width > 0.0 => pair(w, height * w / width)?,
        (None, Some(h)) if height > 0.0 => pair(width * h / height, h)?,
        (Some(_), None) | (None, Some(_)) => return Err(bad(COMMAND, "asset default aspect ratio is not representable")),
        _ => (width, height),
    };
    Ok((width.round().max(1.0), height.round().max(1.0)))
}
