//! Photocraft's first UI shell, built on egui/eframe.
//!
//! This crate is deliberately thin. Every action goes through
//! [`photocraft_engine::Session::execute`], and all UI state lives in [`state::UiState`] (plain
//! data). The [`control`] module exposes both to automation, so agents can drive and inspect every
//! part of the interface.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// Translate a string literal into the current UI language: `tl!("Image Size…")`.
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}

pub mod actions;
pub mod adjust_dialog;
pub mod adjust_editors;
pub mod adjust_preview;
pub mod adjust_ui;
pub mod analysis_ui;
pub mod artboard_ui;
pub(crate) mod blend_preview;
mod brand;
pub mod brush_panel;
pub mod brush_picker;
pub mod brush_preview;
pub mod brush_resize;
pub mod brush_sections;
pub mod brushes_tab;
mod camera_raw_detail_ui;
mod camera_raw_preview_ui;
mod camera_raw_scope_ui;
pub mod camera_raw_ui;
pub mod canvas;
pub mod canvas_tool_menu;
pub mod channel_view;
pub mod channels_panel;
pub mod chrome_ui;
pub mod cjk_fonts;
pub mod clip_line_ui;
pub mod color_picker_ui;
pub mod color_range_ui;
pub mod comps_ui;
pub mod control;
pub mod credits;
pub mod crop_ui;
pub mod dialogs;
pub mod direct_select;
pub mod discard_ui;
pub mod distort_ui;
pub mod doc_props_ui;
pub mod dock;
pub mod enable_rules;
pub mod eraser_ui;
pub mod export_dialog;
pub mod file_dialog;
pub mod file_open;
pub mod file_ui;
pub mod fill_ui;
pub mod filter_dialog;
pub mod gallery_ui;
pub mod gpu_canvas;
pub mod gpu_status;
pub mod gradient_ui;
pub mod hold_keys;
pub mod i18n;
mod icon_data;
pub mod icons;
mod import_assessment_ui;
mod xcf_notices;
mod vector_path_rows_cache;
pub mod jobs_ui;
pub mod lasso_ui;
pub mod layer_menu_ui;
pub mod layer_pick_ui;
pub mod layer_props_ui;
mod layer_reveal;
pub mod layer_row_ui;
pub mod layer_style;
mod layer_transfer;
pub mod layer_tree_ui;
pub mod links;
pub mod liquify_ui;
pub mod magnetic_lasso_ui;
pub mod mask_thumbs_ui;
pub mod menu_catalog;
pub mod menu_nav;
pub mod menus;
pub mod monitor_status;
pub mod move_mods;
pub mod move_ui;
pub mod native_menu;
pub mod new_doc_ui;
pub mod notices;
mod opacity_keys;
pub mod outline;
pub mod paint_mouse;
pub mod palette;
pub mod panels;
pub mod parity;
pub mod patch_preview;
pub mod perspective_ui;
pub mod plugin_ui;
pub mod point_curve;
pub mod prefs_ui;
pub mod preset_files_ui;
pub mod preset_panels;
pub mod props_layout;
pub mod proxy;
pub mod puppet_ui;
pub mod quick_pick;
pub mod rasterize_prompt;
pub mod retouch_ui;
mod rgb_histogram;
pub mod rotate_view;
pub mod rulers;
pub mod screen_picker;
pub mod scrollbars;
pub mod services;
pub mod shortcut_dispatch;
pub mod shortcuts;
mod sizing;
mod trim;
mod duplicate;
mod profile_dialog;
mod mode_conversion_dialog;
mod channel_action_dialog;
mod channel_create_dialog;
mod guide_dialog;
mod path_selection_dialog;
mod path_transform_dialog;
mod path_fill_dialog;
mod path_clipping_dialog;
mod path_stroke_dialog;
pub mod slice_ui;
pub mod smart_ui;
pub mod snap_ui;
pub mod state;
pub mod stroke_constraint;
pub mod stroke_trail;
pub mod stylus;
pub mod swatches_ui;
mod tab_strip;
pub mod theme;
pub mod tiff_options_ui;
mod timeline_ui;
mod titlebar;
pub mod tone;
mod tool_cursor;
pub mod tool_feedback;
pub mod transform_tex;
pub mod transform_tool;
pub mod type_panels_ui;
pub mod type_tool;
mod type_transform;
mod variables_ui;
pub mod vector_ui;
pub mod view_cmds;
pub mod web_ui_assets;
pub mod wheel_nav;
pub mod wide_angle_ui;
pub mod widgets;
pub mod work_area;
pub mod workspace_ui;
mod xcf_labels;
mod xcf_panels;
pub mod zoom_tool;

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

use photocraft_doc::{DocId, Document};
use photocraft_engine::Session;
use serde_json::Value;

pub use control::{ControlRequest, ControlResponse};
pub use file_dialog::{FileDialogAnswer, FileDialogFn, FileDialogReply, FileDialogRequest};
pub use file_open::OsEvent;
pub use state::{Tool, UiState};

/// Decode a file, retaining its detected origin and warnings about approximations or drops.
pub type ImportFn = Box<dyn Fn(&str, &[u8]) -> Result<photocraft_io::ImportResult, String>>;
/// Encoder settings chosen in Export As (the file format comes from the name's extension).
#[derive(Clone, Debug, PartialEq)]
pub struct ExportSettings {
    /// JPEG quality 1–100 (None = codec default).
    pub jpeg_quality: Option<u8>,
    /// WebP: lossless (VP8L) rather than lossy (VP8 at [`Self::webp_quality`]).
    pub webp_lossless: bool,
    /// Lossy WebP quality 1–100 (None = codec default).
    pub webp_quality: Option<u8>,
    /// TIFF: keep the layers (Photoshop layer data); `false` is "Discard Layers and Save a Copy".
    pub tiff_layers: bool,
    /// Embed the document's whole XMP packet. `true` by default (Save As keeps the metadata);
    /// Export As starts at Metadata: None, since the packet can carry the text of every type
    /// layer and one id per placed document (#647).
    pub xmp_all: bool,
}

impl Default for ExportSettings {
    fn default() -> Self {
        ExportSettings { jpeg_quality: None, webp_lossless: true, webp_quality: None, tiff_layers: true, xmp_all: true }
    }
}

/// Encode a document: (file bytes, warnings about anything approximated or dropped).
pub type ExportFn = Box<dyn Fn(&Document, &str, &ExportSettings) -> Result<(Vec<u8>, Vec<String>), String>>;
pub type WriteFn = Box<dyn FnMut(&str, &[u8]) -> Result<(), String>>;
/// Validate a destination within the same authority as its eventual write, before encoding.
pub type ValidateWriteFn = Box<dyn Fn(&str, Option<&photocraft_engine::SourceIdentity>) -> Result<(), String>>;
/// Read bytes through the desktop control session's authorized read root.
pub type AutomationReadFn = Box<dyn FnMut(&str) -> Result<(String, Vec<u8>), String>>;
/// Write bytes through the desktop control session's authorized write root.
pub type AutomationWriteFn = Box<dyn FnMut(&str, &[u8], &photocraft_engine::WriteProtection) -> Result<(), String>>;
/// Capability writer validation carries persisted fingerprints as exclusions only.
pub type AutomationValidateWriteFn = Box<dyn Fn(&str, &photocraft_engine::WriteProtection) -> Result<(), String>>;
/// Reject engine commands which still perform ambient filesystem I/O.
pub type AutomationCommandFn = Box<dyn Fn(&str, &Value) -> Result<(), String>>;
/// Open a URL in the system browser (native) — reliable cross-platform, unlike `ctx.open_url`.
pub type OpenUrlFn = Box<dyn Fn(&str) -> Result<(), String>>;
pub type EncodePngFn = Box<dyn Fn(u32, u32, &[u8]) -> Result<Vec<u8>, String>>;
/// Put an RGBA8 image (width, height, pixels) on the OS clipboard.
pub type ClipboardSetFn = Box<dyn FnMut(u32, u32, &[u8]) -> Result<(), String>>;
/// Read an RGBA8 image from the OS clipboard.
pub type ClipboardGetFn = Box<dyn FnMut() -> Option<(u32, u32, Vec<u8>)>>;
/// Shared queue of (file name, bytes) delivered asynchronously.
pub type Inbox = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;
/// Read the saved preferences text (`None` when there is none yet).
pub type LoadTextFn = Box<dyn FnMut() -> Option<String>>;
/// Persist the preferences text.
pub type SaveTextFn = Box<dyn FnMut(&str) -> Result<(), String>>;
/// Autosave a document snapshot for crash recovery: (snapshot, revision, original path).
pub type AutosaveFn = Box<dyn FnMut(&photocraft_engine::document_snapshot::DocumentSnapshot, u64, Option<&str>) -> Result<(), String>>;
/// Poll successful or failed background writes: (document id, revision, result).
pub type AutosaveResultsFn = Box<dyn FnMut() -> Vec<(u64, u64, Result<(), String>)>>;
/// Drop the recovery data of a document (by `DocId` value) once it is saved or closed.
pub type DiscardAutosaveFn = Box<dyn FnMut(u64)>;
/// Load recoverable documents left by a previous session. Their recovery data stays until the
/// documents are saved or closed.
pub type RecoverFn = Box<dyn FnMut() -> Vec<Recovered>>;
/// A recovered document (by `DocId` value, once open) takes over its recovery entry (by key):
/// its autosaves replace the entry, and saving or closing it drops the entry.
pub type AdoptAutosaveFn = Box<dyn FnMut(u64, &str)>;

/// A document [`RecoverFn`] found.
pub struct Recovered {
    /// The recovery entry it was loaded from (see [`AdoptAutosaveFn`]).
    pub key: String,
    /// Where the user last saved it, if anywhere.
    pub path: Option<String>,
    pub doc: Document,
}
/// Append text to a file (History Log).
pub type AppendTextFn = Box<dyn FnMut(&str, &str) -> Result<(), String>>;
/// Requests from the operating system since the last call (see [`OsEvent`]).
pub type OsEventsFn = Box<dyn FnMut() -> Vec<OsEvent>>;
/// Ends the app the platform's own way (see [`Services::quit`]).
pub type QuitFn = Box<dyn FnMut()>;
/// Where the OS pointer is now, in egui points within the window; `None` when unknown.
pub type CursorPosFn = Box<dyn FnMut(&egui::Context) -> Option<egui::Pos2>>;

