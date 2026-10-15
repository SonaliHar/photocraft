//! UNRUN held-root owned native read regression drafts.
use super::*;
use photocraft_doc::{BackedLayer, ColorMode, Document, Layer, LayerContent, PixelFormat, SampleType, accounted_document::AccountedDocument};
use photocraft_engine::DocumentSnapshot;
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{quota::*, *},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-accounted-rooted-open-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Defaults {
    descriptor: NativePlaneDescriptor,
    _lease: QuotaLease,
}
impl NativePlaneStore for Defaults {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        for (i, v) in out.iter_mut().enumerate() {
            *v = self.descriptor.default_bytes()[((offset + i as u64) % self.descriptor.default_bytes().len() as u64) as usize];
        }
        Ok(())
    }
}
fn fixture(depth: SampleType) -> DocumentSnapshot {
    let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 16 << 20, scratch: 128 << 20 });
    let format = PixelFormat::new(ColorMode::Rgb, depth, true);
    let default = if depth == SampleType::U32 { 0x01000001u32.to_ne_bytes().repeat(4) } else { (-0.0f64).to_ne_bytes().repeat(4) };
    let descriptor = NativePlaneDescriptor::new(format, Rect::new(0, 0, 2, 1), &default).unwrap();
    let store = Arc::new(Defaults { descriptor, _lease: q.reserve(QuotaKind::Resident, 512).unwrap() });
    let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
    let layer = BackedLayer::new(raster, descriptor, &ctl()).unwrap();
    let mut doc = Document::new("unchanged current", Size::new(2, 1), ColorMode::Rgb, depth);
    doc.layers.push(Layer::new("native", LayerContent::BackedRaster(layer)));
    let guard = q.reserve(QuotaKind::Index, 65536).unwrap();
    DocumentSnapshot::from_accounted(AccountedDocument::from_owned(doc, Arc::new(guard), q, &ctl()).unwrap()).unwrap()
}

fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 16 << 20, scratch: 128 << 20 })
}
const SOURCE_CAP: u64 = 256 << 20;
#[test]
fn rooted_reader_preserves_owned_native_words_and_survives_source_removal() {
    let root = Root::new();
    let workspace = AuthorizedWorkspace::new(Some(&root.0), Some(&root.0)).unwrap();
    for depth in [SampleType::U32, SampleType::F64] {
        let original = fixture(depth);
        let name = format!("{depth:?}.pcraft");
        workspace.save_accounted_native_with_read_control(&original, &name, &Default::default(), Default::default(), &ctl()).unwrap();
        let q = quota();
        let baseline = q.usage().unwrap();
        let archive = workspace.open_accounted_native_with_read_control(&name, SOURCE_CAP, Default::default(), q.clone(), &ctl()).unwrap();
        std::fs::remove_file(root.0.join(&name)).unwrap();
        assert_eq!(archive.document().depth, depth);
        assert_eq!(archive.document().name, original.name);
        assert_eq!(archive.document().source_color_context, original.source_color_context);
        let LayerContent::BackedRaster(expected) = &original.layers[0].content else { panic!("fixture not backed") };
        let LayerContent::BackedRaster(actual) = &archive.document().layers[0].content else { panic!("archive not backed") };
        assert_eq!(actual.descriptor().format(), expected.descriptor().format());
        assert_eq!(actual.descriptor().default_bytes(), expected.descriptor().default_bytes());
        let owned = archive.into_accounted_document(&ctl()).unwrap();
        assert!(owned.quota().shares_ledger(&q));
        assert!(q.usage().unwrap().resident > baseline.resident);
        drop(owned);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
#[test]
fn root_authority_path_class_and_source_cap_refusals_refund_bookkeeping() {
    let root = Root::new();
    let workspace = AuthorizedWorkspace::new(Some(&root.0), None).unwrap();
    std::fs::write(root.0.join("bytes.pcraft"), [0u8; 32]).unwrap();
    std::fs::create_dir(root.0.join("directory.pcraft")).unwrap();
    for (path, cap) in
        [("../outside.pcraft", SOURCE_CAP), ("bytes.pcraft", 31), ("directory.pcraft", SOURCE_CAP), ("bytes.pcraft", SOURCE_CAP + 1), ("bytes.pcraft", 0)]
    {
        let q = quota();
        let usage = q.usage().unwrap();
        assert!(workspace.open_accounted_native_with_read_control(path, cap, Default::default(), q.clone(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), usage);
    }
    let no_root = AuthorizedWorkspace::new(None, None).unwrap();
    let q = quota();
    assert!(no_root.open_accounted_native_with_read_control("bytes.pcraft", SOURCE_CAP, Default::default(), q.clone(), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn cancellation_and_actual_quota_refuse_before_capture() {
    let root = Root::new();
    let workspace = AuthorizedWorkspace::new(Some(&root.0), None).unwrap();
    let q = quota();
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(
        workspace.open_accounted_native_with_read_control("not-opened.pcraft", SOURCE_CAP, Default::default(), q.clone(), &c),
        Err(AutomationError::Engine(photocraft_engine::EngineError::Cancelled))
    ));
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let small = BackingQuota::new(QuotaLimits { resident: 65535, index: 65535, scratch: 0 });
    assert!(workspace.open_accounted_native_with_read_control("not-opened.pcraft", SOURCE_CAP, Default::default(), small.clone(), &ctl()).is_err());
    assert_eq!(small.usage().unwrap(), QuotaUsage::default());
}
#[cfg(unix)]
#[test]
fn a_symlink_cannot_expand_the_held_root() {
    let root = Root::new();
    let outside = Root::new();
    std::fs::write(outside.0.join("outside.pcraft"), [0u8; 32]).unwrap();
    std::os::unix::fs::symlink(outside.0.join("outside.pcraft"), root.0.join("link.pcraft")).unwrap();
    let workspace = AuthorizedWorkspace::new(Some(&root.0), None).unwrap();
    let q = quota();
    assert!(workspace.open_accounted_native_with_read_control("link.pcraft", SOURCE_CAP, Default::default(), q.clone(), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
