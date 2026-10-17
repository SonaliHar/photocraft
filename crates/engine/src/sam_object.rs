//! Object Selection through SAM 2 (`photocraft-ml`): a click or a rectangle becomes a prompt, the
//! model answers with a mask, and the mask's edge is snapped to the image with Refine Edge.
//!
//! Providers, in order: a **remote** service when the user configured one (`PHOTOCRAFT_REMOTE_URL`
//! and `PHOTOCRAFT_REMOTE_KEY`; the working image, about 1 MP as JPEG, is uploaded once per
//! document state), then the **local** model. Both are optional: [`find`] returns `None` when
//! neither answers (ONNX Runtime or the model files missing, `PHOTOCRAFT_SAM2=0`, the remote down),
//! and the callers fall back to the classical GrabCut path. After a remote failure the remote is
//! skipped for [`REMOTE_COOLDOWN`] so hovering never stalls on a dead connection. The image
//! embedding (the expensive half, ~0.1–0.2 s on a GPU) is cached for the document's pixels, so
//! after the first object every further click or hover costs only the decoder (~6 ms).
//!
//! * `PHOTOCRAFT_SAM2=0` turns it off; `PHOTOCRAFT_SAM2_SIZE=tiny|small|base_plus|large` picks the
//!   model (default `small`); `PHOTOCRAFT_ML_DIR` is the assets folder (see `photocraft_ml::Assets`).

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use photocraft_algo::matting::{self, RefineParams};
use photocraft_algo::segment::scale_for;
use photocraft_algo::selection::Region;
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_geom::Rect;
pub use photocraft_ml::Prompt;
use photocraft_ml::{Assets, Backend, Embedding, ModelSize, Sam2};
#[cfg(not(target_arch = "wasm32"))]
use photocraft_ml::{RemoteClient, SegmentPrompt};

use crate::smartselect_cmds::with_doc_sampler;

/// Working-resolution budget (pixels) of the image given to the encoder; it resizes to 1024².
const WORK_PX: usize = 1_100_000;

static MODEL: OnceLock<Option<Arc<Sam2>>> = OnceLock::new();

fn enabled() -> bool {
    let v = std::env::var("PHOTOCRAFT_SAM2").unwrap_or_default();
    // The unit tests stay on the deterministic classical path unless a test opts in with `=1`.
    if cfg!(test) { v.trim() == "1" } else { !matches!(v.trim(), "0" | "off" | "false") }
}

/// The loaded model, loading it on first use. `None` when unavailable (never panics).
fn model() -> Option<&'static Arc<Sam2>> {
    MODEL
        .get_or_init(|| {
            if !enabled() {
                return None;
            }
            let size = std::env::var("PHOTOCRAFT_SAM2_SIZE").ok().and_then(|s| ModelSize::parse(&s)).unwrap_or(ModelSize::Small);
            match Assets::locate(size).and_then(|a| Sam2::load(&a)) {
                Ok(m) => {
                    log::info!("SAM 2 {size:?} loaded on {:?}", m.backend());
                    Some(Arc::new(m))
                }
                Err(e) => {
                    log::info!("SAM 2 unavailable, using the classical object finder: {e}");
                    None
                }
            }
        })
        .as_ref()
}

/// Loads the model in the background so the first click does not wait for it (native only).
pub fn warm_up() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            if enabled() {
                let _ = std::thread::Builder::new().name("sam2-warmup".into()).spawn(|| {
                    model();
                });
            }
        });
    }
}

/// Where the model runs, once loaded (`None` = not loaded or unavailable).
pub fn backend() -> Option<Backend> {
    MODEL.get().and_then(|m| m.as_ref()).map(|m| m.backend())
}

/// The encoder's result for one document's pixels.
struct Cached {
    doc_id: DocId,
    doc: Weak<Document>,
    target: Option<LayerId>,
    window: Rect,
    step: usize,
    emb: Arc<Embedding>,
}

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

