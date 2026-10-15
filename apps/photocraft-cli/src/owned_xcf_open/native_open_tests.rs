//! UNRUN explicit native keepCurrent CLI routing and real archive workflows.
use super::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
#[path = "../../../../crates/xcf/tests/common/mod.rs"]
mod fixture;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-cli-native-open-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn invoke(args: &[&str]) -> (i32, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = crate::run(&args.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>(), &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[test]
fn malformed_duplicate_and_mixed_policies_refuse_before_input_or_root_io() {
    for args in [
        vec!["info", "missing.pcraft", "--native-profile-policy", "convert"],
        vec!["info", "missing.pcraft", "--native-profile-policy", "keepCurrent", "--native-profile-policy", "keepCurrent"],
        vec!["info", "missing.pcraft", "--native-profile-policy=keepCurrent", "--xcf-profile-policy=keepCurrent"],
        vec!["info", "missing.xcf", "--xcf-profile-policy=keepCurrent", "--native-profile-policy=keepCurrent"],
        vec!["run", "missing.pcraft", "--native-profile-policy", "keepCurrent", "--new", "{}", "--cmd", "select.all", "--automation-read-root", "missing-root"],
        vec!["run", "missing.pcraft", "--native-profile-policy", "keepCurrent", "--out", "copy.png", "--automation-read-root", "missing-root"],
        vec!["run", "missing.pcraft", "--native-profile-policy", "keepCurrent", "--out", "copy.pcraft", "--format", "png"],
        vec!["run", "missing.pcraft", "--native-profile-policy", "keepCurrent", "--out", "one.pcraft", "--out", "two.pcraft"],
    ] {
        let (code, _, error) = invoke(&args);
        assert_ne!(code, 0);
        assert!(!error.contains("I/O:"), "{error}");
        assert!(error.contains("profile-policy") || error.contains("keepCurrent"), "{error}");
    }
    for args in [
        vec!["info", "missing.pcraft", "--native-profile-policy"],
        vec!["info", "missing.xcf", "--native-profile-policy", "keepCurrent"],
        vec!["info", "--native-profile-policy", "keepCurrent"],
        vec!["info", "one.pcraft", "two.pcraft", "--native-profile-policy", "keepCurrent"],
        vec!["convert", "missing.pcraft", "copy.pcraft", "--native-profile-policy", "keepCurrent"],
        vec!["batch", "--in", "missing", "--out", "missing-out", "--actions", "missing.json", "--native-profile-policy", "keepCurrent"],
    ] {
        let (code, _, error) = invoke(&args);
        assert_ne!(code, 0);
        assert!(!error.contains("I/O:"), "{error}");
    }
}

#[test]
fn real_native_info_registered_edit_and_save_as_preserve_original_archive() {
    let dir = Directory::new();
    let xcf = dir.0.join("source.xcf");
    let native = dir.0.join("current.pcraft");
    let output = dir.0.join("edited.pcraft");
    let xcf_bytes = fixture::image(13, 150, 0, 1, 1, &[31, 61, 91, 255]);
    std::fs::write(&xcf, &xcf_bytes).unwrap();
    // Existing XCF route creates a genuine native archive, without any test-only adoption.
    let (code, _, error) = invoke(&["run", xcf.to_str().unwrap(), "--xcf-profile-policy", "keepCurrent", "--out", native.to_str().unwrap()]);
    assert_eq!(code, 0, "{error}");
    let original = std::fs::read(&native).unwrap();
    let (code, summary, error) = invoke(&["info", native.to_str().unwrap(), "--native-profile-policy", "keepCurrent"]);
    assert_eq!(code, 0, "{error}");
    let summary: Value = serde_json::from_str(&summary).unwrap();
    assert_eq!(summary["width"], 1);
    assert_eq!(summary["height"], 1);
    assert_eq!(summary["nativeProfilePolicy"], "keepCurrent");
    let (code, results, error) =
        invoke(&["run", native.to_str().unwrap(), "--native-profile-policy", "keepCurrent", "--cmd", "select.all", "--out", output.to_str().unwrap()]);
    assert_eq!(code, 0, "{error}");
    assert!(results.contains("select.all"));
    assert_eq!(std::fs::read(&native).unwrap(), original);
    assert_eq!(std::fs::read(&xcf).unwrap(), xcf_bytes);
    let loaded = photocraft_format::load_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(loaded.name, "edited.pcraft");
    assert!(loaded.selection.is_some());
    assert!(loaded.xcf_state.is_some());
    let (code, _, error) = invoke(&["info", output.to_str().unwrap(), "--native-profile-policy=keepCurrent"]);
    assert_eq!(code, 0, "{error}");
    let (code, ordinary, error) = invoke(&["info", native.to_str().unwrap()]);
    assert_eq!(code, 0, "{error}");
    assert!(!ordinary.contains("nativeProfilePolicy"));
}
