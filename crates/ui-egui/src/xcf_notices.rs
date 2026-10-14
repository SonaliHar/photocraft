//! Recorded imported metadata is not current semantic qualification. A requested
//! assessment uses the registered command once and publishes a snapshot-stamped
//! bounded notice; old import warnings and ordinary notice rendering remain intact.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{DocId, xcf_items::EditableSource, xcf_resources::ResourceFidelity};
use photocraft_engine::{document_snapshot::WeakDocumentSnapshot, jobs::{JobEvent, JobId, JobOutcome}};
use serde_json::{Value, json};

#[path = "xcf_panels/resource_details.rs"]
mod resource_details;
pub(crate) use resource_details::ResourcePageDraft;

const ASSESS: &str = "xcf.assess";
const LIMIT: &str = "xcf.assess.ui.limit";
const STALE: &str = "xcf.assess.ui.stale-target";
#[derive(Clone)]
pub(crate) struct Target { document: DocId, revision: u64, snapshot: WeakDocumentSnapshot }
struct View {
    target: Target, version: u32, resources: usize, unresolved: usize,
    substituted: usize,
    modified_text: usize, modified_vector: usize,
}
fn capture(app: &PhotocraftApp) -> Result<Option<View>, String> {
    let Some(active) = app.session.active() else { return Ok(None) };
    let Some(state) = active.doc.xcf_state.as_ref().filter(|state| state.provenance.format == "xcf") else { return Ok(None) };
    if state.resources.len() > 1000 || state.items.len() > 10_000 || state.source_items.len() > 10_000 { return Err(LIMIT.into()); }
    let mut view = View { target: Target { document: active.doc.id, revision: active.revision, snapshot: active.doc.downgrade() }, version: state.provenance.format_version, resources: state.resources.len(), unresolved: 0, substituted: 0, modified_text: 0, modified_vector: 0 };
    // No locator/name/resource payload/cache hashing or validation during a frame.
    for resource in &state.resources {
        match resource.resolution.as_ref().map(|resolution| resolution.fidelity) {
            Some(ResourceFidelity::Exact) => {},
            Some(ResourceFidelity::Substituted) => view.substituted += 1,
            None if resource.owned_bytes.is_some() => {},
            None => view.unresolved += 1,
        }
    }
    for source in &state.source_items {
        match source.source {
            EditableSource::Text { source_modified: true, .. } => view.modified_text += 1,
            EditableSource::Vector { source_modified: true, .. } => view.modified_vector += 1,
            _ => {}
        }
    }
    Ok(Some(view))
}
fn stable(app: &PhotocraftApp, target: &Target) -> bool {
    target.snapshot.upgrade().is_some_and(|original| app.session.active().is_some_and(|active|
        active.doc.id == target.document && active.revision == target.revision && original.shares_state(&active.doc)))
}
fn enabled(app: &PhotocraftApp) -> bool {
    app.jobs.xcf_assessment.is_none() && !app.session.has_jobs() && app.ui.transform.is_none() && app.session.active().is_some_and(|active|
        active.floating.is_none() && active.coalesce.is_none() && active.doc.xcf_state.as_ref().is_none_or(|state| state.floating.is_none()))
}
fn push(lines: &mut Vec<String>, text: &str) -> Result<(), String> {
    let mut owned = String::new();
    owned.try_reserve_exact(text.len()).map_err(|_| LIMIT)?;
    owned.push_str(text);
    lines.push(owned);
    Ok(())
}
fn decimal(text: &str) -> bool {
    !text.is_empty() && text.len() <= 20 && !text.starts_with('0') && text.bytes().all(|byte| byte.is_ascii_digit()) && text.parse::<u64>().is_ok()
}
fn report_lines(target: &Target, report: &Value) -> Result<(Vec<String>, bool), String> {
    let fidelity = report.get("fidelity").and_then(Value::as_str).filter(|value| matches!(*value, "full" | "reduced" | "dependency_limited")).ok_or(LIMIT)?;
    if report.get("sourceFormat").and_then(Value::as_str) != Some("xcf") { return Err(LIMIT.into()); }
    let dependencies = report.get("dependencies").and_then(Value::as_array).filter(|rows| rows.len() <= 1000).ok_or(LIMIT)?;
    let diagnostics = report.get("diagnostics").and_then(Value::as_array).filter(|rows| rows.len() <= 1000).ok_or(LIMIT)?;
    let mut lines = Vec::new();
    lines.try_reserve_exact(20).map_err(|_| LIMIT)?;
    push(&mut lines, &format!("{} {} · {} {}", crate::i18n::t("Document"), target.document.0, crate::i18n::t("Revision"), target.revision))?;
    let fidelity_label = match fidelity {
        "full" => crate::i18n::t("Full fidelity"),
        "reduced" => crate::i18n::t("Reduced fidelity"),
        _ => crate::i18n::t("Fidelity limited by dependencies"),
    };
    push(&mut lines, fidelity_label)?;
    // Notice cards show only a short prefix. Put the fidelity explanation and
    // native preservation route before individual dependency rows.
    for diagnostic in diagnostics {
        if lines.len() == 19 { break; }
        let message = match diagnostic.get("code").and_then(Value::as_str) {
            Some("missing_resource") => "A retained dependency is unavailable; its stored locator grants no read permission.",
            Some("resource_not_activated") => "Supplied bytes have not established an exact active resource consumer.",
            Some("substituted_resource") => "An explicitly substituted dependency lowers this project's fidelity.",
            _ => continue,
        };
        // Deliberately do not copy report messages, locators, payloads or actions.
        if !lines.iter().any(|line| line == message) { push(&mut lines, message)?; }
    }
    if fidelity == "reduced" && !lines.iter().any(|line| line.contains("substituted dependency")) {
        push(&mut lines, "Current edits or explicit substitutions reduce this project's fidelity.")?;
    } else if fidelity == "dependency_limited" && !lines.iter().any(|line| line.contains("unavailable") || line.contains("active resource consumer")) {
        push(&mut lines, "Unavailable or inactive dependencies limit this project's fidelity.")?;
    }
    push(&mut lines, "Use Save As to keep retained project data in a native .pcraft copy.")?;
    push(&mut lines, "This assessment applies only to the document revision shown above.")?;
    let mut displayed_dependencies=0usize;
    for row in dependencies.iter().take(16) {
        let id = row.get("resourceId").and_then(Value::as_str).filter(|value| decimal(value)).ok_or(LIMIT)?;
        let kind = row.get("kind").and_then(Value::as_str).filter(|value| matches!(*value, "font" | "pattern" | "gradient" | "brush" | "palette" | "color-profile" | "linked-image")).ok_or(LIMIT)?;
        let status = row.get("status").and_then(Value::as_str).filter(|value| matches!(*value, "exact_available" | "unresolved" | "pending_activation" | "substituted")).ok_or(LIMIT)?;
        let consumer = row.get("consumerAvailable").and_then(Value::as_bool).ok_or(LIMIT)?;
        let kind_label = crate::i18n::t(match kind { "font" => "Font", "pattern" => "Pattern", "gradient" => "Gradient", "brush" => "Brush", "palette" => "Palette", "color-profile" => "Color Profile", _ => "Linked Image" });
        let status_label = crate::i18n::t(match status { "exact_available" => "Available", "unresolved" => "Unavailable", "pending_activation" => "Awaiting activation", _ => "Explicit substitution" });
        if status == "exact_available" && !consumer { return Err(LIMIT.into()); }
        if lines.len() >= 19 { break; }
        push(&mut lines, &format!("{} {id} · {kind_label} · {status_label}", crate::i18n::t("Resource")))?;
        displayed_dependencies+=1;
    }
    if dependencies.len() > displayed_dependencies || diagnostics.len() > 3 { push(&mut lines, "…")?; }
    Ok((lines, fidelity != "full"))
}
fn publish(app: &mut PhotocraftApp, target: &Target, report: &Value) -> Result<(), String> {
    if !stable(app, target) { return Err(STALE.into()); }
    let (lines, warning) = report_lines(target, report)?;
    crate::notices::post(app, tl!("Metadata"), lines, warning, None);
    Ok(())
}
fn assess(app: &mut PhotocraftApp, target: Target) -> Result<(), String> {
    if !enabled(app) || !stable(app, &target) { return Err(STALE.into()); }
    let result = app.run(ASSESS, json!({}))?;
    if result.get("pending").and_then(Value::as_bool) == Some(true) {
        let id = result.get("job").and_then(Value::as_u64).filter(|id| *id != 0).ok_or(LIMIT)?;
        app.jobs.xcf_assessment = Some((JobId(id), target));
        return Ok(());
    }
    publish(app, &target, &result)
}
fn status_error(app:&mut PhotocraftApp,error:&str) {
    app.ui.status=match error {
        STALE=>tl!("The document changed. Request Details again."),
        LIMIT=>tl!("The project details exceed the supported display limit."),
        _=>tl!("The current project could not be assessed. Try Details again after finishing pending edits."),
    }.into();
    app.ui.status_error=true;
}
/// Assessment failures are not a reduced/success classification. Do not echo
/// underlying messages, source paths, payloads or guessed recovery actions.
fn failure_notice(app:&mut PhotocraftApp,target:&Target,cancelled:bool)->Result<(),String> {
    if !stable(app,target) {return Err(STALE.into());}
    let message=if cancelled {tl!("Project assessment was cancelled. Your document was not changed.")}
        else {tl!("The current project could not be assessed. Your document was not changed.")};
    let mut lines=Vec::new();lines.try_reserve_exact(4).map_err(|_|LIMIT)?;
    push(&mut lines,&format!("{} {} · {} {}",crate::i18n::t("Document"),target.document.0,crate::i18n::t("Revision"),target.revision))?;
    push(&mut lines,message)?;
    push(&mut lines,"Request Details again after finishing pending edits.")?;
    push(&mut lines,"Use Save As to keep retained project data in a native .pcraft copy.")?;
    crate::notices::post(app,tl!("Project assessment"),lines,!cancelled,None);
    app.ui.status=message.into();app.ui.status_error=!cancelled;Ok(())
}
/// Called after ordinary control waiters are serviced, before generic job notices.
/// A stale result is discarded; it cannot become the active document's fidelity.
pub(crate) fn on_job(app: &mut PhotocraftApp, event: &JobEvent) -> bool {
    if event.command != ASSESS {return false;}
    // Waiters already receive the actual result. Untracked/stale assessment
    // events cannot fall through to generic notices that echo failure payloads.
    if !app.jobs.xcf_assessment.as_ref().is_some_and(|(id,_)|*id==event.id) {return true;}
    let Some((_, target)) = app.jobs.xcf_assessment.take() else { return false };
    let result = match &event.outcome {
        JobOutcome::Done(report) => publish(app, &target, report),
        JobOutcome::Failed(_) => failure_notice(app,&target,false),
        JobOutcome::Cancelled => failure_notice(app,&target,true),
    };
    if let Err(error) = result { status_error(app,&error); }
    true
}
/// Document Properties once, above per-layer source controls. Recorded rows are
/// flags only; Details requests actual current fidelity once. Native Save As uses
/// the existing platform dialog and source-preserving .pcraft suggestion.
pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.jobs.xcf_assessment.as_ref().is_some_and(|(_, target)| !stable(app, target)) { app.jobs.xcf_assessment = None;status_error(app,STALE); }
    let view = match capture(app) { Ok(Some(view)) => view, Ok(None) => return, Err(error) => { ui.label(if error==LIMIT {tl!("The project metadata exceeds the supported display limit.")}else {tl!("The project metadata is unavailable.")}); return } };
    let interactive = enabled(app);
    ui.separator();
    ui.label(egui::RichText::new(tl!("Metadata")).color(Tokens::get(ui.ctx()).text));
    ui.horizontal(|ui| { ui.label(tl!("Source")); ui.label(format!("XCF {}", view.version)); });
    for (label, count) in [(crate::i18n::t("Resources"), view.resources), (crate::i18n::t("Unavailable"), view.unresolved), (crate::i18n::t("Explicit substitutions"), view.substituted)] {
        ui.horizontal(|ui| { ui.label(label); ui.label(count.to_string()); });
    }
    if view.modified_text != 0 { ui.horizontal(|ui| { ui.label(tl!("Type")); ui.label(tl!("modified")); ui.label(view.modified_text.to_string()); }); }
    if view.modified_vector != 0 { ui.horizontal(|ui| { ui.label(tl!("Shape")); ui.label(tl!("modified")); ui.label(view.modified_vector.to_string()); }); }
    ui.label(tl!("Recorded import metadata. Details checks the current project."));
    resource_details::properties(app, ui);
    ui.horizontal(|ui| { ui.label(tl!("Native")); ui.label(".pcraft"); });
    let details = ui.add_enabled(interactive, egui::Button::new(tl!("Details"))).clicked();
    let save = ui.add_enabled(interactive, egui::Button::new(tl!("Save As…"))).clicked();
    if details {
        let target=view.target;
        if let Err(error)=assess(app,target.clone()) {
            if error==STALE || error==LIMIT {status_error(app,&error);}
            else if let Err(error)=failure_notice(app,&target,false) {status_error(app,&error);}
        }
    } else if save {
        if !stable(app,&view.target) {status_error(app,STALE);return;}
        if let Err(error) = crate::menus::invoke(app, ui.ctx(), "file.saveAs", json!({"format":"pcraft"})) { app.ui.status = error; app.ui.status_error = true; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use photocraft_doc::{Document, XcfState, item_identity::ResourceId, xcf_resources::{ResourceKind, ResourceReference}};
    fn app() -> PhotocraftApp {
        let mut document = Document::new("Own retained source", photocraft_doc::Size::new(2, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let mut state = XcfState::default();
        state.provenance.format = "xcf".into();
        state.provenance.format_version = 22;
        state.provenance.container = "plain".into();
        state.resources.push(ResourceReference { id: ResourceId(1), kind: ResourceKind::LinkedImage, name: "private-source-name".into(), collection: None, internal: false, locator: Some("../private-must-not-be-read.png".into()), content_hash: None, owned_bytes: None, resolution: None });
        document.xcf_state = Some(state);
        document.validate_xcf_state().unwrap();
        let mut session = photocraft_engine::Session::new();
        session.open_document(document, None).unwrap();
        PhotocraftApp::new(session, crate::Services::default())
    }
    fn report() -> Value {
        json!({"sourceFormat":"xcf","fidelity":"dependency_limited","dependencies":[{"resourceId":"1","kind":"linked-image","status":"unresolved","consumerAvailable":false,"locator":"private-locator"}],"diagnostics":[{"code":"missing_resource","message":"private-message","payload":"private-payload"}],"container":"private-container"})
    }
    fn event(id: JobId, target: &Target, outcome: JobOutcome) -> JobEvent {
        JobEvent { id, command: ASSESS.into(), label: "Own synthetic completion".into(), document: Some(target.document), outcome }
    }
    #[test]
    fn recorded_frame_never_assesses_or_reads_resources_or_changes_native_document() {
        let app = app();
        let before = app.session.active().unwrap().doc.clone();
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let mut harness = Harness::builder().with_size(egui::vec2(650., 300.)).build_ui_state(|ui, app| properties(app, ui), app);
        harness.run_steps(3);
        assert!(harness.query_all_by_label("Metadata").next().is_some());
        assert!(harness.query_all_by_label("Details").next().is_some());
        assert!(harness.query_all_by_label("Save As…").next().is_some());
        assert!(harness.state().ui.notices.is_empty());
        assert!(harness.state().jobs.xcf_assessment.is_none());
        assert!(harness.state().session.active().unwrap().doc.shares_state(&before));
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
        assert_eq!(photocraft_format::save_to_bytes(&harness.state().session.active().unwrap().doc, &Default::default()).unwrap(), native);
        let view = capture(harness.state()).unwrap().unwrap();
        assert_eq!(view.unresolved, 1);
        assert_eq!(view.substituted, 0);
    }
    #[test]
    fn bounded_report_rows_whitelist_codes_without_echoing_source_payloads() {
        let app = app();
        let target = capture(&app).unwrap().unwrap().target;
        let (lines, warning) = report_lines(&target, &report()).unwrap();
        assert!(warning);
        assert!(lines.iter().any(|line| line.contains("Fidelity limited by dependencies")));
        assert!(lines.iter().any(|line| line.contains("Unavailable")));
        assert!(lines.iter().all(|line| !line.contains("private-")));
        let mut excessive = report();
        excessive["dependencies"] = json!(vec![json!({}); 1001]);
        assert!(report_lines(&target, &excessive).is_err());
        let mut invalid = report();
        invalid["dependencies"][0]["resourceId"] = json!("01");
        assert!(report_lines(&target, &invalid).is_err());
        invalid = report();
        invalid["fidelity"] = json!("exact-by-filename");
        assert!(report_lines(&target, &invalid).is_err());
        let mut bounded = report();
        bounded["dependencies"] = json!(vec![report()["dependencies"][0].clone(); 1000]);
        let (lines, _) = report_lines(&target, &bounded).unwrap();
        assert!(lines.len() <= 20);
        assert_eq!(lines.last().unwrap(), "…");
    }
    #[test]
    fn matching_synthetic_job_publishes_only_snapshot_notice_and_cancel_clears_target() {
        let mut app = app();
        let target = capture(&app).unwrap().unwrap().target;
        let completion = event(JobId(7), &target, JobOutcome::Done(report()));
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        app.jobs.xcf_assessment = Some((JobId(7), target));
        assert!(on_job(&mut app, &completion));
        assert!(app.jobs.xcf_assessment.is_none());
        assert_eq!(app.ui.notices.len(), 1);
        assert!(app.ui.notices[0].lines[0].contains(&format!("Revision {revision}")));
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_eq!(app.session.active().unwrap().history.past_len(), 0);
        let target = capture(&app).unwrap().unwrap().target;
        let cancel = event(JobId(8), &target, JobOutcome::Cancelled);
        app.jobs.xcf_assessment = Some((JobId(8), target));
        assert!(on_job(&mut app, &cancel));
        assert!(app.jobs.xcf_assessment.is_none());
        assert_eq!(app.ui.notices.len(), 2);
        assert!(!app.ui.status_error);
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
    #[test]
    fn edit_then_undo_revision_prevents_old_job_result_claim_even_when_weak_matches() {
        let mut app = app();
        let target = capture(&app).unwrap().unwrap().target;
        let completion = event(JobId(7), &target, JobOutcome::Done(report()));
        app.session.edit("Own revision seed", |document, _| { document.name = "Own changed name".into(); Ok(()) }).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        assert!(app.session.active().unwrap().history.can_redo());
        let revision = app.session.active().unwrap().revision;
        app.jobs.xcf_assessment = Some((JobId(7), target));
        assert!(on_job(&mut app, &completion));
        assert!(app.ui.notices.is_empty());
        assert!(app.jobs.xcf_assessment.is_none());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert!(app.session.active().unwrap().history.can_redo());
        assert_eq!(app.ui.status,tl!("The document changed. Request Details again."));
    }
    #[test]
    fn native_save_as_uses_existing_picker_and_cancel_preserves_source() {
        use crate::file_dialog::{self, FileDialogRequest};
        let mut app = app();
        let before = app.session.active().unwrap().doc.clone();
        let (picker, requests) = file_dialog::fake(vec![None]);
        app.services.file_dialog = Some(picker);
        let mut harness = Harness::builder().with_size(egui::vec2(650., 300.)).build_ui_state(|ui, app| properties(app, ui), app);
        harness.run();
        harness.get_by_label("Save As…").click();
        harness.run();
        harness.state_mut().poll_file_dialog(&egui::Context::default(), None);
        assert!(matches!(requests.borrow().as_slice(), [FileDialogRequest::Save { suggested }] if suggested.ends_with(".pcraft")));
        assert!(harness.state().session.active().unwrap().doc.shares_state(&before));
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
    }
    #[test]
    fn material_dependency_guidance_and_native_save_fit_visible_notice_prefix() {
        let app=app();let target=capture(&app).unwrap().unwrap().target;let mut assessment=report();
        assessment["dependencies"]=json!(vec![report()["dependencies"][0].clone();1000]);
        let (lines,warning)=report_lines(&target,&assessment).unwrap();assert!(warning);
        assert!(lines.iter().take(8).any(|line|line.contains("unavailable")));
        assert!(lines.iter().take(8).any(|line|line.contains("native .pcraft copy")));
        assert!(lines.iter().all(|line|!line.contains("private-")));assert!(lines.len()<=20);
    }
    #[test]
    fn failed_job_never_claims_fidelity_or_echoes_private_error_and_retains_redo() {
        let mut app=app();app.session.edit("Own redo seed",|doc,_|{doc.name="Own edited".into();Ok(())}).unwrap();app.run("edit.undo",json!({})).unwrap();
        let target=capture(&app).unwrap().unwrap().target;let before=app.session.active().unwrap().doc.clone();let revision=app.session.active().unwrap().revision;
        let completion=event(JobId(77),&target,JobOutcome::Failed("private-locator/payload-notice".into()));app.jobs.xcf_assessment=Some((JobId(77),target));
        assert!(on_job(&mut app,&completion));assert!(app.jobs.xcf_assessment.is_none());assert!(app.ui.status_error);assert_eq!(app.ui.notices.len(),1);
        assert!(app.ui.notices[0].lines.iter().any(|line|line.contains("could not be assessed")));assert!(app.ui.notices[0].lines.iter().all(|line|!line.contains("private-") && !line.contains("Full fidelity") && !line.contains("Reduced fidelity")));
        assert!(app.session.active().unwrap().doc.shares_state(&before));assert_eq!(app.session.active().unwrap().revision,revision);assert!(app.session.active().unwrap().history.can_redo());
    }

    #[test]
    fn untracked_assessment_failure_never_echoes_into_generic_notice_flow() {
        let mut app=app();let target=capture(&app).unwrap().unwrap().target;let before=app.session.active().unwrap().doc.clone();
        let completion=event(JobId(999),&target,JobOutcome::Failed("private-locator/not-active-report".into()));
        assert!(on_job(&mut app,&completion));assert!(app.ui.notices.is_empty());assert!(app.session.active().unwrap().doc.shares_state(&before));
    }

}
