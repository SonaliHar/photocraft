//! Real bounded Vulkan uploads; no desktop or fallback adapter masking.
use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, Size};
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn canvas() -> GpuCanvas {
    let mut setup = egui_kittest::wgpu::default_wgpu_setup();
    if let egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
        create.instance_descriptor.backends = wgpu::Backends::VULKAN;
    }
    let rs = egui_kittest::wgpu::create_render_state(setup, Default::default());
    let gpu = GpuCanvas::new(&rs);
    gpu.set_memory_budget(64 << 20);
    gpu
}
fn solid(width: u32, height: u32, color: [f32; 4]) -> Document {
    let mut doc = Document::new("atomic GPU upload", Size::new(width, height), ColorMode::Rgb, SampleType::F32);
    doc.layers.push(Layer::new("native default", LayerContent::Raster(Surface::with_default(PixelFormat::RGBA32F, &color))));
    doc
}
fn visual(gpu: &GpuCanvas, key: u64, name: &str) {
    let Some(directory) = std::env::var_os("PHOTOCRAFT_GPU_ATOMIC_VISUALS") else { return };
    let (_, size, pixels) = gpu.read_texels(key).unwrap();
    let bytes = pixels.iter().flat_map(|pixel| pixel.map(|value| (value.clamp(0., 1.) * 255.).round() as u8)).collect();
    let image = photocraft_codecs::Image::from_raw(size[0], size[1], photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, bytes).unwrap();
    let encoded = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(std::path::Path::new(&directory).join(name), encoded).unwrap();
}

fn texture(gpu: &GpuCanvas, key: u64) -> wgpu::Texture {
    gpu.rs.renderer.read().callback_resources.get::<Resources>().unwrap().docs[&key].tiles[0].texture.clone()
}

#[test]
#[ignore = "actual bounded Vulkan late-cancellation texture publication proof"]
fn gpu_upload_late_band_cancel_keeps_exact_last_good_texture_and_source() {
    let gpu = canvas();
    let original = solid(64, 513, [1., 0., 0., 1.]);
    let key = original.id.0;
    gpu.upload_composite(key, &original, None).unwrap();
    let old_texture = texture(&gpu, key);
    let old_pixels = gpu.read_texels(key).unwrap();
    let mut changed = solid(64, 513, [0., 0., 1., 1.]);
    changed.id = original.id;
    let source = changed.clone();
    let cancelled = AtomicBool::new(false);
    let submitted = AtomicUsize::new(0);
    let cancel = || cancelled.load(Ordering::Acquire);
    let progress = |fraction: f32| {
        assert!(fraction > 0. && fraction < 1., "must signal after the first real partial band submission");
        submitted.fetch_add(1, Ordering::AcqRel);
        cancelled.store(true, Ordering::Release);
    };
    let control = Interrupt::new(&cancel, &progress);
    assert!(matches!(gpu.upload_composite_bands(key, &changed, None, &control, 256), Err(CanvasUploadError::Raster(RasterInputError::Cancelled))));
    assert_eq!(submitted.load(Ordering::Acquire), 1);
    assert_eq!(texture(&gpu, key), old_texture);
    assert!(gpu.read_texels(key).unwrap() == old_pixels, "cancelled bands must not replace any last-good pixels");
    assert_eq!(gpu.rs.renderer.read().callback_resources.get::<Resources>().unwrap().pending_upload_bytes.load(Ordering::Acquire), 0);
    visual(&gpu, key, "last-good-after-cancel.png");
    gpu.upload_composite(key, &changed, None).unwrap();
    assert!(gpu.read_texels(key).unwrap() != old_pixels);
    visual(&gpu, key, "successful-replacement.png");
    assert_eq!(changed, source);
    assert!(gpu.fault().is_none());
}

