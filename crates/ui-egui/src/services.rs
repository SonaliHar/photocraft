//! Optional platform capabilities which avoid a whole-file export buffer.
use photocraft_engine::{WriteProtection, document_snapshot::DocumentSnapshot};

/// Successful atomic native output. Warnings use the ordinary Save As flow.
pub struct NativeSaveResult {
    pub warnings: Vec<String>,
}

/// Interactive native-save authority only. The platform validates exclusions
/// before preparation and again before atomic commit. The borrowed snapshot is
/// frozen for the call; it does not confer access to any retained source path.
/// Standard admission limits remain owned by the platform/native writer.
pub type NativeSaveFn = Box<dyn FnMut(&DocumentSnapshot, &str, &WriteProtection) -> Result<NativeSaveResult, String>>;

/// Explicit native input from a platform-authorized selected path. The host
/// retains its actual reader until immutable capture finishes. Existing catalog
/// quota, when present, must be reused rather than copying its statistics.
/// No retained document locator or profile policy grants read authority.
pub type OwnedNativeOpenFn = Box<
    dyn FnMut(
        &str,
        Option<&photocraft_raster::backing::quota::BackingQuota>,
        &photocraft_raster::backing::ReadControl<'_>,
    ) -> Result<photocraft_format::owned_document::OwnedDocumentArchive, String>,
>;

/// Explicit original-XCF input. A host captures its already authorized reader
/// on the actual supplied catalog quota; retained source locators grant no reads.
pub type OwnedPlainXcfOpenFn = Box<
    dyn FnMut(
        &str,
        Option<&photocraft_raster::backing::quota::BackingQuota>,
        &photocraft_raster::backing::ReadControl<'_>,
    ) -> Result<photocraft_io::OwnedPlainXcfImport, String>,
>;

/// The same explicit current-profile decision, with one source-selected decode
/// over the supported original-XCF classes. A failed read/import is final; this
/// capability never authorizes retrying another reader or widening eligibility.
pub type OwnedCurrentXcfOpenFn = Box<
    dyn FnMut(
        &str,
        Option<&photocraft_raster::backing::quota::BackingQuota>,
        &photocraft_raster::backing::ReadControl<'_>,
    ) -> Result<photocraft_io::OwnedXcfImport, String>,
>;