/// The embedding of `doc` (sampling `target`), encoded now unless cached. Holds the cache lock
/// while encoding so a hover lookup and a click on the same pixels encode once.
fn embedding(sam: &Sam2, doc: &Arc<Document>, layer: Option<LayerId>, all_layers: bool) -> Option<(Arc<Embedding>, Rect, usize)> {
    let target = crate::smartselect_cmds::sampled_target(doc, layer, all_layers);
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(c) = cache.as_ref()
        && c.doc_id == doc.id
        && c.target == target
        && c.doc.upgrade().is_some_and(|old| crate::layer_multi_cmds::same_pixels(&old, doc))
    {
        let hit = (Arc::clone(&c.emb), c.window, c.step);
        if let Some(c) = cache.as_mut() {
            c.doc = Arc::downgrade(doc);
        }
        return Some(hit);
    }
    *cache = None;
    let window = doc.bounds();
    if window.width() < 4 || window.height() < 4 {
        return None;
    }
    let step = scale_for(window, WORK_PX);
    let img = with_doc_sampler(doc, layer, all_layers, |smp, _| smp.rgb_scaled(window, step));
    let emb = Arc::new(sam.encode(&img.px, img.w, img.h).map_err(|e| log::warn!("SAM 2 encode failed: {e}")).ok()?);
    *cache = Some(Cached { doc_id: doc.id, doc: Arc::downgrade(doc), target, window, step, emb: Arc::clone(&emb) });
    Some((emb, window, step))
}

/// A click whose mask covers more than this share of the canvas is the backdrop, not an object
/// (the classical finder's rule, so hovering over empty background highlights nothing).
const BACKDROP_SHARE: f32 = 0.6;

/// How long the remote provider is skipped after a failed request.
pub const REMOTE_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(60);

/// Which provider answered last (for the UI and tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    /// The configured remote service.
    Remote,
    /// The local SAM 2 model.
    Local,
    /// The built-in GrabCut finder (no model available).
    Classical,
}

impl Provider {
    /// Short text for the options bar.
    pub fn label(self) -> &'static str {
        match self {
            Self::Remote => "Remote service (SAM 2.1)",
            Self::Local => "Local model (SAM 2)",
            Self::Classical => "Built-in finder",
        }
    }
}

static LAST_PROVIDER: AtomicU8 = AtomicU8::new(0);

/// Records the provider of the latest answer; the first answer and every change are logged, so
/// the log file says what answered (and when it fell back).
pub(crate) fn set_provider(p: Provider) {
    let code = match p {
        Provider::Remote => 1,
        Provider::Local => 2,
        Provider::Classical => 3,
    };
    if LAST_PROVIDER.swap(code, Ordering::Relaxed) != code {
        log::info!("Object Selection answered by: {}", p.label());
    }
}

/// The provider that produced the most recent object, if any has.
pub fn last_provider() -> Option<Provider> {
    match LAST_PROVIDER.load(Ordering::Relaxed) {
        1 => Some(Provider::Remote),
        2 => Some(Provider::Local),
        3 => Some(Provider::Classical),
        _ => None,
    }
}

/// Mask logits (positive = object) on a grid covering the encoded image.
struct Grid {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

impl Grid {
    fn from_sam(m: &photocraft_ml::SamMask) -> Self {
        Self { w: photocraft_ml::MASK_SIZE, h: photocraft_ml::MASK_SIZE, v: m.logits.clone() }
    }

    /// An 8-bit mask (128+ = object) as logits of ±8, so bilinear sampling keeps a soft edge.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn from_u8(px: &[u8], w: usize, h: usize) -> Option<Self> {
        (w > 0 && h > 0 && w.checked_mul(h) == Some(px.len())).then(|| Self { w, h, v: px.iter().map(|p| (f32::from(*p) / 255.0 - 0.5) * 16.0).collect() })
    }

