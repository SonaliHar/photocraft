//! Immutable source payload sharing; no locator/evaluator admission is implied.
use photocraft_doc::{
    ColorMode, Document, PixelFormat, SampleType, Size, XcfState,
    item_identity::{ItemRef, ResourceId},
    xcf_items::{EditableSource, SourceCache, SourceItem},
    xcf_resources::{ResourceKind, ResourceReference, ResourceResolution},
};
use serde::Serialize;
use std::sync::Arc;
#[derive(Serialize)]
struct LegacyCache {
    bytes: Vec<u8>,
    blake3: String,
    width: u32,
    height: u32,
    format: PixelFormat,
}
#[derive(Serialize)]
struct LegacyResource {
    id: ResourceId,
    kind: ResourceKind,
    name: String,
    collection: Option<String>,
    internal: bool,
    locator: Option<String>,
    content_hash: Option<String>,
    owned_bytes: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<ResourceResolution>,
}
fn resource() -> ResourceReference {
    let bytes = vec![13, 29, 0, 255];
    ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "Owned inert payload".into(),
        collection: None,
        internal: false,
        locator: Some("inert/not-a-grant".into()),
        content_hash: Some(blake3::hash(&bytes).to_hex().to_string()),
        owned_bytes: Some(Arc::new(bytes)),
        resolution: None,
    }
}
fn cache() -> SourceCache {
    let bytes = vec![9, 8, 7, 255];
    SourceCache { blake3: blake3::hash(&bytes).to_hex().to_string(), bytes: Arc::new(bytes), width: 1, height: 1, format: PixelFormat::RGBA8 }
}
#[test]
fn arc_payloads_keep_exact_legacy_json_and_deserialize_owned_arrays() {
    let c = cache();
    let old = LegacyCache { bytes: c.bytes.as_ref().clone(), blake3: c.blake3.clone(), width: c.width, height: c.height, format: c.format };
    let encoded = serde_json::to_vec(&old).unwrap();
    assert_eq!(serde_json::to_vec(&c).unwrap(), encoded);
    let restored: SourceCache = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(restored, c);
    restored.validate().unwrap();
    let r = resource();
    let old = LegacyResource {
        id: r.id,
        kind: r.kind.clone(),
        name: r.name.clone(),
        collection: r.collection.clone(),
        internal: r.internal,
        locator: r.locator.clone(),
        content_hash: r.content_hash.clone(),
        owned_bytes: r.owned_bytes.as_ref().map(|bytes| bytes.as_ref().clone()),
        resolution: r.resolution.clone(),
    };
    let encoded = serde_json::to_vec(&old).unwrap();
    assert_eq!(serde_json::to_vec(&r).unwrap(), encoded);
    assert_eq!(serde_json::from_slice::<ResourceReference>(&encoded).unwrap(), r);
}
#[test]
fn document_clone_shares_payloads_and_explicit_test_cow_preserves_original() {
    let mut original = Document::with_background("Sharing", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, photocraft_doc::Color::WHITE);
    let owner = original.layers[0].id;
    original.xcf_state = Some(XcfState {
        resources: vec![resource()],
        source_items: vec![SourceItem {
            owner: ItemRef::Layer(owner),
            source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: vec![], current: None },
            transform: None,
            cache: Some(cache()),
        }],
        ..Default::default()
    });
    let mut cloned = original.clone();
    let old = original.xcf_state.as_ref().unwrap();
    let new = cloned.xcf_state.as_ref().unwrap();
    assert!(Arc::ptr_eq(&old.source_items[0].cache.as_ref().unwrap().bytes, &new.source_items[0].cache.as_ref().unwrap().bytes));
    assert!(Arc::ptr_eq(old.resources[0].owned_bytes.as_ref().unwrap(), new.resources[0].owned_bytes.as_ref().unwrap()));
    let state = cloned.xcf_state.as_mut().unwrap();
    Arc::make_mut(&mut state.source_items[0].cache.as_mut().unwrap().bytes)[0] = 42;
    Arc::make_mut(state.resources[0].owned_bytes.as_mut().unwrap())[0] = 99;
    assert_eq!(old.source_items[0].cache.as_ref().unwrap().bytes.as_slice(), [9, 8, 7, 255]);
    old.source_items[0].cache.as_ref().unwrap().validate().unwrap();
    assert_eq!(old.resources[0].owned_bytes.as_ref().unwrap().as_slice(), [13, 29, 0, 255]);
    assert_ne!(state.source_items[0].cache.as_ref().unwrap().bytes, old.source_items[0].cache.as_ref().unwrap().bytes);
    assert!(!Arc::ptr_eq(old.resources[0].owned_bytes.as_ref().unwrap(), state.resources[0].owned_bytes.as_ref().unwrap()));
}

#[derive(Serialize)]
struct LegacyOpaque {
    owner: Option<ItemRef>,
    namespace: String,
    identity: String,
    source_version: u32,
    order: u32,
    flags: u32,
    bytes: Vec<u8>,
    blake3: String,
    rendering_required: bool,
}
#[test]
fn opaque_clone_keeps_legacy_wire_and_original_raw_bytes_after_explicit_test_cow() {
    use photocraft_doc::xcf_provenance::OpaqueRecord;
    let mut doc = Document::new("Opaque sharing", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let bytes = vec![0, 255, 17, 29];
    let record = OpaqueRecord {
        owner: None,
        namespace: "own".into(),
        identity: "inert-property".into(),
        source_version: 22,
        order: 3,
        flags: 7,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        bytes: Arc::new(bytes),
        rendering_required: false,
    };
    let old = LegacyOpaque {
        owner: record.owner,
        namespace: record.namespace.clone(),
        identity: record.identity.clone(),
        source_version: record.source_version,
        order: record.order,
        flags: record.flags,
        bytes: record.bytes.as_ref().clone(),
        blake3: record.blake3.clone(),
        rendering_required: record.rendering_required,
    };
    let json = serde_json::to_vec(&old).unwrap();
    assert_eq!(serde_json::to_vec(&record).unwrap(), json);
    assert_eq!(serde_json::from_slice::<OpaqueRecord>(&json).unwrap(), record);
    doc.xcf_state = Some(XcfState { opaque: vec![record], ..Default::default() });
    let mut copy = doc.clone();
    let original = &doc.xcf_state.as_ref().unwrap().opaque[0];
    let changed = &mut copy.xcf_state.as_mut().unwrap().opaque[0];
    assert!(Arc::ptr_eq(&original.bytes, &changed.bytes));
    Arc::make_mut(&mut changed.bytes)[0] = 42;
    assert_eq!(original.bytes.as_slice(), [0, 255, 17, 29]);
    assert_eq!(blake3::hash(original.bytes.as_slice()).to_hex().as_str(), original.blake3);
    assert!(!Arc::ptr_eq(&original.bytes, &changed.bytes));
    assert_eq!(original.namespace, changed.namespace);
    assert_eq!(original.flags, changed.flags);
    assert_eq!(original.source_version, changed.source_version);
    assert_eq!(original.rendering_required, changed.rendering_required);
    // Opaque validation remains uncached and refuses tampering/unknown rendering.
    assert!(copy.xcf_state.as_ref().unwrap().validate(&copy).is_err());
    doc.xcf_state.as_ref().unwrap().validate(&doc).unwrap();
    let mut required = doc.clone();
    required.xcf_state.as_mut().unwrap().opaque[0].rendering_required = true;
    assert_eq!(required.xcf_state.as_ref().unwrap().validate(&required).unwrap_err().code, "opaque_rendering_state");
}
