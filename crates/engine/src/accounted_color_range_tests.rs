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
    fixture_depth(SampleType::F32)
}
fn fixture_depth(depth: SampleType) -> (Session, BackingQuota, Arc<AtomicUsize>) {
    let raw = [0.23456789f32, 0.5, 0.7, 1., 0.8, 0.125, 0.3, 0.5];
    let mut doc = Document::new("current", Size::new(2, 1), ColorMode::Rgb, depth);
    doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    let format = PixelFormat::new(ColorMode::Rgb, depth, true);
    let mut layer = Layer::raster("pixels", format);
    layer.surface_mut().unwrap().write_region(doc.bounds(), &raw);
    doc.layers.push(layer);
    let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 16 << 20, scratch: 16 << 20 });
    let reads = Arc::new(AtomicUsize::new(0));
    let default = vec![0; format.bytes_per_pixel()];
    let descriptor = NativePlaneDescriptor::new(format, doc.bounds(), &default).unwrap();
    let bytes = doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap();
    let raster = BackedRaster::new(Arc::new(Store { descriptor, bytes, reads: reads.clone() }), q.clone(), 1, &crate::read_control_none()).unwrap();
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
// start is native background work and wasm inline work. Only inspect the
// published document after this real completion boundary.
fn finish_started(session: &mut Session, started: crate::jobs::Started) -> Value {
    match started {
        crate::jobs::Started::Done(value) => value,
        crate::jobs::Started::Job(job) => session.wait_job(job).unwrap(),
    }
}
fn start_and_finish(session: &mut Session, params: Value) -> Value {
    let started = session.start(CMD, params).unwrap();
    finish_started(session, started)
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
    assert_eq!(mask.format(), PixelFormat::GRAY8);
    let mut first = [0; 1];
    mask.try_read_pixel_bytes(0, 0, &mut first).unwrap();
    assert_eq!(first, [255]);
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
    assert_eq!(native.bytes, [26, 128]);
    assert_ne!((f32::from(native.bytes[0]) / 255.).to_bits(), 0.1f32.to_bits());
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
fn registered_start_completes_platform_job_then_publishes_one_native_selection_history_step() {
    let (mut session, _, reads) = fixture();
    let original = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let started = session.start(CMD, params()).unwrap();
    #[cfg(not(target_arch = "wasm32"))]
    {
        assert!(matches!(&started, crate::jobs::Started::Job(_)));
        assert!(session.active().unwrap().doc.shares_state(&original));
        assert_eq!(session.active().unwrap().revision, revision);
    }
    #[cfg(target_arch = "wasm32")]
    assert!(matches!(&started, crate::jobs::Started::Done(_)));
    let response = finish_started(&mut session, started);
    assert_eq!(response["selected"], true);
    assert!(session.active_job().is_none());
    let selected = session.active().unwrap().doc.clone();
    assert_eq!(session.active().unwrap().revision, revision + 1);
    assert!(selected.accounted().is_some());
    let mask = selected.selection.as_ref().unwrap();
    assert_eq!(mask.format(), PixelFormat::GRAY8);
    let mut native = [0; 1];
    mask.try_read_pixel_bytes(0, 0, &mut native).unwrap();
    assert_eq!(native, [255]);
    assert!(reads.load(Ordering::Relaxed) > 0);
    session.execute("edit.undo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&original));
    session.execute("edit.redo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&selected));
}

#[test]
fn each_admitted_source_depth_uses_upstream_gray8_selection_and_exact_history_owners() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let (mut session, _, reads) = fixture_depth(depth);
        let original = session.active().unwrap().doc.clone();
        let started = session.start(CMD, params()).unwrap();
        let response = finish_started(&mut session, started);
        assert_eq!(response["selected"], true);
        let selected = session.active().unwrap().doc.clone();
        assert_eq!(selected.depth, depth);
        let mask = selected.selection.as_ref().unwrap();
        assert_eq!(mask.format(), PixelFormat::GRAY8);
        let mut word = [0];
        mask.try_read_pixel_bytes(0, 0, &mut word).unwrap();
        assert_eq!(word, [255]);
        assert!(reads.load(Ordering::Relaxed) > 0);
        session.execute("edit.undo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&original));
        session.execute("edit.redo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&selected));
    }
}
#[test]
fn native_prior_depth_is_preserved_until_final_gray8_rounding_and_gray8_repeats() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let (mut session, quota, _) = fixture_depth(depth);
        let format = PixelFormat::new(ColorMode::Grayscale, depth, false);
        let raw = match depth {
            SampleType::U8 => vec![26, 230],
            SampleType::U16 => [6554u16, 58982].into_iter().flat_map(u16::to_ne_bytes).collect(),
            SampleType::F32 => [0.1f32, 0.9].into_iter().flat_map(f32::to_ne_bytes).collect(),
            _ => unreachable!(),
        };
        let default = vec![0; format.bytes_per_pixel()];
        let prior =
            AccountedSelection::try_from_native(format, Rect::new(0, 0, 2, 1), &default, &raw, CoverageDomain::FiniteUnit, &quota, &crate::read_control_none())
                .unwrap();
        let target = AccountedSelectionTarget::capture(&session).unwrap();
        session.replace_accounted_selection(target, "prior", Some(&prior), &crate::read_control_none()).unwrap();
        start_and_finish(&mut session, params());
        let mask = session.active().unwrap().doc.selection.as_ref().unwrap();
        assert_eq!(mask.format(), PixelFormat::GRAY8);
        let mut first = [0];
        mask.try_read_pixel_bytes(0, 0, &mut first).unwrap();
        assert_eq!(first, [26]);
        // Subsequent queries must accept their privately proven Gray8 output,
        // even when the RGB document's own sample depth is U16 or Float.
        start_and_finish(&mut session, params());
        let mask = session.active().unwrap().doc.selection.as_ref().unwrap();
        mask.try_read_pixel_bytes(0, 0, &mut first).unwrap();
        assert_eq!(first, [26]);
    }
}
#[test]
fn wide_source_depths_are_refused_before_provider_reads() {
    for depth in [SampleType::U32, SampleType::F64] {
        let (mut session, _, reads) = fixture_depth(depth);
        let original = session.active().unwrap().doc.clone();
        assert!(session.start(CMD, params()).is_err());
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        assert!(session.active().unwrap().doc.shares_state(&original));
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cancelled_registered_job_preserves_original_history_redo_and_journal() {
    let (mut session, _, _) = fixture();
    session.execute(CMD, params()).unwrap();
    let selected = session.active().unwrap().doc.clone();
    session.execute("edit.undo", json!({})).unwrap();
    assert!(session.active().unwrap().history.can_redo());
    let original = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let past = session.active().unwrap().history.past_len();
    let journal = session.journal.len();
    let crate::jobs::Started::Job(job) = session.start(CMD, params()).unwrap() else {
        panic!("native accounted Color Range must expose its actual job");
    };
    // Results cannot publish before polling. Cancellation discards even a worker
    // result that has already reached its private completion channel.
    assert!(session.active().unwrap().doc.shares_state(&original));
    assert!(session.cancel_job(job));
    assert!(matches!(session.wait_job(job), Err(EngineError::Cancelled)));
    assert!(session.active().unwrap().doc.shares_state(&original));
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(session.active().unwrap().history.past_len(), past);
    assert!(session.active().unwrap().history.can_redo());
    assert_eq!(session.journal.len(), journal);
    assert!(session.active_job().is_none());
    session.execute("edit.redo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&selected));
}
