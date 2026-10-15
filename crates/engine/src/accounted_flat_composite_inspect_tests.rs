//! UNRUN genuine same-ledger synthetic CURRENT metadata, not source or render
//! qualification. This tests the metadata factory without publishing a fake
//! renderer-qualified Session. Store reads must remain zero.
use super::*;
use photocraft_doc::{
    BackedLayer, ColorMode, Document, Layer, LayerContent, PixelFormat, SampleType, Size, accounted_document::AccountedDocument, import_compositing::*,
    item_identity::ItemRef, xcf_state::*,
};
use photocraft_raster::{
    Interrupt,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    reads: Arc<AtomicU64>,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, bytes: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        c.check(bytes.len() as u64)?;
        if offset.checked_add(bytes.len() as u64).ok_or(BackingError::Length)? > self.descriptor.logical_bytes() {
            return Err(BackingError::Bounds);
        }
        self.reads.fetch_add(1, Ordering::Relaxed);
        bytes.fill(0);
        Ok(())
    }
}
fn fixture(count: usize, selected: usize, long: bool) -> (AccountedDocument, Arc<AtomicU64>) {
    assert!((1..=4096).contains(&count) && selected <= count);
    let q = BackingQuota::new(QuotaLimits { resident: 1 << 30, index: 512 << 20, scratch: 0 });
    // Real prospective model/string/native-store header charge before all build
    // allocations. The constructor below adds only its actual owner header.
    let metadata = q.reserve(QuotaKind::Index, (count as u64) * 32768 + (1 << 20)).unwrap();
    let reads = Arc::new(AtomicU64::new(0));
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let bounds = photocraft_geom::Rect::new(-3, -2, -2, -1);
    let descriptor = NativePlaneDescriptor::new(format, bounds, &[0; 16]).unwrap();
    let raster = BackedRaster::new(Arc::new(Store { descriptor, reads: reads.clone() }), q.clone(), 2, &ctl()).unwrap();
    let backed = BackedLayer::new(raster, descriptor, &ctl()).unwrap();
    let mut doc = Document::new("metadata-only", Size::new(37, 29), ColorMode::Rgb, SampleType::F32);
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    for index in 0..count {
        let name = if long && index == count - 1 { "λ".repeat(40000) } else { format!("row-{index}") };
        let layer = Layer::new(name, LayerContent::BackedRaster(backed.clone()));
        state.items.push(ItemState {
            item: ItemRef::Layer(layer.id),
            name: layer.name.clone(),
            custom_name: false,
            visible: layer.visible,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(bounds),
            mask: None,
            composition: Some(GimpComposition {
                compatibility_epoch: if long && index == count - 1 { "é".repeat(40000) } else { "current-inert".into() },
                source_mode: 47,
                legacy: false,
                stored_blend_space: -4,
                stored_composite_space: -1,
                stored_operator: -2,
                resolved: Some(ResolvedGimpComposition {
                    blend_space: GimpSpace::LinearRgb,
                    composite_space: GimpSpace::LinearRgb,
                    operator: GimpOperator::ClipToBackdrop,
                }),
                pass_through: false,
            }),
        });
        if index < selected {
            state.selection.selected.push(ItemRef::Layer(layer.id));
        }
        doc.layers.push(layer);
    }
    state.selection.active = state.selection.selected.first().copied();
    doc.xcf_state = Some(state);
    let actual = photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert!(actual.retained_bytes() as u64 <= metadata.bytes());
    let owner = AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new(metadata), q, &ctl()).unwrap();
    (owner, reads)
}
#[test]
#[ignore = "UNRUN: actual paid1/2/7/129/4096row metadata independent bounded item+selection pages and empty/default semantics; no native reads"]
fn bounded_pages_and_selected_totals() {
    for count in [1, 2, 7, 129, 4096] {
        let (owner, reads) = fixture(count, count, false);
        for (offset, limit) in [(0, 64), (1, 1), (64, 128), (count.saturating_sub(1), 128), (100000, 128)] {
            let reply = prepare_flat_composite(&owner, offset, limit, &ctl()).unwrap();
            let value = reply.value();
            let n = count.saturating_sub(offset).min(limit);
            assert_eq!(value["items"]["total"], count);
            assert_eq!(value["items"]["offset"], offset);
            assert_eq!(value["items"]["limit"], limit);
            assert_eq!(value["items"]["entries"].as_array().unwrap().len(), n);
            assert_eq!(value["selection"]["selected"]["total"], count);
            assert_eq!(value["selection"]["selected"]["entries"].as_array().unwrap().len(), n);
            assert_eq!(value["filters"]["total"], 0);
            assert!(reply.quota().shares_ledger(owner.quota()));
            assert!(reply.charged_index_bytes() > 0);
        }
        assert_eq!(reads.load(Ordering::Relaxed), 0);
    }
    for selected in [0, 1, 128] {
        let (owner, _) = fixture(129, selected, false);
        let reply = prepare_flat_composite(&owner, 64, 128, &ctl()).unwrap();
        assert_eq!(reply.value()["items"]["entries"].as_array().unwrap().len(), 65);
        assert_eq!(reply.value()["selection"]["selected"]["entries"].as_array().unwrap().len(), selected.saturating_sub(64));
    }
    assert_eq!(pagination(&Value::Null, &ctl()).unwrap(), (0, 64));
    assert_eq!(pagination(&serde_json::json!({}), &ctl()).unwrap(), (0, 64));
}
#[test]
#[ignore = "UNRUN: paid page-only Facts actual capacity, conservative full offpage UTF8 debit and complete paged truncation scans"]
fn facts_are_paged_but_full_canonical_text_is_charged() {
    let (owner, reads) = fixture(129, 129, true);
    let doc = owner.document();
    let state = doc.xcf_state.as_ref().unwrap();
    let resolution = photocraft_doc::PhysicalResolution::square(f64::from(doc.resolution_dpi));
    let p = plan(state, &resolution, 0, 1, &ctl()).unwrap();
    let facts = Facts::new(state, &p, 0, 1, owner.quota(), &ctl()).unwrap();
    assert_eq!(facts.items.len(), 1);
    assert!(facts.items.capacity() <= 2);
    drop(facts);
    let spent = AtomicU64::new(0);
    let work = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let offpage = prepare_flat_composite(&owner, 0, 1, &c).unwrap();
    assert!(spent.load(Ordering::Relaxed) > 160000);
    assert_eq!(offpage.value()["items"]["entries"].as_array().unwrap().len(), 1);
    drop(offpage);
    let reply = prepare_flat_composite(&owner, 128, 1, &ctl()).unwrap();
    let row = &reply.value()["items"]["entries"][0];
    assert_eq!(row["name"].as_str().unwrap().chars().count(), 256);
    assert_eq!(row["nameTruncated"], true);
    assert_eq!(row["composition"]["epochTruncated"], true);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}
