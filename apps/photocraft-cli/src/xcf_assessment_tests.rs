use super::*;
#[path = "../../../crates/io/tests/common/xcf.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod fixture;

fn invoke_info(path: &Path) -> Value {
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let args = vec!["info".to_owned(), path.to_string_lossy().into_owned(), "--compact".to_owned()];
    assert_eq!(run(&args, &mut output, &mut errors), 0, "{}", String::from_utf8_lossy(&errors));
    assert!(errors.is_empty());
    serde_json::from_slice(&output).unwrap()
}

#[test]
fn info_reevaluates_native_dependencies_instead_of_reusing_original_full_import() {
    use photocraft_engine::doc::{
        item_identity::ResourceId,
        xcf_resources::{ResourceKind, ResourceReference},
    };
    let folder = std::env::temp_dir().join(format!("photocraft-cli-xcf-assessment-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let source_path = folder.join("source.xcf");
    let native_path = folder.join("with-dependency.pcraft");
    let bytes = fixture::image(12, 150, 0, 1, 1, &[40, 60, 80, 255]);
    std::fs::write(&source_path, &bytes).unwrap();
    let source_info = invoke_info(&source_path);
    assert_eq!(source_info["assessment"]["fidelity"], "full");
    let mut imported = photocraft_io::import("source.xcf", &bytes).unwrap();
    imported.document.xcf_state.as_mut().unwrap().resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "retained dependency".into(),
        collection: None,
        internal: false,
        locator: Some("../never-read-resource.png".into()),
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    });
    let before = imported.document.clone();
    let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    std::fs::write(&native_path, &native).unwrap();
    let info = invoke_info(&native_path);
    assert_eq!(info["assessment"]["fidelity"], "dependency_limited");
    assert_eq!(info["assessment"]["dependencies"][0]["status"], "unresolved");
    assert_eq!(info["assessment"]["dependencies"][0]["consumerAvailable"], false);
    assert_eq!(info["assessment"]["diagnostics"][0]["code"], "missing_resource");
    assert_eq!(imported.document, before);
    assert_eq!(std::fs::read(&source_path).unwrap(), bytes);
    assert_eq!(std::fs::read(&native_path).unwrap(), native);
    std::fs::remove_file(source_path).unwrap();
    std::fs::remove_file(native_path).unwrap();
    std::fs::remove_dir(folder).unwrap();
}
