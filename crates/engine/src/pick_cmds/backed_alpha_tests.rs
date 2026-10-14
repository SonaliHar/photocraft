use super::*;
use photocraft_doc::{ColorMode, Document, LayerContent, PixelFormat, SampleType};
use photocraft_geom::{Rect, Size};
use photocraft_raster::backing::{
    BackedRaster, NativePlaneDescriptor, NativePlaneStore,
    quota::{BackingQuota, QuotaLimits},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize},
};
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    pixel: Vec<u8>,
    reads: Arc<AtomicUsize>,
    fault: bool,
    cancel: Option<Arc<AtomicBool>>,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        control.check(output.len() as u64)?;
        if self.fault {
            return Err(BackingError::Io);
        }
        if offset.checked_add(output.len() as u64).is_none_or(|n| n > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        for (index, value) in output.iter_mut().enumerate() {
            *value = self.pixel[((offset as usize) + index) % self.pixel.len()];
        }
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}
fn fixture(format: PixelFormat, pixel: Vec<u8>, limits: QuotaLimits, fault: bool, cancel: Option<Arc<AtomicBool>>) -> (Document, Arc<AtomicUsize>) {
    let quota = BackingQuota::new(limits);
    let reads = Arc::new(AtomicUsize::new(0));
    let descriptor = NativePlaneDescriptor::new(format, Rect::new(1, 1, 2, 2), &pixel).unwrap();
    let control = crate::read_control_none();
    let raster = BackedRaster::new(Arc::new(Store { descriptor, pixel, reads: Arc::clone(&reads), fault, cancel }), quota, 1, &control).unwrap();
    let backed = BackedLayer::new(raster, descriptor, &control).unwrap();
    let mut doc = Document::new("alpha only", Size::new(3, 3), format.mode, format.sample);
    doc.layers.push(Layer::new("native", LayerContent::BackedRaster(backed)));
    (doc, reads)
}
fn limits() -> QuotaLimits {
    QuotaLimits { resident: 32 << 20, index: 4 << 20, scratch: 0 }
}
fn decision(doc: &Document, work: &PickReadWork, control: &Interrupt<'_>) -> Result<bool> {
    let layer = &doc.layers[0];
    let LayerContent::BackedRaster(backed) = &layer.content else { panic!("own fixture") };
    hit(doc, layer, backed, 1, 1, work, control)
}
#[test]
fn tiny_f64_alpha_with_nan_rgb_hits_without_color_decoding_or_f32_underflow() {
    let pixel = [f64::from_bits(0x7ff8000000000042), f64::NEG_INFINITY, -0.0, 1e-300].map(f64::to_ne_bytes).concat();
    let (mut doc, reads) = fixture(PixelFormat::RGBA64F, pixel, limits(), false, None);
    doc.layers[0].opacity = 0.0;
    doc.layers[0].fill_opacity = 0.0;
    let before = doc.clone();
    assert!(decision(&doc, &PickReadWork::new(), &Interrupt::NONE).unwrap());
    assert!(crate::pick_cmds::layers_at_checked(&doc, 1, 1, &Interrupt::NONE).unwrap().contains(&doc.layers[0].id));
    assert_eq!(doc, before);
    assert!(reads.load(Ordering::Relaxed) > 0);
}
#[test]
fn nonfinite_alpha_refuses_and_exact_native_zero_does_not_hit() {
    for alpha in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let (doc, _) = fixture(PixelFormat::RGBA64F, [0.0, 0.0, 0.0, alpha].map(f64::to_ne_bytes).concat(), limits(), false, None);
        assert!(decision(&doc, &PickReadWork::new(), &Interrupt::NONE).unwrap_err().to_string().contains("backed-alpha-nonfinite"));
    }
    for alpha in [0.0, -0.0] {
        let (doc, _) = fixture(PixelFormat::RGBA64F, [f64::NAN, 0.0, 0.0, alpha].map(f64::to_ne_bytes).concat(), limits(), false, None);
        assert!(!decision(&doc, &PickReadWork::new(), &Interrupt::NONE).unwrap());
    }
    let (doc, _) = fixture(PixelFormat::RGBA32, [0u32, 0, 0, 1].map(u32::to_ne_bytes).concat(), limits(), false, None);
    assert!(decision(&doc, &PickReadWork::new(), &Interrupt::NONE).unwrap());
}
#[test]
fn quota_fault_and_late_cancel_never_turn_an_error_into_transparency() {
    let bytes = vec![255; 4];
    let (doc, reads) = fixture(PixelFormat::RGBA8, bytes.clone(), QuotaLimits { resident: 4096, index: 4096, scratch: 0 }, false, None);
    let before = doc.clone();
    assert!(decision(&doc, &PickReadWork::new(), &Interrupt::NONE).is_err());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert_eq!(doc, before);
    let (doc, reads) = fixture(PixelFormat::RGBA8, bytes.clone(), limits(), true, None);
    assert!(decision(&doc, &PickReadWork::new(), &Interrupt::NONE).is_err());
    assert!(reads.load(Ordering::Relaxed) > 0);
    let stop = Arc::new(AtomicBool::new(false));
    let (doc, reads) = fixture(PixelFormat::RGBA8, bytes, limits(), false, Some(Arc::clone(&stop)));
    let cancelled = || stop.load(Ordering::Relaxed);
    assert!(matches!(decision(&doc, &PickReadWork::new(), &Interrupt::cancel_only(&cancelled)), Err(EngineError::Cancelled)));
    assert!(reads.load(Ordering::Relaxed) > 0);
}
#[test]
fn outside_noalpha_and_unsupported_semantics_do_not_read_backing() {
    let (mut doc, reads) = fixture(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), f64::NAN.to_ne_bytes().to_vec(), limits(), true, None);
    assert!(decision(&doc, &PickReadWork::new(), &Interrupt::NONE).unwrap());
    let layer = &doc.layers[0];
    let LayerContent::BackedRaster(backed) = &layer.content else { panic!("own fixture") };
    assert!(!hit(&doc, layer, backed, 0, 0, &PickReadWork::new(), &Interrupt::NONE).unwrap());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    doc.xcf_state = Some(photocraft_doc::XcfState::default());
    assert!(decision(&doc, &PickReadWork::new(), &Interrupt::NONE).unwrap_err().to_string().contains("backed-pixel-sampling-unsupported"));
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    doc.xcf_state = None;
    doc.layers[0].mask = Some(photocraft_doc::LayerMask {
        surface: Arc::new(photocraft_raster::Surface::new(PixelFormat::GRAY8)),
        enabled: false,
        linked: true,
        density: 1.0,
        feather: 0.0,
    });
    assert!(decision(&doc, &PickReadWork::new(), &Interrupt::NONE).is_err());
    doc.layers[0].mask = None;
    doc.layers[0].effects.psd_raw = Some(Arc::new(vec![1, 2, 3]));
    assert!(decision(&doc, &PickReadWork::new(), &Interrupt::NONE).is_err());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}
#[test]
fn shared_work_does_not_reset_or_refund_between_reads_and_precancel_is_typed() {
    let work = PickReadWork::new();
    work.consume(MAX_WORK - 1, &Interrupt::NONE).unwrap();
    work.consume(1, &Interrupt::NONE).unwrap();
    assert_eq!(work.consume(1, &Interrupt::NONE), Err(BackingError::Work));
    let (doc, reads) = fixture(PixelFormat::RGBA8, vec![255; 4], limits(), false, None);
    assert!(matches!(decision(&doc, &PickReadWork::new(), &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}
