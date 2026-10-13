#![cfg(windows)]
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{BackedLayer, Document, Layer, LayerContent};
use photocraft_format::owned_document::OwnedDocumentLimits;
use photocraft_geom::{Rect, Size};
use photocraft_io::{
    accounted_native::{ServiceAuthorizedNativeReadPath, ServiceAuthorizedNativeWritePath, open_accounted_native, save_accounted_native_path},
    xcf_source::ServiceAuthorizedScratchDirectory,
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use photocraft_xcf::{Budget, ResourceLimits, ResourcePolicy};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-accounted-native-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 64 << 20, index: 8 << 20 })
}
#[derive(Debug)]
struct Defaults(NativePlaneDescriptor);
impl NativePlaneStore for Defaults {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(out.len() as u64)?;
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.0.default_bytes()[((offset + i as u64) % self.0.default_bytes().len() as u64) as usize];
        }
        Ok(())
    }
}
#[test]
fn actual_owned_native_path_roundtrip_current_bits_source_independence_and_cleanup() {
    for sample in [SampleType::U32, SampleType::F64] {
        let root = Root::new();
        let scratch = root.0.join("scratch");
        std::fs::create_dir(&scratch).unwrap();
        let path = root.0.join("working.pcraft");
        let q = quota();
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        let default = if sample == SampleType::U32 { 0x01000001u32.to_ne_bytes().repeat(4) } else { (-0.0f64).to_ne_bytes().repeat(4) };
        let d = NativePlaneDescriptor::new(format, Rect::new(-2, 3, 2, 6), &default).unwrap();
        let layer = BackedLayer::new(BackedRaster::new(Arc::new(Defaults(d)), q.clone(), 2, &control()).unwrap(), d, &control()).unwrap();
        let edit = if sample == SampleType::U32 { 0x01000003u32.to_ne_bytes().repeat(4) } else { f64::from_bits(0x3fd0000000000001).to_ne_bytes().repeat(4) };
        let edited = layer.try_edited_native(Rect::new(-1, 4, 0, 5), &edit, &control()).unwrap();
        drop(layer);
        let layer = edited;
        let mut doc = Document::new("current owned", Size::new(8, 8), ColorMode::Rgb, sample);
        doc.layers.push(Layer::new("native", LayerContent::BackedRaster(layer)));
        let old = doc.clone();
        let write = ServiceAuthorizedNativeWritePath::from_platform_authorized(path.clone(), &q, &control()).unwrap();
        let gates = AtomicUsize::new(0);
        save_accounted_native_path(&doc, &write, OwnedDocumentLimits::default(), &q, &control(), |target, ctl| {
            ctl.check(0)?;
            assert_eq!(target, path.as_path());
            gates.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        assert!(gates.load(Ordering::SeqCst) >= 2);
        let read = ServiceAuthorizedNativeReadPath::from_platform_authorized(path.clone(), &q, &control()).unwrap();
        let loaded = {
            let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
            let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(scratch.clone(), &budget).unwrap();
            open_accounted_native(&read, &service, OwnedDocumentLimits::default(), q.clone(), &control()).unwrap()
        };
        drop(read);
        drop(write);
        std::fs::remove_file(&path).unwrap();
        drop(old);
        drop(doc);
        let LayerContent::BackedRaster(current) = &loaded.document().layers[0].content else { panic!("backed current erased") };
        let mut bytes = vec![0; format.bytes_per_pixel()];
        current.checked_view().read_logical_pixel_native_into(-1, 4, &mut bytes, &control()).unwrap();
        assert_eq!(bytes, edit);
        current.checked_view().read_logical_pixel_native_into(-2, 3, &mut bytes, &control()).unwrap();
        assert_eq!(bytes, default);
        assert!(q.usage().unwrap().scratch > 0);
        let owner = loaded.into_accounted_document(&control()).unwrap();
        let history = owner.clone();
        assert!(owner.shares_model(&history));
        let id = owner.document().layers[0].id;
        let changed = owner
            .try_edit(photocraft_doc::accounted_document::AccountedEdit::NativePixels { layer: id, rect: Rect::new(-2, 3, -1, 4), bytes: &edit }, &control())
            .unwrap();
        assert!(!changed.shares_model(&history));
        let LayerContent::BackedRaster(previous) = &history.document().layers[0].content else { panic!("history erased") };
        previous.checked_view().read_logical_pixel_native_into(-2, 3, &mut bytes, &control()).unwrap();
        assert_eq!(bytes, default);
        let LayerContent::BackedRaster(current) = &changed.document().layers[0].content else { panic!("edit erased") };
        current.checked_view().read_logical_pixel_native_into(-2, 3, &mut bytes, &control()).unwrap();
        assert_eq!(bytes, edit);
        let resave = ServiceAuthorizedNativeWritePath::from_platform_authorized(path.clone(), &q, &control()).unwrap();
        save_accounted_native_path(changed.document(), &resave, OwnedDocumentLimits::default(), &q, &control(), |_, _| Ok(())).unwrap();
        drop(resave);
        let reopen = ServiceAuthorizedNativeReadPath::from_platform_authorized(path.clone(), &q, &control()).unwrap();
        let saved = {
            let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
            let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(scratch.clone(), &budget).unwrap();
            open_accounted_native(&reopen, &service, OwnedDocumentLimits::default(), q.clone(), &control()).unwrap()
        };
        drop(reopen);
        std::fs::remove_file(&path).unwrap();
        let LayerContent::BackedRaster(saved_current) = &saved.document().layers[0].content else { panic!("saved edit erased") };
        saved_current.checked_view().read_logical_pixel_native_into(-2, 3, &mut bytes, &control()).unwrap();
        assert_eq!(bytes, edit);
        drop(saved);
        drop(owner);
        drop(history);
        assert!(q.usage().unwrap().scratch > 0);
        drop(changed);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
        assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0);
    }
}

#[test]
fn protected_destinations_and_final_gate_preserve_last_good_without_temporary_files() {
    use photocraft_io::accounted_native::AccountedNativeError;
    let root = Root::new();
    let q = quota();
    let doc = Document::new("resident", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    for (name, contents) in [("source.xcf", b"old".as_slice()), ("source.pcraft", b"gimp xcf v022\0".as_slice()), ("wrapped.pcraft", &[0x1f, 0x8b, 8, 0][..])] {
        let path = root.0.join(name);
        std::fs::write(&path, contents).unwrap();
        let cap = ServiceAuthorizedNativeWritePath::from_platform_authorized(path.clone(), &q, &control()).unwrap();
        let result = save_accounted_native_path(&doc, &cap, OwnedDocumentLimits::default(), &q, &control(), |_, _| Ok(()));
        assert!(matches!(result, Err(AccountedNativeError::Unsupported(_))));
        assert_eq!(std::fs::read(&path).unwrap(), contents);
        drop(cap);
        std::fs::remove_file(path).unwrap();
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    let path = root.0.join("last.pcraft");
    std::fs::write(&path, b"last-good").unwrap();
    let cap = ServiceAuthorizedNativeWritePath::from_platform_authorized(path.clone(), &q, &control()).unwrap();
    let before = q.usage().unwrap();
    let calls = AtomicUsize::new(0);
    let result = save_accounted_native_path(&doc, &cap, OwnedDocumentLimits::default(), &q, &control(), |_, _| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 { Ok(()) } else { Err(BackingError::Cancelled.into()) }
    });
    assert!(matches!(result, Err(AccountedNativeError::Backing(BackingError::Cancelled))));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(std::fs::read(&path).unwrap(), b"last-good");
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 1);
    assert_eq!(q.usage().unwrap(), before);
    drop(cap);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn mixed_ledger_and_low_quota_deny_before_creation() {
    let root = Root::new();
    let path = root.0.join("absent.pcraft");
    let q = quota();
    let cap = ServiceAuthorizedNativeWritePath::from_platform_authorized(path.clone(), &q, &control()).unwrap();
    let doc = Document::new("resident", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let other = quota();
    assert!(save_accounted_native_path(&doc, &cap, OwnedDocumentLimits::default(), &other, &control(), |_, _| Ok(())).is_err());
    assert_eq!(other.usage().unwrap(), QuotaUsage::default());
    assert!(!path.exists());
    let low = BackingQuota::new(QuotaLimits { resident: 4096, scratch: 4096, index: 4096 });
    let lowcap = ServiceAuthorizedNativeWritePath::from_platform_authorized(path.clone(), &low, &control()).unwrap();
    let before = low.usage().unwrap();
    assert!(save_accounted_native_path(&doc, &lowcap, OwnedDocumentLimits::default(), &low, &control(), |_, _| Ok(())).is_err());
    assert_eq!(low.usage().unwrap(), before);
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
}

#[test]
fn late_source_failure_and_invalid_owned_input_leave_destination_and_leases_intact() {
    #[derive(Debug)]
    struct Fault(NativePlaneDescriptor);
    impl NativePlaneStore for Fault {
        fn descriptor(&self) -> NativePlaneDescriptor {
            self.0
        }
        fn read_exact_at(&self, _: u64, _: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
            ctl.check(0)?;
            Err(BackingError::Io)
        }
    }
    let root = Root::new();
    let q = quota();
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::U32, true);
    let descriptor = NativePlaneDescriptor::new(format, Rect::new(0, 0, 2, 2), &0u32.to_ne_bytes().repeat(4)).unwrap();
    let layer = BackedLayer::new(BackedRaster::new(Arc::new(Fault(descriptor)), q.clone(), 2, &control()).unwrap(), descriptor, &control()).unwrap();
    let mut doc = Document::new("fault", Size::new(2, 2), ColorMode::Rgb, SampleType::U32);
    doc.layers.push(Layer::new("fault", LayerContent::BackedRaster(layer)));
    let path = root.0.join("last.pcraft");
    std::fs::write(&path, b"last-good").unwrap();
    let write = ServiceAuthorizedNativeWritePath::from_platform_authorized(path.clone(), &q, &control()).unwrap();
    let baseline = q.usage().unwrap();
    assert!(save_accounted_native_path(&doc, &write, OwnedDocumentLimits::default(), &q, &control(), |_, _| Ok(())).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"last-good");
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 1);
    assert_eq!(q.usage().unwrap(), baseline);
    let scratch = root.0.join("scratch");
    std::fs::create_dir(&scratch).unwrap();
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
    let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(scratch.clone(), &budget).unwrap();
    let read = ServiceAuthorizedNativeReadPath::from_platform_authorized(path.clone(), &q, &control()).unwrap();
    let baseline = q.usage().unwrap();
    assert!(open_accounted_native(&read, &service, OwnedDocumentLimits::default(), q.clone(), &control()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0);
    drop(read);
    drop(service);
    drop(write);
    drop(doc);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
