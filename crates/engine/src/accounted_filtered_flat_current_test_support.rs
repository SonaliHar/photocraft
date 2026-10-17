//! Test-only class-aware native10 writer/reader; encoded Vec is external harness output.
#![allow(dead_code, unused_imports)]
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "accounted_filtered_flat_remove_test_support.rs"]
mod old;
pub use old::{bytes, ctl, current, data, import, observed, preserved, same, single_root_three, storage, workspace};
use photocraft_doc::accounted_document::AccountedDocument;
pub fn encoded(owner: &AccountedDocument) -> Vec<u8> {
    photocraft_format::owned_document::save_accounted_current_document_archive(
        owner,
        std::io::Cursor::new(Vec::new()),
        Default::default(),
        owner.quota(),
        &ctl(),
    )
    .unwrap()
    .into_inner()
}
pub fn open(owner: &AccountedDocument, raw: &[u8]) -> photocraft_format::owned_document::OwnedDocumentArchive {
    photocraft_io::accounted_native::open_accounted_native_from_reader(
        &mut std::io::Cursor::new(raw),
        raw.len() as u64,
        Default::default(),
        owner.quota().clone(),
        &ctl(),
    )
    .unwrap()
}
pub fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let raw = encoded(owner);
    let a = open(owner, &raw);
    assert_eq!(a.current_storage_hint(), Some(photocraft_format::NativeCurrentStorageHint::FilteredFlatPoint));
    a
}
pub fn unmarked_archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    old::archive(owner)
}
