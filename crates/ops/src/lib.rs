//! Undo/redo history.
//!
//! Because pixel tiles are `Arc`-shared copy-on-write (see `photocraft-raster`), a full
//! [`Document`] clone costs O(layers + tiles) pointer copies, not pixel copies. History therefore
//! stores whole-document snapshots per transaction, which is simple, obviously correct, and the
//! same approach Photoshop's History panel exposes to users (one state per step).
//!
//! Memory is bounded by `max_states` plus an approximate byte budget. Accounting deduplicates shared tiles; archived snapshots do not retain resident pixels.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashSet;
use std::sync::Arc;

use photocraft_doc::{DocId, Document, LayerId};
use serde::{Deserialize, Serialize};

/// The layers a state targeted when it was created: the active ("key") layer and every selected
/// layer. Undo and redo bring them back with the state's document, as the reference app does;
/// selecting layers is not a step of its own, so it doesn't change a state's target.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerTarget {
    pub active: Option<LayerId>,
    pub selected: Vec<LayerId>,
}

/// PhotoCraft document adapter for the shared history engine.
#[derive(Clone, Debug)]
pub struct PhotoCraftHistory;
impl craft_history::Policy for PhotoCraftHistory {
    type Document = Document;
    type Target = LayerTarget;
    type Id = DocId;
    fn has_selection(document: &Document) -> bool {
        document.selection.is_some()
    }
    fn remap_id(document: &mut Arc<Document>, id: DocId) {
        if document.id != id {
            Arc::make_mut(document).id = id;
        }
    }
    fn document_bytes(document: &Document, seen: &mut HashSet<usize>) -> usize {
        document_bytes(document, seen)
    }
    fn target_bytes(target: &LayerTarget) -> usize {
        target.selected.capacity().saturating_mul(std::mem::size_of::<LayerId>())
    }
}
pub use craft_history::{ArchivedDocument, HistoryEntry, StateIdentity};
pub type History = craft_history::History<PhotoCraftHistory>;
pub type HistoryState = craft_history::HistoryState<PhotoCraftHistory>;
pub type HistoryCheckpoint = craft_history::HistoryCheckpoint<PhotoCraftHistory>;

