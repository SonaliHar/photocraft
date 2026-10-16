//! Deferred actual original Source-opacity after explicit release; no execution claim.
use super::*;
#[path = "../accounted_filtered_flat_source_opacity_test_support.rs"]
mod fixture;
#[test]
#[ignore = "UNRUN 8actualSources finiteopacity33→F64/disabled/rawidentity/currentnative10/query/undo; no syntheticCURRENT substitute or producer numeric oracle."]
fn genuine_source_opacity_adopts_and_continues() {
    for c in fixture::cases() {
        let input = fixture::import(&c);
        let photocraft_io::OwnedXcfImport::FilteredFlat(source) = &input else { panic!("source") };
        let path = fixture::path(&c);
        let mut s = Session::new();
        s.add_owned_filtered_flat_xcf_import(source, "original.xcf", path.to_str(), &fixture::ctl()).unwrap();
        let original = s.active().unwrap().doc.clone();
        fixture::observed(&original, &c);
        let node = &original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = node.id;
        let beforebits = node.opacity_f64_bits;
        let mut reopened = Session::new();
        reopened.add_owned_native_filtered_flat_archive(fixture::archive(original.accounted().unwrap()), "initial.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(&original, &reopened.active().unwrap().doc);
        let q = crate::accounted_filter_inspect::prepare(&s, &serde_json::json!({"filterId":id.0.to_string()}), &fixture::ctl()).unwrap();
        assert_eq!(q.value()["opacityF64Hex"], format!("0x{beforebits:016x}"));
        drop(q);
        s.execute("xcf.filter.set", serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":"0x3fe0000000000001"})).unwrap();
        let changed = s.active().unwrap().doc.clone();
        fixture::payload(&original, &changed);
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&original, &s.active().unwrap().doc);
        assert_eq!(fixture::bytes(&c), std::fs::read(path).unwrap());
    }
}
