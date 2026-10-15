//! UNRUN shared trusted-writer extraction regressions; no source qualification claims.
use super::*;
use photocraft_doc::{BackedLayer, ColorMode, Document, Layer, LayerContent, PixelFormat, SampleType, accounted_document::AccountedDocument};
use photocraft_engine::DocumentSnapshot;
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{quota::*, *},
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{path::PathBuf, sync::Arc};
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-accounted-service-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[test]
fn interactive_native_capability_streams_owned_words_without_mutating_snapshot() {
    let root = Root::new();
    for depth in [SampleType::U32, SampleType::F64] {
        let snapshot = fixture(depth);
        let original = snapshot.clone();
        let path = root.0.join(format!("{depth:?}.pcraft"));
        save_accounted_native_selected_path(&snapshot, path.to_str().unwrap(), &Default::default(), &ctl()).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
        assert!(snapshot.shares_state(&original));
        assert_eq!(snapshot.name, "unchanged current");
    }
}
#[test]
fn all_open_source_alias_and_fingerprint_exclusions_precede_output_replacement() {
    let root = Root::new();
    let snapshot = fixture(SampleType::U32);
    let source = root.0.join("inactive-source.xcf");
    let alias = root.0.join("alias.pcraft");
    let raw = b"gimp xcf own protected bytes";
    std::fs::write(&source, raw).unwrap();
    std::fs::hard_link(&source, &alias).unwrap();
    let protection = photocraft_engine::WriteProtection { sources: vec![source.to_str().unwrap().into()], fingerprints: Vec::new() };
    assert!(save_accounted_native_selected_path(&snapshot, alias.to_str().unwrap(), &protection, &ctl()).is_err());
    assert_eq!(std::fs::read(&source).unwrap(), raw);
    assert_eq!(std::fs::read(&alias).unwrap(), raw);
    let empty = root.0.join("retained-fingerprint.pcraft");
    std::fs::write(&empty, []).unwrap();
    let protection = photocraft_engine::WriteProtection {
        sources: Vec::new(),
        fingerprints: vec!["af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262".into()],
    };
    assert!(save_accounted_native_selected_path(&snapshot, empty.to_str().unwrap(), &protection, &ctl()).is_err());
    assert_eq!(std::fs::metadata(&empty).unwrap().len(), 0);
}
#[test]
fn wrapper_and_precancel_refuse_without_creating_or_mutating_destination() {
    let root = Root::new();
    let snapshot = fixture(SampleType::F64);
    let path = root.0.join("wrapped.pcraft");
    let raw = b"\x1f\x8bnot a writable native archive";
    std::fs::write(&path, raw).unwrap();
    assert!(save_accounted_native_selected_path(&snapshot, path.to_str().unwrap(), &Default::default(), &ctl()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), raw);
    let absent = root.0.join("cancelled.pcraft");
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(save_accounted_native_selected_path(&snapshot, absent.to_str().unwrap(), &Default::default(), &control).is_err());
    assert!(!absent.exists());
}

#[test]
fn supplied_work_and_actual_snapshot_quota_refuse_before_destination_creation() {
    let root = Root::new();
    let snapshot = fixture(SampleType::F64);
    let original = snapshot.clone();
    let quota = snapshot.accounted().unwrap().quota();
    let baseline = quota.usage().unwrap();
    let path = root.0.join("no-headroom.pcraft");
    let blocker = quota.reserve(QuotaKind::Resident, (128 << 20) - baseline.resident).unwrap();
    assert!(save_accounted_native_selected_path(&snapshot, path.to_str().unwrap(), &Default::default(), &ctl()).is_err());
    assert!(!path.exists());
    drop(blocker);
    assert_eq!(quota.usage().unwrap(), baseline);
    let units = AtomicU64::new(0);
    let denied = |n| {
        units.fetch_add(n, Ordering::Relaxed);
        Err(BackingError::Work)
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &denied };
    assert!(save_accounted_native_selected_path(&snapshot, path.to_str().unwrap(), &Default::default(), &control).is_err());
    assert!(units.load(Ordering::Relaxed) > 0);
    assert!(!path.exists());
    assert_eq!(quota.usage().unwrap(), baseline);
    assert!(snapshot.shares_state(&original));
}
