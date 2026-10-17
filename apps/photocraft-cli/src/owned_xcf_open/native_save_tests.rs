//! UNRUN actual CLI owned import -> registered edits -> protected native output.
use super::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
#[path = "../../../../crates/xcf/tests/common/mod.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod fixture;
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-cli-owned-native-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
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
fn actual_registered_selection_history_survives_owned_native_cli_output_without_source_write() {
    let dir = Directory::new();
    let source = dir.0.join("source.xcf");
    let output = dir.0.join("copy.pcraft");
    let original = fixture::image(13, 150, 0, 1, 1, &[31, 61, 91, 255]);
    std::fs::write(&source, &original).unwrap();
    let (code, results, error) = invoke(&[
        "run",
        source.to_str().unwrap(),
        "--xcf-profile-policy",
        "keepCurrent",
        "--cmd",
        "select.all",
        "--cmd",
        "edit.undo",
        "--cmd",
        "edit.redo",
        "--out",
        output.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{error}");
    assert!(results.contains("select.all"));
    assert!(results.contains("edit.redo"));
    assert_eq!(std::fs::read(source).unwrap(), original);
    let mut reopened = Headless::trusted_local();
    drop(reopened.open_owned_native_keep_current_bounded(&output).unwrap());
    let loaded = &reopened.session.active().unwrap().doc;
    assert_eq!(loaded.name, "copy.pcraft");
    assert!(loaded.selection.is_some());
    assert!(loaded.xcf_state.is_some());
    assert!(loaded.source_color_context.is_some());
}
#[test]
fn unsupported_native_recipe_refuses_before_missing_input_and_source_alias_stays_unchanged() {
    for args in [
        vec!["run", "missing.xcf", "--xcf-profile-policy", "keepCurrent", "--cmd", "select.all", "--out", "copy.png"],
        vec!["run", "missing.xcf", "--xcf-profile-policy", "keepCurrent", "--cmd", "select.all", "--out", "copy.pcraft", "--format", "png"],
    ] {
        let (code, _, error) = invoke(&args);
        assert_ne!(code, 0);
        assert!(error.contains(".pcraft") || error.contains("pcraft format"));
        assert!(!error.contains("I/O:"));
    }
    let dir = Directory::new();
    let source = dir.0.join("source.xcf");
    let alias = dir.0.join("alias.pcraft");
    let original = fixture::image(13, 150, 0, 1, 1, &[1, 2, 3, 255]);
    std::fs::write(&source, &original).unwrap();
    std::fs::hard_link(&source, &alias).unwrap();
    let (code, _, _) =
        invoke(&["run", source.to_str().unwrap(), "--xcf-profile-policy", "keepCurrent", "--cmd", "select.all", "--out", alias.to_str().unwrap()]);
    assert_ne!(code, 0);
    assert_eq!(std::fs::read(source).unwrap(), original);
    assert_eq!(std::fs::read(alias).unwrap(), original);
}
