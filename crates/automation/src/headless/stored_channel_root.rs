//! Held-root host facade for the existing registered stored-channel Load.
use super::{Filesystem, Headless};
use crate::{AutomationError, workspace::authorize_engine_command};
use photocraft_doc::{DocId, item_identity::ChannelId};
use photocraft_engine::{
    channel_cmds::ChannelView,
    jobs::JobCtx,
    xcf_channel_host_load::{CapturedStoredTarget, HostLoadLimits, Quota},
};
use photocraft_io::CurrentProfilePolicy;
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaKind, QuotaLimits},
};
use serde_json::{Value, json};
const CMD: &str = "select.loadSelection";
const RESIDENT: u64 = 384 << 20;
const INDEX: u64 = 16 << 20;
const WORK: u64 = 1 << 34;
fn bad(message: &str) -> AutomationError {
    AutomationError::BadRequest(format!("select.loadSelection.originalPath: {message}"))
}
fn add(a: u64, b: u64) -> Result<u64, AutomationError> {
    a.checked_add(b).ok_or_else(|| bad("accounting overflow"))
}
fn decimal(p: &Value, key: &str, nonzero: bool) -> Result<u64, AutomationError> {
    let s = p.get(key).and_then(Value::as_str).ok_or_else(|| bad("identities/revision/ordinal must be strings"))?;
    if s.is_empty() || s.len() > 20 || !s.bytes().all(|n| n.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0')) {
        return Err(bad("canonical decimal strings required"));
    }
    let value = s.parse::<u64>().map_err(|_| bad("decimal out of range"))?;
    if nonzero && value == 0 {
        return Err(bad("nonzero identity required"));
    }
    Ok(value)
}
struct Request<'a> {
    target: CapturedStoredTarget,
    path: &'a str,
    ordinal: usize,
    policy: CurrentProfilePolicy,
}
fn parse(p: &Value, wait: bool) -> Result<Request<'_>, AutomationError> {
    if !wait {
        return Err(bad("stored original Load requires wait=true"));
    }
    const KEYS: [&str; 8] = ["documentId", "expectedRevision", "channelId", "rawStorage", "storedSource", "originalPath", "originalOrdinal", "profilePolicy"];
    let object = p.as_object().ok_or_else(|| bad("exact eight-field object required"))?;
    if object.len() != KEYS.len() || object.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err(bad("exact eight-field object required"));
    }
    if p.get("rawStorage").and_then(Value::as_bool) != Some(true) || p.get("storedSource").and_then(Value::as_bool) != Some(true) {
        return Err(bad("rawStorage/storedSource must be true"));
    }
    let target = CapturedStoredTarget {
        document: DocId(decimal(p, "documentId", true)?),
        revision: decimal(p, "expectedRevision", false)?,
        channel: ChannelId(decimal(p, "channelId", true)?),
    };
    if target.revision == u64::MAX {
        return Err(bad("revision exhausted"));
    }
    let ordinal = usize::try_from(decimal(p, "originalOrdinal", false)?).map_err(|_| bad("ordinal range"))?;
    if ordinal >= 10_000 {
        return Err(bad("ordinal must be below 10000"));
    }
    let path = p.get("originalPath").and_then(Value::as_str).ok_or_else(|| bad("explicit originalPath string required"))?;
    if path.is_empty() || path.len() > 4096 || path.contains('\0') {
        return Err(bad("bounded originalPath required"));
    }
    let policy = match p.get("profilePolicy").and_then(Value::as_str) {
        Some("originalExact") => CurrentProfilePolicy::OriginalExact,
        Some("materializedImplicit") => CurrentProfilePolicy::MaterializedImplicit,
        _ => return Err(bad("explicit originalExact/materializedImplicit profile policy required")),
    };
    Ok(Request { target, path, ordinal, policy })
}
fn limits(read_resident: u64, read_index: u64, read_work: u64) -> Result<HostLoadLimits, AutomationError> {
    Ok(HostLoadLimits {
        aggregate: Quota {
            resident_bytes: RESIDENT.checked_sub(read_resident).ok_or_else(|| bad("reader resident quota"))?,
            index_bytes: INDEX.checked_sub(read_index).ok_or_else(|| bad("reader index quota"))?,
            work_units: WORK.checked_sub(read_work).ok_or_else(|| bad("reader work quota"))?,
        },
        original: Quota { resident_bytes: 64 << 20, index_bytes: 2 << 20, work_units: 16_000_000 },
        mapping: photocraft_io::xcf_channel_candidate::Limits::default(),
        current: photocraft_format::PreparationLimits { resident_bytes: 64 << 20, index_bytes: 1 << 20, manifest_bytes: 64 << 20, object_bytes: 64 << 20 },
        selected: photocraft_io::SelectedLimits { resident_bytes: 128 << 20, index_bytes: 4 << 20, work_units: 1 << 32 },
        publication: photocraft_engine::xcf_channel_stored_load::Limits { resident_bytes: 128 << 20, index_bytes: 4 << 20, work_units: 1 << 31 },
    })
}
fn capture_view(view: &ChannelView, allowance: u64, ctl: &Interrupt<'_>) -> Result<ChannelView, AutomationError> {
    let copy = |input: &[bool], remaining: u64| -> Result<Vec<bool>, AutomationError> {
        let mut output = Vec::new();
        output.try_reserve_exact(input.len()).map_err(|_| bad("view allocation"))?;
        if output.capacity() as u64 > remaining {
            return Err(bad("actual view allocation exceeded allowance"));
        }
        for value in input {
            ctl.check().map_err(|_| AutomationError::Engine(photocraft_engine::EngineError::Cancelled))?;
            output.push(*value);
        }
        Ok(output)
    };
    let colors = copy(&view.color_hidden, allowance)?;
    let rest = allowance.checked_sub(colors.capacity() as u64).ok_or_else(|| bad("view allocation overflow"))?;
    let alphas = copy(&view.alpha_visible, rest)?;
    Ok(ChannelView {
        target: view.target,
        color_hidden: colors,
        alpha_visible: alphas,
        quick_mask_hidden: view.quick_mask_hidden,
        layer_mask: view.layer_mask,
        editing_mask: view.editing_mask,
    })
}
impl Headless {
    pub(crate) fn stored_selection_from_root(&mut self, params: &Value, wait: bool) -> Result<Value, AutomationError> {
        self.stored_selection_from_root_with_context(params, wait, &JobCtx::new())
    }
    /// Existing host authority plus a caller-owned cancellation context. No
    /// source path or permission is installed on the document or journal.
    pub fn stored_selection_from_root_with_context(&mut self, params: &Value, wait: bool, context: &JobCtx) -> Result<Value, AutomationError> {
        match &self.filesystem {
            Filesystem::Workspace(workspace) => load_stored_selection_from_root_with_context(&mut self.session, workspace, params, wait, context),
            _ => {
                parse(params, wait)?;
                context.check()?;
                Err(bad("held read-root capability required; no TrustedLocal fallback"))
            }
        }
    }
}
/// Borrow the live native or Headless session and an already-held read root.
/// No Session clone/move, ambient read fallback or persisted grant is performed.
/// The same JobCtx controls capture, read, independent proof and registered Load.
pub fn load_stored_selection_from_root_with_context(
    session: &mut photocraft_engine::Session,
    workspace: &crate::AuthorizedWorkspace,
    params: &Value,
    wait: bool,
    context: &JobCtx,
) -> Result<Value, AutomationError> {
    let request = parse(params, wait)?;
    context.check()?;
    if !workspace.has_read_root() {
        return Err(bad("held read-root capability is unavailable"));
    }
    let current = session.active().ok_or(photocraft_engine::EngineError::NoDocument)?;
    if current.doc.id != request.target.document
        || current.revision != request.target.revision
        || session.has_jobs()
        || current.floating.is_some()
        || current.coalesce.is_some()
        || current.doc.accounted().is_some()
        || current.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
    {
        return Err(bad("captured target changed or pending work exists"));
    }
    if current.channel_view.color_hidden.len() > 16 || current.channel_view.alpha_visible.len() > 10_000 {
        return Err(bad("view count limit"));
    }
    let view_bytes = add(current.channel_view.color_hidden.capacity() as u64, current.channel_view.alpha_visible.capacity() as u64)?;
    let view_allowance = add(add(current.channel_view.color_hidden.len() as u64, current.channel_view.alpha_visible.len() as u64)?, 1024)?;
    // Includes request/relative path, source Vec spare capacity, read scratch,
    // both view owners, generic job/quota/file headers and small JSON gate.
    // OS kernel/cache allocations and already-held root capability backing
    // remain outside this known-memory slice; no process RSS claim.
    let read_resident = add(65536 + 16384 + 4096, add((request.path.len() as u64) * 2, add(view_bytes, view_allowance)?)?)?;
    let read_index = add(65536, view_allowance)?;
    let read_work = add(200_000, add((request.path.len() as u64) * 2, view_allowance * 2)?)?;
    let engine_limits = limits(read_resident, read_index, read_work)?;
    // Use engine-owned actual ResourcePolicy, hidden context flags, generic
    // headers and exact Index-is-Resident SUM before copying or opening.
    photocraft_engine::xcf_channel_host_load::validate_host_request(session, request.target, 16384, &engine_limits)?;
    let quota = BackingQuota::new(QuotaLimits { resident: RESIDENT, index: INDEX, scratch: 0 });
    // Index is resident in this real quota too. Reserve the full published
    // aggregate once, before capture/capability-path allocation or reading.
    let _resident =
        quota.reserve(QuotaKind::Resident, RESIDENT.checked_sub(INDEX).ok_or_else(|| bad("published quota categories"))?).map_err(|e| bad(&e.to_string()))?;
    let _index = quota.reserve(QuotaKind::Index, INDEX).map_err(|e| bad(&e.to_string()))?;
    let snapshot = current.doc.clone();
    let weak = snapshot.downgrade();
    let active = current.active_layer;
    let view = context.stage(0.0, 0.03, "Capture Original Stored Load", |ctl| capture_view(&current.channel_view, view_allowance, ctl))?;
    // Authorize the exact engine step BEFORE I/O, and again normally through
    // Session::execute at publication. The host path is handled solely by
    // the capability reader, never passed as an ambient engine path.
    let step = json!({"channelId":params.get("channelId").and_then(Value::as_str),"documentId":params.get("documentId").and_then(Value::as_str),"expectedRevision":params.get("expectedRevision").and_then(Value::as_str),"rawStorage":true,"storedSource":true});
    authorize_engine_command(CMD, &step)?;
    if let Some(authorize) = session.authorize {
        authorize(CMD, &step)?;
    }
    let bytes = context.stage(0.03, 0.08, "Read Original Stored Channel", |ctl| workspace.read_xcf_stored_channel_source(request.path, ctl))?;
    if bytes.capacity() > 16384 {
        return Err(bad("actual original source capacity exceeded reader quota"));
    }
    context.check()?;
    let current = session.active().ok_or(photocraft_engine::EngineError::NoDocument)?;
    if current.doc.id != request.target.document
        || current.revision != request.target.revision
        || !weak.is_current(&current.doc)
        || !current.doc.shares_state(&snapshot)
        || current.active_layer != active
        || current.channel_view != view
        || session.has_jobs()
        || current.floating.is_some()
        || current.coalesce.is_some()
    {
        return Err(bad("target changed while reading original"));
    }
    // Return the registered result directly; no fallible processing after a
    // successful commit. All read owners and aggregate leases remain held.
    photocraft_engine::xcf_channel_host_load::load_from_original(session, request.target, &bytes, request.ordinal, engine_limits, request.policy, context)
        .map_err(AutomationError::Engine)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Value {
        json!({"documentId":"1","expectedRevision":"0","channelId":"2","rawStorage":true,"storedSource":true,"originalPath":"originals/own.xcf","originalOrdinal":"0","profilePolicy":"originalExact"})
    }
    #[test]
    fn exact_host_envelope_and_no_wait_refuse_before_job_polling() {
        assert!(parse(&request(), true).is_ok());
        assert!(parse(&request(), false).is_err());
        let mut p = request();
        p["channelId"] = json!(2);
        assert!(parse(&p, true).is_err());
        p = request();
        p["originalOrdinal"] = json!("00");
        assert!(parse(&p, true).is_err());
        p = request();
        p["extra"] = json!(true);
        assert!(parse(&p, true).is_err());
        let mut headless = Headless::trusted_local();
        assert!(headless.stored_selection_from_root(&request(), true).unwrap_err().to_string().contains("held read-root"));
        assert!(headless.session.documents().is_empty());
    }
    #[test]
    fn cancelled_context_never_reads_and_reader_reserve_reduces_engine_caps() {
        let ctx = JobCtx::new();
        ctx.cancel();
        let mut headless = Headless::new();
        assert!(matches!(
            headless.stored_selection_from_root_with_context(&request(), true, &ctx),
            Err(AutomationError::Engine(photocraft_engine::EngineError::Cancelled))
        ));
        let l = limits(128_000, 64_000, 200_000).unwrap();
        assert_eq!(l.aggregate.resident_bytes, RESIDENT - 128_000);
        assert_eq!(l.aggregate.work_units, WORK - 200_000);
    }
    #[test]
    fn denied_authorizer_refuses_before_opening_original_and_stale_target_never_reads() {
        fn deny(_: &str, _: &Value) -> photocraft_engine::Result<()> {
            Err(photocraft_engine::EngineError::BadParams { cmd: CMD.into(), msg: "Own denied before original read".into() })
        }
        let workspace = crate::AuthorizedWorkspace::new(Some(&std::env::temp_dir()), None).unwrap();
        let mut h = Headless::with_workspace(workspace);
        h.session.authorize = None;
        h.session.execute("file.new", json!({"width":1,"height":1})).unwrap();
        let before = h.session.active().unwrap().doc.clone();
        let revision = h.session.active().unwrap().revision;
        let mut p = request();
        p["documentId"] = json!(before.id.0.to_string());
        p["expectedRevision"] = json!(revision.to_string());
        h.session.authorize = Some(deny);
        assert!(h.stored_selection_from_root(&p, true).unwrap_err().to_string().contains("Own denied before original read"));
        p["expectedRevision"] = json!((revision + 1).to_string());
        assert!(h.stored_selection_from_root(&p, true).unwrap_err().to_string().contains("target changed"));
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(h.session.active().unwrap().revision, revision);
    }
    #[test]
    fn public_borrowed_api_refuses_missing_root_and_malformed_schema_without_moving_session() {
        let mut session = photocraft_engine::Session::new();
        let workspace = crate::AuthorizedWorkspace::default();
        let ctx = JobCtx::new();
        assert!(
            load_stored_selection_from_root_with_context(&mut session, &workspace, &request(), true, &ctx)
                .unwrap_err()
                .to_string()
                .contains("read-root capability is unavailable")
        );
        let mut p = request();
        p["channelId"] = json!(2);
        assert!(load_stored_selection_from_root_with_context(&mut session, &workspace, &p, true, &ctx).unwrap_err().to_string().contains("must be strings"));
        assert!(session.documents().is_empty());
        assert!(!workspace.has_read_root());
    }
}
