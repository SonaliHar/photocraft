//! Explicit current modern composition policy. Retained Auto resolution is
//! shown without recomputing it; the registered engine qualifies every edit.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{
    DocId, LayerContent, LayerId,
    import_compositing::{GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::ItemRef,
};
use photocraft_engine::{WeakDocumentSnapshot, channel_cmds::ChannelTarget, mask_view_cmds::LayerMaskView};
use serde_json::{Value, json};
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CompositionDraft {
    pub document: u64,
    pub revision: u64,
    pub layer: u64,
    pub source_mode: i32,
    pub blend_space: Option<u8>,
    pub composite_space: Option<u8>,
    pub operator: Option<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct CompositionOrigin {
    document: DocId,
    revision: u64,
    layer: LayerId,
    snapshot: WeakDocumentSnapshot,
    active_layer: Option<LayerId>,
    channel: ChannelTarget,
    editing_mask: Option<LayerId>,
    shown_mask: Option<LayerMaskView>,
    shell_mask: bool,
}
impl PartialEq for CompositionOrigin {
    fn eq(&self, other: &Self) -> bool {
        self.document == other.document
            && self.revision == other.revision
            && self.layer == other.layer
            && self.active_layer == other.active_layer
            && self.channel == other.channel
            && self.editing_mask == other.editing_mask
            && self.shown_mask == other.shown_mask
            && self.shell_mask == other.shell_mask
            && match (self.snapshot.upgrade(), other.snapshot.upgrade()) {
                (Some(a), Some(b)) => a.shares_state(&b),
                (None, None) => true,
                _ => false,
            }
    }
}
struct Current {
    mode: i32,
    legacy: bool,
    pass_through: bool,
    blend: i32,
    composite: i32,
    operator: i32,
    resolved: Option<ResolvedGimpComposition>,
}
fn stale() -> String {
    tl!("The composition target changed. Choose Edit again.").to_owned()
}
fn current(app: &PhotocraftApp, layer: LayerId) -> Option<Current> {
    let active = app.session.active()?;
    let state = active.doc.xcf_state.as_ref()?;
    if layer.0 == 0 || state.items.len() > 10_000 || state.source_items.len() > 10_000 || state.editable_indexed.len() > 10_000 {
        return None;
    }
    let mut items = state.items.iter().filter(|item| item.item == ItemRef::Layer(layer));
    let policy = items.next()?.composition.as_ref()?;
    if items.next().is_some() {
        return None;
    }
    Some(Current {
        mode: policy.source_mode,
        legacy: policy.legacy,
        pass_through: policy.pass_through,
        blend: policy.stored_blend_space,
        composite: policy.stored_composite_space,
        operator: policy.stored_operator,
        resolved: policy.resolved.clone(),
    })
}
fn mode_label(mode: i32) -> Option<&'static str> {
    Some(match mode {
        23 => "Overlay",
        28 => "Normal",
        30 => "Multiply",
        31 => "Screen",
        32 => "Difference",
        33 => "Linear Dodge (Add)",
        34 => "Subtract",
        35 => "Darken",
        36 => "Lighten",
        41 => "Divide",
        42 => "Color Dodge",
        43 => "Color Burn",
        44 => "Hard Light",
        45 => "Soft Light",
        48 => "Vivid Light",
        49 => "Pin Light",
        50 => "Linear Light",
        51 => "Hard Mix",
        52 => "Exclusion",
        53 => "Linear Burn",
        _ => return None,
    })
}
const EDITABLE_MODES: [i32; 20] = [23, 28, 30, 31, 32, 33, 34, 35, 36, 41, 42, 43, 44, 45, 48, 49, 50, 51, 52, 53];
fn owned_flat(app: &PhotocraftApp) -> bool {
    app.session.active().and_then(|state| state.doc.accounted()).is_some_and(|owner| owner.is_readonly_flat_composite_storage())
}
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    let active = app.session.active().ok_or_else(stale)?;
    if active.revision == u64::MAX
        || app.session.has_jobs()
        || app.ui.transform.is_some()
        || active.floating.is_some()
        || active.coalesce.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(tl!("Finish pending edits before changing composition.").to_owned());
    }
    Ok(())
}
fn eligibility(app: &PhotocraftApp, layer: LayerId, current: &Current) -> Result<(), String> {
    idle(app)?;
    let flat = owned_flat(app);
    if current.legacy || current.pass_through || (mode_label(current.mode).is_none() && !(flat && matches!(current.mode, 46 | 47))) {
        return Err(tl!("Editing this legacy or source-specific composition mode is unavailable.").to_owned());
    }
    let active = app.session.active().ok_or_else(stale)?;
    if flat && active.doc.mode == photocraft_doc::ColorMode::Grayscale {
        return Err(tl!("Composition editing for this Gray document is unavailable.").to_owned());
    }
    let state = active.doc.xcf_state.as_ref().ok_or_else(stale)?;
    photocraft_engine::xcf_item_locks::effective_layer_locks(&active.doc, layer)
        .and_then(|locks| locks.require(photocraft_engine::xcf_item_locks::EditKind::Content))
        .map_err(|error| error.to_string())?;
    if !active.doc.layer(layer).is_some_and(|layer| match &layer.content {
        LayerContent::Raster(_) => true,
        LayerContent::BackedRaster(backed) if flat => active.doc.accounted().is_some_and(|owner| owner.quota().shares_ledger(backed.quota())),
        _ => false,
    }) {
        return Err(tl!("Composition editing requires a pixel layer.").to_owned());
    }
    if state.source_items.iter().any(|source| source.owner == ItemRef::Layer(layer)) || state.editable_indexed.iter().any(|raster| raster.owner == layer) {
        return Err(tl!("Composition changes for editable source or indexed layers are unavailable.").to_owned());
    }
    Ok(())
}
fn capture(app: &PhotocraftApp, layer: LayerId) -> Result<CompositionOrigin, String> {
    let metadata = current(app, layer).ok_or_else(stale)?;
    eligibility(app, layer, &metadata)?;
    let active = app.session.active().ok_or_else(stale)?;
    Ok(CompositionOrigin {
        document: active.doc.id,
        revision: active.revision,
        layer,
        snapshot: active.doc.downgrade(),
        active_layer: active.active_layer,
        channel: active.channel_view.target,
        editing_mask: active.channel_view.editing_mask,
        shown_mask: active.channel_view.layer_mask,
        shell_mask: app.layer_mask_targeted(),
    })
}
fn stable(app: &PhotocraftApp, origin: &CompositionOrigin) -> bool {
    app.session.active().is_some_and(|active| {
        active.doc.id == origin.document
            && active.revision == origin.revision
            && origin.snapshot.upgrade().is_some_and(|original| original.shares_state(&active.doc))
            && active.active_layer == origin.active_layer
            && active.channel_view.target == origin.channel
            && active.channel_view.editing_mask == origin.editing_mask
            && active.channel_view.layer_mask == origin.shown_mask
            && app.layer_mask_targeted() == origin.shell_mask
    })
}
fn space_code(value: i32) -> Option<u8> {
    match value {
        1 => Some(1),
        2 => Some(2),
        4 => Some(4),
        _ => None,
    }
}
fn operator_code(value: i32) -> Option<u8> {
    u8::try_from(value).ok().filter(|value| (1..=4).contains(value))
}
fn initial(origin: &CompositionOrigin, current: &Current) -> CompositionDraft {
    CompositionDraft {
        document: origin.document.0,
        revision: origin.revision,
        layer: origin.layer.0,
        source_mode: current.mode,
        blend_space: space_code(current.blend),
        composite_space: space_code(current.composite),
        operator: operator_code(current.operator),
    }
}
fn space_wire(value: Option<u8>) -> Option<&'static str> {
    match value {
        Some(1) => Some("linear-rgb"),
        Some(2) => Some("profile-rgb"),
        Some(4) => Some("perceptual-rgb"),
        _ => None,
    }
}
fn operator_wire(value: Option<u8>) -> Option<&'static str> {
    match value {
        Some(1) => Some("union"),
        Some(2) => Some("clip-to-backdrop"),
        Some(3) => Some("clip-to-layer"),
        Some(4) => Some("intersection"),
        _ => None,
    }
}
fn space_label(value: Option<u8>) -> &'static str {
    match value {
        Some(1) => tl!("Linear RGB"),
        Some(2) => tl!("Profile RGB"),
        Some(4) => tl!("Perceptual RGB"),
        _ => tl!("Choose explicit space"),
    }
}
fn operator_label(value: Option<u8>) -> &'static str {
    match value {
        Some(1) => tl!("Union"),
        Some(2) => tl!("Clip to Backdrop"),
        Some(3) => tl!("Clip to Layer"),
        Some(4) => tl!("Intersection"),
        _ => tl!("Choose explicit operator"),
    }
}
fn stored_space(value: i32) -> &'static str {
    match value {
        1 => tl!("Linear RGB"),
        2 => tl!("Profile RGB"),
        3 => tl!("Lab (editing unavailable)"),
        4 => tl!("Perceptual RGB"),
        -4..=0 => tl!("Auto / retained default"),
        _ => tl!("Unsupported"),
    }
}
fn stored_operator(value: i32) -> &'static str {
    match value {
        1..=4 => operator_label(operator_code(value)),
        -2..=0 => tl!("Auto / retained default"),
        _ => tl!("Unsupported"),
    }
}
fn resolved_space(space: GimpSpace) -> &'static str {
    match space {
        GimpSpace::LinearRgb => tl!("Linear RGB"),
        GimpSpace::ProfileRgb => tl!("Profile RGB"),
        GimpSpace::PerceptualRgb => tl!("Perceptual RGB"),
        GimpSpace::Lab => tl!("Lab"),
    }
}
fn resolved_operator(operator: GimpOperator) -> &'static str {
    match operator {
        GimpOperator::Union => tl!("Union"),
        GimpOperator::ClipToBackdrop => tl!("Clip to Backdrop"),
        GimpOperator::ClipToLayer => tl!("Clip to Layer"),
        GimpOperator::Intersection => tl!("Intersection"),
    }
}
fn request(app: &PhotocraftApp, origin: &CompositionOrigin, draft: &CompositionDraft) -> Result<Value, String> {
    if !stable(app, origin) || draft.document != origin.document.0 || draft.revision != origin.revision || draft.layer != origin.layer.0 {
        return Err(stale());
    }
    let metadata = current(app, origin.layer).ok_or_else(stale)?;
    eligibility(app, origin.layer, &metadata)?;
    if draft.source_mode != metadata.mode && !owned_flat(app) {
        return Err(stale());
    }
    if mode_label(draft.source_mode).is_none() {
        return Err(tl!("Choose an explicit supported mode before applying.").to_owned());
    }
    let missing = || tl!("Choose explicit spaces and an operator before applying.").to_owned();
    let blend = space_wire(draft.blend_space).ok_or_else(missing)?;
    let composite = space_wire(draft.composite_space).ok_or_else(missing)?;
    let operator = operator_wire(draft.operator).ok_or_else(missing)?;
    Ok(json!({"layerId": origin.layer.0.to_string(), "documentId": origin.document.0.to_string(), "expectedRevision": origin.revision.to_string(),
        "sourceMode": draft.source_mode, "legacy":false, "blendSpace":blend, "compositeSpace":composite, "operator":operator}))
}
fn space_choice(ui: &mut egui::Ui, key: &'static str, label: &'static str, value: &mut Option<u8>) {
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(key).selected_text(space_label(*value)).show_ui(ui, |ui| {
            for code in [1, 2, 4] {
                ui.selectable_value(value, Some(code), space_label(Some(code)));
            }
        });
    });
}
pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    let Some(metadata) = current(app, layer) else {
        return;
    };
    let Some(active) = app.session.active() else {
        return;
    };
    let stamp = (active.doc.id.0, active.revision, layer.0);
    let flat = owned_flat(app);
    let editable_flat = flat && active.doc.mode == photocraft_doc::ColorMode::Rgb;
    let permission = eligibility(app, layer, &metadata);
    let mut draft = app
        .ui
        .xcf_composition_draft
        .take()
        .filter(|draft| (draft.document, draft.revision, draft.layer) == stamp && (flat || draft.source_mode == metadata.mode));
    if app.ui.xcf_composition_origin.as_ref().is_some_and(|origin| origin.layer != layer || !stable(app, origin)) {
        app.ui.xcf_composition_origin = None;
    }
    let witnessed = app.ui.xcf_composition_origin.as_ref().is_some_and(|origin| stable(app, origin));
    let mut edit = false;
    let mut apply = false;
    let mut reset = false;
    ui.push_id(("xcf_composition", stamp.0, layer.0), |ui| {
        ui.collapsing(tl!("GIMP Composition"), |ui| {
            ui.label(egui::RichText::new(tl!("Current mode")).color(Tokens::get(ui.ctx()).text));
            ui.label(if flat && metadata.mode == 46 {
                tl!("Grain Extract")
            } else if flat && metadata.mode == 47 {
                tl!("Grain Merge")
            } else {
                mode_label(metadata.mode).map(crate::i18n::t).unwrap_or(tl!("Source-specific mode"))
            });
            if metadata.legacy {
                ui.label(tl!("Legacy"));
            }
            for (label, value) in [
                (tl!("Blend space"), stored_space(metadata.blend)),
                (tl!("Composite space"), stored_space(metadata.composite)),
                (tl!("Operator"), stored_operator(metadata.operator)),
            ] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.label(value);
                });
            }
            if let Some(resolved) = &metadata.resolved {
                ui.label(tl!("Retained resolution"));
                ui.label(format!(
                    "{} · {} · {}",
                    resolved_space(resolved.blend_space),
                    resolved_space(resolved.composite_space),
                    resolved_operator(resolved.operator)
                ));
            } else {
                ui.label(tl!("No retained resolution"));
            }
            ui.label(tl!("Auto choices are retained metadata. Editing requires explicit choices; the project is checked when applying."));
            edit = ui.add_enabled(permission.is_ok(), egui::Button::new(tl!("Edit"))).clicked();
            if let Err(error) = &permission {
                ui.label(egui::RichText::new(error).color(Tokens::get(ui.ctx()).text_dim));
            }
            if let Some(value) = draft.as_mut() {
                if !witnessed {
                    ui.label(tl!("Choose Edit to capture the current layer before applying."));
                }
                ui.add_enabled_ui(witnessed && permission.is_ok(), |ui| {
                    if editable_flat {
                        ui.horizontal(|ui| {
                            ui.label(tl!("Mode"));
                            egui::ComboBox::from_id_salt("source_mode")
                                .selected_text(mode_label(value.source_mode).map(crate::i18n::t).unwrap_or(tl!("Choose explicit mode")))
                                .show_ui(ui, |ui| {
                                    for mode in EDITABLE_MODES {
                                        if let Some(label) = mode_label(mode) {
                                            ui.selectable_value(&mut value.source_mode, mode, crate::i18n::t(label));
                                        }
                                    }
                                });
                        });
                    }
                    space_choice(ui, "blend_space", tl!("Blend space"), &mut value.blend_space);
                    space_choice(ui, "composite_space", tl!("Composite space"), &mut value.composite_space);
                    ui.horizontal(|ui| {
                        ui.label(tl!("Operator"));
                        egui::ComboBox::from_id_salt("operator").selected_text(operator_label(value.operator)).show_ui(ui, |ui| {
                            for code in 1..=4 {
                                ui.selectable_value(&mut value.operator, Some(code), operator_label(Some(code)));
                            }
                        });
                    });
                    let complete = mode_label(value.source_mode).is_some()
                        && space_wire(value.blend_space).is_some()
                        && space_wire(value.composite_space).is_some()
                        && operator_wire(value.operator).is_some();
                    let changed = value.source_mode != metadata.mode
                        || value.blend_space.map(i32::from) != Some(metadata.blend)
                        || value.composite_space.map(i32::from) != Some(metadata.composite)
                        || value.operator.map(i32::from) != Some(metadata.operator);
                    ui.horizontal(|ui| {
                        apply = ui.add_enabled(complete && changed, egui::Button::new(tl!("Apply"))).clicked();
                        reset = ui.button(tl!("Reset")).clicked();
                    });
                });
            }
        });
    });
    if edit {
        match capture(app, layer) {
            Ok(origin) => {
                draft = Some(initial(&origin, &metadata));
                app.ui.xcf_composition_origin = Some(origin);
            }
            Err(error) => {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
    } else if reset {
        if let Some(origin) = app.ui.xcf_composition_origin.as_ref().filter(|origin| stable(app, origin)) {
            draft = Some(initial(origin, &metadata));
        }
    } else if apply {
        let result = app.ui.xcf_composition_origin.as_ref().zip(draft.as_ref()).ok_or_else(stale).and_then(|(origin, draft)| request(app, origin, draft));
        match result.and_then(|params| app.run("xcf.composition.set", params)) {
            Ok(_) => {
                draft = None;
                app.ui.xcf_composition_origin = None;
            }
            Err(error) => {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
    }
    app.ui.xcf_composition_draft = draft;
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Document, Layer, PixelFormat, SampleType, Size, XcfState, import_compositing::GimpComposition};
    fn fixture() -> (PhotocraftApp, LayerId) {
        let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
        let mut doc = Document::new("Own composition UI", Size::new(2, 1), ColorMode::Rgb, SampleType::F64);
        doc.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(photocraft_cms::builtin::Builtin::Srgb.profile().to_bytes());
        let mut layer = Layer::raster("Own policy", format);
        layer.surface_mut().unwrap().try_write_region_f64(doc.bounds(), &[0.25, 0.5, 0.75, 1., 0.75, 0.25, 0.5, 1.]).unwrap();
        let id = layer.id;
        doc.layers.push(layer);
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        for item in &mut state.items {
            item.source_flags = 0x8123_4567;
            item.source_tattoo = Some(55);
            item.raster_bounds = Some(doc.bounds());
            item.composition = Some(GimpComposition {
                compatibility_epoch: "own-policy-test".into(),
                source_mode: 28,
                legacy: false,
                stored_blend_space: 2,
                stored_composite_space: 2,
                stored_operator: 1,
                pass_through: false,
                resolved: Some(ResolvedGimpComposition {
                    blend_space: GimpSpace::ProfileRgb,
                    composite_space: GimpSpace::ProfileRgb,
                    operator: GimpOperator::Union,
                }),
            });
        }
        doc.xcf_state = Some(state);
        let mut session = photocraft_engine::Session::new();
        session.add_document(doc, None).unwrap();
        (PhotocraftApp::new(session, Default::default()), id)
    }
    #[test]
    fn explicit_policy_request_keeps_mode_and_lossless_target_pair() {
        let (app, layer) = fixture();
        let origin = capture(&app, layer).unwrap();
        let mut draft = initial(&origin, &current(&app, layer).unwrap());
        draft.blend_space = Some(1);
        let params = request(&app, &origin, &draft).unwrap();
        assert_eq!(params["layerId"], layer.0.to_string());
        assert_eq!(params["documentId"], origin.document.0.to_string());
        assert_eq!(params["expectedRevision"], origin.revision.to_string());
        assert_eq!(params["sourceMode"], 28);
        assert_eq!(params["legacy"], false);
        assert_eq!(params["blendSpace"], "linear-rgb");
    }
    #[test]
    fn retained_auto_or_lab_does_not_become_an_explicit_choice() {
        let (app, layer) = fixture();
        let origin = capture(&app, layer).unwrap();
        let mut metadata = current(&app, layer).unwrap();
        metadata.blend = 0;
        metadata.composite = 3;
        metadata.operator = -1;
        let draft = initial(&origin, &metadata);
        assert_eq!(draft.blend_space, None);
        assert_eq!(draft.composite_space, None);
        assert_eq!(draft.operator, None);
        assert!(request(&app, &origin, &draft).is_err());
        assert_eq!(metadata.resolved.unwrap().blend_space, GimpSpace::ProfileRgb);
    }
    #[test]
    fn restored_draft_has_no_runtime_edit_authority() {
        let (mut app, layer) = fixture();
        let origin = capture(&app, layer).unwrap();
        let draft = initial(&origin, &current(&app, layer).unwrap());
        app.ui.xcf_composition_draft = Some(serde_json::from_value(serde_json::to_value(draft).unwrap()).unwrap());
        assert!(app.ui.xcf_composition_origin.is_none());
        let before = app.session.active().unwrap().doc.clone();
        let mut harness = egui_kittest::Harness::builder().build_ui_state(move |ui, app| properties(app, ui, layer), app);
        harness.run_steps(2);
        assert!(harness.state().ui.xcf_composition_origin.is_none());
        assert!(harness.state().session.active().unwrap().doc.shares_state(&before));
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
    }
    #[test]
    fn stale_revision_view_and_current_mode_forgery_refuse_before_dispatch() {
        let (mut app, layer) = fixture();
        let origin = capture(&app, layer).unwrap();
        let mut draft = initial(&origin, &current(&app, layer).unwrap());
        draft.source_mode = 30;
        assert!(request(&app, &origin, &draft).is_err());
        draft.source_mode = 28;
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Color(0);
        assert!(request(&app, &origin, &draft).is_err());
        app.session.active_mut().unwrap().channel_view.target = origin.channel;
        app.session
            .edit("Own unrelated change", |document, _| {
                document.name = "Own edited".into();
                Ok(())
            })
            .unwrap();
        app.session.execute("edit.undo", json!({})).unwrap();
        assert!(request(&app, &origin, &draft).is_err());
        assert!(app.session.active().unwrap().history.can_redo());
    }
    #[test]
    fn registered_apply_preserves_native_words_and_source_flags_then_undoes() {
        let (mut app, layer) = fixture();
        let origin = capture(&app, layer).unwrap();
        let mut draft = initial(&origin, &current(&app, layer).unwrap());
        draft.operator = Some(3);
        let before = app.session.active().unwrap().doc.clone();
        let raw = before.layer(layer).unwrap().surface().unwrap().try_read_interleaved(before.bounds()).unwrap();
        let params = request(&app, &origin, &draft).unwrap();
        app.session.execute("xcf.composition.set", params).unwrap();
        let edited = app.session.active().unwrap().doc.clone();
        assert_eq!(edited.layer(layer).unwrap().surface().unwrap().try_read_interleaved(before.bounds()).unwrap(), raw);
        let item = edited.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == ItemRef::Layer(layer)).unwrap();
        assert_eq!(item.source_flags, 0x8123_4567);
        assert_eq!(item.source_tattoo, Some(55));
        assert_eq!(item.composition.as_ref().unwrap().source_mode, 28);
        app.session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*app.session.active().unwrap().doc, *before);
        app.session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*app.session.active().unwrap().doc, *edited);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "composition/flat_current_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod flat_current_tests;

#[cfg(test)]
pub(crate) fn capture_alpha_composition_test(app: &PhotocraftApp, layer: LayerId) -> Result<(), String> {
    capture(app, layer).map(|_| ())
}
