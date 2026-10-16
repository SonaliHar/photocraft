//! Filter › Camera Raw Filter…: a large dialog like Adobe Camera Raw's (preview on the left,
//! edit panels on the right: Light, Color, Effects, Curve, Color Mixer, Color Grading, Detail).
//!
//! The preview runs [`photocraft_algo::camera_raw::develop`] on a CPU proxy of the layer
//! (≤ 900 px, `pixel_scale` keeps pixel radii true to the full image), recomputed when a
//! control changes. OK runs `filter.cameraRaw` with the non-default settings, so the result is
//! one history step (or a smart filter on a smart object) and exactly replayable.
//!
//! Control channel: `ui.menu.invoke {"id":"filter.cameraRaw","params":{"ui":{"set":{…},
//! "commit":true | "cancel":true}}}`; the reply describes the dialog. `{"ui":{"saveSettings":true}}`
//! and `{"ui":{"loadSettings":true}}` run Save Settings… / Load Settings… (Camera Raw `.xmp`
//! settings files, see [`photocraft_io::camera_raw_xmp`]) through the file dialog.

use egui::{Align2, Color32, FontId, Pos2, Rect as ERect, Sense, Stroke, TextureHandle, pos2, vec2};
use photocraft_algo::camera_raw::{CameraRaw, curve_lut};
use photocraft_algo::histogram::RgbHistogram;

use crate::camera_raw_scope_ui::{MAX_SAMPLERS, ToneZone};
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::widgets;

#[path = "camera_raw_bars_ui.rs"]
mod bars;

const PROXY_SIDE: usize = 900;

/// How long a swap to the next photo may take before the dialog shows that it is busy.
const BUSY_AFTER: std::time::Duration = std::time::Duration::from_millis(250);

/// The preview proxy of a photo, built ahead of the dialog on a worker thread (see [`proxy_for`]). Averaging a 45 MP photo
/// down to 900 px takes a few hundred milliseconds even spread over every core; on the editor's own thread that freezes
/// the whole window at each photo.
#[derive(Clone)]
pub(crate) struct ProxyData {
    area: Rect,
    k: usize,
    pw: usize,
    ph: usize,
    pixels: std::sync::Arc<Vec<[f32; 4]>>,
    /// The raw's default tone curve, when the worker that built this also worked it out (see `raw_workspace::Browsed`).
    curve: Option<Vec<[f32; 2]>>,
}

impl ProxyData {
    pub(crate) fn with_curve(mut self, curve: Vec<[f32; 2]>) -> Self {
        self.curve = Some(curve);
        self
    }
}

impl std::fmt::Debug for ProxyData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ProxyData({}x{} of {:?}, 1/{})", self.pw, self.ph, self.area, self.k)
    }
}

/// Where a layer's proxy lives and how big it is.
struct ProxyGeometry {
    area: Rect,
    w: usize,
    h: usize,
    k: usize,
    pw: usize,
    ph: usize,
}

fn proxy_geometry(canvas: Rect, surf: &photocraft_raster::Surface) -> Result<ProxyGeometry, String> {
    // Use the engine filter domain, including pixels outside the canvas: spatial Camera Raw
    // stages (vignette, grain, local contrast) must agree with the committed result.
    let area = canvas.union(&surf.content_bounds());
    let width = area.x1.checked_sub(area.x0).filter(|n| (1..=1_048_576).contains(n));
    let height = area.y1.checked_sub(area.y0).filter(|n| (1..=1_048_576).contains(n));
    let (Some(w), Some(h)) = (width, height) else {
        return Err("Camera Raw preview bounds are too large".into());
    };
    let (w, h) = (w as usize, h as usize);
    if w.checked_mul(h).is_none_or(|pixels| pixels > 512_000_000) {
        return Err("Camera Raw preview bounds are too large".into());
    }
    let k = w.max(h).div_ceil(PROXY_SIDE).max(1);
    let (pw, ph) = (w.div_ceil(k), h.div_ceil(k));
    Ok(ProxyGeometry { area, w, h, k, pw, ph })
}

