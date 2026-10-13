//! Real finite native archive load -> backed UI display, with owned quota lifetime.
use super::*;
use crate::canvas::{ensure_texture, ensure_texture_with_interrupt};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{BackedLayer, Layer, SampleEncoding};
use photocraft_format::{
    LoadOptions,
    canonical_archive::ImmutableCanonicalArchive,
    owned_document::{OwnedDocumentLimits, load_document_from_owned_archive, save_current_document_archive},
};
use photocraft_geom::{Rect, Size};
use photocraft_raster::backing::{
    BackedRaster, NativePlaneDescriptor, NativePlaneStore,
    quota::{BackingQuota, QuotaLimits},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Defaults {
    descriptor: NativePlaneDescriptor,
    pixel: Vec<u8>,
}
impl NativePlaneStore for Defaults {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(out.len() as u64)?;
        if offset.checked_add(out.len() as u64).is_none_or(|end| end > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.pixel[((offset + i as u64) % self.pixel.len() as u64) as usize];
        }
        Ok(())
    }
}
#[derive(Debug)]
struct Archive {
    bytes: Vec<u8>,
    quota: BackingQuota,
    fail: AtomicBool,
    _lease: QuotaLease,
}
impl ImmutableCanonicalArchive for Archive {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(out.len() as u64)?;
        if self.fail.load(Ordering::Acquire) {
            return Err(BackingError::Io);
        }
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn imported(mode: ColorMode) -> (photocraft_format::owned_document::OwnedDocumentArchive, Arc<Archive>, BackingQuota) {
    let q = BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 4 << 20, scratch: 0 });
    let format = PixelFormat::new(mode, SampleType::F64, true);
    let values: &[f64] = if mode == ColorMode::Rgb { &[0.25, 0.5, 0.75, 0.5] } else { &[0.5, 0.5] };
    let bytes: Vec<_> = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let descriptor = NativePlaneDescriptor::new(format, Rect::new(0, 0, 64, 32), &vec![0; format.bytes_per_pixel()]).unwrap();
    let raster = BackedRaster::new(Arc::new(Defaults { descriptor, pixel: bytes }), q.clone(), 1, &control()).unwrap();
    let mut doc = Document::new("finite archive display", Size::new(64, 32), mode, SampleType::F64);
    doc.layers.push(Layer::new("exact source", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control()).unwrap())));
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    doc.icc_profile = Some(if mode == ColorMode::Rgb { photocraft_cms::Builtin::Srgb } else { photocraft_cms::Builtin::SGray }.profile().to_bytes());
    let wire = save_current_document_archive(&doc, std::io::Cursor::new(Vec::new()), OwnedDocumentLimits::default(), &q, &control()).unwrap().into_inner();
    let lease = q.reserve(QuotaKind::Resident, wire.capacity() as u64).unwrap();
    let archive = Arc::new(Archive { bytes: wire, quota: q.clone(), fail: AtomicBool::new(false), _lease: lease });
    let imported = load_document_from_owned_archive(archive.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &control()).unwrap();
    (imported, archive, q)
}
#[test]
fn backed_cpu_imported_archive_matches_independent_pixels_and_retains_display_lease() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        let (owned, _archive, q) = imported(mode);
        let baseline = q.usage().unwrap();
        assert!(
            photocraft_compose::render_reduced_with_interrupt(owned.document(), 32, 16, &Interrupt::NONE).is_err(),
            "legacy resident route must remain refused"
        );
        let (image, lease) = backed_display_image(owned.document(), 32, 16, None, &Interrupt::NONE).unwrap();
        let expected = if mode == ColorMode::Rgb {
            egui::Color32::from_rgba_unmultiplied(64, 128, 191, 128)
        } else {
            egui::Color32::from_rgba_unmultiplied(128, 128, 128, 128)
        };
        assert!(image.pixels.iter().all(|p| *p == expected));
        assert_eq!(lease.bytes(), image.pixels.capacity() as u64 * 4);
        assert!(q.usage().unwrap().resident >= baseline.resident + lease.bytes());
        drop(image);
        drop(lease);
        assert!(q.usage().unwrap().resident >= baseline.resident);
        drop(owned);
        drop(_archive);
        assert_eq!(q.usage().unwrap().resident, 0);
    }
}
#[test]
fn backed_cpu_canvas_archive_io_cancel_and_profile_refusal_preserve_last_good() {
    let (owned, archive, q) = imported(ColorMode::Rgb);
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.add_accounted_document(owned.into_accounted_document(&control()).unwrap(), None, &control()).unwrap();
    let ctx = egui::Context::default();
    let first = ensure_texture(&mut app, &ctx, 0, None).unwrap();
    ctx.tex_manager().write().take_delta().clear();
    let fresh = load_document_from_owned_archive(archive.clone(), &LoadOptions::default(), OwnedDocumentLimits::default(), q.clone(), &control()).unwrap();
    app.session.active_mut().unwrap().doc =
        photocraft_engine::document_snapshot::DocumentSnapshot::from_accounted(fresh.into_accounted_document(&control()).unwrap()).unwrap();
    let baseline = q.usage().unwrap();
    archive.fail.store(true, Ordering::Release);
    app.session.active_mut().unwrap().revision += 1;
    let before = app.session.active().unwrap().doc.clone();
    assert_eq!(ensure_texture(&mut app, &ctx, 0, None), Some(first));
    assert!(app.ui.status_error);
    assert!(ctx.tex_manager().write().take_delta().set.is_empty());
    assert!(app.session.active().unwrap().doc.shares_state(&before));
    assert_eq!(q.usage().unwrap(), baseline);
    archive.fail.store(false, Ordering::Release);
    app.session.active_mut().unwrap().revision += 1;
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::AcqRel) > 20;
    assert_eq!(ensure_texture_with_interrupt(&mut app, &ctx, 0, None, &Interrupt::cancel_only(&cancel)), Some(first));
    assert!(app.ui.status_error);
    assert!(ctx.tex_manager().write().take_delta().set.is_empty());
    assert_eq!(q.usage().unwrap(), baseline);
    // Both old display and a new staging display must coexist in the same quota.
    app.session.active_mut().unwrap().revision += 1;
    let held = q.reserve(QuotaKind::Resident, (64 << 20) - baseline.resident - 1).unwrap();
    let denied_baseline = q.usage().unwrap();
    assert_eq!(ensure_texture(&mut app, &ctx, 0, None), Some(first));
    assert!(app.ui.status_error);
    assert!(ctx.tex_manager().write().take_delta().set.is_empty());
    assert_eq!(q.usage().unwrap(), denied_baseline);
    drop(held);
    assert_eq!(q.usage().unwrap(), baseline);
    {
        let state = app.session.active_mut().unwrap();
        assert!(state.doc.try_resident_mut().is_err(), "unchecked ICC mutation must remain refused for an accounted owner");
    }
    assert!(app.canvases.values().any(|cache| cache.texture.as_ref().is_some_and(|texture| texture.id() == first.0)));
    assert!(ctx.tex_manager().write().take_delta().set.is_empty());
    assert_eq!(q.usage().unwrap(), baseline);
}
#[test]
#[ignore = "bounded actual Vulkan backed CPU texture presentation, no desktop"]
fn backed_cpu_vulkan_preview_from_real_native_archive() {
    let directory = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_BACKED_UI_VISUALS").expect("explicit private visual directory"));
    std::fs::create_dir_all(&directory).unwrap();
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        let (owner, _archive, _quota) = imported(mode);
        let mut session = photocraft_engine::Session::new();
        session.add_accounted_document(owner.into_accounted_document(&control()).unwrap(), None, &control()).unwrap();
        let app = crate::PhotocraftApp::new(session, Default::default());
        let mut setup = egui_kittest::wgpu::default_wgpu_setup();
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
            create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
        }
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(300., 180.)).wgpu_setup(setup).build_ui_state(
            |ui, app| {
                let (texture, _) = ensure_texture(app, ui.ctx(), 0, None).unwrap();
                ui.label("Owned native CPU preview");
                ui.image((texture, egui::vec2(256., 128.)));
            },
            app,
        );
        harness.run_steps(2);
        assert!(!harness.state().ui.status_error);
        harness.render().unwrap().save(directory.join(if mode == ColorMode::Rgb { "backed-rgb.png" } else { "backed-gray.png" })).unwrap();
    }
}

