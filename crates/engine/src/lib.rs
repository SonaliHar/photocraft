//! The Photocraft engine façade: open documents, history, and the command registry.
//!
//! Every user-visible action is a command with a stable id (Photoshop-style, such as
//! `layer.newAdjustmentLayer.invert`) and JSON parameters. Every frontend goes through the same
//! [`Session::execute`] entry point: the egui UI, the CLI, the remote-control channel and the MCP
//! server. This is what makes the UI swappable and the app fully scriptable.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod accounted_commands;
mod accounted_session;
pub mod accounted_view;
pub mod actions_cmds;
pub mod adjust_cmds;
pub mod adjust_params;
pub mod align_cmds;
pub mod analysis_cmds;
pub mod artboard_cmds;
pub mod automate_cmds;
pub mod brush_cmds;
pub mod brush_key_cmds;
pub mod brush_preset_cmds;
pub mod build_info;
mod canvas_geom;
pub mod channel_cmds;
pub mod color_cmds;
pub mod commands;
pub mod comps_cmds;
pub mod cutout_cmds;
pub mod display_color;
pub mod distort_cmds;
pub mod document_history;
pub mod document_preset_cmds;
pub mod document_snapshot;
pub mod edit_cmds;
pub mod edit_menu_cmds;
mod edit_validation;
pub mod eraser_cmds;
pub mod extra_cmds;
pub mod file_cmds;
pub mod fill_cmds;
pub mod fill_key_cmds;
pub mod filters;
pub mod filters_ext;
pub mod float_cmds;
mod frame_cmds;
pub mod fx_view_cmds;
pub mod gallery_cmds;
pub mod gradient_fill_cmds;
pub mod group_view_cmds;
pub mod hidden_target;
pub mod image_cmds;
pub mod inspect;
pub mod inspect_pixel_cmds;
pub mod jobs;
pub mod layer_copy_cmds;
pub mod layer_export_wide;
pub mod layer_label_cmds;
pub mod layer_menu_cmds;
pub mod layer_multi_cmds;
mod layer_duplicate_controlled;
mod layer_mask_storage_controlled;
pub mod layer_nav_cmds;
pub mod layer_style;
pub mod lens_cmds;
pub mod magnetic_cmds;
pub mod mask_view_cmds;
mod migrate_cmds;
pub mod mode_cmds;
pub mod multichannel_cmds;
pub mod notes_cmds;
pub mod paint_cmds;
mod path_edit_cmds;
pub mod pattern_cmds;
pub mod photo_cmds;
pub mod pick_cmds;
mod pixels;
pub mod plugin_cmds;
pub mod prefs;
pub mod preset_import_cmds;
pub mod preset_store;
pub mod presets;
pub mod print_cmds;
pub mod proof_sim;
pub mod redeye_cmds;
pub mod render_cmds;
pub mod retained_surface_validation;
pub mod retouch_cmds;
pub mod select_extra_cmds;
pub mod selection_cmds;
pub mod slice_cmds;
pub mod smart_cmds;
pub mod smartselect_cmds;
pub mod snap;
mod source_policy;
#[cfg(test)]
mod session_target_edit_tests;
pub mod stamp_cmds;
pub mod swatch_cmds;
pub mod symmetry_cmds;
mod timeline_cmds;
pub mod transform_cmds;
mod trap_cmds;
pub mod type_caret_cmds;
pub mod type_cmds;
pub mod type_extra_cmds;
pub mod type_spell_cmds;
pub mod type_styles_cmds;
mod variables_cmds;
pub mod vector_cmds;
mod video_cmds;
pub mod vp_cmds;
pub mod warp_cmds;
pub mod web_cmds;
mod web_export_delivery;
mod wia_cmds;
pub mod xcf_assessment_cmds;
pub mod xcf_composition_cmds;
pub mod xcf_editing;
mod xcf_query_preflight;
pub mod xcf_item_locks;
pub mod xcf_item_state_cmds;
pub mod xcf_item_set_create_cmds;
pub mod xcf_item_set_manage_cmds;
pub mod xcf_mask_cmds;
pub mod xcf_mask_geometry_cmds;
pub mod xcf_filter_mask_cmds;
pub mod xcf_selection_cmds;
pub mod xcf_channel_state_cmds;
pub mod xcf_channel_name_cmds;
pub mod xcf_channel_locks_cmds;
pub mod xcf_channel_indicates_cmds;
pub mod xcf_wide_point_cmds;
pub mod xcf_filter_wire;
pub mod xcf_filter_insert_cmds;
pub mod xcf_indexed_pixel_cmds;
pub mod xcf_indexed_palette_cmds;
pub mod xcf_text_transform_cmds;
pub mod xcf_vector_style_cmds;
pub mod xcf_path_state_cmds;
pub mod xcf_filter_graph_cmds;
pub mod xcf_filter_cmds;
pub mod xcf_floating;
mod xcf_indexed_geometry;
pub mod xcf_inspect_cmds;
pub mod xcf_link_cmds;
pub mod xcf_link_geometry_cmds;
pub mod xcf_link_substitution_cmds;
pub mod xcf_resource_cmds;
pub mod xcf_resources;
pub mod xcf_text_cmds;
pub mod xcf_vector_cmds;
mod xcf_vector_guard;
pub use source_policy::{SourceIdentity, WriteProtection};

