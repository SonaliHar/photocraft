//! Implementation regression drafts; deliberately unrun in this wave.
use super::*;
use photocraft_doc::{BackedLayer, Document, Layer, LayerContent, SourceColorContext, accounted_document::AccountedDocument};
use photocraft_geom::Size;
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
    atomic::{AtomicU64, Ordering},
};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    native: Vec<u8>,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        control.check(output.len() as u64)?;
        let first = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let last = first.checked_add(output.len()).ok_or(BackingError::Length)?;
        output.copy_from_slice(self.native.get(first..last).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn fixture(depth: SampleType) -> (Session, BackingQuota) {
    imported_fixture(depth, false, false)
}
fn imported_fixture(depth: SampleType, imported: bool, filtered: bool) -> (Session, BackingQuota) {
    use photocraft_doc::{
        item_identity::ItemRef,
        xcf_state::{ItemState, XcfState},
    };
    let quota = BackingQuota::new(QuotaLimits { resident: 32 << 20, index: 8 << 20, scratch: 8 << 20 });
    let mut doc = Document::new("own accounted All draft", Size::new(3, 2), ColorMode::Rgb, depth);
    let format = PixelFormat::new(ColorMode::Rgb, depth, true);
    let descriptor = NativePlaneDescriptor::new(format, doc.bounds(), &vec![0; format.bytes_per_pixel()]).unwrap();
    let store = Store { descriptor, native: vec![0; 6 * format.bytes_per_pixel()] };
    let raster = BackedRaster::new(Arc::new(store), quota.clone(), 1, &control()).unwrap();
    let mut layer = Layer::raster("native pixels", format);
    layer.content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control()).unwrap());
    let reference = ItemRef::Layer(layer.id);
    doc.layers.push(layer);
    if imported {
        let mut state = XcfState::default();
        state.provenance.format = "xcf".into();
        state.provenance.format_version = 22;
        state.provenance.container = "synthetic imported metadata".into();
        state.provenance.source_label = "retained draft evidence".into();
        state.items.push(ItemState {
            item: reference,
            name: "native pixels".into(),
            custom_name: false,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: Some(17),
            source_flags: 0,
            raster_bounds: Some(doc.bounds()),
            mask: None,
            composition: None,
        });
        state.selection.selected.push(reference);
        state.selection.active = Some(reference);
        if filtered {
            state.filter_stacks.push(photocraft_doc::xcf_filters::FilterStack { owner: reference, nodes: vec![], revision: 1 });
        }
        doc.xcf_state = Some(state);
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        // Inert independently retained source context, not a rendering grant.
        doc.source_color_context =
            Some(SourceColorContext { original_icc: Some(photocraft_cms::Builtin::DisplayP3.profile().to_bytes()), source_precision: 675 });
    }
    // Trusted fixture retention includes native Store bytes and the actual model.
    let held = quota.reserve(QuotaKind::Resident, 65536).unwrap();
    let owned = AccountedDocument::from_owned(doc, Arc::new(held), quota.clone(), &control()).unwrap();
    let mut session = Session::new();
    session.add_accounted_document(owned, None, &control()).unwrap();
    (session, quota)
}

