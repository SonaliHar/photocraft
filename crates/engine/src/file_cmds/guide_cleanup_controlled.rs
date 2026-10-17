//! Clear current canvas guides, retaining only strict interior artboard guides.
//! Matches Document::artboards: only top-level native artboards participate.
use crate::{EngineError, Result, Session};
use photocraft_doc::{Document, Guides, Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};
fn error(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("Clear Canvas Guides: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled(control: &Interrupt<'_>, message: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { error(message) }
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || (text.len() > 1 && text.starts_with('0')) || !text.bytes().all(|value| value.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    if !params.is_null()
        && !params.as_object().is_some_and(|object| object.len() <= 2 && object.keys().all(|key| matches!(key.as_str(), "documentId" | "expectedRevision")))
    {
        return Err(error("expected optional captured document/revision pair"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.guides.horizontal.len() > 100_000 || state.doc.guides.vertical.len() > 100_000 || state.doc.layers.len() > 100_000 {
        return Err(error("guide/layer count limit"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if decimal(id) == Some(state.doc.id.0) && state.doc.id.0 != 0 && decimal(revision) == Some(state.revision) => {}
        _ => return Err(error("captured document/revision changed or incomplete")),
    }
    if state.revision == u64::MAX
        || state.floating.is_some()
        || state.coalesce.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(error("idle uncoalesced guide context required"));
    }
    Ok(())
}
fn boards(document: &Document, control: &Interrupt<'_>) -> Result<Vec<Rect>> {
    let mut pending: Vec<(&Layer, usize)> = Vec::new();
    pending.try_reserve_exact(document.layers.len()).map_err(|_| error("hierarchy allocation denied"))?;
    for layer in &document.layers {
        poll(control)?;
        pending.push((layer, 0));
    }
    let mut boards = Vec::new();
    let mut count = 0usize;
    while let Some((layer, depth)) = pending.pop() {
        poll(control)?;
        count = count.checked_add(1).filter(|count| *count <= 100_000).ok_or_else(|| error("hierarchy work limit"))?;
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(error("hierarchy depth limit"));
        }
        if let LayerContent::Group(group) = &layer.content {
            if depth == 0
                && let Some(board) = &group.artboard
            {
                let rect = board.rect;
                if rect.x0 > rect.x1 || rect.y0 > rect.y1 {
                    return Err(error("invalid native artboard bounds"));
                }
                if boards.len() >= 100_000 {
                    return Err(error("artboard count limit"));
                }
                boards.try_reserve(1).map_err(|_| error("artboard allocation denied"))?;
                boards.push(rect);
            }
            if pending.len().checked_add(group.children.len()).is_none_or(|count| count > 100_000) {
                return Err(error("hierarchy pending limit"));
            }
            pending.try_reserve(group.children.len()).map_err(|_| error("hierarchy allocation denied"))?;
            for child in &group.children {
                poll(control)?;
                pending.push((child, depth + 1));
            }
        }
    }
    poll(control)?;
    Ok(boards)
}
fn retained(values: &[f64], boards: &[Rect], vertical: bool, control: &Interrupt<'_>) -> Result<(Vec<f64>, usize)> {
    let mut output = Vec::new();
    output.try_reserve_exact(values.len()).map_err(|_| error("guide copy allocation denied"))?;
    if output.capacity() > values.len().saturating_add(1024) {
        return Err(error("guide capacity exceeded"));
    }
    let mut cleared = 0usize;
    for value in values {
        poll(control)?;
        if !value.is_finite() {
            return Err(error("nonfinite retained guide coordinate"));
        }
        let mut inside = false;
        for (index, rect) in boards.iter().enumerate() {
            if index.is_multiple_of(1024) {
                poll(control)?;
            }
            let (first, last) = if vertical { (rect.x0, rect.x1) } else { (rect.y0, rect.y1) };
            if (*value) > f64::from(first) && (*value) < f64::from(last) {
                inside = true;
                break;
            }
        }
        if inside {
            output.push(*value);
        } else {
            cleared = cleared.checked_add(1).ok_or_else(|| error("clear count overflow"))?;
        }
    }
    poll(control)?;
    Ok((output, cleared))
}
fn prepare(document: &crate::DocumentSnapshot, control: &Interrupt<'_>) -> Result<(Guides, usize)> {
    poll(control)?;
    document.validate_resident()?;
    if document.guides.vertical.len() > 100_000 || document.guides.horizontal.len() > 100_000 {
        return Err(error("guide work limit"));
    }
    let boards = boards(document, control)?;
    let count = document.guides.vertical.len().checked_add(document.guides.horizontal.len()).ok_or_else(|| error("guide count overflow"))? as u64;
    let work = count.checked_mul(boards.len() as u64 + 1).and_then(|work| work.checked_add(100_000)).ok_or_else(|| error("membership work overflow"))?;
    if work > 8_000_000_000 {
        return Err(error("artboard/guide membership work limit"));
    }
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(document, limits, control).map_err(|e| controlled(control, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(document, limits, control).map_err(|e| controlled(control, e))? as u64;
    let raster = document.resident_raster_footprint_with(control)?;
    let peak = model
        .checked_mul(4)
        .and_then(|value| value.checked_add(payload))
        .and_then(|value| value.checked_add(raster.retained_bytes))
        .and_then(|value| value.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|value| value.checked_add(8_000_000))
        .ok_or_else(|| error("metadata residency overflow"))?;
    // Known model/payload/maps and bounded hierarchy/guide vectors, not RSS.
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(error("guide/source metadata residency limit"));
    }
    let (vertical, v) = retained(&document.guides.vertical, &boards, true, control)?;
    let (horizontal, h) = retained(&document.guides.horizontal, &boards, false, control)?;
    let cleared = v.checked_add(h).ok_or_else(|| error("clear count overflow"))?;
    poll(control)?;
    let mut guides = Guides { horizontal, vertical, colors: copied_colors(&document.guides.colors, control)? };
    guides.prune_colors();
    Ok((guides, cleared))
}
pub(crate) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let work = baseline.clone();
    crate::jobs::run(
        session,
        "Clear Canvas Guides",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Clear Canvas Guides", |control| {
                let (guides, cleared) = prepare(&work, control)?;
                let response = json!({"cleared":cleared});
                Ok((guides, cleared, response, ctx.clone()))
            })
        },
        move |session, (guides, cleared, response, ctx)| {
            let state = session.active().ok_or(EngineError::NoDocument)?;
            if state.doc.id != baseline.id || state.revision != revision || !state.doc.shares_state(&baseline) {
                return Err(error("guide target changed during preparation"));
            }
            ctx.stage(0.95, 1., "Clear Canvas Guides", |control| {
                poll(control)?;
                if cleared == 0 {
                    return Ok(response);
                }
                session.edit_with_interrupt("Clear Canvas Guides", control, move |document, _| {
                    document.guides = guides;
                    Ok(())
                })?;
                Ok(response)
            })
        },
    )
}

fn copied_colors(values: &[photocraft_doc::GuideColor], control: &Interrupt<'_>) -> Result<Vec<photocraft_doc::GuideColor>> {
    if values.len() > 100_000 {
        return Err(error("guide color metadata limit"));
    }
    let mut output = Vec::new();
    output.try_reserve_exact(values.len()).map_err(|_| error("guide color allocation denied"))?;
    for value in values {
        poll(control)?;
        if !value.position.is_finite() {
            return Err(error("nonfinite guide color position"));
        }
        output.push(*value);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{Color, ColorMode, SampleType};
    use photocraft_doc::{Artboard, Group, PhysicalResolution, Size};
    fn board(name: &str, rect: Rect) -> Layer {
        Layer::new(name, LayerContent::Group(Group { children: Vec::new(), expanded: true, artboard: Some(Artboard::new(rect)) }))
    }
    fn fixture(with_boards: bool) -> Session {
        let mut document = Document::with_background("cleanup guides", Size::new(2, 2), ColorMode::Rgb, SampleType::F64, Color::WHITE);
        document.guides.vertical = vec![10., 20., 40., 150.];
        document.guides.horizontal = vec![20., 30., 60.];
        document.physical_resolution = Some(PhysicalResolution { y_dpi: 288., ..PhysicalResolution::square(144.) });
        document.resolution_dpi = 72.;
        if with_boards {
            document.layers.push(board("Top board", Rect::new(10, 20, 40, 60)));
            document.layers.push(Layer::new(
                "Ordinary group",
                LayerContent::Group(Group { children: vec![board("Nested board", Rect::new(100, 100, 200, 200))], expanded: true, artboard: None }),
            ));
        }
        let mut session = Session::new();
        session.add_document(document, None).unwrap();
        session
    }
    #[test]
    fn strict_edge_and_top_level_membership_match_existing_artboards_oracle() {
        let session = fixture(true);
        let snapshot = &session.active().unwrap().doc;
        let rectangles: Vec<Rect> = snapshot.artboards().iter().map(|board| board.2.rect).collect();
        let expected = Guides {
            vertical: snapshot
                .guides
                .vertical
                .iter()
                .copied()
                .filter(|value| rectangles.iter().any(|rect| (*value) > f64::from(rect.x0) && (*value) < f64::from(rect.x1)))
                .collect(),
            horizontal: snapshot
                .guides
                .horizontal
                .iter()
                .copied()
                .filter(|value| rectangles.iter().any(|rect| (*value) > f64::from(rect.y0) && (*value) < f64::from(rect.y1)))
                .collect(),
            colors: Vec::new(),
        };
        let (actual, count) = prepare(snapshot, &Interrupt::NONE).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual.vertical, vec![20.]);
        assert_eq!(actual.horizontal, vec![30.]);
        assert_eq!(count, 5);
    }
    #[test]
    fn no_artboards_clear_all_with_one_undo_and_raw_document_fields_preserved() {
        let mut session = fixture(false);
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.past_len();
        let native = before.layers[0].surface().unwrap().try_read_interleaved_with(before.bounds(), &Interrupt::NONE).unwrap();
        assert_eq!(run(&mut session, &Value::Null).unwrap(), json!({"cleared":7}));
        let state = session.active().unwrap();
        assert_eq!(state.history.past_len(), history + 1);
        assert!(state.doc.guides.vertical.is_empty());
        assert!(state.doc.guides.horizontal.is_empty());
        assert_eq!(state.doc.depth, before.depth);
        assert_eq!(state.doc.physical_resolution, before.physical_resolution);
        assert_eq!(state.doc.resolution_dpi, before.resolution_dpi);
        assert_eq!(state.doc.icc_profile, before.icc_profile);
        assert_eq!(state.doc.layers[0].surface().unwrap().try_read_interleaved_with(state.doc.bounds(), &Interrupt::NONE).unwrap(), native);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.guides, before.guides);
        session.execute("edit.redo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.guides.vertical.is_empty());
    }
    #[test]
    fn all_retained_noop_keeps_real_redo_and_revision() {
        let mut session = fixture(true);
        run(&mut session, &Value::Null).unwrap();
        session
            .edit("redo fixture", |document, _| {
                document.name = "changed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", json!({})).unwrap();
        let state = session.active().unwrap();
        let before = state.doc.clone();
        let revision = state.revision;
        let redo = state.history.redo_label().unwrap().to_string();
        assert_eq!(run(&mut session, &Value::Null).unwrap(), json!({"cleared":0}));
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().history.redo_label(), Some(redo.as_str()));
    }
    #[test]
    fn stale_and_cancelled_cleanup_preserve_source_guides() {
        let session = fixture(true);
        let state = session.active().unwrap();
        let before = state.doc.clone();
        for params in [
            json!({"documentId":before.id.0.to_string()}),
            json!({"documentId":before.id.0.to_string(),"expectedRevision":state.revision.saturating_add(1).to_string()}),
            json!({"unexpected":true}),
        ] {
            assert!(dispatch_preflight(&session, &params, false).is_err());
        }
        let cancel = || true;
        assert!(matches!(prepare(&before, &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
        assert!(state.doc.shares_state(&before));
        assert!(retained(&[f64::NAN], &[Rect::new(0, 0, 10, 10)], true, &Interrupt::NONE).is_err());
    }
}