/// Platform services injected by the app binary (file dialogs, codecs), keeping this crate free of
/// I/O dependencies.
#[derive(Default)]
pub struct Services {
    /// User-initiated desktop/browser pixel sampling.
    pub screen_pick: Option<screen_picker::Service>,
    /// Decode a file's bytes into a document (PSD, PNG, JPEG, …).
    pub import: Option<ImportFn>,
    /// Encode a document for a file name (format chosen by extension).
    pub export: Option<ExportFn>,
    /// Atomic streamed native saves with all open-document source exclusions.
    pub native_save: Option<services::NativeSaveFn>,
    /// Show an Open or Save dialog without waiting for it (see `file_dialog`).
    pub file_dialog: Option<FileDialogFn>,
    /// Write bytes to a path (native) or trigger a download (web).
    pub write: Option<WriteFn>,
    pub validate_write: Option<ValidateWriteFn>,
    /// Native interactive saves retain inert source exclusions through commit.
    /// Browser downloads keep using `write`, which cannot replace source files.
    pub write_protected: Option<AutomationWriteFn>,
    pub validate_write_protected: Option<AutomationValidateWriteFn>,
    /// File access used only by control/MCP requests. Interactive dialogs keep
    /// using `file_dialog` and `write` with the user's authority.
    pub automation_read: Option<AutomationReadFn>,
    /// Capped asynchronous linked-image read beneath the held automation root.
    /// Stored document locators never supply authority to this service.
    pub automation_read_xcf_linked_replacement: Option<file_dialog::AutomationReadXcfLinkedReplacementFn>,
    pub automation_write: Option<AutomationWriteFn>,
    /// Stream native output beneath the held automation write root only.
    pub automation_native_save: Option<services::NativeSaveFn>,
    pub automation_validate_write: Option<AutomationValidateWriteFn>,
    pub automation_command: Option<AutomationCommandFn>,
    /// Same policy as [`Self::automation_command`], as a function pointer the engine calls for
    /// each step of `actions.play`. Installed on the session only while a control request or
    /// an automation-driven [`PhotocraftApp::run`] runs, so a local play of a recorded `file.*` step still works.
    pub automation_authorize: Option<fn(&str, &serde_json::Value) -> photocraft_engine::Result<()>>,
    /// Encode an RGBA8 image as PNG (used for screenshots and `ui.render`).
    pub encode_png: Option<EncodePngFn>,
    /// Open a URL in the system browser (native). Falls back to `ctx.open_url` (web) when unset.
    pub open_url: Option<OpenUrlFn>,
    /// Files delivered asynchronously (web drag-and-drop): drained every frame.
    pub inbox: Option<Inbox>,
    /// OS clipboard images: copies go out, screenshots and images from other apps come in.
    pub clipboard_set_image: Option<ClipboardSetFn>,
    pub clipboard_get_image: Option<ClipboardGetFn>,
    /// Preferences persistence: the platform config directory on native, browser storage on
    /// the web (see `prefs_ui`).
    pub load_prefs: Option<LoadTextFn>,
    pub save_prefs: Option<SaveTextFn>,
    /// The native window is connected directly to a Wayland compositor.
    pub is_wayland: bool,
    /// On Wayland, the shell command that starts this install under XWayland, where native file
    /// drops work (#386); `None` when there is no X server to run it on.
    pub xwayland_command: Option<String>,
    /// Crash-recovery autosave (Preferences › File Handling) and recovery at launch.
    pub autosave: Option<AutosaveFn>,
    /// None for services that report synchronous success through `autosave`.
    pub autosave_results: Option<AutosaveResultsFn>,
    pub discard_autosave: Option<DiscardAutosaveFn>,
    pub recover: Option<RecoverFn>,
    pub adopt_autosave: Option<AdoptAutosaveFn>,
    /// History Log text file output.
    pub append_text: Option<AppendTextFn>,
    /// OS requests (macOS open-documents / quit Apple events), polled every frame.
    pub os_events: Option<OsEventsFn>,
    /// The pointer position read from the OS (desktop): winit 0.30's file drops carry none, and
    /// the window gets no pointer events during an OS drag (see `file_open::DropTarget`).
    pub cursor_pos: Option<CursorPosFn>,
    /// The persistent brush preset store, loading in the background (desktop; see
    /// `photocraft_engine::preset_store`). Attached to the session once it arrives; without
    /// one, brush presets are session-only (web, tests).
    pub preset_store: Option<std::sync::mpsc::Receiver<photocraft_engine::preset_store::Opened>>,
    /// Reads the displays and their ICC profiles in the background (desktop macOS; see
    /// `monitor_status`). Without one, the canvas uses the profile chosen in Color Settings, or sRGB.
    pub read_displays: Option<monitor_status::ReadDisplaysFn>,
    /// The macOS menu bar, when the desktop app installed one; the in-window menus are hidden then.
    pub native_menu: Option<native_menu::NativeMenu>,
    /// Ends the app once leaving is settled (nothing unsaved, or the prompt answered), instead of
    /// letting eframe close the window. macOS: `-[NSApplication terminate:]`. Closing the window
    /// while AppKit's run loop is still going crashes or hangs Touch Bar Macs (#1575, #1458).
    /// Without one, eframe closes the window and the app ends with it.
    pub quit: Option<QuitFn>,
}

/// A document histogram being computed off the UI thread: (document, revision, receiver of
/// (document, compute ms, histograms)).
pub(crate) type HistogramDelivery = (DocId, f64, Result<std::sync::Arc<tone::Histograms>, String>);
pub(crate) type HistJob = (DocId, u64, std::sync::mpsc::Receiver<HistogramDelivery>);

pub struct PhotocraftApp {
    #[cfg(all(target_arch = "wasm32", feature = "web_external_assets"))]
    web_ui_assets: web_ui_assets::ReadyUiAssets,
    pub session: Session,
    pub ui: UiState,
    pub services: Services,
    /// Canvas caches per (document, display): CPU textures hold monitor values; the GPU
    /// canvas state is shared (`canvas::GPU_OUTPUT`).
    canvases: HashMap<(DocId, u32), canvas::CanvasCache>,
    /// Navigator textures belong to open documents, not the lifetime of the egui context.
    navigator_textures: HashMap<DocId, canvas::NavigatorCache>,
    /// Display profile readings (#569).
    monitors: monitor_status::State,
    checker: Option<egui::TextureHandle>,
    drag: Option<canvas::Drag>,
    /// Brush/Eraser stroke being drawn, rendered by the engine (see `canvas::LiveStroke`).
    live_stroke: Option<canvas::LiveStroke>,
    /// Footprint trail of a retouching drag (see `stroke_trail`).
    trail: Option<stroke_trail::Trail>,
    /// Move tool drag shown live (`move_ui`).
    pub(crate) move_preview: Option<move_ui::MovePreview>,
    /// A blend mode hovered in the Layers panel, shown live (`blend_preview`).
    pub(crate) blend_preview: Option<blend_preview::BlendPreview>,
    /// Patch Tool drag: the healed document at the pointer (`patch_preview`).
    pub(crate) patch_preview: Option<patch_preview::PatchPreview>,
    /// The pixels a Magnetic Lasso border follows (`magnetic_lasso_ui`).
    pub(crate) magnetic: magnetic_lasso_ui::Runtime,
    /// The next tool `Down` is a right-button drag that erases (see `paint_mouse`).
    secondary_erase: bool,
    /// While a batch of recovered pointer samples is replayed, defer the live-stroke update to one
    /// call for the whole frame (see `canvas::canvas_view`).
    defer_live_stroke: bool,
    /// A live painting stroke started on the press (`canvas_view`): the drag egui recognises later,
    /// or the click, continues or ends it rather than starting another.
    press_stroke: bool,
    /// End of the last painting stroke: ⇧-click draws a straight line from it (#178).
    last_stroke_end: Option<(DocId, [f64; 2])>,
    /// Control+Alt-drag brush resize in progress (`brush_resize`, #231).
    pub(crate) brush_resize: Option<brush_resize::Resize>,
    /// A ⌘⌥⌃-click layer pick is in progress; its drag and release are swallowed (`quick_pick`).
    pub(crate) quick_pick: bool,
    /// The next tool `Down` is an Alt+right-drag that resizes the brush (#297). `tool_event`
    /// takes it on every event, so a press another handler consumes can't leave it set.
    pub(crate) brush_resize_armed: bool,
    /// This press began with ⌥ (Alt) held on a painting tool, so it samples colours instead of
    /// painting until it is released (`canvas::alt_eyedropper`, #417).
    pub(crate) alt_sampling: bool,
    /// The first digit of a two-digit opacity typed on the number keys (`opacity_keys`, #352).
    pub(crate) opacity_keys: opacity_keys::Pending,
    control_rx: Option<Receiver<ControlRequest>>,
    pending_screenshots: Vec<(u64, Option<String>, Sender<ControlResponse>)>,
    /// Screenshots not yet requested from the viewport: (token, earliest time in ms, frames seen).
    queued_screenshots: Vec<(u64, f64, u32)>,
    /// Control replies waiting for queued synthetic input to be processed.
    input_waiters: Vec<Sender<ControlResponse>>,
    /// Live (uncommitted) adjustment edit shown on canvas while a slider is dragged.
    pub live_adjust: Option<(photocraft_doc::LayerId, Value)>,
    /// Frames rendered (for tests and the status bar).
    pub frame: u64,
    /// Apply theme on first frame.
    styled: bool,
    /// Whether the window uses an integrated (transparent) macOS title bar.
    pub integrated_titlebar: bool,
    /// Windows and Linux: the window has no OS decorations and the app's top bar is the title bar
    /// (caption buttons, window dragging and edge resizing, `titlebar`).
    pub custom_titlebar: bool,
    fonts_ready: bool,
    /// Screen rect of the main canvas last frame (for overlays and the navigator).
    pub last_canvas_rect: egui::Rect,
    /// The document area showing the active document's canvas last frame (not the tabs, the
    /// start screen or an opening file's card): files dropped here are placed as layers.
    pub(crate) drop_canvas_rect: Option<egui::Rect>,
    /// The document tab strip last frame: a drop there opens the file at the slot under it.
    pub(crate) tab_strip: Option<canvas::TabStrip>,
    /// Files dropped on the canvas still to place, one Free Transform at a time.
    pub(crate) drop_places: std::collections::VecDeque<egui::DroppedFileHandle>,
    /// The document each of `ui.views` belongs to, as of the last [`Self::sync_views`].
    view_docs: Vec<DocId>,
    pub fps: f32,
    last_frame_time: f64,
    thumbs: HashMap<(photocraft_doc::LayerId, u8), (u64, egui::TextureHandle)>,
    /// Snapshots whose live layer/mask keys were last used to prune thumbnail handles.
    thumb_documents: Vec<(DocId, photocraft_engine::document_snapshot::WeakDocumentSnapshot)>,
    /// Content bounds cached per (key, revision): scanning a 36 MP layer every frame cost ~77 ms.
    bounds_cache: HashMap<u64, (u64, photocraft_geom::Rect)>,
    /// Checked editable-source layout, including failed qualification, per revision.
    source_bounds_cache: HashMap<(u64, u64, u64), Option<photocraft_geom::Rect>>,
    /// Downsampled proxy of the active document for live previews: (doc, revision, k, proxy).
    pub(crate) proxy: Option<(DocId, u64, u32, std::sync::Arc<Document>)>,
    /// Key of the preview currently uploaded to the GPU (doc, params hash).
    pub(crate) proxy_uploaded: Option<(DocId, u64)>,
    /// Selection outline cache: (doc, revision, segments).
    /// Live filter preview (proxy document with the filter applied).
    pub(crate) filter_preview: Option<filter_dialog::FilterPreview>,
    /// Select › Color Range dialog preview (proxy document + mask / image textures).
    pub(crate) color_range: Option<color_range_ui::Preview>,
    /// Image › Adjustments dialog preview through a temporary clipped adjustment layer.
    pub(crate) adjust_preview: Option<adjust_preview::AdjustPreview>,
    /// Synthetic input events queued by automation (`ui.click`, `ui.key`, …), injected next frame.
    pub(crate) synthetic: Vec<egui::Event>,
    /// True only while a frame is processing synthetic automation input. It
    /// makes shortcut- and dialog-triggered commands pass the same policy as
    /// direct control calls.
    pub(crate) automation_input: bool,
    /// Levels/Curves histogram cache: (document, adjustment layer, revision it is valid for).
    pub(crate) tone_hist: Option<(DocId, photocraft_doc::LayerId, u64, std::sync::Arc<tone::Histograms>)>,
    /// Histogram panel cache: (document, revision, computed at ms, histograms).
    pub(crate) doc_hist: Option<(DocId, u64, f64, std::sync::Arc<tone::Histograms>)>,
    /// The document histogram being computed on a worker thread: (document, revision, receiver).
    pub(crate) hist_job: Option<HistJob>,
    /// Free Transform preview (document without the moving pixels + their texture).
    pub(crate) transform_preview: Option<transform_tool::TransformPreview>,
    /// Move-tool ⇧/⌥ drag state (move_mods).
    pub(crate) move_mods: move_mods::MoveDrag,
    /// Live Layer Style dialog preview: (key over revision + style fields, preview or validation error).
    pub(crate) style_preview: Option<(u64, Result<std::sync::Arc<Document>, String>)>,
    /// Liquify dialog, Puppet Warp and Perspective Warp sessions (distort_ui).
    pub(crate) distort: distort_ui::Distort,
    /// Gradient tool live-mode drags and previews (gradient_ui).
    pub(crate) gradient: gradient_ui::LiveGradient,
    /// Filter › Camera Raw Filter dialog (camera_raw_ui).
    pub(crate) camera_raw: Option<camera_raw_ui::CameraRawDialog>,
    /// A raw file just opened interactively, waiting for the open-time Camera Raw dialog (shown
    /// on the next frame, which has the egui context).
    pub(crate) pending_raw_open: Option<camera_raw_ui::RawOpen>,
    /// The open-time re-develop (or its Camera Raw step) running in the background.
    pub(crate) raw_redevelop: Option<camera_raw_ui::Redevelop>,
    /// Filter › Adaptive Wide Angle dialog (wide_angle_ui).
    pub(crate) wide_angle: Option<wide_angle_ui::WideAngleDialog>,
    /// Signature of the image we last put on the OS clipboard (to tell ours from other apps').
    os_clip_sig: Option<u64>,
    /// The session clipboard currently holds an image imported from the OS clipboard: it has no
    /// original position, so Paste centres it (see `menus` "edit.paste").
    pub(crate) clip_external: bool,
    /// The OS clipboard was already read for the paste in flight (Edit › Paste positions it first),
    /// so `run` doesn't read it a second time.
    pub(crate) clip_read_for_paste: bool,
    /// Pointer position over the canvas (document px), for the Info panel and status bar.
    pub(crate) hover_doc: Option<[f64; 2]>,
    pub(crate) clone_preview: Option<crate::canvas::ClonePreviewCache>,
    /// Info panel sample cache: ((x, y, revision), composite RGBA).
    info_sample: Option<((i32, i32, u64), [f32; 4])>,
    /// Guide being dragged (from a ruler or with the Move tool).
    pub(crate) guide_drag: Option<rulers::GuideDrag>,
    /// Crop tool gesture in progress (see `crop_ui`).
    pub(crate) crop: crop_ui::CropState,
    /// Type tool layout cache: ((doc, revision, layer), layout).
    pub(crate) type_layout: Option<((u64, u64, u64), std::sync::Arc<photocraft_text::TextLayout>)>,
    pub(crate) type_transform_preview: Option<type_transform::Preview>,
    /// Channel thumbnails for one document snapshot; view-only revisions reuse their pixels.
    channel_thumbs: Option<(DocId, photocraft_engine::document_snapshot::WeakDocumentSnapshot, Vec<egui::TextureHandle>)>,
    /// Channels panel overlays / channel views drawn over the canvas, per document id.
    pub(crate) channel_views: HashMap<u64, channel_view::Cache>,
    /// Selection outline keyed by (document, mask identity × step × visible region).
    pub(crate) outline_cache: Option<(DocId, u64, std::sync::Arc<Vec<outline::Segment>>)>,
    /// GPU canvas renderer, when running on the wgpu backend (see [`Self::set_wgpu`]). Dropped
    /// for the rest of the session when the device is lost (see `gpu_status`).
    gpu: Option<gpu_canvas::GpuCanvas>,
    /// Run once the first frames have rendered (see [`Self::on_started`]).
    started: Option<gpu_status::StartedHook>,
    /// Frame and canvas-upload timings (exposed via `ui.inspect`).
    pub perf: gpu_canvas::Perf,
    /// Preferences, autosave and snapping runtime state (see `prefs_ui`, `snap_ui`).
    pub(crate) prefs_rt: prefs_ui::Runtime,
    /// Close, Revert or Exit parked behind the unsaved-changes prompt (see `discard_ui`).
    pub(crate) discard: Option<discard_ui::Prompt>,
    /// The Open or Save dialog in progress, and the action waiting on it (see `file_dialog`).
    pub(crate) file_dialog: Option<file_dialog::Pending>,
    /// A Save As to a layered TIFF parked behind the TIFF Options prompt (see `tiff_options_ui`).
    pub(crate) tiff_options: Option<tiff_options_ui::Prompt>,
    /// Set once the user has agreed to quit, so the resulting close request goes through.
    pub(crate) allow_close: bool,
    /// Pen pressure/tilt from the platform (see `stylus`).
    pub stylus: stylus::Stylus,
    /// Run long commands and file opens as background jobs with progress and Cancel (#210; see
    /// `jobs_ui`). The desktop app turns it on; off (the default), everything runs inline as
    /// before, which tests and scripts rely on.
    pub background_jobs: bool,
    /// Background job bookkeeping: opening tabs, control replies waiting on a job.
    pub jobs: jobs_ui::JobsUi,
    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
    live_tokens: theme::live::LiveTokens,
}

