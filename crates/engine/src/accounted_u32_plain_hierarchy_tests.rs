//! Deferred U32 addition, separate from prior U8/U16 host contracts.
use super::*;
#[path = "accounted_plain_integer_hierarchy_test_support.rs"]
mod f;
use photocraft_doc::{LayerContent, SampleType, item_identity::ItemRef};
use serde_json::json;
#[test]
#[ignore = "UNRUN/uncompiled U32/350 generated hierarchy actual Source/native13/emptyanchor continuity, lowbit native codes, nested metadata/disclosure/select/history and closed filter/pixel routes. No producer oracle."]
fn u32_hierarchy_host_keeps_exact_native_codes_and_purpose() {
    let input = f::import(2);
    let photocraft_io::OwnedXcfImport::NormalHierarchy(source) = &input else { panic!("hierarchy source") };
    let mut s = Session::new();
    s.add_owned_normal_hierarchy_xcf_import(source, "u32-hierarchy.xcf", Some("authorized/u32-hierarchy.xcf"), &f::ctl()).unwrap();
    let original = s.active().unwrap().doc.clone();
    f::observed(&original, 2);
    assert_eq!(original.depth, SampleType::U32);
    let (_, leaf, _) = f::ids(&original);
    let LayerContent::BackedRaster(r) = &original.layer(leaf).unwrap().content else { panic!("native") };
    let mut pixel = [0; 16];
    r.checked_view().read_logical_pixel_native_into(0, 0, &mut pixel, &f::ctl()).unwrap();
    let red = u32::from_ne_bytes(pixel[..4].try_into().unwrap());
    assert!(red >= 0x80000000);
    let value = f64::from(red) / f64::from(u32::MAX);
    assert_ne!(value.to_bits(), f64::from(value as f32).to_bits());
    let anchored = f::anchored(original.accounted().unwrap());
    let mut s = Session::new();
    s.add_owned_native_normal_hierarchy_archive(f::archive(&anchored), "u32.pcraft", None, &f::ctl()).unwrap();
    let (group, leaf, empty) = f::ids(&s.active().unwrap().doc);
    let source_id = s.active().unwrap().source.clone();
    for layer in [leaf, group, empty] {
        s.execute("layer.select", json!({"layer":layer.0})).unwrap();
        assert_eq!(s.active().unwrap().active_layer, Some(layer));
    }
    let rev = s.active().unwrap().revision;
    let past = s.active().unwrap().history.past_len();
    let journal = s.journal.len();
    s.execute("layer.setExpanded", json!({"layer":group.0,"expanded":false})).unwrap();
    assert_eq!(s.active().unwrap().revision, rev);
    assert_eq!(s.active().unwrap().history.past_len(), past);
    assert_eq!(s.journal.len(), journal);
    s.execute("layer.setProps", json!({"layer":group.0,"name":"U32 group","opacity":0.375,"visible":false})).unwrap();
    let changed = s.active().unwrap().doc.clone();
    f::observed(&changed, 2);
    f::payload(&original, &changed);
    assert_eq!(changed.layer(group).unwrap().opacity.to_bits(), 0.375f32.to_bits());
    assert!(!changed.layer(group).unwrap().visible);
    assert!(s.undo_with_control(&f::ctl()).unwrap());
    assert!(s.redo_with_control(&f::ctl()).unwrap());
    let snapshot = s.active().unwrap().doc.clone();
    assert_eq!(s.active().unwrap().source, source_id);
    let query = crate::accounted_xcf_inspect::prepare(&s, &json!({}), &f::ctl()).unwrap();
    assert!(query.value().is_object());
    drop(query);
    let stack = snapshot.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.owner == ItemRef::Layer(leaf)).unwrap();
    assert_eq!(stack.revision, 7);
    assert!(stack.nodes.is_empty());
    assert!(snapshot.xcf_state.as_ref().unwrap().filter_masks.is_empty());
    for (id, p) in [
        ("xcf.filter.set", json!({"filterId":"1","exposure":1.0})),
        ("xcf.filter.enable", json!({"filterId":"1","enabled":false})),
        ("xcf.filter.move", json!({"filterId":"1","index":0})),
        ("xcf.filter.remove", json!({"filterId":"1"})),
        ("xcf.filter.insert", json!({})),
        ("paint.stroke", json!({"points":[[1,1,1]]})),
        ("select.all", json!({})),
    ] {
        assert!(s.execute(id, p).is_err());
        assert!(snapshot.shares_state(&s.active().unwrap().doc));
    }
    let archive = f::archive(snapshot.accounted().unwrap());
    assert_eq!(archive.current_storage_hint(), Some(photocraft_format::NativeCurrentStorageHint::NormalHierarchy));
    let mut reopen = Session::new();
    reopen.add_owned_native_normal_hierarchy_archive(archive, "continued.pcraft", None, &f::ctl()).unwrap();
    let after = &reopen.active().unwrap().doc;
    f::same(&snapshot, after);
    f::observed(after, 2);
    assert!(after.accounted().unwrap().is_readonly_normal_hierarchy_storage());
}