use std::sync::{Arc, Mutex};

use photocraft_doc::{Document, LayerId};
use photocraft_ops::{History, LayerTarget};
use serde_json::Value;

pub use commands::{CommandSpec, command_specs};
pub use document_history::DocumentHistory;
pub use document_snapshot::{DocumentSnapshot, WeakDocumentSnapshot};
pub use photocraft_doc as doc;
pub use photocraft_paint as paint;
pub use photocraft_paint::BrushSettings;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("command `{0}` is not available right now: {1}")]
    Disabled(String, String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    #[error("no active document")]
    NoDocument,
    #[error("no such layer {0:?}")]
    NoLayer(LayerId),
    #[error("{0}")]
    Other(String),
    /// A background job was cancelled (see [`jobs`]); nothing changed.
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// Legacy synchronous callers have no job cancellation/work callback.
/// Accounted operations still enforce their persistent quota and fixed bounds.
pub fn read_control_none() -> photocraft_raster::backing::ReadControl<'static> {
    photocraft_raster::backing::ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &|_| Ok(()) }
}

impl From<photocraft_raster::RasterInputError> for EngineError {
    fn from(error: photocraft_raster::RasterInputError) -> Self {
        if error == photocraft_raster::RasterInputError::Cancelled { Self::Cancelled } else { Self::Other(error.to_string()) }
    }
}

impl From<photocraft_raster::backing::BackingError> for EngineError {
    fn from(error: photocraft_raster::backing::BackingError) -> Self {
        if error == photocraft_raster::backing::BackingError::Cancelled { Self::Cancelled } else { Self::Other(error.to_string()) }
    }
}

/// The pixels of a raster layer (typically one a command just created), as an error instead
/// of a panic if the layer has none.
pub(crate) fn pixels_mut(l: &mut photocraft_doc::Layer) -> Result<&mut photocraft_raster::Surface> {
    let id = l.id;
    l.surface_mut().ok_or_else(|| EngineError::Other(format!("layer {id:?} has no pixels")))
}

/// The active document's active layer, for `enabled` predicates.
pub(crate) fn active_layer_of(s: &Session) -> std::result::Result<&photocraft_doc::Layer, String> {
    let d = s.active().ok_or("no document open")?;
    d.active_layer.and_then(|id| d.doc.layer(id)).ok_or_else(|| "no active layer".into())
}

/// Per-document editing state.
#[derive(Debug)]
pub struct DocState {
    pub doc: DocumentSnapshot,
    // A Weak keeps pointer identity trustworthy without retaining old pixels.
    // Arc COW mutations dissociate weak references, invalidating this cache.
    surface_validation: Arc<Mutex<WeakDocumentSnapshot>>,
    pub history: DocumentHistory,
    /// The primary ("key") layer: what single-layer commands act on. Always a member of
    /// `selected_layers` when set.
    pub active_layer: Option<LayerId>,
    /// Every selected layer in the Layers panel (⌘/⇧-click), in selection order. Like
    /// `active_layer`, selecting is not a history step, but undo and redo restore the layers each
    /// state targeted when the next step was taken; see [`DocState::selected_layers`].
    pub selected_layers: Vec<LayerId>,
    /// Anchor of ⇧-click range selection (the last plainly or ⌘-clicked layer).
    pub layer_anchor: Option<LayerId>,
    pub path: Option<String>,
    /// Original import provenance, separate from Save As destination and undo history.
    pub source: Option<SourceIdentity>,
    /// Increments on every change; UIs re-render when it moves.
    pub revision: u64,
    pub saved_revision: u64,
    /// Area changed by the latest revision (None = assume everything changed).
    pub last_damage: Option<photocraft_geom::Rect>,
    /// Coalescing key of the latest history step (see [`Session::execute`]'s `coalesce` param).
    pub coalesce: Option<String>,
    /// Channels panel: targeted channel and eye toggles (view state, not history).
    pub channel_view: channel_cmds::ChannelView,
    /// Select › Isolate Layers: the Layers panel lists only these layers (empty = off; view state).
    pub isolated_layers: Vec<LayerId>,
    /// Painting symmetry axis made from the selected path (tool state, not document pixels).
    pub symmetry_path: Option<symmetry_cmds::SymmetryAxis>,
    /// Layers panel: layers whose effects list is collapsed under their row (the fx triangle;
    /// view state, not history). Effects lists start open.
    pub fx_collapsed: Vec<LayerId>,
    /// ⌥-click on a layer's eye (`layer.showOnly`): the layer shown alone and every layer's
    /// visibility before, so the next ⌥-click restores it (view state, not history).
    pub show_only: Option<(LayerId, Vec<(LayerId, bool)>)>,
    /// A floating selection (`select.float`): the cut piece and where it floats, until dropped
    /// (view state: the document is unchanged until `select.drop`).
    pub floating: Option<float_cmds::Floating>,
    accounted_view: Option<accounted_view::AccountedView>,
    _accounted_metadata: Option<photocraft_raster::backing::quota::QuotaLease>,
}

