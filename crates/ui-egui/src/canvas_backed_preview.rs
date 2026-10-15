//! Explicit bounded backed CPU preview; no resident fallback or source materialization.
use super::display_image;
use photocraft_doc::{Document, LayerContent, WorkingProfileInterpretation};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{QuotaKind, QuotaLease},
    },
};

// Field order releases the allocation before its accounting on every error path.
struct ReducedInput {
    buffer: photocraft_compose::Buffer,
    _lease: QuotaLease,
}

fn backed_display_image_with_owner(
    doc: &Document,
    owner: Option<&photocraft_doc::accounted_document::AccountedDocument>,
    width: u32,
    height: u32,
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
    interrupt: &Interrupt<'_>,
) -> Result<(egui::ColorImage, QuotaLease), String> {
    let render = || -> Result<_, String> {
        let threshold = owner.is_some_and(|owner| owner.is_readonly_threshold_alpha_storage());
        // Current bytes alone supply interpretation; import provenance is inert.
        if !threshold && !matches!(doc.working_profile_interpretation, WorkingProfileInterpretation::Icc) {
            return Err("backed canvas requires explicit current ICC".into());
        }
        let bytes = doc.icc_profile.as_deref().ok_or("backed canvas current ICC is missing")?;
        let quota = doc
            .layers
            .iter()
            .find_map(|layer| match &layer.content {
                LayerContent::BackedRaster(backed) => Some(backed.quota()),
                _ => None,
            })
            .ok_or("backed canvas has no owned source")?;
        let work = std::sync::atomic::AtomicU64::new(0);
        let charge = |units: u64| {
            work.try_update(std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire, |used| {
                used.checked_add(units).filter(|next| *next <= 36_000_000u64 * 512)
            })
            .map(|_| ())
            .map_err(|_| BackingError::Work)
        };
        let control = ReadControl { interrupt, work: &charge };
        // The explicit runtime class uses an actual builtin byte witness. Its
        // compositor validates current bytes and the implicit marker again;
        // no source epoch or supplied profile name substitutes for that check.
        // The ordinary ICC parser and its existing envelope stay unchanged.
        let mut profile_lease = None;
        let parsed = if threshold {
            None
        } else {
            if bytes.len() > 64 << 10 {
                return Err("backed canvas ICC exceeds bounded analytic profile scope".into());
            }
            let tags =
                bytes.get(128..132).and_then(|raw| <[u8; 4]>::try_from(raw).ok()).map(u32::from_be_bytes).ok_or("backed canvas ICC tag table missing")?;
            if tags > 32 {
                return Err("backed canvas ICC tag count exceeds scope".into());
            }
            let envelope = u64::try_from(bytes.len())
                .ok()
                .and_then(|n| n.checked_mul(u64::from(tags).max(1)))
                .and_then(|n| n.checked_mul(8))
                .and_then(|n| n.checked_add(65_536))
                .ok_or("backed canvas profile size overflow")?;
            profile_lease = Some(quota.reserve(QuotaKind::Resident, envelope).map_err(|e| e.to_string())?);
            doc.working_profile_interpretation.validate_with(Some(bytes), doc.mode).map_err(|e| e.to_string())?;
            let space = match doc.mode {
                photocraft_doc::ColorMode::Rgb => photocraft_cms::ColorSpace::Rgb,
                photocraft_doc::ColorMode::Grayscale => photocraft_cms::ColorSpace::Gray,
                _ => return Err("backed canvas model unsupported".into()),
            };
            let profile = photocraft_cms::xcf_color::parse_bounded_profile(bytes, space).map_err(|e| e.to_string())?;
            photocraft_cms::xcf_color::qualify_analytic_profile(&profile).map_err(|e| e.to_string())?;
            Some(profile)
        };
        let profile = if let Some(profile) = &parsed {
            profile
        } else {
            control.check(1).map_err(|e| e.to_string())?;
            let known = match doc.mode {
                photocraft_doc::ColorMode::Rgb => photocraft_cms::Builtin::Srgb.profile(),
                photocraft_doc::ColorMode::Grayscale => photocraft_cms::Builtin::SGray.profile(),
                _ => return Err("backed canvas model unsupported".into()),
            };
            control.check(1).map_err(|e| e.to_string())?;
            known
        };
        let limits = photocraft_compose::backed_normal::NormalBandLimits::default();
        // Only the typed owner establishes retained model/mask/source accounting.
        // Errors from this route are authoritative; never retry a resident route.
        let prepared = if let Some(owner) = owner {
            photocraft_compose::backed_document::PreparedBackedDocument::new_accounted(owner, profile, limits, &control)
        } else {
            photocraft_compose::backed_document::PreparedBackedDocument::new(doc, profile, limits, &control)
        }
        .map_err(|e| e.to_string())?;
        let output = prepared.render_reduced(width, height, &control).map_err(|e| e.to_string())?;
        // Keep any parsed profile allocation charged until its prepared consumer
        // and the parsed profile itself leave this scope.
        let _ = &profile_lease;
        let (buffer, lease) = output.into_parts();
        let input = ReducedInput { buffer, _lease: lease };
        let buffer = &input.buffer;
        // Bound existing display conversion to 64 pixels per poll. The source f32
        // owner stays charged; the persistent derivative and small conversion scratch
        // are prepaid on the same quota before any allocation.
        let count = buffer.px.len();
        let envelope = u64::try_from(count).ok().and_then(|n| n.checked_mul(4)).and_then(|n| n.checked_add(64 * 24)).ok_or("backed display size overflow")?;
        let mut display_lease = quota.reserve(QuotaKind::Resident, envelope).map_err(|e| e.to_string())?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(count).map_err(|_| "backed display allocation failed")?;
        let mut scratch = photocraft_compose::Buffer { rect: photocraft_geom::Rect::new(0, 0, 0, 1), px: Vec::new() };
        scratch.px.try_reserve_exact(64).map_err(|_| "backed display scratch allocation failed")?;
        let actual = u64::try_from(pixels.capacity()).ok().and_then(|n| n.checked_mul(4)).ok_or("backed display capacity overflow")?;
        let scratch_bytes = u64::try_from(scratch.px.capacity())
            .ok()
            .and_then(|n| n.checked_mul(16))
            .and_then(|n| n.checked_add(64 * 8))
            .ok_or("backed display scratch overflow")?;
        if actual.checked_add(scratch_bytes).is_none_or(|n| n > display_lease.bytes()) {
            drop(scratch);
            drop(pixels);
            return Err("backed display capacity exceeds precharge".into());
        }
        for chunk in buffer.px.chunks(64) {
            control.check(chunk.len() as u64).map_err(|e| e.to_string())?;
            scratch.px.clear();
            scratch.px.extend_from_slice(chunk);
            scratch.rect = photocraft_geom::Rect::new(0, 0, i32::try_from(chunk.len()).map_err(|_| "backed display chunk overflow")?, 1);
            let converted = display_image(display, &scratch);
            pixels.extend_from_slice(&converted.pixels);
        }
        drop(scratch);
        control.check(0).map_err(|e| e.to_string())?;
        let image = egui::ColorImage::new(
            [
                usize::try_from(buffer.rect.width()).map_err(|_| "backed display width")?,
                usize::try_from(buffer.rect.height()).map_err(|_| "backed display height")?,
            ],
            pixels,
        );
        if let Err(error) = display_lease.reconcile_capacity(actual) {
            drop(image);
            return Err(error.to_string());
        }
        drop(input);
        Ok((image, display_lease))
    };
    render().map_err(|e| format!("Backed canvas: {e}"))
}
#[cfg(test)]
#[path = "canvas_backed_preview_tests.rs"]
mod tests;

pub(super) fn backed_snapshot_display_image(
    snapshot: &photocraft_engine::document_snapshot::DocumentSnapshot,
    width: u32,
    height: u32,
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
    interrupt: &Interrupt<'_>,
) -> Result<(egui::ColorImage, QuotaLease), String> {
    backed_display_image_with_owner(snapshot, snapshot.accounted(), width, height, display, interrupt)
}
#[cfg(test)]
#[path = "canvas_backed_mask_tests.rs"]
mod mask_tests;

#[cfg(test)]
#[path = "canvas_backed_threshold_tests.rs"]
mod threshold_tests;

#[cfg(test)]
fn backed_display_image(
    doc: &Document,
    width: u32,
    height: u32,
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
    interrupt: &Interrupt<'_>,
) -> Result<(egui::ColorImage, QuotaLease), String> {
    backed_display_image_with_owner(doc, None, width, height, display, interrupt)
}
