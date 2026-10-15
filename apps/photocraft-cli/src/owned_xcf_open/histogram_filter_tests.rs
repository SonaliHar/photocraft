//! UNRUN CLI transport workflows over the same original sources whose SHA pins
//! and full padded native storage are checked by the engine companion drafts.
use super::*;
use serde_json::{Value, json};
fn corpus_case(id: &str) -> Vec<u8> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("UNRUN requires pinned threshold precision evidence"))
        .join("threshold-precision-evidence");
    let manifest: Value = serde_json::from_slice(&std::fs::read(root.join("SHA256SUMS.json")).unwrap()).unwrap();
    let name = format!("{id}.xcf");
    assert_eq!(manifest[&name].as_str().unwrap().len(), 64);
    assert!(std::fs::metadata(root.join(&name)).unwrap().len() <= 16 << 20);
    std::fs::read(root.join(name)).unwrap()
}
#[test]
#[ignore = "UNRUN: actual pinned Histogram Threshold source, CLI and native host paths"]
fn explicit_xcf_then_native_cli_filter_edit_history_save_and_reopen_uses_owned_routes() {
    for id in ["rgb-u16_non_linear.channel0", "rgb-float_non_linear.channel0", "rgb-double_non_linear.channel0"] {
        let dir = Directory::new();
        let source = dir.0.join("source.xcf");
        let saved = dir.0.join("edited.pcraft");
        let continued = dir.0.join("continued.pcraft");
        let original = corpus_case(id);
        std::fs::write(&source, &original).unwrap();
        let mut inspection = Headless::trusted_local();
        drop(inspection.open_owned_xcf_keep_current_bounded(&source).unwrap());
        let before = inspection.session.active().unwrap().doc.clone();
        assert!(before.accounted().unwrap().is_readonly_threshold_storage());
        let original_node = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
        let original_low = serde_json::to_value(original_node.arguments.iter().find(|a| a.name == "low").unwrap()).unwrap();
        let low_bits = if original_low["value"]["Number"]["Float32Bits"] == 0.875f32.to_bits() { 0.625f32.to_bits() } else { 0.875f32.to_bits() };
        let filter = original_node.id.0.to_string();
        let patch = json!({"filterId":filter,"arguments":[{"name":"low","value":{"Number":{"Float32Bits":low_bits}}},{"name":"high","value":{"Number":{"Float32Bits":0.125f32.to_bits()}}}]});
        let patch_text = patch.to_string();
        let enabled = json!({"filterId":filter,"enabled":!original_node.enabled}).to_string();
        let (code, results, error) = invoke(&[
            "run",
            source.to_str().unwrap(),
            "--xcf-profile-policy",
            "keepCurrent",
            "--cmd",
            "xcf.filter.set",
            "--params",
            &patch_text,
            "--cmd",
            "edit.undo",
            "--cmd",
            "edit.redo",
            "--cmd",
            "xcf.filter.enable",
            "--params",
            &enabled,
            "--out",
            saved.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{error}");
        assert!(results.contains("xcf.filter.set") && results.contains("edit.redo"));
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let mut reopened = Headless::trusted_local();
        let reply = reopened.open_owned_native_keep_current_bounded(&saved).unwrap();
        assert_eq!(reply.value()["nativeProfilePolicy"], "keepCurrent");
        drop(reply);
        let current = reopened.session.active().unwrap().doc.clone();
        assert!(current.accounted().unwrap().is_readonly_threshold_storage());
        assert_eq!(current.name, "edited.pcraft");
        assert_eq!(current.icc_profile, before.icc_profile);
        assert_eq!(current.source_color_context, before.source_color_context);
        let node = &current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert_eq!(node.enabled, !original_node.enabled);
        for (name, bits) in [("low", low_bits), ("high", 0.125f32.to_bits())] {
            let argument = serde_json::to_value(node.arguments.iter().find(|a| a.name == name).unwrap()).unwrap();
            assert_eq!(argument["value"]["Number"]["Float32Bits"], bits);
        }
        let saved_bytes = std::fs::read(&saved).unwrap();
        let reset = json!({"filterId":filter,"enabled":original_node.enabled}).to_string();
        let (code, _, error) = invoke(&[
            "run",
            saved.to_str().unwrap(),
            "--native-profile-policy",
            "keepCurrent",
            "--cmd",
            "xcf.filter.set",
            "--params",
            &patch_text,
            "--cmd",
            "xcf.filter.enable",
            "--params",
            &reset,
            "--out",
            continued.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{error}");
        assert_eq!(std::fs::read(&saved).unwrap(), saved_bytes);
        let mut last = Headless::trusted_local();
        drop(last.open_owned_native_keep_current_bounded(&continued).unwrap());
        let final_doc = &last.session.active().unwrap().doc;
        assert!(final_doc.accounted().unwrap().is_readonly_threshold_storage());
        assert_eq!(final_doc.name, "continued.pcraft");
        assert_eq!(final_doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled, original_node.enabled);
        assert_eq!(final_doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments, node.arguments);
        // Existing original source identity must prevent a differently named
        // .pcraft destination from replacing the XCF via an existing hardlink.
        let alias = dir.0.join("alias.pcraft");
        std::fs::hard_link(&source, &alias).unwrap();
        let (code, _, _) = invoke(&[
            "run",
            source.to_str().unwrap(),
            "--xcf-profile-policy",
            "keepCurrent",
            "--cmd",
            "xcf.filter.set",
            "--params",
            &patch_text,
            "--out",
            alias.to_str().unwrap(),
        ]);
        assert_ne!(code, 0);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read(alias).unwrap(), original);
        // Filter eligibility does not enroll generic pixel or layer mutations.
        for (command, params) in [("edit.clear", "{}"), ("edit.fill", r#"{"contents":"white"}"#), ("layer.setProps", r#"{"name":"unsupported"}"#)] {
            let (code, _, _) = invoke(&["run", saved.to_str().unwrap(), "--native-profile-policy", "keepCurrent", "--cmd", command, "--params", params]);
            assert_ne!(code, 0);
            assert_eq!(std::fs::read(&saved).unwrap(), saved_bytes);
        }
    }
}
