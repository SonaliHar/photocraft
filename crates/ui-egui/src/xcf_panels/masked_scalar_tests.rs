//! Deferred request-builder scope only; no widget interaction or visual acceptance claim.
use super::*;
#[path = "../../../engine/src/accounted_masked_scalar_test_support.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
#[test]
#[ignore = "UNRUN genuine16 retained nodes, passive raw words/source order, combined changed tuple+enable, zeroarg enable only and invalid complete Levels interval; request builder only"]
fn masked_scalar_requests_preserve_passive_words() {
    for case in fixture::cases() {
        let input = fixture::import(&case);
        let owner = input.document();
        let d = owner.document();
        let node = &d.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let kind = photocraft_engine::xcf_filter_cmds::masked_scalar_point_edit_kind(owner, node).unwrap();
        let (mut values, intent) = match kind {
            K::BrightnessContrast => (words(node, BC).unwrap().map(f32::from_bits).to_vec(), vec![false; 2]),
            K::GeglLevels => (words(node, LEVELS).unwrap().map(f32::from_bits).to_vec(), vec![false; 4]),
            _ => (vec![], vec![]),
        };
        assert!(request(node, kind, &values, &intent, node.enabled).is_none());
        let (id, params) = request(node, kind, &values, &intent, !node.enabled).unwrap();
        assert_eq!(id, "xcf.filter.enable");
        assert!(params.get("arguments").is_none());
        assert!(params.get("opacityF64Bits").is_none());
        if values.is_empty() {
            continue;
        }
        let mut intent = intent;
        values.fill(0.25);
        if kind == K::GeglLevels {
            values.copy_from_slice(&[0.0625, 0.9375, 0.125, 0.875]);
        }
        intent.fill(true);
        // Changed local values without actual widget intent still must not rewrite retained words.
        assert!(request(node, kind, &values, &vec![false; values.len()], node.enabled).is_none());
        let (id, params) = request(node, kind, &values, &intent, !node.enabled).unwrap();
        assert_eq!(id, "xcf.filter.set");
        assert_eq!(params["enabled"], !node.enabled);
        let args = params["arguments"].as_array().unwrap();
        assert_eq!(args.len(), node.arguments.len());
        assert_eq!(args.iter().map(|a| a["name"].as_str().unwrap()).collect::<Vec<_>>(), node.arguments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>());
        assert!(params.get("opacityF64Bits").is_none());
        if kind == K::GeglLevels {
            values[0] = 1.0;
            assert!(request(node, kind, &values, &intent, false).is_none());
        }
    }
}