impl DocState {
    /// Whole editing-state copies are a resident-only compatibility operation.
    /// Accounted models require typed staged view/history transactions.
    pub fn try_clone_resident(&self) -> Result<Self> {
        self.doc.validate_resident()?;
        Ok(Self {
            doc: self.doc.clone(),
            surface_validation: self.surface_validation.clone(),
            history: self.history.clone(),
            active_layer: self.active_layer,
            selected_layers: self.selected_layers.clone(),
            layer_anchor: self.layer_anchor,
            path: self.path.clone(),
            source: self.source.clone(),
            revision: self.revision,
            saved_revision: self.saved_revision,
            last_damage: self.last_damage,
            coalesce: self.coalesce.clone(),
            channel_view: self.channel_view.clone(),
            isolated_layers: self.isolated_layers.clone(),
            symmetry_path: self.symmetry_path.clone(),
            fx_collapsed: self.fx_collapsed.clone(),
            show_only: self.show_only.clone(),
            floating: self.floating.clone(),
            accounted_view: None,
            _accounted_metadata: None,
        })
    }
    /// Record the detected policy without granting filesystem authority.
    pub fn set_import_origin(&mut self, origin: photocraft_io::ImportOrigin, label: impl Into<String>, path: Option<String>) {
        self.source = Some(SourceIdentity::new(label, path.clone(), origin));
        self.path = if origin.is_read_only() { None } else { path };
    }

    pub fn new(doc: Document, path: Option<String>) -> Result<Self> {
        Self::new_with_interrupt(doc, path, &photocraft_raster::Interrupt::NONE)
    }
    /// Restore resident editing/history view with cooperative structural work.
    /// Source decoders retain full model/sample validation responsibility.
    pub fn new_with_interrupt(doc: Document, path: Option<String>, control: &photocraft_raster::Interrupt<'_>) -> Result<Self> {
        let poll = || control.check().map_err(|_| EngineError::Cancelled);
        poll()?;
        let doc = DocumentSnapshot::from_resident_with_interrupt(Arc::new(doc), control).map_err(|error| {
            if matches!(error, photocraft_raster::backing::BackingError::Cancelled) { EngineError::Cancelled } else { EngineError::from(error) }
        })?;
        let mut actual = std::collections::HashSet::new();
        photocraft_doc::resident_structure::visit_layers_with(&doc, control, |layer| {
            actual.try_reserve(1).map_err(|_| photocraft_raster::RasterInputError::AllocationFailed)?;
            actual.insert(layer.id);
            Ok(true)
        })
        .map_err(|error| if error == photocraft_raster::RasterInputError::Cancelled { EngineError::Cancelled } else { EngineError::from(error) })?;
        let selected = doc.xcf_state.as_ref().map_or(&[][..], |state| state.selection.selected.as_slice());
        if selected.len() > 10_000 {
            return Err(EngineError::Other("imported layer selection exceeds canonical limit".into()));
        }
        let mut selected_layers = Vec::new();
        poll()?;
        selected_layers
            .try_reserve_exact(selected.len().checked_add(1).ok_or_else(|| EngineError::Other("selection allocation limit".into()))?)
            .map_err(|_| EngineError::Other("selection allocation limit".into()))?;
        poll()?;
        for item in selected {
            poll()?;
            if let photocraft_doc::item_identity::ItemRef::Layer(id) = item
                && actual.contains(id)
            {
                selected_layers.push(*id);
            }
        }
        let stored_active = doc
            .xcf_state
            .as_ref()
            .and_then(|state| state.selection.active)
            .and_then(|item| if let photocraft_doc::item_identity::ItemRef::Layer(id) = item { actual.contains(&id).then_some(id) } else { None });
        let channel_view = channel_cmds::imported_view_with_interrupt(&doc, control)?;
        let active_layer = channel_view.editing_mask.or(stored_active).or_else(|| selected_layers.first().copied()).or_else(|| doc.top_layer());
        if let Some(id) = active_layer
            && !selected_layers.contains(&id)
        {
            selected_layers.push(id);
        }
        poll()?;
        let mut history_selected = Vec::new();
        history_selected.try_reserve_exact(selected_layers.len()).map_err(|_| EngineError::Other("history view allocation limit".into()))?;
        poll()?;
        for chunk in selected_layers.chunks(1024) {
            poll()?;
            history_selected.extend_from_slice(chunk);
        }
        let mut history = History::default();
        history.set_current_layers(LayerTarget { active: active_layer, selected: history_selected });
        poll()?;
        let state = Self {
            doc,
            surface_validation: Arc::new(Mutex::new(WeakDocumentSnapshot::new())),
            history: DocumentHistory::resident(history),
            active_layer,
            selected_layers,
            layer_anchor: active_layer,
            path,
            source: None,
            revision: 1,
            saved_revision: 1,
            last_damage: None,
            coalesce: None,
            channel_view,
            isolated_layers: Vec::new(),
            symmetry_path: None,
            fx_collapsed: Vec::new(),
            show_only: None,
            floating: None,
            accounted_view: None,
            _accounted_metadata: None,
        };
        poll()?;
        Ok(state)
    }
    pub(crate) fn validate_retained_surfaces(&self) -> std::result::Result<(), String> {
        self.validate_retained_surfaces_with(&photocraft_raster::Interrupt::NONE)
    }
    fn validate_retained_surfaces_with(&self, control: &photocraft_raster::Interrupt<'_>) -> std::result::Result<(), String> {
        control.check().map_err(|error| error.to_string())?;
        let key = self.doc.downgrade();
        let cached = self.surface_validation.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cached.ptr_eq(&key) {
            return Ok(());
        }
        drop(cached);
        retained_surface_validation::validate_document(&self.doc, control).map_err(|error| format!("invalid retained raster: {error}"))?;
        *self.surface_validation.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = key;
        Ok(())
    }
    /// The selected layers in bottom-to-top document order, always including the active layer.
    /// Robust against stale state: ids no longer in the document are skipped, and if the active
    /// layer was changed without updating the set, the selection is just the active layer.
    pub fn selection_ids(&self) -> &[LayerId] {
        self.accounted_view.as_ref().map_or(&self.selected_layers, accounted_view::AccountedView::selected)
    }
    /// A frame/job may share the charged selection owner alongside its model
    /// snapshot. Its borrowed IDs never detach from their physical allocation.
    pub fn accounted_selection(&self) -> Option<&accounted_view::AccountedView> {
        self.accounted_view.as_ref()
    }
    pub fn selected_layers(&self) -> Result<Vec<LayerId>> {
        self.doc.validate_resident()?;
        let Some(active) = self.active_layer else { return Ok(Vec::new()) };
        let set: &[LayerId] = if self.selected_layers.contains(&active) { &self.selected_layers } else { std::slice::from_ref(&active) };
        Ok(self.doc.walk().into_iter().map(|(_, _, l)| l.id).filter(|id| set.contains(id)).collect())
    }
    pub fn is_layer_selected(&self, id: LayerId) -> bool {
        self.active_layer == Some(id) || (self.active_layer.is_some_and(|a| self.selection_ids().contains(&a)) && self.selection_ids().contains(&id))
    }
    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }
    /// The targeted layers, as history stores them with each state.
    fn layer_target(&self) -> Result<LayerTarget> {
        self.doc.validate_resident()?;
        Ok(LayerTarget { active: self.active_layer, selected: self.selected_layers.clone() })
    }
}

