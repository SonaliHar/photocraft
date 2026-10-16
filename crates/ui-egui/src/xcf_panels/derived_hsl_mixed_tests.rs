//! Deferred fixed-HSL request shape and passive rawword contract; actual widget/native workflows separate.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_hsl_test_support.rs"]
mod fixture;
#[test]
#[ignore = "UNRUN HSLfixedmodel1 passiveF32Bool/Enum/order, changed9only andatomicopacity+enablemerge; wrongstoredRGBmodel rejected no normalization or visualclaim"]
fn fixed_hsl_requests_never_emit_model_switch() {
    let owner = fixture::build();
    let n = fixture::node(owner.document());
    assert!(kind(n) == Some(Kind::AlienHsl));
    let p = photocraft_compose::alien_map_hsl_mixed_parameters(&n.arguments).unwrap();
    let (f, phase) = p.raw_words();
    let k = p.keep();
    let mut numbers = [(FREQUENCY[0], f[0]), (FREQUENCY[1], f[1]), (FREQUENCY[2], f[2]), (PHASE[0], phase[0]), (PHASE[1], phase[1]), (PHASE[2], phase[2])];
    let mut bools = [(KEEP[0], k[0]), (KEEP[1], k[1]), (KEEP[2], k[2])];
    assert!(request(n, &numbers, &bools, n.enabled).is_none());
    for (_, v) in &mut numbers {
        *v = (f32::from_bits(*v) + 0.25).to_bits()
    }
    for (_, v) in &mut bools {
        *v = !*v
    }
    let r = request(n, &numbers, &bools, !n.enabled).unwrap();
    assert_eq!(r.1["arguments"].as_array().unwrap().len(), 9);
    assert!(!r.1["arguments"].as_array().unwrap().iter().any(|a| a["name"] == "color-model"));
    let r = super::super::opacity_request(n, Some(r), 0.375, true).unwrap();
    assert_eq!(r.1["opacityF64Bits"], "0x3fd8000000000000");
    assert_eq!(r.1["enabled"], !n.enabled);
    let mut bad = n.clone();
    let FilterValue::Enum { value, .. } = &mut bad.arguments[0].value else { panic!("model") };
    *value = 0;
    assert!(request(&bad, &numbers, &bools, !n.enabled).is_none());
}
