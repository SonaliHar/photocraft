//! The Photocraft engine façade: open documents, history, and the command registry.
//!
//! Every user-visible action is a command with a stable id (Photoshop-style, such as
//! `layer.newAdjustmentLayer.invert`) and JSON parameters. Every frontend goes through the same
//! [`Session::execute`] entry point: the egui UI, the CLI, the remote-control channel and the MCP
//! server. This is what makes the UI swappable and the app fully scriptable.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod accounted_commands;
pub use accounted_commands::{filtered_flat_filter_edit_candidate, masked_filtered_flat_filter_edit_candidate, normal_hierarchy_filter_edit_candidate};
pub(crate) mod accounted_clear;
pub(crate) mod accounted_color_range;
pub(crate) mod accounted_color_range_banded;
pub(crate) mod accounted_fill;
pub mod accounted_filter_inspect;
pub mod accounted_native_open;
mod accounted_native_pixels;
pub mod accounted_native_save;
pub mod accounted_plain_xcf_inspect;
pub(crate) mod accounted_select_all;
mod accounted_session;
pub mod accounted_xcf_inspect;
mod imported_backed_sampling;
pub use accounted_session::metadata_gesture::{assigned_geometry_metadata_candidate, observed_box_translation_candidate, owned_layer_metadata_candidate};
pub mod accounted_selection_session;
pub mod accounted_view;
mod accounted_xcf_open;
pub mod actions_cmds;
pub mod actions_edit_cmds;
pub mod adjust_cmds;
pub mod adjust_params;
pub mod align_cmds;
mod allocation;
pub mod analysis_cmds;
pub mod artboard_cmds;
pub mod atn_cmds;
pub mod automate_cmds;
pub mod background_cmds;
pub mod brush_cmds;
pub mod brush_key_cmds;
pub mod brush_preset_cmds;
pub mod build_info;
mod canvas_geom;
mod channel_clip;
pub mod channel_cmds;
mod clip_group;
pub mod color_cmds;
pub mod color_to_alpha_cmds;
pub mod commands;
pub mod comps_cmds;
mod crop_fill;
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
pub mod export_preset_cmds;
pub mod exr_cmds;
pub mod extra_cmds;
pub mod face_cmds;
pub mod file_cmds;
pub mod fill_cmds;
pub mod fill_key_cmds;
pub mod filters;
pub mod filters_ext;
pub mod float_cmds;
mod frame_cmds;
pub mod fx_view_cmds;
pub mod fx_visibility_cmds;
pub mod gallery_cmds;
pub mod gradient_fill_cmds;
pub mod group_view_cmds;
pub mod guide_layout;
mod held_import_controlled;
pub mod hidden_target;
pub mod history_cmds;
pub mod history_select_cmds;
pub mod image_cmds;
pub mod inspect;
pub mod inspect_pixel_cmds;
pub mod installed_profiles;
pub mod jobs;
pub mod kys;
pub mod layer_copy_cmds;
mod layer_duplicate_controlled;
pub mod layer_export_wide;
pub mod layer_label_cmds;
pub mod layer_list_cmds;
pub mod layer_mask_props_cmds;
mod layer_mask_storage_controlled;
pub mod layer_menu_cmds;
pub mod layer_multi_cmds;
pub mod layer_nav_cmds;
pub mod layer_style;
pub mod lens_cmds;
pub mod link_watch;
pub mod magnetic_cmds;
pub mod mask_view_cmds;
mod merge_publication_controlled;
mod migrate_cmds;
pub mod mode_cmds;
pub mod multichannel_cmds;
mod native_adjustment_alpha;
mod native_gray_curves_controlled;
mod native_gray_desaturate_controlled;
pub(crate) mod native_gray_fill_controlled;
pub(crate) mod native_gray_invert_controlled;
mod native_gray_legacy_brightness_contrast_controlled;
mod native_gray_levels_controlled;
pub(crate) mod native_gray_posterize_controlled;
pub(crate) mod native_gray_threshold_controlled;
mod native_linear_context;
mod native_raster_curves_controlled;
mod native_raster_desaturate_controlled;
mod native_raster_exposure_controlled;
mod native_raster_invert_controlled;
mod native_raster_legacy_brightness_contrast_controlled;
mod native_raster_levels_controlled;
mod native_raster_posterize_controlled;
mod native_raster_threshold_controlled;
mod native_tone_lut;
pub mod new_layer;
pub mod notes_cmds;
pub mod object_finder_controlled;
pub mod paint_cmds;
mod param_check;
mod path_edit_cmds;
pub mod pattern_cmds;
pub mod pattern_fill_cmds;
mod pending_open_cmds;
pub mod perspective_crop_cmds;
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
pub mod rasterize_style_cmds;
pub mod redeye_cmds;
pub mod render_cmds;
pub mod retained_surface_validation;
pub mod retouch_cmds;
pub mod sample_cmds;
pub mod sampler_cmds;
pub mod select_extra_cmds;
pub mod selection_cmds;
mod selection_focus_controlled;
mod selection_grow_controlled;
mod selection_inverse_controlled;
mod selection_mesh_nearest_controlled;
mod selection_modify_controlled;
mod selection_morphology_controlled;
mod selection_object_controlled;
mod selection_polygon_controlled;
mod selection_quick_controlled;
mod selection_refine_controlled;
mod selection_sample_policy;
mod selection_shapes_controlled;
mod selection_sky_controlled;
mod selection_smooth_controlled;
mod selection_subject_controlled;
mod selection_transform_controlled;
mod selection_wand_controlled;
#[cfg(test)]
mod session_target_edit_tests;
pub mod shell_commands;
pub mod shortcut_sets;
pub mod slice_cmds;
pub mod smart_cmds;
pub mod smartselect_cmds;
pub mod snap;
pub mod solid_fill_cmds;
mod source_policy;
mod split_publication_controlled;
pub mod stamp_cmds;
pub mod swatch_cmds;
pub mod symmetry_cmds;
mod timeline_cmds;
pub mod transform_cmds;
mod trap_cmds;
pub mod type_caret_cmds;
pub mod type_cmds;
pub mod type_extra_cmds;
pub mod type_mask_cmds;
pub mod type_spell_cmds;
pub mod type_styles_cmds;
mod variables_cmds;
pub use variables_cmds::{export_data_sets_with, import_data_sets_text};
pub mod vector_cmds;
mod video_cmds;
pub use video_cmds::render_video_with;
mod inspect_xcf_typed;
pub mod vp_cmds;
pub mod warp_cmds;
pub mod web_cmds;
mod web_export_delivery;
mod wia_cmds;
pub mod xcf_assessment_cmds;
pub mod xcf_channel_consumer;
pub mod xcf_channel_gaussian_prepare;
pub mod xcf_channel_host_load;
pub mod xcf_channel_indicates_cmds;
pub mod xcf_channel_locks_cmds;
pub mod xcf_channel_name_cmds;
pub mod xcf_channel_state_cmds;
pub mod xcf_channel_stored_load;
pub mod xcf_composition_cmds;
pub mod xcf_editing;
pub mod xcf_filter_cmds;
pub mod xcf_filter_graph_cmds;
pub mod xcf_filter_insert_cmds;
pub mod xcf_filter_mask_cmds;
pub mod xcf_filter_wire;
pub mod xcf_floating;
mod xcf_indexed_geometry;
pub mod xcf_indexed_palette_cmds;
pub mod xcf_indexed_pixel_cmds;
pub mod xcf_inspect_cmds;
pub mod xcf_item_locks;
pub mod xcf_item_set_create_cmds;
pub mod xcf_item_set_manage_cmds;
pub mod xcf_item_state_cmds;
pub mod xcf_layer_fractional_gaussian_prepare;
pub mod xcf_layer_gaussian_prepare;
pub mod xcf_layer_stored_prepare;
pub mod xcf_link_cmds;
pub mod xcf_link_geometry_cmds;
pub mod xcf_link_substitution_cmds;
pub mod xcf_mask_cmds;
pub mod xcf_mask_geometry_cmds;
pub mod xcf_path_state_cmds;
mod xcf_query_preflight;
pub mod xcf_resource_cmds;
pub mod xcf_resources;
pub mod xcf_selection_cmds;
pub mod xcf_text_cmds;
pub mod xcf_text_transform_cmds;
pub mod xcf_vector_cmds;
mod xcf_vector_guard;
pub mod xcf_vector_style_cmds;
pub mod xcf_wide_point_cmds;
pub use source_policy::{AccountedSaveProtection, SourceIdentity, WriteProtection};

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
    /// [`Self::NoLayer`] as a command reports it: with the layers the document does have, as
    /// ids are not positions (#5058).
    #[error("no such layer {}; {known}", id.0)]
    UnknownLayer { id: LayerId, known: String },
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