/// App-wide tool state that commands read (foreground colour, brush…).
#[derive(Clone, Debug)]
pub struct ToolState {
    pub foreground: [f32; 4],
    pub background: [f32; 4],
    pub brush: photocraft_paint::BrushSettings,
    /// Brush presets (built-ins plus user presets; see `brush.presets.*`).
    pub presets: Vec<photocraft_paint::BrushPreset>,
    /// Bumped whenever `presets` changes ([`Session::brush_presets_changed`]); the preset store
    /// syncs when it moves.
    pub presets_rev: u64,
    /// Mixer Brush paint carried between strokes.
    pub mixer: photocraft_paint::mixer::MixerState,
    /// The coalescing key of the running `tools.setBrush` gesture and the brush before it, so the
    /// gesture journals as one call ([`brush_cmds::coalesce_journal`]).
    pub brush_gesture: Option<(String, photocraft_paint::BrushSettings)>,
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            foreground: [0.0, 0.0, 0.0, 1.0],
            background: [1.0, 1.0, 1.0, 1.0],
            // Photoshop's Brush tool starts at 10 % Smoothing.
            brush: photocraft_paint::BrushSettings {
                smoothing: photocraft_paint::brush::Smoothing { amount: 0.1, ..Default::default() },
                ..Default::default()
            },
            presets: photocraft_paint::presets::builtin(),
            presets_rev: 0,
            mixer: Default::default(),
            brush_gesture: None,
        }
    }
}

