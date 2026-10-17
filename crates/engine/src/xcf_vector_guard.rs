//! Canonical saved-path edits obey the original vector owner's effective locks.
use crate::{EngineError, Result};
use photocraft_doc::{Document, xcf_items::EditableSource};

/// Bound-path edits validate actual owner permissions before generic floating
/// normalization or parameter cloning. Ordinary work/shape paths keep their route.
pub(crate) fn pre_dispatch(session: &crate::Session, command: &str, params: &serde_json::Value) -> Result<()> {
    if !matches!(
        command,
        "path.set" | "path.delete" | "path.rename" | "path.transform" | "path.moveAnchors" | "path.moveHandle" | "path.bendSegment" | "path.convertPoint"
    ) {
        return Ok(());
    }
    let Some(active) = session.active() else {
        return Ok(());
    };
    let Some(imported) = active.doc.xcf_state.as_ref() else {
        return Ok(());
    };
    if imported.source_items.len() > 10_000 || active.doc.paths.len() > 10_000 {
        return Err(EngineError::Other("imported vector binding work limit".into()));
    }
    let id = match crate::vector_cmds::saved_path_id(params)? {
        Some(id) => Some(id),
        None => {
            let name = params.get("name").and_then(serde_json::Value::as_str).unwrap_or("work");
            if name.len() > 1 << 20 {
                return Err(EngineError::Other("path selector length limit".into()));
            }
            if matches!(name, "work" | "layer") {
                None
            } else {
                let mut matches = active.doc.paths.iter().filter(|path| path.name == name);
                let selected = matches.next();
                if matches.next().is_some() {
                    return Err(EngineError::Other("ambiguous saved path name; use pathId".into()));
                }
                selected.map(|path| path.id)
            }
        }
    };
    let Some(path) = id else {
        return Ok(());
    };
    for source in &imported.source_items {
        if !matches!(source.source, EditableSource::Vector { path: bound, source_modified: false, .. } if bound == path) {
            continue;
        }
        if active.floating.is_some() {
            return Err(EngineError::Other("complete or cancel the pixel-move preview before editing a bound path".into()));
        }
        if command != "path.rename" {
            let locks = crate::xcf_item_locks::effective_item_locks(&active.doc, source.owner)?;
            locks.require(crate::xcf_item_locks::EditKind::Content)?;
            locks.require(crate::xcf_item_locks::EditKind::Position)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_path_edits_with_interrupt(before: &Document, edited: &Document, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    let Some(state) = &before.xcf_state else { return Ok(()) };
    if state.source_items.len() > 10_000 || before.paths.len() > 10_000 || edited.paths.len() > 10_000 {
        return Err(EngineError::Other("imported vector binding work limit".into()));
    }
    if !state.source_items.iter().any(|source| matches!(source.source, EditableSource::Vector { source_modified: false, .. })) {
        return Ok(());
    }
    fn census<'a>(
        document: &'a Document,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<std::collections::HashMap<photocraft_doc::item_identity::PathId, &'a photocraft_doc::Path>> {
        let mut paths = std::collections::HashMap::new();
        paths.try_reserve(document.paths.len()).map_err(|_| EngineError::Other("vector binding allocation denied".into()))?;
        for path in &document.paths {
            control.check().map_err(|_| EngineError::Cancelled)?;
            if path.id.0 == 0 || paths.insert(path.id, &path.path).is_some() {
                return Err(EngineError::Other("ambiguous saved-path identity".into()));
            }
        }
        Ok(paths)
    }
    let original_paths = census(before, control)?;
    let current_paths = census(edited, control)?;
    for source in &state.source_items {
        control.check().map_err(|_| EngineError::Cancelled)?;
        let EditableSource::Vector { path, source_modified: false, .. } = source.source else { continue };
        let original = original_paths.get(&path);
        let current = current_paths.get(&path);
        if original != current {
            let locks = crate::xcf_item_locks::effective_item_locks(before, source.owner)?;
            locks.require(crate::xcf_item_locks::EditKind::Content)?;
            locks.require(crate::xcf_item_locks::EditKind::Position)?;
        }
    }
    control.check().map_err(|_| EngineError::Cancelled)?;
    Ok(())
}

pub(crate) fn has_live_sources(doc: &Document) -> bool {
    doc.xcf_state.as_ref().is_some_and(|state| {
        !state.filter_stacks.is_empty()
            || state.source_items.iter().any(|source| {
                matches!(source.source, EditableSource::Vector { source_modified: false, .. } | EditableSource::Text { source_modified: false, .. })
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Layer, NamedPath, Path, SampleType, Size, XcfState, item_identity::ItemRef, xcf_items::SourceItem};
    use photocraft_raster::Interrupt;
    fn documents() -> (Document, Document) {
        let mut document = Document::new("Bound path", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        let child = Layer::raster("Bound owner", document.pixel_format());
        let owner = child.id;
        document.layers.push(Layer::group("Parent", vec![child]));
        let path = NamedPath::new("Current path", Path::default());
        let id = path.id;
        document.paths.push(path);
        let mut imported = XcfState::default();
        imported.reconcile_layers(&document).unwrap();
        imported.source_items.push(SourceItem {
            owner: ItemRef::Layer(owner),
            source: EditableSource::Vector { path: id, source_modified: false, configuration: Vec::new() },
            transform: None,
            cache: None,
        });
        document.xcf_state = Some(imported);
        let mut edited = document.clone();
        edited.paths[0].path.inverted = true;
        (document, edited)
    }
    #[test]
    fn bound_path_changes_honor_actual_native_parent_content_and_position_locks() {
        let (before, edited) = documents();
        validate_path_edits_with_interrupt(&before, &edited, &Interrupt::NONE).unwrap();
        for field in ["pixels", "position", "all"] {
            let mut locked = before.clone();
            match field {
                "pixels" => locked.layers[0].locks.pixels = true,
                "position" => locked.layers[0].locks.position = true,
                _ => locked.layers[0].locks.all = true,
            }
            assert!(validate_path_edits_with_interrupt(&locked, &edited, &Interrupt::NONE).is_err(), "{field}");
            assert!(validate_path_edits_with_interrupt(&locked, &locked, &Interrupt::NONE).is_ok());
        }
    }
    #[test]
    fn ambiguous_path_identity_and_cancellation_refuse_without_changing_inputs() {
        let (before, mut edited) = documents();
        let retained = before.clone();
        assert!(matches!(validate_path_edits_with_interrupt(&before, &edited, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert_eq!(before, retained);
        edited.paths.push(edited.paths[0].clone());
        assert!(validate_path_edits_with_interrupt(&before, &edited, &Interrupt::NONE).is_err());
        assert_eq!(before, retained);
    }

    #[test]
    fn dispatcher_refuses_bound_path_before_stale_floating_preview_can_drop() {
        use serde_json::json;
        use std::sync::Arc;
        let (before, _) = documents();
        let path = before.paths[0].id;
        let owner = before.xcf_state.as_ref().unwrap().source_items[0].owner;
        let ItemRef::Layer(layer) = owner else {
            panic!("fixture owner");
        };
        let mut session = crate::Session::new();
        session.open_document(before, None).unwrap();
        let mut preview = (*session.active().unwrap().doc).clone();
        preview.selection = Some(
            photocraft_raster::Surface::try_from_interleaved(
                photocraft_doc::PixelFormat::GRAY8,
                preview.bounds(),
                &vec![255; preview.size.width as usize * preview.size.height as usize],
            )
            .unwrap(),
        );
        let parts = Arc::new(crate::float_cmds::CutParts::new(&preview, layer, true, [0.0; 4], false).unwrap());
        session.active_mut().unwrap().floating = Some(crate::float_cmds::Floating { layer, offset: (4, 5), revision: u64::MAX, parts: parts.clone() });
        let state = session.active().unwrap().doc.clone();
        assert!(session.execute("path.delete", json!({"pathId":path.0.to_string()})).is_err());
        assert_eq!(session.active().unwrap().doc, state);
        assert!(Arc::ptr_eq(&session.active().unwrap().floating.as_ref().unwrap().parts, &parts));
    }
}
