//! Current layer-mask crop/placement drafts; native samples remain engine-owned.
use crate::{PhotocraftApp, state::XcfMaskGeometryDraft, theme::Tokens};
use photocraft_doc::{
    DocId, LayerId,
    item_identity::{ItemRef, MaskId},
};
use photocraft_engine::WeakDocumentSnapshot;
use photocraft_geom::Rect;
use serde_json::{Value, json};
const STALE: &str = "xcf.mask.geometry.ui.stale-target";
#[derive(Clone)]
struct Target {
    document: DocId,
    revision: u64,
    layer: LayerId,
    mask: MaskId,
    bounds: Rect,
    snapshot: WeakDocumentSnapshot,
}
fn capture(app: &PhotocraftApp, layer: LayerId) -> Option<Target> {
    let active = app.session.active()?;
    if active.doc.accounted().is_some() || active.doc.id.0 == 0 || layer.0 == 0 {
        return None;
    }
    let state = active.doc.xcf_state.as_ref()?;
    if state.items.len() > 10_000 || state.filter_masks.len() > 10_000 {
        return None;
    }
    // Bound actual ancestry before native lookup; no native pixel/plane scans.
    photocraft_engine::xcf_item_locks::effective_layer_locks(&active.doc, layer).ok()?;
    let native = active.doc.layer(layer)?.mask.as_ref()?;
    let format = native.surface.format();
    if format.mode != photocraft_doc::ColorMode::Grayscale
        || format.alpha
        || format.sample != active.doc.depth
        || native.linked
        || native.density != 1.0
        || native.feather != 0.0
    {
        return None;
    }
    let mut filter_ids = std::collections::HashSet::new();
    filter_ids.try_reserve(state.filter_masks.len()).ok()?;
    for mask in &state.filter_masks {
        filter_ids.insert(mask.id);
    }
    let mut found = None;
    for item in &state.items {
        let Some(metadata) = &item.mask else {
            continue;
        };
        if metadata.owner != ItemRef::Layer(layer) {
            continue;
        }
        let ItemRef::Mask(mask) = item.item else {
            return None;
        };
        if filter_ids.contains(&mask) {
            continue;
        }
        let outside = f64::from_bits(metadata.outside_f64_bits);
        if found.is_some()
            || mask.0 == 0
            || metadata.bounds.is_empty()
            || metadata.offset != [metadata.bounds.x0, metadata.bounds.y0]
            || item.raster_bounds != Some(metadata.bounds)
            || metadata.enabled != native.enabled
            || metadata.linked
            || metadata.shown
            || metadata.editing
            || !outside.is_finite()
            || !(0.0..=1.0).contains(&outside)
        {
            return None;
        }
        found = Some((mask, metadata.bounds));
    }
    let (mask, bounds) = found?;
    Some(Target { document: active.doc.id, revision: active.revision, layer, mask, bounds, snapshot: active.doc.downgrade() })
}
fn same_target(app: &PhotocraftApp, target: &Target) -> bool {
    app.session.active().is_some_and(|active| active.doc.id == target.document && active.revision == target.revision && target.snapshot.is_current(&active.doc))
}
fn permitted(app: &PhotocraftApp, target: &Target) -> bool {
    let Some(active) = app.session.active() else {
        return false;
    };
    if app.ui.transform.is_some()
        || app.session.has_jobs()
        || active.revision == u64::MAX
        || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
    {
        return false;
    }
    let Ok(owner) = photocraft_engine::xcf_item_locks::effective_layer_locks(&active.doc, target.layer) else {
        return false;
    };
    let Ok(mask) = photocraft_engine::xcf_item_locks::effective_item_locks(&active.doc, ItemRef::Mask(target.mask)) else {
        return false;
    };
    [owner, mask].into_iter().all(|locks| {
        locks.require(photocraft_engine::xcf_item_locks::EditKind::Content).is_ok()
            && locks.require(photocraft_engine::xcf_item_locks::EditKind::Position).is_ok()
    })
}
fn draft(target: &Target) -> XcfMaskGeometryDraft {
    XcfMaskGeometryDraft {
        document: target.document.0,
        revision: target.revision,
        layer: target.layer.0,
        mask: target.mask.0,
        crop: [target.bounds.x0, target.bounds.y0, target.bounds.x1, target.bounds.y1],
        offset: [target.bounds.x0, target.bounds.y0],
    }
}
fn draft_matches(target: &Target, value: &XcfMaskGeometryDraft) -> bool {
    value.document == target.document.0 && value.revision == target.revision && value.layer == target.layer.0 && value.mask == target.mask.0
}
fn geometry_valid(target: &Target, value: &XcfMaskGeometryDraft) -> bool {
    let [x0, y0, x1, y1] = value.crop;
    let crop = Rect::new(x0, y0, x1, y1);
    !crop.is_empty()
        && crop.intersect(&target.bounds) == crop
        && x1.checked_sub(x0).and_then(|w| value.offset[0].checked_add(w)).is_some()
        && y1.checked_sub(y0).and_then(|h| value.offset[1].checked_add(h)).is_some()
}
fn request(app: &PhotocraftApp, target: &Target, value: &XcfMaskGeometryDraft) -> Result<Value, String> {
    if !same_target(app, target) || !draft_matches(target, value) {
        return Err(STALE.into());
    }
    let fresh = capture(app, target.layer).ok_or(STALE)?;
    if fresh.mask != target.mask || fresh.bounds != target.bounds {
        return Err(STALE.into());
    }
    if !permitted(app, target) {
        return Err("xcf.mask.geometry.ui.context-or-lock".into());
    }
    if !geometry_valid(target, value) {
        return Err("xcf.mask.geometry.ui.bounds".into());
    }
    Ok(
        json!({"documentId":value.document.to_string(),"expectedRevision":value.revision.to_string(),"maskId":value.mask.to_string(),"crop":value.crop,"offset":value.offset}),
    )
}
/// Bounded declarations only. Engine validates raw/native samples, default,
/// complete renderer context and resource admission on immediate Apply.
pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    let Some(fresh) = capture(app, layer) else {
        return;
    };
    let lease = egui::Id::new(("xcf-mask-geometry-lease", fresh.document.0, layer.0, fresh.mask.0));
    let retained = ui.ctx().data_mut(|data| data.get_temp::<Target>(lease)).filter(|t| same_target(app, t));
    let has_lease = retained.is_some();
    let target = retained.unwrap_or(fresh);
    ui.ctx().data_mut(|data| data.insert_temp(lease, target.clone()));
    let mut value = match app.ui.xcf_mask_geometry_draft.take() {
        Some(v) if has_lease && draft_matches(&target, &v) => v,
        _ => draft(&target),
    };
    let current = draft(&target);
    let editable = permitted(app, &target);
    let mut apply = false;
    let mut reset = false;
    ui.push_id(("xcf-mask-geometry", target.document.0, target.mask.0), |ui| {
        ui.label(egui::RichText::new(tl!("Layer Mask")).color(Tokens::get(ui.ctx()).text_dim));
        ui.label(format!("[{}, {}, {}, {}]", target.bounds.x0, target.bounds.y0, target.bounds.x1, target.bounds.y1));
        ui.label(tl!("Crop"));
        for (edge, label) in value.crop.iter_mut().zip([tl!("Left"), tl!("Top"), tl!("Right"), tl!("Bottom")]) {
            ui.horizontal(|ui| {
                let label = ui.label(label);
                ui.add_enabled(editable, egui::DragValue::new(edge)).labelled_by(label.id);
            });
        }
        ui.label(tl!("Position"));
        for (axis, label) in value.offset.iter_mut().zip(["X", "Y"]) {
            ui.horizontal(|ui| {
                let label = ui.label(label);
                ui.add_enabled(editable, egui::DragValue::new(axis)).labelled_by(label.id);
            });
        }
        let changed = value.crop != current.crop || value.offset != current.offset;
        ui.horizontal(|ui| {
            apply = ui.add_enabled(editable && changed && geometry_valid(&target, &value), egui::Button::new(tl!("Apply"))).clicked();
            reset = ui.add_enabled(changed, egui::Button::new(tl!("Reset"))).clicked();
        });
    });
    if reset {
        value = current;
    }
    if apply {
        match request(app, &target, &value).and_then(|params| app.run("xcf.mask.geometry.set", params)) {
            Ok(_) => {
                app.ui.xcf_mask_geometry_draft = None;
                ui.ctx().data_mut(|data| data.remove::<Target>(lease));
                return;
            }
            Err(error) => {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
    }
    app.ui.xcf_mask_geometry_draft = Some(value);
}
#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> Target {
        Target {
            document: DocId(7),
            revision: 8,
            layer: LayerId(9),
            mask: MaskId(10),
            bounds: Rect::new(-3, 1, 2, 4),
            snapshot: WeakDocumentSnapshot::default(),
        }
    }
    #[test]
    fn exact_reset_keeps_independent_negative_bounds_and_identity() {
        let t = target();
        let mut d = draft(&t);
        d.crop = [-2, 1, 1, 3];
        d.offset = [17, -9];
        assert!(geometry_valid(&t, &d));
        let reset = draft(&t);
        assert_eq!(reset.crop, [-3, 1, 2, 4]);
        assert_eq!(reset.offset, [-3, 1]);
        assert!(draft_matches(&t, &reset));
    }
    #[test]
    fn bad_crop_overflow_and_restored_identity_are_not_authority() {
        let t = target();
        let mut d = draft(&t);
        d.crop = [-4, 1, 1, 3];
        assert!(!geometry_valid(&t, &d));
        d = draft(&t);
        d.offset = [i32::MAX, 0];
        assert!(!geometry_valid(&t, &d));
        d = draft(&t);
        d.mask = 11;
        assert!(!draft_matches(&t, &d));
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        assert_eq!(request(&app, &t, &draft(&t)).unwrap_err(), STALE);
    }
}
