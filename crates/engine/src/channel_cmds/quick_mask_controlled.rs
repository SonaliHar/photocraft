//! Cancellable Quick Mask transitions, with no presentation-driven inversion.
//! Native same-depth wide Gray coverage is transferred raw after unit validation.
use crate::{EngineError, Result, Session};
use photocraft_color::{
    ColorMode, PixelFormat, SampleType,
    checked_arithmetic::{self, Domain, Operand, Storage},
    read_sample, write_sample,
};
use photocraft_doc::{ColorIndicates, Layer, LayerContent};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{
    Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface,
    staged::{SOURCE_CELL_SIZE, StagedSurfaceBuilder, StagingBudget},
};
use serde_json::{Value, json};

fn error(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("Quick Mask: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled(control: &Interrupt<'_>, message: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { error(message) }
}
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    if !params.is_null()
        && !params
            .as_object()
            .is_some_and(|object| object.len() <= 3 && object.keys().all(|key| matches!(key.as_str(), "on" | "documentId" | "expectedRevision")))
    {
        return Err(error("expected bounded on/captured-target parameters"));
    }
    if params.get("on").is_some_and(|value| !value.is_boolean()) {
        return Err(error("on must be boolean"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.revision == u64::MAX || session.command_has_pending_job() || session.command_has_job_on(state.doc.id) {
        return Err(error("target revision/pending work refused"));
    }
    if state.doc.xcf_state.is_some() || state.doc.color_table.is_some() || matches!(state.doc.mode, ColorMode::Indexed | ColorMode::Multichannel) {
        return Err(error("current indexed/imported source selection policy is not qualified"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) => {
            let exact = |value: &Value| -> Result<u64> {
                let text = value
                    .as_str()
                    .filter(|text| {
                        !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0'))
                    })
                    .ok_or_else(|| error("captured IDs must be canonical decimal strings"))?;
                text.parse().map_err(|_| error("captured ID exceeds range"))
            };
            let id = exact(id)?;
            if id == 0 || id != state.doc.id.0 || exact(revision)? != state.revision {
                return Err(error("captured Quick Mask target changed"));
            }
            if state.floating.is_some()
                || state.coalesce.is_some()
                || session.has_jobs()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
            {
                return Err(error("captured Quick Mask target has floating or transient work"));
            }
        }
        _ => return Err(error("documentId and expectedRevision must be supplied together")),
    }
    if !session.quick_mask_options.opacity.is_finite()
        || !(0.0..=1.0).contains(&session.quick_mask_options.opacity)
        || session.quick_mask_options.color.to_rgb().iter().any(|value| !value.is_finite())
    {
        return Err(error("invalid Quick Mask presentation options"));
    }
    Ok(())
}
fn source_guard(layers: &[Layer], depth: usize, count: &mut u64, control: &Interrupt<'_>) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(error("layer hierarchy limit"));
    }
    for layer in layers {
        poll(control)?;
        *count = count.checked_add(1).filter(|count| *count <= 100_000).ok_or_else(|| error("layer census limit"))?;
        if layer.import_compositing.is_some()
            || layer.indexed_source.is_some()
            || layer.video.is_some()
            || matches!(&layer.content, LayerContent::BackedRaster(_))
        {
            return Err(error("source-owned/semantic Quick Mask context is not qualified"));
        }
        if let LayerContent::Group(group) = &layer.content {
            source_guard(&group.children, depth + 1, count, control)?;
        }
    }
    Ok(())
}
struct Budget {
    resident: u64,
    work: u64,
}
impl StagingBudget for Budget {
    type Lease = ();
    fn reserve(&mut self, bytes: u64) -> std::result::Result<(), RasterInputError> {
        self.resident = self.resident.checked_sub(bytes).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn work(&mut self, units: u64) -> std::result::Result<(), RasterInputError> {
        self.work = self.work.checked_sub(units).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
fn admission(snapshot: &crate::DocumentSnapshot, source: Option<&Surface>, format: PixelFormat, extra_work: u64, control: &Interrupt<'_>) -> Result<Budget> {
    poll(control)?;
    snapshot.validate_resident()?;
    let area = snapshot.bounds();
    if area.is_empty() {
        return Err(error("empty Quick Mask canvas"));
    }
    let mut count = 0;
    source_guard(&snapshot.layers, 0, &mut count, control)?;
    if let Some(source) = source {
        if source.format().mode != ColorMode::Grayscale || source.format().alpha {
            return Err(error("Quick Mask requires a native Gray coverage plane without alpha"));
        }
        if matches!(snapshot.depth, SampleType::U32 | SampleType::F64) && source.format() != super::channel_format(snapshot) {
            return Err(error("wide Quick Mask requires same-depth native Gray coverage"));
        }
        source.try_validate_tile_origins_with(control).map_err(|e| controlled(control, e))?;
    }
    let tiles = u64::from(snapshot.size.width.div_ceil(TILE_SIZE as u32))
        .checked_mul(u64::from(snapshot.size.height.div_ceil(TILE_SIZE as u32)))
        .ok_or_else(|| error("output tile span overflow"))?;
    let output = tiles
        .checked_mul((TILE_SIZE as u64).pow(2) * format.bytes_per_pixel() as u64 + 1024)
        .and_then(|n| n.checked_add(128 * 1024))
        .ok_or_else(|| error("output capacity overflow"))?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(snapshot, limits, control).map_err(|e| controlled(control, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, limits, control).map_err(|e| controlled(control, e))? as u64;
    let raster = snapshot.resident_raster_footprint_with(control)?;
    // Known payload/model/map slice and private native output, not an RSS claim.
    let peak = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|n| n.checked_add(output))
        .ok_or_else(|| error("residency overflow"))?;
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(error("aggregate Quick Mask source/output residency limit"));
    }
    let samples = u64::from(snapshot.size.width).checked_mul(u64::from(snapshot.size.height)).ok_or_else(|| error("sample count overflow"))?;
    let source_tiles = source.map_or(0, Surface::tile_count) as u64;
    // A clipped canvas still initializes every byte in each physical tile.
    // Charge that work separately from the logical sample reads and copies.
    let tile_initialization = tiles
        .checked_mul((TILE_SIZE as u64).pow(2))
        .and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64))
        .ok_or_else(|| error("tile initialization work overflow"))?;
    let work = samples
        .checked_mul(96)
        .and_then(|n| n.checked_add(tile_initialization))
        .and_then(|n| n.checked_add(tiles.checked_mul(source_tiles)?.checked_mul(16 * 256)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(1024)?))
        .and_then(|n| n.checked_add(count.checked_mul(64)?))
        .and_then(|n| n.checked_add(extra_work))
        .ok_or_else(|| error("work overflow"))?;
    if work > 8_000_000_000 {
        return Err(error("Quick Mask native read/output work limit"));
    }
    poll(control)?;
    Ok(Budget { resident: output, work })
}
/// Incoming selections remain selected coverage: indicates only changes the
/// overlay presentation. No inversion is introduced by this transition.
struct TransferPolicy {
    enter: bool,
    unselected: bool,
    indicates: ColorIndicates,
    rewrite_empty: Option<f64>,
}
fn transfer(snapshot: &crate::DocumentSnapshot, source: Option<&Surface>, policy: TransferPolicy, control: &Interrupt<'_>) -> Result<Option<Surface>> {
    let TransferPolicy { enter, unselected, indicates, rewrite_empty } = policy;
    let wide = matches!(snapshot.depth, SampleType::U32 | SampleType::F64);
    let format = if enter || wide { super::channel_format(snapshot) } else { PixelFormat::GRAY8 };
    let extra_work = if rewrite_empty.is_some() {
        u64::from(snapshot.size.width)
            .checked_mul(u64::from(snapshot.size.height))
            .and_then(|n| n.checked_mul(64))
            .ok_or_else(|| error("Quick Mask reorientation work"))?
    } else {
        0
    };
    let mut budget = admission(snapshot, source, format, extra_work, control)?;
    let area = snapshot.bounds();
    let bpp = format.bytes_per_pixel();
    let zero = [0u8; 8];
    let mut builder = StagedSurfaceBuilder::new(format, zero.get(..bpp).ok_or_else(|| error("output sample width"))?, area, &mut budget, control)
        .map_err(|e| controlled(control, e))?;
    // Reorientation is a separately paid scalar pass on the same phase budget.
    if rewrite_empty.is_some() {
        let count = u64::from(area.width())
            .checked_mul(u64::from(area.height()))
            .and_then(|n| n.checked_mul(64))
            .ok_or_else(|| error("Quick Mask reorientation work"))?;
        budget.work(count).map_err(|e| controlled(control, e))?;
    }
    let mut untouched = true;
    let mut all_white = true;
    let mut any_nonzero = false;
    let mut top = area.y0;
    while top < area.y1 {
        let bottom = top.checked_add(SOURCE_CELL_SIZE).ok_or_else(|| error("cell row overflow"))?.min(area.y1);
        let mut left = area.x0;
        while left < area.x1 {
            poll(control)?;
            let right = left.checked_add(SOURCE_CELL_SIZE).ok_or_else(|| error("cell column overflow"))?.min(area.x1);
            let part = Rect::new(left, top, right, bottom);
            let samples = (part.width() as usize).checked_mul(part.height() as usize).ok_or_else(|| error("cell sample overflow"))?;
            let input = source.map(|source| source.try_read_interleaved_with(part, control).map_err(|e| controlled(control, e))).transpose()?;
            let mut output = Vec::new();
            let length = samples.checked_mul(bpp).ok_or_else(|| error("cell length overflow"))?;
            output.try_reserve_exact(length).map_err(|_| error("cell allocation failed"))?;
            if output.capacity() > length.saturating_add(4096) {
                return Err(error("cell capacity exceeded"));
            }
            let input_format = source.map(Surface::format);
            for index in 0..samples {
                if index.is_multiple_of(part.width() as usize) {
                    poll(control)?;
                }
                let mut encoded = [0u8; 8];
                let dest = encoded.get_mut(..bpp).ok_or_else(|| error("native sample width"))?;
                if let (Some(input), Some(input_format)) = (&input, input_format) {
                    let start = index.checked_mul(input_format.bytes_per_pixel()).ok_or_else(|| error("source offset overflow"))?;
                    let sample = input
                        .get(start..start.checked_add(input_format.bytes_per_pixel()).ok_or_else(|| error("source end overflow"))?)
                        .ok_or_else(|| error("short source coverage"))?;
                    if wide {
                        let value =
                            checked_arithmetic::decode_finite(Operand { bytes: sample, storage: Storage::Native(input_format.sample), index: 0 }, Domain::Unit)
                                .map_err(|e| controlled(control, e))?;
                        dest.copy_from_slice(sample);
                        all_white &= value >= f64::from(1.0f32 - 0.5f32 / 255.0f32);
                        any_nonzero |= value != 0.;
                    } else {
                        // Exactly the old f32 coverage clamp / 8-bit selection
                        // quantization; this path is never used for wide words.
                        let value = read_sample(sample, input_format.sample, 0);
                        if !value.is_finite() {
                            return Err(error("nonfinite Quick Mask coverage"));
                        }
                        all_white &= value >= 1.0f32 - 0.5f32 / 255.0f32;
                        if enter {
                            write_sample(dest, format.sample, 0, value.clamp(0., 1.));
                            any_nonzero |= value > 0.;
                        } else {
                            let code = (value.clamp(0., 1.) * 255.0 + 0.5) as u8;
                            *dest.first_mut().ok_or_else(|| error("selection sample width"))? = code;
                            any_nonzero |= code != 0;
                        }
                    }
                } else {
                    let value = if indicates == ColorIndicates::SelectedAreas { 0. } else { 1. };
                    photocraft_color::try_write_sample_f64(dest, format.sample, 0, value).map_err(|e| controlled(control, e))?;
                    any_nonzero |= value != 0.;
                }
                if let Some(expected) = rewrite_empty {
                    let source_format = input_format.ok_or_else(|| error("missing reorientation source"))?;
                    let start = index.checked_mul(source_format.bytes_per_pixel()).ok_or_else(|| error("reorientation offset"))?;
                    let word = input
                        .as_ref()
                        .and_then(|bytes| bytes.get(start..start.checked_add(source_format.bytes_per_pixel())?))
                        .ok_or_else(|| error("reorientation source length"))?;
                    let value = f64::from(read_sample(word, source_format.sample, 0));
                    untouched &= (value - expected).abs() <= f64::from(0.5f32 / 255.0f32);
                    let replacement = if indicates == ColorIndicates::SelectedAreas { 0. } else { 1. };
                    photocraft_color::try_write_sample_f64(dest, format.sample, 0, replacement).map_err(|e| controlled(control, e))?;
                }
                output.extend_from_slice(dest);
            }
            builder.push_region(part, &output, &mut budget, control).map_err(|e| controlled(control, e))?;
            left = right;
        }
        top = bottom;
    }
    let (surface, ()) = builder.finalize(&mut budget, control).map_err(|e| controlled(control, e))?.into_parts();
    poll(control)?;
    if rewrite_empty.is_some() {
        return Ok(if untouched { Some(surface) } else { None });
    }
    if !enter && (!any_nonzero || (unselected && indicates != ColorIndicates::SelectedAreas && all_white)) { Ok(None) } else { Ok(Some(surface)) }
}
pub(super) fn reorient_untouched(snapshot: &crate::DocumentSnapshot, indicates: ColorIndicates, control: &Interrupt<'_>) -> Result<Option<Surface>> {
    if !snapshot.quick_mask_unselected {
        return Ok(None);
    }
    let Some(channel) = &snapshot.quick_mask else { return Ok(None) };
    if channel.indicates == indicates {
        return Ok(None);
    }
    let expected = if channel.indicates == ColorIndicates::SelectedAreas { 0. } else { 1. };
    transfer(snapshot, Some(&channel.surface), TransferPolicy { enter: true, unselected: true, indicates, rewrite_empty: Some(expected) }, control)
}
pub(crate) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let on = state.doc.quick_mask.is_some();
    let want = params.get("on").and_then(Value::as_bool).unwrap_or(!on);
    let response = json!({"quickMask":want});
    if want == on {
        return Ok(response);
    }
    state.doc.validate_resident()?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let work_base = baseline.clone();
    let options = session.quick_mask_options;
    let label = if want { "Edit in Quick Mask" } else { "Exit Quick Mask" };
    crate::jobs::run(
        session,
        label,
        true,
        move |ctx| {
            ctx.stage(0., 0.95, label, |control| {
                let source = if want { work_base.selection.as_ref() } else { work_base.quick_mask.as_ref().map(|channel| &channel.surface) };
                let unselected = if want { work_base.selection.is_none() } else { work_base.quick_mask_unselected };
                let indicates = if want { options.indicates } else { work_base.quick_mask.as_ref().ok_or_else(|| error("missing Quick Mask"))?.indicates };
                let surface = transfer(&work_base, source, TransferPolicy { enter: want, unselected, indicates, rewrite_empty: None }, control)?;
                poll(control)?;
                let mut document = (*work_base).clone();
                if want {
                    let surface = surface.ok_or_else(|| error("missing prepared Quick Mask"))?;
                    let mut name = String::new();
                    name.try_reserve_exact(10).map_err(|_| error("Quick Mask name allocation failed"))?;
                    name.push_str("Quick Mask");
                    let mut channel = photocraft_doc::AlphaChannel::new(name, surface);
                    channel.color = options.color;
                    channel.opacity = options.opacity;
                    channel.indicates = options.indicates;
                    document.quick_mask_unselected = work_base.selection.is_none();
                    document.quick_mask = Some(channel);
                    document.selection = None;
                } else {
                    document.quick_mask = None;
                    document.selection = surface;
                }
                poll(control)?;
                Ok((document, ctx.clone()))
            })
        },
        move |session, (document, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id || current.revision != revision || !current.doc.shares_state(&baseline) {
                return Err(error("target changed while preparing Quick Mask"));
            }
            ctx.stage(0.95, 1., label, |control| {
                poll(control)?;
                session.edit_with_interrupt(label, control, move |current, _| {
                    *current = document;
                    Ok(())
                })?;
                if want && let Some(state) = session.active_mut() {
                    state.channel_view.target = super::ChannelTarget::Composite;
                    state.channel_view.quick_mask_hidden = false;
                }
                Ok(response)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipped_native_tiles_can_enter_and_leave_all_selected_quick_mask() {
        for sample in SampleType::ALL {
            for width in [2, TILE_SIZE as u32 + 1] {
                let mut document = photocraft_doc::Document::new("Clipped Quick Mask", photocraft_doc::Size::new(width, 1), ColorMode::Rgb, sample);
                document.layers.push(Layer::new("Pixels", LayerContent::Raster(Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true)))));
                let mut session = Session::new();
                session.add_document(document, None).unwrap();
                let snapshot = session.active().unwrap().doc.clone();
                let entered = transfer(
                    &snapshot,
                    None,
                    TransferPolicy { enter: true, unselected: true, indicates: ColorIndicates::MaskedAreas, rewrite_empty: None },
                    &Interrupt::NONE,
                )
                .unwrap()
                .unwrap();
                assert_eq!(entered.format(), super::super::channel_format(&snapshot));
                let bytes = entered.try_read_interleaved_with(snapshot.bounds(), &Interrupt::NONE).unwrap();
                for word in bytes.chunks_exact(entered.format().bytes_per_pixel()) {
                    assert_eq!(photocraft_color::try_read_sample_f64(word, sample, 0).unwrap(), 1.);
                }
                assert!(
                    transfer(
                        &snapshot,
                        Some(&entered),
                        TransferPolicy { enter: false, unselected: true, indicates: ColorIndicates::MaskedAreas, rewrite_empty: None },
                        &Interrupt::NONE
                    )
                    .unwrap()
                    .is_none()
                );
                assert!(session.active().unwrap().doc.shares_state(&snapshot));
                assert!(snapshot.selection.is_none());
                assert!(snapshot.quick_mask.is_none());
            }
        }
    }
    fn fixture(sample: SampleType, bytes: &[u8]) -> Session {
        let mut document = photocraft_doc::Document::new("Quick Mask test", photocraft_doc::Size::new(2, 1), ColorMode::Rgb, sample);
        let color = PixelFormat::new(ColorMode::Rgb, sample, true);
        document.layers.push(Layer::new("Pixels", LayerContent::Raster(Surface::new(color))));
        let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
        selection.try_write_interleaved_with(document.bounds(), bytes, &Interrupt::NONE).unwrap();
        document.selection = Some(selection);
        document.physical_resolution = Some(photocraft_doc::PhysicalResolution { y_dpi: 288., ..photocraft_doc::PhysicalResolution::square(144.) });
        document.resolution_dpi = 72.;
        let mut session = Session::new();
        session.add_document(document, None).unwrap();
        session
    }
    #[test]
    fn wide_native_coverage_survives_enter_exit_without_narrowing() {
        for (sample, bytes) in [
            (SampleType::U32, [0x1234_5679u32.to_ne_bytes(), 0x3456_789bu32.to_ne_bytes()].concat()),
            (SampleType::F64, [(-0.0f64).to_ne_bytes(), f64::from_bits(1).to_ne_bytes()].concat()),
        ] {
            let mut session = fixture(sample, &bytes);
            let before = session.active().unwrap().doc.clone();
            let history = session.active().unwrap().history.past_len();
            let colors = before.layers[0].surface().unwrap().try_read_interleaved_with(before.bounds(), &Interrupt::NONE).unwrap();
            assert_eq!(run(&mut session, &json!({"on":true})).unwrap(), json!({"quickMask":true}));
            let entered = &session.active().unwrap().doc;
            assert!(entered.selection.is_none());
            assert_eq!(entered.quick_mask.as_ref().unwrap().surface.try_read_interleaved_with(entered.bounds(), &Interrupt::NONE).unwrap(), bytes);
            assert_eq!(run(&mut session, &json!({"on":false})).unwrap(), json!({"quickMask":false}));
            let state = session.active().unwrap();
            assert_eq!(state.history.past_len(), history + 2);
            assert!(state.doc.quick_mask.is_none());
            assert_eq!(state.doc.selection.as_ref().unwrap().try_read_interleaved_with(state.doc.bounds(), &Interrupt::NONE).unwrap(), bytes);
            assert_eq!(state.doc.layers[0].surface().unwrap().try_read_interleaved_with(state.doc.bounds(), &Interrupt::NONE).unwrap(), colors);
            assert_eq!(state.doc.physical_resolution, before.physical_resolution);
            assert_eq!(state.doc.resolution_dpi, before.resolution_dpi);
            session.execute("edit.undo", json!({})).unwrap();
            assert!(session.active().unwrap().doc.quick_mask.is_some());
            session.execute("edit.redo", json!({})).unwrap();
            assert!(session.active().unwrap().doc.quick_mask.is_none());
        }
    }
    #[test]
    fn ordinary_exit_keeps_legacy_eight_bit_quantization_and_white_sentinel() {
        let bytes = [12345u16.to_ne_bytes(), 54321u16.to_ne_bytes()].concat();
        let session = fixture(SampleType::U16, &bytes);
        let snapshot = &session.active().unwrap().doc;
        let source = snapshot.selection.as_ref().unwrap();
        let legacy_entry = super::super::quick_mask_from(snapshot, &super::super::QuickMaskOptions::default());
        let entry = transfer(
            snapshot,
            Some(source),
            TransferPolicy { enter: true, unselected: false, indicates: ColorIndicates::MaskedAreas, rewrite_empty: None },
            &Interrupt::NONE,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            entry.try_read_interleaved_with(snapshot.bounds(), &Interrupt::NONE).unwrap(),
            legacy_entry.surface.try_read_interleaved_with(snapshot.bounds(), &Interrupt::NONE).unwrap()
        );
        let result = transfer(
            snapshot,
            Some(source),
            TransferPolicy { enter: false, unselected: false, indicates: ColorIndicates::MaskedAreas, rewrite_empty: None },
            &Interrupt::NONE,
        )
        .unwrap()
        .unwrap();
        let legacy_exit = super::super::selection_from(snapshot, &photocraft_doc::AlphaChannel::new("oracle", source.clone())).unwrap();
        let expected: Vec<u8> = bytes.as_chunks::<2>().0.iter().map(|word| (read_sample(word, SampleType::U16, 0).clamp(0., 1.) * 255. + 0.5) as u8).collect();
        assert_eq!(result.format(), PixelFormat::GRAY8);
        assert_eq!(result.try_read_interleaved_with(snapshot.bounds(), &Interrupt::NONE).unwrap(), expected);
        assert_eq!(
            result.try_read_interleaved_with(snapshot.bounds(), &Interrupt::NONE).unwrap(),
            legacy_exit.try_read_interleaved_with(snapshot.bounds(), &Interrupt::NONE).unwrap()
        );
        let white = transfer(
            snapshot,
            None,
            TransferPolicy { enter: true, unselected: true, indicates: ColorIndicates::MaskedAreas, rewrite_empty: None },
            &Interrupt::NONE,
        )
        .unwrap()
        .unwrap();
        assert!(
            transfer(
                snapshot,
                Some(&white),
                TransferPolicy { enter: false, unselected: true, indicates: ColorIndicates::MaskedAreas, rewrite_empty: None },
                &Interrupt::NONE
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    fn invalid_native_domain_and_cancellation_leave_source_unchanged() {
        let bytes = [0.25f64.to_ne_bytes(), f64::NAN.to_ne_bytes()].concat();
        let session = fixture(SampleType::F64, &bytes);
        let snapshot = session.active().unwrap().doc.clone();
        let source = snapshot.selection.as_ref().unwrap();
        assert!(
            transfer(
                &snapshot,
                Some(source),
                TransferPolicy { enter: true, unselected: false, indicates: ColorIndicates::MaskedAreas, rewrite_empty: None },
                &Interrupt::NONE
            )
            .is_err()
        );
        let cancel = || true;
        let control = Interrupt::cancel_only(&cancel);
        assert!(matches!(
            transfer(
                &snapshot,
                Some(source),
                TransferPolicy { enter: true, unselected: false, indicates: ColorIndicates::MaskedAreas, rewrite_empty: None },
                &control
            ),
            Err(EngineError::Cancelled)
        ));
        assert!(session.active().unwrap().doc.shares_state(&snapshot));
        assert_eq!(source.try_read_interleaved_with(snapshot.bounds(), &Interrupt::NONE).unwrap(), bytes);
    }
    #[test]
    fn malformed_and_stale_captured_target_refuse_before_history() {
        let session = fixture(SampleType::U32, &[0x1234_5679u32.to_ne_bytes(), 0x3456_789bu32.to_ne_bytes()].concat());
        let state = session.active().unwrap();
        let snapshot = state.doc.clone();
        let history = state.history.past_len();
        for params in [
            json!({"on":"true"}),
            json!({"on":true,"documentId":snapshot.id.0.to_string()}),
            json!({"on":true,"documentId":snapshot.id.0.to_string(),"expectedRevision":state.revision.saturating_add(1).to_string()}),
            json!({"on":true,"extra":false}),
        ] {
            assert!(dispatch_preflight(&session, &params, false).is_err());
            assert!(session.active().unwrap().doc.shares_state(&snapshot));
            assert_eq!(session.active().unwrap().history.past_len(), history);
        }
    }
}
