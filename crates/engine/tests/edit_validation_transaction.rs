//! Proposed public-Session baseline/candidate transaction observations; TEMP only.
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, LayerId, SampleType, Size};
use photocraft_engine::{EngineError, Session};
use photocraft_geom::TileCoord;
use photocraft_raster::{Interrupt, Surface, Tile};
use std::sync::{
    Arc, Mutex, PoisonError, Weak,
    atomic::{AtomicBool, Ordering},
};

struct DropProbe {
    tile: Weak<Tile>,
    events: Arc<Mutex<Vec<bool>>>,
}
impl Drop for DropProbe {
    fn drop(&mut self) {
        self.events.lock().unwrap_or_else(PoisonError::into_inner).push(self.tile.upgrade().is_some());
    }
}
fn fixture() -> (Session, LayerId) {
    let mut document = Document::new("Own edit transaction", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let layer = Layer::raster("Own raster", document.pixel_format());
    let id = layer.id;
    document.layers.push(layer);
    let mut session = Session::new();
    session.add_document(document, None).unwrap();
    session
        .edit("Own redo seed", |doc, _| {
            doc.name.push('!');
            Ok(())
        })
        .unwrap();
    assert!(session.undo());
    assert!(session.active().unwrap().history.can_redo());
    (session, id)
}
fn candidate_surface(forged: bool) -> (Surface, Weak<Tile>) {
    let mut surface = Surface::new(photocraft_color::PixelFormat::RGBA8);
    surface.try_write_pixel_f64(0, 0, &[0.25, 0.5, 0.75, 1.0]).unwrap();
    let tile = surface.tiles().next().unwrap().1.clone();
    let weak = Arc::downgrade(&tile);
    if forged {
        let mut invalid = Surface::new(surface.format());
        invalid.put_tiles([(TileCoord::new(i32::MAX, i32::MIN), tile)]);
        (invalid, weak)
    } else {
        (surface, weak)
    }
}
#[derive(Debug, PartialEq)]
struct StateSnapshot {
    doc: photocraft_engine::DocumentSnapshot,
    active_layer: Option<LayerId>,
    selected_layers: Vec<LayerId>,
    layer_anchor: Option<LayerId>,
    revision: u64,
    saved_revision: u64,
    past: Vec<String>,
    redo: Vec<String>,
    native: Vec<u8>,
}
fn snapshot(session: &Session) -> StateSnapshot {
    let state = session.active().unwrap();
    StateSnapshot {
        doc: state.doc.clone(),
        active_layer: state.active_layer,
        selected_layers: state.selected_layers.clone(),
        layer_anchor: state.layer_anchor,
        revision: state.revision,
        saved_revision: state.saved_revision,
        past: state.history.entry_labels().map(str::to_owned).collect(),
        redo: state.history.redo_labels().map(str::to_owned).collect(),
        native: photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap(),
    }
}
fn assert_redo_preserved(session: &mut Session) {
    assert!(session.redo());
    assert!(session.active().unwrap().doc.name.ends_with('!'));
    assert!(session.undo());
}
#[test]
fn post_callback_refusal_drops_result_before_staged_tile_owner() {
    let (mut session, id) = fixture();
    let before = snapshot(&session);
    let events = Arc::new(Mutex::new(Vec::new()));
    let (surface, weak) = candidate_surface(true);
    let result = session.edit("Own rejected candidate", |doc, _| {
        doc.layer_mut(id).unwrap().content = LayerContent::Raster(surface);
        Ok(DropProbe { tile: weak.clone(), events: events.clone() })
    });
    assert!(result.is_err());
    assert_eq!(*events.lock().unwrap_or_else(PoisonError::into_inner), vec![true]);
    assert!(weak.upgrade().is_none(), "staged owner must be released after rejection");
    assert_eq!(snapshot(&session), before);
    assert_redo_preserved(&mut session);
    assert_eq!(snapshot(&session).doc, before.doc);
}
#[test]
fn successful_edit_returns_result_until_caller_drop_and_keeps_native_history() {
    let (mut session, id) = fixture();
    let before = snapshot(&session);
    let events = Arc::new(Mutex::new(Vec::new()));
    let (surface, weak) = candidate_surface(false);
    let token = session
        .edit("Own valid candidate", |doc, _| {
            doc.layer_mut(id).unwrap().content = LayerContent::Raster(surface);
            Ok(DropProbe { tile: weak.clone(), events: events.clone() })
        })
        .unwrap();
    assert!(events.lock().unwrap_or_else(PoisonError::into_inner).is_empty());
    let committed = snapshot(&session);
    assert_eq!(photocraft_format::load_from_bytes(&committed.native).unwrap(), *session.active().unwrap().doc);
    drop(token);
    assert_eq!(*events.lock().unwrap_or_else(PoisonError::into_inner), vec![true]);
    assert!(session.undo());
    assert_eq!(session.active().unwrap().doc, before.doc);
    assert!(session.redo());
    assert_eq!(session.active().unwrap().doc, committed.doc);
}
#[test]
fn post_callback_cancel_drops_result_before_staged_owner_without_commit() {
    let (mut session, id) = fixture();
    let before = snapshot(&session);
    let events = Arc::new(Mutex::new(Vec::new()));
    let cancelled = AtomicBool::new(false);
    let cancel = || cancelled.load(Ordering::SeqCst);
    let (surface, weak) = candidate_surface(false);
    let result = session.edit_with_interrupt("Own cancelled candidate", &Interrupt::cancel_only(&cancel), |doc, _| {
        doc.layer_mut(id).unwrap().content = LayerContent::Raster(surface);
        cancelled.store(true, Ordering::SeqCst);
        Ok(DropProbe { tile: weak.clone(), events: events.clone() })
    });
    assert!(matches!(result, Err(EngineError::Cancelled)));
    assert_eq!(*events.lock().unwrap_or_else(PoisonError::into_inner), vec![true]);
    assert!(weak.upgrade().is_none());
    assert_eq!(snapshot(&session), before);
    assert_redo_preserved(&mut session);
}
#[test]
fn callback_error_is_unchanged_and_drops_once_without_post_validation() {
    let (mut session, id) = fixture();
    let before = snapshot(&session);
    let events = Arc::new(Mutex::new(Vec::new()));
    let (surface, weak) = candidate_surface(false);
    let result: Result<DropProbe, EngineError> = session.edit("Own callback error", |doc, _| {
        doc.layer_mut(id).unwrap().content = LayerContent::Raster(surface);
        let _token = DropProbe { tile: weak.clone(), events: events.clone() };
        Err(EngineError::Other("Own exact callback failure".into()))
    });
    assert!(matches!(result, Err(EngineError::Other(ref message)) if message == "Own exact callback failure"));
    assert_eq!(*events.lock().unwrap_or_else(PoisonError::into_inner), vec![true]);
    assert!(weak.upgrade().is_none());
    assert_eq!(snapshot(&session), before);
    assert_redo_preserved(&mut session);
}
