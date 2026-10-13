//! Ordinary checked reduced preview publication and cancellation regressions.
use super::*;
use photocraft_doc::{ColorMode, Layer, SampleType, Size};
use photocraft_geom::TileCoord;
use photocraft_raster::{Interrupt, RasterInputError};
use std::sync::atomic::{AtomicUsize, Ordering};

fn document() -> Document {
    let mut doc = Document::new("checked UI preview", Size::new(64, 513), ColorMode::Rgb, SampleType::F32);
    let mut layer = Layer::raster("native", doc.pixel_format());
    layer.surface_mut().unwrap().try_write_pixel_f64(0, 0, &[1., 0., 0., 1.]).unwrap();
    doc.layers.push(layer);
    doc
}

#[test]
fn ordinary_preview_checked_reduction_preserves_values_and_late_cancel_source() {
    let doc = document();
    let before = doc.clone();
    let actual = ordinary_preview(&doc, 32, 257, None, &Interrupt::NONE).unwrap();
    let legacy = photocraft_compose::render_reduced(&doc, 32, 257).unwrap();
    assert_eq!(actual.rect, legacy.rect);
    assert_eq!(actual.px, legacy.px);
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::AcqRel) >= 20;
    assert!(ordinary_preview(&doc, 32, 257, None, &Interrupt::cancel_only(&cancel)).is_err());
    assert!(calls.load(Ordering::Acquire) > 20);
    assert_eq!(doc, before);
    let rect = DRect::new(0, 0, 32, 200);
    let damage = ordinary_preview(&doc, 32, 257, Some(rect), &Interrupt::NONE).unwrap();
    for y in damage.rect.y0..damage.rect.y1 {
        for x in damage.rect.x0..damage.rect.x1 {
            assert_eq!(damage.get(x, y), actual.get(x, y));
        }
    }
}

#[test]
fn ordinary_cpu_canvas_refusal_and_late_cancel_keep_last_good_texture_and_status() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.add_document(document(), None).unwrap();
    let ctx = egui::Context::default();
    let first = ensure_texture(&mut app, &ctx, 0, None).unwrap();
    ctx.tex_manager().write().take_delta().clear();
    {
        let state = app.session.active_mut().unwrap();
        state.revision += 1;
    }
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::AcqRel) >= 20;
    let before = app.session.active().unwrap().doc.clone();
    assert_eq!(ensure_texture_with_interrupt(&mut app, &ctx, 0, None, &Interrupt::cancel_only(&cancel)), Some(first));
    assert!(app.ui.status.to_lowercase().contains("cancel"));
    assert!(ctx.tex_manager().write().take_delta().set.is_empty());
    assert_eq!(app.session.active().unwrap().doc, before);
    {
        let state = app.session.active_mut().unwrap();
        let doc = state.doc.try_resident_mut().unwrap();
        let surface = doc.layers[0].surface_mut().unwrap();
        let tile = surface.tiles().next().unwrap().1.clone();
        surface.put_tiles([(TileCoord::new(i32::MIN, i32::MAX), tile)]);
        state.revision += 1;
    }
    assert_eq!(ensure_texture(&mut app, &ctx, 0, None), Some(first));
    assert!(ctx.tex_manager().write().take_delta().set.is_empty());
    assert!(!app.ui.status.is_empty());
    let source = app.session.active().unwrap().doc.clone();
    assert_eq!(source.layers[0].surface().unwrap().try_validate_tile_origins(), Err(RasterInputError::InvalidBounds));
    assert_eq!(ensure_texture(&mut app, &ctx, 0, None), Some(first));
    assert_eq!(app.session.active().unwrap().doc, source);
}

#[test]
fn backed_layer_thumbnail_refuses_without_read_or_empty_texture_publication() {
    use photocraft_raster::backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaLimits},
    };
    use std::sync::Arc;
    #[derive(Debug)]
    struct Store {
        descriptor: NativePlaneDescriptor,
        reads: AtomicUsize,
    }
    impl NativePlaneStore for Store {
        fn descriptor(&self) -> NativePlaneDescriptor {
            self.descriptor
        }
        fn read_exact_at(&self, _: u64, _: &mut [u8], _: &ReadControl<'_>) -> Result<(), BackingError> {
            self.reads.fetch_add(1, Ordering::AcqRel);
            Err(BackingError::Io)
        }
    }
    let descriptor = NativePlaneDescriptor::new(photocraft_color::PixelFormat::RGBA8, DRect::new(0, 0, 1, 1), &[0, 0, 0, 255]).unwrap();
    let store = Arc::new(Store { descriptor, reads: AtomicUsize::new(0) });
    let quota = BackingQuota::new(QuotaLimits { resident: 1 << 20, scratch: 0, index: 1 << 20 });
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
    let raster = BackedRaster::new(store.clone(), quota.clone(), 1, &control).unwrap();
    let layer = Layer::new("owned native", photocraft_doc::LayerContent::BackedRaster(photocraft_doc::BackedLayer::new(raster, descriptor, &control).unwrap()));
    let mut doc = Document::new("backed thumbnail", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(layer);
    let source = doc.clone();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    ctx.tex_manager().write().take_delta().clear(); // Exclude the context's initial font/white texture.
    assert!(app.layer_thumb(&ctx, &doc, &doc.layers[0]).is_err());
    assert!(app.ui.status_error);
    assert!(app.ui.status.contains("Native-backed"));
    assert!(app.thumbs.is_empty());
    let mut delta = ctx.tex_manager().write().take_delta();
    let published = delta.set.len();
    delta.clear();
    assert_eq!(published, 0);
    assert_eq!(store.reads.load(Ordering::Acquire), 0);
    assert_eq!(doc, source);
}