#[derive(Default)]
pub struct Session {
    /// Explicit platform capability; only Save for Web may invoke it.
    web_export_delivery: Option<web_export_delivery::Delivery>,
    docs: Vec<DocState>,
    accounted_slots: Option<(photocraft_raster::backing::quota::BackingQuota, photocraft_raster::backing::quota::QuotaLease)>,
    active: Option<usize>,
    pub tools: ToolState,
    /// Log of executed commands (for action recording and debugging).
    pub journal: Vec<(String, Value)>,
    /// Coalescing key of the command being executed.
    coalesce_request: Option<String>,
    /// Pixels copied with Edit › Copy / Cut (shared by all documents, like Photoshop).
    pub clipboard: Option<edit_cmds::Clip>,
    /// Layer › Layer Style › Copy Layer Style: effects, blend mode, fill opacity and the Advanced
    /// Blending switches.
    pub style_clipboard: Option<(photocraft_doc::Effects, photocraft_color::BlendMode, f32, photocraft_doc::AdvancedBlending)>,
    /// Shape path context menu: copied vector fill and stroke styles.
    pub path_fill_clipboard: Option<photocraft_doc::Fill>,
    pub path_stroke_clipboard: Option<photocraft_doc::ShapeStroke>,
    /// Colour management: proofing state, monitor profile, display transforms.
    pub color: color_cmds::ColorState,
    /// Open Edit Contents documents and the smart objects they update.
    pub smart_links: Vec<smart_cmds::SmartLink>,
    /// Type › Save Default Type Styles: character and paragraph style new type layers start from.
    pub type_defaults: Option<(photocraft_doc::text::CharStyle, photocraft_doc::text::ParagraphStyle)>,
    /// Quick Mask Options (colour, opacity, colour indicates) used when entering Quick Mask.
    pub quick_mask_options: channel_cmds::QuickMaskOptions,
    /// Colour channel the running command may change (a single colour channel is targeted).
    color_restrict: Option<usize>,
    /// Edit › Preferences, keyboard shortcuts, menu and toolbar customisation (see `prefs`).
    pub prefs: prefs::PrefsStore,
    /// Edit menu state: Fade source, custom shape library (see `edit_menu_cmds`).
    pub edit_state: edit_menu_cmds::EditState,
    /// The pattern library (Window › Patterns; see `pattern.*`).
    pub patterns: pattern_cmds::PatternLibrary,
    /// Image › Analysis: Measurement Log and Select Data Points (see `analysis_cmds`).
    pub analysis: analysis_cmds::AnalysisState,
    /// Window › Gradients, Patterns (groups), Styles, Shapes, Tool Presets and Clone Source.
    pub presets: presets::PresetState,
    /// File menu state: Lock Slices, Image Assets, last Print / Save for Web settings, script
    /// event log (see `automate_cmds`).
    pub file_menu: automate_cmds::FileMenuState,
    /// Persistent brush preset store (desktop only; `None` keeps presets session-only, as in
    /// headless and test sessions). See `preset_store`. The same store holds the Actions list.
    pub preset_store: Option<preset_store::PresetStore>,
    /// Window › Actions. The list persists with the preset store when one is attached.
    pub actions: actions_cmds::ActionState,
    /// Gate for every command the session runs, including the ones a command runs on its own
    /// behalf and `actions.play` steps. Untrusted sessions (MCP, the control channel) install
    /// the same check a top-level request sees. `None` runs everything, which is what a local
    /// UI and `photocraft-cli run` do.
    pub authorize: Option<fn(&str, &serde_json::Value) -> Result<()>>,
    /// Background jobs (see [`jobs`]).
    jobs: jobs::Jobs,
    // The public journal Vec is destroyed before its retained allocation lease.
    accounted_journal: Option<photocraft_raster::backing::quota::QuotaLease>,
}

/// Move item `i` of `v` to position `to`, clamped to the end. Returns where it went; `None` when
/// `i` is out of range.
pub fn move_item<T>(v: &mut Vec<T>, i: usize, to: usize) -> Option<usize> {
    if i >= v.len() {
        return None;
    }
    let x = v.remove(i);
    let to = to.min(v.len());
    v.insert(to, x);
    Some(to)
}