impl From<photocraft_raster::AllocationError> for EngineError {
    fn from(error: photocraft_raster::AllocationError) -> Self {
        Self::Other(format!("not enough memory for this operation ({error}); the document was not changed"))
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
    /// An Affinity document (or its preview) must not acquire the native source as its Save path.
    pub source_read_only: bool,
    /// Increments on every change; UIs re-render when it moves.
    pub revision: u64,
    pub saved_revision: u64,
    /// How much of `revision` came from view-only changes (channel eyes, targeting): pixel caches can ignore those.
    pub view_revision: u64,
    /// Area changed by the latest revision (None = assume everything changed).
    pub last_damage: Option<photocraft_geom::Rect>,
    /// Coalescing key of the latest history step (see [`Session::execute`]'s `coalesce` param).
    pub coalesce: Option<String>,
    /// Channels panel: targeted channel and eye toggles (view state, not history).
    pub channel_view: channel_cmds::ChannelView,
    /// Select › Isolate Layers: the Layers panel lists only these layers (empty = off; view state).
    pub isolated_layers: Vec<LayerId>,
    /// Painting symmetry preset or sampled path (tool state, not document pixels).
    pub symmetry: Option<symmetry_cmds::PaintingSymmetry>,
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
    /// Retains the separately prepared native save path after publication.
    _accounted_save_identity: Option<photocraft_raster::backing::quota::QuotaLease>,
    /// Exact paid metadata gesture key; model/source validation stays per request.
    _accounted_metadata_gesture: Option<accounted_session::metadata_gesture::PaidMetadataGesture>,
    /// Color Sampler points (`view.colorSamplers.*`, #1046): numbered sample positions in document
    /// pixel coordinates. Per-document view state: discarded when the document closes, never saved
    /// to `.pcraft` or PSD, and not a history step.
    pub color_samplers: Vec<[f64; 2]>,
    /// History panel snapshots, oldest first (`history.newSnapshot`, #2966). Kept for the
    /// session only, as in Photoshop: not history states, and not saved with the document.
    pub snapshots: Vec<history_cmds::Snapshot>,
    /// The id the next snapshot gets; ids are never reused, so deleting one keeps the others.
    next_snapshot: u64,
}

impl DocState {
    /// Trim the history to its budget, counting the tiles only History panel snapshots hold
    /// against it too (#4530). Returns how many states were dropped.
    pub fn trim_history(&mut self) -> Result<usize> {
        self.trim_history_with_interrupt(&photocraft_raster::Interrupt::NONE)
    }

    /// Prepare a bounded pinned-snapshot-aware trim privately before replacing history.
    pub fn trim_history_with_interrupt(&mut self, control: &photocraft_raster::Interrupt<'_>) -> Result<usize> {
        let history = self.history.resident_try_clone_trim_with_interrupt(
            &self.doc,
            self.snapshots.iter().map(|snapshot| &*snapshot.doc),
            self.history.max_states,
            self.history.max_bytes,
            photocraft_ops::ResidentHistoryLimits::default(),
            control,
        )?;
        control.check().map_err(|_| EngineError::Cancelled)?;
        let dropped = self.history.past_len().saturating_sub(history.past_len());
        self.history = history;
        Ok(dropped)
    }