#[test]
fn accounted_ui_inspection_and_resident_preview_refusals_preserve_snapshot_and_quota() {
    let (owned, archive, quota) = imported(ColorMode::Rgb);
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.add_accounted_document(owned.into_accounted_document(&control()).unwrap(), None, &control()).unwrap();
    let source = app.session.active().unwrap().doc.clone();
    let baseline = quota.usage().unwrap();
    let revision = app.session.active().unwrap().revision;
    let history = app.session.active().unwrap().history.past_len();
    assert!(crate::control::inspect(&app, &egui::Context::default()).is_err(), "accounted JSON cloning must not bypass the charged model owner");
    assert!(crate::transform_tool::begin(&mut app, &egui::Context::default()).is_err(), "unported transform must refuse before making a resident model copy");
    assert!(crate::tone::compute_histograms(&source, crate::tone::HistSource::Layer(source.layers[0].id), photocraft_doc::adjust::ToneSpace::Rgb).is_err());
    let current = app.session.active().unwrap();
    assert!(current.doc.shares_state(&source));
    assert_eq!(current.revision, revision);
    assert_eq!(current.history.past_len(), history);
    assert_eq!(quota.usage().unwrap(), baseline);
    drop(source);
    drop(app);
    drop(archive);
    assert_eq!(quota.usage().unwrap().resident, 0);
    assert_eq!(quota.usage().unwrap().index, 0);
}