fn proxy_pixels(surf: &photocraft_raster::Surface, g: &ProxyGeometry) -> Vec<[f32; 4]> {
    let ProxyGeometry { area, w, h, k, pw, ph } = *g;
    let mut proxy = vec![[0.0f32; 4]; pw * ph];
    // One proxy row: the box average of its k source rows. Rows are independent, so they are built in
    // parallel (a 45 MP photo's proxy took 2.5-4 s on one core, with the whole editor frozen meanwhile).
    let build_row = |py: usize, out: &mut [[f32; 4]]| {
        let mut row = vec![[0.0f32; 4]; w];
        let mut acc = vec![[0.0f32; 5]; pw];
        for dy in 0..k {
            let y = area.y0 + (py * k + dy).min(h - 1) as i32;
            surf.read_rgba_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
            for (x, q) in row.iter().enumerate() {
                let a = &mut acc[x / k];
                for c in 0..4 {
                    a[c] += q[c];
                }
                a[4] += 1.0;
            }
        }
        for (o, a) in out.iter_mut().zip(&acc) {
            *o = [a[0] / a[4], a[1] / a[4], a[2] / a[4], a[3] / a[4]];
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        proxy.par_chunks_mut(pw).enumerate().for_each(|(py, out)| build_row(py, out));
    }
    #[cfg(target_arch = "wasm32")]
    for (py, out) in proxy.chunks_mut(pw).enumerate() {
        build_row(py, out);
    }
    proxy
}

/// The proxy of a freshly developed raw's pixel layer, for a worker thread to build before the dialog needs it. `None`
/// when the document has no raster layer or it is too large; the dialog then builds it itself, as before.
pub(crate) fn proxy_for(document: &photocraft_doc::Document) -> Option<ProxyData> {
    let surf = document.walk().into_iter().find_map(|(_, _, layer)| match &layer.content {
        photocraft_doc::LayerContent::Raster(surface) => Some(surface.clone()),
        _ => None,
    })?;
    let geometry = proxy_geometry(document.bounds(), &surf).ok()?;
    let pixels = proxy_pixels(&surf, &geometry);
    Some(ProxyData { area: geometry.area, k: geometry.k, pw: geometry.pw, ph: geometry.ph, pixels: std::sync::Arc::new(pixels), curve: None })
}
const BANDS: [&str; 8] = ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"];

pub struct CameraRawDialog {
    pub layer: LayerId,
    layer_name: String,
    pub params: CameraRaw,
    /// Settings when the dialog opened (stored settings of an edited smart filter).
    original: CameraRaw,
    target: Target,
    curve_state: crate::state::PointCurveState,
    curve_rect: Option<ERect>,
    /// Proxy pixels (straight RGBA) and size.
    pub(crate) proxy: Vec<[f32; 4]>,
    pub(crate) selection: Option<Vec<f32>>,
    pub(crate) pw: usize,
    pub(crate) ph: usize,
    full_w: usize,
    full_h: usize,
    preview_ppp: f32,
    pub(crate) detail: super::camera_raw_detail_ui::DetailPreview,
    display: std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>,
    pub(crate) viewport: Option<ERect>,
    pub(crate) float: bool,
    tex: Option<TextureHandle>,
    before_tex: Option<TextureHandle>,
    before_display_key: Option<u64>,
    pub(crate) before_histogram: RgbHistogram,
    pub(crate) processed: Vec<[f32; 4]>,
    pub(crate) lab_transform: std::sync::Arc<photocraft_cms::Transform>,
    pub(crate) scope_transform: std::sync::Arc<photocraft_cms::Transform>,
    pub(crate) scope: super::camera_raw_scope_ui::ScopeView,
    pub(crate) after_histogram: RgbHistogram,
    pub(crate) preview_revision: u64,
    histogram_ms: f64,
    pub(crate) dirty: bool,
    pub show_before: bool,
    camera_info: photocraft_algo::exif::CameraInfo,
    document_name: String,
    pub render_ms: f64,
    /// Set for the open-time dialog ([`Target::OpenRaw`]).
    raw_open: Option<RawOpen>,
    /// The photo was left for the next one, which has not taken the dialog over yet.
    busy_since: Option<std::time::Instant>,
    /// The open-time dialog's camera tone curve: Shadows / Highlights act on the raw data there,
    /// in linear light before this curve, and the preview undoes it to show the same.
    raw_curve: Option<Vec<[f32; 2]>>,
    /// Settings before each finished edit, so Ctrl+Z works inside the dialog (#4100).
    undo: Vec<CameraRaw>,
    redo: Vec<CameraRaw>,
    /// Params at pointer-down. One drag is one undo step.
    edit_origin: Option<CameraRaw>,
}

impl CameraRawDialog {
    /// Whether the photo was left for the next one, which has not taken over yet.
    pub(crate) fn busy(&self) -> bool {
        self.busy_since.is_some()
    }

    /// Whether the document under the dialog is a reduced-resolution stand-in for the photo.
    pub(crate) fn is_stand_in(&self) -> bool {
        self.raw_open.as_ref().is_some_and(|raw| raw.full_size.is_some())
    }

    pub fn describe(
        &self,
        view: &crate::state::CameraRawScopeState,
        navigation: &crate::state::CameraRawPreviewState,
        workspace: &crate::camera_raw_workspace_ui::WorkspaceState,
    ) -> Value {
        let h = self.histogram();
        let mut result = json!({"layer": self.layer.0, "params": serde_json::to_value(&self.params).unwrap_or(Value::Null), "proxy": [self.pw, self.ph],
            "renderMs": self.render_ms, "histogramMs": self.histogram_ms, "previewRevision": self.preview_revision, "before": self.show_before,
            "histogram": {"source": if self.show_before { "before" } else { "after" }, "approximate": true, "size": [self.pw, self.ph],
                "sampleDomain": "preview-rgb", "range": [0, 1], "red": h.channels[0].as_slice(), "green": h.channels[1].as_slice(), "blue": h.channels[2].as_slice(),
                "samples": h.samples, "transparent": h.transparent, "invalid": h.invalid, "underflow": h.underflow, "overflow": h.overflow, "shadows": h.shadows, "highlights": h.highlights}});
        result["smartFilter"] = json!(match self.target {
            Target::SmartFilter(index) => Some(index),
            Target::NewFilter | Target::OpenRaw => None,
        });
        result["openingRaw"] = json!(self.raw_open.as_ref().map(|r| json!({"name": r.name, "path": r.path})));
        result["view"] = json!(navigation);
        result["workspace"] = json!(workspace);
        result["sourceSize"] = json!([self.full_w, self.full_h]);
        result["previewApproximate"] = json!(!self.detail.ready && (self.pw != self.full_w || self.ph != self.full_h));
        result["detailPending"] = json!(self.detail.pending());
        result["detailError"] = json!(self.detail.error);
        result["viewportRect"] = json!(self.viewport.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["zoom"] = json!(self.viewport.map(|r| navigation.scale(r, vec2(self.full_w as f32, self.full_h as f32) / self.preview_ppp)));
        result["scope"] = serde_json::to_value(view).unwrap_or(Value::Null);
        result["hasScopeSelection"] = json!(self.selection.is_some());
        result["vectorscope"] = json!(
            self.scope
                .cache
                .as_ref()
                .map(|(_, before, selected, data)| json!({"samples":data.samples,"before":before,"selectedRegion":selected,"bins":data.counts}))
        );
        result["scopeMs"] = json!(self.scope.ms);
        result["scopeRevision"] = json!(self.scope.revision);
        result["overlayRevision"] = json!(self.scope.overlay_key.map(|k| k.0));
        result["pointerReadout"] = self.scope.pointer_sample.and_then(|p| self.readout(p, view.lab)).unwrap_or(Value::Null);
        result["samplerReadouts"] = json!(view.samplers.iter().take(MAX_SAMPLERS).map(|p| self.readout(*p, view.lab)).collect::<Vec<_>>());
        result["hoveredZone"] = json!(self.scope.hovered_zone);
        result["previewRect"] = json!(self.scope.preview_rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["curveState"] = json!(self.curve_state);
        result["curveRect"] = json!(self.curve_rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["hoverSample"] = json!(self.hover_sample(view.vectorscope));
        result["vectorscopeRect"] = json!(self.scope.vectorscope_rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["scopeRect"] = json!(self.scope.rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result
    }

    pub(crate) fn histogram(&self) -> &RgbHistogram {
        if self.show_before { &self.before_histogram } else { &self.after_histogram }
    }

    /// Params for the engine: only the settings that differ from the defaults.
    pub fn command_params(&self) -> Value {
        non_defaults(&self.params)
    }

    fn image(px: &[[f32; 4]], w: usize, h: usize, display: &photocraft_engine::display_color::CanvasDisplay) -> egui::ColorImage {
        // Bands copied straight from `px`, converted in parallel (#4682), not a whole copy of the
        // proxy converted on one thread.
        super::camera_raw_detail_ui::display_image(Rect::new(0, 0, w as i32, h as i32), display, |r, out| {
            let start = (r.y0.max(0) as usize).saturating_mul(w);
            if let Some(src) = px.get(start..start.saturating_add(out.len())) {
                out.copy_from_slice(src);
            }
        })
    }

    pub(crate) fn render(&mut self, ctx: &egui::Context) {
        let t0 = crate::gpu_canvas::now_ms();
        let mut px = self.proxy.clone();
        let scale = (self.pw as f32 / self.full_w.max(1) as f32).min(1.0);
        crate::raw_recipe::apply(&mut px, self.pw, self.ph, &self.params, self.raw_curve.as_deref(), scale, self.float);
        if let Some(mask) = &self.selection {
            for ((out, original), coverage) in px.iter_mut().zip(&self.proxy).zip(mask) {
                for (out, original) in out.iter_mut().zip(original) {
                    *out = *original + (*out - *original) * coverage;
                }
            }
        }
        let histogram_start = crate::gpu_canvas::now_ms();
        self.after_histogram = RgbHistogram::from_rgba(&px);
        self.histogram_ms = crate::gpu_canvas::now_ms() - histogram_start;
        let img = Self::image(&px, self.pw, self.ph, &self.display);
        match &mut self.tex {
            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
            None => self.tex = Some(ctx.load_texture("camera-raw-after", img, egui::TextureOptions::LINEAR)),
        }
        if self.before_display_key != Some(self.display.key) {
            let before = Self::image(&self.proxy, self.pw, self.ph, &self.display);
            match &mut self.before_tex {
                Some(t) => t.set(before, egui::TextureOptions::LINEAR),
                None => self.before_tex = Some(ctx.load_texture("camera-raw-before", before, egui::TextureOptions::LINEAR)),
            }
            self.before_display_key = Some(self.display.key);
        }
        self.processed = px;
        self.scope.cache = None;
        self.render_ms = crate::gpu_canvas::now_ms() - t0;
        self.preview_revision = self.preview_revision.saturating_add(1);
        self.dirty = false;
    }
}

/// Opens the dialog on the active layer.
/// Where OK writes: a new filter on the layer, or an existing smart filter (by index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    NewFilter,
    SmartFilter(usize),
    /// The open-time dialog of a raw file just opened (File › Open): OK is **Open**, Cancel
    /// closes the document again. See [`RawOpen`].
    OpenRaw,
}

/// A raw file opened interactively, shown in Camera Raw before it is kept (as Photoshop opens
/// raws in Adobe Camera Raw first). The document is already developed with the defaults; Open
/// re-develops it when Temperature, Tint, Exposure, Shadows, Highlights, Clarity, Texture or
/// Dehaze changed (those act on the sensor data, see [`photocraft_io::raw::import_raw_tuned`])
/// and applies the other settings as one Camera Raw step, then marks the document unmodified.
/// Cancel closes it.
#[derive(Clone, Debug)]
pub struct RawOpen {
    pub document: photocraft_doc::DocId,
    pub name: String,
    pub path: Option<String>,
    /// The file, when it was opened from bytes (no path to read it again from).
    pub bytes: Option<std::sync::Arc<Vec<u8>>>,
    /// Camera Raw workspace: the photo's stored recipe, which the dialog opens with.
    pub initial: Option<CameraRaw>,
    /// Camera Raw workspace: where the recipe and its render are kept.
    pub targets: crate::raw_recipe::Targets,
    /// Camera Raw workspace: "Open" continues this saved document (the photo's full-editor project)
    /// instead of developing a new one, so earlier work in the full editor is never replaced.
    pub continue_from: Option<String>,
    /// The preview proxy, built ahead on a worker thread; the dialog builds its own when this is missing.
    pub(crate) proxy: Option<ProxyData>,
    /// Set when the document is a reduced-resolution stand-in made for browsing: the size of the full development it stands
    /// in for. Open then develops the photo in full, whatever was adjusted.
    pub(crate) full_size: Option<(u32, u32)>,
}

/// Should an interactive open of `name` show the open-time Camera Raw dialog? `warnings` are
/// the import's: only a raw developed from its sensor data qualifies (not a preview fallback).
pub fn wants_open_dialog(app: &PhotocraftApp, warnings: &[String]) -> bool {
    app.session.prefs().raw_defaults.open_in_camera_raw && warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE))
}

/// `warnings` without the "developed with default settings" note: the open-time dialog shows
/// how the raw is developed.
pub fn without_develop_note(warnings: &[String]) -> Vec<String> {
    warnings.iter().filter(|w| !w.contains(photocraft_io::raw::DEVELOPED_NOTE)).cloned().collect()
}

/// Queues the open-time dialog for the active document (shown on the next frame).
pub fn queue_open_dialog(app: &mut PhotocraftApp, name: &str, path: Option<&str>, bytes: Option<&[u8]>) {
    if let Some(st) = app.session.active() {
        app.pending_raw_open = Some(RawOpen {
            document: st.doc.id,
            name: name.to_string(),
            path: path.map(str::to_string),
            bytes: bytes.map(|b| std::sync::Arc::new(b.to_vec())),
            initial: None,
            targets: Default::default(),
            continue_from: None,
            proxy: None,
            full_size: None,
        });
    }
}

/// Opens the queued open-time dialog on its document's (only) layer.
pub(crate) fn open_raw(app: &mut PhotocraftApp, ctx: &egui::Context, mut raw: RawOpen) -> Result<(), String> {
    let prepared = raw.proxy.take();
    let full_size = raw.full_size;
    let index = document_index(app, raw.document).ok_or("the raw document was closed")?;
    app.session.set_active(index);
    let (layer, surf, _) = crate::distort_ui::active_pixels(app)?;
    let initial = raw.initial.clone().unwrap_or_default();
    // The worker that developed the photo for browsing worked the curve out too (decoding the raw again here, on the
    // editor's own thread, held every frame and control answer for half a second on a 45 MP file).
    let prepared_curve = prepared.as_ref().and_then(|p| p.curve.clone());
    open_pixels(app, ctx, layer, surf, initial, OpenAs { coverage: Coverage::None, target: Target::OpenRaw, prepared, full_size })?;
    let curve = match (prepared_curve, &raw.bytes, &raw.path) {
        (Some(curve), _, _) => Some(curve),
        (None, Some(b), _) => Some(photocraft_io::raw::tone_curve(b)),
        #[cfg(not(target_arch = "wasm32"))]
        (None, None, Some(p)) => std::fs::read(p).ok().map(|b| photocraft_io::raw::tone_curve(&b)),
        _ => None,
    };
    if let Some(d) = app.camera_raw.as_mut() {
        d.layer_name = raw.name.clone();
        d.raw_curve = curve;
        d.raw_open = Some(raw);
    }
    Ok(())
}

fn document_index(app: &PhotocraftApp, id: photocraft_doc::DocId) -> Option<usize> {
    app.session.documents().iter().position(|st| st.doc.id == id)
}

/// The non-default settings of `p` as engine params.
pub(crate) fn non_defaults(p: &CameraRaw) -> Value {
    let full = serde_json::to_value(p).unwrap_or(json!({}));
    let def = serde_json::to_value(CameraRaw::default()).unwrap_or(json!({}));
    let mut out = serde_json::Map::new();
    if let (Value::Object(f), Value::Object(d)) = (full, def) {
        for (k, v) in f {
            if k != "pixelScale" && d.get(&k) != Some(&v) {
                out.insert(k, v);
            }
        }
    }
    Value::Object(out)
}

/// The job id of an open-time re-develop (not an engine command: the shell starts it).
pub const REDEVELOP_JOB: &str = "cameraRaw.redevelop";

/// An open-time re-develop running in the background: what to finish with when it lands.
#[derive(Clone, Debug)]
pub struct Redevelop {
    pub job: photocraft_engine::jobs::JobId,
    params: CameraRaw,
    raw: RawOpen,
    /// The job is the final Camera Raw step (`filter.cameraRaw`), not the re-develop: when it
    /// lands the document is marked unmodified.
    filter_step: bool,
}

/// The sliders a raw open applies while developing, before the tone curve (as Camera Raw does).
pub(crate) fn raw_stage(p: &CameraRaw) -> photocraft_io::raw::Tone {
    photocraft_io::raw::Tone { shadows: p.shadows, highlights: p.highlights, clarity: p.clarity, texture: p.texture, dehaze: p.dehaze }
}

/// Open: re-develop for raw-stage settings, apply the rest, keep the document unmodified.
///
/// Re-developing reads and decodes the whole raw again, so in the desktop app (background jobs
/// on) it runs as a job like the open itself: the reply is `{"job", "pending": true}`, the
/// document is locked meanwhile, and [`on_redevelop_event`] finishes when it lands. Elsewhere
/// (tests, the web) it runs inline.
fn commit_open_raw(app: &mut PhotocraftApp, params: &CameraRaw, raw: &RawOpen) -> Result<Value, String> {
    let index = document_index(app, raw.document).ok_or("the raw document was closed")?;
    let tuning = crate::raw_recipe::tuning(params);
    // Without the file the raw-stage settings stay RGB adjustments, like the filter's.
    let source = match (&raw.bytes, &raw.path) {
        (Some(b), _) => Some(photocraft_engine::jobs::OpenSource::Bytes(b.clone())),
        #[cfg(not(target_arch = "wasm32"))]
        (None, Some(p)) => Some(photocraft_engine::jobs::OpenSource::Path(p.clone())),
        _ => None,
    };
    // A reduced stand-in is never the photo that gets edited: Open develops the real one (with the adjustments that act on the
    // sensor data applied while doing so), whatever was changed.
    let stand_in = raw.full_size.is_some();
    let Some(source) = source.filter(|_| stand_in || tuning != photocraft_io::raw::RawTuning::default()) else {
        return finish_open_raw(app, index, params.clone(), None, Some(raw));
    };
    app.session.set_active(index);
    let (name, doc_id) = (raw.name.clone(), raw.document);
    let work = move |ctx: &photocraft_engine::jobs::JobCtx| -> photocraft_engine::Result<(photocraft_io::ImportResult, bool)> {
        ctx.progress(0.0, "Reading");
        let bytes = match source {
            photocraft_engine::jobs::OpenSource::Bytes(b) => b,
            #[cfg(not(target_arch = "wasm32"))]
            photocraft_engine::jobs::OpenSource::Path(p) => {
                std::sync::Arc::new(std::fs::read(&p).map_err(|e| photocraft_engine::EngineError::Other(format!("{p}: {e}")))?)
            }
            #[cfg(target_arch = "wasm32")]
            photocraft_engine::jobs::OpenSource::Path(p) => return Err(photocraft_engine::EngineError::Other(format!("{p}: cannot read files here"))),
        };
        ctx.check()?;
        ctx.progress(0.05, "Developing");
        photocraft_io::raw::import_raw_tuned(&name, &bytes, &tuning).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
    };
    let apply = move |s: &mut photocraft_engine::Session, (r, wb): (photocraft_io::ImportResult, bool)| replace_raw_document(s, doc_id, r, wb);
    let label = crate::i18n::fmt(tl!("Developing {name}"), &[("name", &raw.name)]);
    let started = if app.background_jobs {
        app.session.start_job(REDEVELOP_JOB, json!({"name": raw.name}), &label, true, work, apply).map_err(|e| e.to_string())?
    } else {
        let v = work(&photocraft_engine::jobs::JobCtx::new()).and_then(|t| apply(&mut app.session, t)).map_err(|e| e.to_string())?;
        photocraft_engine::jobs::Started::Done(v)
    };
    match started {
        photocraft_engine::jobs::Started::Done(v) => finish_redevelop(app, params, raw, &v),
        photocraft_engine::jobs::Started::Job(job) => {
            app.raw_redevelop = Some(Redevelop { job, params: params.clone(), raw: raw.clone(), filter_step: false });
            app.ui.status = label;
            app.ui.status_error = false;
            Ok(json!({"job": job.0, "pending": true}))
        }
    }
}

/// Swap the default-developed document `doc_id` for the re-developed one, at the same tab
/// position and with the same path. Returns `{document, previous, warnings, wb}`.
fn replace_raw_document(
    s: &mut photocraft_engine::Session,
    doc_id: photocraft_doc::DocId,
    r: photocraft_io::ImportResult,
    wb: bool,
) -> photocraft_engine::Result<Value> {
    let index =
        s.documents().iter().position(|st| st.doc.id == doc_id).ok_or_else(|| photocraft_engine::EngineError::Other("the raw document was closed".into()))?;
    let path = s.documents().get(index).and_then(|st| st.path.clone());
    s.close(index);
    let (opened, _) = s.open_document(r.document, path);
    let index = match s.execute("document.move", json!({"document": opened, "to": index})) {
        Ok(moved) => moved.get("document").and_then(Value::as_u64).map_or(opened, |i| i as usize),
        Err(_) => opened,
    };
    s.set_active(index);
    Ok(json!({"document": index, "previous": doc_id.0, "warnings": r.warnings, "wb": wb}))
}

/// After the re-develop: keep the old document's view (zoom, scroll) on the new one, report
/// the import's notes, then apply the remaining settings.
fn finish_redevelop(app: &mut PhotocraftApp, params: &CameraRaw, raw: &RawOpen, v: &Value) -> Result<Value, String> {
    let index = v.get("document").and_then(Value::as_u64).ok_or("the re-develop returned no document")? as usize;
    if let Some(new_id) = app.session.documents().get(index).map(|st| st.doc.id) {
        for id in app.view_docs.iter_mut().filter(|id| **id == raw.document) {
            *id = new_id;
        }
    }
    if let Some(new_id) = app.session.documents().get(index).map(|st| st.doc.id) {
        crate::raw_workspace::claim(app, new_id);
        crate::control_files::rebind(app, raw.document, new_id);
    }
    let warnings: Vec<String> =
        v.get("warnings").and_then(Value::as_array).map(|a| a.iter().filter_map(|w| w.as_str().map(str::to_string)).collect()).unwrap_or_default();
    crate::notices::io_warnings(app, &format!("Opened {}", raw.name), &without_develop_note(&warnings));
    let rest = crate::raw_recipe::after_develop(params, v.get("wb").and_then(Value::as_bool) == Some(true));
    finish_open_raw(app, index, rest, Some(true), Some(raw))
}

/// The other settings as one Camera Raw step on document `index`, then the document is marked
/// unmodified (developing is part of opening). The step runs like any filter (in the
/// background in the desktop app); the document is marked unmodified when it lands.
fn finish_open_raw(app: &mut PhotocraftApp, index: usize, rest: CameraRaw, redeveloped: Option<bool>, raw: Option<&RawOpen>) -> Result<Value, String> {
    app.session.set_active(index);
    app.sync_views();
    let mut result = json!({"document": index, "redeveloped": redeveloped.unwrap_or(false)});
    if !rest.is_identity() {
        let layer = app.session.active().and_then(|st| st.active_layer).ok_or("no layer to develop")?;
        let mut p = non_defaults(&rest);
        p["layer"] = json!(layer.0);
        let r = app.run("filter.cameraRaw", p)?;
        if r.get("pending").and_then(Value::as_bool) == Some(true)
            && let (Some(job), Some(raw)) = (r.get("job").and_then(Value::as_u64), raw)
        {
            app.raw_redevelop = Some(Redevelop { job: photocraft_engine::jobs::JobId(job), params: rest.clone(), raw: raw.clone(), filter_step: true });
        }
        result["filter"] = r;
    }
    mark_unmodified(app, index);
    Ok(result)
}

fn mark_unmodified(app: &mut PhotocraftApp, index: usize) {
    let active = app.session.active_index();
    app.session.set_active(index);
    if let Some(st) = app.session.active_mut() {
        st.saved_revision = st.revision;
    }
    if let Some(i) = active {
        app.session.set_active(i);
    }
    app.sync_views();
}

/// A finished background re-develop ([`REDEVELOP_JOB`]) or open-time Camera Raw step. `true`
/// when the event was one of those (handled here).
pub fn on_redevelop_event(app: &mut PhotocraftApp, e: &photocraft_engine::jobs::JobEvent) -> bool {
    let Some(pending) = app.raw_redevelop.take_if(|r| r.job == e.id) else { return false };
    if pending.filter_step {
        if matches!(e.outcome, photocraft_engine::jobs::JobOutcome::Done(_))
            && let Some(index) = e.document.and_then(|id| document_index(app, id))
        {
            mark_unmodified(app, index);
        }
        // The usual filter reporting still applies.
        return false;
    }
    match &e.outcome {
        photocraft_engine::jobs::JobOutcome::Done(v) => match finish_redevelop(app, &pending.params, &pending.raw, v) {
            Ok(_) => {
                app.ui.status = crate::i18n::fmt(tl!("Opened {name}"), &[("name", &pending.raw.name)]);
                app.ui.status_error = false;
            }
            Err(err) => crate::notices::error(app, format!("{}: {err}", e.label)),
        },
        photocraft_engine::jobs::JobOutcome::Failed(err) => {
            // The document stays open, developed with the defaults.
            app.sync_views();
            crate::notices::error(app, format!("{}: {err}", e.label));
        }
        photocraft_engine::jobs::JobOutcome::Cancelled => {
            app.ui.status = crate::i18n::fmt(tl!("Cancelled {label}"), &[("label", &e.label)]);
            app.ui.status_error = false;
        }
    }
    true
}

/// Cancel: closes the dialog; for the open-time dialog also the document it was opening.
pub(crate) fn cancel(app: &mut PhotocraftApp) {
    let Some(d) = app.camera_raw.take() else { return };
    // Cancel hands the window to the full editor: its canvas draws from now on.
    app.raw_workspace.browsing = None;
    // In the Camera Raw workspace the host owns the document: Cancel only closes the dialog.
    if let (Target::OpenRaw, Some(raw)) = (d.target, &d.raw_open)
        && !app.raw_workspace.enabled
        && let Some(index) = document_index(app, raw.document)
    {
        app.session.close(index);
        app.sync_views();
        app.ui.status = crate::i18n::fmt(tl!("Cancelled opening {name}"), &[("name", &raw.name)]);
        app.ui.status_error = false;
    }
}

/// What limits the preview: the document selection (a new filter applies through it, as the
/// engine does), a smart object's filter mask, or nothing.
enum Coverage {
    Selection,
    Mask(photocraft_doc::LayerMask),
    None,
}

pub fn open(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<(), String> {
    photocraft_engine::commands::find("filter.cameraRaw").map(|c| (c.enabled)(&app.session)).unwrap_or(Err("unknown command".into()))?;
    let (layer, surf, _) = crate::distort_ui::active_pixels(app)?;
    open_pixels(
        app,
        ctx,
        layer,
        surf,
        CameraRaw::default(),
        OpenAs { coverage: Coverage::Selection, target: Target::NewFilter, prepared: None, full_size: None },
    )
}

/// Opens Camera Raw on smart filter `index` of smart layer `layer`, with its stored settings and
/// the pixels below it; OK updates that filter (`layer.smartFilter.setParams`), as double-clicking
/// a Camera Raw smart filter does in Photoshop.
pub fn open_smart_filter(app: &mut PhotocraftApp, ctx: &egui::Context, layer: LayerId, index: usize) -> Result<(), String> {
    use photocraft_engine::lens_cmds::{RAW, raw_params};
    let st = app.session.active().ok_or("no document")?;
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(layer).map(|l| &l.content) else {
        return Err("the layer is not a smart object".into());
    };
    let filter = sm.smart_filters.get(index).ok_or("no such smart filter")?;
    if filter.command != RAW {
        return Err("the smart filter is not a Camera Raw filter".into());
    }
    let params = raw_params(RAW, &filter.params).map_err(|e| e.to_string())?;
    let surf = photocraft_engine::smart_cmds::render_below_filter(&st.doc, sm, index)
        .map_err(|e| e.to_string())?
        .ok_or("the smart object's contents are unavailable (missing linked file?)")?;
    let coverage = sm.filter_mask.clone().filter(|m| m.enabled).map_or(Coverage::None, Coverage::Mask);
    open_pixels(app, ctx, layer, surf, params, OpenAs { coverage, target: Target::SmartFilter(index), prepared: None, full_size: None })
}

/// How a dialog is opened on a layer.
struct OpenAs {
    coverage: Coverage,
    target: Target,
    /// The preview proxy, when a worker thread already built it.
    prepared: Option<ProxyData>,
    /// The size of the photo the layer is a reduced stand-in for (its pixels are fewer; everything the dialog says about size,
    /// and the scale its pixel-radius effects use, is the photo's).
    full_size: Option<(u32, u32)>,
}

fn open_pixels(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    layer: LayerId,
    surf: photocraft_raster::Surface,
    params: CameraRaw,
    how: OpenAs,
) -> Result<(), String> {
    let OpenAs { coverage, target, prepared, full_size } = how;
    let st = app.session.active().ok_or("no document")?;
    let canvas = st.doc.bounds();
    let name = st.doc.layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    let mut params = params;
    if params.encoding_gamma == 0.0 {
        params.encoding_gamma = photocraft_engine::lens_cmds::encoding_gamma(&st.doc);
    }
    let geometry = proxy_geometry(canvas, &surf)?;
    let (area, w, h, k, pw, ph) = (geometry.area, geometry.w, geometry.h, geometry.k, geometry.pw, geometry.ph);
    let (full_w, full_h) = full_size.map(|(fw, fh)| (fw as usize, fh as usize)).unwrap_or((w, h));
    // The proxy a worker thread built ahead, when it is for this very layer; otherwise built here.
    let proxy: Vec<[f32; 4]> = match prepared.filter(|p| p.area == area && p.k == k && p.pw == pw && p.ph == ph) {
        Some(p) => (*p.pixels).clone(),
        None => proxy_pixels(&surf, &geometry),
    };
    // Coverage at the centre of each proxy block.
    let sample = |value: &dyn Fn(i32, i32) -> f32| {
        (0..ph)
            .flat_map(|y| (0..pw).map(move |x| (area.x0 + (x * k + k / 2).min(w - 1) as i32, area.y0 + (y * k + k / 2).min(h - 1) as i32)))
            .map(|(x, y)| value(x, y).clamp(0.0, 1.0))
            .collect::<Vec<_>>()
    };
    let selection = match &coverage {
        Coverage::Selection => st.doc.selection.as_ref().map(|mask| sample(&|x, y| mask.sample_channel(x, y, 0))),
        Coverage::Mask(mask) => Some(sample(&|x, y| mask.value(x, y))),
        Coverage::None => None,
    };
    let source = photocraft_engine::color_cmds::composite_profile(&st.doc);
    let display = app.session.color.canvas_display(&st.doc).map_err(|e| e.to_string())?;
    let lab_transform = photocraft_cms::cached(&source, photocraft_cms::Builtin::LabD50.profile(), Default::default()).map_err(|e| e.to_string())?;
    let scope_transform = photocraft_cms::cached(&source, photocraft_cms::Builtin::Srgb.profile(), Default::default()).map_err(|e| e.to_string())?;
    if let Some(saved) = app.session.prefs().dialogs.get("filter.cameraRaw.scope")
        && saved.get("samplers").and_then(Value::as_array).is_none_or(|a| a.len() <= MAX_SAMPLERS)
        && let Ok(view) = serde_json::from_value(saved.clone())
    {
        app.ui.camera_raw_scope = view;
    }
    app.ui.camera_raw_preview = Default::default();
    app.ui.camera_raw_scope.samplers.clear();
    // Keep Camera Raw compact on open; vectorscope is an opt-in context-menu view.
    app.ui.camera_raw_scope.vectorscope = false;
    if selection.is_none() {
        app.ui.camera_raw_scope.selected_region = false;
    }
    let float = surf.format().sample == photocraft_color::SampleType::F32;
    let mut d = CameraRawDialog {
        layer,
        layer_name: name,
        original: params.clone(),
        params,
        target,
        before_histogram: RgbHistogram::from_rgba(&proxy),
        processed: Vec::new(),
        lab_transform,
        scope_transform,
        scope: Default::default(),
        after_histogram: RgbHistogram::default(),
        preview_revision: 0,
        histogram_ms: 0.0,
        proxy,
        selection,
        pw,
        ph,
        full_w,
        full_h,
        preview_ppp: ctx.pixels_per_point(),
        detail: super::camera_raw_detail_ui::DetailPreview::new(
            surf,
            area,
            match coverage {
                Coverage::Selection => {
                    st.doc.selection.clone().map(super::camera_raw_detail_ui::Coverage::Selection).unwrap_or(super::camera_raw_detail_ui::Coverage::None)
                }
                Coverage::Mask(mask) => super::camera_raw_detail_ui::Coverage::Mask(mask),
                Coverage::None => super::camera_raw_detail_ui::Coverage::None,
            },
        ),
        display,
        viewport: None,
        float,
        tex: None,
        before_tex: None,
        before_display_key: None,
        dirty: true,
        show_before: false,
        camera_info: app
            .session
            .active()
            .and_then(|state| state.doc.metadata.exif.as_ref())
            .map(|bytes| photocraft_algo::exif::read(bytes))
            .unwrap_or_default(),
        document_name: app.session.active().map(|state| state.doc.name.clone()).unwrap_or_default(),
        curve_state: Default::default(),
        curve_rect: None,
        render_ms: 0.0,
        raw_open: None,
        busy_since: None,
        raw_curve: None,
        undo: Vec::new(),
        redo: Vec::new(),
        edit_origin: None,
    };
    d.render(ctx);
    app.camera_raw = Some(d);
    // The dialog is up: the loading frame that stood in for it is done.
    app.raw_workspace.end_shell();
    Ok(())
}

/// Menu / control-channel entry point. `None` when the call isn't for this dialog.
pub fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != "filter.cameraRaw" {
        return None;
    }
    let Some(fields) = params.as_object() else {
        return Some(Err("Camera Raw params must be an object".into()));
    };
    if fields.is_empty() {
        return Some(open(app, ctx).map(|_| {
            app.camera_raw
                .as_ref()
                .map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview, &app.ui.camera_raw_workspace))
                .unwrap_or(Value::Null)
        }));
    }
    if let Some(sf) = params.get("smartFilter") {
        let opened = (|| {
            if fields.len() > 1 {
                return Err("smartFilter opens the dialog; send ui requests separately".to_string());
            }
            let layer = sf.get("layer").and_then(Value::as_u64).ok_or("smartFilter.layer must be a layer id")?;
            let index = sf.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or("smartFilter.index must be a filter index")?;
            open_smart_filter(app, ctx, LayerId(layer), index)?;
            Ok(app
                .camera_raw
                .as_ref()
                .map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview, &app.ui.camera_raw_workspace))
                .unwrap_or(Value::Null))
        })();
        return Some(opened);
    }
    let ui = params.get("ui")?;
    let Some(fields) = ui.as_object() else {
        return Some(Err("Camera Raw ui must be an object".into()));
    };
    for (key, value) in fields {
        match key.as_str() {
            "set" | "scope" | "view" | "workspace" => {}
            "reset" if matches!(value.as_str(), Some("open" | "default")) => {}
            "reset" => return Some(Err("Camera Raw reset must be open or default".into())),
            "before" | "commit" | "cancel" | "saveSettings" | "loadSettings" | "undo" | "redo" if value.is_boolean() => {}
            "before" | "commit" | "cancel" | "saveSettings" | "loadSettings" | "undo" | "redo" => {
                return Some(Err(format!("Camera Raw {key} must be a boolean")));
            }
            _ => return Some(Err(format!("unknown Camera Raw ui property {key}"))),
        }
    }
    if ui.get("commit").and_then(Value::as_bool) == Some(true) && ui.get("cancel").and_then(Value::as_bool) == Some(true) {
        return Some(Err("Camera Raw cannot commit and cancel together".into()));
    }
    for (key, run) in [("saveSettings", save_settings as fn(&mut PhotocraftApp) -> Result<Value, String>), ("loadSettings", load_settings)] {
        if ui.get(key).and_then(Value::as_bool) == Some(true) {
            if fields.len() > 1 {
                return Some(Err(format!("Camera Raw {key} must be sent on its own")));
            }
            return Some(run(app));
        }
    }
    let was_closed = app.camera_raw.is_none();
    let previous_view = app.ui.camera_raw_scope.clone();
    let previous_navigation = app.ui.camera_raw_preview;
    let previous_workspace = app.ui.camera_raw_workspace.clone();
    if was_closed && let Err(e) = open(app, ctx) {
        return Some(Err(e));
    }
    let result = menu_update(app, ctx, ui);
    if was_closed && result.as_ref().is_some_and(Result::is_err) {
        app.camera_raw = None;
        app.ui.camera_raw_scope = previous_view;
        app.ui.camera_raw_preview = previous_navigation;
        app.ui.camera_raw_workspace = previous_workspace;
    }
    result
}

