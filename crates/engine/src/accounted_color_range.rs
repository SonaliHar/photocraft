//! Closed registered accounted Color Range publication adapter.
//! Original active-only sampled query/CMS/math stays in BackedColorRangeSamples;
//! source-free RGB/nonlinear analytic ICC Float, fullcanvas<=200side only.
//! Derived unit selection values follow invert -> prior min -> final GRAY8 q,
//! then exact native Gray Float storage. RGB source words are never narrowed.
//! Larger/asynchronous, merged, imported and other selection modes remain refused.
use crate::{
    EngineError, Result, Session,
    accounted_selection_session::AccountedSelectionTarget,
    channel_cmds::ChannelTarget,
    selection_cmds::{BackedColorRangeSamples, ColorRangeLimits},
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    LayerContent, SampleEncoding, WorkingProfileInterpretation,
    accounted_document::accounted_layer_access,
    accounted_selection::{AccountedSelection, CoverageDomain},
};
use photocraft_geom::Rect;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use serde_json::{Value, json};
const CMD: &str = "select.colorRange";
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: message.into() }
}
/// Cheap borrowed branch selection before the journal copies parameters. The
/// existing query constructor alone owns complete query-field/schema validation.
pub(crate) fn validate_request(params: &Value) -> Result<()> {
    let object = params.as_object().filter(|o| o.len() <= 17).ok_or_else(|| bad("explicit bounded active-only Color Range parameters required"))?;
    if object.get("select").is_some_and(|v| v.as_str() != Some("sampledColors")) {
        return Err(bad("accounted Color Range currently supports Sampled Colors only"));
    }
    if object.get("mode").is_some_and(|v| v.as_str() != Some("replace")) {
        return Err(bad("accounted Color Range currently supports replace only"));
    }
    if object.get("sampleAllLayers").and_then(Value::as_bool) != Some(false) {
        return Err(bad("accounted Color Range requires explicit sampleAllLayers:false"));
    }
    if object.get("invert").is_some_and(|v| !v.is_boolean()) {
        return Err(bad("invert must be a boolean"));
    }
    Ok(())
}
fn context(session: &Session, params: &Value, control: &ReadControl<'_>) -> Result<Rect> {
    control.check(1)?;
    validate_request(params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let doc = &state.doc;
    if state.floating.is_some()
        || state.coalesce.is_some()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.active_job().is_some()
        || session.command_has_job_on(doc.id)
        || session.command_has_pending_job()
    {
        return Err(bad("accounted Color Range requires an idle original target"));
    }
    let owner = doc.accounted().ok_or(BackingError::Unsupported)?;
    if doc.mode != ColorMode::Rgb
        || doc.depth != SampleType::F32
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || doc.working_profile_interpretation != WorkingProfileInterpretation::Icc
        || doc.xcf_state.is_some()
        || doc.source_color_context.is_some()
        || doc.selection_channel_id.is_some()
        || doc.quick_mask.is_some()
        || doc.icc_profile.as_ref().is_none_or(|p| p.is_empty() || p.len() > 16 << 20)
    {
        return Err(bad("accounted Color Range requires source-free explicit nonlinear RGB Float/analytic ICC context"));
    }
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 200 || doc.size.height > 200 {
        return Err(bad("accounted Color Range synchronous canvas limit is 200 pixels per side"));
    }
    let view = &state.channel_view;
    if view.target != ChannelTarget::Composite
        || view.layer_mask.is_some()
        || view.editing_mask.is_some()
        || view.color_hidden.len() > 3
        || view.alpha_visible.len() > 16
        || view.color_hidden.iter().any(|v| *v)
        || view.alpha_visible.iter().any(|v| *v)
    {
        return Err(bad("accounted Color Range requires the ordinary color-channel view"));
    }
    // The actual current private selection guard and bounded clone domain are
    // checked before query/CMS/pixel work; readonly adopted unknown selections refuse.
    owner.validate_selection_replacement(control)?;
    let id = state.active_layer.ok_or_else(|| bad("active native pixel layer required"))?;
    let access = accounted_layer_access(doc, id, control)?.ok_or(BackingError::Bounds)?;
    let layer = access.layer;
    let LayerContent::BackedRaster(backed) = &layer.content else {
        return Err(BackingError::Unsupported.into());
    };
    let area = doc.bounds();
    if backed.format().mode != ColorMode::Rgb
        || backed.format().sample != SampleType::F32
        || !backed.operative_bounds().contains_rect(&area)
        || !backed.quota().shares_ledger(owner.quota())
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || !layer.effects.items.is_empty()
        || layer.effects.psd_raw.is_some()
        || layer.import_compositing.is_some()
        || layer.indexed_source.is_some()
        || layer.video.is_some()
        || layer.fill_cache.is_some()
    {
        return Err(bad("active native owner or fullcanvas bounds are unsupported for accounted Color Range"));
    }
    control.check(1)?;
    Ok(area)
}
struct NativeSelectionBytes {
    bytes: Vec<u8>,
    _lease: QuotaLease,
}
fn native_bytes(
    mask: &[f32],
    prior: Option<&photocraft_raster::Surface>,
    area: Rect,
    invert: bool,
    quota: &BackingQuota,
    control: &ReadControl<'_>,
) -> Result<(NativeSelectionBytes, bool)> {
    let count = usize::try_from(area.size().area()).map_err(|_| BackingError::Length)?;
    if mask.len() != count {
        return Err(BackingError::Length.into());
    }
    let length = count.checked_mul(4).ok_or(BackingError::Length)?;
    let envelope = length.checked_mul(2).and_then(|v| v.checked_add(std::mem::size_of::<NativeSelectionBytes>())).ok_or(BackingError::Length)?;
    let mut lease = quota.reserve(QuotaKind::Resident, envelope as u64).map_err(|_| BackingError::Quota)?;
    control.check(length as u64)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| BackingError::Allocation)?;
    if bytes.capacity() > length.checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    // Destroy any oversized buffer before its physical reservation leaves scope.
    let actual = bytes.capacity().checked_add(std::mem::size_of::<NativeSelectionBytes>()).ok_or(BackingError::Length)?;
    lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Quota)?;
    let mut native = NativeSelectionBytes { bytes, _lease: lease };
    let width = usize::try_from(area.width()).map_err(|_| BackingError::Length)?;
    let mut selected = false;
    for (i, value) in mask.iter().copied().enumerate() {
        if i.is_multiple_of(256) {
            control.check(256 * 64)?;
        }
        if !value.is_finite() {
            return Err(bad("finite Color Range coverage required"));
        }
        let mut value = if invert { 1. - value } else { value };
        if let Some(prior) = prior {
            if prior.format() != PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false) {
                return Err(BackingError::Unsupported.into());
            }
            let x = area.x0.checked_add(i32::try_from(i % width).map_err(|_| BackingError::Length)?).ok_or(BackingError::Bounds)?;
            let y = area.y0.checked_add(i32::try_from(i / width).map_err(|_| BackingError::Length)?).ok_or(BackingError::Bounds)?;
            let mut word = [0; 4];
            prior.try_read_pixel_bytes(x, y, &mut word)?;
            let old = f32::from_ne_bytes(word);
            if !old.is_finite() || !(0.0..=1.0).contains(&old) {
                return Err(bad("finite unit prior selection required"));
            }
            value = value.min(old);
        }
        // Exact authoritative command order, including unquantized Float priors.
        let quantized = (value.clamp(0., 1.) * 255. + 0.5) as u8;
        selected |= quantized != 0;
        let derived = f32::from(quantized) / 255.;
        native.bytes.extend_from_slice(&derived.to_ne_bytes());
    }
    control.check(1)?;
    if native.bytes.len() != length {
        return Err(BackingError::Length.into());
    }
    Ok((native, selected))
}
/// Called only by the registered AccountedCommand branch after its real journal
/// has been staged. No extra job, scratch Session or reset work ledger is created.
pub(crate) fn run(session: &mut Session, params: &Value, control: &ReadControl<'_>) -> Result<Value> {
    let area = context(session, params, control)?;
    let target = AccountedSelectionTarget::capture(session)?;
    let source = session.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let quota = source.quota().ok_or(BackingError::Unsupported)?.clone();
    let query = BackedColorRangeSamples::new_with_read_control(session, params, control)?;
    let pixels = query.backed_pixels_with_read_control(area, control)?;
    let coverage = query.coverage_backed_pixels_with_read_control(&pixels, ColorRangeLimits::default(), control)?;
    let invert = params.get("invert").and_then(Value::as_bool).unwrap_or(false);
    let (native, selected) = native_bytes(coverage.values(), source.selection.as_ref(), area, invert, &quota, control)?;
    let selection = if selected {
        Some(AccountedSelection::try_from_native(
            PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false),
            area,
            &0f32.to_ne_bytes(),
            &native.bytes,
            CoverageDomain::FiniteUnit,
            &quota,
            control,
        )?)
    } else {
        None
    };
    // Bounded API response belongs to the command caller, not the retained model.
    // Construct it before the final transactional publication; no fallible work after.
    let result = json!({"selected":selected});
    control.check(1)?;
    if query.backed_source_is_current(session) != Some(true) {
        return Err(bad("the original Color Range source or view changed during preparation"));
    }
    session.replace_accounted_selection(target, "Color Range", selection.as_ref(), control)?;
    Ok(result)
}
#[cfg(test)]
#[path = "accounted_color_range_tests.rs"]
mod tests;
