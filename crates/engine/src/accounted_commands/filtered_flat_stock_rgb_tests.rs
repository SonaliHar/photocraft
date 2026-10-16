//! Deferred target-aware primitive patch proof; synthetic CURRENT, not producer Source evidence.
use super::*;
#[path = "../accounted_filtered_flat_stock_rgb_test_support.rs"]
mod fixture;
use fixture::Kind;
#[test]
#[ignore = "UNRUN actual mixed target caps2/2/10 and allten slots including namedBool/Enum0, onecompoundrevision+opacity/enable/undo, wrongdomain/HSL/type/11/emptyrefusal; no mathoracle"]
fn target_aware_ten_patch_is_not_generic_permission() {
    for kind in [Kind::Temperature, Kind::Sepia, Kind::AlienRgb] {
        let owner = fixture::build();
        let mut s = Session::new();
        s.add_owned_native_filtered_flat_archive(fixture::archive(&owner), "stock.pcraft", None, &fixture::ctl()).unwrap();
        let before = s.active().unwrap().doc.clone();
        let old = fixture::node(&before, kind).clone();
        let mut params = fixture::patch(&before, kind);
        let expected = params["arguments"].as_array().unwrap().clone();
        let revision = s.active().unwrap().revision;
        params.as_object_mut().unwrap().insert("enabled".into(), serde_json::json!(false));
        params.as_object_mut().unwrap().insert("opacityF64Bits".into(), serde_json::json!("0x3fd8000000000000"));
        s.execute("xcf.filter.set", params).unwrap();
        let changed = s.active().unwrap().doc.clone();
        let node = fixture::node(&changed, kind);
        assert_eq!(s.active().unwrap().revision, revision + 1);
        assert!(!node.enabled);
        assert_eq!(node.opacity_f64_bits, 0.375f64.to_bits());
        assert_eq!(node.arguments.len(), expected.len());
        for want in &expected {
            let a = node.arguments.iter().find(|a| a.name == want["name"].as_str().unwrap()).unwrap();
            assert_eq!(serde_json::to_value(&a.value).unwrap(), want["value"]);
        }
        assert_eq!(
            before.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.nodes.iter().any(|n| n.id == old.id)).unwrap().revision + 1,
            changed.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.nodes.iter().any(|n| n.id == old.id)).unwrap().revision
        );
        fixture::payload(&before, &changed);
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&before, &s.active().unwrap().doc);
        let mut too_many = fixture::patch(&before, kind);
        let a = too_many["arguments"].as_array_mut().unwrap();
        for i in a.len()..11 {
            a.push(serde_json::json!({"name":format!("unknown-{i}"),"value":{"Bool":true}}));
        }
        assert!(s.execute("xcf.filter.set", too_many).is_err());
        assert!(s.execute("xcf.filter.set", serde_json::json!({"filterId":old.id.0.to_string(),"arguments":[],"enabled":true})).is_err());
        if kind == Kind::AlienRgb {
            for (domain, value) in [("gegl:alien-map/color-model", 1), ("wrong/domain", 0)] {
                let mut p = fixture::patch(&before, kind);
                p["arguments"][0]["value"] = serde_json::json!({"Enum":{"domain":domain,"value":value}});
                assert!(s.execute("xcf.filter.set", p).is_err());
            }
            let mut p = fixture::patch(&before, kind);
            p["arguments"][1]["value"] = serde_json::json!({"Bool":true});
            assert!(s.execute("xcf.filter.set", p).is_err());
        } else {
            let alien = fixture::patch(&before, Kind::AlienRgb);
            let mut p = alien;
            p["filterId"] = serde_json::json!(old.id.0.to_string());
            assert!(s.execute("xcf.filter.set", p).is_err());
        }
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
}