/// Managed tile and shared binary-payload bytes not already in `seen`.
/// Metadata containers, strings, renderer caches and allocator overhead are additional memory.
pub fn document_bytes(doc: &Document, seen: &mut HashSet<usize>) -> usize {
    let mut add = |surface: &photocraft_doc::Surface| {
        surface.tiles().filter(|(_, tile)| seen.insert(Arc::as_ptr(tile) as usize)).fold(0usize, |n, (_, tile)| n.saturating_add(tile.bytes().len()))
    };
    let mut total = 0usize;
    let layers = doc.walk();
    for (_, _, layer) in &layers {
        if let Some(surface) = layer.surface() {
            total = total.saturating_add(add(surface));
        }
        if let Some(mask) = &layer.mask {
            total = total.saturating_add(add(&mask.surface));
        }
        if let Some(cache) = &layer.fill_cache {
            total = total.saturating_add(add(&cache.surface));
        }
        if let photocraft_doc::LayerContent::Smart(smart) = &layer.content
            && let Some(mask) = &smart.filter_mask
        {
            total = total.saturating_add(add(&mask.surface));
        }
        if let Some(video) = &layer.video {
            for frame in video.frames.iter().chain(video.original_frames.iter().flatten()) {
                total = total.saturating_add(add(frame));
            }
        }
    }
    for channel in doc.channels.iter().chain(&doc.quick_mask) {
        total = total.saturating_add(add(&channel.surface));
    }
    if let Some(selection) = &doc.selection {
        total = total.saturating_add(add(selection));
    }
    for pattern in &doc.patterns {
        total = total.saturating_add(add(&pattern.surface));
    }
    // Distinct live Arc allocations cannot have the same address, including across
    // payload types. A caller keeps all traversed documents alive while deduplicating.
    let mut blob = |bytes: &Arc<Vec<u8>>| {
        if seen.insert(Arc::as_ptr(bytes) as usize) { bytes.capacity() } else { 0 }
    };
    for bytes in doc.icc_profile.iter().chain(doc.metadata.exif.iter()) {
        total = total.saturating_add(blob(bytes));
    }
    for (_, _, bytes) in &doc.metadata.psd_resources {
        total = total.saturating_add(blob(bytes));
    }
    for (_, _, bytes) in &doc.metadata.psd_global_blocks {
        total = total.saturating_add(blob(bytes));
    }
    for path in &doc.paths {
        if let Some(bytes) = &path.psd_raw {
            total = total.saturating_add(blob(bytes));
        }
    }
    for (_, _, layer) in &layers {
        for (_, bytes) in &layer.psd_blocks {
            total = total.saturating_add(blob(bytes));
        }
        if let Some(bytes) = &layer.effects.psd_raw {
            total = total.saturating_add(blob(bytes));
        }
        match &layer.content {
            photocraft_doc::LayerContent::Text(text) => {
                if let Some(bytes) = &text.psd_raw {
                    total = total.saturating_add(blob(bytes));
                }
            }
            photocraft_doc::LayerContent::Shape(shape) => {
                if let Some(bytes) = &shape.psd_raw {
                    total = total.saturating_add(blob(bytes));
                }
            }
            photocraft_doc::LayerContent::Smart(smart) => {
                if let Some(bytes) = &smart.psd_raw {
                    total = total.saturating_add(blob(bytes));
                }
                if let photocraft_doc::SmartSource::Embedded { bytes, .. } = &smart.source {
                    total = total.saturating_add(blob(bytes));
                }
            }
            _ => {}
        }
    }
    for (_, _, layer) in &layers {
        if let photocraft_doc::LayerContent::Adjustment(photocraft_doc::Adjustment::ColorLookup { lut: Some(lut), .. }) = &layer.content
            && seen.insert(Arc::as_ptr(lut) as usize)
        {
            total = total.saturating_add(lut.capacity().saturating_mul(std::mem::size_of::<f32>()));
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, Layer, SampleType, Size};

    fn base() -> Document {
        Document::with_background("h", Size::new(64, 64), ColorMode::Rgb, SampleType::U8, Color::WHITE)
    }

    /// Apply an edit the way the engine does: snapshot, mutate a clone, record.
    fn edit(h: &mut History, cur: &mut Arc<Document>, label: &str, f: impl FnOnce(&mut Document)) {
        let before = cur.clone();
        let mut d = (**cur).clone();
        f(&mut d);
        *cur = Arc::new(d);
        h.record(label, before, LayerTarget::default());
    }

    #[test]
    fn undo_redo_cycle() {
        let mut h = History::default();
        let mut cur = Arc::new(base());
        edit(&mut h, &mut cur, "New Layer", |d| {
            d.insert_above(None, Layer::raster("L", d.pixel_format()));
        });
        assert_eq!(cur.layers.len(), 2);
        assert_eq!(h.undo_label(), Some("New Layer"));
        cur = h.undo(cur).unwrap().0;
        assert_eq!(cur.layers.len(), 1);
        assert_eq!(h.redo_label(), Some("New Layer"));
        cur = h.redo(cur).unwrap().0;
        assert_eq!(cur.layers.len(), 2);
        assert!(h.redo(cur.clone()).is_none());
    }

    #[test]
    fn undo_and_redo_return_the_layers_each_state_targeted_when_created() {
        let mut h = History::default();
        let open = Arc::new(base());
        let bg = open.layers[0].id;
        let target = |id: LayerId| LayerTarget { active: Some(id), selected: vec![id] };
        h.set_current_target(target(bg));
        // New Layer targets the new layer; the background is then selected (not a step) and filled.
        let mut d = (*open).clone();
        let new = d.insert_above(None, Layer::raster("L", d.pixel_format()));
        let layered = Arc::new(d);
        h.record("New Layer", open, target(new));
        let mut d = (*layered).clone();
        d.name = "filled".into();
        h.record("Fill", layered, target(bg));
        let (d, layers) = h.undo(Arc::new(d)).unwrap();
        assert_eq!(layers, target(new), "New Layer's state targeted the new layer when created");
        let (d, layers) = h.undo(d).unwrap();
        assert_eq!(layers, target(bg), "the opened document's target");
        let (d, layers) = h.redo(d).unwrap();
        assert_eq!(layers, target(new));
        let (_, layers) = h.redo(d).unwrap();
        assert_eq!(layers, target(bg), "Fill's state targeted the background");
    }

    #[test]
    fn state_accessor_matches_entries() {
        let mut h = History::default();
        let mut cur = Arc::new(base());
        let open = cur.clone();
        edit(&mut h, &mut cur, "A", |d| d.name = "a".into());
        edit(&mut h, &mut cur, "B", |d| d.name = "b".into());
        assert_eq!(h.entries().len(), h.past_len() + 1);
        assert!(Arc::ptr_eq(&h.state(0).unwrap(), &open));
        assert_eq!(h.state(1).unwrap().name, "a");
        assert!(h.state(2).is_none(), "the current document is not held");
        cur = h.undo(cur).unwrap().0;
        h.undo(cur).unwrap();
        assert_eq!(h.redo_labels().collect::<Vec<_>>(), ["A", "B"], "next redo first");
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut h = History::default();
        let mut cur = Arc::new(base());
        edit(&mut h, &mut cur, "A", |d| d.name = "a".into());
        cur = h.undo(cur).unwrap().0;
        edit(&mut h, &mut cur, "B", |d| d.name = "b".into());
        assert!(!h.can_redo());
        assert_eq!(h.entries(), vec!["Open".to_string(), "B".to_string()]);
    }

    #[test]
    fn max_states_bound() {
        let mut h = History::new(3);
        let mut cur = Arc::new(base());
        for i in 0..10 {
            edit(&mut h, &mut cur, &format!("step {i}"), |d| d.name = format!("{i}"));
        }
        let mut n = 0;
        while let Some((d, _)) = h.undo(cur.clone()) {
            cur = d;
            n += 1;
        }
        assert_eq!(n, 3);
        assert_eq!(cur.name, "6");
    }

    #[test]
    fn byte_budget_drops_oldest_states_but_keeps_one() {
        let mut h = History::new(50);
        let mut cur = Arc::new(base());
        let tile = 256 * 256 * 4;
        // Each step repaints the background's tile: one unique tile per state.
        for i in 0..6 {
            edit(&mut h, &mut cur, &format!("paint {i}"), |d| {
                let id = d.layers[0].id;
                d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
            });
        }
        assert_eq!(h.trim(&cur), 0, "unlimited by default");
        assert_eq!(h.past_len(), 6);
        // The current tile plus three history tiles fit.
        h.max_bytes = 4 * tile;
        assert_eq!(h.trim(&cur), 3);
        assert_eq!(h.past_len(), 3);
        assert_eq!(h.unique_bytes(&cur), 3 * tile);
        assert_eq!(h.pixel_bytes(&cur), 4 * tile, "the current tile plus history's");
        assert_eq!(h.entries()[0], "paint 2");
        // A budget smaller than one state still keeps the last step undoable.
        h.max_bytes = 1;
        h.trim(&cur);
        assert_eq!(h.past_len(), 1);
        assert!(h.undo(cur.clone()).is_some());
    }

    #[test]
    fn history_shares_untouched_tiles() {
        let mut h = History::default();
        let mut cur = Arc::new(base());
        assert_eq!(h.unique_bytes(&cur), 0);
        // rename only: no pixel changes, history holds no unique tiles
        edit(&mut h, &mut cur, "Rename", |d| d.name = "x".into());
        assert_eq!(h.unique_bytes(&cur), 0);
        // paint one pixel: exactly one tile becomes unique to history
        edit(&mut h, &mut cur, "Paint", |d| {
            let id = d.layers[0].id;
            d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[0.0, 0.0, 0.0, 1.0]);
        });
        assert_eq!(h.unique_bytes(&cur), 256 * 256 * 4);
    }
    #[derive(Debug)]
    struct Archive(Result<Arc<Document>, String>);
    impl ArchivedDocument<PhotoCraftHistory> for Archive {
        fn load(&self) -> Result<Arc<Document>, String> {
            self.0.clone()
        }
    }

    #[test]
    fn selection_tiles_count_current_and_history_without_double_counting() {
        use photocraft_doc::{PixelFormat, Surface};
        for depth in SampleType::ALL {
            let mut doc = Document::new("selection", Size::new(300, 64), ColorMode::Rgb, depth);
            let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
            selection.write_pixel(1, 1, &[1.0]);
            selection.write_pixel(257, 1, &[1.0]);
            doc.selection = Some(selection);
            let mut cur = Arc::new(doc);
            let tile = match depth {
                SampleType::U8 => 65_536,
                SampleType::U16 => 131_072,
                SampleType::F32 => 262_144,
            };
            let mut h = History::default();
            h.max_bytes = usize::MAX;
            assert_eq!(h.pixel_bytes(&cur), 2 * tile, "current selection counts without any history");
            assert_eq!(h.unique_bytes(&cur), 0);
            edit(&mut h, &mut cur, "Rename", |doc| doc.name = "renamed".into());
            assert_eq!(h.pixel_bytes(&cur), 2 * tile, "the shared selection tiles count once");
            edit(&mut h, &mut cur, "Selection", |doc| {
                doc.selection.as_mut().unwrap().write_pixel(1, 1, &[0.5]);
            });
            assert_eq!(h.pixel_bytes(&cur), 3 * tile, "only the changed selection tile is copied");
            assert_eq!(h.unique_bytes(&cur), tile);
            h.trim(&cur);
            assert_counter(&h, &cur);
            let selected_value = cur.selection.as_ref().unwrap().sample_channel(1, 1, 0);
            edit(&mut h, &mut cur, "Deselect", |doc| doc.selection = None);
            assert_eq!(h.pixel_bytes(&cur), 3 * tile, "deselecting leaves the previous masks in history");
            assert_eq!(h.unique_bytes(&cur), 3 * tile);
            assert_counter(&h, &cur);
            cur = h.undo(cur).unwrap().0;
            assert_eq!(cur.selection.as_ref().unwrap().sample_channel(1, 1, 0), selected_value);
            assert_eq!(h.pixel_bytes(&cur), 3 * tile);
            assert_eq!(h.unique_bytes(&cur), tile);
            assert_counter(&h, &cur);
            // Coalescing can change the current mask without recording another state.
            Arc::make_mut(&mut cur).selection.as_mut().unwrap().write_pixel(257, 1, &[0.25]);
            assert_eq!(h.pixel_bytes(&cur), 4 * tile);
            assert_eq!(h.trim(&cur), 0);
            assert_counter(&h, &cur);
            cur = h.redo(cur).unwrap().0;
            assert!(cur.selection.is_none());
            assert_eq!(h.pixel_bytes(&cur), 4 * tile);
            assert_counter(&h, &cur);
        }
    }

    #[test]
    fn selection_byte_budget_drops_old_masks_and_keeps_the_last_undo() {
        use photocraft_doc::{PixelFormat, Surface};
        for depth in SampleType::ALL {
            let mut doc = Document::new("selection", Size::new(64, 64), ColorMode::Rgb, depth);
            doc.selection = Some(Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false)));
            let mut cur = Arc::new(doc);
            let tile = 256 * 256 * depth.bytes();
            let mut h = History::default();
            for i in 1..=6 {
                edit(&mut h, &mut cur, "Selection", |doc| {
                    doc.selection.as_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0]);
                });
            }
            assert_eq!(h.pixel_bytes(&cur), 6 * tile);
            h.max_bytes = 3 * tile;
            assert_eq!(h.trim(&cur), 4);
            assert_eq!(h.past_len(), 2);
            assert_eq!(h.pixel_bytes(&cur), 3 * tile);
            assert_eq!(h.unique_bytes(&cur), 2 * tile);
            assert_counter(&h, &cur);
            h.max_bytes = 1;
            assert_eq!(h.trim(&cur), 1);
            assert_eq!(h.past_len(), 1, "the most recent undo survives even below one mask's size");
            assert_eq!(h.pixel_bytes(&cur), 2 * tile);
            cur = h.undo(cur).unwrap().0;
            assert_counter(&h, &cur);
        }
    }

    #[test]
    fn document_patterns_count_as_document_pixels() {
        use photocraft_doc::{Pattern, Surface};
        for depth in SampleType::ALL {
            let tile = 4 * 256 * 256 * depth.bytes();
            let mut cur = Arc::new(Document::new("patterns", Size::new(64, 64), ColorMode::Rgb, depth));
            let mut h = History::default();
            h.max_bytes = usize::MAX;
            for name in ["A", "B"] {
                edit(&mut h, &mut cur, "Rename", |doc| doc.name = name.into());
                h.trim(&cur);
            }
            let mut surface = Surface::new(cur.pixel_format());
            surface.write_pixel(1, 1, &[0.0, 0.0, 1.0, 1.0]);
            let pattern = Pattern::new("Pattern", surface, 4, 4);
            edit(&mut h, &mut cur, "Pattern Fill", |doc| doc.patterns.push(pattern));
            assert_eq!(h.pixel_bytes(&cur), tile, "the document's pattern");
            assert_eq!(h.unique_bytes(&cur), 0);
            assert_counter(&h, &cur);
            edit(&mut h, &mut cur, "Rename", |doc| doc.name = "C".into());
            assert_eq!(h.pixel_bytes(&cur), tile, "shared with history, it counts once");
            assert_counter(&h, &cur);
            for _ in 0..2 {
                cur = h.undo(cur).unwrap().0;
                assert_counter(&h, &cur);
            }
            assert!(cur.patterns.is_empty());
            assert_eq!(h.unique_bytes(&cur), tile, "only the redo states hold the pattern");
            assert_eq!(h.pixel_bytes(&cur), tile);
            cur = h.redo(cur).unwrap().0;
            assert_eq!(h.unique_bytes(&cur), 0);
            assert_counter(&h, &cur);
            h.clear();
            assert_eq!(h.pixel_bytes(&cur), tile);
        }
    }

    #[test]
    fn shared_masks_channels_and_repeated_snapshot_references_count_once() {
        use photocraft_doc::{AlphaChannel, LayerMask, PixelFormat, Surface};
        let mut doc = base();
        let mut gray = Surface::new(PixelFormat::GRAY8);
        gray.write_pixel(1, 1, &[1.0]);
        doc.layers[0].mask = Some(LayerMask { surface: gray.clone(), ..LayerMask::reveal_all() });
        doc.channels.push(AlphaChannel::new("alpha", gray.clone()));
        doc.selection = Some(gray.clone());
        doc.quick_mask = Some(AlphaChannel::new("quick", gray));
        doc.layers.push(doc.layers[0].clone());
        let mut cur = Arc::new(doc);
        let bytes = document_bytes(&cur, &mut HashSet::new());
        let mut h = History::new(50);
        // Counting current separately would double count its tiles; the exact union fits.
        h.max_bytes = bytes;
        for _ in 0..6 {
            h.record("Same", cur.clone(), LayerTarget::default());
            assert_eq!(h.trim(&cur), 0);
        }
        assert_counter(&h, &cur);
        for _ in 0..3 {
            cur = h.undo(cur).unwrap().0;
            assert_counter(&h, &cur);
        }
        h.clear_redo();
        h.purge_last();
        assert_counter(&h, &cur);
        assert_eq!(h.pixel_bytes(&cur), bytes);
    }

    #[test]
    fn threshold_rechecks_coalesced_current_and_changed_budget() {
        let mut h = History::new(50);
        let mut cur = Arc::new(base());
        h.max_bytes = usize::MAX;
        for i in 0..6 {
            edit(&mut h, &mut cur, "Paint", |doc| {
                doc.layers[0].surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 10.0, 0.0, 0.0, 1.0]);
            });
            h.trim(&cur);
        }
        // A coalesced edit may change current without recording another history state.
        Arc::make_mut(&mut cur).layers[0].surface_mut().unwrap().write_pixel(300, 1, &[1.0; 4]);
        let tile = 256 * 256 * 4;
        h.max_bytes = 4 * tile;
        assert_eq!(h.trim(&cur), 4);
        assert_eq!(h.past_len(), 2);
        assert_counter(&h, &cur);
        cur = h.undo(cur).unwrap().0;
        assert_counter(&h, &cur);
        h.max_bytes = 0;
        edit(&mut h, &mut cur, "Unlimited", |doc| doc.name = "u".into());
        assert_counter(&h, &cur);
        h.max_bytes = 1;
        h.trim(&cur);
        assert_eq!(h.past_len(), 1, "always retain the most recent undo");
        assert_counter(&h, &cur);
    }

    #[test]
    fn transferred_tiles_cannot_make_the_budget_check_underestimate_current() {
        use photocraft_doc::{PixelFormat, Surface};
        let before = Arc::new(base());
        let mut current = (*before).clone();
        let mut large = Surface::new(PixelFormat::new(ColorMode::Cmyk, SampleType::F32, true));
        large.write_pixel(256, 0, &[1.0; 5]);
        current.layers[0].surface_mut().unwrap().put_tiles(large.take_tiles(photocraft_doc::Rect::from_xywh(256, 0, 1, 1)));
        let mut h = History::new(50);
        h.max_bytes = document_bytes(&current, &mut HashSet::new()) - 1;
        h.record("A", before.clone(), LayerTarget::default());
        h.record("B", before, LayerTarget::default());
        assert_eq!(h.trim(&current), 1, "current alone exceeds the budget; only the mandatory undo survives");
        assert_counter(&h, &current);
    }

    fn assert_counter(h: &History, current: &Document) {
        let own = document_bytes(current, &mut HashSet::new());
        let total = own.saturating_add(h.unique_bytes(current));
        let mut seen = HashSet::new();
        let mut union = document_bytes(current, &mut seen);
        for state in h.resident_states() {
            union = union.saturating_add(document_bytes(&state, &mut seen));
        }
        assert_eq!(h.pixel_bytes(current), total);
        assert_eq!(h.pixel_bytes(current), union);
    }

    fn video_document(depth: SampleType, frames: usize) -> Document {
        let mut doc = Document::new("video", Size::new(64, 64), ColorMode::Rgb, depth);
        let mut layer = Layer::raster("Video", doc.pixel_format());
        let footage: Vec<_> = (0..frames)
            .map(|i| {
                let mut surface = photocraft_doc::Surface::new(doc.pixel_format());
                surface.write_pixel(1, 1, &[i as f32 / frames as f32, 0.0, 0.0, 1.0]);
                surface
            })
            .collect();
        layer.content = photocraft_doc::LayerContent::Raster(footage[0].clone());
        layer.video = Some(photocraft_doc::VideoData::new(footage, 24.0));
        doc.layers.push(layer);
        doc
    }

    #[test]
    fn video_memory_counts_originals_and_deduplicates_displayed_frames() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = video_document(depth, 3);
            let tile = doc.layers[0].surface().unwrap().tiles().next().unwrap().1.bytes().len();
            let video = doc.layers[0].video.as_mut().unwrap();
            video.original_frames = Some(video.frames.clone());
            // Original footage and the displayed frame alias the stack, except this painted frame.
            video.frames[2].write_pixel(1, 1, &[0.0, 1.0, 0.0, 1.0]);
            let mut cur = Arc::new(doc);
            let mut h = History::default();
            h.max_bytes = usize::MAX;
            assert_eq!(h.pixel_bytes(&cur), 4 * tile);
            for name in ["A", "B"] {
                edit(&mut h, &mut cur, "Rename", |doc| doc.name = name.into());
                h.trim(&cur);
            }
            assert_counter(&h, &cur);
            assert_eq!(h.pixel_bytes(&cur), 4 * tile);
            assert_eq!(h.unique_bytes(&cur), 0);
            // Rasterizing keeps the displayed frame; the rest of the clip survives only in undo.
            edit(&mut h, &mut cur, "Rasterize", |doc| doc.layers[0].video = None);
            assert_eq!(h.unique_bytes(&cur), 3 * tile);
            assert_eq!(h.pixel_bytes(&cur), 4 * tile);
            assert_counter(&h, &cur);
            cur = h.undo(cur).unwrap().0;
            assert_eq!(h.unique_bytes(&cur), 0);
            assert_counter(&h, &cur);
            cur = h.redo(cur).unwrap().0;
            assert_eq!(h.unique_bytes(&cur), 3 * tile);
            assert_counter(&h, &cur);
            h.clear();
            assert_eq!(h.pixel_bytes(&cur), tile);
        }
    }

    #[test]
    fn video_memory_budget_evicts_deleted_off_playhead_frames() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut cur = Arc::new(video_document(depth, 6));
            let tile = cur.layers[0].surface().unwrap().tiles().next().unwrap().1.bytes().len();
            let mut h = History::default();
            for _ in 0..4 {
                edit(&mut h, &mut cur, "Delete Frame", |doc| {
                    doc.layers[0].video.as_mut().unwrap().frames.remove(1);
                });
            }
            assert_eq!(h.pixel_bytes(&cur), 6 * tile);
            assert_eq!(h.unique_bytes(&cur), 4 * tile);
            h.max_bytes = 3 * tile;
            assert_eq!(h.trim(&cur), 3);
            assert_eq!(h.past_len(), 1);
            assert_eq!(h.pixel_bytes(&cur), 3 * tile);
            assert_eq!(h.unique_bytes(&cur), tile);
            assert_counter(&h, &cur);
            cur = h.undo(cur).unwrap().0;
            assert_eq!(cur.layers[0].video.as_ref().unwrap().frames.len(), 3);
            assert_counter(&h, &cur);
            cur = h.redo(cur).unwrap().0;
            assert_eq!(cur.layers[0].video.as_ref().unwrap().frames.len(), 2);
            assert_counter(&h, &cur);
            // A budget below the remaining clip still keeps the latest step undoable.
            h.max_bytes = 1;
            assert_eq!(h.trim(&cur), 0);
            assert!(h.can_undo());
        }
    }

    #[test]
    fn counter_tracks_eviction_undo_redo_branching_purge_and_clones() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut cur = Arc::new(Document::with_background("h", Size::new(300, 64), ColorMode::Rgb, depth, Color::WHITE));
            Arc::make_mut(&mut cur).selection = Some(photocraft_doc::Surface::new(photocraft_doc::PixelFormat::GRAY8));
            let mut h = History::new(4);
            h.max_bytes = usize::MAX;
            for i in 0..8 {
                edit(&mut h, &mut cur, "Paint", |doc| {
                    doc.layers[0].surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 10.0, 0.0, 0.0, 1.0]);
                    doc.selection.as_mut().unwrap().write_pixel(1, 1, &[i as f32 / 10.0]);
                });
                h.trim(&cur);
                if i >= 1 {
                    assert_counter(&h, &cur);
                }
            }
            cur = h.undo(cur).unwrap().0;
            assert_counter(&h, &cur);
            cur = h.undo(cur).unwrap().0;
            assert_counter(&h, &cur);
            cur = h.redo(cur).unwrap().0;
            assert_counter(&h, &cur);
            let copy = h.clone();
            edit(&mut h, &mut cur, "Branch", |doc| doc.name = "branch".into());
            assert!(!h.can_redo());
            assert_counter(&h, &cur);
            assert_counter(&copy, &cur);
            cur = h.undo(cur).unwrap().0;
            h.clear_redo();
            assert_counter(&h, &cur);
            h.purge_last();
            assert_counter(&h, &cur);
            h.clear();
            assert!(h.resident_states().is_empty());
            assert_eq!(h.pixel_bytes(&cur), document_bytes(&cur, &mut HashSet::new()));
        }
    }

    #[test]
    fn recovery_checkpoint_preserves_cursor_labels_and_both_stack_orders() {
        let mut history = History::new(37);
        history.max_bytes = 1234;
        let mut current = Arc::new(base());
        for name in ["A", "B", "C", "D"] {
            edit(&mut history, &mut current, name, |doc| doc.name = name.into());
        }
        current = history.try_undo(current).unwrap().unwrap();
        current = history.try_undo(current).unwrap().unwrap();
        assert_eq!(current.name, "B");
        let checkpoint = history.checkpoint();
        assert_eq!(checkpoint.current_label, "B");
        assert_eq!(checkpoint.undo.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["Open", "A"]);
        assert_eq!(checkpoint.redo.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["D", "C"]);
        let mut recovered = History::from_checkpoint(checkpoint).unwrap();
        assert_eq!(recovered.max_states, 37);
        assert_eq!(recovered.max_bytes, 1234);
        for expected in ["C", "D"] {
            current = recovered.try_redo(current).unwrap().unwrap();
            assert_eq!(current.name, expected);
        }
        for expected in ["C", "B", "A"] {
            current = recovered.try_undo(current).unwrap().unwrap();
            assert_eq!(current.name, expected);
        }
        assert_eq!(recovered.redo_label(), Some("B"));
    }

    #[test]
    fn recovery_checkpoint_keeps_cold_states_lazy_and_failed_navigation_atomic() {
        let mut checkpoint = History::new(50).checkpoint();
        checkpoint.current_label = "Current".into();
        checkpoint.undo.push(HistoryState::from_archive("Before", true, Arc::new(Archive(Err("unreadable recovery".into())))));
        checkpoint.redo.push(HistoryState::from_document("After", Arc::new(base())));
        let mut history = History::from_checkpoint(checkpoint).unwrap();
        let captured = history.checkpoint();
        assert!(captured.undo.first().unwrap().has_selection());
        assert!(captured.undo.first().unwrap().load_document().is_err());
        assert_eq!(history.archived_states(), 1);
        assert!(history.has_past_selection());
        assert!(history.try_undo(Arc::new(base())).is_err());
        assert_eq!(history.entries(), ["Before", "Current"]);
        assert_eq!(history.redo_label(), Some("After"));
        assert_eq!(history.past_len(), 1);
    }

    #[test]
    fn recovered_identity_remaps_resident_and_cold_states_without_changing_sources() {
        let original = Arc::new(base());
        let old_id = original.id;
        let new_id = DocId::fresh();
        let mut checkpoint = History::default().checkpoint();
        checkpoint.undo.push(HistoryState::from_archive("Cold", false, Arc::new(Archive(Ok(original.clone())))));
        checkpoint.redo.push(HistoryState::from_document("Resident", original.clone()));
        let mut history = History::from_checkpoint(checkpoint).unwrap();
        history.remap_document_id(new_id);
        assert_eq!(history.archived_states(), 1, "remapping does not hydrate cold history");
        let checkpoint = history.checkpoint();
        assert_eq!(checkpoint.undo[0].load_document().unwrap().id, new_id);
        assert_eq!(checkpoint.redo[0].load_document().unwrap().id, new_id);
        assert_eq!(original.id, old_id, "other tabs retain their identities");
    }

    #[test]
    fn remapped_current_replay_preserves_identity_through_undo_redo() {
        let original = Arc::new(base());
        let new_id = DocId::fresh();
        let mut history = History::default();
        history.record("Move", original.clone(), LayerTarget::default());
        history.set_current_archive(&original, Arc::new(Archive(Ok(original.clone()))));
        history.remap_document_id(new_id);
        let mut current = (*original).clone();
        current.id = new_id;
        let previous = history.try_undo(Arc::new(current)).unwrap().unwrap();
        assert_eq!(previous.id, new_id);
        let restored = history.try_redo(previous).unwrap().unwrap();
        assert_eq!(restored.id, new_id);
        assert_ne!(original.id, new_id);
    }

    #[test]
    fn recovery_checkpoint_rejects_oversized_metadata() {
        let mut checkpoint = History::new(50).checkpoint();
        checkpoint.current_label = "x".repeat(4097);
        assert!(History::from_checkpoint(checkpoint).is_err());
        let mut checkpoint = History::new(50).checkpoint();
        let state = HistoryState::from_document("State", Arc::new(base()));
        checkpoint.undo = vec![state];
        checkpoint.current_identity = StateIdentity { id: 1, parent: Some(1), branch: 0, revision: 0 };
        assert!(History::from_checkpoint(checkpoint).is_err());
        let mut checkpoint = History::new(50).checkpoint();
        checkpoint.max_states = 0;
        assert!(History::from_checkpoint(checkpoint).is_ok());
        let mut checkpoint = History::default().checkpoint();
        checkpoint.current_identity.id = u64::MAX - 1;
        assert!(History::from_checkpoint(checkpoint).is_err(), "hostile identities must leave room for future edits");
    }

    #[test]
    fn cold_restore_and_failed_read_preserve_stacks() {
        let mut history = History::new(50);
        let mut current = Arc::new(base());
        let original = current.clone();
        edit(&mut history, &mut current, "A", |doc| doc.name = "a".into());
        let labels = history.entries();
        assert!(history.replace_resident(&original, Arc::new(Archive(Err("corrupt scratch blob".into())))));
        assert_eq!(history.archived_states(), 1);
        assert!(history.try_state(0).is_err());
        assert!(history.try_undo(current.clone()).is_err());
        assert_eq!(history.entries(), labels);
        assert!(!history.can_redo());
        history.clear();
        history.record("A", original.clone(), LayerTarget::default());
        assert!(history.replace_resident(&original, Arc::new(Archive(Ok(original.clone())))));
        let restored = history.try_undo(current.clone()).unwrap().unwrap();
        assert!(Arc::ptr_eq(&restored, &original));
        assert!(Arc::ptr_eq(&history.try_redo(restored).unwrap().unwrap(), &current));
    }

    #[test]
    fn replacement_deduplicates_and_stale_results_are_ignored() {
        let mut history = History::new(50);
        let original = Arc::new(base());
        history.record("A", original.clone(), LayerTarget::default());
        history.record("B", original.clone(), LayerTarget::default());
        assert!(history.spill_candidate().is_none(), "shared immediate undo allocation stays hot");
        assert!(history.replace_resident(&original, Arc::new(Archive(Ok(original.clone())))));
        assert_eq!(history.archived_states(), 2);
        assert!(history.resident_states().is_empty());
        assert!(!history.replace_resident(&original, Arc::new(Archive(Ok(original.clone())))));
        assert!(history.drop_oldest());
        assert!(!history.drop_oldest());
        history.clear();
        assert_eq!(history.archived_states(), 0);
    }

    #[test]
    fn selection_patterns_and_video_pixels_are_accounted_once() {
        let mut doc = base();
        let format = doc.pixel_format();
        let mut surface = photocraft_doc::Surface::new(format);
        surface.write_pixel(0, 0, &[0.25, 0.5, 0.75, 1.0]);
        doc.selection = Some(surface.clone());
        doc.patterns.push(photocraft_doc::Pattern::new("P", surface.clone(), 1, 1));
        doc.layers[0].video = Some(photocraft_doc::VideoData::new(vec![surface], 24.0));
        let mut seen = HashSet::new();
        assert_eq!(document_bytes(&doc, &mut seen), 2 * 256 * 256 * 4);
        assert_eq!(document_bytes(&doc, &mut seen), 0);
        // Original video frames are shared until an edit replaces one. The new allocation
        // must enter the same resident history budget without counting aliases twice.
        let mut original = doc.selection.clone().unwrap();
        original.write_pixel(0, 0, &[0.1, 0.2, 0.3, 1.0]);
        let video = doc.layers[0].video.as_mut().unwrap();
        video.frames.push(video.frames[0].clone());
        video.original_frames = Some(vec![video.frames[0].clone(), original]);
        seen.clear();
        assert_eq!(document_bytes(&doc, &mut seen), 3 * 256 * 256 * 4);
        assert_eq!(document_bytes(&doc, &mut seen), 0);
    }

    #[test]
    fn cold_redo_failure_keeps_the_next_step() {
        let mut history = History::new(50);
        let mut current = Arc::new(base());
        edit(&mut history, &mut current, "A", |doc| doc.name = "a".into());
        let future = current.clone();
        current = history.try_undo(current).unwrap().unwrap();
        assert!(history.replace_resident(&future, Arc::new(Archive(Err("read failed".into())))));
        let label = history.redo_label().map(str::to_owned);
        assert!(history.try_redo(current).is_err());
        assert_eq!(history.redo_label(), label.as_deref());
        assert!(!history.can_undo());
    }
    #[test]
    fn menu_queries_never_load_a_cold_selection() {
        let mut history = History::new(50);
        let mut doc = base();
        doc.selection = Some(photocraft_doc::Surface::new(doc.pixel_format()));
        let doc = Arc::new(doc);
        history.record("Select", doc.clone(), LayerTarget::default());
        assert!(history.replace_resident(&doc, Arc::new(Archive(Err("must not read".into())))));
        assert!(history.has_past_selection());
        assert_eq!(history.latest_selection_state(), Some(0));
        history.record("Clear selection", Arc::new(base()), LayerTarget::default());
        assert_eq!(history.latest_selection_state(), Some(0), "skip newer no-selection states without loading archives");
        assert!(history.resident_state(0).is_none());
        assert!(history.try_state(0).is_err());
    }

    #[test]
    fn spill_and_drop_keep_adjacent_steps_hot_and_contiguous() {
        let mut history = History::new(50);
        let mut current = Arc::new(base());
        let oldest = current.clone();
        for label in ["A", "B", "C"] {
            edit(&mut history, &mut current, label, |doc| doc.name = label.into());
        }
        assert!(Arc::ptr_eq(&history.spill_candidate().unwrap(), &oldest));
        assert!(history.drop_oldest());
        assert_eq!(history.entries(), ["A", "B", "C"]);
        current = history.try_undo(current).unwrap().unwrap();
        current = history.try_undo(current).unwrap().unwrap();
        assert_eq!(current.name, "A");
        assert!(history.drop_oldest(), "discard farthest redo");
        assert_eq!(history.redo_label(), Some("B"));
        assert!(!history.drop_oldest());
        assert_eq!(history.try_redo(current).unwrap().unwrap().name, "B");
    }
    #[test]
    fn explicit_budget_reduction_can_discard_final_cold_state() {
        let mut history = History::new(50);
        let doc = Arc::new(base());
        history.record("A", doc.clone(), LayerTarget::default());
        history.replace_resident(&doc, Arc::new(Archive(Err("must not load".into()))));
        assert!(!history.drop_oldest());
        assert!(history.discard_oldest());
        assert!(!history.can_undo());
        assert!(!history.discard_oldest());
    }

    #[test]
    fn lowered_count_bounds_both_stacks_without_loading() {
        let mut history = History::new(50);
        let mut current = Arc::new(base());
        for label in ["A", "B", "C"] {
            edit(&mut history, &mut current, label, |doc| doc.name = label.into());
        }
        current = history.undo(current).unwrap().0;
        history.max_states = 1;
        assert_eq!(history.enforce_state_limit(), 2);
        assert_eq!(history.past_len(), 0);
        assert_eq!(history.redo_label(), Some("C"));
        assert_eq!(history.redo(current).unwrap().0.name, "C");
    }
    #[test]
    fn asynchronous_candidate_can_become_hot_after_navigation() {
        let mut history = History::new(50);
        let mut current = Arc::new(base());
        let oldest = current.clone();
        for label in ["A", "B", "C"] {
            edit(&mut history, &mut current, label, |doc| doc.name = label.into());
        }
        assert!(!history.is_hot(&oldest));
        current = history.undo(current).unwrap().0;
        let _current = history.undo(current).unwrap().0;
        assert!(history.is_hot(&oldest));
        assert!(history.spill_candidate().is_none_or(|candidate| !Arc::ptr_eq(&candidate, &oldest)));
    }
    #[test]
    fn large_shared_binary_payloads_are_counted_once() {
        let mut doc = base();
        let bytes = Arc::new(vec![17; 4096]);
        doc.icc_profile = Some(bytes.clone());
        doc.metadata.exif = Some(bytes.clone());
        doc.metadata.psd_resources.push((1, "resource".into(), bytes.clone()));
        doc.metadata.psd_global_blocks.push((*b"8BIM", *b"lnk2", bytes.clone()));
        doc.layers[0].psd_blocks.push((*b"test", bytes));
        let baseline = 256 * 256 * 4;
        let mut seen = HashSet::new();
        assert_eq!(document_bytes(&doc, &mut seen), baseline + 4096);
        let mut clone = doc.clone();
        clone.metadata.exif = Some(Arc::new(vec![4; 1024]));
        assert_eq!(document_bytes(&clone, &mut seen), 1024);
    }
}

