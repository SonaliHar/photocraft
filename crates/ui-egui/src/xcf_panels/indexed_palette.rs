//! Exact current palette bytes. Drafts are view state, never resource authority.
use crate::{PhotocraftApp, state::XcfIndexedPaletteDraft, theme::Tokens};
use photocraft_doc::{ColorMode, DocId, LayerId, SampleType};
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Value, json};

const COMMAND: &str = "xcf.indexed.palette.set";
const STALE: &str = "xcf.indexed.palette.ui.stale-target";
#[derive(Clone)]
struct Target {
    document: DocId,
    revision: u64,
    layer: LayerId,
    snapshot: WeakDocumentSnapshot,
}
struct Palette {
    colors: [[u8; 3]; 256],
    count: usize,
    transparent: Option<u8>,
}
fn capture(app: &PhotocraftApp, layer: LayerId) -> Option<(Target, Palette)> {
    let active = app.session.active()?;
    let doc = &active.doc;
    if doc.id.0 == 0 || layer.0 == 0 || doc.mode != ColorMode::Indexed || doc.depth != SampleType::U8 || doc.accounted().is_some() {
        return None;
    }
    let state = doc.xcf_state.as_ref()?;
    if state.editable_indexed.is_empty() || state.editable_indexed.len() > 10_000 {
        return None;
    }
    let mut owners = state.editable_indexed.iter().filter(|entry| entry.owner == layer);
    owners.next()?;
    if owners.next().is_some() {
        return None;
    }
    let table = doc.color_table.as_ref()?;
    if table.colors.is_empty() || table.colors.len() > 256 || table.transparent.is_some_and(|slot| usize::from(slot) >= table.colors.len()) {
        return None;
    }
    let mut colors = [[0; 3]; 256];
    for (out, source) in colors.iter_mut().zip(&table.colors) {
        *out = *source;
    }
    Some((
        Target { document: doc.id, revision: active.revision, layer, snapshot: doc.downgrade() },
        Palette { colors, count: table.colors.len(), transparent: table.transparent },
    ))
}
fn same_target(app: &PhotocraftApp, target: &Target) -> bool {
    app.session.active().is_some_and(|active| {
        active.doc.id == target.document
            && active.revision == target.revision
            && target.snapshot.upgrade().is_some_and(|original| original.shares_state(&active.doc))
    })
}
fn editable(app: &PhotocraftApp) -> bool {
    app.ui.transform.is_none()
        && !app.session.has_jobs()
        && app.session.active().is_some_and(|active| {
            active.revision != u64::MAX && active.floating.is_none() && active.doc.xcf_state.as_ref().is_none_or(|state| state.floating.is_none())
        })
}
fn draft(target: &Target, palette: &Palette, slot: u8) -> Option<XcfIndexedPaletteDraft> {
    let rgb = *palette.colors.get(usize::from(slot)).filter(|_| usize::from(slot) < palette.count)?;
    Some(XcfIndexedPaletteDraft { document: target.document.0, layer: target.layer.0, revision: target.revision, slot, rgb })
}
fn request(app: &PhotocraftApp, target: &Target, value: &XcfIndexedPaletteDraft) -> Result<Value, String> {
    if !same_target(app, target) || value.document != target.document.0 || value.layer != target.layer.0 || value.revision != target.revision {
        return Err(STALE.into());
    }
    if !editable(app) {
        return Err("xcf.indexed.palette.ui.context".into());
    }
    let (_, palette) = capture(app, target.layer).ok_or(STALE)?;
    if usize::from(value.slot) >= palette.count {
        return Err(STALE.into());
    }
    Ok(
        json!({"documentId":value.document.to_string(),"expectedRevision":value.revision.to_string(),"layerId":value.layer.to_string(),"slot":value.slot,"rgb":value.rgb}),
    )
}

