//! UNRUN accounted-selection lifecycle drafts; no codec/import qualification.
use super::*;
use crate::{
    BackedLayer, Document, Layer, LayerContent, Size,
    accounted_document::{AccountedDocument, AccountedEdit, AccountedHistory},
};
use photocraft_raster::backing::quota::QuotaLimits;
use photocraft_raster::{
    Interrupt,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore},
};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 16 << 20, index: 8 << 20 })
}
fn selection(q: &BackingQuota) -> AccountedSelection {
    let f = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let raw: Vec<u8> = [-0.0f64, 0.125, 0.5, 1., 0.75, 0.25].into_iter().flat_map(f64::to_ne_bytes).collect();
    AccountedSelection::try_from_native(f, Rect::new(0, 0, 3, 2), &(-0.0f64).to_ne_bytes(), &raw, CoverageDomain::FiniteUnit, q, &control()).unwrap()
}
#[derive(Debug)]
struct Store(NativePlaneDescriptor);
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let pixel = self.0.default_bytes();
        for (i, b) in out.iter_mut().enumerate() {
            *b = pixel[((offset + i as u64) % pixel.len() as u64) as usize];
        }
        Ok(())
    }
}
fn document(q: &BackingQuota) -> AccountedDocument {
    let f = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let default: Vec<u8> = [0.25f64, 0., 0.5, 1.].into_iter().flat_map(f64::to_ne_bytes).collect();
    let descriptor = NativePlaneDescriptor::new(f, Rect::new(0, 0, 3, 2), &default).unwrap();
    let store = Arc::new(Store(descriptor));
    let backed = BackedRaster::new(store, q.clone(), 1, &control()).unwrap();
    let mut d = Document::new("accounted selection draft", Size::new(3, 2), ColorMode::Rgb, SampleType::F64);
    d.layers.push(Layer::new("layer", LayerContent::BackedRaster(BackedLayer::new(backed, descriptor, &control()).unwrap())));
    let guard = Arc::new(q.reserve(QuotaKind::Resident, 16384).unwrap());
    AccountedDocument::from_owned(d, guard, q.clone(), &control()).unwrap()
}
#[test]
fn native_words_default_and_guarded_tile_identity_are_exact() {
    let q = quota();
    let s = selection(&q);
    let before = q.usage().unwrap();
    let view = s.clone_view(&control()).unwrap();
    assert!(q.usage().unwrap().resident > before.resident);
    assert!(s.matches(&view.surface, &q, &control()).unwrap());
    assert_eq!(view.surface.default_bytes(), &(-0.0f64).to_ne_bytes());
    let raw = view.surface.try_read_interleaved(Rect::new(0, 0, 3, 2)).unwrap();
    assert_eq!(&raw[..8], &(-0.0f64).to_ne_bytes());
    let mut forged = view.surface.clone();
    forged.try_write_interleaved(Rect::new(0, 0, 1, 1), &(-0.0f64).to_ne_bytes()).unwrap();
    assert!(!s.matches(&forged, &q, &control()).unwrap());
    drop(forged);
    drop(view);
    assert_eq!(q.usage().unwrap(), before);
}
#[test]
fn invalid_default_or_late_native_float_never_allocates_selection_payload() {
    let q = quota();
    let before = q.usage().unwrap();
    let f = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    for (v, default) in [(f64::NAN, 0.), (1.1, 0.), (0.5, f64::INFINITY)] {
        let raw: Vec<u8> = [0.25f64, v].into_iter().flat_map(f64::to_ne_bytes).collect();
        assert!(
            AccountedSelection::try_from_native(f, Rect::new(0, 0, 2, 1), &default.to_ne_bytes(), &raw, CoverageDomain::FiniteUnit, &q, &control()).is_err()
        );
        assert_eq!(q.usage().unwrap(), before);
    }
}
#[test]
fn actual_ledger_guard_depth_bounds_and_sourcefree_require_none_are_required() {
    let q = quota();
    let d = document(&q);
    let other = quota();
    let wrong = selection(&other);
    assert!(matches!(d.try_replace_selection(Some(&wrong), &control()), Err(BackingError::Integrity)));
    let f = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
    let s =
        AccountedSelection::try_from_native(f, Rect::new(0, 0, 1, 1), &[0; 4], &u32::MAX.to_ne_bytes(), CoverageDomain::FiniteUnit, &q, &control()).unwrap();
    assert!(matches!(d.try_replace_selection(Some(&s), &control()), Err(BackingError::Bounds)));
    let f = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let outside =
        AccountedSelection::try_from_native(f, Rect::new(-1, 0, 0, 1), &[0; 8], &0.5f64.to_ne_bytes(), CoverageDomain::FiniteUnit, &q, &control()).unwrap();
    assert!(matches!(d.try_replace_selection(Some(&outside), &control()), Err(BackingError::Bounds)));
}
#[test]
fn typed_layer_clone_retains_payload_and_each_map_default_view_is_paid() {
    let q = quota();
    let d = document(&q);
    let s = selection(&q);
    let selected = d.try_replace_selection(Some(&s), &control()).unwrap();
    selected.validate_selection_replacement(&control()).unwrap();
    let original_tile = selected.document().selection.as_ref().unwrap().tiles().next().unwrap().1.clone();
    drop(s);
    let before = q.usage().unwrap();
    let layer = selected.document().layers[0].id;
    let edited = selected.try_edit(AccountedEdit::LayerVisibility { layer, visible: false }, &control()).unwrap();
    assert!(q.usage().unwrap().resident > before.resident);
    let copied_tile = edited.document().selection.as_ref().unwrap().tiles().next().unwrap().1;
    assert!(Arc::ptr_eq(&original_tile, copied_tile));
    let clear = edited.try_replace_selection(None, &control()).unwrap();
    assert!(clear.document().selection.is_none());
    assert!(selected.document().selection.is_some());
    drop(original_tile);
}