    /// Pixel bytes of the document, the tiles only its history holds and the tiles only its
    /// History panel snapshots hold. Accounted histories use their independent quota census.
    pub fn memory_bytes(&self) -> Result<usize> {
        self.memory_bytes_with_interrupt(&photocraft_raster::Interrupt::NONE)
    }

    /// Count resident native tile ownership with bounded metadata and cooperative traversal.
    pub fn memory_bytes_with_interrupt(&self, control: &photocraft_raster::Interrupt<'_>) -> Result<usize> {
        let (current_and_history, snapshots, _) = self.history.resident_memory_bytes_with_interrupt(
            &self.doc,
            self.snapshots.iter().map(|snapshot| &*snapshot.doc),
            photocraft_ops::ResidentHistoryLimits::default(),
            control,
        )?;
        current_and_history.checked_add(snapshots).ok_or(photocraft_raster::RasterInputError::LimitExceeded.into())
    }

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
            source_read_only: self.source_read_only,
            revision: self.revision,
            view_revision: self.view_revision,
            saved_revision: self.saved_revision,
            last_damage: self.last_damage,
            coalesce: self.coalesce.clone(),
            channel_view: self.channel_view.clone(),
            isolated_layers: self.isolated_layers.clone(),
            symmetry: self.symmetry.clone(),
            fx_collapsed: self.fx_collapsed.clone(),
            show_only: self.show_only.clone(),
            floating: self.floating.clone(),
            color_samplers: self.color_samplers.clone(),
            snapshots: self.snapshots.clone(),
            next_snapshot: self.next_snapshot,
            accounted_view: None,
            _accounted_metadata: None,
            _accounted_save_identity: None,
            _accounted_metadata_gesture: None,
        })
    }
    /// Record the detected policy without granting filesystem authority.
    pub fn set_import_origin(&mut self, origin: photocraft_io::ImportOrigin, label: impl Into<String>, path: Option<String>) {
        self.source = Some(SourceIdentity::new(label, path.clone(), origin));
        self.source_read_only |= origin.is_read_only();
        self.path = if self.source_read_only { None } else { path };
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
            source_read_only: false,
            revision: 1,
            saved_revision: 1,
            view_revision: 0,
            last_damage: None,
            coalesce: None,
            channel_view,
            isolated_layers: Vec::new(),
            symmetry: None,
            fx_collapsed: Vec::new(),
            show_only: None,
            floating: None,
            color_samplers: Vec::new(),
            snapshots: Vec::new(),
            next_snapshot: 1,
            accounted_view: None,
            _accounted_metadata: None,
            _accounted_save_identity: None,
            _accounted_metadata_gesture: None,
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
    /// Name of the preset the current brush was last picked from. Kept across later edits (the
    /// preset stays "the current brush" so `brush.presets.update` can overwrite it); cleared when
    /// the brush is reset or the preset is gone.
    pub current_preset: Option<String>,
    /// Bumped whenever `brush` or `current_preset` changes ([`ToolState::brush_changed`]). The
    /// last-brush persistence watches it instead of comparing the brush itself, so an unchanged
    /// brush costs a counter compare per frame, not a clone and serialise.
    pub brush_rev: u64,
    /// The revision whose value is in `Preferences::last_brush` (a restored one counts as saved):
    /// [`brush_rev`](Self::brush_rev) == `brush_saved_rev` means there is nothing new to write.
    pub brush_saved_rev: u64,
    /// A layer mask is the edit target ([`ToolState::target_mask`]).
    pub mask_targeted: bool,
    /// The foreground/background pair of the edit target that isn't current: the layer pixels'
    /// colours while a mask is targeted, the mask's while the pixels are (#2166).
    pub other_colors: [[f32; 4]; 2],
    /// Colours picked as CMYK, newest last: the sRGB colour that stands for them and the ink
    /// values chosen. The tool colours are stored as sRGB, and an sRGB round trip does not give
    /// back what was typed (100/0/0/0 cyan is outside sRGB), so a CMYK document is filled with
    /// the inks the person picked (#4196). Matched by colour, so it follows a swap, a reset or a
    /// mask pair without bookkeeping.
    pub cmyk_picks: Vec<([f32; 3], [f32; 4])>,
}

impl ToolState {
    /// How many CMYK picks are remembered.
    const CMYK_PICKS: usize = 8;

    /// Remember that sRGB colour `rgb` was picked as the inks `cmyk` (each 0..1).
    pub fn remember_cmyk(&mut self, rgb: [f32; 3], cmyk: [f32; 4]) {
        self.cmyk_picks.retain(|(c, _)| *c != rgb);
        self.cmyk_picks.push((rgb, cmyk));
        let extra = self.cmyk_picks.len().saturating_sub(Self::CMYK_PICKS);
        self.cmyk_picks.drain(..extra);
    }

    /// The inks `rgb` was picked as, if it was picked as CMYK.
    pub fn picked_cmyk(&self, rgb: [f32; 3]) -> Option<[f32; 4]> {
        self.cmyk_picks.iter().rev().find(|(c, _)| *c == rgb).map(|(_, k)| *k)
    }

    /// Photoshop's colour pair for a layer mask that has none yet: white foreground, black
    /// background (reveal-paint first).
    pub const MASK_COLORS: [[f32; 4]; 2] = [[1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0]];

    /// Note that `brush` or `current_preset` changed. Every setter of either (commands and the
    /// shell's tool-switch paths) calls this; the last-brush persistence only serialises when the
    /// revision has moved since the stored value ([`Self::brush_saved_rev`]).
    pub fn brush_changed(&mut self) {
        self.brush_rev = self.brush_rev.wrapping_add(1);
    }