/// Explicit accounted native output. Preparation, source protection and the
/// atomic writer use the caller's same control and the snapshot's actual quota.
/// This capability does not complete Session save bookkeeping itself.
pub type AccountedNativeSaveFn = Box<dyn FnMut(&DocumentSnapshot, &str, &WriteProtection, &photocraft_raster::backing::ReadControl<'_>) -> Result<(), String>>;

/// Explicit browser selected-byte capability; no filesystem locator authority.
pub type OwnedXcfBytesPickerFn = crate::owned_xcf_bytes::PickerFn;

/// Explicit browser native archive selection; never a filesystem grant.
pub type OwnedNativeBytesPickerFn = crate::owned_xcf_bytes::PickerFn;

#[cfg(test)]
#[path = "services_accounted_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod accounted_tests;

/// Shared paid all-open exclusions; the engine owns the one implementation.
pub(crate) use photocraft_engine::AccountedSaveProtection;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PhotocraftApp, Services};
    use std::{cell::RefCell, rc::Rc};

    fn app(services: Services) -> PhotocraftApp {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", serde_json::json!({"width":2,"height":2})).unwrap();
        PhotocraftApp::new(session, services)
    }

    #[test]
    fn native_capability_precedes_vector_export_and_preserves_save_bookkeeping() {
        let called = Rc::new(RefCell::new(0));
        let observed = called.clone();
        let mut app = app(Services {
            native_save: Some(Box::new(move |snapshot, path, _| {
                assert_eq!(path, "own.pcraft");
                assert_eq!(snapshot.size, photocraft_doc::Size::new(2, 2));
                *observed.borrow_mut() += 1;
                Ok(NativeSaveResult { warnings: vec!["Own retained warning".into()] })
            })),
            export: Some(Box::new(|_, _, _| Err("whole-output exporter must not run".into()))),
            ..Default::default()
        });
        let before = app.session.active().unwrap().doc.clone();
        let result = app.save_to("own.pcraft".into(), None).unwrap();
        assert_eq!(*called.borrow(), 1);
        assert_eq!(result["warnings"][0], "Own retained warning");
        let active = app.session.active().unwrap();
        assert_eq!(active.doc.id, before.id);
        assert_eq!(active.doc.size, before.size);
        assert_eq!(active.doc.name, "own.pcraft");
        assert_eq!(active.path.as_deref(), Some("own.pcraft"));
        assert_eq!(active.saved_revision, active.revision);
    }

    #[test]
    fn native_failure_keeps_document_path_and_saved_revision() {
        let mut app = app(Services { native_save: Some(Box::new(|_, _, _| Err("own destination refused".into()))), ..Default::default() });
        let active = app.session.active().unwrap();
        let (snapshot, path, saved) = (active.doc.clone(), active.path.clone(), active.saved_revision);
        assert_eq!(app.save_to("failed.pcraft".into(), None).unwrap_err(), "own destination refused");
        let active = app.session.active().unwrap();
        assert!(active.doc.shares_state(&snapshot));
        assert_eq!((active.path.clone(), active.saved_revision), (path, saved));
    }

    #[test]
    fn native_service_receives_exclusions_from_other_open_documents() {
        let mut app = app(Services {
            native_save: Some(Box::new(|_, _, protection| {
                assert!(protection.sources.iter().any(|path| path == "other-source.xcf"));
                Err("own alias refusal".into())
            })),
            ..Default::default()
        });
        app.session.active_mut().unwrap().source =
            Some(photocraft_engine::SourceIdentity::new("Other", Some("other-source.xcf".into()), photocraft_io::ImportOrigin::xcf()));
        app.session.execute("file.new", serde_json::json!({"width":2,"height":2})).unwrap();
        assert_eq!(app.save_to("own-alias.pcraft".into(), None).unwrap_err(), "own alias refusal");
        assert!(app.session.active().unwrap().path.is_none());
    }

    #[test]
    fn automation_native_save_uses_only_rooted_capability() {
        let mut app = app(Services {
            native_save: Some(Box::new(|_, _, _| panic!("ambient native writer reached"))),
            automation_native_save: Some(Box::new(|_, path, _| {
                assert_eq!(path, "rooted.pcraft");
                Ok(NativeSaveResult { warnings: vec!["rooted warning".into()] })
            })),
            export: Some(Box::new(|_, _, _| panic!("whole archive export reached"))),
            ..Default::default()
        });
        let result = app.save_automation(Some("rooted.pcraft".into())).unwrap();
        assert_eq!(result, serde_json::json!({"path":"rooted.pcraft","warnings":["rooted warning"]}));
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("rooted.pcraft"));
    }

    #[test]
    fn absent_rooted_capability_never_falls_back_to_interactive_writer() {
        let mut app = app(Services {
            native_save: Some(Box::new(|_, _, _| panic!("ambient native writer reached"))),
            export: Some(Box::new(|_, _, _| Ok((vec![1], vec![])))),
            ..Default::default()
        });
        assert_eq!(app.save_automation(Some("rooted.pcraft".into())).unwrap_err(), "automation write authority is not configured");
        assert!(app.session.active().unwrap().path.is_none());
    }

    #[test]
    fn ordinary_export_final_writer_receives_all_open_source_exclusions() {
        let mut app = app(Services {
            export: Some(Box::new(|_, _, _| Ok((vec![1], vec![])))),
            write_protected: Some(Box::new(|_, _, protection| {
                assert!(protection.sources.iter().any(|path| path == "other-source.xcf"));
                Err("final alias refusal".into())
            })),
            ..Default::default()
        });
        app.session.active_mut().unwrap().source =
            Some(photocraft_engine::SourceIdentity::new("Other", Some("other-source.xcf".into()), photocraft_io::ImportOrigin::xcf()));
        app.session.execute("file.new", serde_json::json!({"width":2,"height":2})).unwrap();
        assert_eq!(app.write_document("own.png".into(), &crate::ExportSettings::default(), false).unwrap_err(), "final alias refusal");
        assert!(app.session.active().unwrap().path.is_none());
    }
}
