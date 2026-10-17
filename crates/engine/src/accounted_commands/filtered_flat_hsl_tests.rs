//! Deferred fixed-HSL typed target proof. Synthetic CURRENT; no producer Source or HDR completeness claim.
use super::*;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "../accounted_filtered_flat_hsl_test_support.rs"]
mod fixture;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "../accounted_filtered_flat_stock_rgb_test_support.rs"]
mod rgb;
#[test]
#[ignore = "UNRUN HSL actualtarget all10 slots/model1 and onecompoundrevision, unchangedmodelnoop/fullOLDSTAGED, RGB0/HSL1 crosspatch bothdirections rejected, wrongdomain/type/11; no numerical acceptance"]
fn hsl_ten_slots_do_not_switch_model() {
    let owner = fixture::build();
    let mut s = Session::new();
    s.add_owned_native_filtered_flat_archive(fixture::archive(&owner), "hsl.pcraft", None, &fixture::ctl()).unwrap();
    let before = s.active().unwrap().doc.clone();
    let id = fixture::node(&before).id;
    let noop = serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"color-model","value":{"Enum":{"domain":"gegl:alien-map/color-model","value":1}}}]});
    s.execute("xcf.filter.set", noop).unwrap();
    assert!(s.active().unwrap().doc.shares_state(&before));
    let mut params = fixture::patch(&before);
    let expected = params["arguments"].as_array().unwrap().clone();
    assert_eq!(expected.len(), 10);
    let rev = s.active().unwrap().revision;
    params.as_object_mut().unwrap().insert("enabled".into(), serde_json::json!(false));
    params.as_object_mut().unwrap().insert("opacityF64Bits".into(), serde_json::json!("0x3fd8000000000000"));
    s.execute("xcf.filter.set", params).unwrap();
    assert_eq!(s.active().unwrap().revision, rev + 1);
    let changed = s.active().unwrap().doc.clone();
    let n = fixture::node(&changed);
    assert!(!n.enabled);
    assert_eq!(n.opacity_f64_bits, 0.375f64.to_bits());
    for want in &expected {
        let a = n.arguments.iter().find(|a| a.name == want["name"].as_str().unwrap()).unwrap();
        assert_eq!(serde_json::to_value(&a.value).unwrap(), want["value"]);
    }
    fixture::payload(&before, &changed);
    s.execute("edit.undo", serde_json::json!({})).unwrap();
    fixture::same(&before, &s.active().unwrap().doc);
    for (domain, value) in [("gegl:alien-map/color-model", 0), ("wrong/domain", 1)] {
        let mut p = fixture::patch(&before);
        p["arguments"][0]["value"] = serde_json::json!({"Enum":{"domain":domain,"value":value}});
        assert!(s.execute("xcf.filter.set", p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    let mut p = fixture::patch(&before);
    p["arguments"][1]["value"] = serde_json::json!({"Bool":true});
    assert!(s.execute("xcf.filter.set", p).is_err());
    let mut p = fixture::patch(&before);
    p["arguments"].as_array_mut().unwrap().push(serde_json::json!({"name":"unknown","value":{"Bool":true}}));
    assert!(s.execute("xcf.filter.set", p).is_err());
    let owner = rgb::build();
    let mut s = Session::new();
    s.add_owned_native_filtered_flat_archive(rgb::archive(&owner), "rgb.pcraft", None, &rgb::ctl()).unwrap();
    let before = s.active().unwrap().doc.clone();
    let n = rgb::node(&before, rgb::Kind::AlienRgb);
    let mut p = rgb::patch(&before, rgb::Kind::AlienRgb);
    p["arguments"][0]["value"] = serde_json::json!({"Enum":{"domain":"gegl:alien-map/color-model","value":1}});
    assert!(s.execute("xcf.filter.set", p).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(rgb::node(&before, rgb::Kind::AlienRgb).id, n.id);
}