fn menu_update(app: &mut PhotocraftApp, ctx: &egui::Context, ui: &Value) -> Option<Result<Value, String>> {
    // Stage both filter and presentation updates. No texture, preference or dialog mutation
    // occurs until every part of the request has passed validation.
    let d = app.camera_raw.as_ref()?;
    let mut next_params = match ui.get("reset").and_then(Value::as_str) {
        Some("open") => d.original.clone(),
        Some("default") => CameraRaw { encoding_gamma: d.params.encoding_gamma, pixel_scale: d.params.pixel_scale, ..Default::default() },
        _ => d.params.clone(),
    };
    if ui.get("reset").is_some() && ui.get("set").is_some() {
        return Some(Err("Camera Raw reset and set must be separate requests".into()));
    }
    let workspace = match ui.get("workspace") {
        Some(value) => match super::camera_raw_workspace_ui::prepare(&app.ui.camera_raw_workspace, value) {
            Ok(next) => Some(next),
            Err(error) => return Some(Err(error)),
        },
        None => None,
    };
    if let Some(set) = ui.get("set") {
        if !set.is_object() {
            return Some(Err("bad Camera Raw settings: expected an object".into()));
        }
        let mut cur = serde_json::to_value(&d.params).unwrap_or(json!({}));
        if let (Value::Object(c), Value::Object(s)) = (&mut cur, set) {
            for (k, v) in s {
                if !c.contains_key(k) {
                    return Some(Err(format!("unknown Camera Raw setting {k}")));
                }
                c.insert(k.clone(), v.clone());
            }
        }
        next_params = match serde_json::from_value::<CameraRaw>(cur) {
            Ok(p) => p,
            Err(e) => return Some(Err(format!("bad Camera Raw settings: {e}"))),
        };
        if let Err(e) = next_params.validate_changes(&d.original) {
            return Some(Err(format!("bad Camera Raw settings: {e}")));
        }
    }
    let scope_update = match ui.get("scope") {
        Some(scope) => match super::camera_raw_scope_ui::prepare_control(&next_params, d.selection.is_some(), &app.ui.camera_raw_scope, scope) {
            Ok(update) => Some(update),
            Err(e) => return Some(Err(e)),
        },
        None => None,
    };
    let navigation = match ui.get("view") {
        Some(view) => match super::camera_raw_preview_ui::prepare(app.ui.camera_raw_preview, view) {
            Ok(next) => Some(next),
            Err(e) => return Some(Err(e)),
        },
        None => None,
    };
    let d = app.camera_raw.as_mut()?;
    if let Some(next) = workspace {
        app.ui.camera_raw_workspace = next;
    }
    if let Some(view) = navigation {
        app.ui.camera_raw_preview = view;
    }
    if d.params.point_curve != next_params.point_curve
        || d.params.point_curve_red != next_params.point_curve_red
        || d.params.point_curve_green != next_params.point_curve_green
        || d.params.point_curve_blue != next_params.point_curve_blue
    {
        d.curve_state = Default::default();
    }
    if d.params != next_params {
        remember_edit(d);
        d.dirty = true;
        d.params = next_params;
    }
    if let Some(b) = ui.get("before").and_then(Value::as_bool) {
        d.show_before = b;
    }
    if let Some(update) = scope_update {
        update.apply(d, &mut app.ui.camera_raw_scope);
    }
    if ui.get("undo").and_then(Value::as_bool) == Some(true) {
        undo_edit(d);
    }
    if ui.get("redo").and_then(Value::as_bool) == Some(true) {
        redo_edit(d);
    }
    if d.dirty {
        d.render(ctx);
    }
    if app.ui.camera_raw_scope.vectorscope
        && let Some(d) = app.camera_raw.as_mut()
    {
        d.ensure_scope(app.ui.camera_raw_scope.selected_region);
    }
    super::camera_raw_scope_ui::persist(app, ctx);
    if ui.get("cancel").and_then(Value::as_bool) == Some(true) {
        cancel(app);
        return Some(Ok(json!({"cancelled": true})));
    }
    if ui.get("commit").and_then(Value::as_bool) == Some(true) {
        return Some(commit(app));
    }
    Some(Ok(app
        .camera_raw
        .as_ref()
        .map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview, &app.ui.camera_raw_workspace))
        .unwrap_or(Value::Null)))
}

