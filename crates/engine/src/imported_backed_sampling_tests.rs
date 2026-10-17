//! UNRUN synthetic current-model imported sampling regressions, not source qualification.
use super::*;
use crate::Session;
use crate::selection_cmds::BackedColorRangeSamples;
use photocraft_color::ColorMode;
use photocraft_color::PixelFormat;
use photocraft_doc::{
    BackedLayer, Document, Layer, SampleEncoding, SourceColorContext, XcfState,
    accounted_document::AccountedDocument,
    import_compositing::{GimpComposition, ImportCompositing, ResolvedGimpComposition},
    xcf_state::{ItemLocks, ItemState},
};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore,
        quota::{BackingQuota, QuotaLimits},
    },
};
use serde_json::json;
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
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        c.check(out.len() as u64)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        let a = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let b = a.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(a..b).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn fixture(implicit: bool, change: impl FnOnce(&mut Document)) -> (Session, Arc<AtomicUsize>) {
    let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 32 << 20, scratch: 32 << 20 });
    let reads = Arc::new(AtomicUsize::new(0));
    let mut d = Document::new("own imported current", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
    let bytes = photocraft_cms::Builtin::Srgb.profile().to_bytes();
    d.icc_profile = Some(bytes.clone());
    d.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    d.source_color_context = Some(SourceColorContext { original_icc: if implicit { None } else { Some(bytes.clone()) }, source_precision: 650 });
    if implicit {
        d.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&bytes);
    }
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA32F, d.bounds(), &[0; 16]).unwrap();
    let raw = [0.25f32, 0.5, 0.75, 1., 0.8, 0.1, 0.2, 0.5].map(f32::to_ne_bytes).concat();
    let raster = BackedRaster::new(Arc::new(Store { descriptor, bytes: raw, reads: reads.clone() }), q.clone(), 1, &crate::read_control_none()).unwrap();
    let mut l = Layer::new("pixels", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &crate::read_control_none()).unwrap()));
    l.import_compositing = Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    let mut state = XcfState::default();
    state.items.push(ItemState {
        item: ItemRef::Layer(l.id),
        name: l.name.clone(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(42),
        source_flags: 0,
        raster_bounds: Some(descriptor.bounds()),
        mask: None,
        composition: Some(GimpComposition {
            compatibility_epoch: "xcf-22-normal".into(),
            source_mode: 28,
            legacy: false,
            stored_blend_space: 0,
            stored_composite_space: 0,
            stored_operator: 1,
            resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
            pass_through: false,
        }),
    });
    d.layers.push(l);
    d.xcf_state = Some(state);
    change(&mut d);
    let retained = Arc::new(q.reserve(QuotaKind::Resident, 65536).unwrap());
    let owned = AccountedDocument::from_owned(d, retained, q, &crate::read_control_none()).unwrap();
    let mut session = Session::new();
    session.add_accounted_document(owned, None, &crate::read_control_none()).unwrap();
    (session, reads)
}
fn params() -> serde_json::Value {
    json!({"sampleAllLayers":false,"points":[[0,0]],"fuzziness":0})
}
#[test]
fn both_exact_profile_branches_read_original_pixels_and_selection_history_preserves_evidence() {
    for implicit in [false, true] {
        let (mut session, reads) = fixture(implicit, |_| {});
        let before = session.active().unwrap().doc.clone();
        let query = BackedColorRangeSamples::new_with_read_control(&session, &params(), &crate::read_control_none()).unwrap();
        let pixels = query.backed_pixels_with_read_control(Rect::new(0, 0, 2, 1), &crate::read_control_none()).unwrap();
        assert_eq!(pixels.pixels()[0][0].to_bits(), 0.25f32.to_bits());
        assert!(reads.load(Ordering::Relaxed) > 0);
        let ctx = crate::jobs::JobCtx::new();
        crate::accounted_color_range_banded::CapturedAccountedColorRange::capture(&session, &params(), &crate::read_control_none())
            .unwrap()
            .prepare(&ctx)
            .unwrap()
            .apply(&mut session, &ctx)
            .unwrap();
        let after = &session.active().unwrap().doc;
        assert_eq!(after.source_color_context, before.source_color_context);
        assert_eq!(after.xcf_state, before.xcf_state);
        assert_eq!(after.icc_profile, before.icc_profile);
        assert_eq!(after.working_profile_interpretation, before.working_profile_interpretation);
        assert_eq!(after.layers, before.layers);
        assert!(session.undo_with_control(&crate::read_control_none()).unwrap());
        assert!(before.shares_state(&session.active().unwrap().doc));
    }
}
#[test]
fn half_context_profile_mismatch_and_tuple_contradiction_fail_before_backing_reads() {
    let cases: [fn(&mut Document); 3] = [
        |d| d.source_color_context = None,
        |d| {
            let mut wrong = d.icc_profile.as_ref().unwrap().as_ref().clone();
            wrong[50] ^= 1;
            d.icc_profile = Some(Arc::new(wrong));
        },
        |d| d.layers[0].import_compositing.as_mut().unwrap().blend_encoding = SampleEncoding::NonlinearProfile,
    ];
    for change in cases {
        let (session, reads) = fixture(false, change);
        assert!(BackedColorRangeSamples::new_with_read_control(&session, &params(), &crate::read_control_none()).is_err());
        assert_eq!(reads.load(Ordering::Relaxed), 0);
    }
    let (session, reads) = fixture(true, |d| d.working_profile_interpretation = WorkingProfileInterpretation::Icc);
    assert!(BackedColorRangeSamples::new_with_read_control(&session, &params(), &crate::read_control_none()).is_err());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}
#[test]
fn cancellation_before_profile_verification_never_reads_or_changes_source() {
    let (session, reads) = fixture(true, |_| {});
    let before = session.active().unwrap().doc.clone();
    let stop = Interrupt::cancel_only(&|| true);
    let c = ReadControl { interrupt: &stop, work: &|_| Ok(()) };
    assert!(matches!(BackedColorRangeSamples::new_with_read_control(&session, &params(), &c), Err(EngineError::Cancelled)));
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert!(before.shares_state(&session.active().unwrap().doc));
}