impl PhotocraftApp {
    #[cfg(not(all(target_arch = "wasm32", feature = "web_external_assets")))]
    pub fn new(session: Session, services: Services) -> Self {
        Self::new_inner(session, services)
    }
    #[cfg(all(target_arch = "wasm32", feature = "web_external_assets"))]
    pub fn new_ready(session: Session, services: Services, ready: web_ui_assets::ReadyUiAssets) -> Result<Self, web_ui_assets::UiAssetError> {
        let _ = i18n::Lang::try_from_pref(&session.prefs().interface.language)?;
        Ok(Self::new_inner(session, services, ready))
    }
    fn new_inner(
        session: Session,
        services: Services,
        #[cfg(all(target_arch = "wasm32", feature = "web_external_assets"))] ready: web_ui_assets::ReadyUiAssets,
    ) -> Self {
        let mut app = Self {
            #[cfg(all(target_arch = "wasm32", feature = "web_external_assets"))]
            web_ui_assets: ready,
            session,
            ui: UiState::default(),
            services,
            canvases: HashMap::new(),
            navigator_textures: HashMap::new(),
            monitors: Default::default(),
            checker: None,
            drag: None,
            live_stroke: None,
            trail: None,
            move_preview: None,
            blend_preview: None,
            patch_preview: None,
            magnetic: Default::default(),
            secondary_erase: false,
            defer_live_stroke: false,
            press_stroke: false,
            last_stroke_end: None,
            brush_resize: None,
            quick_pick: false,
            brush_resize_armed: false,
            alt_sampling: false,
            opacity_keys: None,
            control_rx: None,
            pending_screenshots: Vec::new(),
            queued_screenshots: Vec::new(),
            input_waiters: Vec::new(),
            live_adjust: None,
            frame: 0,
            styled: false,
            integrated_titlebar: false,
            custom_titlebar: false,
            fonts_ready: false,
            last_canvas_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            drop_canvas_rect: None,
            tab_strip: None,
            drop_places: Default::default(),
            view_docs: Vec::new(),
            fps: 0.0,
            last_frame_time: 0.0,
            thumbs: HashMap::new(),
            thumb_documents: Vec::new(),
            bounds_cache: HashMap::new(),
            source_bounds_cache: HashMap::new(),
            proxy: None,
            proxy_uploaded: None,
            outline_cache: None,
            filter_preview: None,
            color_range: None,
            adjust_preview: None,
            synthetic: Vec::new(),
            automation_input: false,
            channel_thumbs: None,
            channel_views: HashMap::new(),
            type_layout: None,
            type_transform_preview: None,
            guide_drag: None,
            crop: Default::default(),
            hover_doc: None,
            clone_preview: None,
            info_sample: None,
            os_clip_sig: None,
            clip_external: false,
            clip_read_for_paste: false,
            transform_preview: None,
            move_mods: Default::default(),
            style_preview: None,
            distort: Default::default(),
            gradient: Default::default(),
            camera_raw: None,
            pending_raw_open: None,
            raw_redevelop: None,
            wide_angle: None,
            tone_hist: None,
            doc_hist: None,
            hist_job: None,
            gpu: None,
            started: None,
            perf: Default::default(),
            prefs_rt: Default::default(),
            discard: None,
            file_dialog: None,
            tiff_options: None,
            allow_close: false,
            stylus: Default::default(),
            background_jobs: false,
            jobs: Default::default(),
            #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
            live_tokens: theme::live::LiveTokens::from_env(),
        };
        // Saved preferences (and recovered documents) are in place before the first frame.
        prefs_ui::load(&mut app);
        notices::wayland_file_drop_guidance(&mut app);
        // File › Scripts › Script Events Manager: "Start Application".
        photocraft_engine::automate_cmds::fire_event(&mut app.session, "startApplication");
        app
    }

