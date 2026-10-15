//! UNRUN generated CURRENT translated masked geometry; no source enrollment.
use super::*;
use crate::{
    backed_stock_point_context::tests::{ctl, fixture},
    gimp_composite::WorkingSpace,
};
use photocraft_doc::{
    ColorMode, Layer, LayerContent, LayerMask, SampleEncoding, SampleType, WorkingProfileInterpretation,
    import_compositing::*,
    item_identity::{ItemRef, MaskId},
    xcf_state::MaskState,
};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{BackingError, quota::QuotaKind},
};
use std::sync::Arc;
fn owner(stored: SampleEncoding, word: i32, operator: GimpOperator, enabled: bool, defect: u8) -> AccountedDocument {
    let (mut doc, q, guard, _, basis, _) = fixture(2, ColorMode::Rgb, SampleType::F64, stored);
    let fee = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    let mut top = doc.layers.remove(0);
    let mut base = Layer::new("Backdrop", top.content.clone());
    let original = Arc::new(basis.encoded_bytes().unwrap().to_vec());
    let parsed = photocraft_cms::Profile::parse(&original).unwrap();
    let effective = photocraft_cms::xcf_color::effective_profile_from_definition(&parsed, crate::gimp_composite::encoding(stored)).unwrap();
    doc.icc_profile = Some(Arc::new(effective.encoded_bytes().unwrap().to_vec()));
    doc.source_color_context.as_mut().unwrap().original_icc = Some(original);
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    let LayerContent::BackedRaster(backed) = &top.content else { panic!("backed") };
    let mut translated = backed.try_translated(3, -2, &ctl()).unwrap();
    let extent = translated.operative_bounds();
    if defect == 1 {
        let r = photocraft_geom::Rect::new(extent.x1 - 1, extent.y1 - 1, extent.x1, extent.y1);
        let mut bytes = vec![0; translated.format().bytes_per_pixel()];
        translated.checked_view().read_logical_pixel_native_into(r.x0, r.y0, &mut bytes, &ctl()).unwrap();
        photocraft_color::try_write_sample_f64(&mut bytes, SampleType::F64, 0, f64::NAN).unwrap();
        translated = translated.try_edited_native(r, &bytes, &ctl()).unwrap();
    }
    top.content = LayerContent::BackedRaster(translated);
    top.opacity = 0.375;
    top.visible = defect == 0;
    let maskformat = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut surface = Surface::with_default(maskformat, &[0.5]);
    if defect == 2 {
        surface.try_write_pixel_f64(extent.x1 - 1, extent.y1 - 1, &[1.5]).unwrap();
    }
    top.mask = Some(LayerMask { surface: Arc::new(surface), enabled, linked: true, density: 1., feather: 0. });
    let physical = Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    top.import_compositing = physical;
    base.import_compositing = physical;
    let epoch = match stored {
        SampleEncoding::NonlinearProfile => XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH,
        SampleEncoding::Linear => XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH,
        SampleEncoding::Perceptual => XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH,
    };
    let canvas = doc.bounds();
    let state = doc.xcf_state.as_mut().unwrap();
    let template = state.items[0].clone();
    state.items.clear();
    state.filter_stacks.clear();
    state.filter_masks.clear();
    state.provenance.format_version = 12;
    for (layer, foreground) in [(&base, false), (&top, true)] {
        let mut item = template.clone();
        item.item = ItemRef::Layer(layer.id);
        item.name = layer.name.clone();
        item.visible = layer.visible;
        item.mask = None;
        item.raster_bounds = Some(if foreground { extent } else { canvas });
        item.composition = Some(GimpComposition {
            compatibility_epoch: epoch.into(),
            source_mode: 28,
            legacy: false,
            stored_blend_space: 0,
            stored_composite_space: if foreground { 1 } else { -1 },
            stored_operator: if foreground { word } else { -1 },
            resolved: Some(ResolvedGimpComposition {
                blend_space: GimpSpace::LinearRgb,
                composite_space: GimpSpace::LinearRgb,
                operator: if foreground { operator } else { GimpOperator::Union },
            }),
            pass_through: false,
        });
        state.items.push(item);
    }
    let mut mask = template;
    mask.item = ItemRef::Mask(MaskId(9000));
    mask.name = "Mask".into();
    mask.visible = true;
    mask.composition = None;
    mask.raster_bounds = Some(extent);
    mask.mask = Some(MaskState {
        owner: ItemRef::Layer(top.id),
        bounds: extent,
        offset: [extent.x0, extent.y0],
        outside_f64_bits: 0f64.to_bits(),
        enabled,
        shown: false,
        editing: false,
        linked: true,
    });
    state.items.push(mask);
    if defect == 3 {
        doc.icc_profile.as_mut().unwrap() = Arc::new(vec![0; 132]);
    }
    doc.layers = vec![base, top];
    AccountedDocument::from_owned(doc, Arc::new((guard, fee)), q, &ctl()).unwrap()
}
fn reference(owner: &AccountedDocument, working: &WorkingSpace, x: i32, y: i32, operator: GimpOperator) -> [f64; 4] {
    let mut bg = [0.; 4];
    for (index, layer) in owner.document().layers.iter().enumerate() {
        if !layer.visible {
            continue;
        }
        let LayerContent::BackedRaster(backed) = &layer.content else { panic!("backed") };
        let mut pixel = [0.; 4];
        if backed.operative_bounds().contains(x, y) {
            backed.checked_view().read_logical_pixel_f64_into(x, y, &mut pixel, &ctl()).unwrap();
        }
        let mask = if index == 1 && layer.mask.as_ref().unwrap().enabled { if backed.operative_bounds().contains(x, y) { 0.5 } else { 0. } } else { 1. };
        bg = working.normal(bg, pixel, f64::from(layer.opacity) * mask, if index == 1 { operator } else { GimpOperator::Union }).unwrap();
    }
    bg
}
#[test]
fn shared_assigned_validation_preserves_existing_translated_masked_recipe_math() {
    for stored in [SampleEncoding::NonlinearProfile, SampleEncoding::Linear, SampleEncoding::Perceptual] {
        for (word, op) in [(1, GimpOperator::Union), (2, GimpOperator::ClipToBackdrop), (3, GimpOperator::ClipToLayer), (4, GimpOperator::Intersection)] {
            for enabled in [false, true] {
                let owner = owner(stored, word, op, enabled, 0);
                validate_current_assigned_geometry_storage(&owner, &ctl()).unwrap();
                let profile = PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).unwrap();
                let prepared = PreparedBackedDocument::new_accounted_with_assigned_profile(&owner, &profile, Default::default(), &ctl()).unwrap();
                let working = WorkingSpace::new(profile.profile(), stored, GimpSpace::LinearRgb).unwrap();
                let band = prepared.render_band(owner.document().bounds(), &ctl()).unwrap();
                for y in 0..6 {
                    for x in 0..8 {
                        assert_eq!(band.pixels()[(y * 8 + x) as usize].map(f64::to_bits), reference(&owner, &working, x, y, op).map(f64::to_bits));
                    }
                }
            }
        }
    }
}
#[test]
fn inactive_malformed_translated_tail_mask_or_profile_still_refuse() {
    for defect in [1, 2, 3] {
        let owner = owner(SampleEncoding::Perceptual, 4, GimpOperator::Intersection, false, defect);
        assert!(validate_current_assigned_geometry_storage(&owner, &ctl()).is_err());
    }
}
#[test]
fn shared_validation_preserves_typed_failures_and_releases_both_profile_roles() {
    let owner = owner(SampleEncoding::Linear, 1, GimpOperator::Union, true, 0);
    validate_current_assigned_geometry_storage(&owner, &ctl()).unwrap();
    let before = owner.quota().usage().unwrap();
    let snapshot = owner.clone();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(validate_current_assigned_geometry_storage(&owner, &denied), Err(BackedNormalError::Backing(BackingError::Work))));
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    assert!(matches!(
        validate_current_assigned_geometry_storage(&owner, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }),
        Err(BackedNormalError::Backing(BackingError::Cancelled))
    ));
    assert_eq!(owner.quota().usage().unwrap(), before);
    let fill = owner.quota().reserve(QuotaKind::Resident, (64 << 20) - before.resident).unwrap();
    assert!(matches!(validate_current_assigned_geometry_storage(&owner, &ctl()), Err(BackedNormalError::Quota(_))));
    drop(fill);
    assert!(owner.shares_model(&snapshot));
    assert_eq!(owner.quota().usage().unwrap(), before);
}