/// Bounded palette metadata only; engine checks all owners/locks/planes on Apply.
pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    let Some((fresh, palette)) = capture(app, layer) else { return };
    // A restored serial draft cannot recreate a runtime snapshot lease. Reset it
    // when no live matching lease exists, including edit/Undo/refocus changes.
    let lease_id = egui::Id::new(("xcf_indexed_palette_lease", fresh.document.0, layer.0));
    let previous = ui.ctx().data_mut(|data| data.get_temp::<Target>(lease_id));
    let retained = previous.filter(|target| same_target(app, target));
    let has_lease = retained.is_some();
    let target = retained.unwrap_or(fresh);
    ui.ctx().data_mut(|data| data.insert_temp(lease_id, target.clone()));
    let restored = app.ui.xcf_indexed_palette_draft.take();
    let mut value = match restored {
        Some(value)
            if has_lease
                && value.document == target.document.0
                && value.layer == layer.0
                && value.revision == target.revision
                && usize::from(value.slot) < palette.count =>
        {
            value
        }
        _ => match draft(&target, &palette, 0) {
            Some(value) => value,
            None => return,
        },
    };
    let enabled = editable(app);
    let mut apply = false;
    let mut reset = false;
    ui.push_id(("xcf_indexed_palette", target.document.0, layer.0), |ui| {
        ui.label(egui::RichText::new(tl!("Color Table…")).color(Tokens::get(ui.ctx()).text_dim));
        let old_slot = value.slot;
        let maximum = u8::try_from(palette.count.saturating_sub(1)).unwrap_or(255);
        ui.horizontal(|ui| {
            ui.label("#");
            ui.add(egui::DragValue::new(&mut value.slot).range(0..=maximum));
        });
        if old_slot != value.slot
            && let Some(selected) = draft(&target, &palette, value.slot)
        {
            value = selected;
        }
        ui.horizontal(|ui| {
            ui.label(tl!("Transparency"));
            ui.label(palette.transparent.map_or_else(|| "—".into(), |slot| slot.to_string()));
            ui.label(tl!("Alpha"));
            ui.label(if palette.transparent == Some(value.slot) { "0" } else { "255" });
        });
        // RGB-only controls: the independent alpha plane is never edited here.
        for (component, label) in value.rgb.iter_mut().zip([tl!("Red"), tl!("Green"), tl!("Blue")]) {
            ui.horizontal(|ui| {
                let label = ui.label(label);
                ui.add_enabled(enabled, egui::DragValue::new(component).range(0..=255)).labelled_by(label.id);
            });
        }
        let changed = palette.colors.get(usize::from(value.slot)).is_some_and(|current| *current != value.rgb);
        ui.horizontal(|ui| {
            apply = ui.add_enabled(enabled && changed, egui::Button::new(tl!("Apply"))).clicked();
            reset = ui.add_enabled(changed, egui::Button::new(tl!("Reset"))).clicked();
        });
    });
    if reset && let Some(original) = draft(&target, &palette, value.slot) {
        value = original;
    }
    if apply {
        match request(app, &target, &value).and_then(|params| app.run(COMMAND, params)) {
            Ok(_) => {
                app.ui.xcf_indexed_palette_draft = None;
                ui.ctx().data_mut(|data| data.remove::<Target>(lease_id));
                return;
            }
            Err(error) => {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
    }
    app.ui.xcf_indexed_palette_draft = Some(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reset_uses_exact_current_slot_bytes_without_alpha_fields() {
        let target = Target { document: DocId(7), revision: 8, layer: LayerId(9), snapshot: WeakDocumentSnapshot::default() };
        let mut palette = Palette { colors: [[0; 3]; 256], count: 2, transparent: Some(1) };
        palette.colors[1] = [0, 127, 255];
        let value = draft(&target, &palette, 1).unwrap();
        assert_eq!(value.rgb, [0, 127, 255]);
        assert_eq!(value.slot, 1);
        assert!(draft(&target, &palette, 2).is_none());
    }
}
