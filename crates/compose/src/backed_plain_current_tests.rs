//! UNRUN generated CURRENT plain recipes; no source qualification claim.
use super::*;
use crate::{
    backed_stock_point_context::tests::{ctl, fixture},
    gimp_composite::WorkingSpace,
};
use photocraft_color::try_read_sample_f64;
use photocraft_doc::{
    BackedLayer, ImportCompositing, SampleEncoding, SampleType,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::ItemRef,
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore,
        quota::{BackingQuota, QuotaLease},
    },
};
use std::sync::Arc;
#[derive(Debug)]
struct InvalidDefault {
    descriptor: NativePlaneDescriptor,
    q: BackingQuota,
    _fee: QuotaLease,
}
impl NativePlaneStore for InvalidDefault {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, at: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        if at.checked_add(out.len() as u64).is_none_or(|n| n > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        out.fill(0);
        Ok(())
    }
}
fn current(mode: ColorMode, sample: SampleType, dx: i32, dy: i32, hidden: bool, defect: u8) -> (AccountedDocument, Vec<u8>, &'static photocraft_cms::Profile) {
    let (mut doc, q, guard, _, profile, raw) = fixture(2, mode, sample, SampleEncoding::NonlinearProfile);
    // Real current metadata staging reservation precedes its model edits.
    let fee = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    let layer = &mut doc.layers[0];
    let LayerContent::BackedRaster(original) = &layer.content else { panic!("backed fixture") };
    let mut backed = original.try_translated(dx, dy, &ctl()).unwrap();
    if defect == 1 {
        let r = backed.operative_bounds();
        let last = Rect::new(r.x1 - 1, r.y1 - 1, r.x1, r.y1);
        let mut pixel = vec![0; backed.format().bytes_per_pixel()];
        backed.checked_view().read_logical_pixel_native_into(last.x0, last.y0, &mut pixel, &ctl()).unwrap();
        photocraft_color::try_write_sample_f64(&mut pixel, sample, 0, f64::NAN).unwrap();
        backed = backed.try_edited_native(last, &pixel, &ctl()).unwrap();
    }
    if defect == 3 {
        let mut default = vec![0; backed.format().bytes_per_pixel()];
        photocraft_color::try_write_sample_f64(&mut default, sample, 0, f64::NAN).unwrap();
        let descriptor = NativePlaneDescriptor::new(backed.format(), backed.operative_bounds(), &default).unwrap();
        let storefee = q.reserve(QuotaKind::Resident, (std::mem::size_of::<InvalidDefault>() + 128) as u64).unwrap();
        let store = Arc::new(InvalidDefault { descriptor, q: q.clone(), _fee: storefee });
        backed = BackedLayer::new(BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap(), descriptor, &ctl()).unwrap();
    }
    layer.content = LayerContent::BackedRaster(backed);
    layer.visible = !hidden;
    layer.import_compositing =
        Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    let id = layer.id;
    let LayerContent::BackedRaster(backed) = &layer.content else { panic!("backed") };
    let state = doc.xcf_state.as_mut().unwrap();
    state.filter_stacks.clear();
    state.filter_masks.clear();
    state.items.retain(|item| item.item == ItemRef::Layer(id));
    let item = &mut state.items[0];
    item.mask = None;
    item.visible = !hidden;
    item.raster_bounds = Some(backed.operative_bounds());
    item.composition = Some(GimpComposition {
        compatibility_epoch: "xcf-12-normal".into(),
        source_mode: 28,
        legacy: false,
        stored_blend_space: 1,
        stored_composite_space: 1,
        stored_operator: 1,
        resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
        pass_through: false,
    });
    if defect == 2 {
        doc.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) };
    }
    (AccountedDocument::from_owned(doc, Arc::new((guard, fee)), q, &ctl()).unwrap(), raw, profile)
}
#[test]
fn translated_plain_recipe_uses_existing_normal_math_and_exact_raw_words() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in SampleType::ALL {
            let (owner, raw, profile) = current(mode, sample, 3, -2, false, 0);
            validate_current_plain_import_storage(&owner, &ctl()).unwrap();
            let doc = owner.document();
            let prepared = PreparedBackedDocument::new_accounted(&owner, profile, Default::default(), &ctl()).unwrap();
            let band = prepared.render_band(doc.bounds(), &ctl()).unwrap();
            let working = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
            let format = doc.pixel_format();
            let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("backed") };
            for y in 0..6 {
                for x in 0..8 {
                    let expected = if layer.operative_bounds().contains(x, y) {
                        let index = ((y + 2) * 8 + (x - 3)) as usize * format.bytes_per_pixel();
                        let pixel = &raw[index..index + format.bytes_per_pixel()];
                        let mut actual = vec![0; pixel.len()];
                        layer.checked_view().read_logical_pixel_native_into(x, y, &mut actual, &ctl()).unwrap();
                        assert_eq!(actual, pixel);
                        let mut rgba = [0.; 4];
                        for (channel, v) in rgba.iter_mut().enumerate().take(format.channels()) {
                            *v = try_read_sample_f64(pixel, sample, channel).unwrap();
                        }
                        let rgba = if mode == ColorMode::Grayscale { [rgba[0], rgba[0], rgba[0], rgba[1]] } else { rgba };
                        working.normal([0.; 4], rgba, 1., GimpOperator::Union).unwrap()
                    } else {
                        [0.; 4]
                    };
                    assert_eq!(band.pixels()[(y * 8 + x) as usize].map(f64::to_bits), expected.map(f64::to_bits));
                }
            }
        }
    }
}
#[test]
fn hidden_offcanvas_or_default_malformed_words_and_bad_marker_refuse() {
    for (hidden, dx, defect) in [(true, 0, 1), (false, 258, 1), (true, 258, 3), (false, 0, 2)] {
        let (owner, _, _) = current(ColorMode::Rgb, SampleType::F64, dx, 255, hidden, defect);
        assert!(validate_current_plain_import_storage(&owner, &ctl()).is_err());
    }
    let (owner, _, _) = current(ColorMode::Rgb, SampleType::F64, 258, 255, true, 0);
    validate_current_plain_import_storage(&owner, &ctl()).unwrap();
}
#[test]
fn current_plain_validation_preserves_owner_and_returns_typed_control_and_quota_errors() {
    let (owner, _, _) = current(ColorMode::Rgb, SampleType::F64, 3, -2, false, 0);
    validate_current_plain_import_storage(&owner, &ctl()).unwrap();
    let baseline = owner.quota().usage().unwrap();
    let snapshot = owner.clone();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(validate_current_plain_import_storage(&owner, &denied), Err(Error::Backing(BackingError::Work))));
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    assert!(matches!(
        validate_current_plain_import_storage(&owner, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }),
        Err(Error::Backing(BackingError::Cancelled))
    ));
    assert_eq!(owner.quota().usage().unwrap(), baseline);
    let fill = owner.quota().reserve(QuotaKind::Resident, (64 << 20) - baseline.resident).unwrap();
    assert!(matches!(validate_current_plain_import_storage(&owner, &ctl()), Err(Error::Quota(_))));
    drop(fill);
    assert_eq!(owner.quota().usage().unwrap(), baseline);
    assert!(owner.shares_model(&snapshot));
}
