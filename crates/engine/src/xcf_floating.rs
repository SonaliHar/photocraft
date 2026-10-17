//! Atomic anchoring of qualified Normal raster floating attachments.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_color::{try_read_sample_f64, try_write_sample_f64};
use photocraft_compose::gimp_composite::WorkingSpace;
use photocraft_doc::{BlendMode, ColorMode, Document, Layer, LayerContent, LayerId, Rect};
use photocraft_doc::{import_compositing::GimpOperator, item_identity::ItemRef};
use photocraft_raster::{Interrupt, Surface};

const CMD: &str = "xcf.floating.anchor";
pub(crate) mod controlled;
fn error(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("cannot edit floating layer: {message}"))
}
struct AnchorPlan {
    source: LayerId,
    target: LayerId,
    surface: Surface,
}
fn unlocked(doc: &Document, id: LayerId, alpha: bool) -> Result<()> {
    doc.xcf_state.as_ref().ok_or_else(|| error("missing imported state"))?;
    let effective = crate::xcf_item_locks::effective_layer_locks(doc, id).map_err(error)?;
    let locks = effective.native;
    let imported = effective.imported;
    if locks.all || locks.pixels || imported.content || (alpha && (locks.transparency || imported.alpha)) {
        return Err(error("source or target content is locked"));
    }
    Ok(())
}
fn plain(layer: &Layer) -> Result<&Surface> {
    if layer.blend != BlendMode::Normal
        || !layer.visible
        || layer.fill_opacity != 1.0
        || layer.clipped
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || layer.excluded_channels != 0
        || !layer.blend_if.is_default()
        || !layer.advanced.is_default()
        || layer.video.is_some()
        || !layer.effects.items.is_empty()
    {
        return Err(error("unsupported raster blend, visibility, clipping, mask or effects"));
    }
    match &layer.content {
        LayerContent::Raster(surface) => Ok(surface),
        _ => Err(error("unsupported non-raster attachment")),
    }
}
fn checked_bounds(bounds: Rect) -> Result<()> {
    if bounds.is_empty()
        || bounds.width() > 262144
        || bounds.height() > 262144
        || [bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().any(|n| i64::from(*n).abs() > 1 << 29)
    {
        return Err(error("invalid declared raster bounds"));
    }
    Ok(())
}
#[cfg(test)]
fn prepare(doc: &Document, ctl: &Interrupt<'_>) -> Result<AnchorPlan> {
    prepare_with_profile_limits(doc, photocraft_cms::control::Limits::default(), ctl)
}
fn prepare_with_profile_limits(doc: &Document, profile_limits: photocraft_cms::control::Limits, ctl: &Interrupt<'_>) -> Result<AnchorPlan> {
    if ctl.cancelled() {
        return Err(EngineError::Cancelled);
    }
    doc.validate_xcf_state_with(ctl).map_err(error)?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| error("missing imported state"))?;
    let attachment = state.floating.as_ref().ok_or_else(|| error("no saved floating attachment"))?;
    let ItemRef::Layer(target_id) = attachment.target else {
        return Err(error("unsupported channel, mask or path anchor target"));
    };
    let source = doc.layer(attachment.layer).ok_or_else(|| error("missing floating layer"))?;
    let target = doc.layer(target_id).ok_or_else(|| error("missing target layer"))?;
    unlocked(doc, source.id, false)?;
    let source_locks = crate::xcf_item_locks::effective_layer_locks(doc, source.id).map_err(error)?;
    if source_locks.native.position || source_locks.imported.position || source_locks.imported.visibility {
        return Err(error("floating layer removal is locked"));
    }
    unlocked(doc, target.id, true)?;
    let src = plain(source)?;
    let dst = plain(target)?;
    let format = dst.format();
    let stored = doc.sample_encoding.ok_or_else(|| error("missing explicit sample encoding"))?;
    if format != src.format()
        || format.mode != doc.mode
        || format.sample != doc.depth
        || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !format.alpha
        || !source.opacity.is_finite()
        || !(0.0..=1.0).contains(&source.opacity)
    {
        return Err(error("unsupported mismatched, indexed or alpha-less raster formats"));
    }
    for id in [source.id, target.id] {
        if state.editable_indexed.iter().any(|r| r.owner == id)
            || state.source_items.iter().any(|s| s.owner == ItemRef::Layer(id))
            || state.filter_stacks.iter().any(|s| s.owner == ItemRef::Layer(id))
            || state.opaque.iter().any(|record| record.rendering_required && (record.owner.is_none() || record.owner == Some(ItemRef::Layer(id))))
        {
            return Err(error("unsupported indexed, editable-source or filtered attachment"));
        }
    }
    let source_item = state.items.iter().find(|i| i.item == ItemRef::Layer(source.id)).ok_or_else(|| error("missing floating metadata"))?;
    let target_item = state.items.iter().find(|i| i.item == ItemRef::Layer(target.id)).ok_or_else(|| error("missing target metadata"))?;
    let bounds = source_item.raster_bounds.ok_or_else(|| error("missing floating extent"))?;
    let target_bounds = target_item.raster_bounds.ok_or_else(|| error("missing target extent"))?;
    checked_bounds(bounds)?;
    checked_bounds(target_bounds)?;
    let composition = source_item.composition.as_ref().ok_or_else(|| error("missing floating composition"))?;
    if composition.source_mode != 28 || composition.legacy || composition.pass_through {
        return Err(error("unsupported floating composition mode"));
    }
    let resolved = composition.resolved.as_ref().ok_or_else(|| error("unresolved floating composition"))?;
    if resolved.operator != GimpOperator::Union || resolved.composite_space != photocraft_compose::gimp_composite::space(stored) {
        return Err(error("unsupported floating coverage operator or cross-encoding composite space"));
    }
    let overlap = bounds.intersect(&target_bounds);
    let pixels = u64::from(overlap.width()).checked_mul(u64::from(overlap.height())).ok_or_else(|| error("anchor work overflow"))?;
    if pixels > 36_000_000 {
        return Err(error("anchor work limit exceeded"));
    }
    let bpp = format.bytes_per_pixel();
    let tiles_x = u64::from(overlap.width()).div_ceil(256).saturating_add(1);
    let tiles_y = u64::from(overlap.height()).div_ceil(256).saturating_add(1);
    let replacement = tiles_x
        .checked_mul(tiles_y)
        .and_then(|n| n.checked_mul(256 * 256))
        .and_then(|n| n.checked_mul(bpp as u64))
        .ok_or_else(|| error("anchor allocation overflow"))?;
    let resident = src
        .resident_payload_bytes()
        .map_err(error)?
        .checked_add(dst.resident_payload_bytes().map_err(error)?)
        .and_then(|n| n.checked_add(replacement))
        .ok_or_else(|| error("anchor resident overflow"))?;
    if resident > 512 << 20 {
        return Err(error("anchor resident limit exceeded"));
    }
    // Reserve the strict current profile while both native owners and the
    // prospective replacement remain resident, before private output/plane clones.
    let maximum = photocraft_cms::control::Limits::default();
    if profile_limits.resident_bytes > maximum.resident_bytes || profile_limits.work_units > maximum.work_units {
        return Err(error("profile limits may only be lowered"));
    }
    let remaining = (512u64 << 20).checked_sub(resident).ok_or_else(|| error("anchor profile resident limit"))?;
    let limits = photocraft_cms::control::Limits {
        resident_bytes: usize::try_from(remaining.min(profile_limits.resident_bytes as u64)).map_err(error)?,
        work_units: profile_limits.work_units,
    };
    let required = crate::color_cmds::document_profile_requirements_with_limits(doc, limits, ctl)?;
    // WorkingSpace's unchanged validation is still an outer-polled CMS boundary;
    // reserve its repeated validation work rather than claiming deep cancellation.
    if required.work_units.checked_mul(2).is_none_or(|work| work > limits.work_units) {
        return Err(error("anchor profile/working-space work limit"));
    }
    resident
        .checked_add(required.peak_resident_bytes as u64)
        .filter(|bytes| *bytes <= 512 << 20)
        .ok_or_else(|| error("anchor aggregate profile/native headroom limit"))?;
    let profile = crate::color_cmds::try_document_profile_with_limits(doc, limits, ctl)?;
    let working = WorkingSpace::new_with_interrupt(&profile, stored, resolved.composite_space, ctl)
        .map_err(|error| if ctl.cancelled() { EngineError::Cancelled } else { self::error(error) })?;
    ctl.check().map_err(|_| EngineError::Cancelled)?;
    let mut output = dst.clone();
    let channels = format.channels();
    let mut source_bytes = [0u8; 32];
    for tile in overlap.tiles() {
        if ctl.cancelled() {
            return Err(EngineError::Cancelled);
        }
        let region = tile.rect().intersect(&overlap);
        let mut bytes = dst.try_read_interleaved_with(region, ctl).map_err(error)?;
        let mut changed = false;
        for (index, pixel) in bytes.chunks_exact_mut(bpp).enumerate() {
            if index.is_multiple_of(256) && ctl.cancelled() {
                return Err(EngineError::Cancelled);
            }
            let width = usize::try_from(region.width()).map_err(error)?;
            let x = region.x0.checked_add(i32::try_from(index % width).map_err(error)?).ok_or_else(|| error("coordinate overflow"))?;
            let y = region.y0.checked_add(i32::try_from(index / width).map_err(error)?).ok_or_else(|| error("coordinate overflow"))?;
            let raw = source_bytes.get_mut(..bpp).ok_or_else(|| error("unsupported pixel width"))?;
            src.try_read_pixel_bytes(x, y, raw).map_err(error)?;
            let alpha = try_read_sample_f64(raw, format.sample, channels - 1).map_err(error)?;
            if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
                return Err(error("invalid floating alpha"));
            }
            if alpha == 0.0 || source.opacity == 0.0 {
                continue;
            }
            if alpha == 1.0 && source.opacity == 1.0 {
                for channel in 0..channels - 1 {
                    if !try_read_sample_f64(raw, format.sample, channel).map_err(error)?.is_finite() {
                        return Err(error("nonfinite floating color"));
                    }
                }
                pixel.copy_from_slice(raw);
                changed = true;
                continue;
            }
            let read = |raw: &[u8]| -> Result<[f64; 4]> {
                let c = try_read_sample_f64(raw, format.sample, 0).map_err(error)?;
                let rgb = if format.mode == ColorMode::Grayscale {
                    [c, c, c]
                } else {
                    [c, try_read_sample_f64(raw, format.sample, 1).map_err(error)?, try_read_sample_f64(raw, format.sample, 2).map_err(error)?]
                };
                Ok([rgb[0], rgb[1], rgb[2], try_read_sample_f64(raw, format.sample, channels - 1).map_err(error)?])
            };
            let composed = working.normal(read(pixel)?, read(raw)?, f64::from(source.opacity), GimpOperator::Union).map_err(error)?;
            for channel in 0..channels {
                let component = if channel == channels - 1 { 3 } else { channel };
                try_write_sample_f64(pixel, format.sample, channel, *composed.get(component).ok_or_else(|| error("invalid composition channel"))?)
                    .map_err(error)?;
            }
            changed = true;
        }
        if changed {
            output.try_write_interleaved_with(region, &bytes, ctl).map_err(error)?;
        }
    }
    Ok(AnchorPlan { source: source.id, target: target.id, surface: output })
}
fn enabled(s: &Session) -> std::result::Result<(), String> {
    s.active().and_then(|s| s.doc.xcf_state.as_ref()).and_then(|s| s.floating.as_ref()).map(|_| ()).ok_or_else(|| "no saved floating attachment".into())
}
fn anchor(s: &mut Session, params: &serde_json::Value) -> Result<serde_json::Value> {
    controlled::anchor(s, params)
}
/// Clear a qualified raster attachment without changing either pixel plane.
fn release_attachment(s: &mut Session, params: &serde_json::Value, delete: bool) -> Result<serde_json::Value> {
    controlled::release(s, params, delete)
}
fn release_on_document(doc: &mut Document, active: &mut Option<LayerId>, delete: bool, ctl: &Interrupt<'_>) -> Result<LayerId> {
    doc.validate_xcf_state_with(ctl).map_err(error)?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| error("missing imported state"))?;
    let attachment = state.floating.as_ref().ok_or_else(|| error("no saved floating attachment"))?;
    let source = attachment.layer;
    let ItemRef::Layer(target) = attachment.target else {
        return Err(error("unsupported non-layer floating target"));
    };
    if doc.layers.len() != 2 || doc.layers.first().is_none_or(|layer| layer.id != target) || doc.layers.last().is_none_or(|layer| layer.id != source) {
        return Err(error("unsupported floating hierarchy"));
    }
    for id in [source, target] {
        plain(doc.layer(id).ok_or_else(|| error("missing raster layer"))?)?;
        let item = state.items.iter().find(|item| item.item == ItemRef::Layer(id)).ok_or_else(|| error("missing raster metadata"))?;
        let composition = item.composition.as_ref().ok_or_else(|| error("missing raster composition"))?;
        if composition.source_mode != 28
            || composition.legacy
            || composition.pass_through
            || composition.resolved.as_ref().is_none_or(|resolved| resolved.operator != GimpOperator::Union)
        {
            return Err(error("unsupported floating composition"));
        }
        if state.filter_stacks.iter().any(|stack| stack.owner == ItemRef::Layer(id)) {
            return Err(error("unsupported floating filters"));
        }
    }
    if delete {
        let effective = crate::xcf_item_locks::effective_layer_locks(doc, source).map_err(error)?;
        let locks = effective.native;
        let imported = effective.imported;
        if locks.all || locks.pixels || locks.position || imported.content || imported.position || imported.visibility {
            return Err(error("floating layer is locked"));
        }
    }
    let selected = if delete { target } else { source };
    let state = doc.xcf_state.as_mut().ok_or_else(|| error("missing imported state"))?;
    state.floating = None;
    state.selection.selected.clear();
    state.selection.selected.try_reserve(1).map_err(|_| error("selection allocation"))?;
    state.selection.selected.push(ItemRef::Layer(selected));
    state.selection.active = Some(ItemRef::Layer(selected));
    if delete {
        doc.remove(source).ok_or_else(|| error("missing floating layer"))?;
    }
    // Reconcile on this private transaction before asking the renderer to validate it.
    let mut state = doc.xcf_state.take().ok_or_else(|| error("missing imported state"))?;
    state.reconcile_layers_with_interrupt(doc, ctl).map_err(error)?;
    doc.xcf_state = Some(state);
    photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, ctl).map_err(error)?;
    *active = Some(selected);
    ctl.check().map_err(|_| EngineError::Cancelled)?;
    Ok(selected)
}
fn to_layer(s: &mut Session, params: &serde_json::Value) -> Result<serde_json::Value> {
    release_attachment(s, params, false)
}
fn delete(s: &mut Session, params: &serde_json::Value) -> Result<serde_json::Value> {
    release_attachment(s, params, true)
}
pub fn specs() -> Vec<CommandSpec> {
    [
        (
            CMD,
            "Anchor Floating Layer",
            "{documentId:canonical-decimal-str?,expectedRevision:canonical-decimal-str?} -> {anchored,target} (qualified Normal RGB/Gray raster attachments only)",
            anchor as fn(&mut Session, &serde_json::Value) -> Result<serde_json::Value>,
        ),
        (
            "xcf.floating.toLayer",
            "Floating Selection to Layer",
            "{documentId:canonical-decimal-str?,expectedRevision:canonical-decimal-str?} -> {deleted,layer} (qualified raster layer target)",
            to_layer as fn(&mut Session, &serde_json::Value) -> Result<serde_json::Value>,
        ),
        (
            "xcf.floating.delete",
            "Delete Floating Layer",
            "{documentId:canonical-decimal-str?,expectedRevision:canonical-decimal-str?} -> {deleted,layer} (qualified raster layer target)",
            delete as fn(&mut Session, &serde_json::Value) -> Result<serde_json::Value>,
        ),
    ]
    .into_iter()
    .map(|(id, label, params, run)| CommandSpec { id, label, menu: &[], shortcut: None, params, enabled, run, journal: true })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        PixelFormat, SampleEncoding, SampleType, Size, XcfState,
        import_compositing::{GimpComposition, GimpSpace, ResolvedGimpComposition},
        xcf_editing::FloatingAttachment,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn fixture() -> Document {
        let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
        let mut doc = Document::new("Own cancellation fixture", Size::new(1024, 1024), ColorMode::Rgb, SampleType::F64);
        doc.sample_encoding = Some(SampleEncoding::Linear);
        let target = Layer::raster("Own target", format);
        let mut source = Layer::raster("Own source", format);
        source.surface_mut().unwrap().try_write_pixel_f64(0, 0, &[0.2, 0.3, 0.4, 0.5]).unwrap();
        let source_id = source.id;
        let target_id = target.id;
        doc.layers = vec![target, source];
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        for item in &mut state.items {
            item.raster_bounds = Some(doc.bounds());
            item.composition = Some(GimpComposition {
                compatibility_epoch: "own-anchor-test".into(),
                source_mode: 28,
                legacy: false,
                stored_blend_space: 1,
                stored_composite_space: 1,
                stored_operator: 1,
                resolved: Some(ResolvedGimpComposition {
                    blend_space: GimpSpace::LinearRgb,
                    composite_space: GimpSpace::LinearRgb,
                    operator: GimpOperator::Union,
                }),
                pass_through: false,
            });
        }
        state.floating = Some(FloatingAttachment { layer: source_id, target: ItemRef::Layer(target_id) });
        doc.xcf_state = Some(state);
        doc
    }
    #[test]
    fn preparation_pre_cancel_and_mid_work_are_immutable() {
        let doc = fixture();
        let original = doc.clone();
        assert!(prepare(&doc, &Interrupt::cancel_only(&|| true)).is_err());
        let count = AtomicUsize::new(0);
        let cancel = || count.fetch_add(1, Ordering::Relaxed) >= 100;
        assert!(prepare(&doc, &Interrupt::cancel_only(&cancel)).is_err());
        assert!(count.load(Ordering::Relaxed) >= 100);
        assert_eq!(doc, original);
    }
    #[test]
    fn controlled_profile_anchor_preserves_old_normal_words_and_source_identity() {
        let mut doc = fixture();
        doc.size = Size::new(2, 2);
        let bounds = doc.bounds();
        for item in &mut doc.xcf_state.as_mut().unwrap().items {
            item.raster_bounds = Some(bounds);
        }
        let original = doc.clone();
        let old = crate::color_cmds::try_document_profile(&doc).unwrap();
        let old_working = WorkingSpace::new(&old, SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
        let expected = old_working.normal([0.; 4], [0.2, 0.3, 0.4, 0.5], 1.0, GimpOperator::Union).unwrap();
        let actual = prepare(&doc, &Interrupt::NONE).unwrap();
        let words = actual.surface.try_read_region_f64(Rect::new(0, 0, 1, 1)).unwrap();
        assert_eq!(words.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.map(f64::to_bits));
        assert_eq!(doc, original);
        assert!(prepare_with_profile_limits(&doc, photocraft_cms::control::Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE).is_err());
        assert_eq!(doc, original);
    }
    #[test]
    fn assigned_model_mismatch_keeps_floating_relation_and_raw_samples() {
        let mut doc = fixture();
        doc.icc_profile = Some(photocraft_cms::Builtin::SGray.profile().to_bytes());
        let original = doc.clone();
        assert!(prepare(&doc, &Interrupt::NONE).is_err());
        assert_eq!(doc, original);
        assert!(doc.xcf_state.as_ref().unwrap().floating.is_some());
    }
}
#[cfg(test)]
mod benchmark {
    use super::*;
    use photocraft_doc::{
        PixelFormat, SampleEncoding, SampleType, Size, XcfState,
        import_compositing::{GimpComposition, GimpSpace, ResolvedGimpComposition},
        xcf_editing::FloatingAttachment,
    };
    #[test]
    #[ignore = "24MP anchor timing; run named test in release"]
    fn normal_anchor_24mp_release() {
        let format = PixelFormat::RGBA8;
        let mut doc = Document::new("Own 24MP anchor", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8);
        doc.sample_encoding = Some(SampleEncoding::Linear);
        let mut target = Layer::raster("Own target", format);
        target.content = LayerContent::Raster(Surface::with_default(format, &[0.25, 0.5, 0.75, 0.5]));
        let mut source = Layer::raster("Own source", format);
        source.content = LayerContent::Raster(Surface::with_default(format, &[0.75, 0.25, 0.5, 0.5]));
        let source_id = source.id;
        let target_id = target.id;
        doc.layers = vec![target, source];
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        for item in &mut state.items {
            item.raster_bounds = Some(doc.bounds());
            item.composition = Some(GimpComposition {
                compatibility_epoch: "own-anchor-benchmark".into(),
                source_mode: 28,
                legacy: false,
                stored_blend_space: 1,
                stored_composite_space: 1,
                stored_operator: 1,
                resolved: Some(ResolvedGimpComposition {
                    blend_space: GimpSpace::LinearRgb,
                    composite_space: GimpSpace::LinearRgb,
                    operator: GimpOperator::Union,
                }),
                pass_through: false,
            });
        }
        state.floating = Some(FloatingAttachment { layer: source_id, target: ItemRef::Layer(target_id) });
        doc.xcf_state = Some(state);
        let started = std::time::Instant::now();
        let plan = prepare(&doc, &Interrupt::NONE).unwrap();
        println!("24MP U8 Normal anchor staged {:.3}s, resident {} bytes", started.elapsed().as_secs_f64(), plan.surface.resident_payload_bytes().unwrap());
        let values = plan.surface.try_read_region_f64(Rect::new(0, 0, 1, 1)).unwrap();
        assert!((values[3] - 0.75).abs() < 0.01);
        assert_eq!(doc.layers.len(), 2);
    }
}
