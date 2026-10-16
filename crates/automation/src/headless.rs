//! Headless backend: a `photocraft_engine::Session` plus file I/O. Synchronous
//! and UI-free; the MCP server and the CLI both drive it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use photocraft_engine::{Session, command_specs, file_cmds};
use photocraft_format::PcraftWriter;
use photocraft_io::ExportOptions;
use serde_json::{Value, json};

use crate::workspace::{ExportSequence, authorize_engine_command, authorize_engine_step};
use crate::{AuthorizedWorkspace, AutomationError, files};

mod linked_root;
mod stored_channel_root;
mod owned_xcf_open;
mod owned_native_save;
mod owned_native_open;
mod owned_filter_inspect;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_point_command_tests.rs"]
mod owned_point_command_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_residual_point_command_tests.rs"]
mod owned_residual_point_command_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_configured_point_command_tests.rs"]
mod owned_configured_point_command_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_point_stack_command_tests.rs"]
mod owned_point_stack_command_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_scalar_point_command_tests.rs"]
mod owned_scalar_point_command_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_point_remove_command_tests.rs"]
mod owned_point_remove_command_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_empty_point_command_tests.rs"]
mod owned_empty_point_command_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_neighborhood_command_tests.rs"]
mod owned_neighborhood_command_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_analytic_geometry_command_tests.rs"]
mod owned_analytic_geometry_command_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_analytic_geometry_layer_props_tests.rs"]
mod owned_analytic_geometry_layer_props_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_analytic_geometry_mask_enabled_tests.rs"]
mod owned_analytic_geometry_mask_enabled_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_isolated_normal_group_tests.rs"]
mod owned_isolated_normal_group_tests;
pub use photocraft_engine::accounted_filter_inspect::{OwnedFilterInspectReply, OwnedFilterInspectWire};
pub use owned_native_open::OwnedNativeOpenReply;
pub use owned_xcf_open::OwnedXcfOpenReply;
pub use stored_channel_root::{load_stored_selection_from_root_with_context, validate_stored_selection_request};

fn native_save_requested(name: &str, format: Option<&str>) -> bool {
    format
        .map(|value| value.trim_start_matches('.'))
        .or_else(|| Path::new(name).extension().and_then(|value| value.to_str()))
        .is_some_and(|value| value.eq_ignore_ascii_case(photocraft_format::EXTENSION))
}