    /// Switch the edit target between layer pixels and a layer mask. Like Photoshop, the mask and
    /// the pixels keep separate app-wide foreground/background pairs: changing the target puts
    /// the current pair aside and brings back the other one, so colours picked while a mask is
    /// targeted are remembered for masks (#2166). The mask pair starts at [`Self::MASK_COLORS`].
    /// Returns whether the target changed.
    pub fn target_mask(&mut self, mask: bool) -> bool {
        if self.mask_targeted == mask {
            return false;
        }
        self.mask_targeted = mask;
        let [fg, bg] = std::mem::replace(&mut self.other_colors, [self.foreground, self.background]);
        self.foreground = fg;
        self.background = bg;
        true
    }
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
            current_preset: None,
            brush_rev: 0,
            brush_saved_rev: 0,
            mask_targeted: false,
            other_colors: Self::MASK_COLORS,
            cmyk_picks: Vec::new(),
        }
    }
}

/// Layer › Layer Style › Copy Layer Style: the effects and the Blending Options, as Photoshop
/// copies them (Paste Layer Style puts them all on each target).
#[derive(Clone, Debug)]
pub struct CopiedStyle {
    pub effects: photocraft_doc::Effects,
    pub blend: photocraft_color::BlendMode,
    pub fill: f32,
    pub advanced: photocraft_doc::AdvancedBlending,
    pub blend_if: photocraft_doc::BlendIf,
    pub excluded_channels: u32,
    /// The colour mode it was copied in: Blend If's channel ranges and the channel restrictions
    /// only carry over to a document in the same mode.
    pub mode: photocraft_color::ColorMode,
}