    /// Draw the document canvas on the GPU (custom WGSL shader) instead of via egui textures.
    /// Call from the app creator with `cc.wgpu_render_state`; without it the CPU path is used.
    pub fn set_wgpu(&mut self, rs: eframe::egui_wgpu::RenderState) {
        // Preferences › Performance › cache tile size (PHOTOCRAFT_GPU_TILE still overrides).
        let tile = self.session.prefs().performance.cache_tile_size;
        // Escaped driver/setup panics must leave the session and CPU canvas alive.
        self.perf.gpu_info.set_adapter(&rs.adapter.get_info());
        let gpu = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gpu_canvas::GpuCanvas::with_tile(&rs, Some(tile)))) {
            Ok(gpu) => gpu,
            Err(payload) => {
                let detail = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                    .unwrap_or_else(|| "GPU canvas initialization failed".into());
                self.perf.gpu_info.canvas = "cpu".into();
                self.perf.gpu_info.fallback = Some(detail.clone());
                gpu_status::queue_fallback_notice(self, detail);
                return;
            }
        };
        self.perf.gpu_info.canvas = "gpu".into();
        self.gpu = Some(gpu);
        self.prefs_rt.gpu_style = None;
    }

    /// Whether the document canvas currently draws on the GPU (false on the CPU path, and after
    /// the device was lost).
    pub fn gpu_active(&self) -> bool {
        self.gpu.is_some()
    }

    /// The GPU canvas's device health flag (tests inject faults through it).
    pub fn gpu_health(&self) -> Option<photocraft_gpu::DeviceHealth> {
        self.gpu.as_ref().map(|g| g.health().clone())
    }

    /// Run `hook` once the app has rendered its first frames (and a document opened at launch
    /// has drawn): the desktop app clears its crash-safe GPU startup marker there.
    pub fn on_started(&mut self, hook: impl FnOnce(&mut PhotocraftApp) + 'static) {
        self.started = Some(Box::new(hook));
    }

    /// Per-frame GPU health check (called from `logic`; tests call it directly): falls back to
    /// the CPU canvas after a device loss and runs the started hook.
    pub fn check_gpu(&mut self, ctx: &egui::Context) {
        gpu_status::check(self, ctx);
    }

    /// Attach a control channel (requests arrive from a transport thread: TCP, stdin, tests).
    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Run an engine command, reporting errors in the status bar.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        // Automation input also gates every step a command runs on its behalf (`actions.play`).
        let gate = if self.automation_input && self.session.authorize.is_none() { self.services.automation_authorize } else { None };
        if gate.is_some() {
            self.session.authorize = gate;
        }
        let result = self.run_command(id, params);
        if gate.is_some() {
            self.session.authorize = None;
        }
        result
    }

    fn run_command(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let clip_read = std::mem::take(&mut self.clip_read_for_paste);
        if self.automation_input
            && let Some(authorize) = self.services.automation_command.as_ref()
        {
            authorize(id, &params)?;
        }
        if let Some(r) = transform_tool::intercept(self, id) {
            return r;
        }
        let suppress_events = self.automation_input && self.session.prefs().script_events.enabled;
        if suppress_events {
            self.session.edit_prefs(|prefs| prefs.script_events.enabled = false).map_err(|error| error.to_string())?;
        }
        let t0 = gpu_canvas::now_ms();
        // The OS clipboard is read only on an explicit paste, never in the background (privacy, CPU).
        if matches!(id, "edit.paste" | "edit.pasteSpecial.pasteInPlace" | "file.newFromClipboard") {
            if !clip_read {
                self.import_os_clipboard();
            }
            if self.session.clipboard.is_none() && self.services.clipboard_get_image.is_some() {
                // Enabled on the strength of the OS clipboard, which held no image: a quiet no-op.
                self.ui.status = "Nothing to paste: the clipboard holds no image".into();
                self.ui.status_error = false;
                return Ok(serde_json::json!({"pasted": false}));
            }
        }
        // Long commands become background jobs when enabled (`jobs_ui`); the rest run inline.
        let params = self.with_mask_target(id, params);
        let params = vector_ui::with_active_path(self, id, params);
        let path_mask = vector_ui::takes_path_mask(id) && params.get("path").is_some_and(|v| !v.is_null());
        let r = if id == "actions.play" {
            actions::play(self, &params)
        } else if photocraft_engine::actions_cmds::shell_view_command(id) {
            actions::run_view(self, id, params)
        } else {
            jobs_ui::run(self, id, params)
        };
        if r.is_ok() && id == "select.toWorkPath" {
            // Make Work Path selects the new work path in the Paths panel, as in Photoshop.
            self.ui.selected_path = Some("work".into());
        }
        if r.is_ok() && path_mask {
            // The new layer's vector mask becomes the active path, as in Photoshop: the path it
            // was made from is no longer selected, so the next fill layer isn't masked by it too.
            self.ui.selected_path = Some("layer".into());
        }
        if r.is_ok() && matches!(id, "edit.copy" | "edit.cut" | "edit.copyMerged") {
            self.clip_external = false;
            self.export_os_clipboard();
        }
        self.perf.last_command = id.to_string();
        self.perf.command_ms = gpu_canvas::now_ms() - t0;
        match &r {
            Ok(value) => {
                self.sync_views();
                if self.ui.status_error {
                    self.ui.status.clear();
                    self.ui.status_error = false;
                }
                if let Some(items) = value.get("warnings").and_then(Value::as_array) {
                    let warnings: Vec<String> = items.iter().filter_map(|warning| warning.as_str().map(str::to_string)).collect();
                    notices::io_warnings(self, id, &warnings);
                }
            }
            Err(e) => {
                self.ui.status = e.clone();
                self.ui.status_error = true;
            }
        }
        if suppress_events {
            self.session.edit_prefs(|prefs| prefs.script_events.enabled = true).map_err(|error| error.to_string())?;
        }
        r
    }

    /// Keep one view per document, in tab order: a view and its windows stay with their document
    /// when tabs move (`document.move`) or close.
    pub fn sync_views(&mut self) {
        type_transform::cancel_stale(self);
        crate::lasso_ui::cancel_stale(self);
        let ids: Vec<DocId> = self.session.documents().iter().map(|d| d.doc.id).collect();
        // Where the document of view `i` is now. Views not tracked yet keep their index.
        let now = |i: usize| match self.view_docs.get(i) {
            Some(id) => ids.iter().position(|d| d == id),
            None => (i < ids.len()).then_some(i),
        };
        let mut views: Vec<Option<state::View>> = ids.iter().map(|_| None).collect();
        for (i, v) in std::mem::take(&mut self.ui.views).into_iter().enumerate() {
            if let Some(slot) = now(i).and_then(|to| views.get_mut(to)) {
                *slot = Some(v);
            }
        }
        self.ui.views = views.into_iter().map(Option::unwrap_or_default).collect();
        self.ui.windows.retain_mut(|w| now(w.document).map(|d| w.document = d).is_some());
        self.canvases.retain(|(doc, _), _| ids.contains(doc));
        self.navigator_textures.retain(|doc, _| ids.contains(doc));
        self.view_docs = ids;
        self.prune_thumbs();
        self.sync_mask_targets();
        // Channel-view textures outlive a hidden view (cheap re-show), not their document.
        if !self.channel_views.is_empty() {
            let docs = self.session.documents();
            self.channel_views.retain(|id, _| docs.iter().any(|d| d.doc.id.0 == *id));
        }
        // Commands may close and reopen a preserved-ID document before the next repaint.
        canvas::retain_gpu_documents(self);
    }

    /// Record `path` as the most-recently-opened file (File › Open Recent): de-duplicated, newest
    /// first, capped at Preferences › File Handling › Recent File List Contains. The list lives in
    /// the preferences (`fileHandling.recentFiles`), so it survives a restart; `ui.recent_files`
    /// mirrors it for the menus and the Home screen.
    pub fn push_recent(&mut self, path: &str) {
        let cap = self.recent_cap();
        let mut r = self.session.prefs().file_handling.recent_files.clone();
        r.retain(|p| p != path);
        r.insert(0, path.to_string());
        r.truncate(cap);
        self.ui.recent_files = r.clone();
        self.session.prefs.edit(|p| p.file_handling.recent_files = r);
    }

    /// File › Open Recent › Clear Recent File List.
    pub fn clear_recent(&mut self) {
        self.ui.recent_files.clear();
        self.session.prefs.edit(|p| p.file_handling.recent_files.clear());
    }

    /// How many recent files to remember ("Recent File List Contains", 0–100).
    pub fn recent_cap(&self) -> usize {
        self.session.prefs().file_handling.recent_file_count.min(100) as usize
    }

    /// Mirror the stored recent-file list into the UI state (after loading the preferences, or
    /// when the Preferences dialog or an agent changes the list or its length). Cheap enough to
    /// run every frame: a slice comparison of at most 100 paths, no allocation unless it changed.
    pub fn sync_recent(&mut self) {
        let cap = self.recent_cap();
        let stored = &self.session.prefs().file_handling.recent_files;
        let want = stored.get(..cap.min(stored.len())).unwrap_or_default();
        if self.ui.recent_files.as_slice() != want {
            self.ui.recent_files = want.to_vec();
        }
    }

    /// Open a file's bytes as a new document named `name`; returns the import warnings (also
    /// shown to the user). Files from disk go through [`open_file`](Self::open_file), which also
    /// remembers the path. Brush and gradient files go to the preset libraries instead.
    pub fn open_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
        if let Some(r) = preset_files_ui::open(self, name, bytes) {
            return r.map(|()| Vec::new());
        }
        let name = &self.open_name(name);
        // Decoded on a worker: a tab with progress appears now, the document when it's ready
        // (warnings are shown then).
        if self.background_jobs {
            jobs_ui::start_open(self, name, None, jobs_ui::bytes(bytes))?;
            return Ok(Vec::new());
        }
        let import = self.services.import.as_ref().ok_or("no importer configured")?;
        let imported = import(name, bytes)?;
        self.finish_import(name, imported, Some(bytes))
    }

    /// Open bytes supplied by an authenticated automation client without
    /// invoking user-configured script-event paths outside the granted root.
    /// Returns the import warnings (also shown to the user). Brush and gradient
    /// files go to the preset libraries, as for interactive opens.
    pub fn open_automation_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
        let events_enabled = self.session.prefs().script_events.enabled;
        if events_enabled {
            self.session.edit_prefs(|prefs| prefs.script_events.enabled = false).map_err(|error| error.to_string())?;
        }
        let result = match preset_files_ui::open(self, name, bytes) {
            Some(r) => r.map(|()| Vec::new()),
            None => self.import_automation_document(name, bytes),
        };
        if events_enabled {
            self.session.edit_prefs(|prefs| prefs.script_events.enabled = true).map_err(|error| error.to_string())?;
        }
        result
    }

    /// The document half of [`open_automation_bytes`](Self::open_automation_bytes): no script
    /// events and no Color Settings policy (which may read user-configured profile paths).
    fn import_automation_document(&mut self, name: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
        let import = self.services.import.as_ref().ok_or("no importer configured")?;
        let photocraft_io::ImportResult { document: doc, mut warnings, origin, assessment } = import(name, bytes)?;
        import_assessment_ui::append(&mut warnings, assessment.as_ref());
        // The caller records the path it read from.
        self.session.add_document(doc, None).map_err(|error| error.to_string())?;
        if let Some(state) = self.session.active_mut() {
            state.set_import_origin(origin, name, None);
        }
        self.sync_views();
        self.ui.status = format!("Opened {name}");
        self.ui.status_error = false;
        notices::io_warnings(self, &format!("Opened {name}"), &warnings);
        Ok(warnings)
    }

    /// Save the active document to `path`, or to a path chosen in the save dialog (which
    /// continues on a later frame, see `file_dialog`). Returns `{"path", "warnings"}` (the export
    /// warnings are also shown to the user).
    pub fn save_as(&mut self, path: Option<String>) -> Result<Value, String> {
        // Edit Contents documents save back into their smart object.
        if path.is_none() && self.session.is_enabled("layer.smartObjects.saveContents") {
            self.run("layer.smartObjects.saveContents", serde_json::json!({}))?;
            return Ok(serde_json::json!({"path": "smart object", "warnings": []}));
        }
        if let Some(path) = path {
            return self.save_to(path);
        }
        let st = self.session.active().ok_or("no document")?;
        // Preserve writable paths; prefer native saves for retained XCF state and keep owned display names.
        let ext = st.path.as_deref().and_then(|p| std::path::Path::new(p).extension()).map(|e| e.to_string_lossy().to_ascii_lowercase());
        let writable = ext.is_some_and(|e| {
            matches!(e.as_str(), photocraft_format::EXTENSION | "psd" | "psb") || photocraft_codecs::from_extension(&e).is_some_and(|f| f.caps().write)
        });
        let suggested_extension = if st.doc.xcf_state.is_some()
            || st.source.as_ref().is_some_and(|source| source.origin.is_read_only() || matches!(source.origin.format, photocraft_io::ImportFormat::Pcraft))
        {
            "pcraft"
        } else {
            "psd"
        };
        let suggested = match &st.path {
            Some(p) if writable => p.clone(),
            None if st.doc.xcf_state.is_some()
                && st.source.as_ref().is_none_or(|source| matches!(source.origin.format, photocraft_io::ImportFormat::Pcraft)) =>
            {
                if st.doc.name.rsplit_once('.').is_some_and(|(base, ext)| !base.is_empty() && ext.eq_ignore_ascii_case(photocraft_format::EXTENSION)) {
                    st.doc.name.clone()
                } else {
                    format!("{}.pcraft", st.doc.name)
                }
            }
            p => std::path::Path::new(p.as_deref().unwrap_or(&st.doc.name)).with_extension(suggested_extension).to_string_lossy().into_owned(),
        };
        let doc = st.doc.id;
        self.pick_save(&suggested, move |app, path| {
            app.refocus(doc)?;
            app.save_to(path)
        })
    }

    /// Explicit native Save As for imported-project preservation controls.
    /// The ordinary Save As menu retains its existing format choice policy.
    pub fn save_native_as(&mut self, path: Option<String>) -> Result<Value, String> {
        let active = self.session.active().ok_or("no document")?;
        if self.ui.transform.is_some() || self.session.has_jobs() || active.floating.is_some() { return Err("Native Save As requires the current edit to finish".into()); }
        if active.path.as_deref().unwrap_or(&active.doc.name).len() > 32768 { return Err("Native Save As suggestion exceeds the path limit".into()); }
        let document = active.doc.id;
        let revision = active.revision;
        let snapshot = active.doc.downgrade();
        let suggested = std::path::Path::new(active.path.as_deref().unwrap_or(&active.doc.name))
            .with_extension(photocraft_format::EXTENSION).to_string_lossy().into_owned();
        let save = move |app: &mut Self, path: String| {
            if path.len() > 32768 || path.contains('\0') { return Err("Native Save As destination exceeds the path limit".into()); }
            if !std::path::Path::new(&path).extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case(photocraft_format::EXTENSION)) {
                return Err("Native Save As requires a .pcraft destination".into());
            }
            if !snapshot.upgrade().is_some_and(|captured| app.session.active().is_some_and(|current|
                current.doc.id == document && current.revision == revision && current.floating.is_none() && captured.shares_state(&current.doc)))
                || app.ui.transform.is_some() || app.session.has_jobs() {
                return Err("Native Save As target changed while choosing a destination".into());
            }
            app.save_to(path)
        };
        match path {
            Some(path) => save(self, path),
            None => self.pick_save(&suggested, save),
        }
    }

    /// [`Self::save_as`] once the path is known.
    fn save_to(&mut self, path: String) -> Result<Value, String> {
        // A layered TIFF asks about its layers first (Preferences › File Handling); the save
        // continues from the prompt.
        if tiff_options_ui::wants_prompt(self, &path) {
            tiff_options_ui::park(self, path.clone());
            return Ok(serde_json::json!({"path": path, "warnings": []}));
        }
        if photocraft_engine::file_cmds::extension(&path).as_deref() == Some("pcraft") && self.services.native_save.is_some() {
            self.validate_document_destination(&path, false)?;
            let mut protection = photocraft_engine::WriteProtection::default();
            for document in self.session.documents() {
                protection.add_document(&document.doc, document.source.as_ref())?;
            }
            if let Some(validate) = self.services.validate_write_protected.as_ref() {
                validate(&path, &protection)?;
            }
            let captured = self.session.active().ok_or("no document")?.doc.clone();
            let revision = self.session.active().ok_or("no document")?.revision;
            let saved = self.services.native_save.as_mut().ok_or("no native writer configured")?(&captured, &path, &protection)?;
            // The synchronous service receives no Session access. Still verify
            // the exact target before updating saved-path bookkeeping.
            let state = self.session.active_mut().ok_or("no document")?;
            if state.revision != revision || !state.doc.shares_state(&captured) {
                return Err("native-save.stale-target".into());
            }
            state.path = Some(path.clone());
            state.saved_revision = state.revision;
            self.ui.status = format!("Saved {path}");
            if let Some(i) = self.session.active_index()
                && let Some(result) = photocraft_engine::automate_cmds::document_saved(&mut self.session, i)
            {
                self.ui.status = format!("Saved {path}; {} image assets in {}", result["files"].as_array().map_or(0, Vec::len), result["dir"].as_str().unwrap_or(""));
            }
            self.ui.status_error = false;
            notices::io_warnings(self, &format!("Saved {}", file_open::display_name(&path)), &saved.warnings);
            self.sync_views();
            return Ok(serde_json::json!({"path": path, "warnings": saved.warnings}));
        }
        let (path, warnings) = self.write_document(path, &ExportSettings::default())?;
        Ok(serde_json::json!({"path": path, "warnings": warnings}))
    }

    /// Encodes the active document with `settings` and writes it to `path`, which becomes the
    /// document's path. Returns the path and the export warnings (also shown to the user).
    pub(crate) fn write_document(&mut self, path: String, settings: &ExportSettings) -> Result<(String, Vec<String>), String> {
        self.validate_document_destination(&path, false)?;
        let st = self.session.active().ok_or("no document")?;
        let export = self.services.export.as_ref().ok_or("no exporter configured")?;
        let protection = self.open_document_write_protection()?;
        let (bytes, warnings) = export(&st.doc, &path, settings)?;
        if let Some(write) = self.services.write_protected.as_mut() {
            write(&path, &bytes, &protection)?;
        } else {
            let write = self.services.write.as_mut().ok_or("no writer configured")?;
            write(&path, &bytes)?;
        }
        if let Some(st) = self.session.active_mut() {
            st.path = Some(path.clone());
            st.saved_revision = st.revision;
        }
        self.ui.status = format!("Saved {path}");
        // "Save Document" script events and File › Generate › Image Assets.
        if let Some(i) = self.session.active_index()
            && let Some(r) = photocraft_engine::automate_cmds::document_saved(&mut self.session, i)
        {
            self.ui.status = format!("Saved {path}; {} image assets in {}", r["files"].as_array().map_or(0, Vec::len), r["dir"].as_str().unwrap_or(""));
        }
        self.ui.status_error = false;
        notices::io_warnings(self, &format!("Saved {}", file_open::display_name(&path)), &warnings);
        self.sync_views();
        Ok((path, warnings))
    }

    /// Validate using the authority of the selected write route before encoding or touching it.
    pub(crate) fn validate_document_destination(&self, path: &str, automation: bool) -> Result<(), String> {
        let state = self.session.active().ok_or("no document")?;
        if photocraft_engine::file_cmds::extension(path).as_deref() == Some("xcf") {
            return Err("XCF is an import-only format; choose a writable destination such as .pcraft".into());
        }
        for document in self.session.documents() {
            if let Some(source) = document.source.as_ref() {
                source.check_destination(path)?;
            }
        }
        if automation {
            if let Some(validate) = self.services.automation_validate_write.as_ref() {
                let protection = self.open_document_write_protection()?;
                validate(path, &protection)?;
            }
        } else if let Some(validate) = self.services.validate_write_protected.as_ref() {
            let protection = self.open_document_write_protection()?;
            validate(path, &protection)?;
        } else if let Some(validate) = self.services.validate_write.as_ref() {
            validate(path, state.source.as_ref())?;
        }
        Ok(())
    }

    fn open_document_write_protection(&self) -> Result<photocraft_engine::WriteProtection, String> {
        let mut protection = photocraft_engine::WriteProtection::default();
        for document in self.session.documents() {
            protection.add_document(&document.doc, document.source.as_ref())?;
        }
        Ok(protection)
    }

    /// Save through the control session's capability-scoped writer. No file
    /// picker or ambient writer is reachable from this path. Returns the path
    /// written and the export warnings (also shown to the user).
    pub fn save_automation(&mut self, path: Option<String>) -> Result<(String, Vec<String>), String> {
        let state = self.session.active().ok_or("no document")?;
        // As File › Save: without `path` only a layered file is written back (#416).
        let target = path
            .or_else(|| state.path.clone().filter(|p| photocraft_engine::file_cmds::saves_in_place(p)))
            .ok_or("pass `path`: a save without one writes back only to the document's own PSD, PSB or .pcraft file")?;
        self.validate_document_destination(&target, true)?;
        let protection = self.open_document_write_protection()?;
        if photocraft_engine::file_cmds::extension(&target).as_deref() == Some("pcraft")
            && self.services.automation_native_save.is_some()
        {
            let captured = state.doc.clone();
            let revision = state.revision;
            let saved = self.services.automation_native_save.as_mut().ok_or("automation native write authority is not configured")?(&captured, &target, &protection)?;
            let state = self.session.active_mut().ok_or("no document")?;
            if state.revision != revision || !state.doc.shares_state(&captured) {
                return Err("native-save.stale-target".into());
            }
            state.path = Some(target.clone());
            state.saved_revision = state.revision;
            self.ui.status = format!("Saved {target}");
            self.ui.status_error = false;
            notices::io_warnings(self, &format!("Saved {}", file_open::display_name(&target)), &saved.warnings);
            self.sync_views();
            return Ok((target, saved.warnings));
        }
        let export = self.services.export.as_ref().ok_or("no exporter configured")?;
        let (bytes, warnings) = export(&state.doc, &target, &ExportSettings::default())?;
        let write = self.services.automation_write.as_mut().ok_or("automation write authority is not configured")?;
        write(&target, &bytes, &protection)?;
        if let Some(state) = self.session.active_mut() {
            state.path = Some(target.clone());
            state.saved_revision = state.revision;
        }
        self.ui.status = format!("Saved {target}");
        self.ui.status_error = false;
        notices::io_warnings(self, &format!("Saved {}", file_open::display_name(&target)), &warnings);
        self.sync_views();
        Ok((target, warnings))
    }

    fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        while let Ok(req) = rx.try_recv() {
            let reply = req.reply.clone();
            if req.deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
                let _ = reply.send(serde_json::json!({"ok": false, "error": "timeout"}));
                continue;
            }
            match control::handle(self, ctx, &req) {
                control::Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                control::Outcome::AfterInput => self.input_waiters.push(reply),
                control::Outcome::Deferred => {},
                control::Outcome::AfterJob(job) => self.jobs.waiters.push((job, reply)),
                control::Outcome::Screenshot { token, path } => {
                    // Wait out egui's fade animations (~83 ms) and a few rendered frames first.
                    let settle = ctx.global_style().animation_time as f64 * 2000.0 + 60.0;
                    self.queued_screenshots.push((token, gpu_canvas::now_ms() + settle, 0));
                    self.pending_screenshots.push((token, path, reply));
                }
            }
        }
        self.control_rx = Some(rx);
    }

    fn issue_screenshots(&mut self, ctx: &egui::Context) {
        let now = gpu_canvas::now_ms();
        self.queued_screenshots.retain_mut(|(token, at, frames)| {
            *frames += 1;
            if now >= *at && *frames >= 3 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(*token)));
                false
            } else {
                true
            }
        });
    }

    fn collect_screenshots(&mut self, ctx: &egui::Context) {
        if self.pending_screenshots.is_empty() {
            return;
        }
        let events: Vec<_> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { user_data, image, .. } => {
                        let token = user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()).copied()?;
                        Some((token, image.clone()))
                    }
                    _ => None,
                })
                .collect()
        });
        for (token, image) in events {
            if let Some(i) = self.pending_screenshots.iter().position(|(t, _, _)| *t == token) {
                let (_, path, reply) = self.pending_screenshots.remove(i);
                let r = control::save_screenshot(self, &image, path.as_deref());
                let _ = reply.send(r);
            }
        }
    }
}