impl Session {
    pub(crate) fn publish_accounted_journal(&mut self, journal: Vec<(String, Value)>, lease: photocraft_raster::backing::quota::QuotaLease) {
        // Drop the previous allocation while its old charge is still retained.
        self.journal = journal;
        self.accounted_journal = Some(lease);
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Install explicit byte-delivery authority for Save for Web only.
    /// Absent by default; does not grant other filesystem commands a writer.
    pub fn set_web_export_delivery(&mut self, callback: Option<web_export_delivery::Deliver>) {
        self.web_export_delivery = callback.map(web_export_delivery::Delivery::new);
    }

    pub fn has_web_export_delivery(&self) -> bool {
        self.web_export_delivery.is_some()
    }

    pub fn documents(&self) -> &[DocState] {
        &self.docs
    }
    pub fn active_index(&self) -> Option<usize> {
        self.active
    }
    pub fn active(&self) -> Option<&DocState> {
        self.active.and_then(|i| self.docs.get(i))
    }
    pub fn active_mut(&mut self) -> Option<&mut DocState> {
        self.active.and_then(|i| self.docs.get_mut(i))
    }
    pub fn set_active(&mut self, index: usize) -> bool {
        if index < self.docs.len() {
            self.active = Some(index);
            true
        } else {
            false
        }
    }

    /// Move the document at `from` to tab position `to` (clamped to the last), keeping the active
    /// document active. Returns its new index; `None` when `from` is out of range.
    pub fn move_document(&mut self, from: usize, to: usize) -> Option<usize> {
        let active = self.active().map(|d| d.doc.id);
        let to = move_item(&mut self.docs, from, to)?;
        self.active = active.and_then(|id| self.docs.iter().position(|d| d.doc.id == id));
        Some(to)
    }

    /// Add a document (from File → New, an import, etc.) and make it active.
    /// Preserve its identity unless another open document already owns it.
    /// Identity-dependent callers must read the admitted document at the returned index.
    pub fn add_document(&mut self, doc: Document, path: Option<String>) -> Result<usize> {
        self.add_document_with_interrupt(doc, path, &photocraft_raster::Interrupt::NONE)
    }

    /// Prepare the incoming editing state privately, then publish it after the
    /// final cancellation check. Failure leaves the existing tabs and focus intact.
    pub fn add_document_with_interrupt(&mut self, mut doc: Document, path: Option<String>, control: &photocraft_raster::Interrupt<'_>) -> Result<usize> {
        control.check().map_err(|_| EngineError::Cancelled)?;
        // Persisted IDs can overlap the allocator, so check every replacement too.
        let mut attempts = 0usize;
        loop {
            let mut collision = false;
            for state in &self.docs {
                control.check().map_err(|_| EngineError::Cancelled)?;
                if state.doc.id == doc.id { collision = true; break; }
            }
            if !collision { break; }
            attempts = attempts.checked_add(1).filter(|count| *count <= self.docs.len().saturating_add(1))
                .ok_or_else(|| EngineError::Other("document identity allocation exhausted".into()))?;
            doc.id = photocraft_doc::DocId::fresh();
        }
        let mut st = DocState::new_with_interrupt(doc, path, control)?;
        st.history.max_states = self.prefs.get().performance.history_states.max(1) as usize;
        st.history.max_bytes = self.prefs.get().performance.history_budget_bytes();
        let read_control = photocraft_raster::backing::ReadControl { interrupt: control, work: &|_| Ok(()) };
        self.publish_document_state(st, &read_control)
    }

    pub fn close(&mut self, index: usize) -> Option<DocState> {
        if index >= self.docs.len() {
            return None;
        }
        smart_cmds::on_close(self, index);
        if let Some(id) = self.docs.get(index).map(|d| d.doc.id) {
            self.cancel_jobs_on(id);
        }
        let d = self.docs.remove(index);
        if self.docs.is_empty() {
            drop(std::mem::take(&mut self.docs));
            self.accounted_slots = None;
        }
        self.active = if self.docs.is_empty() {
            None
        } else {
            Some(match self.active {
                Some(active) if active > index => active - 1,
                Some(active) if active < index => active,
                _ => index.min(self.docs.len() - 1),
            })
        };
        Some(d)
    }

    /// Run a command by id with JSON params. Returns a JSON result.
    ///
    /// Any command accepts an optional `"coalesce": "<key>"` param: consecutive edits with the same
    /// key (and no other edit, undo or redo in between) share one history step, like Photoshop's
    /// single "Edit Type Layer" step for a whole typing session or one step per slider drag.
    ///
    /// Runs synchronously, including job-capable commands (see [`jobs`]; [`Session::start`] runs
    /// those in the background).
    pub fn execute(&mut self, id: &str, params: Value) -> Result<Value> {
        match self.dispatch(id, params, false)? {
            jobs::Started::Done(v) => Ok(v),
            // Not reached: inline dispatch never starts a job. Waiting is still correct.
            jobs::Started::Job(j) => self.wait_job(j),
        }
    }

    /// Is the command currently runnable? (drives menu enablement)
    pub fn is_enabled(&self, id: &str) -> bool {
        self.is_enabled_with(id, &Value::Null)
    }

    /// [`Self::is_enabled`] for a call with `params`: their `"target"` can enable a command, as a
    /// targeted layer mask enables Image › Adjustments › Invert on an adjustment layer (#780).
    pub fn is_enabled_with(&self, id: &str, params: &Value) -> bool {
        self.disabled_reason_with(id, params).is_none()
    }

    /// Why the command can't run now (None = it can): its own precondition, or a background job
    /// running on the active document.
    pub fn disabled_reason(&self, id: &str) -> Option<String> {
        self.disabled_reason_with(id, &Value::Null)
    }

    /// [`Self::disabled_reason`] for a call with `params`.
    pub fn disabled_reason_with(&self, id: &str, params: &Value) -> Option<String> {
        let Some(s) = commands::find(id) else { return Some(format!("unknown command `{id}`")) };
        if self.active().is_some_and(|state| state.doc.accounted().is_some()) && accounted_commands::supports(id) {
            return match accounted_commands::disabled_reason(self, id, params) {
                Ok(reason) => reason.map(str::to_string).or_else(|| self.job_conflict(id, s.journal)),
                Err(error) => Some(error.to_string()),
            };
        }
        if let Some(state) = self.active()
            && let Err(error) = state.validate_retained_surfaces()
        {
            return Some(error);
        }
        self.precondition(s, &channel_cmds::inject_target(self, id, params.clone())).err().or_else(|| self.job_conflict(id, s.journal))
    }

    /// The command's own precondition for a call with `params` (their target filled in).
    fn precondition(&self, spec: &commands::CommandSpec, params: &Value) -> std::result::Result<(), String> {
        if self.active().is_some_and(|state| state.doc.accounted().is_some()) {
            return Err("this command has not yet been qualified for an accounted native document".into());
        }
        channel_cmds::mask_target_enabled(self, spec.id, params).unwrap_or_else(|| (spec.enabled)(self))
    }

    /// Apply an undoable edit to the active document.
    pub fn edit<R>(&mut self, label: &str, f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>) -> Result<R> {
        self.edit_with_interrupt(label, &photocraft_raster::Interrupt::NONE, f)
    }

    /// Stage an undoable edit with cooperative validation and a cancellation
    /// fence immediately before document installation and history recording.
    /// The closure must pass the same control through its own expensive work.
    pub fn edit_with_interrupt<R>(
        &mut self,
        label: &str,
        control: &photocraft_raster::Interrupt<'_>,
        f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>,
    ) -> Result<R> {
        let index = self.active.ok_or(EngineError::NoDocument)?;
        self.edit_index_with_interrupt(index, label, control, None, f)
    }

    /// Commit to a caller-fenced resident document without changing tab focus.
    /// The caller must fence identity/revision and destination jobs before entry.
    pub(crate) fn edit_document_with_interrupt<R>(
        &mut self,
        index: usize,
        label: &str,
        control: &photocraft_raster::Interrupt<'_>,
        f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>,
    ) -> Result<R> {
        control.check().map_err(|_| EngineError::Cancelled)?;
        let target = self.docs.get(index).ok_or(EngineError::NoDocument)?;
        if target.floating.is_some() || target.coalesce.is_some()
            || target.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
            || self.coalesce_request.is_some() || self.color_restrict.is_some()
        {
            return Err(EngineError::Other("finish transient edits before editing this document".into()));
        }
        self.edit_index_with_interrupt(index, label, control, None, f)
    }

    /// Commit a prepared selection and its history target with the document.
    /// No fallible reselection step runs after publication.
    pub(crate) fn edit_with_layer_target_with_interrupt<R>(
        &mut self,
        label: &str,
        control: &photocraft_raster::Interrupt<'_>,
        target: LayerTarget,
        anchor: Option<LayerId>,
        f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>,
    ) -> Result<R> {
        let index = self.active.ok_or(EngineError::NoDocument)?;
        if self.coalesce_request.is_some() || self.color_restrict.is_some() {
            return Err(EngineError::Other("finish transient edits before replacing layer selection".into()));
        }
        self.edit_index_with_interrupt(index, label, control, Some((target, anchor)), f)
    }

    fn edit_index_with_interrupt<R>(
        &mut self,
        index: usize,
        label: &str,
        control: &photocraft_raster::Interrupt<'_>,
        target: Option<(LayerTarget, Option<LayerId>)>,
        f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>,
    ) -> Result<R> {
        control.check().map_err(|_| EngineError::Cancelled)?;
        let restrict = self.color_restrict;
        let st = self.docs.get_mut(index).ok_or(EngineError::NoDocument)?;
        st.validate_retained_surfaces_with(control).map_err(|error| if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(error) })?;
        st.doc.validate_resident()?;
        let revision = st.revision.checked_add(1).ok_or_else(|| EngineError::Other("document revision exhausted".into()))?;
        let before = st.doc.clone();
        // Selecting layers is not a step, so the state this edit leaves behind targets what was
        // selected just before it (#1356): undoing a stroke keeps the painted layer active.
        let prior = st.layer_target()?;
        let mut doc = (*before).clone();
        let mut active = st.active_layer;
        let r = f(&mut doc, &mut active)?;
        let (mut selected, mut anchor) = match target {
            Some((target, anchor)) => {
                if target.selected.len() > 10_000 {
                    return Err(EngineError::Other("prepared layer selection exceeds limit".into()));
                }
                let mut seen = std::collections::HashSet::new();
                seen.try_reserve(target.selected.len()).map_err(|_| EngineError::Other("prepared layer selection allocation".into()))?;
                for id in &target.selected {
                    control.check().map_err(|_| EngineError::Cancelled)?;
                    if id.0 == 0 || !seen.insert(*id) || doc.layer(*id).is_none() {
                        return Err(EngineError::Other("invalid prepared layer selection".into()));
                    }
                }
                if target.active.is_some_and(|id| !seen.contains(&id)) || anchor.is_some_and(|id| doc.layer(id).is_none()) {
                    return Err(EngineError::Other("invalid prepared active layer or anchor".into()));
                }
                active = target.active;
                (target.selected, anchor)
            }
            None => (st.selected_layers.clone(), st.layer_anchor),
        };
        edit_validation::validate(&before, &mut doc, active, restrict, control)?;
        let snapshot = DocumentSnapshot::from_resident(Arc::new(doc))?;
        fix_selection_parts(&snapshot, active, &mut selected, &mut anchor);
        let layers = LayerTarget { active, selected: selected.clone() };
        let key = self.coalesce_request.clone();
        let st = self.docs.get_mut(index).ok_or(EngineError::NoDocument)?;
        let mut history = st.history.clone();
        let history_control = photocraft_raster::backing::ReadControl { interrupt: control, work: &|_| Ok(()) };
        if key.is_none() || st.coalesce != key || !history.can_undo() {
            history.set_current_layers(prior, &history_control)?;
            history.record(label, before, layers, &history_control)?;
            history.trim(&snapshot, &history_control)?;
        } else {
            history.set_current_layers(layers, &history_control)?;
        }
        control.check().map_err(|_| EngineError::Cancelled)?;
        st.doc = snapshot;
        st.history = history;
        st.active_layer = active;
        st.selected_layers = selected;
        st.layer_anchor = anchor;
        channel_cmds::fix_view(st);
        st.coalesce = key;
        st.revision = revision;
        st.last_damage = None;
        Ok(r)
    }

