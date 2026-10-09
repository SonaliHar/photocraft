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