impl eframe::App for PhotocraftApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        match i18n::Lang::try_from_pref(&self.session.prefs().interface.language) {
            Ok(lang) => i18n::set_current(lang),
            Err(error) => {
                self.ui.status = error.to_string();
                self.ui.status_error = true;
                return;
            }
        }
        if !self.styled {
            #[cfg(all(target_arch = "wasm32", feature = "web_external_assets"))]
            let setup = Self::setup_context_ready(ctx, self.ui.theme, self.web_ui_assets);
            #[cfg(not(all(target_arch = "wasm32", feature = "web_external_assets")))]
            let setup = Self::try_setup_context(ctx, self.ui.theme);
            if let Err(error) = setup {
                self.ui.status = error.to_string();
                self.ui.status_error = true;
                return;
            }
            self.styled = true;
        } else {
            self.fonts_ready = true;
        }
        self.frame += 1;
        let now = ctx.input(|i| i.time);
        let dt = (now - self.last_frame_time) as f32;
        if dt > 0.0 {
            self.fps = self.fps * 0.9 + (1.0 / dt).min(240.0) * 0.1;
        }
        self.last_frame_time = now;
        self.sync_views();
        self.check_gpu(ctx);
        #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
        if self.live_tokens.poll(ctx, self.ui.theme) {
            self.checker = None;
        }
        self.drain_control(ctx);
        native_menu::run(self, ctx);
        if self.ui.text_edit.is_some() && !self.ui.tool.is_type() {
            type_tool::commit(self);
        }
        if self.ui.pen.is_some() && self.ui.tool != state::Tool::Pen {
            vector_ui::pen_commit(self, false);
        }
        transform_tool::end_if_left(self);
        self.collect_screenshots(ctx);
        self.issue_screenshots(ctx);
        prefs_ui::tick(self, ctx);
        monitor_status::poll(self, ctx);
        // Control requests and persisted preferences can change the language in this frame.
        if let Err(error) = i18n::try_sync_context(ctx, &self.session.prefs().interface.language) {
            self.ui.status = error.to_string();
            self.ui.status_error = true;
            return;
        }
        // A window bigger than its display (1440 × 900 on 1366 × 768) runs under the taskbar:
        // maximize it into the work area once (#315).
        work_area::fit_window(ctx);
        discard_ui::guard_window_close(self, ctx);
        // Background jobs: apply finished ones, keep frames coming, Esc cancels (before the
        // shortcuts see Esc).
        if !screen_picker::tick(self, ctx) {
            jobs_ui::tick(self, ctx);
            shortcuts::handle(self, ctx);
        }
        let arrived: Vec<(String, Vec<u8>)> =
            self.services.inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
        for (name, bytes) in arrived {
            if let Err(e) = self.open_bytes(&name, &bytes) {
                self.open_failed(&name, &e);
            }
        }
        // Finder double-click / Open With / Dock drops (macOS open-documents events).
        self.drain_os_events(ctx);
        // While files are dragged over the window it gets no pointer events: keep frames coming so
        // the tab strip can follow the pointer.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            ctx.request_repaint();
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() {
            let at = self.services.cursor_pos.as_mut().and_then(|f| f(ctx));
            self.open_dropped(ctx, dropped, at);
        }
        self.place_next_dropped(ctx);
        // The control transport wakes the UI on arrival (ctx.request_repaint); only poll while a
        // screenshot is pending. (Polling every 50 ms here made idle apps render at 20 fps.)
        if !self.pending_screenshots.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        self.poll_file_dialog(ctx, Some(frame));
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Native menu key equivalents become the key presses they were (see `native_menu`).
        if let Some(menu) = self.services.native_menu.as_mut() {
            menu.raw_input(raw_input);
        }
        shortcuts::clipboard_keys(ctx, ctx.text_edit_focused() || self.ui.text_edit.is_some(), raw_input);
        // Windows sends a touchpad pinch as Ctrl + wheel; make it a pinch again (wheel_nav.rs).
        if cfg!(target_os = "windows") {
            wheel_nav::fold_legacy_pinch(ctx, raw_input);
        }
        raw_input.events.extend(self.take_synthetic_step());
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        match i18n::Lang::try_from_pref(&self.session.prefs().interface.language) {
            Ok(lang) => i18n::set_current(lang),
            Err(error) => {
                self.ui.status = error.to_string();
                self.ui.status_error = true;
                return;
            }
        }
        // Fonts registered via set_fonts only take effect next frame; named families would panic now.
        if !self.fonts_ready {
            ctx.request_repaint();
            self.automation_input = false;
            return;
        }
        let t0 = gpu_canvas::now_ms();
        // Each painting tool keeps its own brush (#218), so switch the active tool's brush in
        // before anything this frame reads it (the cursor, the options bar, a stroke).
        paint_mouse::sync_tool_brush(self);
        // View › Screen Mode › Full Screen Mode: only the image, on black (F or Esc returns).
        screen_picker::show(&ctx);
        if screen_picker::busy(&ctx) && screen_picker::showing(&ctx) {
            egui::Modal::new(egui::Id::new("screen-color-wait")).show(&ctx, |ui| {
                ui.spinner();
                ui.label(tl!("Pick a screen pixel or press Esc to cancel"));
                if ui.button(tl!("Cancel")).clicked() {
                    screen_picker::cancel(&ctx);
                }
            });
            self.automation_input = false;
            return;
        }
        if screen_picker::busy(&ctx) {
            ui.disable();
            ui.set_opacity(1.0);
        }
        let chrome = !self.ui.view.hides_chrome();
        if !chrome && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            let _ = menus::invoke(self, &ctx, "view.screenMode.standard", serde_json::json!({}));
        }
        if chrome {
            panels::title_bar(self, ui);
        }
        if chrome && self.ui.panels.options_bar {
            panels::options_bar(self, ui);
        }
        if chrome && self.ui.panels.status_bar {
            panels::status_bar(self, ui);
        }
        if chrome && self.ui.panels.toolbar {
            panels::toolbar(self, ui);
        }
        if chrome && self.ui.panels.dock {
            panels::right_dock(self, ui);
        }
        let t = theme::Tokens::get(&ctx);
        let backdrop = if chrome { prefs_ui::pasteboard_color(self).unwrap_or(t.canvas) } else { egui::Color32::BLACK };
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(backdrop)).show(ui, |ui| {
            canvas::document_area(self, ui);
        });
        panels::properties_window(self, &ctx);
        brush_panel::window(self, &ctx);
        preset_panels::windows(self, &ctx);
        type_panels_ui::windows(self, &ctx);
        analysis_ui::windows(self, &ctx);
        timeline_ui::windows(self, &ctx);
        workspace_ui::windows(self, &ctx);
        palette::show(self, &ctx);
        dialogs::show(self, &ctx);
        jobs_ui::dialog(self, &ctx);
        discard_ui::show(self, &ctx);
        tiff_options_ui::show(self, &ctx);
        distort_ui::show(self, &ctx);
        camera_raw_ui::show(self, &ctx);
        wide_angle_ui::show(self, &ctx);
        canvas::extra_windows(self, &ctx);
        notices::show(self, &ctx);
        gpu_status::show_fallback(self, &ctx);
        if self.custom_titlebar {
            titlebar::resize_zones(ui);
        }
        // A device lost while drawing this frame: switch to the CPU canvas before the next one.
        gpu_status::check(self, &ctx);
        self.automation_input = false;
        native_menu::sync(self, &ctx);
        if screen_picker::busy(&ctx) {
            // Block input without a backdrop: the worker may still be capturing the screen.
            egui::Modal::new(egui::Id::new("screen-color-wait")).frame(egui::Frame::NONE).backdrop_color(egui::Color32::TRANSPARENT).show(&ctx, |_| {});
            ctx.set_cursor_icon(egui::CursorIcon::Wait);
        }
        self.perf.frame(gpu_canvas::now_ms() - t0);
        // Synthetic input is injected one press/release step per frame: keep frames coming until
        // the queue is empty, then release control replies waiting on it.
        if !self.synthetic.is_empty() {
            ctx.request_repaint();
        } else if !self.input_waiters.is_empty() {
            for w in self.input_waiters.drain(..) {
                let _ = w.send(serde_json::json!({"ok": true, "result": null}));
            }
        }
        // Menus and buttons in this frame may have asked for a file dialog.
        self.poll_file_dialog(&ctx, Some(frame));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_dropped(f: &(dyn egui::DroppedFile + Send + Sync)) -> Result<Vec<u8>, String> {
    f.bytes()
}

