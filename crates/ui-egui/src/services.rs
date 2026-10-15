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

/// Explicit accounted native output. Preparation, source protection and the
/// atomic writer use the caller's same control and the snapshot's actual quota.
/// This capability does not complete Session save bookkeeping itself.
pub type AccountedNativeSaveFn = Box<
    dyn FnMut(&DocumentSnapshot, &str, &WriteProtection, &photocraft_raster::backing::ReadControl<'_>) -> Result<(), String>,
>;

#[cfg(test)]
#[path = "services_accounted_tests.rs"]
mod accounted_tests;

/// All-open exclusions paid before copying, retained until the synchronous
/// writer returns. These inert strings do not grant source-file access.
pub(crate) struct AccountedSaveProtection {
    pub protection: WriteProtection,
    _lease: photocraft_raster::backing::quota::QuotaLease,
}
impl AccountedSaveProtection {
    pub(crate) fn prepare(session: &photocraft_engine::Session, control: &photocraft_raster::backing::ReadControl<'_>) -> Result<Self, String> {
        use photocraft_raster::backing::{BackingError, quota::QuotaKind};
        let fail = || "accounted-save.source-protection-limit".to_owned();
        let docs = session.documents();
        if docs.len() > 4096 { return Err(fail()); }
        let quota = session.active().and_then(|s| s.doc.quota()).ok_or_else(fail)?;
        let mut paths = 0usize;
        let mut path_bytes = 0usize;
        let mut hashes = 0usize;
        let mut largest = 0usize;
        for state in docs {
            control.check(1).map_err(|e| e.to_string())?;
            if let Some(path) = state.source.as_ref().filter(|s| s.origin.is_read_only()).and_then(|s| s.path.as_ref()) {
                if path.len() > 4096 { return Err(fail()); }
                paths = paths.checked_add(1).ok_or_else(fail)?;
                path_bytes = path_bytes.checked_add(path.len()).ok_or_else(fail)?;
            }
            if let Some(state) = &state.doc.xcf_state {
                let count = state.provenance.protected_source_fingerprints.len().checked_add(usize::from(state.provenance.source_hash.is_some())).ok_or_else(fail)?;
                if count > photocraft_io::source_protection::MAX_FINGERPRINTS { return Err(fail()); }
                hashes = hashes.checked_add(count).ok_or_else(fail)?;
                largest = largest.max(count);
            }
        }
        if hashes > photocraft_io::source_protection::MAX_FINGERPRINTS { return Err(fail()); }
        let string = std::mem::size_of::<String>();
        let retained = paths.checked_add(hashes).and_then(|n| n.checked_mul(string)).and_then(|n| n.checked_add(path_bytes))
            .and_then(|n| hashes.checked_mul(128).and_then(|v| n.checked_add(v))).and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>() + 256)).ok_or_else(fail)?;
        let temporary = largest.checked_mul(string + 32 + 128).and_then(|n| n.checked_mul(2)).and_then(|n| n.checked_add(256)).ok_or_else(fail)?;
        let envelope = retained.checked_add(temporary).ok_or_else(fail)?;
        let mut lease = quota.reserve(QuotaKind::Index, envelope as u64).map_err(|e| e.to_string())?;
        let mut protection = WriteProtection::default();
        protection.sources.try_reserve_exact(paths).map_err(|_| BackingError::Allocation.to_string())?;
        protection.fingerprints.try_reserve_exact(hashes).map_err(|_| BackingError::Allocation.to_string())?;
        if protection.sources.capacity() > paths.saturating_mul(2) || protection.fingerprints.capacity() > hashes.saturating_mul(2) { return Err(fail()); }
        for state in docs {
            let count = state.doc.xcf_state.as_ref().map_or(0, |s| s.provenance.protected_source_fingerprints.len() + usize::from(s.provenance.source_hash.is_some()));
            // The existing bounded collector performs two literal passes and
            // at most count*(count-1)/2 comparisons, each on one exact digest.
            let work = (count as u64).checked_mul(count as u64 + 1).and_then(|n| n.checked_add((count as u64).checked_mul(256)?)).ok_or_else(fail)?;
            control.check(work).map_err(|e| e.to_string())?;
            let values = photocraft_io::source_protection::document_fingerprints_with_resident_limit(&state.doc, temporary as u64, control.interrupt).map_err(|e| e.to_string())?;
            for value in values { control.check(1).map_err(|e| e.to_string())?; protection.fingerprints.push(value); }
            if let Some(path) = state.source.as_ref().filter(|s| s.origin.is_read_only()).and_then(|s| s.path.as_ref()) {
                let mut owned = String::new();
                owned.try_reserve_exact(path.len()).map_err(|_| BackingError::Allocation.to_string())?;
                if owned.capacity() > path.len().saturating_mul(2) { return Err(fail()); }
                for ch in path.chars() { control.check(ch.len_utf8() as u64).map_err(|e| e.to_string())?; owned.push(ch); }
                protection.sources.push(owned);
            }
        }
        control.check((hashes as u64).checked_mul(128).ok_or_else(fail)?).map_err(|e| e.to_string())?;
        let actual = protection.resident_bytes_with_interrupt(control.interrupt).map_err(|e| e.to_string())?;
        if actual > retained { return Err(fail()); }
        lease.reconcile_capacity(actual as u64).map_err(|e| e.to_string())?;
        control.check(1).map_err(|e| e.to_string())?;
        Ok(Self { protection, _lease: lease })
    }
}

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
                assert_eq!(snapshot.size, photocraft_doc::Size::new(2,2));
                *observed.borrow_mut() += 1;
                Ok(NativeSaveResult { warnings: vec!["Own retained warning".into()] })
            })),
            export: Some(Box::new(|_,_,_| Err("whole-output exporter must not run".into()))),
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
        let mut app = app(Services { native_save: Some(Box::new(|_,_,_| Err("own destination refused".into()))), ..Default::default() });
        let active = app.session.active().unwrap();
        let (snapshot, path, saved) = (active.doc.clone(), active.path.clone(), active.saved_revision);
        assert_eq!(app.save_to("failed.pcraft".into(), None).unwrap_err(), "own destination refused");
        let active = app.session.active().unwrap();
        assert!(active.doc.shares_state(&snapshot));
        assert_eq!((active.path.clone(),active.saved_revision), (path,saved));
    }

    #[test]
    fn native_service_receives_exclusions_from_other_open_documents() {
        let mut app = app(Services { native_save: Some(Box::new(|_,_,protection| {
            assert!(protection.sources.iter().any(|path| path == "other-source.xcf"));
            Err("own alias refusal".into())
        })), ..Default::default() });
        app.session.active_mut().unwrap().source = Some(photocraft_engine::SourceIdentity::new("Other", Some("other-source.xcf".into()), photocraft_io::ImportOrigin::xcf()));
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
        app.session.active_mut().unwrap().source = Some(photocraft_engine::SourceIdentity::new("Other", Some("other-source.xcf".into()), photocraft_io::ImportOrigin::xcf()));
        app.session.execute("file.new", serde_json::json!({"width":2,"height":2})).unwrap();
        assert_eq!(app.write_document("own.png".into(), &crate::ExportSettings::default(), false).unwrap_err(), "final alias refusal");
        assert!(app.session.active().unwrap().path.is_none());
    }
}
