//! Source raster caches are exact inert blobs, independent of current pixels.
use photocraft_doc::item_identity::{ItemRef, ResourceId};
use photocraft_doc::xcf_items::{EditableSource, SourceCache, SourceItem};
use photocraft_doc::xcf_resources::{ResourceKind, ResourceReference};
use photocraft_doc::xcf_state::XcfState;
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use photocraft_format::{SaveOptions, load_from_bytes, save_to_bytes, zip::ZipReader};
fn document(sample: SampleType) -> Document {
    let mut doc = Document::with_background("Own cache", Size::new(2, 1), ColorMode::Rgb, sample, Color::WHITE);
    let owner = ItemRef::Layer(doc.layers[0].id);
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    state.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "Own inert source".into(),
        collection: None,
        internal: false,
        locator: Some("inert/source".into()),
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    });
    let format = doc.pixel_format();
    let bytes = (0..2 * format.bytes_per_pixel()).map(|i| (i * 37 + 11) as u8).collect::<Vec<_>>();
    state.source_items.push(SourceItem {
        owner,
        source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: vec![], current: None },
        transform: None,
        cache: Some(SourceCache { blake3: blake3::hash(&bytes).to_hex().to_string(), bytes: bytes.into(), width: 2, height: 1, format }),
    });
    doc.xcf_state = Some(state);
    doc
}
#[test]
fn aggregate_cache_validation_is_interruptible_and_preflights_before_hashes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut doc = document(SampleType::U8);
    let cache = doc.xcf_state.as_mut().unwrap().source_items[0].cache.as_mut().unwrap();
    cache.width = 512;
    cache.height = 512;
    cache.bytes = vec![73; 512 * 512 * 4].into();
    cache.blake3 = blake3::hash(&cache.bytes).to_hex().to_string();
    let before = doc.clone();
    let checks = AtomicUsize::new(0);
    let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 12;
    let err = doc.validate_xcf_state_with(&photocraft_raster::Interrupt::cancel_only(&cancel)).unwrap_err();
    assert_eq!(err.code, "cancelled");
    assert_eq!(doc, before);
    assert_eq!(doc.validate_xcf_state_with(&photocraft_raster::Interrupt::cancel_only(&|| true)).unwrap_err().code, "cancelled");
    doc.validate_xcf_state().unwrap();
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let checks = AtomicUsize::new(0);
    let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 10;
    assert!(matches!(
        photocraft_format::load_from_bytes_with_interrupt(&bytes, &Default::default(), &photocraft_raster::Interrupt::cancel_only(&cancel)),
        Err(photocraft_format::FormatError::Cancelled)
    ));
    assert_eq!(doc, before);
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    // A later invalid descriptor must fail before the earlier intentionally wrong payload hash.
    let state = doc.xcf_state.as_mut().unwrap();
    state.source_items[0].cache.as_mut().unwrap().blake3 = "0".repeat(64);
    let mut later = state.source_items[0].clone();
    std::sync::Arc::make_mut(&mut later.cache.as_mut().unwrap().bytes).clear();
    state.source_items.push(later);
    assert_eq!(doc.validate_xcf_state().unwrap_err().code, "cache_bounds");
}
#[test]
fn all_depth_cache_bytes_use_native_blob_and_require_new_reader() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let doc = document(sample);
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
        let zip = ZipReader::new(&bytes).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&zip.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap();
        assert_eq!(manifest["minimum_reader_version"], 3);
        let rows = manifest["document"]["xcf_source_caches"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        let cache = &doc.xcf_state.as_ref().unwrap().source_items[0].cache.as_ref().unwrap();
        assert_eq!(rows[0]["blob"], cache.blake3);
        assert_eq!(rows[0]["byte_len"], cache.bytes.len());
    }
}

