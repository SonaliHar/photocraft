//! Paid synthetic CURRENT control; original source/projection evidence is separate.
#[path = "canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod native;
#[path = "canvas_observed_box_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod observed;
use photocraft_doc::{Document, SampleType, item_identity::ItemRef};
pub(super) fn ctl() -> photocraft_raster::backing::ReadControl<'static> {
    native::ctl()
}
pub(super) fn archive(owner: &photocraft_doc::accounted_document::AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    native::archive(owner)
}
pub(super) fn encoded(owner: &photocraft_doc::accounted_document::AccountedDocument) -> Vec<u8> {
    native::encoded(owner)
}
pub(super) fn owner(sample: SampleType, assigned: bool) -> photocraft_doc::accounted_document::AccountedDocument {
    let (base, _) = native::fixture(sample, assigned, 1);
    observed::with_raw(&base, [i32::MIN, 0, i32::MAX], &ctl())
}
pub(super) fn preserved(before: &Document, after: &Document) {
    assert_eq!(before.size, after.size);
    assert_eq!(before.icc_profile, after.icc_profile);
    assert_eq!(before.source_color_context, after.source_color_context);
    assert_eq!(before.working_profile_interpretation, after.working_profile_interpretation);
    assert_eq!(before.sample_encoding, after.sample_encoding);
    assert_eq!(native::native(before), native::native(after));
    let (photocraft_doc::LayerContent::BackedRaster(a), photocraft_doc::LayerContent::BackedRaster(b)) = (&before.layers[0].content, &after.layers[0].content)
    else {
        panic!("native")
    };
    assert_eq!(a.descriptor(), b.descriptor());
    assert_eq!(a.default_bytes(), b.default_bytes());
    assert_eq!(observed::raw(after), [i32::MIN, 0, i32::MAX]);
    let mut expected = before.xcf_state.clone().unwrap();
    let old = &before.layers[0];
    let current = &after.layers[0];
    for item in &mut expected.items {
        if item.item == ItemRef::Layer(old.id) {
            if old.name != current.name {
                item.custom_name = true;
            }
            item.name = current.name.clone();
            item.visible = current.visible;
        }
    }
    assert_eq!(after.xcf_state, Some(expected));
}