/// Unrun during the implementation-first pass.
#[cfg(test)]
mod calculation_dispatch_drafts {
    use super::*;
    use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};

    #[test]
    fn nonwaiting_calculations_finish_inline_and_bad_params_preserve_revision() {
        let mut backend = Headless::new();
        let doc = Document::with_background("Owned", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        backend.session.add_document(doc, None).unwrap();
        assert!(backend.command_start("image.applyImage", Value::Null, false).unwrap().is_null());
        let result = backend.command_start("image.calculations", json!({"result":"newChannel"}), false).unwrap();
        assert_eq!(result["channel"], 0);
        assert!(result.get("job").is_none());
        let before = backend.session.active().unwrap().doc.clone();
        let revision = backend.session.active().unwrap().revision;
        assert!(backend.command_start("image.applyImage", json!({"opacity":"invalid"}), false).is_err());
        assert_eq!(backend.session.active().unwrap().doc, before);
        assert_eq!(backend.session.active().unwrap().revision, revision);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod prepared_native_save_tests {
    use super::*;
    fn root() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path =
            std::env::temp_dir().join(format!("photocraft-headless-native-stream-{}-{stamp}-{}", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        path
    }
    fn own_document() -> photocraft_doc::Document {
        photocraft_doc::Document::with_background(
            "Own streaming document",
            photocraft_doc::Size::new(3, 2),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U16,
            photocraft_doc::Color::WHITE,
        )
    }
    // Implementation-first drafts: exercise actual prepared plan/sink joins later.
    #[test]
    fn workspace_and_trusted_native_save_roundtrip_preserve_model_history_and_source() {
        for trusted in [false, true] {
            let root = root();
            let mut headless =
                if trusted { Headless::trusted_local() } else { Headless::with_workspace(AuthorizedWorkspace::new(Some(&root), Some(&root)).unwrap()) };
            headless.session.add_document(own_document(), None).unwrap();
            headless
                .session
                .edit("Own real edit", |doc, _| {
                    doc.name = "Own current model".into();
                    Ok(())
                })
                .unwrap();
            let before = headless.session.active().unwrap().doc.clone();
            let revision = headless.session.active().unwrap().revision;
            let labels = headless.session.active().unwrap().history.entry_labels().map(str::to_owned).collect::<Vec<_>>();
            let path = if trusted { root.join("own.pcraft") } else { PathBuf::from("own.pcraft") };
            headless.save_with_interrupt(None, Some(&path), None, &Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
            let bytes = std::fs::read(root.join("own.pcraft")).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *before);
            let active = headless.session.active().unwrap();
            assert!(active.doc.shares_state(&before));
            assert_eq!(active.revision, revision);
            assert_eq!(active.saved_revision, revision);
            assert!(active.source.is_none());
            assert_eq!(active.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(), labels);
            assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
            std::fs::remove_file(root.join("own.pcraft")).unwrap();
            std::fs::remove_dir(root).unwrap();
        }
    }
    #[test]
    fn nonactive_open_source_alias_refuses_before_output_and_does_not_mark_saved() {
        let root = root();
        let original = root.join("original-source.bin");
        std::fs::write(&original, b"own original source bytes").unwrap();
        std::fs::hard_link(&original, root.join("alias.pcraft")).unwrap();
        let mut headless = Headless::with_workspace(AuthorizedWorkspace::new(Some(&root), Some(&root)).unwrap());
        headless.session.add_document(own_document(), None).unwrap();
        headless.session.active_mut().unwrap().set_import_origin(
            photocraft_io::ImportOrigin::xcf(),
            "Own protected source",
            Some("original-source.bin".into()),
        );
        headless.session.add_document(own_document(), None).unwrap();
        headless
            .session
            .edit("Own dirty edit", |doc, _| {
                doc.name = "dirty output".into();
                Ok(())
            })
            .unwrap();
        let before = headless.session.active().unwrap().doc.clone();
        let saved = headless.session.active().unwrap().saved_revision;
        assert!(headless.save_with_interrupt(None, Some(Path::new("alias.pcraft")), None, &Default::default(), &photocraft_raster::Interrupt::NONE).is_err());
        assert!(headless.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(headless.session.active().unwrap().saved_revision, saved);
        assert!(headless.session.active().unwrap().path.is_none());
        assert_eq!(std::fs::read(&original).unwrap(), b"own original source bytes");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        std::fs::remove_file(root.join("alias.pcraft")).unwrap();
        std::fs::remove_file(original).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn lower_archive_cap_refusal_never_publishes_partial_native_output() {
        let root = root();
        std::fs::write(root.join("out.pcraft"), b"original destination").unwrap();
        let workspace = AuthorizedWorkspace::new(Some(&root), Some(&root)).unwrap();
        let document = own_document();
        let mut limits = photocraft_io::NativeSaveLimits::default();
        limits.archive.max_archive_bytes = 1;
        // A lower cap may refuse during preparation or writing, never publication.
        if let Ok(prepared) = photocraft_io::prepare_native_stream(&document, limits, &photocraft_raster::Interrupt::NONE) {
            let result = workspace.write_protected_stream_with_interrupt("out.pcraft", &[], &[], &photocraft_raster::Interrupt::NONE, move |file| {
                let (_, result) = prepared.write(file).map_err(native_save_error)?;
                Ok(result)
            });
            assert!(result.is_err());
        }
        assert_eq!(std::fs::read(root.join("out.pcraft")).unwrap(), b"original destination");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_file(root.join("out.pcraft")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn streamed_open_trusted_and_workspace_preserve_native_model_and_runtime_origin() {
        for trusted in [false, true] {
            let root = root();
            let original = own_document();
            let bytes = photocraft_format::save_to_bytes(&original, &Default::default()).unwrap();
            std::fs::write(root.join("own.pcraft"), &bytes).unwrap();
            let mut headless = if trusted { Headless::trusted_local() } else { Headless::with_workspace(AuthorizedWorkspace::new(Some(&root), None).unwrap()) };
            let path = if trusted { root.join("own.pcraft") } else { PathBuf::from("own.pcraft") };
            let response = headless.open_with_interrupt(&path, &photocraft_raster::Interrupt::NONE).unwrap();
            assert_eq!(response["index"], 0);
            let active = headless.session.active().unwrap();
            assert_eq!(*active.doc, original);
            assert_eq!(active.path.as_deref(), path.to_str());
            assert_eq!(active.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
            assert!(!active.history.can_undo());
            std::fs::remove_file(root.join("own.pcraft")).unwrap();
            std::fs::remove_dir(root).unwrap();
        }
    }
    #[test]
    fn cancelled_open_keeps_existing_tabs_focus_source_and_real_redo() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let root = root();
        let original = own_document();
        std::fs::write(root.join("own.pcraft"), photocraft_format::save_to_bytes(&original, &Default::default()).unwrap()).unwrap();
        let setup = || {
            let mut headless = Headless::with_workspace(AuthorizedWorkspace::new(Some(&root), None).unwrap());
            headless.session.add_document(original.clone(), None).unwrap();
            headless
                .session
                .edit("Real redo", |doc, _| {
                    doc.name = "redo sentinel".into();
                    Ok(())
                })
                .unwrap();
            assert!(headless.session.undo());
            headless
        };
        let mut measured = setup();
        let count = AtomicUsize::new(0);
        let measure = || {
            count.fetch_add(1, Ordering::Relaxed);
            false
        };
        measured.open_with_interrupt(Path::new("own.pcraft"), &photocraft_raster::Interrupt::cancel_only(&measure)).unwrap();
        assert!(count.load(Ordering::Relaxed) > 1);
        for threshold in 0..count.load(Ordering::Relaxed) {
            let mut headless = setup();
            let before = headless.session.active().unwrap().doc.clone();
            let revision = headless.session.active().unwrap().revision;
            let source = headless.session.active().unwrap().source.clone();
            let calls = AtomicUsize::new(0);
            let stop = || calls.fetch_add(1, Ordering::Relaxed) >= threshold;
            let result = headless.open_with_interrupt(Path::new("own.pcraft"), &photocraft_raster::Interrupt::cancel_only(&stop));
            if matches!(result, Err(AutomationError::Engine(photocraft_engine::EngineError::Cancelled))) {
                assert_eq!(headless.session.documents().len(), 1);
                let active = headless.session.active().unwrap();
                assert!(active.doc.shares_state(&before));
                assert_eq!(active.revision, revision);
                assert_eq!(active.source, source);
                assert!(active.path.is_none());
                assert!(headless.session.redo());
                assert_eq!(headless.session.active().unwrap().doc.name, "redo sentinel");
            } else {
                assert!(result.is_ok());
                assert_eq!(headless.session.documents().len(), 2);
            }
        }
        std::fs::remove_file(root.join("own.pcraft")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn denied_or_failed_native_open_does_not_install_partial_tab() {
        let root = root();
        std::fs::write(root.join("bad.pcraft"), b"PK\x03\x04").unwrap();
        let mut denied = Headless::new();
        assert!(matches!(denied.open_with_interrupt(&root.join("missing.pcraft"), &photocraft_raster::Interrupt::NONE), Err(AutomationError::BadRequest(_))));
        assert!(denied.session.documents().is_empty());
        let mut headless = Headless::with_workspace(AuthorizedWorkspace::new(Some(&root), None).unwrap());
        headless.session.add_document(own_document(), None).unwrap();
        let before = headless.session.active().unwrap().doc.clone();
        assert!(headless.open_with_interrupt(Path::new("bad.pcraft"), &photocraft_raster::Interrupt::NONE).is_err());
        assert_eq!(headless.session.documents().len(), 1);
        assert!(headless.session.active().unwrap().doc.shares_state(&before));
        std::fs::remove_file(root.join("bad.pcraft")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
fn native_save_error(error: photocraft_io::IoError) -> AutomationError {
    if matches!(error, photocraft_io::IoError::Cancelled) {
        AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
    } else {
        AutomationError::Import(error)
    }
}

fn encode_save_with_interrupt(
    document: &photocraft_doc::Document,
    name: &str,
    format: Option<&str>,
    options: &ExportOptions,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<(Vec<u8>, Vec<String>), AutomationError> {
    control.check().map_err(|_| photocraft_engine::EngineError::Cancelled)?;
    files::validate_write_name(Path::new(name), format)?;
    if native_save_requested(name, format) {
        return Err(AutomationError::BadRequest("native file saves require the prepared streaming sink".into()));
    }
    // Legacy flat encoders currently poll at their boundary. The final guarded
    // writer still observes cancellation before creating or publishing output.
    let encoded = files::save_bytes(document, name, format, options)?;
    control.check().map_err(|_| photocraft_engine::EngineError::Cancelled)?;
    Ok(encoded)
}

fn untrusted(filesystem: Filesystem) -> Headless {
    let mut session = Session::new();
    session.authorize = Some(authorize_engine_step);
    Headless { session, writers: HashMap::new(), filesystem }
}

enum Filesystem {
    Denied,
    TrustedLocal,
    Workspace(AuthorizedWorkspace),
}

/// A headless editing session.
pub struct Headless {
    pub session: Session,
    /// Incremental `.pcraft` writers per document id.
    writers: HashMap<u64, PcraftWriter>,
    filesystem: Filesystem,
}

impl Default for Headless {
    fn default() -> Self {
        untrusted(Filesystem::Denied)
    }
}

impl Headless {
    /// Explicit resource request through a separately authorized read root.
    pub fn recover_xcf_resource(&mut self, resource: &str, path: &str, policy: &str) -> Result<Value, AutomationError> {
        use photocraft_engine::xcf_resources::RecoveryPolicy;
        if resource.is_empty() || resource.len() > 20 || !resource.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(AutomationError::BadRequest("invalid resource identity".into()));
        }
        let id = resource.parse::<u64>().map_err(|_| AutomationError::BadRequest("invalid resource identity".into()))?;
        if id == 0 || id.to_string() != resource {
            return Err(AutomationError::BadRequest("resource identity must be a canonical nonzero decimal string".into()));
        }
        let policy = match policy {
            "exact" => RecoveryPolicy::Exact,
            "substitute" => RecoveryPolicy::AllowSubstitution,
            _ => return Err(AutomationError::BadRequest("resource recovery policy must be exact or substitute".into())),
        };
        let Filesystem::Workspace(workspace) = &self.filesystem else {
            return Err(AutomationError::BadRequest("XCF resource recovery requires an explicit read-root capability".into()));
        };
        let fidelity = workspace.recover_xcf_resource(
            &mut self.session,
            photocraft_doc::item_identity::ResourceId(id),
            path,
            policy,
            &photocraft_raster::Interrupt::NONE,
        )?;
        Ok(json!({"resource":resource,"byteIdentity":fidelity,"activated":false}))
    }
    /// Create an automation session with no filesystem authority.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a session for an explicit local CLI invocation. The CLI caller,
    /// not a remote automation client, supplies these host paths. Nested
    /// `actions.play` steps are not re-checked: this caller is already trusted.
    pub fn trusted_local() -> Self {
        Self { session: Session::new(), writers: HashMap::new(), filesystem: Filesystem::TrustedLocal }
    }

    /// Create an automation session with capability-scoped file access.
    /// Each step of `actions.play` is checked with [`authorize_engine_step`].
    pub fn with_workspace(workspace: AuthorizedWorkspace) -> Self {
        untrusted(Filesystem::Workspace(workspace))
    }

    /// Apply background jobs that finished since the last request, so every request (save,
    /// export, inspect, preview, `session.list`, commands) sees their result. Cheap when no job
    /// runs. The JSON-lines server and the MCP server call it before each request.
    pub fn sync_jobs(&mut self) {
        self.session.poll_jobs();
    }

    fn doc_index(&self, index: Option<usize>) -> Result<usize, AutomationError> {
        match index {
            Some(i) if i < self.session.documents().len() => Ok(i),
            Some(i) => Err(AutomationError::BadRequest(format!("no document at index {i}"))),
            None => self.session.active_index().ok_or_else(|| AutomationError::BadRequest("no document open".into())),
        }
    }

    /// Open a file and make it the active document.
    pub fn open(&mut self, path: &Path) -> Result<Value, AutomationError> {
        self.open_with_interrupt(path, &photocraft_raster::Interrupt::NONE)
    }
    /// Controlled native stream opening and atomic Session publication. Directory
    /// native bundles retain their legacy boundary-only controlled loader.
    pub fn open_with_interrupt(&mut self, path: &Path, control: &photocraft_raster::Interrupt<'_>) -> Result<Value, AutomationError> {
        control.check().map_err(|_| photocraft_engine::EngineError::Cancelled)?;
        let requested = path.to_str().ok_or_else(|| AutomationError::BadRequest("automation paths must be valid UTF-8".into()))?;
        let mut o = match &self.filesystem {
            Filesystem::Denied => return Err(AutomationError::BadRequest("automation filesystem access is not granted: read authority is absent".into())),
            Filesystem::TrustedLocal => files::open_with_interrupt(path, control)?,
            Filesystem::Workspace(workspace) => workspace.open_document_with_interrupt(requested, control)?,
        };
        // Complete response serialization and owned runtime provenance before
        // admission: a failure must not leave a newly focused partial tab.
        let assessment =
            o.assessment.map(serde_json::to_value).transpose().map_err(|error| AutomationError::Other(format!("XCF assessment serialization: {error}")))?;
        let owned = |value: &str| -> Result<String, AutomationError> {
            let mut result = String::new();
            result.try_reserve_exact(value.len()).map_err(|_| AutomationError::Other("open provenance allocation failed".into()))?;
            result.push_str(value);
            Ok(result)
        };
        let path = match file_cmds::template_name(&self.session, requested) {
            Some(untitled) => {
                o.document.name = untitled;
                None
            }
            None => Some(owned(requested)?),
        };
        let read_only = o.source_read_only || o.origin.is_read_only();
        let path = path.filter(|_| !read_only);
        let source_path = if read_only { Some(owned(requested)?) } else { path.as_deref().map(owned).transpose()? };
        let source_label = owned(requested)?;
        let mut response = json!({
            "name": o.document.name,
            "width": o.document.size.width,
            "height": o.document.size.height,
            "layers": o.document.layer_count(),
            "warnings": o.warnings,
        });
        if let Some(assessment) = assessment {
            response["assessment"] = assessment;
        }
        control.check().map_err(|_| photocraft_engine::EngineError::Cancelled)?;
        let index = self.session.add_document_with_interrupt(o.document, path, control)?;
        // Successful publication is the commit point. No postcommit poll may
        // report cancellation for a tab that has already been installed.
        if let Some(state) = self.session.active_mut() {
            state.set_import_origin(o.origin, source_label, source_path);
            state.source_read_only = read_only;
        }
        response["index"] = json!(index);
        Ok(response)
    }

    /// Place a file into a document as a Smart Object layer: the capability-scoped counterpart
    /// of `file.placeEmbedded`, which reads an ambient path and is therefore refused in
    /// automation (#2810). `path` resolves beneath the read root the way `open` resolves one,
    /// and `params` carries the command's own options (`scale`, `fit`, `center`).
    pub fn place(&mut self, index: Option<usize>, path: &str, params: &Value) -> Result<Value, AutomationError> {
        let i = self.doc_index(index)?;
        if self.session.active_index() != Some(i) && !self.session.set_active(i) {
            return Err(AutomationError::BadRequest(format!("no document at index {i}")));
        }
        let mut opts = params.as_object().cloned().unwrap_or_default();
        opts.insert("path".into(), json!(path));
        // Where ambient paths are allowed the command reads the file itself, as everywhere else.
        if matches!(&self.filesystem, Filesystem::TrustedLocal) {
            return self.command_run("file.placeEmbedded", Value::Object(opts));
        }
        let bytes = match &self.filesystem {
            Filesystem::Workspace(workspace) => workspace.read(path)?,
            _ => {
                return Err(AutomationError::BadRequest("automation filesystem access is not granted: read authority is absent".into()));
            }
        };
        let name = Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path);
        self.sync_jobs();
        // The bytes are already in memory and the path resolved beneath the root capability, so
        // placing them needs no filesystem check of its own (`place_bytes` is the command body).
        Ok(file_cmds::place_bytes(&mut self.session, name, bytes, None, &Value::Object(opts))?)
    }

    /// Export each exported layer of a document as its own file beneath the write root: the
    /// capability-scoped `file.export.layersToFiles`, which takes an ambient `dir` and is
    /// therefore refused in automation (#3216). The bytes come from the command's own core, so
    /// both paths write the same files. The write capability refuses to overwrite a file that
    /// already exists, where the ambient command would, and it creates the directory before the
    /// export runs, so a refused export can leave an empty directory behind â€” never a file.
    pub fn export_layers(&mut self, index: Option<usize>, dir: &str, params: &Value) -> Result<Value, AutomationError> {
        let i = self.doc_index(index)?;
        if self.session.active_index() != Some(i) && !self.session.set_active(i) {
            return Err(AutomationError::BadRequest(format!("no document at index {i}")));
        }
        if matches!(&self.filesystem, Filesystem::TrustedLocal) {
            let mut opts = params.as_object().cloned().unwrap_or_default();
            opts.insert("dir".into(), json!(dir));
            return self.command_run("file.export.layersToFiles", Value::Object(opts));
        }
        self.sync_jobs();
        let workspace = match &self.filesystem {
            Filesystem::Workspace(workspace) => workspace,
            _ => return Err(AutomationError::BadRequest("automation filesystem access is not granted: write authority is absent".into())),
        };
        let mut outputs = workspace.export_sequence(dir)?;
        let written = file_cmds::export_layers_with(&self.session, params, |name, bytes| {
            outputs.write(name, bytes).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
        });
        exported(dir, written, &mut outputs)
    }

    /// Export each data set of a document as its own flattened file beneath the write root: the
    /// capability-scoped `file.export.dataSetsAsFiles` (#3216). As in the command, the whole
    /// batch is planned first and colliding names are refused before any file is written; the
    /// directory itself is created before that, so a refused batch can leave it empty behind.
    pub fn export_data_sets(&mut self, index: Option<usize>, dir: &str, params: &Value) -> Result<Value, AutomationError> {
        let i = self.doc_index(index)?;
        if self.session.active_index() != Some(i) && !self.session.set_active(i) {
            return Err(AutomationError::BadRequest(format!("no document at index {i}")));
        }
        if matches!(&self.filesystem, Filesystem::TrustedLocal) {
            let mut opts = params.as_object().cloned().unwrap_or_default();
            opts.insert("dir".into(), json!(dir));
            return self.command_run("file.export.dataSetsAsFiles", Value::Object(opts));
        }
        self.sync_jobs();
        let workspace = match &self.filesystem {
            Filesystem::Workspace(workspace) => workspace,
            _ => return Err(AutomationError::BadRequest("automation filesystem access is not granted: write authority is absent".into())),
        };
        let mut outputs = workspace.export_sequence(dir)?;
        let written = photocraft_engine::export_data_sets_with(&self.session, params, |name, bytes| {
            outputs.write(name, bytes).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
        });
        exported(dir, written, &mut outputs)
    }

    /// Import a data set CSV from the read root: the capability-scoped
    /// `file.import.variableDataSets`, which takes an ambient `path` and is therefore refused
    /// in automation (#3216). The file is read beneath the root the way `open` reads one and
    /// handed to the command's own parser over the text it already has.
    pub fn import_data_sets(&mut self, index: Option<usize>, path: &str, params: &Value) -> Result<Value, AutomationError> {
        let i = self.doc_index(index)?;
        if self.session.active_index() != Some(i) && !self.session.set_active(i) {
            return Err(AutomationError::BadRequest(format!("no document at index {i}")));
        }
        if matches!(&self.filesystem, Filesystem::TrustedLocal) {
            let mut opts = params.as_object().cloned().unwrap_or_default();
            opts.insert("path".into(), json!(path));
            return self.command_run("file.import.variableDataSets", Value::Object(opts));
        }
        let text = match &self.filesystem {
            Filesystem::Workspace(workspace) => {
                let bytes = workspace.read(path)?;
                String::from_utf8(bytes).map_err(|e| AutomationError::BadRequest(format!("automation read `{path}` is not valid UTF-8: {e}")))?
            }
            _ => return Err(AutomationError::BadRequest("automation filesystem access is not granted: read authority is absent".into())),
        };
        self.sync_jobs();
        Ok(photocraft_engine::import_data_sets_text(&mut self.session, &text, params)?)
    }

    /// Save (`.pcraft` or any export format by extension). With no path,
    /// saves to the document's own path.
    pub fn save(&mut self, index: Option<usize>, path: Option<&Path>, format: Option<&str>, opts: &ExportOptions) -> Result<Value, AutomationError> {
        self.save_with_interrupt(index, path, format, opts, &photocraft_raster::Interrupt::NONE)
    }

    /// Controlled byte saves preserve the token through native preparation and commit.
    pub fn save_with_interrupt(
        &mut self,
        index: Option<usize>,
        path: Option<&Path>,
        format: Option<&str>,
        opts: &ExportOptions,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<Value, AutomationError> {
        control.check().map_err(|_| photocraft_engine::EngineError::Cancelled)?;
        let i = self.doc_index(index)?;
        if self.session.documents().get(i).is_some_and(|state|state.doc.accounted().is_some()) {
            if self.session.active_index()!=Some(i) {
                return Err(AutomationError::BadRequest("owned native save currently requires the active document; select it explicitly first".into()));
            }
            return self.save_owned_native_active_bounded(path,format,control);
        }
        let (stored_path, doc) = {
            let state = &self.session.documents()[i];
            state.doc.validate_resident().map_err(|error| AutomationError::Engine(error.into()))?;
            (state.path.clone(), state.doc.clone())
        };
        let target: PathBuf = match (path, stored_path.as_deref()) {
            (Some(p), _) => p.to_path_buf(),
            // Like the desktop's File â€º Save: without a new path, only a layered file is written
            // back, in its own format; a flattened or converted copy never replaces it (#416).
            (None, Some(p)) => {
                let own = file_cmds::extension(p);
                let written = format.map(|f| f.trim_start_matches('.').to_ascii_lowercase()).or_else(|| own.clone());
                if !file_cmds::holds_at(&doc, p) || written != own {
                    let lost = file_cmds::losses_at(&doc, p);
                    let why = if lost.is_empty() { String::new() } else { format!(" (it can't keep: {})", lost.join("; ")) };
                    return Err(AutomationError::BadRequest(format!(
                        "pass `path`: without one, only a PSD, PSB, .pcraft or OpenRaster file that holds the whole document is written back, in its own format, so `{p}` was left unchanged{why}"
                    )));
                }
                PathBuf::from(p)
            }
            (None, None) => {
                return Err(AutomationError::BadRequest("document has no path; pass `path`".into()));
            }
        };
        let target_text = target.to_str().ok_or_else(|| AutomationError::BadRequest("automation paths must be valid UTF-8".into()))?;
        let mut warnings = match &self.filesystem {
            Filesystem::Denied => return Err(AutomationError::BadRequest("automation filesystem access is not granted: write authority is absent".into())),
            Filesystem::TrustedLocal => {
                let protection = self.write_protection(control)?;
                files::validate_destination_protected_with_interrupt(&target, &protection, format, control)?;
                if target.is_dir() {
                    control.check().map_err(|_| photocraft_engine::EngineError::Cancelled)?;
                    if !protection.sources.is_empty() || !protection.fingerprints.is_empty() {
                        files::save_protected_with_interrupt(&doc, &target, format, opts, &protection, control)?
                    } else {
                        let writer = self.writers.entry(doc.id.0).or_default();
                        files::save(&doc, &target, format, opts, Some(writer))?
                    }
                } else if native_save_requested(target_text, format) {
                    // Source identities and target checks precede preparation; all
                    // preview/model admission completes before opening the temp.
                    let prepared = photocraft_io::prepare_native_stream(&doc, Default::default(), control).map_err(native_save_error)?;
                    let result = files::write_protected_stream_with_interrupt(&target, &protection, control, move |file| {
                        let (_, result) = prepared.write(file).map_err(native_save_error)?;
                        Ok(result)
                    })?;
                    result.warnings
                } else {
                    let (bytes, warnings) = encode_save_with_interrupt(&doc, target_text, format, opts, control)?;
                    files::write_protected_native_with_interrupt(&target, &bytes, &protection, control)?;
                    warnings
                }
            }
            Filesystem::Workspace(workspace) => {
                files::validate_write_name(&target, format)?;
                let protection = self.write_protection(control)?;
                workspace.guard_write_protected_with_interrupt(target_text, &protection.sources, &protection.fingerprints, control)?;
                if native_save_requested(target_text, format) {
                    workspace.save_native_with_interrupt(&doc, target_text, &protection, Default::default(), control)?.warnings
                } else {
                    let (bytes, warnings) = encode_save_with_interrupt(&doc, target_text, format, opts, control)?;
                    workspace.write_protected_with_interrupt(target_text, &bytes, &protection.sources, &protection.fingerprints, control)?;
                    warnings
                }
            }
        };
        warnings.extend(photocraft_engine::type_extra_cmds::missing_font_warnings(&doc));
        // A layered document save (PSD, PSB, OpenRaster or .pcraft) that holds the whole document is a
        // full write, not a flattened copy: record the current revision as saved so the session
        // stops reporting `dirty`, and make the file the document's path. A flat export (PNG,
        // JPEG, â€¦), or an OpenRaster file that drops part of it, is a copy and leaves both alone
        // (#1547, #3806).
        let ext =
            format.map(|f| f.trim_start_matches('.').to_ascii_lowercase()).or_else(|| target.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()));
        if ext.as_deref().is_some_and(|e| file_cmds::holds(&doc, e)) {
            // Saving by index must not retarget the next automation command.
            let previously_active = self.session.active_index();
            self.session.set_active(i);
            let saved = self
                .session
                .active_mut()
                .ok_or(photocraft_engine::EngineError::NoDocument)
                .and_then(|state| state.saved_to(target.to_string_lossy().into_owned()));
            if let Some(active) = previously_active {
                self.session.set_active(active);
            }
            saved?;
        }
        Ok(json!({ "path": target.to_string_lossy(), "warnings": warnings }))
    }

    /// Write rendered bytes through the configured write authority.
    pub fn write_render(&self, path: &Path, bytes: &[u8]) -> Result<(), AutomationError> {
        self.write_render_with_interrupt(path, bytes, &photocraft_raster::Interrupt::NONE)
    }

    fn write_protection(&self, control: &photocraft_raster::Interrupt<'_>) -> Result<photocraft_engine::WriteProtection, AutomationError> {
        let mut protection = photocraft_engine::WriteProtection::default();
        for state in self.session.documents() {
            control.check().map_err(|_| photocraft_engine::EngineError::Cancelled)?;
            protection.add_document_with_interrupt(&state.doc, state.source.as_ref(), control)?;
        }
        Ok(protection)
    }

    /// Write already rendered bytes using the caller's cancellation token.
    pub fn write_render_with_interrupt(&self, path: &Path, bytes: &[u8], control: &photocraft_raster::Interrupt<'_>) -> Result<(), AutomationError> {
        control.check().map_err(|_| photocraft_engine::EngineError::Cancelled)?;
        let requested = path.to_str().ok_or_else(|| AutomationError::BadRequest("automation paths must be valid UTF-8".into()))?;
        match &self.filesystem {
            Filesystem::Denied => Err(AutomationError::BadRequest("automation filesystem access is not granted: write authority is absent".into())),
            Filesystem::TrustedLocal => {
                let protection = self.write_protection(control)?;
                files::write_protected_native_with_interrupt(path, bytes, &protection, control)
            }
            Filesystem::Workspace(workspace) => {
                let protection = self.write_protection(control)?;
                workspace.write_protected_with_interrupt(requested, bytes, &protection.sources, &protection.fingerprints, control)
            }
        }
    }

    pub fn inspect(&self, index: Option<usize>) -> Result<Value, AutomationError> {
        let i = self.doc_index(index)?;
        let state = &self.session.documents()[i];
        state.doc.validate_resident().map_err(|error| AutomationError::Engine(error.into()))?;
        Ok(photocraft_engine::inspect::document(state)?)
    }

    pub fn render_png(&self, index: Option<usize>, max_side: u32) -> Result<Vec<u8>, AutomationError> {
        let i = self.doc_index(index)?;
        let doc = &self.session.documents()[i].doc;
        doc.validate_resident().map_err(|error| AutomationError::Engine(error.into()))?;
        if !matches!(&self.filesystem, Filesystem::TrustedLocal) {
            crate::budgets::check_preview(doc.size.width, doc.size.height, max_side)?;
        }
        let png = files::render_png(doc, max_side)?;
        if !matches!(&self.filesystem, Filesystem::TrustedLocal) {
            crate::budgets::check_png(png.len())?;
        }
        Ok(png)
    }

    pub fn session_list(&self) -> Result<Value, AutomationError> {
        Ok(photocraft_engine::inspect::session(&self.session)?)
    }

    pub fn select(&mut self, index: usize) -> Result<Value, AutomationError> {
        // Prepare the response before retargeting, so typed inspection refusal
        // cannot leave a failed request pointing at a different document.
        let mut response = self.session_list()?;
        if self.session.set_active(index) {
            response["active"] = json!(self.session.active_index());
            Ok(response)
        } else {
            Err(AutomationError::BadRequest(format!("no document at index {index}")))
        }
    }

    pub fn close(&mut self, index: Option<usize>) -> Result<Value, AutomationError> {
        let i = self.doc_index(index)?;
        // Closing the last accounted owner is allowed, but a response requiring
        // another unadopted accounted inspector must fail before any close.
        for (other, state) in self.session.documents().iter().enumerate() {
            if other != i {
                state.doc.validate_resident().map_err(|error| AutomationError::Engine(error.into()))?;
            }
        }
        if let Some(d) = self.session.close(i) {
            self.writers.remove(&d.doc.id.0);
        }
        self.session_list()
    }

    pub fn command_list(&self) -> Value {
        Value::Array(command_specs().iter().map(|c| photocraft_engine::commands::list_entry(&self.session, c, self.session.disabled_reason(c.id))).collect())
    }

    pub fn command_run(&mut self, id: &str, params: Value) -> Result<Value, AutomationError> {
        self.command_start(id, params, true)
    }

    /// Run a command; with `wait` false, a job-capable command (filters, Content-Aware Fill,
    /// Photomerge, â€¦) runs in the background and this returns `{"job": id}` at once (poll with
    /// `jobs.list`, stop with `jobs.cancel`). Other commands finish before returning either way.
    pub fn command_start(&mut self, id: &str, params: Value, wait: bool) -> Result<Value, AutomationError> {
        if matches!(id,"xcf.filter.inspect"|"xcf.inspect") && self.session.active().is_some_and(|state| state.doc.accounted().is_some()) {
            return Err(AutomationError::BadRequest("accounted filter inspection requires a retained reply transport or typed Headless API".into()));
        }
        if id == "select.loadSelection" && params.get("originalPath").is_some() {
            return self.stored_selection_from_root(&params, wait);
        }
        let params = if params.is_null() { json!({}) } else { params };
        if !matches!(&self.filesystem, Filesystem::TrustedLocal) {
            if id == "file.export.renderVideo" {
                return self.render_video(params, |_, _| true);
            }
            authorize_engine_command(id, &params)?;
        }
        // Current Point and assigned geometry metadata use the synchronous paid engine adapter
        // as its filter edits. Refuse start before syncing unrelated jobs; a
        // browser/native storage marker does not grant background execution.
        if id == "layer.setProps"
            && self.session.active().and_then(|state| state.doc.accounted()).is_some_and(|owner| {
                owner.is_readonly_point_storage() || photocraft_engine::owned_layer_metadata_candidate(owner)
            })
        {
            if !wait {
                return Err(AutomationError::BadRequest(format!("{id}.context: synchronous dispatch is required")));
            }
            return Ok(self.session.execute(id, params)?);
        }
        if id == "xcf.composition.set"
            && self.session.active().and_then(|state| state.doc.accounted()).is_some_and(|owner| owner.is_readonly_flat_composite_storage())
        {
            if !wait {
                return Err(AutomationError::BadRequest(format!("{id}.context: synchronous dispatch is required")));
            }
            return Ok(self.session.execute(id, params)?);
        }
        if id == "layer.translate" && self.session.active().is_some_and(|state| state.doc.accounted().is_some()) {
            if !wait {
                return Err(AutomationError::BadRequest(format!("{id}.context: synchronous dispatch is required")));
            }
            return Ok(self.session.execute(id, params)?);
        }
        if matches!(
            id,
            "document.pixel.inspect"
                | "xcf.assess"
                | "xcf.inspect"
                | "xcf.composition.inspect"
                | "xcf.filter.inspect"
                | "xcf.resource.inspect"
                | "xcf.filter.graph.inspectPlan"
        ) {
            // These queries read the currently installed snapshot. Publishing
            // a queued job is a separate event, not an inspection side effect.
            return Ok(self.session.execute(id, params)?);
        }
        if matches!(id, "image.applyImage" | "image.calculations") {
            // These commands publish one synchronous controlled transaction.
            // A nonwaiting request still receives its completed inline result.
            return Ok(self.session.execute(id, params)?);
        }
        if matches!(
            id,
            "xcf.link.substitute"
                | "xcf.link.recover"
                | "xcf.item.locks.set"
                | "xcf.items.select"
                | "xcf.items.visibility.set"
                | "xcf.channel.overlay.set"
                | "xcf.mask.enabled.set"
                | "xcf.mask.geometry.set"
                | "xcf.layer.affine.apply"
                | "xcf.filter.move"
                | "xcf.filter.remove"
                | "xcf.filter.insert"
                | "xcf.filter.set"
                | "xcf.filter.enable"
                | "xcf.filter.mask.enabled.set"
                | "xcf.indexed.pixel.set"
                | "xcf.indexed.palette.set"
                | "xcf.text.name.set"
                | "xcf.channel.name.set"
                | "xcf.channel.locks.set"
                | "xcf.channel.indicates.set"
                | "xcf.path.name.set"
                | "xcf.vector.fill.color.set"
                | "xcf.vector.stroke.color.set"
                | "xcf.itemSet.create"
                | "xcf.itemSet.rename"
                | "xcf.itemSet.delete"
                | "xcf.itemSet.setMembers"
                | "xcf.link.refresh"
                | "xcf.link.setPlacement"
                | "xcf.link.setTransform"
                | "layer.renameLayer"
        ) {
            if !wait {
                return Err(AutomationError::BadRequest(format!("{id}.context: synchronous dispatch is required")));
            }
            return Ok(self.session.execute(id, params)?);
        }
        // Background jobs that finished since the last request (or batch step) are applied first.
        self.sync_jobs();
        if wait {
            return Ok(self.session.execute(id, params)?);
        }
        Ok(match self.session.start(id, params)? {
            photocraft_engine::jobs::Started::Done(v) => v,
            photocraft_engine::jobs::Started::Job(job) => json!({"job": job.0, "pending": true}),
        })
    }
}

impl Headless {
    /// A direct automation Render Video uses only the launch-time write capability. Nested
    /// engine actions still reject this filesystem command until they can carry capabilities.
    pub(crate) fn render_video(&mut self, params: Value, progress: impl FnMut(usize, usize) -> bool) -> Result<Value, AutomationError> {
        self.sync_jobs();
        let Filesystem::Workspace(workspace) = &self.filesystem else {
            return Err(AutomationError::BadRequest("automation filesystem access is not granted: write authority is absent".into()));
        };
        let dir = params.get("dir").and_then(Value::as_str).ok_or_else(|| AutomationError::BadRequest("renderVideo needs `dir`".into()))?;
        let mut outputs = workspace.export_sequence(dir)?;
        let result = photocraft_engine::render_video_with(
            &self.session,
            &params,
            |name, bytes| outputs.write(name, bytes).map_err(|e| photocraft_engine::EngineError::Other(e.to_string())),
            progress,
        );
        match result {
            Ok(value) => {
                outputs.commit();
                Ok(value)
            }
            Err(error) => {
                outputs.cleanup()?;
                Err(error.into())
            }
        }
    }
}

/// Common tail of the two capability-scoped exports: the sequence keeps what it wrote, or drops
/// every file it created, so a half-finished export never looks like a finished one.
fn exported(dir: &str, written: photocraft_engine::Result<Vec<String>>, outputs: &mut ExportSequence) -> Result<Value, AutomationError> {
    match written {
        Ok(names) => {
            outputs.commit();
            let files: Vec<String> = names.iter().map(|name| format!("{dir}/{name}")).collect();
            let count = files.len();
            Ok(json!({"files": files, "count": count}))
        }
        Err(error) => {
            outputs.cleanup()?;
            Err(error.into())
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_isolated_group_props_tests.rs"]
mod owned_isolated_group_props_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_plain_translate_tests.rs"]
mod owned_plain_translate_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_assigned_translate_tests.rs"]
mod owned_assigned_translate_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_plain_bulk_translate_tests.rs"]
mod owned_plain_bulk_translate_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_gaussian_original_tests.rs"]
mod owned_gaussian_original_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_gaussian_group_tests.rs"]
mod owned_gaussian_group_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_gaussian_group_activation_tests.rs"]
mod owned_gaussian_group_activation_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_gaussian_group_metadata_tests.rs"]
mod owned_gaussian_group_metadata_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_gaussian_group_parameter_tests.rs"]
mod owned_gaussian_group_parameter_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_separable_blend_tests.rs"]
mod owned_separable_blend_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_separable_metadata_tests.rs"]
mod owned_separable_metadata_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_flat_current_tests.rs"]
mod owned_flat_current_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_flat_composition_tests.rs"]
mod owned_flat_composition_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_original_flat_tests.rs"]
mod owned_original_flat_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_flat_translate_tests.rs"]
mod owned_flat_translate_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_legacy_flat_tests.rs"]
mod owned_legacy_flat_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_component_flat_tests.rs"]
mod owned_component_flat_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_alpha_flat_tests.rs"]
mod owned_alpha_flat_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_lch_flat_tests.rs"]
mod owned_lch_flat_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_assigned_box_tests.rs"]
mod owned_assigned_box_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_assigned_box_enabled_tests.rs"]
mod owned_assigned_box_enabled_tests;
#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_assigned_median_tests.rs"]
mod owned_assigned_median_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_assigned_gaussian_tests.rs"]
mod owned_assigned_gaussian_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_streamed_box_tests.rs"]
mod owned_streamed_box_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_observed_box_metadata_tests.rs"]
mod owned_observed_box_metadata_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_observed_box_move_tests.rs"]
mod owned_observed_box_move_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_observed_box_source_activity_tests.rs"]
mod owned_observed_box_source_activity_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_assigned_f64_box_tests.rs"]
mod owned_assigned_f64_box_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_linear_box_tests.rs"]
mod owned_linear_box_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_perceptual_box_tests.rs"]
mod owned_perceptual_box_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_gray_box_tests.rs"]
mod owned_gray_box_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_gray_transfer_box_tests.rs"]
mod owned_gray_transfer_box_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_semi_flatten_tests.rs"]
mod owned_semi_flatten_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_hue_chroma_tests.rs"]
mod owned_hue_chroma_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_vibrance_tests.rs"]
mod owned_vibrance_tests;

#[cfg(all(test, not(target_arch="wasm32")))]
#[path="headless/owned_pure_point_geometry_tests.rs"]
mod owned_pure_point_geometry_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_nine_point_geometry_tests.rs"]
mod owned_nine_point_geometry_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_residual_point_geometry_tests.rs"]
mod owned_residual_point_geometry_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "headless/owned_filtered_flat_tests.rs"]
mod owned_filtered_flat_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="headless/owned_filtered_flat_edit_tests.rs"]
mod owned_filtered_flat_edit_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="headless/owned_filtered_flat_source_activity_tests.rs"]
mod owned_filtered_flat_source_activity_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="headless/owned_filtered_flat_ordered_tests.rs"]
mod owned_filtered_flat_ordered_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="headless/owned_filtered_flat_move_tests.rs"]
mod owned_filtered_flat_move_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="headless/owned_filtered_flat_remove_tests.rs"]
mod owned_filtered_flat_remove_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="headless/owned_filtered_flat_current_tests.rs"]
mod owned_filtered_flat_current_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="headless/owned_filtered_flat_exposure_tests.rs"]
mod owned_filtered_flat_exposure_tests;

#[cfg(all(test,not(target_arch="wasm32")))]
#[path="headless/owned_filtered_flat_invert_tests.rs"]
mod owned_filtered_flat_invert_tests;
