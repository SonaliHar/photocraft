use photocraft_automation::{AuthorizedWorkspace, Headless, files};
use photocraft_engine::SourceIdentity;
use photocraft_io::ImportOrigin;
use std::path::Path;

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "photocraft-xcf-protection-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn import_only_names_reject_overrides_before_encoding() {
    let mut h = Headless::trusted_local();
    h.command_run("file.new", serde_json::json!({"width":2,"height":2})).unwrap();
    let doc = &h.session.active().unwrap().doc;
    for (name, format) in [("new.XcF", Some("pcraft")), ("new.png", Some(".XCF")), ("new.xcf", None)] {
        assert!(files::save_bytes(doc, name, format, &Default::default()).is_err());
    }
}

#[test]
fn renamed_existing_xcf_and_source_aliases_are_protected() {
    let temp = Temp::new();
    let source = temp.0.join("project.psd");
    let alias = temp.0.join("alias.pcraft");
    let original = b"gimp xcf v011\0content";
    std::fs::write(&source, original).unwrap();
    std::fs::hard_link(&source, &alias).unwrap();
    let identity = SourceIdentity::new("project.psd", Some(source.to_string_lossy().into()), ImportOrigin::xcf());
    assert!(files::validate_destination(&source, Some(&identity), Some("pcraft")).is_err());
    assert!(files::validate_destination(&alias, Some(&identity), Some("pcraft")).is_err());
    assert!(files::validate_destination(&alias, None, Some("pcraft")).is_err());
    assert!(files::validate_destination(&temp.0.join("new.pcraft"), Some(&identity), None).is_ok());
    assert_eq!(std::fs::read(source).unwrap(), original);
}

#[test]
fn capability_guards_keep_authority_and_protect_hardlinks() {
    let temp = Temp::new();
    let source = temp.0.join("project.psd");
    std::fs::write(&source, b"gimp xcf v011\0content").unwrap();
    std::fs::hard_link(&source, temp.0.join("alias.png")).unwrap();
    let workspace = AuthorizedWorkspace::new(Some(&temp.0), Some(&temp.0)).unwrap();
    for name in ["project.psd", "alias.png", "../outside.png", "/absolute.png", "new.XCF"] {
        assert!(workspace.guard_write(name, Some("project.psd")).is_err(), "{name}");
    }
    assert!(workspace.guard_write("new.pcraft", Some("project.psd")).is_ok());
    assert!(workspace.write("alias.png", b"changed").is_err());
    assert_eq!(std::fs::read(&source).unwrap(), b"gimp xcf v011\0content");
    // Protect file identity even if another authorized caller subsequently changed source contents.
    std::fs::write(&source, b"externally replaced contents").unwrap();
    assert!(workspace.guard_write("alias.png", Some("project.psd")).is_err());
    assert!(files::validate_write_name(Path::new("safe.pcraft"), None).is_ok());
}

#[test]
fn saving_another_document_keeps_all_open_runtime_xcf_sources_protected_after_external_header_change() {
    let temp = Temp::new();
    let original = temp.0.join("renamed-source.psd");
    let alias = temp.0.join("alias.pcraft");
    std::fs::write(&original, b"externally replaced bytes, no XCF magic").unwrap();
    std::fs::hard_link(&original, &alias).unwrap();
    for rooted in [false, true] {
        let mut headless = if rooted { Headless::with_workspace(AuthorizedWorkspace::new(Some(&temp.0), Some(&temp.0)).unwrap()) } else { Headless::trusted_local() };
        headless.command_run("file.new", serde_json::json!({"width":2,"height":2})).unwrap();
        let source_path = if rooted { "renamed-source.psd".into() } else { original.to_string_lossy().into_owned() };
        headless.session.active_mut().unwrap().set_import_origin(ImportOrigin::xcf(), "own imported source", Some(source_path));
        headless.command_run("file.new", serde_json::json!({"width":2,"height":2})).unwrap();
        let before = headless.session.active().unwrap().doc.clone();
        for target in [
            if rooted { Path::new("alias.pcraft") } else { alias.as_path() },
            if rooted { Path::new("renamed-source.psd") } else { original.as_path() },
        ] {
            assert!(headless.save(None, Some(target), Some("pcraft"), &Default::default()).is_err());
        }
        assert_eq!(headless.session.active().unwrap().doc, before);
        assert_eq!(std::fs::read(&original).unwrap(), b"externally replaced bytes, no XCF magic");
        assert_eq!(std::fs::read(&alias).unwrap(), b"externally replaced bytes, no XCF magic");
    }
}