/// The commands whose [`Session::precheck`] is complete: it accepts what `execute` does (Free
/// Transform applies its box before these, and only when they will run), but for the group
/// nesting limit ([`photocraft_doc::MAX_GROUP_DEPTH`]): grouping layers already that deep is
/// refused only as the edit is made. Layer via Copy and
/// Cut aren't: whether they have anything to copy (a selection over transparent pixels, a group
/// or fill layer) is known only when they run.
pub const PRECHECKED: &[&str] = &["layer.new.layer", "layer.new.group", "layer.new.groupFromLayers", "layer.new.layerFromBackground"];

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
    native_stroke_work: Option<std::sync::Arc<accounted_native_stroke::NativeStrokeWork>>,
    /// Pixels copied with Edit › Copy / Cut (shared by all documents, like Photoshop).
    pub clipboard: Option<edit_cmds::Clip>,
    /// Layer › Layer Style › Copy Layer Style: the effects and the Blending Options
    /// ([`CopiedStyle`]: blend mode, fill opacity, Advanced Blending, Blend If, channels).
    pub style_clipboard: Option<CopiedStyle>,
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
    /// Color Sampler points dropped by the last canvas-size change (crop, Canvas Size, Image Size,
    /// rotation), for the status message. Read with [`Session::take_samplers_removed`].
    samplers_removed: usize,
    /// The canvas-geometry map the next [`Session::edit`] applies to the Color Sampler points (crop,
    /// Canvas Size, Image Size, rotation set it just before their edit). Consumed by the edit.
    sampler_map: Option<photocraft_geom::Affine>,
    /// Each open document's linked smart objects' files: size and modification time when last
    /// seen ([`link_watch`]).
    pub link_stamps: std::collections::HashMap<(u64, String), link_watch::Stamp>,
    /// The stamps being taken on a worker thread ([`Session::start_link_scan`]).
    pub link_scan: Option<link_watch::LinkScan>,
    /// Select Subject's last result, reused while the pixels are unchanged (#2865).
    subject_cache: smartselect_cmds::SubjectCache,
    /// Object Selection's Object Finder results (#4087), shared by the hover preview and the
    /// click that commits it; valid while the pixels are unchanged.
    pub object_finder: smartselect_cmds::ObjectFinderCache,
    /// A finished live stroke offered to the `paint.stroke` / `paint.pencil` that commits it
    /// ([`Session::offer_live_stroke`]).
    pub(crate) live_commit: Option<brush_cmds::LiveCommit>,
    /// Finished native Luminosity preview, consumed only by its matching command.
    pub(crate) native_replace_commit: Option<brush_cmds::NativeLuminosityCommit>,
    /// Host-bound stored-channel read, valid for one Load Selection dispatch only.
    pub(crate) stored_load_offer: Option<channel_cmds::StoredLoadOffer>,
    /// Documents a background open decoded but held back until the user answers the profile
    /// prompt ([`ColorState::open_question`](color_cmds::ColorState::open_question)): `(id,
    /// document, source read-only, import origin)`. The shell takes them with
    /// [`Session::take_held_open_with_origin`].
    held_opens: Vec<(u64, Document, bool, Option<photocraft_io::ImportOrigin>)>,
    next_held_open: u64,
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
    /// End exactly one completed ordinary resident history group. This changes
    /// only the merge cursor: model, revision, history entries and redo stay exact.
    pub fn finish_history_group(&mut self, document: photocraft_doc::DocId, key: &str) -> Result<bool> {
        if key.is_empty() || key.len() > 4096 || key.contains('\0') {
            return Err(EngineError::Other("invalid history group key".into()));
        }
        if self.has_jobs() || self.command_has_pending_job() || self.coalesce_request.is_some() || self.color_restrict.is_some() {
            return Err(EngineError::Other("finish pending work before ending a history group".into()));
        }
        let state = self.active_mut().ok_or(EngineError::NoDocument)?;
        if state.doc.id != document
            || state.doc.accounted().is_some()
            || state.floating.is_some()
            || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        {
            return Err(EngineError::Other("history group target is not the captured ordinary document".into()));
        }
        match state.coalesce.as_deref() {
            None => Ok(false),
            Some(actual) if actual == key => {
                state.coalesce = None;
                Ok(true)
            }
            Some(_) => Err(EngineError::Other("history group key no longer matches".into())),
        }
    }

    pub fn set_active(&mut self, index: usize) -> bool {
        if index < self.docs.len() {
            self.active = Some(index);
            self.sync_tools_mask_target();
            true
        } else {
            false
        }
    }

    /// Color Sampler points dropped by the last canvas-size change (crop, Canvas Size, Image Size,
    /// rotation) since this was last called, for the status message. See [`edit`](Self::edit).
    pub fn take_samplers_removed(&mut self) -> usize {
        std::mem::take(&mut self.samplers_removed)
    }

    /// Move the Color Sampler points through `a` in the next [`Session::edit`] (crop, Canvas Size,
    /// Image Size, rotation), dropping the ones that land outside the new canvas. View state: the
    /// points stay pinned to their pixels, no history step. Cleared by the edit it is set for.
    pub(crate) fn set_sampler_map(&mut self, a: photocraft_geom::Affine) {
        self.sampler_map = Some(a);
    }

    pub fn sync_tools_mask_target(&mut self) {
        let mask = self.active().is_some_and(|st| st.channel_view.target == crate::channel_cmds::ChannelTarget::LayerMask);
        self.tools.target_mask(mask);
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
                if state.doc.id == doc.id {
                    collision = true;
                    break;
                }
            }
            if !collision {
                break;
            }
            attempts = attempts
                .checked_add(1)
                .filter(|count| *count <= self.docs.len().saturating_add(1))
                .ok_or_else(|| EngineError::Other("document identity allocation exhausted".into()))?;
            doc.id = photocraft_doc::DocId::fresh();
        }
        let mut st = DocState::new_with_interrupt(doc, path, control)?;
        st.history.max_states = self.prefs.get().performance.history_states.max(1) as usize;
        st.history.max_bytes = self.prefs.get().performance.history_budget_bytes();
        let read_control = photocraft_raster::backing::ReadControl { interrupt: control, work: &|_| Ok(()) };
        self.publish_document_state(st, &read_control)
    }

    /// Holds a decoded document back from opening (see `held_opens`); returns its id.
    pub(crate) fn hold_open_import(
        &mut self,
        doc: Document,
        source_read_only: bool,
        origin: photocraft_io::ImportOrigin,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<u64> {
        let next = self.next_held_open.checked_add(1).ok_or_else(|| EngineError::Other("held-open identity exhausted".into()))?;
        let admission = held_import_controlled::preflight(&self.held_opens, self.held_opens.capacity(), &doc, control)?;
        self.held_opens.try_reserve_exact(1).map_err(|_| EngineError::Other("could not reserve held-open storage".into()))?;
        admission.check_capacity(self.held_opens.capacity(), control)?;
        self.held_opens.push((next, doc, source_read_only, Some(origin)));
        self.next_held_open = next;
        Ok(next)
    }

    /// Takes a document a background open held back for the profile prompt: the document and
    /// whether its source is read-only. `None` when there is no such document (already taken).
    pub fn take_held_open(&mut self, id: u64) -> Option<(Document, bool)> {
        self.take_held_open_with_origin(id).map(|(doc, read_only, _)| (doc, read_only))
    }

    /// Borrow a held import for fallible destination admission before transferring ownership.
    pub fn held_open_with_origin(&self, id: u64) -> Option<(&Document, bool, Option<photocraft_io::ImportOrigin>)> {
        let (_, doc, read_only, origin) = self.held_opens.iter().find(|(held, _, _, _)| *held == id)?;
        Some((doc, *read_only, *origin))
    }

    /// Decoded imports that have not transferred to a document or UI profile queue.
    pub fn held_open_count(&self) -> usize {
        self.held_opens.len()
    }

    /// Discard only the private unopened import the user explicitly cancelled.
    pub fn discard_held_open(&mut self, id: u64) -> bool {
        let Some(index) = self.held_opens.iter().position(|(held, _, _, _)| *held == id) else { return false };
        self.held_opens.remove(index);
        true
    }

    /// Cancel private pending imports; existing document tabs/history are untouched.
    pub fn discard_held_opens(&mut self) -> usize {
        let count = self.held_opens.len();
        drop(std::mem::take(&mut self.held_opens));
        count
    }

    /// Import identity accompanies the private document through the profile queue.
    pub fn take_held_open_with_origin(&mut self, id: u64) -> Option<(Document, bool, Option<photocraft_io::ImportOrigin>)> {
        let i = self.held_opens.iter().position(|(h, _, _, _)| *h == id)?;
        let (_, doc, read_only, origin) = self.held_opens.remove(i);
        Some((doc, read_only, origin))
    }

    pub fn close(&mut self, index: usize) -> Option<DocState> {
        if index >= self.docs.len() {
            return None;
        }
        smart_cmds::on_close(self, index);
        if let Some(id) = self.docs.get(index).map(|d| d.doc.id) {
            self.cancel_jobs_on(id);
            self.subject_cache.forget_doc(id);
            self.object_finder.forget_doc(id);
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

    /// Offer a finished live stroke to the next command: a `paint.stroke` / `paint.pencil` whose
    /// params match it exactly takes its pixels instead of rendering the stroke again (its result
    /// then says `"fromLive": true`); any other command drops the offer. The command still runs
    /// through the normal dispatch, so authorization, the journal and history are unchanged.
    pub fn offer_live_stroke(&mut self, live: brush_cmds::LiveStroke) -> Result<()> {
        self.live_commit = Some(live.into_commit()?);
        Ok(())
    }

    /// [`Self::offer_live_stroke`] for a live Clone Stamp or History Brush stroke, taken by the
    /// `paint.cloneStamp` / `paint.historyBrush` it was drawn for. Refused for the Healing Brush
    /// (it blends on release) and when the brush, layer or clone source changed during the stroke.
    pub fn offer_live_retouch(&mut self, live: retouch_cmds::LiveRetouch) -> Result<()> {
        let commit = live.into_commit(self)?;
        self.live_commit = Some(commit);
        Ok(())
    }

    /// Withdraw an offer the command didn't take.
    pub fn clear_live_stroke(&mut self) {
        self.live_commit = None;
    }

    /// Check a call without running it: the command exists, its params have the shape
    /// [`Self::execute`] requires (New Layer's fully), and it can run now. For a caller that does
    /// something first only when the command will go through: Free Transform applies its box
    /// before a new layer is made.
    ///
    /// Complete for the commands in [`PRECHECKED`]: there it accepts what `execute` accepts, but
    /// for the group nesting limit (a test holds the two together). For others it checks only what they share.
    pub fn precheck(&mut self, id: &str, params: &Value) -> Result<()> {
        let spec = crate::commands::find(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        if !(params.is_object() || params.is_null()) {
            return Err(EngineError::BadParams { cmd: id.to_string(), msg: "params must be a JSON object".into() });
        }
        crate::param_check::check(spec, params)?;
        crate::filters::validate_params(id, params)?;
        if id == "layer.new.layer" {
            crate::new_layer::parse(params)?;
        }
        // A command that takes `"layer"` (an id or a name, as it resolves it) fails on one the
        // document doesn't have.
        if spec.params.contains("\"layer\":id")
            && let Some(l) = commands::explicit_layer(self, params)?
            && self.active().is_none_or(|st| st.doc.layer(l).is_none())
        {
            return Err(EngineError::BadParams { cmd: id.to_string(), msg: format!("no layer {}", l.0) });
        }
        self.admit(id, spec, params).map(|_| ())
    }

    /// What `dispatch` checks once the params are valid and a floating selection has dropped:
    /// the precondition (on a named layer for commands that take one), a job running on the
    /// document, a hidden target. The run params (the Channels panel's target filled in) when it
    /// may run.
    pub(crate) fn admit(&mut self, id: &str, spec: &commands::CommandSpec, params: &Value) -> Result<Value> {
        let run_params = channel_cmds::inject_target(self, id, commands::inject_kind(id, params.clone()));
        if let Err(why) = self.explicit_layer_enabled(spec, params).unwrap_or_else(|| self.precondition(spec, &run_params)) {
            return Err(EngineError::Disabled(id.to_string(), why));
        }
        if let Some(why) = self.job_conflict(id, spec.journal) {
            return Err(EngineError::Disabled(id.to_string(), why));
        }
        // Painting on or moving a hidden layer is refused, as in Photoshop (#571).
        if let Some(why) = hidden_target::refusal(self, id, params) {
            return Err(EngineError::Other(why.into()));
        }
        Ok(run_params)
    }

    /// Offer a privately completed native Luminosity preview to the registered
    /// `paint.colorReplacement` command. The command still validates its captured
    /// target, settings and points before publishing one history entry.
    pub fn offer_native_color_replacement(&mut self, commit: brush_cmds::NativeLuminosityCommit) -> Result<()> {
        self.native_replace_commit = None;
        commit.validate_for_session(self)?;
        self.native_replace_commit = Some(commit);
        Ok(())
    }

    /// Withdraw a native preview offer without changing the document or history.
    pub fn clear_native_color_replacement(&mut self) {
        self.native_replace_commit = None;
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
        if self.active().is_some_and(|state| state.doc.accounted().is_some()) && accounted_commands::supports_for_session(self, id) {
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
        if self.names_existing_layer(spec.id, params) {
            return Ok(());
        }
        // `ifPresent`: with no Background there is nothing to do, which isn't an error.
        if spec.id == "layer.new.layerFromBackground" && params.get("ifPresent").and_then(Value::as_bool) == Some(true) {
            return self.active().map(|_| ()).ok_or_else(|| "no document open".into());
        }
        channel_cmds::mask_target_enabled(self, spec.id, params).unwrap_or_else(|| (spec.enabled)(self))
    }

    /// Whether `id` is a layer command whose `"layer"` param names an existing layer, so it needs no
    /// active layer (#2457). Matches how `layer_param` resolves the layer when the command runs.
    fn names_existing_layer(&self, id: &str, params: &Value) -> bool {
        matches!(id, "layer.hideLayers" | "layer.showLayers" | "layer.renameLayer")
            && params.get("layer").and_then(Value::as_u64).is_some_and(|l| self.active().is_some_and(|st| st.doc.layer(LayerId(l)).is_some()))
    }

    /// The precondition of a command whose params document `"layer":id` (it acts on that layer),
    /// when `params` name an existing layer of the active document: checked on that layer rather
    /// than on the active one, which stays active (#466). A script, action or agent can then
    /// Replace Contents, Rasterize or Merge Down a layer without selecting it first. `None` for
    /// other commands, or without such a layer.
    pub(crate) fn explicit_layer_enabled(&mut self, spec: &commands::CommandSpec, params: &Value) -> Option<std::result::Result<(), String>> {
        if !spec.params.contains("\"layer\":id") {
            return None;
        }
        let target = LayerId(params.get("layer")?.as_u64()?);
        let d = self.active_mut()?;
        d.doc.layer(target)?;
        let active = d.active_layer.replace(target);
        let r = (spec.enabled)(self);
        if let Some(d) = self.active_mut() {
            d.active_layer = active;
        }
        Some(r)
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
        self.edit_index_with_interrupt(index, label, control, None, false, f)
    }

    /// Caller-admitted resident history staging for a synchronous transaction.
    /// Document/output backing and the complete history envelope remain held by
    /// the caller through this method. The controlled history is installed only
    /// with the privately validated document at the final cancellation fence.
    pub(crate) fn edit_with_controlled_resident_history_with_interrupt<R>(
        &mut self,
        label: &str,
        limits: photocraft_ops::ResidentHistoryLimits,
        control: &photocraft_raster::Interrupt<'_>,
        f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>,
    ) -> Result<R> {
        if self.coalesce_request.is_some() || self.color_restrict.is_some() {
            return Err(EngineError::Other("finish transient edits before controlled history publication".into()));
        }
        let index = self.active.ok_or(EngineError::NoDocument)?;
        self.edit_index_font_input(index, label, control, None, false, None, Some(limits), f)
    }

    /// Reuses a caller-admitted runtime font through the final edit validator.
    /// The borrowed handle is scoped to this transaction and is never stored in
    /// the session, document, or history. All normal edit fences still apply.
    pub fn edit_with_runtime_font_with_interrupt<R>(
        &mut self,
        label: &str,
        font: &photocraft_text::xcf::runtime_font_input::PreparedRuntimeFont<'_>,
        control: &photocraft_raster::Interrupt<'_>,
        f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>,
    ) -> Result<R> {
        let index = self.active.ok_or(EngineError::NoDocument)?;
        self.edit_index_font_input(index, label, control, None, false, Some(font), None, f)
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
        if target.floating.is_some()
            || target.coalesce.is_some()
            || target.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
            || self.coalesce_request.is_some()
            || self.color_restrict.is_some()
        {
            return Err(EngineError::Other("finish transient edits before editing this document".into()));
        }
        self.edit_index_with_interrupt(index, label, control, None, false, f)
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
        self.edit_index_with_interrupt(index, label, control, Some((target, anchor)), false, f)
    }

    /// Publish a prepared layer target and reset the channel view at the same
    /// document/history fence. A cancelled flatten must retain the old view.
    pub(crate) fn edit_with_layer_target_and_channel_reset_with_interrupt<R>(
        &mut self,
        label: &str,
        control: &photocraft_raster::Interrupt<'_>,
        target: LayerTarget,
        anchor: Option<LayerId>,
        f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>,
    ) -> Result<R> {
        let index = self.active.ok_or(EngineError::NoDocument)?;
        if self.coalesce_request.is_some() || self.color_restrict.is_some() {
            return Err(EngineError::Other("finish transient edits before replacing layer and channel selection".into()));
        }
        self.edit_index_with_interrupt(index, label, control, Some((target, anchor)), true, f)
    }

    fn edit_index_with_interrupt<R>(
        &mut self,
        index: usize,
        label: &str,
        control: &photocraft_raster::Interrupt<'_>,
        target: Option<(LayerTarget, Option<LayerId>)>,
        reset_channel_view: bool,
        f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>,
    ) -> Result<R> {
        self.edit_index_font_input(index, label, control, target, reset_channel_view, None, None, f)
    }

    #[allow(clippy::too_many_arguments)]
    fn edit_index_font_input<R>(
        &mut self,
        index: usize,
        label: &str,
        control: &photocraft_raster::Interrupt<'_>,
        target: Option<(LayerTarget, Option<LayerId>)>,
        reset_channel_view: bool,
        font: Option<&photocraft_text::xcf::runtime_font_input::PreparedRuntimeFont<'_>>,
        resident_history: Option<photocraft_ops::ResidentHistoryLimits>,
        f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>,
    ) -> Result<R> {
        let _authorization = crate::jobs::render_authorization_scope(self.authorize)?;
        control.check().map_err(|_| EngineError::Cancelled)?;
        let restrict = self.color_restrict;
        let sampler_map = if self.active == Some(index) { self.sampler_map.take() } else { None };
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
        match font {
            Some(font) => edit_validation::validate_with_authorized_font(&before, &mut doc, active, restrict, font, control)?,
            None => edit_validation::validate(&before, &mut doc, active, restrict, control)?,
        }
        let snapshot = DocumentSnapshot::from_resident(Arc::new(doc))?;
        fix_selection_parts(&snapshot, active, &mut selected, &mut anchor);
        let layers = LayerTarget { active, selected: selected.clone() };
        // Prepare view state before history/publication; a cancelled geometry
        // edit must not move or drop sampler points in the live document.
        if st.color_samplers.len() > 10_000 {
            return Err(EngineError::Other("color sampler count exceeds edit limit".into()));
        }
        let mut color_samplers = Vec::new();
        color_samplers.try_reserve_exact(st.color_samplers.len()).map_err(|_| EngineError::Other("color sampler preparation allocation".into()))?;
        for point in &st.color_samplers {
            control.check().map_err(|_| EngineError::Cancelled)?;
            let [x, y] = *point;
            if !x.is_finite() || !y.is_finite() {
                return Err(EngineError::Other("invalid color sampler coordinates".into()));
            }
            let transformed = sampler_map.map_or(photocraft_geom::Point::new(x, y), |map| map.apply(photocraft_geom::Point::new(x, y)));
            if !transformed.x.is_finite() || !transformed.y.is_finite() {
                return Err(EngineError::Other("invalid transformed color sampler".into()));
            }
            let inside =
                transformed.x >= 0. && transformed.y >= 0. && transformed.x < f64::from(snapshot.size.width) && transformed.y < f64::from(snapshot.size.height);
            if (sampler_map.is_none() && snapshot.size == before.size) || inside {
                color_samplers.push([transformed.x, transformed.y]);
            }
        }
        let samplers_removed = st.color_samplers.len().saturating_sub(color_samplers.len());
        let damage = layer_multi_cmds::step_damage(&before, &snapshot);
        let key = self.coalesce_request.clone();
        let st = self.docs.get_mut(index).ok_or(EngineError::NoDocument)?;
        let history_control = photocraft_raster::backing::ReadControl { interrupt: control, work: &|_| Ok(()) };
        let history = if let Some(limits) = resident_history {
            if key.is_some() || st.coalesce.is_some() {
                return Err(EngineError::Other("controlled history cannot coalesce".into()));
            }
            st.history.try_clone_record_trim_with_interrupt(before, &snapshot, label, &prior, &layers, limits, control).map_err(|error| {
                if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(format!("controlled resident history: {error}")) }
            })?
        } else {
            let mut history = st.history.clone();
            if key.is_none() || st.coalesce != key || !history.can_undo() {
                history.set_current_layers(prior, &history_control)?;
                history.record(label, before, layers, &history_control)?;
                history.trim(&snapshot, &history_control)?;
            } else {
                history.set_current_layers(layers, &history_control)?;
            }
            history
        };
        control.check().map_err(|_| EngineError::Cancelled)?;
        st.doc = snapshot;
        st.history = history;
        st.active_layer = active;
        st.selected_layers = selected;
        st.layer_anchor = anchor;
        if reset_channel_view {
            st.channel_view = channel_cmds::ChannelView::default();
        }
        channel_cmds::fix_view(st);
        st.coalesce = key;
        st.revision = revision;
        st.last_damage = damage;
        st.color_samplers = color_samplers;
        self.samplers_removed = self.samplers_removed.saturating_add(samplers_removed);
        if damage.is_none_or(|damage| !damage.is_empty()) {
            self.subject_cache = Default::default();
            self.object_finder = Default::default();
        }
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
        channel_cmds::fix_view(st);
        // Selecting a layer is not an edit: keep a clean document clean.
        let clean = st.saved_revision == st.revision;
        st.revision = revision;
        if clean {
            st.saved_revision = st.revision;
        }
        st.last_damage = Some(photocraft_geom::Rect::EMPTY);
        Ok(())
    }

    /// Re-renders the pixels of type layers whose font arrived after they were drawn (the web
    /// build fetches served fonts on demand, `photocraft_text::served`). The layers' model is
    /// unchanged, so this is no history step and leaves a clean document clean. Unknown documents
    /// and layers are skipped. A document a background job is computing from must not move under
    /// it: its layers are returned, to try again later.
    pub fn refresh_type_layers(&mut self, layers: &[(photocraft_doc::DocId, LayerId)]) -> Vec<(photocraft_doc::DocId, LayerId)> {
        let mut busy = Vec::new();
        let mut docs: Vec<photocraft_doc::DocId> = Vec::new();
        for (d, _) in layers {
            if !docs.contains(d) {
                docs.push(*d);
            }
        }
        for doc_id in docs {
            let mine = layers.iter().filter(|(d, _)| *d == doc_id);
            if self.job_on(doc_id).is_some() {
                busy.extend(mine);
                continue;
            }
            let Some(st) = self.docs.iter_mut().find(|st| st.doc.id == doc_id) else { continue };
            if st.doc.accounted().is_some() || st.revision == u64::MAX || st.doc.validate_resident().is_err() {
                busy.extend(mine);
                continue;
            }
            let snapshot = st.doc.clone();
            let mut doc = (*snapshot).clone();
            let mut changed = false;
            let mut failed = false;
            for (_, id) in mine {
                if let Some(photocraft_doc::Layer { content: photocraft_doc::LayerContent::Text(t), .. }) = doc.layer_mut(*id) {
                    if type_cmds::try_refresh(&snapshot, t).is_ok() {
                        changed = true;
                    } else {
                        failed = true;
                    }
                }
            }
            if failed {
                busy.extend(layers.iter().filter(|(d, _)| *d == doc_id));
                continue;
            }
            if changed {
                let Ok(prepared) = DocumentSnapshot::from_resident(Arc::new(doc)) else {
                    busy.extend(layers.iter().filter(|(d, _)| *d == doc_id));
                    continue;
                };
                let clean = st.saved_revision == st.revision;
                st.doc = prepared;
                st.revision += 1;
                if clean {
                    st.saved_revision = st.revision;
                }
                st.last_damage = None;
            }
        }
        busy
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
            st._accounted_metadata_gesture = None;
            return Ok(false);
        };
        let (mut snapshot, target) = change.into_parts();
        let revision = st.revision.checked_add(1).ok_or(photocraft_raster::backing::BackingError::Length)?;
        if let Some(owner) = snapshot.accounted()
            && snapshot.name != st.doc.name
        {
            // File identity is outside editing history, as on the resident route.
            // Prepare its paid copy in the private travelled history before any
            // Session publication, preserving the saved title across undo/redo.
            let named = owner.try_with_document_name(&st.doc.name, control)?;
            snapshot = history.try_replace_accounted_current(owner, named, control)?;
        }
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
            st._accounted_metadata_gesture = None;
            return Ok(true);
        }
        // Save As updates the document title outside history. Upstream preserves
        // that title when travelling to an older editing state.
        if snapshot.name != st.doc.name {
            let limits = photocraft_io::NativeSaveLimits::default().preparation;
            photocraft_io::retained_native_model_bytes(&snapshot, limits, control.interrupt)
                .map_err(|error| if control.interrupt.cancelled() { EngineError::Cancelled } else { EngineError::Other(error.to_string()) })?;
            let mut name = String::new();
            name.try_reserve_exact(st.doc.name.len()).map_err(|_| EngineError::Other("history title allocation failed".into()))?;
            name.push_str(&st.doc.name);
            snapshot.try_resident_mut_with_raster_limit(photocraft_raster::MAX_CHECKED_RASTER_BYTES, control.interrupt)?.name = name;
        }
        let (mut values, lease) = target.into_parts();
        if lease.is_some() {
            return Err(photocraft_raster::backing::BackingError::Integrity.into());
        }
        let deselected = values.active.is_none() && values.selected.is_empty();
        if !deselected && values.active.is_none_or(|id| snapshot.layer(id).is_none()) {
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
        st._accounted_metadata_gesture = None;
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
mod fill_layer_mode_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "Synthetic lifecycle assertions are test-only")]
mod history_group_lifecycle_tests;
#[cfg(test)]
mod pattern_mode_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod fx_mode_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_normal_hierarchy_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod normal_hierarchy_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_normal_hierarchy_native_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod normal_hierarchy_native_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_masked_normal_hierarchy_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod accounted_masked_normal_hierarchy_tests;

mod accounted_native_stroke;
mod accounted_programme_current;
pub use accounted_native_stroke::{AccountedNativeStrokeTarget, candidate as accounted_native_stroke_candidate, capture_accounted_native_stroke_target};

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_plain_integer_hierarchy_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod plain_integer_hierarchy_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_plain_quantized_normal_hierarchy_native_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod plain_quantized_normal_hierarchy_native_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_wide_plain_xcf_source_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod wide_plain_xcf_source_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_native_wide_stroke_native_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod native_wide_stroke_native_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_u32_plain_hierarchy_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod u32_plain_hierarchy_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_u32_normal_hierarchy_native_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod u32_normal_hierarchy_native_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_plain_hierarchy_paint_source_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod plain_hierarchy_paint_source_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_hierarchy_native_stroke_native_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod hierarchy_native_stroke_native_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_programme_paint_source_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod programme_paint_source_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_programme_stroke_native_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod programme_stroke_native_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_programme_select_native_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod programme_select_native_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_common_edit_native_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod common_edit_native_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_programme_selection_native_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod programme_selection_native_tests;

mod accounted_select_inverse;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "accounted_inverse_fractional_clear_native_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod inverse_fractional_clear_native_tests;
