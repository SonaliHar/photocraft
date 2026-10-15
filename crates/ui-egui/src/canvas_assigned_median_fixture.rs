//! Two actual assigned Median originals; native/profile words share the
//! independently measured Box input palette, not an inferred display oracle.
#[path = "canvas_assigned_box_fixture.rs"]
#[allow(dead_code)]
mod original;
pub(super) fn ctl() -> photocraft_raster::backing::ReadControl<'static> {
    original::ctl()
}
pub(super) fn observed(doc: &photocraft_doc::Document) {
    original::observed(doc)
}
pub(super) fn exact(a: &photocraft_doc::Document, b: &photocraft_doc::Document) {
    original::exact(a, b)
}
pub(super) fn archive(owner: &photocraft_doc::accounted_document::AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    original::archive(owner)
}
pub(super) fn cases() -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("median-raster-color-evidence");
    [
        ("source", "60540a9d83f0c1998ab4bbf6796028b456698973026d942ae880b050412dad94"),
        ("edit", "c41d90e4c9ef73874c03ba52a80f173b3b558a1740971aff4c84a4abb49fc7cd"),
    ]
    .into_iter()
    .map(|(phase, hash)| {
        let path = root.join(format!("assigned-unit-f32.{phase}.xcf"));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(original::hash(&bytes), hash);
        let native = std::fs::read(root.join(format!("assigned-unit-f32.{phase}.reopened.layer.buffer-extent.native.none.bin"))).unwrap();
        assert_eq!(original::hash(&native), "3102cfa5ba9eb6cf718b767257d474fc865e0b5fa93e8b3622adeebd7fefaadf");
        (path, bytes)
    })
    .collect()
}
