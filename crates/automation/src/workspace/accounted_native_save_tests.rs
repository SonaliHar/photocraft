//! UNRUN held-root accounted native save regression drafts.
use super::*;
use photocraft_doc::{BackedLayer, ColorMode, Document, Layer, LayerContent, PixelFormat, SampleType, accounted_document::AccountedDocument};
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
fn owned_native_save_streams_within_root_and_preserves_snapshot() {
    let root = Root::new();
    let workspace = AuthorizedWorkspace::new(Some(&root.0), Some(&root.0)).unwrap();
    for depth in [SampleType::U32, SampleType::F64] {
        let snapshot = fixture(depth);
        let original = snapshot.clone();
        let name = format!("{depth:?}.pcraft");
        workspace.save_accounted_native_with_read_control(&snapshot, &name, &Default::default(), Default::default(), &ctl()).unwrap();
        assert!(std::fs::metadata(root.0.join(name)).unwrap().len() > 0);
        assert!(snapshot.shares_state(&original));
        assert_eq!(snapshot.name, "unchanged current");
    }
}
#[test]
fn alias_fingerprint_wrapper_and_escape_preserve_existing_bytes() {
    let root = Root::new();
    let workspace = AuthorizedWorkspace::new(Some(&root.0), Some(&root.0)).unwrap();
    let snapshot = fixture(SampleType::U32);
    let source = root.0.join("source.xcf");
    let alias = root.0.join("alias.pcraft");
    let raw = b"protected original";
    std::fs::write(&source, raw).unwrap();
    std::fs::hard_link(&source, &alias).unwrap();
    let protected = WriteProtection { sources: vec!["source.xcf".into()], fingerprints: vec![] };
    assert!(workspace.save_accounted_native_with_read_control(&snapshot, "alias.pcraft", &protected, Default::default(), &ctl()).is_err());
    assert_eq!(std::fs::read(&source).unwrap(), raw);
    std::fs::write(root.0.join("fingerprint.pcraft"), raw).unwrap();
    let fingerprint = WriteProtection { sources: vec![], fingerprints: vec![blake3::hash(raw).to_hex().to_string()] };
    assert!(workspace.save_accounted_native_with_read_control(&snapshot, "fingerprint.pcraft", &fingerprint, Default::default(), &ctl()).is_err());
    assert_eq!(std::fs::read(root.0.join("fingerprint.pcraft")).unwrap(), raw);
    let wrapped = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0];
    std::fs::write(root.0.join("wrapped.pcraft"), wrapped).unwrap();
    assert!(workspace.save_accounted_native_with_read_control(&snapshot, "wrapped.pcraft", &Default::default(), Default::default(), &ctl()).is_err());
    assert_eq!(std::fs::read(root.0.join("wrapped.pcraft")).unwrap(), wrapped);
    assert!(workspace.save_accounted_native_with_read_control(&snapshot, "../escape.pcraft", &Default::default(), Default::default(), &ctl()).is_err());
}
#[test]
fn cancellation_and_missing_write_authority_do_not_create_output() {
    let root = Root::new();
    let workspace = AuthorizedWorkspace::new(Some(&root.0), Some(&root.0)).unwrap();
    let snapshot = fixture(SampleType::F64);
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(
        workspace.save_accounted_native_with_read_control(&snapshot, "cancel.pcraft", &Default::default(), Default::default(), &control),
        Err(AutomationError::Engine(photocraft_engine::EngineError::Cancelled))
    ));
    assert!(!root.0.join("cancel.pcraft").exists());
    let read_only = AuthorizedWorkspace::new(Some(&root.0), None).unwrap();
    assert!(read_only.save_accounted_native_with_read_control(&snapshot, "denied.pcraft", &Default::default(), Default::default(), &ctl()).is_err());
    assert!(!root.0.join("denied.pcraft").exists());
}

#[test]
fn late_cancel_cleans_only_owned_temp_and_keeps_destination() {
    use std::io::Write;
    use std::sync::atomic::AtomicBool;
    let root = Root::new();
    let workspace = AuthorizedWorkspace::new(Some(&root.0), Some(&root.0)).unwrap();
    std::fs::write(root.0.join("kept.pcraft"), b"existing").unwrap();
    let stop = AtomicBool::new(false);
    let cancelled = || stop.load(Ordering::Relaxed);
    let interrupt = Interrupt::cancel_only(&cancelled);
    let result = source_write::write_stream_with_resident_limit(
        &workspace,
        "kept.pcraft",
        source_write::Sources::many(&[]),
        &[],
        &interrupt,
        photocraft_io::source_protection::SOURCE_PROTECTION_FIXED_BYTES,
        |file| {
            file.write_all(b"private incomplete archive").unwrap();
            stop.store(true, Ordering::Relaxed);
            Ok(())
        },
    );
    assert!(matches!(result, Err(AutomationError::Engine(photocraft_engine::EngineError::Cancelled))));
    assert_eq!(std::fs::read(root.0.join("kept.pcraft")).unwrap(), b"existing");
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 1);
}
