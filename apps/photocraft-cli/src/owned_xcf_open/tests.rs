//! UNRUN CLI policy and actual owned-source workflows; no test execution claimed.
use super::*;
#[path = "../../../../crates/xcf/tests/common/mod.rs"]
mod fixture;
fn invoke(args: &[&str]) -> (i32, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = crate::run(&args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(), &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}
#[test]
fn malformed_duplicate_and_unsupported_owned_policy_decline_before_input_or_root_io() {
    for args in [
        vec!["info", "missing.xcf", "--xcf-profile-policy", "unknown"],
        vec!["info", "missing.xcf", "--xcf-profile-policy", "keepCurrent", "--xcf-profile-policy", "keepCurrent"],
        vec!["run", "missing.xcf", "--xcf-profile-policy", "keepCurrent", "--out", "never.png", "--automation-read-root", "missing-root"],
        vec!["run", "--new", "{}", "--xcf-profile-policy", "keepCurrent", "--cmd", "edit.undo"],
    ] {
        let (code, _, error) = invoke(&args);
        assert_ne!(code, 0);
        assert!(error.contains("keepCurrent") || error.contains("--xcf-profile-policy"));
        assert!(!error.contains("I/O:"));
    }
    assert_ne!(invoke(&["convert", "missing.xcf", "never.pcraft", "--xcf-profile-policy", "keepCurrent"]).0, 0);
}
#[test]
fn actual_info_owned_summary_and_run_registered_noop_keep_ordinary_open_default() {
    let path = std::env::temp_dir().join(format!("photocraft-cli-owned-{}.xcf", std::process::id()));
    std::fs::write(&path, fixture::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])).unwrap();
    let path = path.to_str().unwrap();
    let (code, out, error) = invoke(&["info", path, "--xcf-profile-policy", "keepCurrent"]);
    assert_eq!(code, 0, "{error}");
    let result: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(result["inspectionScope"], "owned-import-summary");
    assert_eq!(result["sourceReadOnly"], true);
    assert_eq!(result["width"], 1);
    // An actual registered selection command uses the existing accounted route.
    let (code, out, error) = invoke(&["run", path, "--xcf-profile-policy", "keepCurrent", "--cmd", "select.all"]);
    assert_eq!(code, 0, "{error}");
    assert!(out.contains("select.all"));
    let (code, out, error) = invoke(&["info", path]);
    assert_eq!(code, 0, "{error}");
    assert!(!out.contains("owned-import-summary"));
    std::fs::remove_file(path).unwrap();
}
