//! Existing qualified Exposure canvas path; no source qualification or numeric change.
use photocraft_compose::gimp_singleton_preparation::{self as singleton, Budget, Lease, Limits, SingletonBuffer};
use photocraft_doc::Document;
use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
fn error(e: impl std::fmt::Display) -> String {
    format!("XCF singleton canvas: {e}")
}
pub(super) fn preview(doc: &Document, w: u32, h: u32, damage: Option<Rect>, control: &Interrupt<'_>) -> Result<SingletonBuffer<Lease>, String> {
    if !singleton::supports(doc) {
        return Err("XCF singleton canvas: unsupported member".into());
    }
    let limits = Limits::default();
    let mut budget = Budget::new(limits).map_err(error)?;
    // Admission of genuine current-model/payload census temporaries and source
    // before their indexes allocate. The immutable current snapshot is still
    // held by the canvas caller; no source model/payload is cloned here.
    let mut source = budget.reserve(limits.resident_bytes, control).map_err(error)?;
    budget.work(limits.resident_bytes.checked_mul(8).ok_or_else(|| error("census work overflow"))?, control).map_err(error)?;
    let raster = photocraft_engine::DocumentSnapshot::resident_raster_footprint_of(doc, control).map_err(error)?;
    let raster_bytes = raster.retained_bytes;
    let model_room = limits.resident_bytes.checked_sub(raster_bytes).ok_or_else(|| error("retained raster limit"))?;
    // Covers full bounded byte/identity census plus repeated validators; actual
    // constructor and render work additionally consume this same lower ledger.
    let mut scan = photocraft_format::PreparationLimits::default();
    scan.resident_bytes = usize::try_from(model_room).map_err(error)?;
    scan.index_bytes = scan.index_bytes.min(scan.resident_bytes);
    scan.manifest_bytes = scan.manifest_bytes.min(scan.resident_bytes);
    scan.object_bytes = scan.object_bytes.min(scan.resident_bytes);
    let model = photocraft_io::retained_native_model_census(doc, scan, control).map_err(error)?;
    scan.resident_bytes =
        usize::try_from(limits.resident_bytes).map_err(error)?.checked_sub(model.retained_bytes()).ok_or_else(|| error("model/payload coexistence"))?;
    scan.index_bytes = scan.index_bytes.min(scan.resident_bytes);
    scan.manifest_bytes = scan.manifest_bytes.min(scan.resident_bytes);
    scan.object_bytes = scan.object_bytes.min(scan.resident_bytes);
    let payload = photocraft_io::retained_native_payload_census(doc, scan, control).map_err(error)?;
    let census = photocraft_io::NativeCurrentCensus::join(model, payload).map_err(error)?;
    if !census.belongs_to(doc) {
        return Err(error("current census identity"));
    }
    let retained = census
        .retained_bytes()
        .checked_add(census.known_index_reservation_bytes())
        .and_then(|n| n.checked_add(census.known_map_reservation_bytes()))
        .ok_or_else(|| error("source census overflow"))?;
    // Scanner-owned index allocations have been destroyed; shrink only their
    // construction envelope, retaining the complete genuine source bound.
    source.retain_bytes(u64::try_from(retained).map_err(error)?).map_err(error)?;
    let mut prepared = singleton::prepare(doc, control, &mut budget, source).map_err(error)?;
    prepared.render_reduced(w, h, damage, control).map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frozen_exposure() -> Document {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_POINT_PREVIEW_BASELINE").expect("reviewed frozen Exposure authority directory"));
        let bytes = std::fs::read(root.join("exposure-source.before.pcraft")).unwrap();
        photocraft_format::load_from_bytes(&bytes).unwrap()
    }

    #[test]
    #[ignore = "requires reviewed unpublished frozen Exposure native BEFORE authority"]
    fn full_and_damage_adapter_preserve_existing_qualified_renderer_words() {
        let doc = frozen_exposure();
        let before = doc.clone();
        assert!(singleton::supports(&doc));
        let baseline = photocraft_compose::gimp_render::PreparedRenderer::new(&doc).unwrap();
        for damage in [None, Some(Rect::new(3, 4, 11, 13))] {
            let expected = baseline.render_reduced(32, 32, damage, &mut |_| Ok(())).unwrap();
            let output = preview(&doc, 32, 32, damage, &Interrupt::NONE).unwrap();
            assert_eq!(output.buffer().rect, expected.rect);
            assert_eq!(output.buffer().px.len(), expected.px.len());
            for (actual, old) in output.buffer().px.iter().zip(&expected.px) {
                assert_eq!(actual.map(f32::to_bits), old.map(f32::to_bits));
            }
        }
        assert_eq!(doc, before);
    }

    #[test]
    #[ignore = "requires reviewed unpublished frozen Exposure native BEFORE authority"]
    fn cancellation_and_selected_invalid_member_do_not_fall_back_or_mutate_source() {
        let mut doc = frozen_exposure();
        let before = doc.clone();
        assert!(preview(&doc, 32, 32, None, &Interrupt::cancel_only(&|| true)).is_err());
        assert_eq!(doc, before);
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
            photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(f32::NAN.to_bits()));
        assert!(singleton::supports(&doc), "metadata member selection precedes parameter validation");
        let invalid_before = doc.clone();
        assert!(preview(&doc, 32, 32, None, &Interrupt::NONE).is_err());
        assert_eq!(doc, invalid_before);
    }
}