#[test]
fn upstream_gray8_selection_on_wide_document_survives_typed_edits() {
    let q = quota();
    let d = document(&q);
    let s = AccountedSelection::try_from_native(
        PixelFormat::GRAY8,
        Rect::new(0, 0, 3, 2),
        &[0],
        &[0, 64, 128, 255, 192, 32],
        CoverageDomain::FiniteUnit,
        &q,
        &control(),
    )
    .unwrap();
    let selected = d.try_replace_selection(Some(&s), &control()).unwrap();
    assert_eq!(selected.document().depth, SampleType::F64);
    let layer = selected.document().layers[0].id;
    let edited = selected.try_edit(AccountedEdit::LayerVisibility { layer, visible: false }, &control()).unwrap();
    let coverage = edited.document().selection.as_ref().unwrap();
    assert_eq!(coverage.format(), PixelFormat::GRAY8);
    assert_eq!(coverage.try_read_interleaved(Rect::new(0, 0, 3, 2)).unwrap(), [0, 64, 128, 255, 192, 32]);
    let no_op = edited.try_replace_selection(Some(&s), &control()).unwrap();
    assert!(edited.shares_model(&no_op));
}
#[test]
fn history_noop_preserves_redo_and_full_slot_refuses_before_staging() {
    let q = quota();
    let d = document(&q);
    let s = selection(&q);
    let mut h = AccountedHistory::new(d, 3, &control()).unwrap();
    h.try_replace_selection(Some(&s), &control()).unwrap();
    h.try_replace_selection(None, &control()).unwrap();
    assert!(h.undo(&control()).unwrap());
    let before = q.usage().unwrap();
    let current = h.current().unwrap().clone();
    h.try_replace_selection(Some(&s), &control()).unwrap();
    assert!(h.can_redo());
    assert!(current.shares_model(h.current().unwrap()));
    assert_eq!(q.usage().unwrap(), before);
    let mut full = AccountedHistory::new(current, 1, &control()).unwrap();
    let before = q.usage().unwrap();
    assert!(matches!(full.try_replace_selection(None, &control()), Err(BackingError::Quota)));
    assert_eq!(q.usage().unwrap(), before);
}
#[test]
fn work_failure_and_cancellation_return_all_temporary_charges() {
    let q = quota();
    let before = q.usage().unwrap();
    let f = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let cancelled = Interrupt::cancel_only(&|| true);
    let c = ReadControl { interrupt: &cancelled, work: &|_| Ok(()) };
    assert!(matches!(
        AccountedSelection::try_from_native(f, Rect::new(0, 0, 1, 1), &[0; 8], &0.5f64.to_ne_bytes(), CoverageDomain::FiniteUnit, &q, &c),
        Err(BackingError::Cancelled)
    ));
    assert_eq!(q.usage().unwrap(), before);
    let work = |units| if units > 32 { Err(BackingError::Work) } else { Ok(()) };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(AccountedSelection::try_from_native(f, Rect::new(0, 0, 1, 1), &[0; 8], &0.5f64.to_ne_bytes(), CoverageDomain::FiniteUnit, &q, &c).is_err());
    assert_eq!(q.usage().unwrap(), before);
}

#[test]
fn readonly_prepaid_codec_adoption_with_selection_does_not_grant_typed_edits() {
    let q = quota();
    let initial = document(&q);
    let selection = selection(&q);
    let view = selection.clone_view(&control()).unwrap();
    let model_lease = q.reserve(QuotaKind::Resident, 32768).unwrap();
    let mut raw = initial.document().clone();
    raw.selection = Some(view.surface);
    // This externally prepaid codec contract retains actual native/view/model
    // owners. It is deliberately opaque to the internal selection proof path.
    let retention = Arc::new((initial, selection, view.charges, model_lease));
    let adopted = AccountedDocument::from_owned(raw, retention, q.clone(), &control()).unwrap();
    assert!(adopted.document().selection.is_some());
    let history = AccountedHistory::new(adopted.clone(), 3, &control()).unwrap();
    assert!(adopted.shares_model(history.current().unwrap()));
    let layer = adopted.document().layers[0].id;
    assert!(adopted.try_edit(AccountedEdit::LayerVisibility { layer, visible: false }, &control()).is_err());
    assert!(adopted.try_replace_selection(None, &control()).is_err());
    assert!(adopted.validate_selection_replacement(&control()).is_err());
}