/// On the web, dropped-file bytes arrive asynchronously; the web shell handles drops itself.
#[cfg(target_arch = "wasm32")]
fn read_dropped(_f: &dyn egui::DroppedFile) -> Result<Vec<u8>, String> {
    Err("drag-and-drop on the web is handled by the page; use File → Open".into())
}

impl PhotocraftApp {
    /// `params` aimed at the active layer's mask (`"target":"mask"`) when the Layers panel targets
    /// it and command `id` edits the target (adjustments, filters, fills) without naming one:
    /// ⌘I then inverts the mask, as in Photoshop (#780). A targeted alpha channel or Quick Mask
    /// mode wins, as the engine routes those itself.
    pub fn with_mask_target(&self, id: &str, params: Value) -> Value {
        if !self.ui.mask_target || !photocraft_engine::channel_cmds::follows_target(id) || params.get("target").is_some() {
            return params;
        }
        let Some(st) = self.session.active() else { return params };
        let composite = st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::Composite && st.doc.quick_mask.is_none();
        if !composite || st.active_layer.and_then(|id| st.doc.layer(id)).is_none_or(|l| l.mask.is_none()) {
            return params;
        }
        match params {
            Value::Object(mut m) => {
                m.insert("target".into(), Value::from("mask"));
                Value::Object(m)
            }
            _ => serde_json::json!({ "target": "mask" }),
        }
    }

    /// Viewing a layer mask (#196) targets it; a vector-mask target needs a vector mask on the
    /// active layer (a shape layer's path is its content, not a mask).
    fn sync_mask_targets(&mut self) {
        let Some(st) = self.session.active() else { return };
        if photocraft_engine::mask_view_cmds::current(st).is_some() {
            self.ui.mask_target = true;
            self.ui.vector_mask_target = false;
        }
        if self.ui.vector_mask_target && !mask_thumbs_ui::has_vector_mask(st) {
            self.ui.vector_mask_target = false;
        }
    }

    fn prune_thumbs(&mut self) {
        let documents = self.session.documents();
        if self.thumb_documents.len() == documents.len()
            && self.thumb_documents.iter().zip(documents).all(|((id, snapshot), st)| *id == st.doc.id && snapshot.ptr_eq(&st.doc.downgrade()))
        {
            return;
        }
        // Check snapshots every sync, but walk the layer tree only after it changes. A Weak
        // tracks replacement (including undo and preserved-ID reopen) without retaining pixels.
        if !self.thumbs.is_empty() {
            let mut live = std::collections::HashSet::new();
            let mut pending: Vec<_> = documents.iter().flat_map(|st| &st.doc.layers).collect();
            while let Some(layer) = pending.pop() {
                live.insert((layer.id, mask_thumbs_ui::THUMB_LAYER));
                if layer.mask.is_some() {
                    live.insert((layer.id, mask_thumbs_ui::THUMB_MASK));
                }
                if layer.vector_mask.is_some() {
                    live.insert((layer.id, mask_thumbs_ui::THUMB_VECTOR));
                }
                if let Some(children) = layer.children() {
                    pending.extend(children);
                }
            }
            // Native files can restore IDs: retain the union across all open documents, rather
            // than deleting an ID just because one document containing it was closed.
            self.thumbs.retain(|key, _| live.contains(key));
        }
        self.thumb_documents = documents.iter().map(|st| (st.doc.id, st.doc.downgrade())).collect();
    }

    pub fn set_theme(&mut self, ctx: &egui::Context, kind: theme::ThemeKind) {
        self.ui.theme = kind;
        theme::apply(ctx, kind);
        self.checker = None;
    }

    /// Cached 64px thumbnail of a pixel-ish layer, laid out in document space.
    pub fn layer_thumb(&mut self, ctx: &egui::Context, doc: &Document, layer: &photocraft_doc::Layer) -> Result<egui::TextureId, String> {
        if matches!(layer.content, photocraft_doc::LayerContent::BackedRaster(_)) {
            let error = "Native-backed layer thumbnail requires the checked CPU compositor".to_owned();
            self.ui.status = error.clone();
            self.ui.status_error = true;
            return Err(error);
        }
        // Key by content, not document revision: COW tiles change pointer only when their pixels
        // change, so unrelated edits (e.g. painting another layer) don't rebuild this thumbnail.
        let rev = layer.surface().map_or(0, surface_fingerprint) ^ (doc.size.width as u64) << 40;
        let key = (layer.id, mask_thumbs_ui::THUMB_LAYER);
        if let Some((r, tex)) = self.thumbs.get(&key)
            && *r == rev
        {
            return Ok(tex.id());
        }
        let mut px = [0.0f32; 8];
        let img = thumb_image(doc, 64, |x, y| {
            let Some(s) = layer.surface() else { return [0.0; 4] };
            let n = s.channels();
            s.read_pixel(x, y, &mut px[..n]);
            photocraft_raster::to_rgba(&s.format(), &px[..n])
        });
        Ok(self.store_thumb(ctx, key, rev, img))
    }

