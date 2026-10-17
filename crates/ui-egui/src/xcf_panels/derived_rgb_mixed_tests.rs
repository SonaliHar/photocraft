//! Deferred raw primitive request construction. Actual UI/native lifecycle owned separately.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_stock_rgb_test_support.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use fixture::Kind as K;
#[test]
#[ignore = "UNRUN cheap exactshape/rawF32BoolEnum0, passiveword/order +single args/enable/opacitymerge intent; syntheticCURRENT no Source/visual acceptance"]
fn primitive_requests_preserve_passive_words_and_order() {
    let owner = fixture::build();
    let d = owner.document();
    let n = fixture::node(d, K::Temperature);
    let p = photocraft_compose::color_temperature_mixed_parameters(&n.arguments).unwrap().raw_words();
    assert!(request(n, &[(TEMP[0], p[0]), (TEMP[1], p[1])], &[], n.enabled).is_none());
    let r = request(n, &[(TEMP[0], 6000f32.to_bits()), (TEMP[1], 8000f32.to_bits())], &[], !n.enabled).unwrap();
    assert_eq!(r.0, "xcf.filter.set");
    assert_eq!(r.1["arguments"].as_array().unwrap().len(), 2);
    assert_eq!(r.1["enabled"], !n.enabled);
    let n = fixture::node(d, K::Sepia);
    let p = photocraft_compose::sepia_mixed_parameters(&n.arguments).unwrap();
    assert!(request(n, &[("scale", p.raw_scale_word())], &[("srgb", p.srgb())], n.enabled).is_none());
    let r = request(n, &[("scale", 0.625f32.to_bits())], &[("srgb", !p.srgb())], n.enabled).unwrap();
    assert_eq!(r.1["arguments"][1]["value"], json!({"Bool":!p.srgb()}));
    let n = fixture::node(d, K::AlienRgb);
    let p = photocraft_compose::alien_map_rgb_mixed_parameters(&n.arguments).unwrap();
    let (f, phase) = p.raw_words();
    let k = p.keep();
    let nums = [(FREQUENCY[0], f[0]), (FREQUENCY[1], f[1]), (FREQUENCY[2], f[2]), (PHASE[0], phase[0]), (PHASE[1], phase[1]), (PHASE[2], phase[2])];
    let bools = [(KEEP[0], k[0]), (KEEP[1], k[1]), (KEEP[2], k[2])];
    assert!(request(n, &nums, &bools, n.enabled).is_none());
    let mut nums = nums;
    let mut bools = bools;
    for (_, v) in &mut nums {
        *v = (f32::from_bits(*v) + 0.25).to_bits()
    }
    for (_, v) in &mut bools {
        *v = !*v
    }
    let r = request(n, &nums, &bools, !n.enabled).unwrap();
    assert_eq!(r.1["arguments"].as_array().unwrap().len(), 9);
    assert!(!r.1["arguments"].as_array().unwrap().iter().any(|a| a["name"] == "color-model"));
    let r = super::super::opacity_request(n, Some(r), 0.375, true).unwrap();
    assert_eq!(r.1["opacityF64Bits"], "0x3fd8000000000000");
    assert_eq!(r.1["arguments"].as_array().unwrap().len(), 9);
}