/// Save Settings…: the dialog's settings as a Camera Raw `.xmp` settings file.
fn save_settings(app: &mut PhotocraftApp) -> Result<Value, String> {
    let d = app.camera_raw.as_ref().ok_or(tl!("Camera Raw isn't open"))?;
    let bytes = photocraft_io::camera_raw_xmp::save(&d.params).into_bytes();
    app.pick_save("Camera Raw Settings.xmp", move |app, path| {
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        Ok(json!({"path": path}))
    })
}

/// Load Settings…: applies a Camera Raw `.xmp` settings file's supported settings over the
/// dialog's; the status bar names how many were not supported.
fn load_settings(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.camera_raw.as_ref().ok_or(tl!("Camera Raw isn't open"))?;
    app.pick_file_bytes_filtered(&["xmp"], |app, name, bytes| {
        let file = crate::file_open::display_name(&name);
        let d = app.camera_raw.as_mut().ok_or(tl!("Camera Raw isn't open"))?;
        let mut next = d.params.clone();
        let report = photocraft_io::camera_raw_xmp::load(&mut next, &bytes).map_err(|e| format!("{file}: {e}"))?;
        next.validate_changes(&d.original).map_err(|e| format!("{file}: {e}"))?;
        if d.params.point_curve != next.point_curve {
            d.curve_state = Default::default();
        }
        if d.params != next {
            remember_edit(d);
            d.dirty = true;
            d.params = next;
        }
        app.ui.status_error = false;
        app.ui.status = if report.ignored.is_empty() {
            crate::i18n::fmt(tl!("Loaded Camera Raw settings from {file}"), &[("file", &file)])
        } else {
            let template = "Loaded Camera Raw settings from {file} ({n} not supported)";
            crate::i18n::fmt(tl!(template), &[("file", &file), ("n", &report.ignored.len().to_string())])
        };
        Ok(json!({"applied": report.applied, "ignored": report.ignored}))
    })
}

fn report_file_error(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r
        && e != crate::file_dialog::CANCELLED
    {
        app.ui.status_error = true;
        app.ui.status = e;
    }
}

/// Whether the open dialog's settings differ from the ones it opened with.
pub(crate) fn changed(app: &PhotocraftApp) -> bool {
    app.camera_raw.as_ref().is_some_and(|d| d.params != d.original)
}

const RAW_UNDO_LIMIT: usize = 40;

fn remember_edit(d: &mut CameraRawDialog) {
    if d.undo.len() == RAW_UNDO_LIMIT {
        d.undo.remove(0);
    }
    d.undo.push(d.params.clone());
    d.redo.clear();
}

fn undo_edit(d: &mut CameraRawDialog) -> bool {
    let Some(prev) = d.undo.pop() else { return false };
    d.redo.push(d.params.clone());
    if d.params.point_curve != prev.point_curve {
        d.curve_state = Default::default();
    }
    d.params = prev;
    d.dirty = true;
    d.edit_origin = None;
    true
}

fn redo_edit(d: &mut CameraRawDialog) -> bool {
    let Some(next) = d.redo.pop() else { return false };
    d.undo.push(d.params.clone());
    if d.params.point_curve != next.point_curve {
        d.curve_state = Default::default();
    }
    d.params = next;
    d.dirty = true;
    d.edit_origin = None;
    true
}

fn history_keys(d: &mut CameraRawDialog, ctx: &egui::Context) {
    let cmd_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
    if ctx.input_mut(|i| i.consume_key(cmd_shift, egui::Key::Z)) {
        redo_edit(d);
    } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
        undo_edit(d);
    }
}

pub(crate) fn commit(app: &mut PhotocraftApp) -> Result<Value, String> {
    let d = app.camera_raw.as_ref().ok_or(tl!("Camera Raw isn't open"))?;
    let result = match d.target {
        Target::NewFilter => {
            let mut p = d.command_params();
            p["layer"] = json!(d.layer.0);
            app.run("filter.cameraRaw", p)?
        }
        Target::SmartFilter(index) => {
            d.params.validate_changes(&d.original)?;
            // Every setting, not just the non-defaults: setParams merges into the stored ones,
            // so a control reset to its default must overwrite the old value.
            let mut params = serde_json::to_value(&d.params).map_err(|e| e.to_string())?;
            if let Value::Object(m) = &mut params {
                m.remove("pixelScale");
            }
            app.run("layer.smartFilter.setParams", json!({"layer": d.layer.0, "index": index, "params": params}))?
        }
        Target::OpenRaw => {
            let raw = d.raw_open.clone().ok_or("no raw file is being opened")?;
            let params = d.params.clone();
            let changed = d.params != d.original;
            let developed = developed(d);
            // Open keeps the recipe too, so the next visit starts from what was just chosen. It is kept before anything
            // changes: changed settings that cannot be kept open nothing and leave the dialog as it was. Unchanged ones
            // are already the photo's, so a failed rewrite of them does not stop the Open.
            match crate::raw_recipe::persist(app, raw.document, &raw.targets, &params, &developed) {
                Err(error) if changed => return Err(error),
                Err(error) => log::warn!("{error}"),
                Ok(()) => {}
            }
            // Open goes to the full editor: its canvas draws from now on.
            app.raw_workspace.browsing = None;
            if let Some(project) = raw.continue_from.clone() {
                // The photo already has a full-editor document: go back to it, untouched.
                app.camera_raw = None;
                app.raw_workspace.expect_continue = Some(raw.document);
                return match crate::control_files::open_with_camera_raw(app, &project, true, None) {
                    Ok(photocraft_engine::jobs::Started::Job(job)) => Ok(json!({"job": job.0, "pending": true, "continued": project})),
                    // Restored from memory (a save of it still in flight): it has landed already.
                    Ok(photocraft_engine::jobs::Started::Done(done)) => {
                        crate::raw_workspace::on_open_event(app, None, true);
                        Ok(done)
                    }
                    Err(error) => {
                        app.raw_workspace.expect_continue = None;
                        Err(error)
                    }
                };
            }
            commit_open_raw(app, &params, &raw)?
        }
    };
    app.camera_raw = None;
    Ok(result)
}

/// Which kind of dialog is open, for a host: `openRaw` (a raw opened in Camera Raw), `newFilter` or `smartFilter`.
pub(crate) fn target_name(app: &PhotocraftApp) -> Option<&'static str> {
    // Names for a host, not labels: indexed rather than matched to strings, so the translation scan skips them.
    const NAMES: [&str; 3] = ["openRaw", "newFilter", "smartFilter"];
    app.camera_raw.as_ref().map(|d| {
        NAMES[match d.target {
            Target::OpenRaw => 0,
            Target::NewFilter => 1,
            Target::SmartFilter(_) => 2,
        }]
    })
}

/// How the dialog's photo was developed, for rendering its recipe as the dialog shows it.
fn developed(d: &CameraRawDialog) -> crate::raw_recipe::Developed {
    let full_width = d.raw_open.as_ref().and_then(|raw| raw.full_size).map(|(w, _)| w);
    crate::raw_recipe::Developed { curve: d.raw_curve.clone(), full_width }
}

/// The open dialog's settings as a recipe file's content.
pub(crate) fn recipe(app: &PhotocraftApp) -> Option<Value> {
    app.camera_raw.as_ref().map(|d| crate::raw_recipe::to_json(&d.params))
}

pub(crate) fn source(app: &PhotocraftApp) -> Option<&str> {
    app.camera_raw.as_ref()?.raw_open.as_ref()?.path.as_deref()
}

/// Leaving the photo the dialog is on ("Done"): a raw's settings are kept as its recipe and nothing is
/// developed or applied to a document, so browsing past a photo never costs a develop or a save; any other
/// dialog applies as a photo switch does (changed settings as one history step, untouched ones just close).
pub(crate) fn done(app: &mut PhotocraftApp) -> Result<Value, String> {
    let Some(d) = app.camera_raw.as_ref() else { return Ok(json!({"open": false, "changed": false})) };
    let changed = d.params != d.original;
    if d.target != Target::OpenRaw {
        if changed {
            commit(app)?;
        } else {
            cancel(app);
        }
        return Ok(json!({"open": false, "changed": changed}));
    }
    let raw = d.raw_open.clone().ok_or("no raw file is being opened")?;
    let params = d.params.clone();
    let developed = developed(d);
    // The recipe is kept before the dialog goes: one that cannot be written leaves the dialog, and its settings, as they were.
    if changed {
        crate::raw_recipe::persist(app, raw.document, &raw.targets, &params, &developed)?;
    }
    app.camera_raw = None;
    Ok(json!({"open": false, "changed": changed, "recipe": crate::raw_recipe::to_json(&params)}))
}

fn temp_stops() -> Vec<Color32> {
    vec![Color32::from_rgb(70, 120, 230), Color32::from_rgb(200, 200, 200), Color32::from_rgb(235, 200, 60)]
}
fn tint_stops() -> Vec<Color32> {
    vec![Color32::from_rgb(70, 190, 80), Color32::from_rgb(200, 200, 200), Color32::from_rgb(210, 80, 200)]
}

/// One labelled slider; marks the dialog dirty when it moves.
fn row(ui: &mut egui::Ui, dirty: &mut bool, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, grad: Option<&[Color32]>) {
    let _ = slider(ui, dirty, label, v, range, grad);
}

