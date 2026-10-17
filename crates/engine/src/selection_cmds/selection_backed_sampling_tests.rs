use super::*;
use crate::selection_cmds::BackedColorRangeSamples;
use photocraft_doc::{BackedLayer, ColorMode, LayerContent, SampleType, accounted_document::AccountedDocument};
use photocraft_geom::Size;
use photocraft_raster::backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits};
use std::sync::atomic::{AtomicBool, AtomicUsize};
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    reads: Arc<AtomicUsize>,
    cancel: Option<Arc<AtomicBool>>,
    fault: bool,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], control: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        control.check(out.len() as u64)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        if self.fault {
            return Err(BackingError::Io);
        }
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Bounds)?);
        if let Some(stop) = &self.cancel {
            stop.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}
fn fixture(resident: u64, cancel: Option<Arc<AtomicBool>>, fault: bool) -> (Session, Document, BackingQuota, Arc<AtomicUsize>) {
    fixture_profile(resident, cancel, fault, true)
}
fn fixture_profile(resident: u64, cancel: Option<Arc<AtomicBool>>, fault: bool, assigned: bool) -> (Session, Document, BackingQuota, Arc<AtomicUsize>) {
    let raw = [0.234_567_9_f32, 0.5, 0.7, 1.0, 0.8, 0.125, 0.3, 0.5];
    let mut doc = Document::new("explicit current", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
    if assigned {
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    }
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    let mut ordinary = Layer::raster("pixels", PixelFormat::RGBA32F);
    ordinary.surface_mut().unwrap().write_region(doc.bounds(), &raw);
    doc.layers.push(ordinary);
    let resident_doc = doc.clone();
    let quota = BackingQuota::new(QuotaLimits { resident, index: 4 << 20, scratch: 0 });
    let reads = Arc::new(AtomicUsize::new(0));
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA32F, doc.bounds(), &[0u8; 16]).unwrap();
    let raster = BackedRaster::new(
        Arc::new(Store { descriptor, bytes: raw.map(f32::to_ne_bytes).concat(), reads: Arc::clone(&reads), cancel, fault }),
        quota.clone(),
        1,
        &crate::read_control_none(),
    )
    .unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &crate::read_control_none()).unwrap());
    let retention = quota.reserve(QuotaKind::Resident, 65536).unwrap();
    let owner = AccountedDocument::from_owned(doc, Arc::new(retention), quota.clone(), &crate::read_control_none()).unwrap();
    let mut session = Session::new();
    session.add_accounted_document(owner, None, &crate::read_control_none()).unwrap();
    (session, resident_doc, quota, reads)
}
fn params() -> Value {
    json!({"sampleAllLayers":false,"points":[[0,0]],"fuzziness":0})
}
#[test]
fn real_original_query_and_pixels_match_legacy_narrow_math_without_publication() {
    let (session, ordinary, _, reads) = fixture(64 << 20, None, false);
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let mut old = Session::new();
    old.add_document(ordinary, None).unwrap();
    let expected = ColorRangeSamples::new_with_interrupt(&old, &params(), &Interrupt::NONE).unwrap();
    let query = BackedColorRangeSamples::new_with_interrupt(&session, &params(), &Interrupt::NONE).unwrap();
    assert_eq!(query.query.range.lo.map(f32::to_bits), expected.range.lo.map(f32::to_bits));
    assert_eq!(query.query.range.hi.map(f32::to_bits), expected.range.hi.map(f32::to_bits));
    let pixels = query.backed_pixels_with_interrupt(Rect::new(0, 0, 2, 1), &Interrupt::NONE).unwrap();
    let mut raw = [[0.0; 4]; 2];
    old.active().unwrap().doc.layers[0].surface().unwrap().read_rgba_into(Rect::new(0, 0, 2, 1), &mut raw);
    assert_eq!(pixels.pixels().iter().map(|p| p.map(f32::to_bits)).collect::<Vec<_>>(), raw.map(|p| p.map(f32::to_bits)));
    let result = query.coverage_backed_pixels_with_interrupt(&pixels, Default::default(), &Interrupt::NONE).unwrap();
    let expected = expected.coverage_with_interrupt(&raw, 2, 1.0, [0.0, 0.0], Default::default(), &Interrupt::NONE).unwrap();
    assert_eq!(result.values(), expected);
    assert!(ColorRangeSamples::new_with_interrupt(&session, &params(), &Interrupt::NONE).is_err());
    assert!(before.shares_state(&session.active().unwrap().doc));
    assert_eq!(revision, session.active().unwrap().revision);
    assert!(reads.load(Ordering::Relaxed) > 0);
}
#[test]
fn owned_outputs_keep_source_and_quota_after_query_and_session_drop() {
    let (session, _, quota, _) = fixture(64 << 20, None, false);
    let query = BackedColorRangeSamples::new_with_interrupt(&session, &params(), &Interrupt::NONE).unwrap();
    let pixels = query.backed_pixels_with_interrupt(Rect::new(0, 0, 2, 1), &Interrupt::NONE).unwrap();
    let coverage = query.coverage_backed_pixels_with_interrupt(&pixels, Default::default(), &Interrupt::NONE).unwrap();
    drop(query);
    drop(session);
    assert!(quota.usage().unwrap().resident > 0);
    assert_eq!(pixels.pixels().len(), 2);
    assert_eq!(coverage.values().len(), 2);
    drop(pixels);
    assert!(quota.usage().unwrap().resident > 0);
    drop(coverage);
    assert_eq!(quota.usage().unwrap().resident, 0);
}
#[test]
fn quota_fault_precancel_and_latecancel_preserve_original_snapshot_and_revision() {
    for (cap, fault, late) in [(1 << 20, false, false), (64 << 20, true, false), (64 << 20, false, true)] {
        let stop = Arc::new(AtomicBool::new(false));
        let (session, _, _, reads) = fixture(cap, late.then(|| Arc::clone(&stop)), fault);
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let cancelled = || stop.load(Ordering::Relaxed);
        assert!(BackedColorRangeSamples::new_with_interrupt(&session, &params(), &Interrupt::cancel_only(&cancelled)).is_err());
        assert!(before.shares_state(&session.active().unwrap().doc));
        assert_eq!(revision, session.active().unwrap().revision);
        if cap == 1 << 20 {
            assert_eq!(reads.load(Ordering::Relaxed), 0);
        }
    }
    let (session, _, _, reads) = fixture(64 << 20, None, false);
    assert!(matches!(BackedColorRangeSamples::new_with_interrupt(&session, &params(), &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}
#[test]
fn no_merge_no_absent_profile_alias_and_cross_query_pixel_owner_refusal() {
    let (unassigned, _, _, unassigned_reads) = fixture_profile(64 << 20, None, false, false);
    assert!(BackedColorRangeSamples::new_with_interrupt(&unassigned, &params(), &Interrupt::NONE).is_err());
    assert_eq!(unassigned_reads.load(Ordering::Relaxed), 0);
    let (mut session, _, _, reads) = fixture(64 << 20, None, false);
    assert!(BackedColorRangeSamples::new_with_interrupt(&session, &json!({"sampleAllLayers":true,"points":[[0,0]]}), &Interrupt::NONE).is_err());
    let query = BackedColorRangeSamples::new_with_interrupt(&session, &params(), &Interrupt::NONE).unwrap();
    let second = BackedColorRangeSamples::new_with_interrupt(&session, &params(), &Interrupt::NONE).unwrap();
    let pixels = second.backed_pixels_with_interrupt(Rect::new(0, 0, 2, 1), &Interrupt::NONE).unwrap();
    assert!(query.coverage_backed_pixels_with_interrupt(&pixels, Default::default(), &Interrupt::NONE).is_err());
    assert!(query.backed_pixels_with_interrupt(Rect::new(-1, 0, 1, 1), &Interrupt::NONE).is_err());
    let active = session.active_mut().unwrap();
    active.revision += 1;
    assert_eq!(query.backed_source_is_current(&session), Some(false));
    assert!(reads.load(Ordering::Relaxed) > 0);
}

#[test]
fn supplied_read_control_receives_each_stage_work_and_denial_precedes_reads() {
    let (session, _, quota, reads) = fixture(64 << 20, None, false);
    let before = quota.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(BackedColorRangeSamples::new_with_read_control(&session, &params(), &denied).is_err());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert_eq!(quota.usage().unwrap(), before);
    let total = AtomicU64::new(0);
    let charge = |n| {
        total.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    let query = BackedColorRangeSamples::new_with_read_control(&session, &params(), &control).unwrap();
    let prepared = total.load(Ordering::Relaxed);
    assert!(prepared > 0);
    let pixels = query.backed_pixels_with_read_control(Rect::new(0, 0, 2, 1), &control).unwrap();
    let sampled = total.load(Ordering::Relaxed);
    assert!(sampled > prepared);
    let coverage = query.coverage_backed_pixels_with_read_control(&pixels, Default::default(), &control).unwrap();
    assert!(total.load(Ordering::Relaxed) > sampled);
    assert_eq!(total.load(Ordering::Relaxed), query.query.backed.as_ref().unwrap().work.used.load(Ordering::Relaxed));
    let used = total.load(Ordering::Relaxed);
    query.charge_backed_preview_work_with_read_control(7, &control).unwrap();
    assert_eq!(total.load(Ordering::Relaxed), used + 7);
    let original = session.active().unwrap().doc.clone();
    assert!(query.coverage_backed_pixels_with_read_control(&pixels, Default::default(), &denied).is_err());
    assert!(session.active().unwrap().doc.shares_state(&original));
    assert_eq!(coverage.values().len(), 2);
}

#[test]
fn only_active_localization_debits_point_distance_work_on_the_same_counter() {
    // UNRUN: use the actual query/owned pixel path and a lower work bound that
    // admitted CMS alone but previously rejected inactive point-distance work.
    let (session, _, _, _) = fixture(64 << 20, None, false);
    let original = session.active().unwrap().doc.clone();
    let total = AtomicU64::new(0);
    let charge = |n| {
        total.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    let mut query =
        BackedColorRangeSamples::new_with_read_control(&session, &json!({"sampleAllLayers":false,"points":[[0,0],[0,0]],"fuzziness":40}), &ctl).unwrap();
    let pixels = query.backed_pixels_with_read_control(Rect::new(0, 0, 2, 1), &ctl).unwrap();
    // Reference preparation is external test accounting; actual coverage must
    // pay its own preparation on the retained parent every time.
    let base_work = {
        let plan = query.query.to_lab.prepare_rgb_lab_f32_with_control(Default::default(), &|| false).unwrap();
        coverage_work(&query.query, 2, Some((&plan, Default::default())), &Interrupt::NONE).unwrap().0
    };
    let preparation = photocraft_cms::Transform::RGB_LAB_F32_PREPARATION_WORK;
    let lower = sel::ColorRangeLimits { work_units: base_work, ..Default::default() };
    let before = total.load(Ordering::Relaxed);
    let actual = query.coverage_backed_pixels_with_read_control(&pixels, lower, &ctl).unwrap();
    assert_eq!(total.load(Ordering::Relaxed) - before, base_work + preparation);
    let expected = coverage_inner(&query.query, pixels.pixels(), 2, 1., [0., 0.], Default::default(), &Interrupt::NONE).unwrap();
    assert_eq!(actual.values(), expected);
    query.query.localized = Some(1.);
    let before = total.load(Ordering::Relaxed);
    assert!(query.coverage_backed_pixels_with_read_control(&pixels, lower, &ctl).is_err());
    assert_eq!(total.load(Ordering::Relaxed) - before, preparation);
    let before = total.load(Ordering::Relaxed);
    let localized_work = base_work + 2 * 2 * 48;
    let higher = sel::ColorRangeLimits { work_units: localized_work, ..Default::default() };
    query.coverage_backed_pixels_with_read_control(&pixels, higher, &ctl).unwrap();
    assert_eq!(total.load(Ordering::Relaxed) - before, localized_work + preparation);
    assert_eq!(total.load(Ordering::Relaxed), query.query.backed.as_ref().unwrap().work.used.load(Ordering::Relaxed));
    query.query.plus_at.clear();
    assert_eq!(active_localized_points(&query.query), 0);
    let before = total.load(Ordering::Relaxed);
    query.coverage_backed_pixels_with_read_control(&pixels, lower, &ctl).unwrap();
    assert_eq!(total.load(Ordering::Relaxed) - before, base_work + preparation);
    let cancelled = || true;
    let cancelled_ctl = ReadControl { interrupt: &Interrupt::cancel_only(&cancelled), work: &charge };
    let before = total.load(Ordering::Relaxed);
    assert!(matches!(query.coverage_backed_pixels_with_read_control(&pixels, lower, &cancelled_ctl), Err(EngineError::Cancelled)));
    assert_eq!(total.load(Ordering::Relaxed), before);
    assert!(session.active().unwrap().doc.shares_state(&original));
}