#[test]
fn no_prior_selection_keeps_upstream_gray8_even_on_wide_document_and_noop_redo() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32, SampleType::U32, SampleType::F64] {
        let (mut session, _) = fixture(depth);
        run(&mut session, &Value::Null, &control()).unwrap();
        let all = session.active().unwrap().doc.clone();
        let selection = all.selection.as_ref().unwrap();
        assert_eq!(selection.format(), PixelFormat::GRAY8);
        assert_eq!(selection.default_bytes(), &[0]);
        assert_eq!(selection.try_read_interleaved_with(Rect::new(0, 0, 3, 2), &Interrupt::NONE).unwrap(), vec![255; 6]);
        let mut outside = [255];
        selection.try_read_pixel_bytes(3, 0, &mut outside).unwrap();
        assert_eq!(outside, [0]);
        session.replace_accounted_selection(AccountedSelectionTarget::capture(&session).unwrap(), "Deselect", None, &control()).unwrap();
        session.undo_with_control(&control()).unwrap();
        let revision = session.active().unwrap().revision;
        let past = session.active().unwrap().history.past_len();
        assert!(session.active().unwrap().history.can_redo());
        run(&mut session, &json!({}), &control()).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&all));
        assert_eq!(session.active().unwrap().revision, revision);
        assert_eq!(session.active().unwrap().history.past_len(), past);
        assert!(session.active().unwrap().history.can_redo());
    }
}
#[test]
fn existing_u32_f64_coverage_retains_format_with_zero_default_and_padded_tail() {
    for depth in [SampleType::U32, SampleType::F64] {
        let (mut session, quota) = fixture(depth);
        let format = PixelFormat::new(ColorMode::Grayscale, depth, false);
        let zero = vec![0; 6 * format.bytes_per_pixel()];
        let prior = AccountedSelection::try_from_native(
            format,
            Rect::new(0, 0, 3, 2),
            &vec![0; format.bytes_per_pixel()],
            &zero,
            CoverageDomain::FiniteUnit,
            &quota,
            &control(),
        )
        .unwrap();
        session.replace_accounted_selection(AccountedSelectionTarget::capture(&session).unwrap(), "Prior", Some(&prior), &control()).unwrap();
        run(&mut session, &Value::Null, &control()).unwrap();
        let doc = &session.active().unwrap().doc;
        let selected = doc.selection.as_ref().unwrap();
        assert_eq!(selected.format(), format);
        let (word, n) = unit(format).unwrap();
        assert_eq!(selected.try_read_interleaved_with(Rect::new(0, 0, 3, 2), &Interrupt::NONE).unwrap(), word[..n].repeat(6));
        let mut outside = vec![255; n];
        selected.try_read_pixel_bytes(3, 0, &mut outside).unwrap();
        assert_eq!(outside, vec![0; n]);
    }
}
#[test]
fn denial_cancel_and_stale_pair_preserve_snapshot_history_quota() {
    let (mut session, quota) = fixture(SampleType::U8);
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let usage = quota.usage().unwrap();
    let used = AtomicU64::new(0);
    let deny = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Err(BackingError::Work)
    };
    assert!(run(&mut session, &Value::Null, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    assert!(used.load(Ordering::Relaxed) > 0);
    assert_eq!(quota.usage().unwrap(), usage);
    let cancelled = Interrupt::cancel_only(&|| true);
    assert!(matches!(run(&mut session, &Value::Null, &ReadControl { interrupt: &cancelled, work: &|_| Ok(()) }), Err(EngineError::Cancelled)));
    let params = json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()});
    assert!(run(&mut session, &params, &control()).is_err());
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(session.active().unwrap().revision, revision);
    assert!(!session.active().unwrap().history.can_undo());
}

#[test]
fn imported_anonymous_all_deselect_and_history_preserve_exact_source_metadata() {
    let (mut session, _) = imported_fixture(SampleType::F32, true, false);
    let original = session.active().unwrap().doc.clone();
    session.execute(CMD, json!({})).unwrap();
    let all = session.active().unwrap().doc.clone();
    assert_eq!(all.selection_channel_id, None);
    assert_eq!(all.selection.as_ref().unwrap().format(), PixelFormat::GRAY8);
    assert_eq!(all.selection.as_ref().unwrap().try_read_interleaved(all.bounds()).unwrap(), vec![255; 6]);
    let past = session.active().unwrap().history.past_len();
    session.execute(CMD, json!({})).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&all));
    assert_eq!(session.active().unwrap().history.past_len(), past);
    session.execute("select.deselect", json!({})).unwrap();
    let deselected = session.active().unwrap().doc.clone();
    assert!(deselected.selection.is_none());
    for state in [&all, &deselected] {
        assert_eq!(state.xcf_state, original.xcf_state);
        assert_eq!(state.source_color_context, original.source_color_context);
        assert_eq!(state.icc_profile, original.icc_profile);
        assert_eq!(state.selection_channel_id, None);
        assert!(Arc::ptr_eq(state.icc_profile.as_ref().unwrap(), original.icc_profile.as_ref().unwrap()));
        assert!(Arc::ptr_eq(
            state.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
            original.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
        ));
    }
    session.execute("edit.undo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&all));
    session.execute("edit.undo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&original));
    session.execute("edit.redo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&all));
    session.execute("edit.redo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&deselected));
}
#[test]
fn unsupported_imported_filter_family_refuses_through_paid_gate_without_publication() {
    let (mut session, quota) = imported_fixture(SampleType::F32, true, true);
    let original = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let usage = quota.usage().unwrap();
    // Cheap routing is deliberately not ownership/source qualification.
    validate_request(&session, &json!({})).unwrap();
    assert!(run(&mut session, &json!({}), &control()).is_err());
    assert!(session.active().unwrap().doc.shares_state(&original));
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(quota.usage().unwrap(), usage);
    assert!(!session.active().unwrap().history.can_undo());
}