fn slider(ui: &mut egui::Ui, dirty: &mut bool, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, grad: Option<&[Color32]>) -> egui::Response {
    slider_default(ui, dirty, label, v, range, grad, 0.0)
}

#[allow(clippy::too_many_arguments)]
fn slider_default(
    ui: &mut egui::Ui,
    dirty: &mut bool,
    label: &str,
    v: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    grad: Option<&[Color32]>,
    default: f32,
) -> egui::Response {
    let decimals = Some(if label == "Exposure" {
        2
    } else if *range.start() == 0.5 {
        1
    } else {
        0
    });
    let label = crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", label);
    let r = widgets::slider_row_prec_styled(
        ui,
        label,
        v,
        range,
        "",
        grad,
        widgets::RowGestures { reset: Some(default), wheel_step: None },
        decimals,
        super::camera_raw_design::get(ui.ctx()).value_field_width,
        Some(super::camera_raw_design::get(ui.ctx()).slider_style()),
    );
    *dirty |= r.changed();
    r
}

/// A Light tone slider; Alt-dragging it shows that zone's channel clipping on the preview.
fn tone_row(ui: &mut egui::Ui, dirty: &mut bool, alt: &mut Option<ToneZone>, zone: ToneZone, v: &mut f32) {
    let r = slider(ui, dirty, zone.name(), v, -zone.limit()..=zone.limit(), None);
    if r.dragged() && ui.input(|i| i.modifiers.alt) {
        *alt = Some(zone);
    }
}

// The shared Curve editor must never produce a curve `CameraRaw::validate` rejects.
const _: () = assert!(crate::point_curve::MAX_POINTS == photocraft_algo::camera_raw::MAX_CURVE_POINTS && crate::point_curve::MIN_GAP >= 1.0);

/// Small point-curve editor (master channel): click to add, drag to move, right-click to delete.
fn curve_editor(
    ui: &mut egui::Ui,
    p: &mut CameraRaw,
    dirty: &mut bool,
    state: &mut crate::state::PointCurveState,
    channel: super::camera_raw_workspace_ui::CurveChannel,
    histogram: &RgbHistogram,
) -> ERect {
    let t = Tokens::get(ui.ctx());
    let side = ui.available_width().min(260.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, t.radius_sm, t.field);
    use super::camera_raw_workspace_ui::CurveChannel;
    let counts: Vec<u64> = (0..256)
        .map(|i| match channel {
            CurveChannel::Red => histogram.channels[0][i],
            CurveChannel::Green => histogram.channels[1][i],
            CurveChannel::Blue => histogram.channels[2][i],
            _ => histogram.channels.iter().fold(0_u64, |sum, band| sum.saturating_add(band[i])),
        })
        .collect();
    let peak = counts.iter().copied().max().unwrap_or(1).max(1) as f32;
    for (i, count) in counts.iter().enumerate() {
        let x0 = rect.left() + i as f32 / 256.0 * side;
        let x1 = rect.left() + (i + 1) as f32 / 256.0 * side;
        painter.rect_filled(ERect::from_min_max(pos2(x0, rect.bottom() - *count as f32 / peak * side), pos2(x1, rect.bottom())), 0.0, t.canvas);
    }
    for i in 1..4 {
        let f = i as f32 / 4.0;
        painter.line_segment([pos2(rect.left() + f * side, rect.top()), pos2(rect.left() + f * side, rect.bottom())], Stroke::new(1.0, t.separator));
        painter.line_segment([pos2(rect.left(), rect.top() + f * side), pos2(rect.right(), rect.top() + f * side)], Stroke::new(1.0, t.separator));
    }
    let to_screen = |x: f32, y: f32| pos2(rect.left() + x / 255.0 * side, rect.bottom() - y / 255.0 * side);
    let selected = match channel {
        CurveChannel::Red => &mut p.point_curve_red,
        CurveChannel::Green => &mut p.point_curve_green,
        CurveChannel::Blue => &mut p.point_curve_blue,
        _ => &mut p.point_curve,
    };
    // The identity handles are presentation until a gesture actually edits them.
    let mut points = if selected.is_empty() { vec![[0.0, 0.0], [255.0, 255.0]] } else { selected.clone() };
    if channel != CurveChannel::Parametric && crate::point_curve::interact(ui, &resp, rect, &mut points, state).changed {
        *selected = points.clone();
        *dirty = true;
    }
    // Curve (point curve after the parametric one).
    let lut = if channel == CurveChannel::Parametric {
        let curve = CameraRaw {
            curve_highlights: p.curve_highlights,
            curve_lights: p.curve_lights,
            curve_darks: p.curve_darks,
            curve_shadows: p.curve_shadows,
            curve_splits: p.curve_splits,
            ..Default::default()
        };
        let mut ramp: Vec<[f32; 4]> = (0..256)
            .map(|i| {
                let v = i as f32 / 255.0;
                [v, v, v, 1.0]
            })
            .collect();
        photocraft_algo::camera_raw::develop(&mut ramp, 256, 1, &curve, true);
        ramp.into_iter().map(|pixel| pixel[0]).collect()
    } else {
        curve_lut(&points, 256)
    };
    let pts: Vec<Pos2> = lut.iter().enumerate().map(|(i, value)| to_screen(i as f32, value * 255.0)).collect();
    let color = match channel {
        CurveChannel::Red => Color32::from_rgb(220, 65, 65),
        CurveChannel::Green => Color32::from_rgb(70, 185, 90),
        CurveChannel::Blue => Color32::from_rgb(70, 130, 235),
        _ => t.text,
    };
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, color)));
    for (i, c) in points.iter().enumerate() {
        if channel == CurveChannel::Parametric {
            break;
        }
        let center = to_screen(c[0], c[1]);
        if state.selected == Some(i) {
            painter.circle_filled(center, 3.0, t.accent);
        }
        painter.circle_stroke(center, 4.0, Stroke::new(1.5, t.accent));
    }
    rect
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // A queued dialog opens when none is up, or takes over from the previous photo's dialog that was left for it.
    if app.camera_raw.as_ref().is_none_or(CameraRawDialog::busy)
        && let Some(raw) = app.pending_raw_open.take()
        && let Err(e) = open_raw(app, ctx, raw)
    {
        // The document stays open, developed with the defaults.
        app.ui.status = e;
        app.ui.status_error = true;
    }
    let Some(d) = app.camera_raw.as_ref() else { return };
    // Where the platform has no extra windows (the web build), or in the Camera Raw workspace, draw
    // over the main window.
    if ctx.embed_viewports() || app.raw_workspace.embedded() {
        draw(app, ctx, false);
        return;
    }
    let window_title = d.window_title();
    // Camera Raw is modal, as in Photoshop: the main window is dimmed and takes no input while it
    // is open (in a separate window, or drawn over it where there are no extra windows).
    let main = ctx.content_rect();
    egui::Area::new(egui::Id::new("camera-raw-modal")).order(egui::Order::Foreground).fixed_pos(main.min).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(main.size(), Sense::click_and_drag());
        ui.painter().rect_filled(r, 0.0, Tokens::get(ui.ctx()).scrim);
    });
    // A window of its own (title bar, moved and resized like any window), like Adobe Camera Raw.
    let builder = egui::ViewportBuilder::default().with_title(window_title).with_inner_size([1360.0, 900.0]).with_min_inner_size([900.0, 600.0]);
    let mut close = false;
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("camera-raw-window"), builder, |ui, class| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            close = true;
        }
        draw(app, ui.ctx(), class != egui::ViewportClass::EmbeddedWindow);
    });
    if close && app.camera_raw.is_some() {
        cancel(app);
    }
}

impl CameraRawDialog {
    /// "Camera Raw (name)" for the open-time dialog, "Camera Raw Filter (layer)" otherwise.
    fn window_title(&self) -> String {
        if self.target == Target::OpenRaw {
            crate::i18n::fmt(tl!("Camera Raw ({name})"), &[("name", &self.layer_name)])
        } else {
            crate::i18n::fmt(tl!("Camera Raw Filter ({layer})"), &[("layer", &self.layer_name)])
        }
    }
}

/// Where the dialog's parts go in a window the size of `screen`. One definition for the dialog and for the loading frame
/// ([`draw_shell`]) that stands in for it, so the host's picture of a photo, laid over `view`, does not move when the dialog
/// replaces the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Layout {
    pub title: ERect,
    pub body: ERect,
    pub preview_body: ERect,
    /// Where the photo is shown.
    pub view: ERect,
    /// The settings panel.
    pub right: ERect,
    pub rail: ERect,
    pub nav: ERect,
    pub foot: ERect,
}

pub(crate) fn layout(ctx: &egui::Context, screen: ERect, _own_window: bool) -> Layout {
    let design = super::camera_raw_design::get(ctx);
    let footer_h = design.footer_height.min(screen.height() * 0.2);
    let rail_w = design.rail_width.min(screen.width() * 0.1);
    let panel_w = design.panel_width.min(screen.width() * 0.45);
    let title = ERect::from_min_size(screen.min, vec2(screen.width(), design.header_height.min(screen.height() * 0.15)));
    let body = ERect::from_min_max(pos2(screen.left(), title.bottom()), pos2(screen.right(), screen.bottom() - footer_h));
    let rail = ERect::from_min_max(pos2(body.right() - rail_w, body.top()), body.max);
    let right = ERect::from_min_max(pos2(rail.left() - panel_w, body.top()), rail.left_bottom());
    let preview_body = ERect::from_min_max(body.min, right.left_bottom());
    let nav = ERect::from_min_max(
        pos2(preview_body.left(), (preview_body.bottom() - design.navigation_height.min(preview_body.height() * 0.25)).max(preview_body.top())),
        preview_body.max,
    );
    let margin = design.preview_margin.min(preview_body.width() * 0.25).min((nav.top() - preview_body.top()) * 0.25);
    let view = ERect::from_min_max(
        preview_body.min + vec2(margin, margin),
        pos2((preview_body.right() - margin).max(preview_body.left() + margin + 1.0), (nav.top() - margin).max(preview_body.top() + margin + 1.0)),
    );
    let foot = ERect::from_min_max(pos2(screen.left(), screen.bottom() - footer_h), screen.max);
    Layout { title, body, preview_body, view, right, rail, nav, foot }
}

/// Leaves the open-time dialog's photo without closing the dialog: the next photo is on its way and takes the dialog
/// over, so the window never shows it going away and coming back. What was adjusted is kept as the photo's recipe, as
/// [`done`] does; the dialog stays up, marked busy (its controls dim if the swap takes more than a moment), until the next
/// photo replaces its contents.
pub(crate) fn leave_in_place(app: &mut PhotocraftApp) -> Result<(), String> {
    let Some(d) = app.camera_raw.as_ref() else { return Ok(()) };
    if d.target != Target::OpenRaw {
        return Err("only the open-time dialog can hand over to the next photo".into());
    }
    let raw = d.raw_open.clone().ok_or("no raw file is being opened")?;
    let params = d.params.clone();
    let changed = d.params != d.original;
    let developed = developed(d);
    // Kept first: a recipe that cannot be written leaves the dialog on this photo, unchanged, and the next photo is not opened.
    if changed {
        crate::raw_recipe::persist(app, raw.document, &raw.targets, &params, &developed)?;
    }
    if let Some(d) = app.camera_raw.as_mut() {
        d.original = d.params.clone();
        d.busy_since = d.busy_since.or_else(|| Some(std::time::Instant::now()));
    }
    Ok(())
}