#[cfg(test)]
mod direct_history_tests {
    use super::*;
    use photocraft_doc::{ColorMode, SampleType, Size};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn doc() -> Arc<Document> {
        Arc::new(Document::new("Original", Size::new(2, 2), ColorMode::Rgb, SampleType::U8))
    }
    fn edit(history: &mut History, current: &mut Arc<Document>, label: &str) {
        history.record(label, current.clone(), LayerTarget::default());
        let mut next = (**current).clone();
        next.name = label.into();
        *current = Arc::new(next);
    }
    #[test]
    fn unlimited_retains_original_and_more_than_ten_thousand_states() {
        let mut history = History::default();
        let mut current = doc();
        for i in 0..10_050 {
            edit(&mut history, &mut current, &format!("Brush {i}"));
        }
        assert_eq!(history.past_len(), 10_050);
        let state = history.state_by_id(current.clone(), 0).unwrap();
        let original = state.load_document().unwrap();
        history.jump_loaded(current, 0, original.clone()).unwrap();
        assert_eq!(original.name, "Original");
        assert_eq!(history.timeline().len(), 10_051);
        assert!(History::from_checkpoint(history.checkpoint()).is_ok());
    }
    #[test]
    fn alternative_branches_and_cursor_survive_checkpoint_and_direct_jumps() {
        let mut history = History::default();
        let mut current = doc();
        edit(&mut history, &mut current, "Paint");
        edit(&mut history, &mut current, "Erase");
        let erased_id = history.current_id();
        current = history.try_undo(current).unwrap().unwrap();
        edit(&mut history, &mut current, "Move layer");
        let moved_id = history.current_id();
        assert!(!history.can_redo());
        assert_eq!(history.timeline().len(), 4);
        assert_ne!(history.timeline()[2].identity.branch, history.timeline()[3].identity.branch);
        let mut restored = History::from_checkpoint(history.checkpoint()).unwrap();
        let erased = restored.state_by_id(current.clone(), erased_id).unwrap().load_document().unwrap();
        restored.jump_loaded(current, erased_id, erased.clone()).unwrap();
        assert_eq!(erased.name, "Erase");
        let moved = restored.state_by_id(erased.clone(), moved_id).unwrap().load_document().unwrap();
        restored.jump_loaded(erased, moved_id, moved.clone()).unwrap();
        assert_eq!(moved.name, "Move layer");
        assert_eq!(restored.try_undo(moved).unwrap().unwrap().name, "Paint");
    }
    #[derive(Debug)]
    struct Counted {
        document: Arc<Document>,
        reads: Arc<AtomicUsize>,
        fail: bool,
    }
    impl ArchivedDocument<PhotoCraftHistory> for Counted {
        fn load(&self) -> Result<Arc<Document>, String> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            if self.fail { Err("corrupt archive".into()) } else { Ok(self.document.clone()) }
        }
    }
    #[test]
    fn jumping_over_cold_states_reads_only_the_destination_and_failures_are_atomic() {
        let mut history = History::default();
        let mut current = doc();
        for i in 0..80 {
            edit(&mut history, &mut current, &format!("Edit {i}"));
        }
        let reads = Arc::new(AtomicUsize::new(0));
        for state in history.checkpoint().undo {
            let document = state.load_document().unwrap();
            history.replace_resident(&document, Arc::new(Counted { document: document.clone(), reads: reads.clone(), fail: false }));
        }
        let target = history.state_by_id(current.clone(), 0).unwrap().load_document().unwrap();
        history.jump_loaded(current.clone(), 0, target).unwrap();
        assert_eq!(reads.load(Ordering::Relaxed), 1);
        assert_eq!(history.timeline().len(), 81);
        let before = history.timeline_stamp();
        let bad = HistoryState::from_archive("Bad", false, Arc::new(Counted { document: current.clone(), reads: reads.clone(), fail: true }));
        assert!(bad.load_document().is_err());
        assert!(history.jump_loaded(current.clone(), u64::MAX, current).is_err());
        assert_eq!(history.timeline_stamp(), before);
    }
}