    /// Replace the active document's active layer without creating a history step.
    pub fn select_layer(&mut self, id: LayerId) -> Result<()> {
        let st = self.active_mut().ok_or(EngineError::NoDocument)?;
        if st.doc.layer(id).is_none() {
            return Err(EngineError::NoLayer(id));
        }
        let revision = st.revision.checked_add(1).ok_or_else(|| EngineError::Other("document revision exhausted".into()))?;
        if let (Some(owner), Some(view)) = (st.doc.accounted(), &st.accounted_view) {
            let selection = view.try_replace(owner, Some(id), &[id], Some(id), &read_control_none())?;
            st.accounted_view = Some(selection);
        } else {
            st.selected_layers = vec![id];
        }
        st.active_layer = Some(id);
        st.layer_anchor = Some(id);
        // Selecting a layer is not an edit: keep a clean document clean.
        let clean = st.saved_revision == st.revision;
        st.revision = revision;
        if clean {
            st.saved_revision = st.revision;
        }
        st.last_damage = Some(photocraft_geom::Rect::EMPTY);
        Ok(())
    }

    pub fn undo(&mut self) -> bool {
        self.undo_with_control(&read_control_none()).unwrap_or(false)
    }

    pub fn redo(&mut self) -> bool {
        self.redo_with_control(&read_control_none()).unwrap_or(false)
    }