    pub fn mask_thumb(&mut self, ctx: &egui::Context, doc: &Document, id: photocraft_doc::LayerId, mask: &photocraft_doc::LayerMask) -> egui::TextureId {
        let rev = surface_fingerprint(&mask.surface) ^ (doc.size.width as u64) << 40;
        let key = (id, mask_thumbs_ui::THUMB_MASK);
        if let Some((r, tex)) = self.thumbs.get(&key)
            && *r == rev
        {
            return tex.id();
        }
        let mut v = [0.0f32; 1];
        let img = thumb_image(doc, 64, |x, y| {
            mask.surface.read_pixel(x, y, &mut v);
            [v[0], v[0], v[0], 1.0]
        });
        self.store_thumb(ctx, key, rev, img)
    }

    fn store_thumb(&mut self, ctx: &egui::Context, key: (photocraft_doc::LayerId, u8), rev: u64, img: egui::ColorImage) -> egui::TextureId {
        match self.thumbs.get_mut(&key) {
            Some((r, tex)) => {
                tex.set(img, egui::TextureOptions::LINEAR);
                *r = rev;
                tex.id()
            }
            None => {
                let tex = ctx.load_texture(format!("thumb-{}-{}", key.0.0, key.1), img, egui::TextureOptions::LINEAR);
                let id = tex.id();
                self.thumbs.insert(key, (rev, tex));
                id
            }
        }
    }
}

/// Cheap identity of a surface's pixels: tile coordinates and `Arc` pointers.
pub fn surface_fingerprint(s: &photocraft_raster::Surface) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ s.tile_count() as u64;
    for (c, t) in s.tiles() {
        let p = std::sync::Arc::as_ptr(t) as usize as u64;
        h = (h ^ p ^ ((c.tx as u64) << 32 | c.ty as u32 as u64)).wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Square thumbnail of the canvas area, letterboxed, sampling `f(x, y)` in document space.
fn thumb_image(doc: &Document, side: usize, mut f: impl FnMut(i32, i32) -> [f32; 4]) -> egui::ColorImage {
    let (w, h) = (doc.size.width.max(1) as f32, doc.size.height.max(1) as f32);
    let scale = w.max(h) / side as f32;
    let (ox, oy) = ((side as f32 - w / scale) / 2.0, (side as f32 - h / scale) / 2.0);
    let mut px = vec![egui::Color32::TRANSPARENT; side * side];
    for ty in 0..side {
        for tx in 0..side {
            let dx = (tx as f32 - ox + 0.5) * scale;
            let dy = (ty as f32 - oy + 0.5) * scale;
            if dx < 0.0 || dy < 0.0 || dx >= w || dy >= h {
                continue;
            }
            let c = f(dx as i32, dy as i32);
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            px[ty * side + tx] = egui::Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]));
        }
    }
    egui::ColorImage::new([side, side], px)
}

impl PhotocraftApp {
    /// Next step of queued synthetic input (automation): events up to and including the first
    /// release, so egui sees press and release in separate frames. Hosts that don't call
    /// `raw_input_hook` (offscreen harnesses) feed these to their input themselves.
    pub fn take_synthetic_step(&mut self) -> Vec<egui::Event> {
        let n = self
            .synthetic
            .iter()
            .position(|e| matches!(e, egui::Event::PointerButton { pressed: false, .. } | egui::Event::Key { pressed: false, .. }))
            .map_or(self.synthetic.len(), |i| i + 1);
        let events: Vec<_> = self.synthetic.drain(..n).collect();
        if !events.is_empty() {
            self.automation_input = true;
        }
        events
    }

    /// Install fonts, image loaders and the theme. Call from the app creator when possible so the
    /// very first frame renders; otherwise `logic` does it and the first frame is skipped.
    #[cfg(not(all(target_arch = "wasm32", feature = "web_external_assets")))]
    pub fn setup_context(ctx: &egui::Context, kind: theme::ThemeKind) {
        ctx.add_plugin(tool_cursor::CursorLifecycle);
        theme::install_fonts(ctx);
        Self::setup_context_after_fonts(ctx, kind);
    }
    #[cfg(all(target_arch = "wasm32", feature = "web_external_assets"))]
    pub fn setup_context_ready(ctx: &egui::Context, kind: theme::ThemeKind, ready: web_ui_assets::ReadyUiAssets) -> Result<(), web_ui_assets::UiAssetError> {
        ctx.add_plugin(tool_cursor::CursorLifecycle);
        theme::try_install_fonts_ready(ctx, ready)?;
        Self::setup_context_after_fonts(ctx, kind);
        Ok(())
    }
    #[cfg(not(all(target_arch = "wasm32", feature = "web_external_assets")))]
    fn try_setup_context(ctx: &egui::Context, kind: theme::ThemeKind) -> Result<(), web_ui_assets::UiAssetError> {
        ctx.add_plugin(tool_cursor::CursorLifecycle);
        theme::try_install_fonts(ctx)?;
        Self::setup_context_after_fonts(ctx, kind);
        Ok(())
    }
    fn setup_context_after_fonts(ctx: &egui::Context, kind: theme::ThemeKind) {
        egui_extras::install_image_loaders(ctx);
        theme::apply(ctx, kind);
        // egui's own ⌘+ / ⌘- / ⌘0 scale the whole interface; PhotoCraft zooms the canvas instead
        // (shortcuts.rs), like Photoshop.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        // ⌘/Ctrl + wheel scrolls sideways, as in Photoshop; only a pinch zooms (wheel_nav.rs).
        wheel_nav::configure(ctx);
    }
}

impl PhotocraftApp {
    /// Properties use current editable source geometry when it is qualified.
    pub fn cached_layer_bounds(&mut self, layer: &photocraft_doc::Layer) -> Option<photocraft_geom::Rect> {
        if let Some((doc, revision)) = self.session.active().map(|state| (state.doc.clone(), state.revision)) {
            return self.cached_document_layer_bounds(&doc, revision, layer);
        }
        layer.surface().map(|surface| self.cached_bounds(layer.id.0, surface))
    }

    /// Canvas views use their own document and revision, including secondary views.
    pub fn cached_document_layer_bounds(
        &mut self,
        doc: &photocraft_doc::Document,
        revision: u64,
        layer: &photocraft_doc::Layer,
    ) -> Option<photocraft_geom::Rect> {
        if let Some(source) = doc.xcf_state.as_ref().and_then(|xcf| {
            xcf.source_items.iter().find(|source| {
                source.owner == photocraft_doc::item_identity::ItemRef::Layer(layer.id)
                    && matches!(source.source, photocraft_doc::xcf_items::EditableSource::Text { .. })
            })
        }) {
            let key = (doc.id.0, revision, layer.id.0);
            if let Some(bounds) = self.source_bounds_cache.get(&key) {
                return *bounds;
            }
            let mut resident = 0_u64;
            let mut charge = |bytes| {
                resident =
                    resident.checked_add(bytes).ok_or(photocraft_compose::gimp_composite::CompositionError::Invalid("source layout resident overflow"))?;
                if resident > 8 << 20 {
                    return Err(photocraft_compose::gimp_composite::CompositionError::Invalid("source layout resident limit"));
                }
                Ok(())
            };
            let bounds = photocraft_compose::gimp_text_items::current_source_layout_bounds(doc, source, &photocraft_raster::Interrupt::NONE, &mut charge).ok();
            if self.source_bounds_cache.len() > 256 {
                self.source_bounds_cache.clear()
            }
            self.source_bounds_cache.insert(key, bounds);
            return bounds;
        }
        layer.surface().map(|surface| self.cached_bounds(layer.id.0, surface))
    }

    /// Cached `Surface::content_bounds` keyed by an id and the surface's tile identity.
    pub fn cached_bounds(&mut self, key: u64, surface: &photocraft_raster::Surface) -> photocraft_geom::Rect {
        let rev = surface_fingerprint(surface);
        if let Some((r, b)) = self.bounds_cache.get(&key)
            && *r == rev
        {
            return *b;
        }
        let b = surface.content_bounds();
        if self.bounds_cache.len() > 256 {
            self.bounds_cache.clear();
        }
        self.bounds_cache.insert(key, (rev, b));
        b
    }
}

impl PhotocraftApp {
    /// Zoom of the active document's main view (screen points per document pixel).
    pub fn current_zoom(&self) -> f32 {
        self.session.active_index().and_then(|i| self.ui.views.get(i)).map_or(1.0, |v| v.zoom)
    }
}

