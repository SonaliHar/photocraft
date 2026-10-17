#![cfg(feature = "corpus")]
use photocraft_doc::xcf_items::EditableSource;
use sha2::{Digest, Sha256};
fn corpus() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/transform-interpolation")
}
fn import_preserves(kind: &str, oracle_hash: &str, edited: bool) {
    let name = if edited { format!("link-{kind}.edit.xcf") } else { format!("link-{kind}.xcf") };
    let phase = if edited { "edited" } else { "source" };
    let root = corpus();
    let source = std::fs::read(root.join(&name)).expect("frozen XCF corpus is required");
    let resource_before = std::fs::read(root.join("own-linked-resource.png")).unwrap();
    let imported = photocraft_io::import(&name, &source).unwrap();
    assert_eq!(serde_json::to_value(imported.assessment.as_ref().unwrap()).unwrap()["fidelity"], "dependency_limited");
    let report = imported.assessment.as_ref().unwrap();
    assert!(report.diagnostics.iter().any(|diagnostic| diagnostic.code == "missing_resource"));
    assert!(
        report
            .dependencies
            .iter()
            .all(|dependency| dependency.status == photocraft_io::xcf_assessment::DependencyStatus::Unresolved && !dependency.consumer_available)
    );
    let doc = &imported.document;
    let state = doc.xcf_state.as_ref().unwrap();
    let linked = state.source_items.iter().find(|source| matches!(source.source, EditableSource::Linked { .. })).unwrap();
    let photocraft_doc::item_identity::ItemRef::Layer(id) = linked.owner else { panic!("expected linked layer owner") };
    let bounds = state.items.iter().find(|item| item.item == linked.owner).unwrap().raster_bounds.unwrap();
    assert_eq!(bounds, if edited { photocraft_doc::Rect::new(5, -4, 93, 75) } else { photocraft_doc::Rect::new(2, -6, 90, 73) });
    let oracle = std::fs::read(root.join(format!("link-{kind}.{phase}.0.native.bin"))).unwrap();
    assert_eq!(Sha256::digest(&oracle).iter().map(|b| format!("{b:02x}")).collect::<String>(), oracle_hash);
    assert_eq!(doc.layer(id).unwrap().surface().unwrap().try_read_interleaved(bounds).unwrap(), oracle);
    assert_eq!(linked.cache.as_ref().unwrap().bytes.as_slice(), oracle);
    let EditableSource::Linked { resource, last_verified_hash, .. } = &linked.source else { panic!("expected linked source") };
    assert!(last_verified_hash.is_none());
    let dependency = state.resources.iter().find(|entry| entry.id == *resource).unwrap();
    assert!(dependency.owned_bytes.is_none());
    assert!(dependency.resolution.is_none());
    assert!(dependency.content_hash.is_none());
    photocraft_compose::gimp_link_items::qualify_original_cache(doc, linked, &photocraft_raster::Interrupt::NONE).unwrap();
    let unchanged = doc.clone();
    assert_eq!(photocraft_io::xcf_assessment::assess_xcf_document(doc, &photocraft_raster::Interrupt::NONE).unwrap().as_ref(), Some(report));
    assert_eq!(*doc, unchanged);
    let native = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *doc);
    assert_eq!(std::fs::read(root.join(&name)).unwrap(), source);
    assert_eq!(std::fs::read(root.join("own-linked-resource.png")).unwrap(), resource_before);
}
#[test]
fn other_link_interpolations_remain_refused_instead_of_approximated() {
    for kind in ["nohalo", "lohalo"] {
        let source = std::fs::read(corpus().join(format!("link-{kind}.xcf"))).unwrap();
        assert!(photocraft_io::import(&format!("link-{kind}.xcf"), &source).is_err());
    }
}

#[test]
fn nearest_link_original_import_preserves_independent_drawable_native_pixels_without_resolving_resource() {
    import_preserves("none", "ea6660ef7a5968a36bac5cad9ac9025f0ce7dce424c1f2763bd353464462e0e4", false);
}
#[test]
fn linear_link_original_import_preserves_independent_drawable_native_pixels_without_resolving_resource() {
    import_preserves("linear", "d064490959e917bbb4f6d21e525afdc8adcb899d3b8b421d039467d46f88b518", false);
}
#[test]
fn cubic_link_original_import_preserves_independent_drawable_native_pixels_without_resolving_resource() {
    import_preserves("cubic", "8197076dd709929004fc5f0d42dd7dfc895f1967eb97f719b93853d3a9d25ed0", false);
    import_preserves("cubic", "8197076dd709929004fc5f0d42dd7dfc895f1967eb97f719b93853d3a9d25ed0", true);
}

#[test]
fn translated_link_import_preserves_independent_native_placement_and_original_transform() {
    import_preserves("none", "ea6660ef7a5968a36bac5cad9ac9025f0ce7dce424c1f2763bd353464462e0e4", true);
    import_preserves("linear", "d064490959e917bbb4f6d21e525afdc8adcb899d3b8b421d039467d46f88b518", true);
}