    pub fn undo_with_control(&mut self, control: &photocraft_raster::backing::ReadControl<'_>) -> Result<bool> {
        self.travel_history(false, control)
    }
    pub fn redo_with_control(&mut self, control: &photocraft_raster::backing::ReadControl<'_>) -> Result<bool> {
        self.travel_history(true, control)
    }
    fn travel_history(&mut self, redo: bool, control: &photocraft_raster::backing::ReadControl<'_>) -> Result<bool> {
        control.check(1)?;
        if self.active_job().is_some() {
            return Err(EngineError::Other("the document has an active background job".into()));
        }
        let Some(st) = self.active_mut() else { return Ok(false) };
        let mut history = st.history.clone();
        let change = if redo { history.redo(&st.doc, control)? } else { history.undo(&st.doc, control)? };
        let Some(change) = change else {
            st.coalesce = None;
            return Ok(false);
        };
        let (snapshot, target) = change.into_parts();
        let revision = st.revision.checked_add(1).ok_or(photocraft_raster::backing::BackingError::Length)?;
        if let Some(owner) = snapshot.accounted() {
            let values = target.values();
            let view = accounted_view::AccountedView::new(owner, values.active, &values.selected, values.active, control)?;
            control.check(1)?;
            st.active_layer = values.active;
            st.layer_anchor = values.active;
            st.accounted_view = Some(view);
            st.doc = snapshot;
            st.history = history;
            st.revision = revision;
            st.last_damage = None;
            st.coalesce = None;
            return Ok(true);
        }
        let (mut values, lease) = target.into_parts();
        if lease.is_some() {
            return Err(photocraft_raster::backing::BackingError::Integrity.into());
        }
        if values.active.is_none_or(|id| snapshot.layer(id).is_none()) {
            values.active = snapshot.top_layer();
        }
        let mut anchor = st.layer_anchor;
        fix_selection_parts(&snapshot, values.active, &mut values.selected, &mut anchor);
        let damage = if redo { layer_multi_cmds::step_damage(&st.doc, &snapshot) } else { layer_multi_cmds::step_damage(&snapshot, &st.doc) };
        control.check(1)?;
        st.doc = snapshot;
        st.history = history;
        st.active_layer = values.active;
        st.selected_layers = values.selected;
        st.layer_anchor = anchor;
        channel_cmds::fix_view(st);
        st.coalesce = None;
        st.revision = revision;
        st.last_damage = damage;
        Ok(true)
    }
}

/// Keep the layer selection valid after the document or the active layer changed: drop deleted
/// layers, and collapse to the active layer when a command made a non-member active.
pub(crate) fn fix_selection(st: &mut DocState) {
    if st.accounted_view.is_some() {
        return;
    }
    fix_selection_parts(&st.doc, st.active_layer, &mut st.selected_layers, &mut st.layer_anchor);
}

fn fix_selection_parts(doc: &Document, active: Option<LayerId>, selected: &mut Vec<LayerId>, anchor: &mut Option<LayerId>) {
    let Some(active) = active else {
        selected.clear();
        return;
    };
    if !selected.contains(&active) {
        *selected = vec![active];
    } else {
        let live: std::collections::HashSet<LayerId> = doc.walk().into_iter().map(|(_, _, l)| l.id).collect();
        selected.retain(|id| live.contains(id));
        let mut seen = std::collections::HashSet::new();
        selected.retain(|id| seen.insert(*id));
    }
    if anchor.is_some_and(|a| !selected.contains(&a)) {
        *anchor = Some(active);
    }
}

#[cfg(test)]
mod tests;