impl PhotocraftApp {
    /// Channels panel thumbnails of the active document snapshot: the composite,
    /// each colour channel (when there is more than one), each alpha channel, then the Quick Mask.
    pub fn channel_thumbs(&mut self, ctx: &egui::Context) -> Vec<egui::TextureId> {
        let Some(st) = self.session.active() else {
            self.channel_thumbs = None;
            return Vec::new();
        };
        let (id, doc) = (st.doc.id, st.doc.clone());
        // View-only commands bump revision without changing pixels. Keeping a Weak pins allocation
        // identity against address reuse without retaining the document's pixel data.
        let snapshot = doc.downgrade();
        if !matches!(&self.channel_thumbs, Some((d, old, _)) if *d == id && old.ptr_eq(&snapshot)) {
            let comp = match photocraft_compose::thumbnail(&doc, 56) {
                Ok(comp) => comp,
                Err(error) => {
                    self.ui.status = format!("Channel thumbnails: {error}");
                    return self.channel_thumbs.as_ref().map_or_else(Vec::new, |(_, _, textures)| textures.iter().map(|t| t.id()).collect());
                }
            };
            let (w, h) = (comp.width as usize, comp.height as usize);
            let side = w.max(h);
            let make = |f: &dyn Fn(&[u8]) -> egui::Color32, name: &str| {
                let mut px = vec![egui::Color32::TRANSPARENT; side * side];
                let (ox, oy) = ((side - w) / 2, (side - h) / 2);
                for y in 0..h {
                    for x in 0..w {
                        let o = (y * w + x) * 4;
                        px[(y + oy) * side + x + ox] = f(&comp.pixels[o..o + 4]);
                    }
                }
                ctx.load_texture(format!("chan-{name}"), egui::ColorImage::new([side, side], px), egui::TextureOptions::LINEAR)
            };
            let fmt = doc.pixel_format();
            let colors = fmt.mode.color_channels();
            let mut texs = vec![make(&|p| egui::Color32::from_rgb(p[0], p[1], p[2]), "composite")];
            if colors > 1 {
                for k in 0..colors {
                    let cmyk = fmt.mode == photocraft_doc::ColorMode::Cmyk;
                    texs.push(make(
                        &|p| {
                            let rgba = [p[0], p[1], p[2], 255].map(|v| f32::from(v) / 255.0);
                            let x = photocraft_raster::from_rgba(&fmt, rgba)[k];
                            let g = if cmyk { 1.0 - x } else { x };
                            egui::Color32::from_gray((g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
                        },
                        &format!("c{k}"),
                    ));
                }
            }
            // Alpha channels and the Quick Mask, sampled at thumbnail resolution.
            let (dw, dh) = (doc.size.width.max(1) as f32, doc.size.height.max(1) as f32);
            for (i, ch) in doc.channels.iter().chain(doc.quick_mask.as_ref()).enumerate() {
                let mut px = vec![egui::Color32::TRANSPARENT; side * side];
                let (ox, oy) = ((side - w) / 2, (side - h) / 2);
                for y in 0..h {
                    for x in 0..w {
                        let sx = ((x as f32 + 0.5) * dw / w as f32) as i32;
                        let sy = ((y as f32 + 0.5) * dh / h as f32) as i32;
                        let v = ch.surface.sample_channel(sx, sy, 0);
                        px[(y + oy) * side + x + ox] = egui::Color32::from_gray((v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                    }
                }
                texs.push(ctx.load_texture(format!("chan-a{i}"), egui::ColorImage::new([side, side], px), egui::TextureOptions::LINEAR));
            }
            self.channel_thumbs = Some((id, snapshot, texs));
        }
        self.channel_thumbs.as_ref().map(|(_, _, t)| t.iter().map(|t| t.id()).collect()).unwrap_or_default()
    }
}

fn clip_signature(w: u32, h: u32, px: &[u8]) -> u64 {
    let mut sig = (w as u64) << 32 | h as u64;
    for b in px.iter().step_by(997) {
        sig = sig.rotate_left(5) ^ *b as u64;
    }
    sig
}

impl PhotocraftApp {
    /// Mirror the session clipboard onto the OS clipboard (RGBA8).
    fn export_os_clipboard(&mut self) {
        let (Some(set), Some(clip)) = (self.services.clipboard_set_image.as_mut(), self.session.clipboard.as_ref()) else { return };
        let b = clip.bounds;
        if b.is_empty() {
            return;
        }
        let mut px = vec![[0u8; 4]; b.width() as usize * b.height() as usize];
        clip.surface.read_rgba8_into(b, &mut px);
        let bytes: Vec<u8> = px.into_iter().flatten().collect();
        if set(b.width(), b.height(), &bytes).is_ok() {
            self.os_clip_sig = Some(clip_signature(b.width(), b.height(), &bytes));
        }
    }

    /// File › New's fields: the defaults, plus the Clipboard preset (the clipboard image's size,
    /// selected) when the clipboard holds an image. Opening the dialog is an explicit request, so
    /// the OS clipboard is read here, as for a paste.
    pub(crate) fn new_document_fields(&mut self) -> serde_json::Map<String, serde_json::Value> {
        let mut f = crate::state::UiState::new_document_fields();
        self.import_os_clipboard();
        if let Some(c) = self.session.clipboard.as_ref().filter(|c| !c.bounds.is_empty()) {
            crate::new_doc_ui::set_clipboard(&mut f, c.bounds.width(), c.bounds.height());
        }
        f
    }

    /// If the OS clipboard holds an image that isn't the one we put there, make it the session
    /// clipboard (so ⌘V pastes screenshots and images copied in other apps, like Photoshop).
    /// Returns true when a new external image was imported.
    pub(crate) fn import_os_clipboard(&mut self) -> bool {
        let Some(get) = self.services.clipboard_get_image.as_mut() else { return false };
        let Some((w, h, bytes)) = get() else { return false };
        let Some(expected) = u64::from(w).checked_mul(u64::from(h)).and_then(|n| n.checked_mul(4)).and_then(|n| usize::try_from(n).ok()) else { return false };
        if w == 0 || h == 0 || bytes.len() != expected {
            return false;
        }
        let sig = clip_signature(w, h, &bytes);
        if self.os_clip_sig == Some(sig) && self.session.clipboard.is_some() {
            return false;
        }
        let (Ok(width), Ok(height)) = (i32::try_from(w), i32::try_from(h)) else { return false };
        let r = photocraft_geom::Rect::new(0, 0, width, height);
        let Ok(mut surface) = photocraft_raster::Surface::try_from_interleaved(photocraft_color::PixelFormat::RGBA8, r, &bytes) else { return false };
        surface.prune();
        let Ok(clip) = photocraft_engine::edit_cmds::Clip::from_external_srgb(surface, r) else { return false };
        self.session.clipboard = Some(clip);
        self.os_clip_sig = Some(sig);
        self.clip_external = true;
        true
    }
}

#[cfg(test)]
mod input_tests;

#[cfg(test)]
mod pencil_tests;

#[cfg(test)]
mod transform_undo_tests;

#[cfg(test)]
mod move_auto_select_tests;

#[cfg(test)]
mod hidden_layer_tests;

#[cfg(test)]
mod blend_dropdown_keys_tests;

#[cfg(test)]
mod marquee_tests;

#[cfg(test)]
mod stamp_tests;

#[cfg(test)]
mod polygon_lasso_tests;

#[cfg(test)]
mod clipboard_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn os_clipboard_bridge() {
        let os: OsClip = Arc::default();
        let (a, b) = (os.clone(), os.clone());
        let services = Services {
            clipboard_set_image: Some(Box::new(move |w: u32, h: u32, px: &[u8]| {
                *a.lock().unwrap() = Some((w, h, px.to_vec()));
                Ok(())
            })),
            clipboard_get_image: Some(Box::new(move || b.lock().unwrap().clone())),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(Session::new(), services);
        app.session.execute("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
        app.sync_views();
        app.run("select.rect", serde_json::json!({"x": 0, "y": 0, "width": 8, "height": 4})).unwrap();
        app.run("edit.copy", serde_json::json!({})).unwrap();
        let (w, h, px) = os.lock().unwrap().clone().unwrap();
        assert_eq!((w, h, px.len()), (8, 4, 8 * 4 * 4));
        // Our own image comes back unchanged (no re-import, keeps the original position).
        assert!(!app.import_os_clipboard());
        // Another app puts a 3×2 red image on the clipboard: ⌘V pastes it.
        *os.lock().unwrap() = Some((3, 2, [255u8, 0, 0, 255].repeat(6)));
        app.run("edit.paste", serde_json::json!({})).unwrap();
        let st = app.session.active().unwrap();
        let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        assert_eq!(surf.content_bounds().width(), 3);
    }

    type OsClip = Arc<Mutex<Option<(u32, u32, Vec<u8>)>>>;

    /// An app whose OS clipboard is `os`, counting every read in `reads`.
    fn app_with_os_clipboard(os: &OsClip, reads: &Arc<std::sync::atomic::AtomicUsize>) -> PhotocraftApp {
        let (b, n) = (os.clone(), reads.clone());
        let get = move || {
            n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            b.lock().unwrap().clone()
        };
        let mut app = PhotocraftApp::new(Session::new(), Services { clipboard_get_image: Some(Box::new(get)), ..Default::default() });
        app.session.execute("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
        app.sync_views();
        app
    }

    #[test]
    fn external_clipboard_enables_and_centres_paste() {
        // Paste is enabled from the clipboard service alone (no read); the explicit paste imports
        // the OS image, once, and centres it like Photoshop does for foreign clipboard content.
        let (os, reads) = (OsClip::default(), Arc::default());
        let mut app = app_with_os_clipboard(&os, &reads);
        *os.lock().unwrap() = Some((4, 4, [255u8, 0, 0, 255].repeat(16)));
        assert!(app.session.clipboard.is_none());
        assert!(crate::menus::is_enabled(&app, "edit.paste"));
        assert!(crate::menus::is_enabled(&app, "edit.pasteSpecial.pasteInPlace"));
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0, "enablement never reads the clipboard");
        let r = crate::menus::invoke(&mut app, &egui::Context::default(), "edit.paste", serde_json::json!({})).unwrap();
        assert_ne!(r["offset"], serde_json::json!([0, 0]), "external image is centred: {r}");
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1, "one read per explicit paste");
        // Without a clipboard service Paste greys until something is copied in the app.
        let mut plain = PhotocraftApp::new(Session::new(), Services::default());
        plain.session.execute("file.new", serde_json::json!({"width": 8, "height": 8})).unwrap();
        assert!(!crate::menus::is_enabled(&plain, "edit.paste"));
    }

    /// #368: an image copied in another app opens as a document of its own, from File › New
    /// from Clipboard or from Paste with nothing open.
    #[test]
    fn os_clipboard_image_becomes_a_new_document() {
        let (os, reads) = (OsClip::default(), Arc::new(std::sync::atomic::AtomicUsize::new(0)));
        let (b, n) = (os.clone(), Arc::clone(&reads));
        let get = move || {
            n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            b.lock().unwrap().clone()
        };
        let mut app = PhotocraftApp::new(Session::new(), Services { clipboard_get_image: Some(Box::new(get)), ..Default::default() });
        let ctx = egui::Context::default();
        // Listed right after File › New…, enabled without reading the clipboard.
        let items = crate::menus::menu_items(&app);
        let at = items.iter().position(|i| i.id == "file.new").unwrap();
        assert_eq!(items[at + 1].id, "file.newFromClipboard");
        assert!(items[at + 1].enabled && crate::menus::is_enabled(&app, "edit.paste"));
        assert!(!crate::menus::is_enabled(&app, "edit.pasteSpecial.pasteInPlace"), "Paste in Place needs a document");
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0);
        // An empty clipboard is a quiet no-op.
        let r = crate::menus::invoke(&mut app, &ctx, "file.newFromClipboard", serde_json::json!({})).unwrap();
        assert_eq!(r["pasted"], serde_json::json!(false));
        assert!(app.session.documents().is_empty() && !app.ui.status_error);
        // Paste with nothing open makes the document.
        *os.lock().unwrap() = Some((5, 3, [0u8, 0, 255, 255].repeat(15)));
        crate::menus::invoke(&mut app, &ctx, "edit.paste", serde_json::json!({})).unwrap();
        let d = &app.session.active().unwrap().doc;
        assert_eq!((d.size.width, d.size.height, d.layers.len()), (5, 3, 1));
        assert_eq!(app.ui.views.len(), 1, "the new document has a view");
        // New from Clipboard with a document open adds another.
        crate::menus::invoke(&mut app, &ctx, "file.newFromClipboard", serde_json::json!({})).unwrap();
        assert_eq!(app.session.documents().len(), 2);
    }

    #[test]
    fn pasting_an_empty_or_non_image_os_clipboard_is_a_quiet_no_op() {
        let (os, reads) = (OsClip::default(), Arc::default());
        let mut app = app_with_os_clipboard(&os, &reads);
        let layers = app.session.active().unwrap().doc.layers.len();
        let r = crate::menus::invoke(&mut app, &egui::Context::default(), "edit.paste", serde_json::json!({})).unwrap();
        assert_eq!(r["pasted"], serde_json::json!(false));
        assert_eq!(app.run("edit.pasteSpecial.pasteInPlace", serde_json::json!({})).unwrap()["pasted"], serde_json::json!(false));
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers);
        assert!(!app.ui.status_error && !app.ui.status.is_empty());
    }

    #[test]
    fn idle_frames_never_read_the_os_clipboard() {
        let (os, reads) = (OsClip::default(), Arc::<std::sync::atomic::AtomicUsize>::default());
        *os.lock().unwrap() = Some((4, 4, [255u8, 0, 0, 255].repeat(16)));
        let (b, n) = (os.clone(), reads.clone());
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let get = move || {
                n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                b.lock().unwrap().clone()
            };
            let mut app = PhotocraftApp::new(Session::new(), Services { clipboard_get_image: Some(Box::new(get)), ..Default::default() });
            app.session.execute("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
            app
        });
        for _ in 0..20 {
            h.step();
        }
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0, "the OS clipboard is read only on an explicit paste");
        assert!(h.state().session.clipboard.is_none());
    }

    /// #1035: with the layer mask targeted (its thumbnail clicked), ⌘V pastes the clipboard's
    /// luminosity into the mask instead of making a layer.
    #[test]
    fn paste_with_the_mask_targeted_goes_into_the_mask() {
        let (os, reads) = (OsClip::default(), Arc::default());
        let mut app = app_with_os_clipboard(&os, &reads);
        app.run("layer.new.layer", serde_json::json!({})).unwrap();
        app.run("layer.layerMask.hideAll", serde_json::json!({})).unwrap();
        app.ui.mask_target = true;
        app.ui.views[0].center = [32.0, 32.0];
        *os.lock().unwrap() = Some((4, 4, [128u8, 128, 128, 255].repeat(16)));
        let layers = app.session.active().unwrap().doc.layer_count();
        crate::menus::invoke(&mut app, &egui::Context::default(), "edit.paste", serde_json::json!({})).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layer_count(), layers, "no new layer");
        let mask = st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap();
        assert!((mask.value(32, 32) - 128.0 / 255.0).abs() < 2.0 / 255.0, "the grey, centred in the view: {}", mask.value(32, 32));
        assert_eq!(mask.value(0, 0), 0.0, "the rest of the mask is unchanged");
    }
}