    /// Bilinear logit at `(u, v)` in `0..=1` of the encoded image.
    fn sample(&self, u: f32, v: f32) -> f32 {
        let (fx, fy) = ((u * self.w as f32 - 0.5).clamp(0.0, (self.w - 1) as f32), (v * self.h as f32 - 0.5).clamp(0.0, (self.h - 1) as f32));
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |x: usize, y: usize| self.v.get(y * self.w + x).copied().unwrap_or(-8.0);
        (at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty
    }
}

/// The object `prompts` (document pixels) point at: `None` when no provider is available (the
/// caller uses the classical algorithm), `Some(None)` when one found no object.
pub fn find(doc: &Arc<Document>, layer: Option<LayerId>, all_layers: bool, prompts: &[Prompt]) -> Option<Option<Region>> {
    if let Some(found) = find_remote(doc, layer, all_layers, prompts) {
        set_provider(Provider::Remote);
        return Some(found);
    }
    let found = find_local(doc, layer, all_layers, prompts)?;
    set_provider(Provider::Local);
    Some(found)
}

fn find_local(doc: &Arc<Document>, layer: Option<LayerId>, all_layers: bool, prompts: &[Prompt]) -> Option<Option<Region>> {
    let sam = model()?;
    let (emb, window, step) = embedding(sam, doc, layer, all_layers)?;
    // Prompts into the encoded image's pixels.
    let k = 1.0 / step as f32;
    let local: Vec<Prompt> = prompts
        .iter()
        .map(|p| match *p {
            Prompt::Point { x, y, positive } => Prompt::Point { x: (x - window.x0 as f32) * k, y: (y - window.y0 as f32) * k, positive },
            Prompt::Box { x0, y0, x1, y1 } => Prompt::Box { x0: (x0 - window.x0 as f32) * k, y0: (y0 - window.y0 as f32) * k, x1: (x1 - window.x0 as f32) * k, y1: (y1 - window.y0 as f32) * k },
        })
        .collect();
    let masks = sam.decode(&emb, &local).map_err(|e| log::warn!("SAM 2 decode failed: {e}")).ok()?;
    let Some(best) = masks.iter().max_by(|a, b| a.iou.total_cmp(&b.iou)) else { return Some(None) };
    Some(finish(&Grid::from_sam(best), prompts, window, emb.width * step, emb.height * step, doc, layer, all_layers))
}

/// Turns a mask grid into the finished selection region: backdrop rejection, full-resolution
/// coverage, then Refine Edge. `span_*` is the document-pixel size the encoded image covers (its
/// last working pixel may reach past the canvas).
#[allow(clippy::too_many_arguments)]
fn finish(grid: &Grid, prompts: &[Prompt], window: Rect, span_x: usize, span_y: usize, doc: &Arc<Document>, layer: Option<LayerId>, all_layers: bool) -> Option<Region> {
    if prompts.iter().all(|p| matches!(p, Prompt::Point { .. })) {
        let inside = grid.v.iter().filter(|v| **v > 0.0).count();
        if inside as f32 > BACKDROP_SHARE * grid.v.len() as f32 {
            return None;
        }
    }
    let (span_x, span_y) = (span_x as f32, span_y as f32);
    let coarse = mask_region(grid, window, span_x, span_y)?;
    // Snap the edge to the image: the mask is only as exact as one grid cell; Refine Edge's guided
    // band recovers hair-fine detail.
    let cell = span_x.max(span_y) / grid.w.max(grid.h) as f32;
    let params = RefineParams { radius: (cell * 0.75).clamp(2.0, 24.0), smart_radius: true, smooth: 0.0, feather: 0.0, contrast: 0.0, shift_edge: 0.0 };
    let refined = with_doc_sampler(doc, layer, all_layers, |smp, d| matting::refine_mask(smp, &matting::region_reader(&coarse), coarse.bbox, d.bounds(), &params));
    Some(refined.unwrap_or(coarse))
}

/// The grid's bilinear logits sampled per document pixel inside its positive bounding box, as an
/// anti-aliased coverage [`Region`].
fn mask_region(grid: &Grid, window: Rect, span_x: f32, span_y: f32) -> Option<Region> {
    let (nx, ny) = (grid.w, grid.h);
    // Bounding cells of the positive logits, padded by one cell for the bilinear fringe.
    let (mut cx0, mut cy0, mut cx1, mut cy1) = (nx, ny, 0usize, 0usize);
    for (i, v) in grid.v.iter().enumerate() {
        if *v > 0.0 {
            let (x, y) = (i % nx, i / nx);
            (cx0, cy0, cx1, cy1) = (cx0.min(x), cy0.min(y), cx1.max(x + 1), cy1.max(y + 1));
        }
    }
    if cx1 <= cx0 || cy1 <= cy0 {
        return None;
    }
    let to_x = |c: usize| window.x0 + (c as f32 / nx as f32 * span_x).round() as i32;
    let to_y = |c: usize| window.y0 + (c as f32 / ny as f32 * span_y).round() as i32;
    let bbox = Rect::new(to_x(cx0.saturating_sub(1)), to_y(cy0.saturating_sub(1)), to_x((cx1 + 1).min(nx)), to_y((cy1 + 1).min(ny))).intersect(&window);
    let (w, h) = (bbox.width() as usize, bbox.height() as usize);
    if w == 0 || h == 0 || w.checked_mul(h).is_none_or(|n| n > 1 << 31) {
        return None;
    }
    let row = |y: usize, out: &mut [u8]| {
        let v = ((bbox.y0 - window.y0) as f32 + y as f32 + 0.5) / span_y;
        for (x, o) in out.iter_mut().enumerate() {
            let u = ((bbox.x0 - window.x0) as f32 + x as f32 + 0.5) / span_x;
            // A logit step of about 0.5 per pixel covers the edge: coverage 0..1 over ±1 logit.
            *o = ((grid.sample(u, v) * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    };
    let mut data = vec![0u8; w * h];
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        data.par_chunks_mut(w).enumerate().for_each(|(y, r)| row(y, r));
    }
    #[cfg(target_arch = "wasm32")]
    data.chunks_mut(w).enumerate().for_each(|(y, r)| row(y, r));
    Some(Region { bbox, mask: data })
}

// ---- the remote provider (native only: it needs a clock and sockets)

#[cfg(target_arch = "wasm32")]
fn find_remote(_: &Arc<Document>, _: Option<LayerId>, _: bool, _: &[Prompt]) -> Option<Option<Region>> {
    None
}

/// The uploaded working image of one document state.
#[cfg(not(target_arch = "wasm32"))]
struct RemoteCached {
    doc_id: DocId,
    doc: Weak<Document>,
    target: Option<LayerId>,
    window: Rect,
    step: usize,
    w: usize,
    h: usize,
    /// Encoded once and sent as-is each time, so the service's embedding cache (keyed by the
    /// image hash) hits on every later click.
    jpeg: Arc<Vec<u8>>,
}

/// What [`remote_image`] hands out: the encoded upload and where it came from.
#[cfg(not(target_arch = "wasm32"))]
struct RemoteImage {
    jpeg: Arc<Vec<u8>>,
    window: Rect,
    step: usize,
    w: usize,
    h: usize,
}

#[cfg(not(target_arch = "wasm32"))]
static REMOTE_CACHE: Mutex<Option<RemoteCached>> = Mutex::new(None);
#[cfg(not(target_arch = "wasm32"))]
static REMOTE_PAUSED_UNTIL: Mutex<Option<std::time::Instant>> = Mutex::new(None);

#[cfg(not(target_arch = "wasm32"))]
fn remote_image(doc: &Arc<Document>, layer: Option<LayerId>, all_layers: bool) -> Option<RemoteImage> {
    let target = crate::smartselect_cmds::sampled_target(doc, layer, all_layers);
    let mut cache = REMOTE_CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(c) = cache.as_mut()
        && c.doc_id == doc.id
        && c.target == target
        && c.doc.upgrade().is_some_and(|old| crate::layer_multi_cmds::same_pixels(&old, doc))
    {
        c.doc = Arc::downgrade(doc);
        return Some(RemoteImage { jpeg: Arc::clone(&c.jpeg), window: c.window, step: c.step, w: c.w, h: c.h });
    }
    *cache = None;
    let window = doc.bounds();
    if window.width() < 4 || window.height() < 4 {
        return None;
    }
    let step = scale_for(window, WORK_PX);
    let img = with_doc_sampler(doc, layer, all_layers, |smp, _| smp.rgb_scaled(window, step));
    let rgb: Vec<u8> = img.px.iter().flat_map(|p| p.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
    let jpeg = photocraft_codecs::web::encode_jpeg_rgb8(img.w as u32, img.h as u32, &rgb, 92, false, false, None, None, None).map_err(|e| log::warn!("remote upload encode failed: {e}")).ok()?;
    let jpeg = Arc::new(jpeg);
    *cache = Some(RemoteCached { doc_id: doc.id, doc: Arc::downgrade(doc), target, window, step, w: img.w, h: img.h, jpeg: Arc::clone(&jpeg) });
    Some(RemoteImage { jpeg, window, step, w: img.w, h: img.h })
}

/// Asks the remote service. `None` when none is configured, it is cooling down, or the request
/// failed (the caller then tries the local model); `Some(None)` when it found no object.
#[cfg(not(target_arch = "wasm32"))]
fn find_remote(doc: &Arc<Document>, layer: Option<LayerId>, all_layers: bool, prompts: &[Prompt]) -> Option<Option<Region>> {
    let client = RemoteClient::new(crate::remote::selection_config()?);
    if REMOTE_PAUSED_UNTIL.lock().unwrap_or_else(PoisonError::into_inner).is_some_and(|t| std::time::Instant::now() < t) {
        return None;
    }
    let RemoteImage { jpeg, window, step, w, h } = remote_image(doc, layer, all_layers)?;
    let k = 1.0 / step as f32;
    let sent: Vec<SegmentPrompt> = prompts
        .iter()
        .map(|p| match *p {
            Prompt::Point { x, y, positive } => SegmentPrompt::Point { x: (x - window.x0 as f32) * k, y: (y - window.y0 as f32) * k, positive },
            Prompt::Box { x0, y0, x1, y1 } => SegmentPrompt::Box { x0: (x0 - window.x0 as f32) * k, y0: (y0 - window.y0 as f32) * k, x1: (x1 - window.x0 as f32) * k, y1: (y1 - window.y0 as f32) * k },
        })
        .collect();
    let masks = match client.segment(&jpeg, "image/jpeg", &sent) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("remote segmentation failed, using the local model for {}s: {e}", REMOTE_COOLDOWN.as_secs());
            *REMOTE_PAUSED_UNTIL.lock().unwrap_or_else(PoisonError::into_inner) = Some(std::time::Instant::now() + REMOTE_COOLDOWN);
            return None;
        }
    };
    let Some(best) = masks.iter().max_by(|a, b| a.score.total_cmp(&b.score)) else { return Some(None) };
    let grid = photocraft_codecs::decode(&best.png).ok().and_then(|img| {
        let (mw, mh) = (img.width() as usize, img.height() as usize);
        let grey: Vec<u8> = img.to_rgba8().chunks_exact(4).map(|p| p[0]).collect();
        Grid::from_u8(&grey, mw, mh)
    });
    let Some(grid) = grid else {
        log::warn!("remote segmentation returned an unreadable mask");
        return None;
    };
    Some(finish(&grid, prompts, window, w * step, h * step, doc, layer, all_layers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u8_masks_become_signed_grids() {
        let g = Grid::from_u8(&[0, 255, 255, 0], 2, 2).unwrap();
        assert!(g.v[0] < -7.0 && g.v[1] > 7.0);
        assert!(g.sample(0.75, 0.25) > 0.0 && g.sample(0.25, 0.25) < 0.0);
        assert!(Grid::from_u8(&[0; 3], 2, 2).is_none() && Grid::from_u8(&[], 0, 0).is_none());
    }

    #[test]
    fn no_provider_means_the_classical_path() {
        // The test build never loads a model or reaches a network, so `find` reports "unavailable".
        let mut s = crate::Session::new();
        s.execute("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
        let doc = Arc::clone(&s.active().unwrap().doc);
        assert!(find(&doc, None, false, &[Prompt::Point { x: 10.0, y: 10.0, positive: true }]).is_none());
    }
}
