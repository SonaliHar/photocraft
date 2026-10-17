//! Selective linked current geometry floor, immutable source, and owned load parity.
use photocraft_doc::{
    Color, ColorMode, Document, SampleType, Size,
    item_identity::{ItemRef, ResourceId},
    xcf_items::{EditableSource, LinkedCurrentState, SourceCache, SourceItem, TransformState},
    xcf_resources::{ResourceFidelity, ResourceKind, ResourceReference, ResourceResolution},
    xcf_state::XcfState,
};
use photocraft_format::{
    SaveOptions, load_from_bytes, save_to_bytes,
    zip::{ZipReader, ZipWriter},
};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{BackingError, ReadControl, quota::*},
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn document(current: bool, transformed: bool) -> Document {
    let mut doc = Document::with_background("linked current native", Size::new(2, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let owner = ItemRef::Layer(doc.layers[0].id);
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    if current {
        state.items.iter_mut().find(|item| item.item == owner).unwrap().raster_bounds = Some(Rect::new(0, 0, 2, 1));
    }
    let resource_bytes = b"own verified linked resource payload".to_vec();
    let resource_hash = blake3::hash(&resource_bytes).to_hex().to_string();
    state.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "inert original reference".into(),
        collection: None,
        internal: false,
        locator: Some("inert/source-image".into()),
        content_hash: current.then_some(resource_hash.clone()),
        owned_bytes: current.then(|| resource_bytes.into()),
        resolution: current.then_some(ResourceResolution { fidelity: ResourceFidelity::Exact, payload_hash: resource_hash }),
    });
    let transform = TransformState { matrix_f64_bits: [1., -0., 0., 0., 1., 0., 0., 0., 1.].map(f64::to_bits), interpolation: 0, pre_offset: [0, 0] };
    let bytes = vec![11, 37, 59, 255, 17, 41, 61, 255];
    state.source_items.push(SourceItem {
        owner,
        source: EditableSource::Linked {
            resource: ResourceId(1),
            last_verified_hash: None,
            configuration: vec![],
            current: current.then_some(LinkedCurrentState { transform: transformed.then_some(transform.clone()), bounds: Rect::new(0, 0, 2, 1) }),
        },
        transform: Some(transform),
        cache: Some(SourceCache { blake3: blake3::hash(&bytes).to_hex().to_string(), bytes: bytes.into(), width: 2, height: 1, format: doc.pixel_format() }),
    });
    doc.xcf_state = Some(state);
    doc
}
fn forged_floor(bytes: &[u8], floor: u32) -> Vec<u8> {
    let reader = ZipReader::new(bytes).unwrap();
    let mut writer = ZipWriter::new();
    for entry in &reader.entries {
        let mut payload = reader.read(entry, 16 << 20).unwrap();
        if entry.name == "manifest.json" {
            let mut wire: serde_json::Value = serde_json::from_slice(&payload).unwrap();
            wire["minimum_reader_version"] = floor.into();
            payload = serde_json::to_vec(&wire).unwrap();
        }
        writer.add(&entry.name, &payload).unwrap();
    }
    writer.finish().unwrap()
}
#[derive(Debug)]
struct Archive(Vec<u8>, BackingQuota);
impl photocraft_format::canonical_archive::ImmutableCanonicalArchive for Archive {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.1.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(out.len() as u64)?;
        let at = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = at.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.0.get(at..end).ok_or(BackingError::Io)?);
        Ok(())
    }
}
fn owned_load(bytes: Vec<u8>, q: &BackingQuota) -> photocraft_format::Result<photocraft_format::owned_document::OwnedDocumentArchive> {
    photocraft_format::owned_document::load_document_from_owned_archive(
        std::sync::Arc::new(Archive(bytes, q.clone())),
        &Default::default(),
        Default::default(),
        q.clone(),
        &ctl(),
    )
}
#[test]
fn current_geometry_requires_seven_without_changing_original_transform_cache_or_locator() {
    for transformed in [false, true] {
        let doc = document(true, transformed);
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        let wire = photocraft_format::read_manifest(&bytes).unwrap();
        assert_eq!((wire.format_version, wire.minimum_reader_version), (photocraft_format::FORMAT_VERSION, 7));
        assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    }
}
#[test]
fn absent_current_keeps_reader_three_and_original_serialized_shape() {
    let doc = document(false, false);
    let source = &doc.xcf_state.as_ref().unwrap().source_items[0].source;
    let wire = serde_json::to_value(source).unwrap();
    assert!(wire["Linked"].get("current").is_none());
    let legacy: EditableSource = serde_json::from_value(wire).unwrap();
    assert_eq!(&legacy, source);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(photocraft_format::read_manifest(&bytes).unwrap().minimum_reader_version, 3);
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}
#[test]
fn forged_reader_six_cannot_hide_current_geometry_in_standard_or_owned_load() {
    let bytes = forged_floor(&save_to_bytes(&document(true, true), &Default::default()).unwrap(), 6);
    assert!(load_from_bytes(&bytes).is_err());
    let q = BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 64 << 20, index: 16 << 20 });
    let input = q.reserve(QuotaKind::Resident, bytes.capacity() as u64).unwrap();
    assert!(owned_load(bytes, &q).is_err());
    drop(input);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn accounted_writer_and_loader_preserve_current_and_source_at_selective_floor_seven() {
    for transformed in [false, true] {
        let doc = document(true, transformed);
        let q = BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 64 << 20, index: 16 << 20 });
        let bytes = photocraft_format::owned_document::save_current_document_archive(&doc, std::io::Cursor::new(Vec::new()), Default::default(), &q, &ctl())
            .unwrap()
            .into_inner();
        assert_eq!(photocraft_format::read_manifest(&bytes).unwrap().minimum_reader_version, 7);
        let input = q.reserve(QuotaKind::Resident, bytes.capacity() as u64).unwrap();
        let loaded = owned_load(bytes, &q).unwrap();
        assert_eq!(loaded.document(), &doc);
        drop(loaded);
        drop(input);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
