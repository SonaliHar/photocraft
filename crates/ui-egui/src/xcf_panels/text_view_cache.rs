//! Derived text hints only, never an edit/font/resource capability. A single
//! context-local entry includes negative results and is invalidated by exact
//! snapshot identity as well as revision, document and canonical layer identity.
use photocraft_doc::{DocId, Document, Layer, LayerContent, LayerId, item_identity::ItemRef, xcf_items::EditableSource};
use photocraft_engine::document_snapshot::{DocumentSnapshot, WeakDocumentSnapshot};
use photocraft_text::xcf::TextColor;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct TextHints {
    pub source_index: Option<usize>,
    pub content_editable: bool,
    pub font_size: Option<u32>,
    pub color: Option<TextColor>,
    pub display_rgb: Option<[f32; 3]>,
    pub color_editable: bool,
    pub spacing: Option<f32>,
    pub spacing_editable: bool,
    pub line_spacing: Option<f32>,
    pub line_spacing_editable: bool,
    pub origin: Option<[i32; 2]>,
    pub position_editable: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CacheError { Limit }
impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("xcf.text.properties.limit") }
}
#[derive(Clone)]
struct Cache {
    document: DocId, revision: u64, layer: LayerId,
    snapshot: WeakDocumentSnapshot, result: Result<TextHints, CacheError>,
}
impl Cache {
    fn matches(&self, snapshot: &DocumentSnapshot, revision: u64, layer: LayerId) -> bool {
        self.document == snapshot.id && self.revision == revision && self.layer == layer
            && self.snapshot.upgrade().is_some_and(|old| old.shares_state(snapshot))
    }
}
fn layer_exists(document: &Document, target: LayerId) -> Result<bool, CacheError> {
    let mut frames: [Option<std::slice::Iter<'_, Layer>>; photocraft_doc::MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    *frames.first_mut().ok_or(CacheError::Limit)? = Some(document.layers.iter());
    let (mut depth, mut visits, mut found) = (0usize, 0usize, false);
    loop {
        let next = frames.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else { if depth == 0 { return Ok(found); } depth -= 1; continue };
        visits = visits.checked_add(1).filter(|visits| *visits <= 10_000).ok_or(CacheError::Limit)?;
        if layer.id == target { if found { return Err(CacheError::Limit); } found = true; }
        if let LayerContent::Group(group) = &layer.content {
            depth = depth.checked_add(1).filter(|depth| *depth <= photocraft_doc::MAX_GROUP_DEPTH).ok_or(CacheError::Limit)?;
            *frames.get_mut(depth).ok_or(CacheError::Limit)? = Some(group.children.iter());
        }
    }
}
fn prepare(document: &Document, layer: LayerId) -> Result<TextHints, CacheError> {
    if layer.0 == 0 { return Ok(TextHints::default()); }
    let Some(state) = &document.xcf_state else { return Ok(TextHints::default()) };
    if state.items.len() > 10_000 || state.source_items.len() > 10_000 || state.resources.len() > 10_000 { return Err(CacheError::Limit); }
    let mut sources = state.source_items.iter().enumerate().filter(|(_, source)| source.owner == ItemRef::Layer(layer));
    let Some((source_index, source)) = sources.next() else { return Ok(TextHints::default()) };
    if sources.next().is_some() || !matches!(source.source, EditableSource::Text { .. }) || !layer_exists(document, layer)? { return Ok(TextHints::default()); }
    // Existing authoritative helpers perform their own complete bounded state,
    // cache/hash, exact-font and layout checks. They run only on a cache miss.
    // Cold preparation still repeats helper qualification; no aggregate/RSS or
    // single-pass qualification claim is implied by this view cache.
    use photocraft_engine::xcf_text_cmds as text;
    let color = text::current_text_color(document, layer);
    Ok(TextHints {
        source_index: Some(source_index),
        content_editable: text::content_edit_candidate(document, layer),
        font_size: text::current_font_size(document, layer),
        color, display_rgb: color.and_then(|color| super::text_color_rgb(document, color)),
        color_editable: text::color_edit_candidate(document, layer),
        spacing: text::current_letter_spacing(document, layer),
        spacing_editable: text::letter_spacing_edit_candidate(document, layer),
        line_spacing: text::current_line_spacing(document, layer),
        line_spacing_editable: text::line_spacing_edit_candidate(document, layer),
        origin: text::current_text_origin(document, layer),
        position_editable: text::position_edit_candidate(document, layer),
    })
}
fn get_with(
    context: &egui::Context, snapshot: &DocumentSnapshot, revision: u64, layer: LayerId,
    derive: impl FnOnce(&Document, LayerId) -> Result<TextHints, CacheError>,
) -> Result<TextHints, CacheError> {
    let key = egui::Id::new("xcf_text_property_bundle");
    if let Some(cache) = context.data_mut(|data| data.get_temp::<Cache>(key))
        && cache.matches(snapshot, revision, layer) { return cache.result; }
    let result = derive(snapshot, layer);
    context.data_mut(|data| data.insert_temp(key, Cache { document: snapshot.id, revision, layer, snapshot: snapshot.downgrade(), result }));
    result
}
pub(super) fn get(context: &egui::Context, snapshot: &DocumentSnapshot, revision: u64, layer: LayerId) -> Result<TextHints, CacheError> {
    get_with(context, snapshot, revision, layer, prepare)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::{Arc, atomic::{AtomicUsize, Ordering}}};
    fn document() -> Document { Document::new("Own text hint cache", photocraft_doc::Size::new(2, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8) }
    #[test]
    fn bundle_hits_and_negative_results_derive_once_revision_layer_and_weak_invalidate() {
        let mut snapshot = DocumentSnapshot::from_resident(Arc::new(document())).unwrap();
        let context = egui::Context::default();
        let calls = AtomicUsize::new(0);
        let derive = |_: &Document, _: LayerId| { calls.fetch_add(1, Ordering::Relaxed); Ok(TextHints { font_size: Some(24), spacing: Some(2.), ..Default::default() }) };
        let first = get_with(&context, &snapshot, 7, LayerId(9), derive).unwrap();
        assert_eq!(first, get_with(&context, &snapshot, 7, LayerId(9), derive).unwrap());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        get_with(&context, &snapshot, 8, LayerId(9), derive).unwrap();
        get_with(&context, &snapshot, 8, LayerId(10), derive).unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 3);
        snapshot.try_resident_mut().unwrap().name = "Changed snapshot at same revision".into();
        get_with(&context, &snapshot, 8, LayerId(10), derive).unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 4);
        let unavailable = |_: &Document, _: LayerId| { calls.fetch_add(1, Ordering::Relaxed); Err(CacheError::Limit) };
        assert_eq!(get_with(&context, &snapshot, 9, LayerId(10), unavailable), Err(CacheError::Limit));
        assert_eq!(get_with(&context, &snapshot, 9, LayerId(10), derive), Err(CacheError::Limit));
        assert_eq!(calls.load(Ordering::Relaxed), 5);
        let other = DocumentSnapshot::from_resident(Arc::new(document())).unwrap();
        get_with(&context, &other, 9, LayerId(10), derive).unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 6);
    }
    #[test]
    fn absent_source_is_cached_and_count_depth_bounds_precede_helper_qualification() {
        let snapshot = DocumentSnapshot::from_resident(Arc::new(document())).unwrap();
        let context = egui::Context::default();
        let calls = AtomicUsize::new(0);
        let derive = |doc: &Document, layer| { calls.fetch_add(1, Ordering::Relaxed); prepare(doc, layer) };
        assert_eq!(get_with(&context, &snapshot, 0, LayerId(1), derive).unwrap(), TextHints::default());
        assert_eq!(get_with(&context, &snapshot, 0, LayerId(1), derive).unwrap(), TextHints::default());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let mut doc = document();
        let source = photocraft_doc::xcf_items::SourceItem { owner: ItemRef::Layer(LayerId(9)), source: EditableSource::Text { content: String::new(), markup: None, font: photocraft_doc::item_identity::ResourceId(1), direction: 0, source_modified: false, configuration: Vec::new(), current_origin: None }, transform: None, cache: None };
        doc.xcf_state = Some(photocraft_doc::XcfState { source_items: vec![source; 10_001], ..Default::default() });
        assert_eq!(prepare(&doc, LayerId(9)), Err(CacheError::Limit));
        let mut leaf = photocraft_doc::Layer::raster("Own deep leaf", doc.pixel_format());
        let id = leaf.id;
        for _ in 0..=photocraft_doc::MAX_GROUP_DEPTH { leaf = photocraft_doc::Layer::group("Own group", vec![leaf]); }
        doc.layers = vec![leaf];
        assert_eq!(layer_exists(&doc, id), Err(CacheError::Limit));
    }
    #[test]
    #[ignore = "retained text hint/cache consistency draft; not executed"]
    fn retained_current_bundle_matches_existing_helpers_without_document_mutation() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("retained qualified text corpus"));
        let document = photocraft_io::import("source.xcf", &std::fs::read(root.join("text-stem-evidence/simple/source.xcf")).unwrap()).unwrap().document;
        let owner = document.xcf_state.as_ref().unwrap().source_items.iter().find(|source| matches!(source.source, EditableSource::Text { .. })).unwrap().owner;
        let ItemRef::Layer(layer) = owner else { panic!("text layer") };
        let before = document.clone();
        let snapshot = DocumentSnapshot::from_resident(Arc::new(document)).unwrap();
        let context = egui::Context::default();
        let cold = get(&context, &snapshot, 0, layer).unwrap();
        assert!(cold.source_index.is_some());
        assert_eq!(cold, prepare(&snapshot, layer).unwrap());
        assert_eq!(cold, get(&context, &snapshot, 0, layer).unwrap());
        assert_eq!(*snapshot, before);
    }
}
