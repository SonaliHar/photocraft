//! A small original-coordinate selection preview with genuine backing ownership.
//! This path never creates an uncharged resident proxy of an accounted document.
use super::*;
use photocraft_engine::selection_cmds::{BackedColorRangePixels, BackedColorRangeSamples, ColorRangeCoverage};
use photocraft_raster::{
    Interrupt,
    backing::quota::{QuotaKind, QuotaLease},
};

pub(super) struct NativePreview {
    query: BackedColorRangeSamples,
    _pixels: BackedColorRangePixels,
    _coverage: ColorRangeCoverage,
    mask: Vec<f32>,
    size: [usize; 2],
    // Retained source/query/output reservations follow all derivative storage.
    _view: QuotaLease,
}
fn current(query: &BackedColorRangeSamples, app: &PhotocraftApp) -> Result<(), String> {
    if query.backed_source_is_current(&app.session) != Some(true) {
        return Err(tl!("The Color Range preview target changed.").into());
    }
    Ok(())
}
fn pixels(values: &[f32], mode: Option<&str>, control: &Interrupt<'_>) -> Result<Vec<Color32>, String> {
    let mut out = Vec::new();
    out.try_reserve_exact(values.len()).map_err(|_| tl!("There is not enough memory for the Color Range preview."))?;
    if out.capacity() > values.len().saturating_mul(2) {
        return Err(tl!("The Color Range preview is too large.").into());
    }
    for chunk in values.chunks(1024) {
        preview_poll(control)?;
        for value in chunk {
            if !value.is_finite() {
                return Err(tl!("The Color Range preview contains invalid selection values.").into());
            }
            out.push(if let Some(mode) = mode { overlay_pixel(mode, *value) } else { Color32::from_gray((value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8) });
        }
    }
    preview_poll(control)?;
    Ok(out)
}
/// Same inversion, original-selection scope, then GRAY8 rounding as the
/// registered controlled command. Canonical native samples never change here.
fn finish_selection_view(mask: &mut [f32], prior: Option<&[f32]>, invert: bool, control: &Interrupt<'_>) -> Result<(), String> {
    if prior.is_some_and(|values| values.len() != mask.len()) {
        return Err(tl!("The Color Range preview contains an incomplete selection.").into());
    }
    for (index, part) in mask.chunks_mut(1024).enumerate() {
        preview_poll(control)?;
        for (offset, value) in part.iter_mut().enumerate() {
            if !value.is_finite() {
                return Err(tl!("The Color Range preview contains invalid selection values.").into());
            }
            let v = if invert { 1.0 - *value } else { *value };
            *value = v;
            if let Some(prior) = prior {
                let position = index.checked_mul(1024).and_then(|n| n.checked_add(offset)).ok_or(tl!("The Color Range preview is too large."))?;
                let bound = prior.get(position).ok_or(tl!("The Color Range preview contains an incomplete selection."))?;
                if !bound.is_finite() || !(0.0..=1.0).contains(bound) {
                    return Err(tl!("The Color Range preview contains invalid selection values.").into());
                }
                *value = value.min(*bound);
            }
            *value = (value.clamp(0.0, 1.0) * 255.0 + 0.5).floor() / 255.0;
        }
    }
    preview_poll(control)
}
pub(super) fn preview(app: &mut PhotocraftApp, ctx: &egui::Context, params: &Value, key: u64) -> Result<(egui::TextureId, [usize; 2], u32), String> {
    let (doc, revision, bounds, quota) = {
        let state = app.session.active().ok_or(tl!("No document is open."))?;
        let bounds = state.doc.bounds();
        // A full native rectangle is deliberately small until owned reduction is
        // available. This is a view limit, not an import or canonical raster cap.
        if bounds.is_empty() || bounds.width() > PREVIEW || bounds.height() > PREVIEW {
            return Err(tl!("Color Range preview is unavailable at this image size.").into());
        }
        (state.doc.id, state.revision, bounds, state.doc.quota().ok_or(tl!("Color Range preview is unavailable for this document."))?.clone())
    };
    if let Some(p) = app.color_range.as_ref().filter(|p| p.doc == doc && p.revision == revision && p.mask_key == key && p.error.is_none()) {
        if let (Some(native), Some(texture)) = (&p.backed, &p.mask) {
            current(&native.query, app)?;
            return Ok((texture.id(), native.size, 1));
        }
    }
    let control = &Interrupt::NONE;
    let query = BackedColorRangeSamples::new_with_interrupt(&app.session, params, control).map_err(|e| e.to_string())?;
    current(&query, app)?;
    let original = query.backed_pixels_with_interrupt(bounds, control).map_err(|e| e.to_string())?;
    let coverage = query
        .coverage_backed_pixels_with_interrupt(&original, photocraft_engine::selection_cmds::ColorRangeLimits::default(), control)
        .map_err(|e| e.to_string())?;
    let count = preview_count(bounds, std::mem::size_of::<f32>())?;
    if coverage.values().len() != count {
        return Err(tl!("The Color Range preview contains an incomplete selection.").into());
    }
    // Two retained texture derivatives, coverage copy and prior-mask scratch.
    // Renderer-internal/GPU allocations remain the existing UI boundary.
    let bytes = (count as u64)
        .checked_mul(32)
        .and_then(|n| n.checked_add(std::mem::size_of::<NativePreview>() as u64 + 4096))
        .ok_or(tl!("The Color Range preview is too large."))?;
    query.charge_backed_preview_work((count as u64).saturating_mul(64), control).map_err(|e| e.to_string())?;
    let lease = quota.reserve(QuotaKind::Resident, bytes).map_err(|e| e.to_string())?;
    let mut mask = Vec::new();
    mask.try_reserve_exact(count).map_err(|_| tl!("There is not enough memory for the Color Range preview."))?;
    if mask.capacity() > count.saturating_mul(2) {
        return Err(tl!("The Color Range preview is too large.").into());
    }
    for part in coverage.values().chunks(1024) {
        preview_poll(control)?;
        mask.extend_from_slice(part);
    }
    // Keep the command's inversion, original-selection scope and rounding. Combine
    // modes have a separate publication path; this initial view is replacement.
    if !matches!(params.get("mode").and_then(Value::as_str), None | Some("replace")) {
        return Err(tl!("This Color Range preview combination is unavailable.").into());
    }
    let previous = if let Some(prior) = app.session.active().and_then(|state| state.doc.selection.as_ref()) {
        if prior.format().mode != photocraft_doc::ColorMode::Grayscale || prior.format().alpha {
            return Err(tl!("Color Range preview is unavailable for this stored selection.").into());
        }
        // Both validation passes visit the entire retained map, including tiles
        // outside this small canvas. Admit their actual count before either walk.
        let metadata_work = u64::try_from(prior.tile_count())
            .ok()
            .and_then(|n| n.checked_mul(16))
            .and_then(|n| n.checked_add(64))
            .ok_or(tl!("The Color Range preview is too large."))?;
        query.charge_backed_preview_work(metadata_work, control).map_err(|e| e.to_string())?;
        Some(preview_prior_mask(Some(prior), bounds, control)?)
    } else {
        None
    };
    finish_selection_view(&mut mask, previous.as_deref(), params.get("invert").and_then(Value::as_bool).unwrap_or(false), control)?;
    let size = [bounds.width() as usize, bounds.height() as usize];
    let image = egui::ColorImage::new(size, pixels(&mask, None, control)?);
    current(&query, app)?;
    preview_poll(control)?;
    let texture = ctx.load_texture("color-range-native-selection", image, egui::TextureOptions::LINEAR);
    let id = texture.id();
    app.color_range = Some(Preview {
        doc,
        revision,
        k: 1,
        proxy: None,
        image: None,
        mask_key: key,
        mask: Some(texture),
        error: None,
        canvas_proxy: None,
        overlay: None,
        overlay_key: 0,
        backed: Some(NativePreview { query, _pixels: original, _coverage: coverage, mask, size, _view: lease }),
    });
    Ok((id, size, 1))
}
pub(super) fn paint_overlay(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    painter: &egui::Painter,
    doc: DocId,
    image: Rect,
    flip: bool,
    mode: &str,
) -> Result<(), String> {
    let p = app.color_range.as_ref().ok_or(tl!("Color Range preview is unavailable."))?;
    let native = p.backed.as_ref().ok_or(tl!("Color Range preview is unavailable."))?;
    current(&native.query, app)?;
    if p.doc != doc {
        return Err(tl!("The Color Range preview target changed.").into());
    }
    let key = (p.mask_key ^ hash(mode)).max(1);
    if p.overlay_key != key || p.overlay.is_none() {
        let control = &Interrupt::NONE;
        native.query.charge_backed_preview_work((native.mask.len() as u64).saturating_mul(16), control).map_err(|e| e.to_string())?;
        let color = egui::ColorImage::new(native.size, pixels(&native.mask, Some(mode), control)?);
        current(&native.query, app)?;
        preview_poll(control)?;
        let texture = ctx.load_texture("color-range-native-overlay", color, egui::TextureOptions::LINEAR);
        let p = app.color_range.as_mut().ok_or(tl!("Color Range preview is unavailable."))?;
        p.overlay = Some(texture);
        p.overlay_key = key;
    }
    if let Some(texture) = app.color_range.as_ref().and_then(|p| p.overlay.as_ref()) {
        let uv = if flip { Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)) } else { Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)) };
        painter.image(texture.id(), image, uv, Color32::WHITE);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inversion_and_prior_scope_precede_command_gray8_rounding() {
        let mut values = [0.2, 0.5, 0.8];
        finish_selection_view(&mut values, Some(&[0.25, 1.0, 0.1]), true, &Interrupt::NONE).unwrap();
        assert_eq!(values, [64.0 / 255.0, 128.0 / 255.0, 26.0 / 255.0]);
    }
    #[test]
    fn malformed_prior_or_cancel_never_creates_a_texture_result() {
        assert!(finish_selection_view(&mut [0.5], Some(&[]), false, &Interrupt::NONE).is_err());
        assert!(finish_selection_view(&mut [0.5], Some(&[f32::NAN]), false, &Interrupt::NONE).is_err());
        let original = vec![0.5; 4096];
        let mut values = original.clone();
        let cancel = || true;
        assert!(finish_selection_view(&mut values, None, true, &Interrupt::cancel_only(&cancel)).is_err());
        assert_eq!(values, original);
    }
}
