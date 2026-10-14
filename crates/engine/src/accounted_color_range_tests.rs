//! Regression drafts: authored but UNRUN; no runtime or numerical qualification.
use super::*;
use photocraft_doc::{BackedLayer, Document, Layer, accounted_document::AccountedDocument};
use photocraft_geom::Size;
use photocraft_raster::{
    Interrupt, Surface,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    reads: Arc<AtomicUsize>,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        ctl.check(out.len() as u64)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn fixture() -> (Session, BackingQuota, Arc<AtomicUsize>) {
    let raw = [0.23456789f32, 0.5, 0.7, 1., 0.8, 0.125, 0.3, 0.5];
    let mut doc = Document::new("current", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
    doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    let mut layer = Layer::raster("pixels", PixelFormat::RGBA32F);
    layer.surface_mut().unwrap().write_region(doc.bounds(), &raw);
    doc.layers.push(layer);
    let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 16 << 20, scratch: 16 << 20 });
    let reads = Arc::new(AtomicUsize::new(0));
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA32F, doc.bounds(), &[0; 16]).unwrap();
    let raster = BackedRaster::new(
        Arc::new(Store { descriptor, bytes: raw.map(f32::to_ne_bytes).concat(), reads: reads.clone() }),
        q.clone(),
        1,
        &crate::read_control_none(),
    )
    .unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &crate::read_control_none()).unwrap());
    // Trusted fixture retention includes the actual Store bytes and original model.
    let lease = q.reserve(QuotaKind::Resident, 65536).unwrap();
    let owner = AccountedDocument::from_owned(doc, Arc::new(lease), q.clone(), &crate::read_control_none()).unwrap();
    let mut s = Session::new();
    s.add_accounted_document(owner, None, &crate::read_control_none()).unwrap();
    (s, q, reads)
}
fn params() -> Value {
    json!({"sampleAllLayers":false,"points":[[0,0]],"fuzziness":0})
}
#[test]
fn registered_command_publishes_native_selection_and_history_retains_exact_source() {
    let (mut s, _, reads) = fixture();
    let before = s.active().unwrap().doc.clone();
    let rev = s.active().unwrap().revision;
    let result = s.execute(CMD, params()).unwrap();
    assert_eq!(result["selected"], true);
    let selected = s.active().unwrap().doc.clone();
    assert_eq!(s.active().unwrap().revision, rev + 1);
    let mask = selected.selection.as_ref().unwrap();
    assert_eq!(mask.format(), PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    let mut first = [0; 4];
    mask.try_read_pixel_bytes(0, 0, &mut first).unwrap();
    assert_eq!(f32::from_ne_bytes(first), 1.);
    assert!(reads.load(Ordering::Relaxed) > 0);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.shares_state(&before));
    s.execute("edit.redo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.shares_state(&selected));
}
#[test]
fn invert_then_unquantized_prior_min_then_quantization_matches_command_order() {
    let (_, q, _) = fixture();
    let area = Rect::new(0, 0, 2, 1);
    let mut prior = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    prior.write_region(area, &[0.1, 0.9]);
    let (native, selected) = native_bytes(&[0.5, 0.5], Some(&prior), area, true, &q, &crate::read_control_none()).unwrap();
    assert!(selected);
    let values: Vec<_> = native.bytes.chunks_exact(4).map(|p| f32::from_ne_bytes(p.try_into().unwrap())).collect();
    assert_eq!(values, [26f32 / 255., 128f32 / 255.]);
    assert_ne!(values[0].to_bits(), 0.1f32.to_bits());
}
#[test]
fn unsupported_modes_and_work_denial_refuse_before_provider_reads_or_publication() {
    let (mut s, q, reads) = fixture();
    let original = s.active().unwrap().doc.clone();
    let rev = s.active().unwrap().revision;
    for p in [json!({"sampleAllLayers":true}), json!({"sampleAllLayers":false,"mode":"add"}), json!({"sampleAllLayers":false,"select":"reds"})] {
        assert!(s.execute(CMD, p).is_err());
    }
    let before = q.usage().unwrap();
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(run(&mut s, &params(), &deny).is_err());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert_eq!(q.usage().unwrap(), before);
    assert!(s.active().unwrap().doc.shares_state(&original));
    assert_eq!(s.active().unwrap().revision, rev);
}
#[test]
fn cancelled_native_encoding_and_nonfinite_prior_return_all_temporary_charges() {
    let (_, q, _) = fixture();
    let before = q.usage().unwrap();
    let area = Rect::new(0, 0, 1, 1);
    let stop = Interrupt::cancel_only(&|| true);
    let c = ReadControl { interrupt: &stop, work: &|_| Ok(()) };
    assert!(native_bytes(&[0.5], None, area, false, &q, &c).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let mut prior = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    prior.write_region(area, &[f32::NAN]);
    assert!(native_bytes(&[0.5], Some(&prior), area, false, &q, &crate::read_control_none()).is_err());
    assert_eq!(q.usage().unwrap(), before);
}

#[test]
fn registered_start_finishes_inline_and_publishes_one_native_selection_history_step() {
    let (mut session, _, reads) = fixture();
    let original = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let started = session.start(CMD, params()).unwrap();
    let crate::jobs::Started::Done(response) = started else {
        panic!("the closed <=200-side accounted Color Range lane must finish inline");
    };
    assert_eq!(response["selected"], true);
    assert!(session.active_job().is_none());
    let selected = session.active().unwrap().doc.clone();
    assert_eq!(session.active().unwrap().revision, revision + 1);
    assert!(selected.accounted().is_some());
    let mask = selected.selection.as_ref().unwrap();
    assert_eq!(mask.format(), PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    let mut native = [0; 4];
    mask.try_read_pixel_bytes(0, 0, &mut native).unwrap();
    assert_eq!(f32::from_ne_bytes(native).to_bits(), 1f32.to_bits());
    assert!(reads.load(Ordering::Relaxed) > 0);
    session.execute("edit.undo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&original));
    session.execute("edit.redo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&selected));
}
