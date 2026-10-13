use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{BackedLayer, Document, Layer, LayerContent, LayerId, accounted_document::AccountedDocument};
use photocraft_engine::accounted_view::AccountedView;
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits, QuotaUsage},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    reads: Arc<AtomicUsize>,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, _: u64, _: &mut [u8], _: &ReadControl<'_>) -> Result<(), BackingError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Err(BackingError::Io)
    }
}
fn owner() -> (AccountedDocument, BackingQuota, Arc<AtomicUsize>) {
    let q = BackingQuota::new(QuotaLimits { resident: 32 << 20, scratch: 0, index: 1 << 20 });
    let retained_model = q.reserve(QuotaKind::Resident, 65536).unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let descriptor =
        NativePlaneDescriptor::new(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), Rect::new(0, 0, 2, 2), &0f64.to_ne_bytes().repeat(4)).unwrap();
    let raster = BackedRaster::new(Arc::new(Store { descriptor, reads: reads.clone() }), q.clone(), 1, &control()).unwrap();
    let backed = BackedLayer::new(raster, descriptor, &control()).unwrap();
    let mut doc = Document::new("view", Size::new(2, 2), ColorMode::Rgb, SampleType::F64);
    doc.layers.push(Layer::new("one", LayerContent::BackedRaster(backed.clone())));
    doc.layers.push(Layer::new("two", LayerContent::BackedRaster(backed)));
    (AccountedDocument::from_owned(doc, Arc::new(retained_model), q.clone(), &control()).unwrap(), q, reads)
}
#[test]
fn clones_share_selection_and_paid_changes_preserve_old_history_target() {
    let (doc, q, reads) = owner();
    let ids: Vec<_> = doc.document().layers.iter().map(|l| l.id).collect();
    let baseline = q.usage().unwrap();
    let original = AccountedView::new(&doc, Some(ids[0]), &ids, Some(ids[1]), &control()).unwrap();
    let retained = q.usage().unwrap();
    assert!(retained.index > baseline.index);
    let history = original.clone();
    assert!(history.shares_state(&original));
    assert_eq!(q.usage().unwrap(), retained);
    let current = original.try_replace(&doc, Some(ids[1]), &ids[1..], Some(ids[1]), &control()).unwrap();
    assert!(!current.shares_state(&original));
    assert_eq!(history.selected(), ids);
    assert_eq!(history.target().active, Some(ids[0]));
    assert_eq!(current.selected(), &ids[1..]);
    drop(current);
    drop(original);
    assert_eq!(q.usage().unwrap(), retained);
    drop(history);
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    drop(doc);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn quota_failure_and_noop_never_publish_or_detach_selection() {
    let (doc, q, _) = owner();
    let ids: Vec<_> = doc.document().layers.iter().map(|l| l.id).collect();
    let original = AccountedView::new(&doc, Some(ids[0]), &ids, Some(ids[0]), &control()).unwrap();
    let fill = q.reserve(QuotaKind::Index, (1 << 20) - q.usage().unwrap().index).unwrap();
    let full = q.usage().unwrap();
    assert!(matches!(original.try_replace(&doc, Some(ids[1]), &ids[1..], Some(ids[1]), &control()), Err(BackingError::Quota)));
    assert_eq!(q.usage().unwrap(), full);
    let noop = original.try_replace(&doc, Some(ids[0]), &ids, Some(ids[0]), &control()).unwrap();
    assert!(noop.shares_state(&original));
    assert_eq!(q.usage().unwrap(), full);
    assert_eq!(original.selected(), ids);
    drop(fill);
    drop(noop);
    drop(original);
    drop(doc);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn cancellation_at_validation_reserve_copy_and_publication_refunds() {
    let (doc, q, reads) = owner();
    let ids: Vec<_> = doc.document().layers.iter().map(|l| l.id).collect();
    let before = q.usage().unwrap();
    for phase in 1..=5 {
        let calls = AtomicUsize::new(0);
        let work = |_| if calls.fetch_add(1, Ordering::Relaxed) + 1 == phase { Err(BackingError::Cancelled) } else { Ok(()) };
        let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        assert!(matches!(AccountedView::new(&doc, Some(ids[0]), &ids, Some(ids[1]), &ctl), Err(BackingError::Cancelled)), "phase {phase}");
        assert_eq!(q.usage().unwrap(), before);
    }
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    drop(doc);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn malformed_selection_and_foreign_ledger_fail_before_charge_or_pixel_reads() {
    let (doc, q, reads) = owner();
    let ids: Vec<_> = doc.document().layers.iter().map(|l| l.id).collect();
    let before = q.usage().unwrap();
    for (active, selected, anchor) in [
        (Some(ids[0]), vec![ids[0], ids[0]], None),
        (Some(ids[0]), vec![ids[0], LayerId(u64::MAX)], None),
        (Some(ids[1]), vec![ids[0]], None),
        (Some(ids[0]), vec![ids[0]], Some(ids[1])),
        (None, vec![ids[0]], None),
    ] {
        assert!(AccountedView::new(&doc, active, &selected, anchor, &control()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    assert!(matches!(AccountedView::new(&doc, Some(ids[0]), &vec![ids[0]; 4097], None, &control()), Err(BackingError::Length)));
    let view = AccountedView::new(&doc, Some(ids[0]), &ids, None, &control()).unwrap();
    let (foreign, _, _) = owner();
    assert!(matches!(view.try_replace(&foreign, Some(ids[0]), &ids, None, &control()), Err(BackingError::Unsupported)));
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    drop(view);
    drop(doc);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
