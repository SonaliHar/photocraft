//! Test-only CURRENT re-epoch control: prospective model-copy fee plus retained
//! actual base model/native/profile owner. Not an independently authored source.
use photocraft_doc::{Document, accounted_document::AccountedDocument};
use photocraft_raster::{
    Interrupt,
    backing::{ReadControl, quota::QuotaKind},
};
use std::sync::Arc;
pub(super) const EPOCH: &str = "xcf22-observed-box-geometry-v1";
pub(super) fn with_raw(base: &AccountedDocument, raw: [i32; 3], ctl: &ReadControl<'_>) -> AccountedDocument {
    let fee = base.quota().reserve(QuotaKind::Index, 2 << 20).unwrap();
    ctl.check(2 << 20).unwrap();
    let mut document: Document = base.document().clone();
    let node = &mut document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation.schema_epoch = EPOCH.into();
    let c = node.composition.as_mut().unwrap();
    c.compatibility_epoch = EPOCH.into();
    c.stored_blend_space = raw[0];
    c.stored_composite_space = raw[1];
    c.stored_operator = raw[2];
    assert!(photocraft_io::retained_native_model_census(&document, Default::default(), &Interrupt::NONE).unwrap().retained_bytes() as u64 <= fee.bytes());
    // The clone has a distinct prepaid model; the real immutable native store,
    // profile, base model and their original receipts remain held together.
    if base.is_readonly_assigned_point_storage() {
        AccountedDocument::new_readonly_assigned_point_storage(document, Arc::new((base.clone(), fee)), base.quota().clone(), ctl).unwrap()
    } else {
        AccountedDocument::new_readonly_point_storage(document, Arc::new((base.clone(), fee)), base.quota().clone(), ctl).unwrap()
    }
}
pub(super) fn raw(doc: &Document) -> [i32; 3] {
    let c = doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].composition.as_ref().unwrap();
    [c.stored_blend_space, c.stored_composite_space, c.stored_operator]
}
pub(super) fn source_cases() -> Vec<(std::path::PathBuf, Vec<u8>)> {
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol3-box-geometry-gimp-20261016/author-v1/outputs");
    [
        ("aligned-r4", 89214, "8862b7d94c64c25755a01b741a0f55cbaaadf313698f289748af4fe179c9a16d"),
        ("aligned-r5", 89214, "15b7292d92c4ffc2743f0caaeef6657aeca70a3cef71929e957659ef19ea7221"),
        ("negative-x-r4", 89274, "425aba7dd99bb00860566c00783981643b5f24da3a52b12a28ddea7f2e0c6d9e"),
        ("negative-x-r5", 89274, "957c26510b6fcc2a98a66b8eeb6a5bb20c16d594bfa740290ed0ceacb206ba2d"),
        ("positive-r4", 89274, "8272989febb342eef70a219a0cde3a6b8e71a27e03090e53458831bd15100f34"),
        ("positive-r5", 89274, "f8591f039adf31576e8bbf40da6c60ae2c2adab6dc585da6319b8b1418334561"),
    ]
    .into_iter()
    .map(|(name, len, sha)| {
        let path = root.join(format!("{name}.xcf"));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), len);
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), sha);
        (path, bytes)
    })
    .collect()
}
