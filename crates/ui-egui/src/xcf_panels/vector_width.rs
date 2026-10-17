//! Discrete current stroke-width edits. Metadata decides which control is
//! visible; only the registered command qualifies the source and renderer.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{
    Document, LayerId,
    item_identity::{ItemRef, PathId},
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
    xcf_items::EditableSource,
};
use photocraft_engine::{
    document_snapshot::WeakDocumentSnapshot,
    xcf_item_locks::{EditKind, effective_layer_locks},
};
use serde_json::{Value, json};
const COMMAND: &str = "xcf.vector.stroke.width.set";
const WIDTHS: [(u8, u32); 9] =
    [(1, 0x3f800000), (2, 0x40000000), (3, 0x40400000), (4, 0x40800000), (5, 0x40a00000), (6, 0x40c00000), (7, 0x40e00000), (8, 0x41000000), (9, 0x41100000)];
struct Target {
    document: photocraft_doc::DocId,
    revision: u64,
    layer: LayerId,
    active: Option<LayerId>,
    path: PathId,
    width: u32,
    snapshot: WeakDocumentSnapshot,
}
fn field<'a>(values: &'a [FilterArgument], name: &str) -> Option<&'a FilterValue> {
    let mut matches = values.iter().filter(|value| value.name == name);
    let value = &matches.next()?.value;
    matches.next().is_none().then_some(value)
}
// Borrowed, bounded metadata only: no cache digest, path tessellation or pixels.
fn current(document: &Document, layer: LayerId) -> Option<(PathId, u32)> {
    let state = document.xcf_state.as_ref()?;
    if layer.0 == 0 || state.source_items.len() > 10_000 || document.paths.len() > 10_000 {
        return None;
    }
    let mut sources = state.source_items.iter().filter(|source| source.owner == ItemRef::Layer(layer));
    let source = sources.next()?;
    if sources.next().is_some() {
        return None;
    }
    let EditableSource::Vector { path, source_modified: false, configuration } = &source.source else {
        return None;
    };
    if path.0 == 0
        || configuration.len() > 5
        || document.paths.iter().filter(|entry| entry.id == *path).count() != 1
        || field(configuration, "filled") != Some(&FilterValue::Bool(false))
        || field(configuration, "stroked") != Some(&FilterValue::Bool(true))
    {
        return None;
    }
    let FilterValue::Config(stroke) = field(configuration, "stroke")? else {
        return None;
    };
    if stroke.len() > 6 {
        return None;
    }
    let FilterValue::Number(ExactNumber::Float32Bits(bits)) = field(stroke, "width")? else {
        return None;
    };
    f32::from_bits(*bits).is_finite().then_some((*path, *bits))
}
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    let active = app.session.active().ok_or_else(|| tl!("No document is open.").to_owned())?;
    if app.ui.transform.is_some()
        || app.session.has_jobs()
        || active.floating.is_some()
        || active.coalesce.is_some()
        || active.revision == u64::MAX
        || active.doc.accounted().is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(tl!("Finish pending edits before changing the stroke width.").to_owned());
    }
    Ok(())
}
fn permitted(app: &PhotocraftApp, layer: LayerId) -> Result<(), String> {
    idle(app)?;
    let active = app.session.active().ok_or_else(|| tl!("No document is open.").to_owned())?;
    let locks = effective_layer_locks(&active.doc, layer).map_err(|error| error.to_string())?;
    locks.require(EditKind::Content).and_then(|()| locks.require(EditKind::Position)).map_err(|error| error.to_string())
}
fn capture(app: &PhotocraftApp, layer: LayerId) -> Option<Target> {
    let active = app.session.active()?;
    let (path, width) = current(&active.doc, layer)?;
    Some(Target { document: active.doc.id, revision: active.revision, layer, active: active.active_layer, path, width, snapshot: active.doc.downgrade() })
}
fn params(layer: LayerId, path: PathId, bits: u32) -> Result<Value, String> {
    if layer.0 == 0 || path.0 == 0 || !WIDTHS.iter().any(|(_, width)| *width == bits) {
        return Err(tl!("Choose a supported stroke width.").to_owned());
    }
    Ok(json!({"layerId":layer.0.to_string(),"pathId":path.0.to_string(),"widthF32Bits":format!("{bits:08x}")}))
}
fn apply(app: &mut PhotocraftApp, target: &Target, bits: u32) -> Result<(), String> {
    permitted(app, target.layer)?;
    let active = app.session.active().ok_or_else(|| tl!("No document is open.").to_owned())?;
    let original = target.snapshot.upgrade().ok_or_else(|| tl!("The stroke target changed. Choose its properties again.").to_owned())?;
    if active.doc.id != target.document
        || active.revision != target.revision
        || active.active_layer != target.active
        || !original.shares_state(&active.doc)
        || current(&active.doc, target.layer) != Some((target.path, target.width))
    {
        return Err(tl!("The stroke target changed. Choose its properties again.").to_owned());
    }
    let request = params(target.layer, target.path, bits)?;
    // Shared checked dispatch is immediate; no focus change or await intervenes.
    super::run_checked(app, &original, target.revision, COMMAND, request);
    Ok(())
}
pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    let Some(target) = capture(app, layer) else {
        return;
    };
    let permission = permitted(app, layer);
    let mut selected = target.width;
    ui.push_id(("xcf_vector_width", layer.0), |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Stroke Width")).color(Tokens::get(ui.ctx()).text_dim));
            let response = ui
                .add_enabled_ui(permission.is_ok(), |ui| {
                    egui::ComboBox::from_id_salt("width").selected_text(f32::from_bits(target.width).to_string()).show_ui(ui, |ui| {
                        for (label, bits) in WIDTHS {
                            ui.selectable_value(&mut selected, bits, label.to_string());
                        }
                    });
                })
                .response;
            if let Err(reason) = &permission {
                response.on_hover_text(reason);
            }
        });
    });
    if selected != target.width
        && let Err(error) = apply(app, &target, selected)
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_discrete_width_words_and_high_identity_payload() {
        for (number, bits) in WIDTHS {
            assert_eq!(bits, f32::from(number).to_bits());
            let value = params(LayerId(u64::MAX), PathId(u64::MAX - 1), bits).unwrap();
            assert_eq!(value["layerId"], u64::MAX.to_string());
            assert_eq!(value["pathId"], (u64::MAX - 1).to_string());
            assert_eq!(value["widthF32Bits"], format!("{bits:08x}"));
            assert_eq!(value.as_object().unwrap().len(), 3);
        }
        assert!(params(LayerId(1), PathId(2), 0).is_err());
        assert!(params(LayerId(0), PathId(2), 1f32.to_bits()).is_err());
    }
    #[test]
    #[ignore = "retained qualified stroke fixture continuation draft; not executed"]
    fn registered_width_edit_preserves_original_cache_and_stale_undo_target_refuses() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("qualified vector fixture"));
        let document = photocraft_io::import("source.xcf", &std::fs::read(root.join("vector-stroke-evidence/source.xcf")).unwrap()).unwrap().document;
        let layer = document
            .xcf_state
            .as_ref()
            .unwrap()
            .source_items
            .iter()
            .find_map(|source| match (&source.owner, &source.source) {
                (ItemRef::Layer(layer), EditableSource::Vector { source_modified: false, .. }) if current(&document, *layer).is_some() => Some(*layer),
                _ => None,
            })
            .unwrap();
        let mut session = photocraft_engine::Session::new();
        session.open_document(document, None).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        let target = capture(&app, layer).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        // This genuine packet is the no-AA Butt/Miter width6 source; its
        // independently qualified width continuation is8, not the AA width2 cohort.
        assert_eq!(target.width, 6f32.to_bits());
        assert!(app.session.execute(COMMAND, params(layer, target.path, 2f32.to_bits()).unwrap()).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(app.session.active().unwrap().history.past_len(), 0);
        let bits = 8f32.to_bits();
        apply(&mut app, &target, bits).unwrap();
        assert!(!app.ui.status_error, "{}", app.ui.status);
        let after = app.session.active().unwrap().doc.clone();
        assert_eq!(current(&after, layer), Some((target.path, bits)));
        assert_eq!(after.paths, before.paths);
        assert_eq!(after.xcf_state.as_ref().unwrap().provenance, before.xcf_state.as_ref().unwrap().provenance);
        for (original, edited) in before.xcf_state.as_ref().unwrap().source_items.iter().zip(&after.xcf_state.as_ref().unwrap().source_items) {
            assert_eq!(original.cache, edited.cache);
        }
        app.run("edit.undo", json!({})).unwrap();
        let restored = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        assert!(apply(&mut app, &target, bits).is_err());
        assert!(app.session.active().unwrap().history.can_redo());
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert!(app.session.active().unwrap().doc.shares_state(&restored));
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, after);
    }
    #[test]
    fn duplicate_width_metadata_is_not_a_selector() {
        let values = vec![
            FilterArgument { name: "width".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits())) },
            FilterArgument { name: "width".into(), value: FilterValue::Number(ExactNumber::Float32Bits(2f32.to_bits())) },
        ];
        assert!(field(&values, "width").is_none());
    }
}