mod common;
fn bundle(doc: &Document) -> std::path::PathBuf {
    let path = common::temp_dir("source-cache");
    photocraft_format::PcraftWriter::new().save_dir(doc, &path, &SaveOptions::default()).unwrap();
    path
}
fn edit_manifest(path: &std::path::Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let file = path.join("manifest.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    edit(&mut v);
    std::fs::write(file, serde_json::to_vec(&v).unwrap()).unwrap();
}
fn write_state(path: &std::path::Path, state: &XcfState) -> String {
    let bytes = serde_json::to_vec(state).unwrap();
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let encoded = ruzstd::encoding::compress_to_vec(bytes.as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
    std::fs::write(path.join("blobs").join(format!("{hash}.zst")), encoded).unwrap();
    hash
}
#[test]
fn legacy_inline_cache_migrates_and_ambiguous_inline_plus_blob_is_rejected() {
    let doc = document(SampleType::F64);
    let path = bundle(&doc);
    let hash = write_state(&path, doc.xcf_state.as_ref().unwrap());
    edit_manifest(&path, |v| v["document"]["xcf_state"] = hash.clone().into());
    assert!(photocraft_format::load_path(&path).is_err());
    edit_manifest(&path, |v| {
        v["format_version"] = 2.into();
        v["minimum_reader_version"] = 2.into();
        v["document"].as_object_mut().unwrap().remove("xcf_source_caches");
    });
    assert_eq!(photocraft_format::load_path(&path).unwrap(), doc);
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn cache_records_reject_downgrade_duplicates_bad_descriptors_and_missing_owners() {
    for mode in 0..6 {
        let path = bundle(&document(SampleType::U32));
        edit_manifest(&path, |v| match mode {
            0 => v["minimum_reader_version"] = 2.into(),
            1 => {
                let row = v["document"]["xcf_source_caches"][0].clone();
                v["document"]["xcf_source_caches"].as_array_mut().unwrap().push(row);
            }
            2 => v["document"]["xcf_source_caches"][0]["byte_len"] = usize::MAX.into(),
            3 => v["document"]["xcf_source_caches"][0]["width"] = 0.into(),
            4 => v["document"]["xcf_source_caches"][0]["owner"]["Layer"] = u64::MAX.into(),
            _ => v["document"]["xcf_source_caches"][0]["blob"] = "0".repeat(64).into(),
        });
        assert!(photocraft_format::load_path(&path).is_err(), "mode {mode}");
        std::fs::remove_dir_all(path).unwrap();
    }
}
#[test]
fn equal_cache_blobs_share_storage_but_keep_distinct_remapped_owners() {
    let mut doc = document(SampleType::F64);
    let layer = photocraft_doc::Layer::raster("Own second", doc.pixel_format());
    let owner = ItemRef::Layer(layer.id);
    doc.layers.push(layer);
    let mut state = doc.xcf_state.take().unwrap();
    state.reconcile_layers(&doc).unwrap();
    let mut source = state.source_items[0].clone();
    source.owner = owner;
    state.source_items.push(source);
    doc.xcf_state = Some(state);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let zip = ZipReader::new(&bytes).unwrap();
    let hash = &doc.xcf_state.as_ref().unwrap().source_items[0].cache.as_ref().unwrap().blake3;
    assert_eq!(zip.entries.iter().filter(|entry| entry.name == format!("blobs/{hash}.zst")).count(), 1);
    let restored = photocraft_format::load_from_bytes_with(&bytes, &photocraft_format::LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    assert_ne!(restored.layers[0].id, doc.layers[0].id);
    let sources = &restored.xcf_state.as_ref().unwrap().source_items;
    assert_ne!(sources[0].owner, sources[1].owner);
    assert_eq!(sources[0].owner, ItemRef::Layer(restored.layers[0].id));
    assert_eq!(sources[1].owner, ItemRef::Layer(restored.layers[1].id));
    assert_eq!(sources[0].cache, sources[1].cache);
}
#[test]
fn native_cache_decode_honors_cancellation_and_per_descriptor_expansion_bound() {
    let mut doc = document(SampleType::U8);
    let cache = doc.xcf_state.as_mut().unwrap().source_items[0].cache.as_mut().unwrap();
    cache.width = 500;
    cache.height = 200;
    cache.bytes = (0..400000).map(|i| (i * 37) as u8).collect::<Vec<_>>().into();
    cache.blake3 = blake3::hash(&cache.bytes).to_hex().to_string();
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 3;
    assert!(matches!(
        photocraft_format::load_from_bytes_with_interrupt(&bytes, &Default::default(), &photocraft_raster::Interrupt::cancel_only(&cancel)),
        Err(photocraft_format::FormatError::Cancelled)
    ));
    assert!(calls.load(std::sync::atomic::Ordering::SeqCst) > 3);
    let path = bundle(&document(SampleType::U8));
    let bomb = vec![9_u8; 1 << 20];
    let hash = blake3::hash(&bomb).to_hex().to_string();
    let compressed = ruzstd::encoding::compress_to_vec(bomb.as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
    std::fs::write(path.join("blobs").join(format!("{hash}.zst")), compressed).unwrap();
    edit_manifest(&path, |v| v["document"]["xcf_source_caches"][0]["blob"] = hash.into());
    assert!(matches!(photocraft_format::load_path(&path), Err(photocraft_format::FormatError::LimitExceeded(_))));
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn linked_configuration_defaults_and_nonempty_typed_state_requires_reader_three() {
    let mut doc = document(SampleType::U8);
    let source = &mut doc.xcf_state.as_mut().unwrap().source_items[0];
    source.cache = None;
    let EditableSource::Linked { configuration, .. } = &mut source.source else { panic!("own linked fixture") };
    configuration.push(photocraft_doc::xcf_filters::FilterArgument {
        name: "source-width".into(),
        value: photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Unsigned { width: 32, value: 2 }),
    });
    let bytes = save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    let path = bundle(&doc);
    edit_manifest(&path, |v| v["minimum_reader_version"] = 2.into());
    assert!(photocraft_format::load_path(&path).is_err());
    std::fs::remove_dir_all(path).unwrap();
    let legacy: EditableSource = serde_json::from_value(serde_json::json!({"Linked":{"resource":1,"last_verified_hash":null}})).unwrap();
    assert!(matches!(legacy,EditableSource::Linked{configuration,..} if configuration.is_empty()));
}

#[test]
fn aggregate_cache_budget_fails_before_any_cache_blob_is_read() {
    let mut doc = document(SampleType::U8);
    let mut state = doc.xcf_state.take().unwrap();
    state.source_items[0].cache = None;
    for _ in 0..4 {
        let layer = photocraft_doc::Layer::raster("Own budget owner", doc.pixel_format());
        let owner = ItemRef::Layer(layer.id);
        doc.layers.push(layer);
        let mut source = state.source_items[0].clone();
        source.owner = owner;
        state.source_items.push(source);
    }
    state.reconcile_layers(&doc).unwrap();
    doc.xcf_state = Some(state);
    let path = bundle(&doc);
    let rows = doc
        .xcf_state
        .as_ref()
        .unwrap()
        .source_items
        .iter()
        .map(|source| serde_json::json!({"owner":source.owner,"blob":"0".repeat(64),"byte_len":64<<20,"width":65536,"height":256,"format":doc.pixel_format()}))
        .collect::<Vec<_>>();
    edit_manifest(&path, |v| {
        v["format_version"] = 3.into();
        v["minimum_reader_version"] = 3.into();
        v["document"]["xcf_source_caches"] = rows.into();
    });
    // No corresponding blobs exist. The aggregate limit must win before missing-blob I/O.
    assert!(matches!(photocraft_format::load_path(&path), Err(photocraft_format::FormatError::LimitExceeded(_))));
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
#[ignore = "local timing evidence; scheduling is not a real-time guarantee"]
fn source_cache_checkpoint_timing() {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    let mut doc = document(SampleType::U8);
    let cache = doc.xcf_state.as_mut().unwrap().source_items[0].cache.as_mut().unwrap();
    cache.width = 2048;
    cache.height = 2048;
    cache.bytes = (0..(16 << 20)).map(|i| (i * 37 + i / 997) as u8).collect::<Vec<_>>().into();
    cache.blake3 = blake3::hash(&cache.bytes).to_hex().to_string();
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let observations = Mutex::new((Instant::now(), Duration::ZERO, 0_usize));
    let check = || {
        let now = Instant::now();
        let mut observed = observations.lock().unwrap();
        observed.1 = observed.1.max(now.duration_since(observed.0));
        observed.0 = now;
        observed.2 += 1;
        false
    };
    let start = Instant::now();
    let loaded = photocraft_format::load_from_bytes_with_interrupt(&bytes, &Default::default(), &photocraft_raster::Interrupt::cancel_only(&check)).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(loaded, doc);
    let observations = observations.lock().unwrap();
    println!("16MiB cache load {:?}; {} cancellation checks; largest observed gap {:?}", elapsed, observations.2, observations.1);
}