#[test]
#[ignore = "actual bounded Vulkan checked-input texture publication proof"]
fn gpu_upload_refused_resized_context_keeps_previous_texture() {
    let gpu = canvas();
    let original = solid(16, 16, [0., 1., 0., 1.]);
    let key = original.id.0;
    gpu.upload_composite(key, &original, None).unwrap();
    let before_texture = texture(&gpu, key);
    let before = gpu.read_texels(key).unwrap();
    let mut hostile = solid(32, 17, [0., 0., 1., 1.]);
    hostile.id = original.id;
    hostile.xcf_state = Some(Default::default());
    let retained = hostile.clone();
    assert!(gpu.upload_composite(key, &hostile, None).is_err());
    assert_eq!(texture(&gpu, key), before_texture);
    assert_eq!(gpu.read_texels(key).unwrap(), before);
    assert!(hostile == retained);
}

#[test]
fn gpu_upload_footprint_checks_full_mips_scratch_and_hostile_bounds() {
    assert_eq!(upload_footprint([4, 4], 64, FORMAT, true), Some((100, 20)));
    assert_eq!(upload_footprint([4, 4], 64, FORMAT_HIGH, true), Some((184, 40)));
    assert_eq!(upload_footprint([0, 4], 64, FORMAT, false), None);
    assert_eq!(upload_footprint([4, 4], 0, FORMAT, false), None);
    assert_eq!(upload_footprint([u32::MAX, 4], 64, FORMAT, false), None);
    assert_eq!(upload_footprint([1 << 30, 1 << 30], 64, FORMAT, false), None);
}

#[test]
#[ignore = "actual Vulkan old plus new aggregate denial and refund"]
fn gpu_upload_aggregate_one_byte_under_preserves_last_good_and_refunds() {
    let gpu = canvas();
    let original = solid(16, 16, [1., 0., 0., 1.]);
    let key = original.id.0;
    gpu.upload_composite(key, &original, None).unwrap();
    let old = texture(&gpu, key);
    let pixels = gpu.read_texels(key).unwrap();
    let format = gpu.format_for(original.depth, [16, 16]);
    let separate = gpu.rs.renderer.read().callback_resources.get::<Resources>().unwrap().separate_mip_targets;
    let (fresh, scratch) = upload_footprint([16, 16], gpu.tile, format, separate).unwrap();
    gpu.set_memory_budget(fresh * 2 + scratch - 1);
    assert!(matches!(gpu.upload_composite(key, &original, None), Err(CanvasUploadError::LimitExceeded)));
    assert_eq!(texture(&gpu, key), old);
    assert!(gpu.read_texels(key).unwrap() == pixels);
    assert_eq!(gpu.rs.renderer.read().callback_resources.get::<Resources>().unwrap().pending_upload_bytes.load(Ordering::Acquire), 0);
    gpu.set_memory_budget(fresh * 2 + scratch);
    gpu.upload_composite(key, &original, None).unwrap();
    assert!(gpu.read_texels(key).unwrap() == pixels);
}

#[test]
#[ignore = "actual Vulkan navigator context with checked CPU thumbnail cancellation"]
fn gpu_upload_navigator_checked_thumbnail_cancel_keeps_last_good_preview() {
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.add_document(solid(64, 513, [1., 0., 0., 1.]), None).unwrap();
    app.gpu = Some(canvas());
    let ctx = egui::Context::default();
    let first = crate::canvas::navigator_texture(&mut app, &ctx, 0).unwrap();
    ctx.tex_manager().write().take_delta().clear();
    {
        let state = app.session.active_mut().unwrap();
        state.doc.try_resident_mut().unwrap().name.push_str(" changed snapshot");
        state.revision += 1;
    }
    let source = app.session.active().unwrap().doc.clone();
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::AcqRel) >= 20;
    assert_eq!(crate::canvas::navigator_texture_with_interrupt(&mut app, &ctx, 0, &Interrupt::cancel_only(&cancel)), Some(first));
    assert!(polls.load(Ordering::Acquire) > 20);
    assert!(app.ui.status_error && app.ui.status.to_lowercase().contains("cancel"));
    assert!(ctx.tex_manager().write().take_delta().set.is_empty());
    assert_eq!(app.session.active().unwrap().doc, source);
}