/// The dialog's frame while the Camera Raw workspace is still developing the photo (its dialog opens a moment later):
/// the same title bar, preview area, settings panel and footer as [`draw`], with nothing in it usable. The window shows
/// this instead of the editor's menus and panels, which a photo being browsed has no use for, and the host's picture of
/// the photo lies over `layout().view` until the dialog takes it over.
pub(crate) fn draw_shell(ctx: &egui::Context, name: &str) {
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let lay = layout(ctx, screen, false);
    // No fade-in: while it fades, the editor's own windows show through, which is what the frame is there to hide.
    egui::Area::new(egui::Id::new("camera-raw-shell")).order(egui::Order::Foreground).fixed_pos(screen.min).fade_in(false).show(ctx, |ui| {
        let (full, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, t.chrome);
        painter.rect_filled(lay.title, 0.0, t.dock);
        painter.line_segment([lay.title.left_bottom(), lay.title.right_bottom()], Stroke::new(1.0, t.separator));
        painter.rect_filled(lay.view, 0.0, t.canvas);
        // The photo's name is not known in the moment between leaving one photo and being asked for the next: no title
        // rather than the wrong one.
        if !name.is_empty() {
            painter.text(
                lay.title.center(),
                Align2::CENTER_CENTER,
                crate::i18n::fmt(tl!("Camera Raw ({name})"), &[("name", name)]),
                FontId::proportional(13.0),
                t.text,
            );
            painter.text(
                lay.view.center(),
                Align2::CENTER_CENTER,
                crate::i18n::fmt(tl!("Developing {name}"), &[("name", name)]),
                FontId::proportional(13.0),
                t.text_faint,
            );
        }
        painter.rect_filled(lay.right, 0.0, t.dock);
        painter.line_segment([lay.right.left_top(), lay.right.left_bottom()], Stroke::new(1.0, t.separator));
        painter.rect_filled(lay.nav, 0.0, t.dock);
        painter.line_segment([lay.rail.left_top(), lay.rail.left_bottom()], Stroke::new(1.0, t.separator));
        let mut y = lay.right.top() + 158.0;
        for section in ["Light", "Color", "Effects", "Curve", "Color Mixer", "Color Grading", "Detail"] {
            painter.text(
                pos2(lay.right.left() + 18.0, y),
                Align2::LEFT_CENTER,
                crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", section),
                FontId::proportional(13.0),
                t.text_faint,
            );
            y += 30.0;
        }
        painter.rect_filled(lay.foot, 0.0, t.dock);
        painter.line_segment([lay.foot.left_top(), lay.foot.right_top()], Stroke::new(1.0, t.separator));
        let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(lay.foot.shrink2(vec2(16.0, 9.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        fu.add_enabled_ui(false, |fu| {
            widgets::dialog_buttons(
                fu,
                &[
                    widgets::DialogButton::new(widgets::ButtonRole::Default, tl!("Open"), 90.0),
                    widgets::DialogButton::new(widgets::ButtonRole::Cancel, tl!("Cancel"), 90.0),
                ],
            );
        });
    });
}

/// The dialog's contents over the whole of `ctx`'s window. `own_window`: the OS window already
/// shows the title, so none is painted.
fn draw(app: &mut PhotocraftApp, ctx: &egui::Context, own_window: bool) {
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let design = super::camera_raw_design::get(ctx);
    super::camera_raw_design::apply_preview(ctx, &design, &mut app.ui.camera_raw_workspace);
    let current_display = app.session.active().and_then(|st| app.session.color.canvas_display(&st.doc).ok());
    let mut action: Option<&str> = None;
    // The photo was left for the next one, which has not taken the dialog over yet: nothing in the dialog may act
    // meanwhile, and if the swap takes more than a moment the controls dim to say so.
    let busy_since = app.camera_raw.as_ref().and_then(|d| d.busy_since);
    let busy = busy_since.is_some();
    let busy_visible = busy_since.is_some_and(|since| since.elapsed() >= BUSY_AFTER);
    if busy && !busy_visible {
        ctx.request_repaint_after(BUSY_AFTER);
    }
    egui::Area::new(egui::Id::new("camera-raw-dialog")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let Some(d) = app.camera_raw.as_mut() else { return };
        history_keys(d, ctx);
        let frame_origin = d.params.clone();
        let pointer = ctx.input(|i| i.pointer.primary_down());
        if pointer && d.edit_origin.is_none() {
            d.edit_origin = Some(d.params.clone());
        }
        if let Some(display) = &current_display
            && display.key != d.display.key
        {
            d.display = display.clone();
            d.dirty = true;
        }
        d.curve_rect = None;
        d.scope.vectorscope_rect = None;
        if d.dirty {
            d.render(ctx);
        }
        let (full, _) = ui.allocate_exact_size(screen.size(), Sense::click());
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, t.chrome);
        let lay = layout(ctx, screen, own_window);
        let title = lay.title;
        {
            painter.rect_filled(title, 0.0, t.dock);
            painter.line_segment([title.left_bottom(), title.right_bottom()], Stroke::new(1.0, t.separator));
            painter.text(title.center(), Align2::CENTER_CENTER, d.window_title(), FontId::proportional(13.0), t.text);
        }
        let ok_label = if d.target == Target::OpenRaw { tl!("Open") } else { tl!("OK") };
        let scope = &mut app.ui.camera_raw_scope;
        bars::top_actions(ui, title, &mut action, scope);
        // Clip both pixels and scope overlays before painting the settings dock/footer.
        let view = lay.view;
        d.viewport = Some(view);
        d.preview_ppp = ctx.pixels_per_point();
        let size = vec2(d.full_w as f32, d.full_h as f32) / ctx.pixels_per_point();
        let navigation = &mut app.ui.camera_raw_preview;
        let workspace = &mut app.ui.camera_raw_workspace;
        let mut preview = ui.new_child(egui::UiBuilder::new().max_rect(view));
        preview.set_clip_rect(view.intersect(ui.clip_rect()));
        preview.painter().rect_filled(view, 0.0, t.canvas);
        let mut preview_sampling = scope.sampler_tool || workspace.white_balance_tool;
        let (r, response, panning, zoom_box) = super::camera_raw_preview_ui::interact(&mut preview, navigation, view, size, &mut preview_sampling);
        if !preview_sampling {
            scope.sampler_tool = false;
            workspace.white_balance_tool = false;
        }
        let tex = if d.show_before { d.before_tex.as_ref() } else { d.tex.as_ref() };
        if let Some(tex) = tex {
            // Never tile a checker across the offscreen extent of a highly zoomed image.
            widgets::checker(preview.painter(), r.intersect(view), 8.0);
            preview.painter().image(tex.id(), r, ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            let raw_tone = d.raw_curve.is_some() && !raw_stage(&d.params).is_neutral();
            // A stand-in made for browsing has no more detail to show: zoomed in, the photo is developed in full by Open.
            let detail_needed =
                !raw_tone && (r.width() * ctx.pixels_per_point() > d.pw as f32 || r.height() * ctx.pixels_per_point() > d.ph as f32) && !d.is_stand_in();
            d.detail.paint(&preview, r, d.preview_revision, d.show_before, &d.params, detail_needed, &d.display);
            if d.detail.pending() {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            d.scope.preview_rect = Some(r);
            super::camera_raw_scope_ui::preview(&mut preview, d, scope, r, &response, panning);
            bars::pick_white_balance(&preview, d, r, &response, &mut workspace.white_balance_tool, panning);
        }
        super::camera_raw_workspace_ui::grid(&preview, r, view, workspace);
        if let Some(region) = zoom_box {
            preview.painter().rect_stroke(region, 0.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        }
        painter.rect_filled(lay.nav, 0.0, t.dock);
        painter.line_segment([lay.nav.left_top(), lay.nav.right_top()], Stroke::new(1.0, t.separator));
        let mut nav_ui =
            ui.new_child(egui::UiBuilder::new().max_rect(lay.nav.shrink2(vec2(8.0, 2.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        nav_ui.set_clip_rect(lay.nav.intersect(ui.clip_rect()));
        super::camera_raw_preview_ui::toolbar(&mut nav_ui, navigation, view, size, &mut scope.sampler_tool);
        widgets::checkbox(&mut nav_ui, &mut d.show_before, tl!("Before (Y)"));
        if d.detail.pending() || busy_visible {
            nav_ui.spinner();
        }
        if let Some(error) = &d.detail.error {
            nav_ui.label(egui::RichText::new(tl!("Full-resolution preview unavailable")).color(t.text_faint)).on_hover_text(error);
        }
        // Panels.
        let right = lay.right;
        painter.rect_filled(right, 0.0, t.dock);
        let mut props = ui.new_child(egui::UiBuilder::new().max_rect(right));
        props.style_mut().override_font_id = Some(FontId::proportional(design.panel_font_size));
        props.spacing_mut().item_spacing.y = 0.0;
        // Overflow must not resize a curve while selecting a point reveals its fields.
        props.spacing_mut().scroll.floating = true;
        if busy_visible {
            props.disable();
        }
        // This header is outside the settings scroll area and follows the same Before/After
        // selector as the texture. Both derive from the same completed preview revision.
        super::camera_raw_scope_ui::header(&mut props, d, scope);
        bars::metadata(&mut props, &d.camera_info);
        bars::edit_bar(&mut props, d, workspace.info);
        if !workspace.info {
            bars::profile_bar(&mut props);
        }
        d.scope.alt_tone = None;
        let mut dirty = false;
        egui::ScrollArea::vertical().id_salt("camera-raw-props").show(&mut props, |ui| {
            if workspace.info {
                egui::Frame::new().inner_margin(14).show(ui, |ui| {
                    ui.strong(tl!("File Name"));
                    ui.label(&d.document_name);
                    if let Some(model) = &d.camera_info.model {
                        ui.strong(model);
                    }
                    if let Some(date) = &d.camera_info.date_taken {
                        ui.strong(tl!("Capture Date"));
                        ui.label(date);
                    }
                    if let Some(lens) = &d.camera_info.lens {
                        ui.strong(tl!("Lens"));
                        ui.label(lens);
                    }
                    ui.separator();
                    ui.strong(tl!("Dimensions"));
                    ui.label(format!("{} × {}", d.full_w, d.full_h));
                    if let Some(raw) = &d.raw_open {
                        ui.label(&raw.name);
                    }
                });
                return;
            }
            ui.spacing_mut().item_spacing.y = design.row_gap;
            let p = &mut d.params;
            let alt = &mut d.scope.alt_tone;
            let (ts, tn) = (temp_stops(), tint_stops());
            for panel_name in &design.panel_order {
                match panel_name.as_str() {
                    "Light" => {
                        super::camera_raw_workspace_ui::section(ui, "Light", &mut workspace.panels, |ui| {
                            tone_row(ui, &mut dirty, alt, ToneZone::Exposure, &mut p.exposure);
                            row(ui, &mut dirty, "Contrast", &mut p.contrast, -100.0..=100.0, None);
                            tone_row(ui, &mut dirty, alt, ToneZone::Highlights, &mut p.highlights);
                            tone_row(ui, &mut dirty, alt, ToneZone::Shadows, &mut p.shadows);
                            tone_row(ui, &mut dirty, alt, ToneZone::Whites, &mut p.whites);
                            tone_row(ui, &mut dirty, alt, ToneZone::Blacks, &mut p.blacks);
                        });
                    }
                    "Color" => {
                        super::camera_raw_workspace_ui::section(ui, "Color", &mut workspace.panels, |ui| {
                            dirty |= bars::white_balance(ui, p, &mut workspace.white_balance_tool);
                            if workspace.white_balance_tool {
                                scope.sampler_tool = false;
                            }
                            row(ui, &mut dirty, "Temperature", &mut p.temperature, -100.0..=100.0, Some(&ts));
                            row(ui, &mut dirty, "Tint", &mut p.tint, -100.0..=100.0, Some(&tn));
                            row(ui, &mut dirty, "Vibrance", &mut p.vibrance, -100.0..=100.0, None);
                            row(ui, &mut dirty, "Saturation", &mut p.saturation, -100.0..=100.0, None);
                        });
                    }
                    "Effects" => {
                        super::camera_raw_workspace_ui::section(ui, "Effects", &mut workspace.panels, |ui| {
                            row(ui, &mut dirty, "Texture", &mut p.texture, -100.0..=100.0, None);
                            row(ui, &mut dirty, "Clarity", &mut p.clarity, -100.0..=100.0, None);
                            row(ui, &mut dirty, "Dehaze", &mut p.dehaze, -100.0..=100.0, None);
                            dirty |= super::camera_raw_workspace_ui::disclosed_slider(
                                ui,
                                "Vignette",
                                &mut p.vignette_amount,
                                -100.0..=100.0,
                                &mut workspace.vignette_details,
                            );
                            if workspace.vignette_details {
                                let mut style = p.vignette_style.clone();
                                if widgets::dropdown(
                                    ui,
                                    "cr-vig-style",
                                    &mut style,
                                    &[
                                        ("highlightPriority".to_string(), tl!("Highlight Priority")),
                                        ("colorPriority".to_string(), tl!("Color Priority")),
                                        ("paintOverlay".to_string(), tl!("Paint Overlay")),
                                    ],
                                    200.0,
                                ) {
                                    p.vignette_style = style;
                                    dirty = true;
                                }
                                let _ = slider_default(ui, &mut dirty, "Midpoint", &mut p.vignette_midpoint, 0.0..=100.0, None, 50.0);
                                row(ui, &mut dirty, "Roundness", &mut p.vignette_roundness, -100.0..=100.0, None);
                                let _ = slider_default(ui, &mut dirty, "Feather", &mut p.vignette_feather, 0.0..=100.0, None, 50.0);
                                row(ui, &mut dirty, "Highlights", &mut p.vignette_highlights, 0.0..=100.0, None);
                            }
                            dirty |=
                                super::camera_raw_workspace_ui::disclosed_slider(ui, "Grain", &mut p.grain_amount, 0.0..=100.0, &mut workspace.grain_details);
                            if workspace.grain_details {
                                let _ = slider_default(ui, &mut dirty, "Size", &mut p.grain_size, 0.0..=100.0, None, 25.0);
                                let _ = slider_default(ui, &mut dirty, "Roughness", &mut p.grain_roughness, 0.0..=100.0, None, 50.0);
                            }
                        });
                    }
                    "Curve" => {
                        super::camera_raw_workspace_ui::section(ui, "Curve", &mut workspace.panels, |ui| {
                            use super::camera_raw_workspace_ui::CurveChannel;
                            let previous = workspace.curve_channel;
                            bars::curve_selectors(ui, &mut workspace.curve_channel);
                            if previous != workspace.curve_channel {
                                d.curve_state = Default::default();
                            }
                            d.curve_rect = Some(curve_editor(ui, p, &mut dirty, &mut d.curve_state, workspace.curve_channel, &d.after_histogram));
                            if workspace.curve_channel == CurveChannel::Parametric {
                                row(ui, &mut dirty, "Highlights", &mut p.curve_highlights, -100.0..=100.0, None);
                                row(ui, &mut dirty, "Lights", &mut p.curve_lights, -100.0..=100.0, None);
                                row(ui, &mut dirty, "Darks", &mut p.curve_darks, -100.0..=100.0, None);
                                row(ui, &mut dirty, "Shadows", &mut p.curve_shadows, -100.0..=100.0, None);
                            }
                        });
                    }
                    "Color Mixer" => {
                        super::camera_raw_workspace_ui::section(ui, "Color Mixer", &mut workspace.panels, |ui| {
                            let mut mode = workspace.mixer;
                            widgets::dropdown(
                                ui,
                                "cr-mixer-mode",
                                &mut mode,
                                &[(super::camera_raw_workspace_ui::MixerMode::Hsl, "HSL"), (super::camera_raw_workspace_ui::MixerMode::Color, tl!("Color"))],
                                120.0,
                            );
                            workspace.mixer = mode;
                            if mode == super::camera_raw_workspace_ui::MixerMode::Color {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 3.0;
                                    let side = ((ui.available_width() - 21.0) / 8.0).clamp(12.0, 28.0);
                                    for (k, name) in BANDS.iter().enumerate() {
                                        let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
                                        let name = crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", name);
                                        response.widget_info(|| {
                                            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), workspace.band == k, name)
                                        });
                                        if response.clicked() {
                                            workspace.band = k;
                                        }
                                        let color = hue_color(photocraft_algo::camera_raw::HSL_BANDS[k]);
                                        ui.painter().circle_stroke(rect.center(), side * 0.35, Stroke::new(2.0, color));
                                        if workspace.band == k {
                                            ui.painter().circle_filled(rect.center(), side * 0.15, t.text);
                                        }
                                        response.on_hover_text(name);
                                    }
                                });
                                let band = workspace.band.min(7);
                                row(ui, &mut dirty, "Hue", &mut p.hsl_hue[band], -100.0..=100.0, Some(&widgets::hue_stops()));
                                row(ui, &mut dirty, "Saturation", &mut p.hsl_sat[band], -100.0..=100.0, None);
                                row(ui, &mut dirty, "Luminance", &mut p.hsl_lum[band], -100.0..=100.0, None);
                            } else {
                                ui.horizontal_wrapped(|ui| {
                                    for (i, source) in ["Hue", "Saturation", "Luminance"].iter().enumerate() {
                                        let name = crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", source);
                                        let response = widgets::pill_tab(ui, name, workspace.hsl_channel == i);
                                        response.widget_info(|| {
                                            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), workspace.hsl_channel == i, name)
                                        });
                                        if response.clicked() {
                                            workspace.hsl_channel = i;
                                        }
                                    }
                                });
                                let arr = match workspace.hsl_channel {
                                    0 => &mut p.hsl_hue,
                                    1 => &mut p.hsl_sat,
                                    _ => &mut p.hsl_lum,
                                };
                                for (k, name) in BANDS.iter().enumerate() {
                                    let grad = [Color32::from_gray(128), hue_color(photocraft_algo::camera_raw::HSL_BANDS[k])];
                                    row(ui, &mut dirty, name, &mut arr[k], -100.0..=100.0, Some(&grad));
                                }
                            }
                        });
                    }
                    "Color Grading" => {
                        super::camera_raw_workspace_ui::section(ui, "Color Grading", &mut workspace.panels, |ui| {
                            dirty |= super::camera_raw_workspace_ui::grading(ui, p, &mut workspace.grading);
                        });
                    }
                    "Detail" => {
                        super::camera_raw_workspace_ui::section(ui, "Detail", &mut workspace.panels, |ui| {
                            dirty |= super::camera_raw_workspace_ui::disclosed_slider(
                                ui,
                                "Sharpening",
                                &mut p.sharpen_amount,
                                0.0..=150.0,
                                &mut workspace.sharpening_details,
                            );
                            if workspace.sharpening_details {
                                let _ = slider_default(ui, &mut dirty, "Radius", &mut p.sharpen_radius, 0.5..=3.0, None, 1.0);
                                let _ = slider_default(ui, &mut dirty, "Detail", &mut p.sharpen_detail, 0.0..=100.0, None, 25.0);
                                row(ui, &mut dirty, "Masking", &mut p.sharpen_masking, 0.0..=100.0, None);
                            }
                            if ui.selectable_label(workspace.noise_details, tl!("Manual Noise Reduction")).clicked() {
                                workspace.noise_details = !workspace.noise_details;
                            }
                            if workspace.noise_details {
                                row(ui, &mut dirty, "Luminance", &mut p.noise_luminance, 0.0..=100.0, None);
                                let _ = slider_default(ui, &mut dirty, "Luminance Detail", &mut p.noise_luminance_detail, 0.0..=100.0, None, 50.0);
                                row(ui, &mut dirty, "Color", &mut p.noise_color, 0.0..=100.0, None);
                                let _ = slider_default(ui, &mut dirty, "Color Detail", &mut p.noise_color_detail, 0.0..=100.0, None, 50.0);
                            }
                        });
                    }
                    _ => {}
                }
            }
        });
        super::camera_raw_scope_ui::floating(ctx, d, scope, right);
        super::camera_raw_scope_ui::shortcuts(ctx, scope);
        if scope.sampler_tool {
            workspace.white_balance_tool = false;
        }
        d.dirty |= dirty;
        if d.dirty {
            ctx.request_repaint();
        }
        ui.add_enabled_ui(!busy_visible, |ui| super::camera_raw_workspace_ui::rail(ui, lay.rail, workspace, navigation, &mut scope.sampler_tool));
        // Section headers paint opaque backgrounds over the dock's earlier edge.
        // ACR keeps one continuous dock boundary through headers and bodies;
        // paint it after the controls instead of outlining each section.
        painter.line_segment([right.left_top(), right.left_bottom()], Stroke::new(1.0, t.separator));
        // Footer.
        let foot = lay.foot;
        painter.rect_filled(foot, 0.0, t.dock);
        painter.line_segment([foot.left_top(), foot.right_top()], Stroke::new(1.0, t.separator));
        let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(foot.shrink2(vec2(16.0, 9.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if busy_visible {
            fu.disable();
        }
        if let Some(role) = widgets::dialog_buttons(
            &mut fu,
            &[
                widgets::DialogButton::new(widgets::ButtonRole::Default, ok_label, 90.0),
                widgets::DialogButton::new(widgets::ButtonRole::Cancel, tl!("Cancel"), 90.0),
            ],
        ) {
            action = Some(if role == widgets::ButtonRole::Default { "ok" } else { "cancel" });
        }
        if d.target == Target::OpenRaw && widgets::secondary_button(&mut fu, tl!("Done"), 70.0).clicked() {
            action = Some("done");
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            let origin = d.edit_origin.take().unwrap_or(frame_origin);
            if origin != d.params {
                let edited = std::mem::replace(&mut d.params, origin);
                remember_edit(d);
                d.params = edited;
            }
        }
    });
    super::camera_raw_scope_ui::persist(app, ctx);
    if ctx.input(|i| i.key_pressed(egui::Key::Y))
        && let Some(d) = app.camera_raw.as_mut()
    {
        d.show_before = !d.show_before;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        action = Some("cancel");
    }
    if busy {
        action = None;
    }
    match action {
        Some("ok") => {
            if let Err(e) = commit(app) {
                app.ui.status = e;
            }
        }
        Some("cancel") => cancel(app),
        Some("done") => {
            if let Err(error) = done(app) {
                app.ui.status = error;
            }
        }
        Some("reset-open" | "reset-default") => {
            let reset = if action == Some("reset-open") { "open" } else { "default" };
            if let Some(Err(error)) = menu_update(app, ctx, &json!({"reset":reset})) {
                app.ui.status = error;
            }
        }
        Some("save") => {
            let r = save_settings(app);
            report_file_error(app, r);
        }
        Some("load") => {
            let r = load_settings(app);
            report_file_error(app, r);
        }
        _ => {}
    }
}

fn hue_color(h: f32) -> Color32 {
    let h6 = (h.rem_euclid(360.0)) / 60.0;
    let x = 1.0 - ((h6 % 2.0) - 1.0).abs();
    let (r, g, b) = match h6 as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    Color32::from_rgb((r * 220.0) as u8, (g * 220.0) as u8, (b * 220.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_entry_points_share_the_reference_shell_without_overlapping_controls() {
        for (width, height) in [(480.0, 400.0), (700.0, 600.0), (1365.0, 767.0), (2560.0, 1440.0)] {
            let screen = ERect::from_min_size(pos2(12.0, 20.0), vec2(width, height));
            let ctx = egui::Context::default();
            let lay = layout(&ctx, screen, false);
            assert_eq!(lay, layout(&ctx, screen, true));
            assert!(screen.contains_rect(lay.rail) && screen.contains_rect(lay.right));
            assert!(lay.view.is_positive() && screen.contains_rect(lay.view));
            assert!(lay.view.right() <= lay.right.left());
            assert!(lay.view.bottom() <= lay.nav.top());
            assert!(lay.nav.bottom() <= lay.foot.top());
            assert_eq!(lay.right.right(), lay.rail.left());
            assert_eq!(lay.rail.right(), screen.right());
            assert!(lay.right.width() <= crate::camera_raw_design::Design::default().panel_width);
        }
    }

    #[test]
    fn preview_texture_uses_linear_document_display_encoding() {
        let mut doc = photocraft_doc::Document::new("test", photocraft_geom::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F32);
        doc.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
        let mut color = photocraft_engine::color_cmds::ColorState::default();
        color.settings.monitor_profile = "srgb".into();
        let display = color.canvas_display(&doc).unwrap();
        let px = [[0.25, 0.25, 0.25, 1.0]];
        let preview = CameraRawDialog::image(&px, 1, 1, &display);
        let expected = display.to_rgba8(&photocraft_compose::Buffer::filled(Rect::new(0, 0, 1, 1), px[0]));
        assert_eq!(preview.pixels[0].to_array(), expected.pixels[..4]);
        assert_ne!(preview.pixels[0].r(), 64, "linear light must be encoded before display");
    }

    #[test]
    fn histogram_follows_preview_and_view_changes_do_not_render_or_edit() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        let steps = app.session.active().unwrap().history.past_len();
        let initial = menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        assert_eq!(initial["histogram"]["samples"], 64 * 48);
        assert_eq!(initial["histogram"]["red"][128], 64 * 48);
        let updated = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0}}})).unwrap().unwrap();
        assert_ne!(updated["histogram"]["red"], initial["histogram"]["red"]);
        let before = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"before": true}})).unwrap().unwrap();
        assert_eq!(before["histogram"]["source"], "before");
        assert_eq!(before["histogram"]["red"], initial["histogram"]["red"]);
        assert_eq!(before["previewRevision"], updated["previewRevision"]);
        let after = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"before": false, "set": {"exposure": 1}}})).unwrap().unwrap();
        assert_eq!(after["histogram"], updated["histogram"]);
        assert_eq!(after["previewRevision"], updated["previewRevision"]);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        let pixel = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(0, 0);
        assert!((pixel[0] - 128.0 / 255.0).abs() < 0.0001);
    }

    /// Closing Camera Raw's window is Cancel. Without a multi-window backend egui embeds the
    /// "window" (`ViewportClass::EmbeddedWindow`), and the close request comes from the window
    /// it is drawn in; the overlay fallback (`embed_viewports`) keeps the dialog open.
    #[test]
    fn closing_the_camera_raw_window_cancels() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        let steps = app.session.active().unwrap().history.past_len();
        let frame = |app: &mut PhotocraftApp, close: bool| {
            let mut input = egui::RawInput::default();
            if close {
                let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
                info.events.push(egui::ViewportEvent::Close);
            }
            ctx.run_ui(input, |ui| show(app, ui.ctx())).textures_delta.clear();
        };
        open(&mut app, &ctx).unwrap();
        frame(&mut app, false);
        assert!(app.camera_raw.is_some(), "drawn over the main window");
        ctx.set_embed_viewports(false);
        frame(&mut app, false);
        assert!(app.camera_raw.is_some(), "drawn in its window");
        frame(&mut app, true);
        assert!(app.camera_raw.is_none(), "closing the window is Cancel");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    }

    #[test]
    fn smart_filter_double_click_edits_the_stored_camera_raw_filter_in_its_own_dialog() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        // An older editor could save stacked curve inputs; the filter must stay editable.
        let legacy = json!([[0.0, 0.0], [60.0, 40.0], [60.0, 200.0], [255.0, 255.0]]);
        app.run("filter.cameraRaw", json!({"exposure": 1.0, "contrast": 20})).unwrap();
        app.run("layer.smartFilter.setParams", json!({"index": 0, "params": {"pointCurve": legacy}})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let smart = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
            photocraft_doc::LayerContent::Smart(sm) => sm.clone(),
            _ => panic!("not a smart object"),
        };
        let steps = app.session.active().unwrap().history.past_len();

        let sm = smart(&app);
        crate::smart_ui::open_editor(&mut app, &ctx, layer.0, &sm, 0);
        assert!(app.ui.dialogs.is_empty(), "no generic parameter form");
        let opened = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {}})).unwrap().unwrap();
        assert_eq!(opened["smartFilter"], 0);
        assert_eq!(opened["params"]["exposure"], 1.0);
        assert_eq!(opened["params"]["pointCurve"], legacy);
        // The preview starts from the pixels below the filter, not the filtered cache.
        let before = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"before": true}})).unwrap().unwrap();
        assert_eq!(before["histogram"]["red"][128], 64 * 48);

        // Resetting a control to its default must overwrite the stored value.
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.0}, "before": false, "commit": true}})).unwrap();
        assert!(r.is_ok(), "{r:?}");
        assert!(app.camera_raw.is_none());
        let sm = smart(&app);
        assert_eq!(sm.smart_filters.len(), 1, "edited in place, not added");
        assert_eq!(sm.smart_filters[0].params["exposure"], 0.0);
        assert_eq!(sm.smart_filters[0].params["contrast"], 20.0);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(smart(&app).smart_filters[0].params["exposure"], 1.0);

        // A broken new curve is still rejected; an unknown index is an error, not a panic.
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": layer.0, "index": 0}})).unwrap().unwrap();
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"pointCurve": [[10, 10]]}}})).unwrap().is_err());
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": layer.0, "index": 7}})).unwrap().is_err());
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": "x"}})).unwrap().is_err());
    }

    #[test]
    fn distant_sparse_pixels_cannot_trigger_an_unbounded_proxy_scan() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width":64,"height":48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let mut source = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
        source.write_region(Rect::new(30_000, 30_000, 30_001, 30_001), &[0.5, 0.5, 0.5, 1.0]);
        let result = open_pixels(
            &mut app,
            &ctx,
            layer,
            source,
            CameraRaw::default(),
            OpenAs { coverage: Coverage::None, target: Target::NewFilter, prepared: None, full_size: None },
        );
        assert!(result.unwrap_err().contains("bounds are too large"));
        assert!(app.camera_raw.is_none());
    }

    #[test]
    fn dialog_drives_the_engine() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        assert_eq!(r["proxy"], json!([64, 48]));
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0, "vignetteAmount": -40}}})).unwrap().unwrap();
        assert_eq!(r["params"]["exposure"], 1.0);
        assert_eq!(app.camera_raw.as_ref().unwrap().command_params(), json!({"exposure": 1.0, "vignetteAmount": -40.0}));
        let before = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 24);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert!(app.camera_raw.is_none());
        let after = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 24);
        assert!(after[0] > before[0] + 0.1, "{before:?} → {after:?}");
        // Cancel leaves the document alone.
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
        assert_eq!(r["cancelled"], true);
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": "x"}}})).unwrap().is_err());
    }

    /// An app whose importer is the real one, opening a synthetic DNG.
    fn raw_app() -> (PhotocraftApp, Vec<u8>) {
        use photocraft_raw::testgen::{DngSpec, mosaic, scene};
        let services = crate::Services {
            import: Some(Box::new(|name: &str, b: &[u8], depth: usize| {
                photocraft_io::import_with_svg_group_depth(name, b, depth).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let (w, h) = (48, 32);
        let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
        spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
        (PhotocraftApp::new(photocraft_engine::Session::new(), services), spec.build())
    }

    fn mean_red(app: &PhotocraftApp) -> f32 {
        let st = app.session.active().unwrap();
        let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        (0..8).map(|i| s.rgba(4 + i * 4, 16)[0]).sum::<f32>() / 8.0
    }

    #[test]
    fn raw_opens_in_camera_raw_and_open_redevelops() {
        let (mut app, dng) = raw_app();
        let ctx = egui::Context::default();
        let warnings = app.open_bytes("IMG_0001.dng", &dng).unwrap();
        assert!(warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE)), "{warnings:?}");
        // A raw has no embedded profile (ProPhoto RGB is the space it is developed into), so Ask
        // When Opening (on by default) has nothing to ask about.
        assert!(app.ui.dialogs.is_empty(), "no profile prompt for a raw");
        let raw = app.pending_raw_open.take().expect("a raw open queues the dialog");
        open_raw(&mut app, &ctx, raw).unwrap();
        let d = app.camera_raw.as_ref().unwrap();
        assert_eq!(d.target, Target::OpenRaw);
        assert_eq!(d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview, &app.ui.camera_raw_workspace)["openingRaw"]["name"], "IMG_0001.dng");
        let before = mean_red(&app);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0, "temperature": 30, "contrast": 20}}})).unwrap().unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert_eq!(r["redeveloped"], true, "{r}");
        assert!(r.get("filter").is_some(), "contrast is applied as a Camera Raw step: {r}");
        assert!(app.camera_raw.is_none());
        assert_eq!(app.session.documents().len(), 1);
        let st = app.session.active().unwrap();
        assert_eq!(st.saved_revision, st.revision, "Open leaves the document unmodified");
        assert_eq!(st.doc.depth, photocraft_color::SampleType::U16);
        assert!(mean_red(&app) > before + 0.05, "{before} → {}", mean_red(&app));
    }

    #[test]
    fn the_recipe_render_is_the_picture_the_dialog_shows() {
        let (mut app, dng) = raw_app();
        let ctx = egui::Context::default();
        app.open_bytes("IMG_0003.dng", &dng).unwrap();
        let raw = app.pending_raw_open.take().unwrap();
        open_raw(&mut app, &ctx, raw).unwrap();
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"shadows": 75, "highlights": -70}}})).unwrap().unwrap();
        let d = app.camera_raw.as_mut().unwrap();
        d.render(&ctx);
        let (shown, (pw, ph)) = (d.processed.clone(), (d.pw, d.ph));
        let (params, developed) = (d.params.clone(), developed(d));
        assert!(developed.curve.is_some());
        let doc = app.session.active().unwrap().doc.clone();
        let rendered = crate::raw_recipe::render_pixels_for_test(&doc, &params, &developed);
        assert_eq!((rendered.rect.width() as usize, rendered.rect.height() as usize), (pw, ph), "a photo this small renders at the preview's size");
        let mean_difference = |a: &[[f32; 4]], b: &[[f32; 4]]| {
            a.iter().zip(b).map(|(a, b)| (0..3).map(|c| (a[c] - b[c]).abs()).sum::<f32>()).sum::<f32>() / (a.len() * 3) as f32 * 255.0
        };
        let difference = mean_difference(&rendered.px, &shown);
        assert!(difference < 1.0, "the render and the dialog differ by {difference:.2} codes on average");
        // Without the raw-stage tone step (as the render was made before), the two differ visibly.
        let without = crate::raw_recipe::render_pixels_for_test(&doc, &params, &crate::raw_recipe::Developed::default());
        assert!(mean_difference(&without.px, &shown) > 3.0 * difference.max(0.5), "{:.2}", mean_difference(&without.px, &shown));
    }

    #[test]
    fn shadows_of_a_raw_open_act_on_the_raw_data_and_the_preview_shows_it() {
        let (mut app, dng) = raw_app();
        let ctx = egui::Context::default();
        app.open_bytes("IMG_0002.dng", &dng).unwrap();
        let raw = app.pending_raw_open.take().unwrap();
        open_raw(&mut app, &ctx, raw).unwrap();
        assert!(app.camera_raw.as_ref().unwrap().raw_curve.is_some(), "the preview knows how the raw was developed");
        let mean = |px: &[[f32; 4]]| px.iter().map(|p| p[0] + p[1] + p[2]).sum::<f32>() / px.len() as f32;
        let preview_before = mean(&app.camera_raw.as_ref().unwrap().processed);
        let before = mean_red(&app);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"shadows": 100}}})).unwrap().unwrap();
        let d = app.camera_raw.as_mut().unwrap();
        d.render(&ctx);
        assert!(mean(&d.processed) > preview_before + 0.01, "{preview_before} → {}", mean(&d.processed));
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert_eq!(r["redeveloped"], true, "{r}");
        assert!(r.get("filter").is_none(), "Shadows acts while developing, not as a later filter step: {r}");
        assert!(mean_red(&app) > before + 0.01, "{before} → {}", mean_red(&app));
    }

    #[test]
    fn clarity_texture_and_dehaze_of_a_raw_open_act_on_the_raw_data() {
        let (mut app, dng) = raw_app();
        let ctx = egui::Context::default();
        app.open_bytes("IMG_0003.dng", &dng).unwrap();
        let raw = app.pending_raw_open.take().unwrap();
        open_raw(&mut app, &ctx, raw).unwrap();
        let before = mean_red(&app);
        let preview_before = app.camera_raw.as_ref().unwrap().processed.clone();
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"clarity": 60, "texture": 40, "dehaze": 50}}})).unwrap().unwrap();
        let d = app.camera_raw.as_mut().unwrap();
        d.render(&ctx);
        assert_ne!(d.processed, preview_before, "the preview shows the raw-stage change");
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert_eq!(r["redeveloped"], true, "{r}");
        assert!(r.get("filter").is_none(), "they act while developing, not as a later filter step: {r}");
        assert!(mean_red(&app) < before, "Dehaze darkens: {before} → {}", mean_red(&app));
    }

    #[test]
    fn background_redevelop_keeps_the_tab_position_and_view() {
        let (mut app, dng) = raw_app();
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 10, "height": 10})).unwrap();
        app.open_bytes("mid.dng", &dng).unwrap();
        app.run("file.new", json!({"width": 12, "height": 12})).unwrap();
        app.sync_views();
        let raw = app.pending_raw_open.take().unwrap();
        let old_id = raw.document;
        assert_eq!(document_index(&app, old_id), Some(1));
        app.ui.views[1].zoom = 3.0;
        open_raw(&mut app, &ctx, raw).unwrap();
        let before = mean_red(&app);
        // The re-develop and the Camera Raw step run as jobs; the UI thread only polls.
        app.background_jobs = true;
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0, "contrast": 20}}})).unwrap().unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert_eq!(r["pending"], true, "{r}");
        assert!(app.camera_raw.is_none());
        for _ in 0..3000 {
            crate::jobs_ui::tick(&mut app, &ctx);
            if app.raw_redevelop.is_none() && !app.session.has_jobs() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.raw_redevelop.is_none() && !app.session.has_jobs(), "the jobs never finished");
        let docs = app.session.documents();
        assert_eq!(docs.len(), 3);
        assert_eq!((docs[0].doc.size.width, docs[2].doc.size.width), (10, 12), "the other tabs keep their places");
        assert_ne!(docs[1].doc.id, old_id, "re-developed");
        assert_eq!(docs[1].doc.depth, photocraft_color::SampleType::U16);
        assert_eq!(docs[1].saved_revision, docs[1].revision, "Open leaves the document unmodified");
        assert_eq!(app.session.active_index(), Some(1));
        assert_eq!(app.ui.views[1].zoom, 3.0, "the view carries over to the re-developed document");
        assert!(mean_red(&app) > before + 0.05, "{before} → {}", mean_red(&app));
    }

    #[test]
    fn cancel_closes_the_raw_and_the_preference_skips_the_dialog() {
        let (mut app, dng) = raw_app();
        let ctx = egui::Context::default();
        app.open_bytes("a.dng", &dng).unwrap();
        let raw = app.pending_raw_open.take().unwrap();
        open_raw(&mut app, &ctx, raw).unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
        assert_eq!(r["cancelled"], true);
        assert!(app.session.documents().is_empty(), "Cancel closes the raw being opened");
        // Unchanged settings: Open keeps the default develop and adds no step.
        app.open_bytes("b.dng", &dng).unwrap();
        let raw = app.pending_raw_open.take().unwrap();
        open_raw(&mut app, &ctx, raw).unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert_eq!(r, json!({"document": 0, "redeveloped": false}));
        // Preference off: raws open straight away.
        app.run("prefs.set", json!({"path": "rawDefaults.openInCameraRaw", "value": false})).unwrap();
        app.open_bytes("c.dng", &dng).unwrap();
        assert!(app.pending_raw_open.is_none());
        // A non-raw never queues the dialog; a closed document is an error, not a panic.
        assert!(
            open_raw(
                &mut app,
                &ctx,
                RawOpen {
                    document: photocraft_doc::DocId(u64::MAX),
                    name: "x".into(),
                    path: None,
                    bytes: None,
                    initial: None,
                    targets: Default::default(),
                    continue_from: None,
                    proxy: None,
                    full_size: None,
                }
            )
            .is_err()
        );
    }

    #[test]
    fn save_and_load_settings_round_trip_through_xmp_files() {
        use crate::file_dialog::FileDialogAnswer;
        use std::sync::{Arc, Mutex};
        type Written = Arc<Mutex<Vec<(String, Vec<u8>)>>>;
        let written: Written = Arc::default();
        let w = written.clone();
        let foreign = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Clarity2012="+25" crs:RedHue="-5"/>
            </rdf:RDF></x:xmpmeta>"#;
        let answers = vec![
            Some(FileDialogAnswer::SaveTo("/out/Look.xmp".into())),
            Some(FileDialogAnswer::Contents("Look.xmp".into(), Vec::new())),
            Some(FileDialogAnswer::Contents("Other.xmp".into(), foreign.to_vec())),
            Some(FileDialogAnswer::Contents("Broken.xmp".into(), b"<nope".to_vec())),
        ];
        let services = crate::Services {
            file_dialog: Some(crate::file_dialog::fake(answers).0),
            write: Some(Box::new(move |p: &str, b: &[u8]| {
                w.lock().unwrap().push((p.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        let frame = |app: &mut PhotocraftApp| app.poll_file_dialog(&egui::Context::default(), None);
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.75, "contrast": -20, "hslSat": [0, 0, 0, 30, 0, 0, 0, 0]}}}))
            .unwrap()
            .unwrap();
        let saved_params = app.camera_raw.as_ref().unwrap().params.clone();
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"saveSettings": true, "before": true}})).unwrap().is_err());
        assert_eq!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"saveSettings": true}})).unwrap().unwrap()["fileDialog"], "save");
        frame(&mut app);
        let (path, bytes) = written.lock().unwrap()[0].clone();
        assert_eq!(path, "/out/Look.xmp");
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains("crs:Exposure2012=\"+0.75\"") && text.contains("crs:SaturationAdjustmentGreen=\"+30\""), "{text}");
        // Reset, then load the saved file back (the fake dialog's second answer, with its bytes).
        app.camera_raw.as_mut().unwrap().params = CameraRaw::default();
        let mut svc = crate::file_dialog::fake(vec![Some(FileDialogAnswer::Contents("Look.xmp".into(), bytes))]).0;
        std::mem::swap(app.services.file_dialog.as_mut().unwrap(), &mut svc);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
        frame(&mut app);
        assert_eq!(app.camera_raw.as_ref().unwrap().params, saved_params);
        std::mem::swap(app.services.file_dialog.as_mut().unwrap(), &mut svc);
        // The original service's next answer is the empty file: an error, settings untouched.
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
        frame(&mut app);
        assert_eq!(app.camera_raw.as_ref().unwrap().params, saved_params);
        // A foreign preset applies what it holds over the current settings and reports the rest.
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
        frame(&mut app);
        let p = &app.camera_raw.as_ref().unwrap().params;
        assert_eq!((p.clarity, p.exposure), (25.0, 0.75));
        assert!(app.ui.status.contains("Other.xmp") && app.ui.status.contains('1'), "{}", app.ui.status);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
        frame(&mut app);
        assert_eq!(app.camera_raw.as_ref().unwrap().params.clarity, 25.0);
    }
}
