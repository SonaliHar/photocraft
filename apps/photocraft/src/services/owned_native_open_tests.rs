//! UNRUN explicit desktop held-reader/ledger lifecycle drafts.
use super::*;
use photocraft_doc::{BackedLayer, ColorMode, Document, Layer, LayerContent, PixelFormat, SampleType, Size};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, quota::*},
};
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}

#[test]
fn rooted_callback_exists_only_with_held_read_root_and_never_uses_ambient_path() {
    let dir = Directory::new();
    archive(&dir.0.join("selected.pcraft"));
    assert!(super::super::native(None).automation_owned_native_open.is_none());
    let write_only = photocraft_automation::AuthorizedWorkspace::new(None, Some(&dir.0)).unwrap();
    assert!(super::super::native(Some(write_only)).automation_owned_native_open.is_none());
    let workspace = photocraft_automation::AuthorizedWorkspace::new(Some(&dir.0), None).unwrap();
    let mut callback = super::super::native(Some(workspace)).automation_owned_native_open.unwrap();
    let quota = photocraft_io::accounted_native::new_accounted_native_open_quota(&ctl()).unwrap();
    let opened = callback("selected.pcraft", Some(&quota), &ctl()).unwrap().into_accounted_document(&ctl()).unwrap();
    assert!(opened.quota().shares_ledger(&quota));
    assert!(callback("../escape.pcraft", Some(&quota), &ctl()).is_err());
}
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-owned-native-open-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[derive(Debug)]
struct Defaults(NativePlaneDescriptor);
impl NativePlaneStore for Defaults {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(output.len() as u64)?;
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = self.0.default_bytes()[((offset + index as u64) % self.0.default_bytes().len() as u64) as usize];
        }
        Ok(())
    }
}
fn archive(path: &Path) {
    let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, scratch: 128 << 20, index: 16 << 20 });
    let _model = q.reserve(QuotaKind::Resident, 8192).unwrap();
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let default = (-0.0f64).to_ne_bytes().repeat(4);
    let descriptor = NativePlaneDescriptor::new(format, Rect::new(0, 0, 2, 1), &default).unwrap();
    let raster = BackedRaster::new(Arc::new(Defaults(descriptor)), q.clone(), 1, &ctl()).unwrap();
    let mut doc = Document::new("retained", Size::new(2, 1), ColorMode::Rgb, SampleType::F64);
    doc.layers.push(Layer::new("native", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap())));
    photocraft_format::owned_document::save_current_document_archive(&doc, File::create(path).unwrap(), Default::default(), &q, &ctl()).unwrap();
}

#[test]
fn selected_regular_archive_reuses_supplied_ledger_and_preserves_native_default() {
    let dir = Directory::new();
    let path = dir.0.join("selected.pcraft");
    archive(&path);
    let q = photocraft_io::accounted_native::new_accounted_native_open_quota(&ctl()).unwrap();
    let opened = open(path.to_str().unwrap(), Some(&q), &ctl()).unwrap().into_accounted_document(&ctl()).unwrap();
    assert!(opened.quota().shares_ledger(&q));
    assert_eq!(opened.document().name, "retained");
    let LayerContent::BackedRaster(layer) = &opened.document().layers[0].content else { panic!("native owner missing") };
    assert_eq!(layer.default_bytes(), (-0.0f64).to_ne_bytes().repeat(4));
    assert_eq!(layer.format().sample, SampleType::F64);
    let fresh = open(path.to_str().unwrap(), None, &ctl()).unwrap().into_accounted_document(&ctl()).unwrap();
    assert!(!fresh.quota().shares_ledger(&q));
}

#[test]
fn zero_headroom_and_cancel_refuse_before_missing_source_open_and_directory_refuses() {
    let dir = Directory::new();
    let missing = dir.0.join("missing.pcraft");
    let zero = BackingQuota::new(QuotaLimits::default());
    let error = open(missing.to_str().unwrap(), Some(&zero), &ctl()).unwrap_err();
    assert!(error.contains("quota"));
    let cancelled = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Cancelled) };
    assert!(open(missing.to_str().unwrap(), None, &cancelled).unwrap_err().contains("cancelled"));
    assert!(open(dir.0.to_str().unwrap(), None, &ctl()).is_err());
    assert!(!missing.exists());
}
