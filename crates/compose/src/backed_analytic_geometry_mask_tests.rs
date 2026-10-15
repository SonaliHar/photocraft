//! UNRUN CURRENT mask activation; original source qualification is unchanged.
use super::*;
use crate::{backed_assigned_current_profile::PreparedAssignedCurrentProfile, backed_document::PreparedBackedDocument, gimp_composite::WorkingSpace};
fn mask_owner(sample: SampleType, stored: SampleEncoding, word: i32, op: GimpOperator, enabled: bool, canonical: bool, defect: bool) -> AccountedDocument {
    let source = assigned_operator(sample, stored, word, op);
    let q = source.quota().clone();
    // Fixed two-layer model/mask replacement, paid before cloning. Immutable
    // original native/profile/source storage remains held by the source owner.
    let fee = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    let mut doc = source.document().clone();
    let bounds = doc.bounds();
    let layer = &mut doc.layers[1];
    layer.mask.as_mut().unwrap().enabled = enabled;
    let id = layer.id;
    if defect {
        layer.visible = false;
        layer.opacity = 0.0;
        let mask = layer.mask.as_mut().unwrap();
        let mut surface = Surface::new(mask.surface.format());
        surface.try_write_pixel_f64(bounds.x1 - 1, bounds.y1 - 1, &[1.5]).unwrap();
        mask.surface = Arc::new(surface);
    }
    let state = doc.xcf_state.as_mut().unwrap();
    if defect {
        state.items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().visible = false;
    }
    state.items.iter_mut().find_map(|i| i.mask.as_mut().filter(|m| m.owner == ItemRef::Layer(id))).unwrap().enabled = canonical;
    adopt(doc, q, Arc::new((source, fee)))
}
fn reference(owner: &AccountedDocument, working: &WorkingSpace, x: i32, y: i32) -> [f64; 4] {
    // Independent existing Normal equations over actual decoded source samples;
    // do not call the source-recipe-only resident document qualifier here.
    let doc = owner.document();
    let mut backdrop = [0.; 4];
    for layer in &doc.layers {
        if !layer.visible {
            continue;
        }
        let LayerContent::BackedRaster(backed) = &layer.content else { panic!("backed fixture") };
        let mut native = [0.; 4];
        let mut source = [0.; 4];
        if backed.operative_bounds().contains(x, y) {
            backed.checked_view().read_logical_pixel_f64_into(x, y, &mut native[..backed.format().channels()], &ctl()).unwrap();
            source = if doc.mode == ColorMode::Grayscale { [native[0], native[0], native[0], native[1]] } else { native };
        }
        let mut coverage = f64::from(layer.opacity);
        if let Some(mask) = layer.mask.as_ref().filter(|m| m.enabled) {
            let meta = doc.xcf_state.as_ref().unwrap().items.iter().find_map(|i| i.mask.as_ref().filter(|m| m.owner == ItemRef::Layer(layer.id))).unwrap();
            let mut value = [f64::from_bits(meta.outside_f64_bits)];
            if meta.bounds.contains(x, y) {
                mask.surface.try_read_pixel_f64(x, y, &mut value).unwrap();
            }
            coverage *= value[0];
        }
        let op = doc
            .xcf_state
            .as_ref()
            .unwrap()
            .items
            .iter()
            .find(|i| i.item == ItemRef::Layer(layer.id))
            .unwrap()
            .composition
            .as_ref()
            .unwrap()
            .checked_resolved()
            .unwrap()
            .operator;
        backdrop = working.normal(backdrop, source, coverage, op).unwrap();
    }
    backdrop
}
#[test]
fn disabled_current_mask_matches_existing_normal_equations_and_keeps_raw_native_storage() {
    for (sample, stored) in [
        (SampleType::U16, SampleEncoding::NonlinearProfile),
        (SampleType::U32, SampleEncoding::NonlinearProfile),
        (SampleType::F32, SampleEncoding::NonlinearProfile),
        (SampleType::F64, SampleEncoding::NonlinearProfile),
        (SampleType::F32, SampleEncoding::Linear),
        (SampleType::F64, SampleEncoding::Linear),
        (SampleType::F32, SampleEncoding::Perceptual),
        (SampleType::F64, SampleEncoding::Perceptual),
    ] {
        for (word, op) in [(1, GimpOperator::Union), (2, GimpOperator::ClipToBackdrop), (3, GimpOperator::ClipToLayer), (4, GimpOperator::Intersection)] {
            for enabled in [true, false] {
                let owner = mask_owner(sample, stored, word, op, enabled, enabled, false);
                let raw = native_storage_hash(&owner);
                let binding = PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).unwrap();
                qualify_assigned(&owner, &binding, &ctl()).unwrap();
                let renderer = PreparedBackedDocument::new_accounted_with_assigned_profile(&owner, &binding, Default::default(), &ctl()).unwrap();
                let r = owner.document().bounds();
                let band = renderer.render_band(r, &ctl()).unwrap();
                let _oracle_fee = owner.quota().reserve(QuotaKind::Resident, 65536).unwrap();
                let oracle = WorkingSpace::new(binding.profile(), stored, GimpSpace::LinearRgb).unwrap();
                for (y, row) in band.pixels().chunks_exact(r.width() as usize).enumerate() {
                    for (x, actual) in row.iter().enumerate() {
                        assert_eq!(*actual, reference(&owner, &oracle, r.x0 + x as i32, r.y0 + y as i32), "{sample:?}/{stored:?}/{op:?}/{enabled}");
                    }
                }
                if !enabled {
                    assert!(qualify(&owner, binding.profile(), &ctl()).is_err(), "narrow/source mask-active route remains closed");
                }
                assert_eq!(native_storage_hash(&owner), raw);
            }
        }
    }
}
#[test]
fn disabled_and_hidden_mask_still_refuses_malformed_last_sample_and_incoherent_flags() {
    for (physical, canonical) in [(false, true), (true, false)] {
        let owner = mask_owner(SampleType::F64, SampleEncoding::NonlinearProfile, 1, GimpOperator::Union, physical, canonical, false);
        assert!(PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).is_err());
    }
    let owner = mask_owner(SampleType::F64, SampleEncoding::NonlinearProfile, 1, GimpOperator::Union, false, false, true);
    let binding = PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).unwrap();
    assert!(qualify_assigned(&owner, &binding, &ctl()).is_err(), "disabled hidden mask tail must still be scanned");
}
#[test]
fn disabled_mask_validation_keeps_parent_control_and_private_scratch_refunds() {
    let owner = mask_owner(SampleType::F64, SampleEncoding::NonlinearProfile, 1, GimpOperator::Union, false, false, false);
    let binding = PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).unwrap();
    qualify_assigned(&owner, &binding, &ctl()).unwrap();
    let before = owner.quota().usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(qualify_assigned(&owner, &binding, &denied), Err(E::Backing(BackingError::Work))));
    let cancelled = || true;
    let stop = Interrupt::cancel_only(&cancelled);
    assert!(matches!(qualify_assigned(&owner, &binding, &ReadControl { interrupt: &stop, work: &|_| Ok(()) }), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(owner.quota().usage().unwrap(), before);
}
