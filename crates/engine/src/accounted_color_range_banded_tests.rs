//! UNRUN production regression drafts; no runtime acceptance claimed.
use super::*;
use photocraft_color::PixelFormat;
use photocraft_doc::{BackedLayer, Document, Layer, SampleEncoding, accounted_document::AccountedDocument};
use photocraft_geom::Size;
use photocraft_raster::backing::{
    BackedRaster, NativePlaneDescriptor, NativePlaneStore,
    quota::{BackingQuota, QuotaLimits},
};
use std::sync::atomic::AtomicUsize;
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
fn fixture_depth(depth: SampleType) -> (Session, BackingQuota, Arc<AtomicUsize>) {
    let raw = [0.23456789f32, 0.5, 0.7, 1., 0.8, 0.125, 0.3, 0.5];
    let mut doc = Document::new("current", Size::new(2, 65), ColorMode::Rgb, depth);
    doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    let format = PixelFormat::new(ColorMode::Rgb, depth, true);
    let mut layer = Layer::raster("pixels", format);
    layer.surface_mut().unwrap().write_region(doc.bounds(), &raw.repeat(65));
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
#[test]
fn bands_crossing_multiple_boundaries_match_inline_gray8_and_repeat_is_noop() {
    let (mut banded, _, _) = fixture_depth(SampleType::F32);
    let (mut inline, _, _) = fixture_depth(SampleType::F32);
    let input = CapturedAccountedColorRange::capture(&banded, &params(), &crate::read_control_none()).unwrap();
    let ctx = JobCtx::new();
    let prepared = input.prepare(&ctx).unwrap();
    let result = prepared.apply(&mut banded, &ctx).unwrap();
    assert_eq!(result, crate::accounted_color_range::run(&mut inline, &params(), &crate::read_control_none()).unwrap());
    let current = banded.active().unwrap().doc.clone();
    assert_eq!(current.selection.as_ref().unwrap(), inline.active().unwrap().doc.selection.as_ref().unwrap());
    let revision = banded.active().unwrap().revision;
    let past = banded.active().unwrap().history.past_len();
    CapturedAccountedColorRange::capture(&banded, &params(), &crate::read_control_none()).unwrap().prepare(&ctx).unwrap().apply(&mut banded, &ctx).unwrap();
    assert!(current.shares_state(&banded.active().unwrap().doc));
    assert_eq!(banded.active().unwrap().revision, revision);
    assert_eq!(banded.active().unwrap().history.past_len(), past);
}
#[test]
fn cancelled_worker_and_stale_final_view_never_publish() {
    let (mut session, quota, reads) = fixture_depth(SampleType::F32);
    let before = session.active().unwrap().doc.clone();
    let input = CapturedAccountedColorRange::capture(&session, &params(), &crate::read_control_none()).unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    let ctx = JobCtx::new();
    ctx.cancel();
    assert!(matches!(input.prepare(&ctx), Err(EngineError::Cancelled)));
    assert!(before.shares_state(&session.active().unwrap().doc));
    let ctx = JobCtx::new();
    let prepared = CapturedAccountedColorRange::capture(&session, &params(), &crate::read_control_none()).unwrap().prepare(&ctx).unwrap();
    session.active_mut().unwrap().view_revision += 1;
    assert!(prepared.apply(&mut session, &ctx).is_err());
    assert!(before.shares_state(&session.active().unwrap().doc));
    assert!(!session.active().unwrap().history.can_undo());
    assert!(quota.usage().unwrap().resident > 0);
}
#[test]
fn journal_and_final_work_share_one_nonrefundable_counter() {
    let (session, _, reads) = fixture_depth(SampleType::F32);
    let captured = CapturedAccountedColorRange::capture(&session, &params(), &crate::read_control_none()).unwrap();
    let before = captured.work.0.load(Ordering::Relaxed);
    captured
        .with_control(&Interrupt::NONE, |ctl| {
            ctl.check(100)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(captured.work.0.load(Ordering::Relaxed), before + 100);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    captured
        .with_control(&Interrupt::NONE, |ctl| {
            ctl.check(LIMIT - before - 100)?;
            Ok(())
        })
        .unwrap();
    let ctx = JobCtx::new();
    assert!(captured.prepare(&ctx).is_err());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}
