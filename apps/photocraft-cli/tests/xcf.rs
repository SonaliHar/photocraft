#[path = "../../../crates/io/tests/common/xcf.rs"]
mod common;
use std::path::Path;
use std::process::Command;
fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_photocraft-cli")).args(args).output().unwrap()
}
fn text(p: &Path) -> &str {
    p.to_str().unwrap()
}
#[test]
fn xcf_process_cli_info_convert_run_batch_and_source_guards() {
    let dir = std::env::temp_dir().join(format!("photocraft-xcf-cli-{}", std::process::id()));
    let input = dir.join("in");
    let output = dir.join("out");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    let source = input.join("sample.xcf");
    let bytes = common::image(12, 150, 0, 1, 1, &[17, 34, 51, 255]);
    std::fs::write(&source, &bytes).unwrap();
    let info = cli(&["info", text(&source), "--compact"]);
    assert!(info.status.success(), "{}", String::from_utf8_lossy(&info.stderr));
    let v: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
    assert_eq!(v["width"], 1);
    let half = dir.join("half.xcf");
    std::fs::write(&half, common::image(12, 550, 0, 1, 1, &[0x38, 0, 0x34, 0, 0x30, 0, 0x3c, 0])).unwrap();
    let info = cli(&["info", text(&half), "--compact"]);
    assert!(info.status.success());
    let v: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("F16")));
    let broken = dir.join("broken.xcf");
    std::fs::write(&broken, b"gimp xcf v012\0").unwrap();
    let error = cli(&["info", text(&broken)]);
    assert_eq!(error.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&error.stderr).contains("XCF"));
    let native = output.join("edited.pcraft");
    assert!(cli(&["convert", text(&source), text(&native)]).status.success());
    let renamed = input.join("renamed.pcraft");
    std::fs::write(&renamed, &bytes).unwrap();
    assert!(cli(&["info", text(&renamed)]).status.success());
    assert!(!cli(&["convert", text(&renamed), text(&renamed), "--format", "pcraft"]).status.success());
    let alias = output.join("alias.png");
    std::fs::hard_link(&renamed, &alias).unwrap();
    assert!(!cli(&["convert", text(&renamed), text(&alias), "--format", "pcraft"]).status.success());
    let noxcf = output.join("new.XCF");
    assert!(!cli(&["convert", text(&source), text(&noxcf), "--format", "pcraft"]).status.success());
    assert!(!noxcf.exists());
    let runfile = output.join("run.pcraft");
    assert!(cli(&["run", text(&source), "--cmd", "layer.new.layer", "--params", r#"{"name":"Edited"}"#, "--out", text(&runfile)]).status.success());
    let actions = dir.join("actions.json");
    std::fs::write(&actions, "[]").unwrap();
    let failed = cli(&["batch", "--actions", text(&actions), "--in", text(&input), "--out", text(&output)]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("requires --format"));
    let batch = cli(&["batch", "--actions", text(&actions), "--in", text(&input), "--out", text(&output), "--format", "pcraft"]);
    assert!(batch.status.success(), "{}", String::from_utf8_lossy(&batch.stderr));
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    assert_eq!(std::fs::read(&renamed).unwrap(), bytes);
    std::fs::remove_dir_all(dir).unwrap();
}
