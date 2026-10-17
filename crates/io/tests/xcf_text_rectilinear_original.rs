#![cfg(feature = "corpus")]
use sha2::{Digest, Sha256};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
fn root() -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-rectilinear-evidence");
    let bytes = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&bytes), "855fd8b448b8190b0cd035e76ce1c503cb98aafa08f634f5ae4002893eca0a4a");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&bytes).unwrap();
    for (name, expected) in files {
        assert_eq!(hash(&std::fs::read(root.join(&name)).unwrap()), expected, "{name}");
    }
    root
}
fn projection(doc: &photocraft_doc::Document) -> Vec<u8> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(()))
        .unwrap()
        .px
        .iter()
        .flat_map(|p| p.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8))
        .collect()
}
#[test]
#[ignore = "requires exact-font original isolated GIMP text controls"]
fn nineteen_rectilinear_originals_current_regeneration_and_native() {
    let root = root();
    for (file, reference) in
        (0..15).map(|i| (format!("rectilinear-controls/control-{i}.xcf"), format!("rectilinear-controls/control-{i}.projection.bin"))).chain(
            ["shape-source", "shape-edit", "kern-source", "kern-edit"]
                .map(|s| (format!("rectilinear-pairs/{s}.xcf"), format!("rectilinear-pairs/{s}.projection.bin"))),
        )
    {
        let encoded = std::fs::read(root.join(file)).unwrap();
        let doc = photocraft_io::import("text.xcf", &encoded).unwrap().document;
        let expected = std::fs::read(root.join(reference)).unwrap();
        assert_eq!(projection(&doc), expected);
        let state = doc.xcf_state.as_ref().unwrap();
        assert_eq!(state.source_items.len(), 1);
        let source = &state.source_items[0];
        let cache = source.cache.as_ref().unwrap();
        cache.validate().unwrap();
        assert_eq!(
            doc.layers[0].surface().unwrap().try_read_interleaved(photocraft_geom::Rect::new(8, 4, 8 + cache.width as i32, 4 + cache.height as i32)).unwrap(),
            cache.bytes.as_slice()
        );
        let native = photocraft_io::export(&doc, "text.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("text.pcraft", &native.bytes).unwrap().document;
        assert_eq!(*photocraft_io::named_for_file(&doc, "text.pcraft"), reopened);
        assert_eq!(projection(&reopened), expected);
    }
}