#[test]
#[ignore = "UNRUN: invalid pagination, finite cumulative work and exhausted real ledger preserve owner and refund temporary query allocations"]
fn query_failures_and_custody() {
    let (owner, reads) = fixture(7, 7, false);
    let baseline = owner.quota().usage().unwrap();
    for p in [
        serde_json::json!({"limit":0}),
        serde_json::json!({"limit":129}),
        serde_json::json!({"offset":100001}),
        serde_json::json!({"offset":-1}),
        serde_json::json!({"limit":1.5}),
        serde_json::json!({"cursor":"new"}),
    ] {
        assert!(pagination(&p, &ctl()).is_err());
    }
    assert!(prepare_flat_composite(&owner, 100001, 1, &ctl()).is_err());
    assert!(prepare_flat_composite(&owner, 0, 0, &ctl()).is_err());
    let work = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(prepare_flat_composite(&owner, 0, 1, &c).is_err());
    assert_eq!(owner.quota().usage().unwrap(), baseline);
    let hold = owner.quota().reserve(QuotaKind::Index, (512u64 << 20) - baseline.index).unwrap();
    assert!(prepare_flat_composite(&owner, 0, 1, &ctl()).is_err());
    drop(hold);
    assert_eq!(owner.quota().usage().unwrap(), baseline);
    let reply = prepare_flat_composite(&owner, 0, 1, &ctl()).unwrap();
    assert!(owner.quota().usage().unwrap().index > baseline.index);
    drop(reply);
    assert_eq!(owner.quota().usage().unwrap(), baseline);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}
