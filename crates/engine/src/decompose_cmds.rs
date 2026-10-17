//! Decompose into Layers (Properties › Quick Actions): a remote AI service splits the image into
//! separate layers (`photocraft-ml`'s [`RemoteClient`]; Qwen-Image-Layered on the reference
//! server). The job takes minutes, so it is a background job: it shows progress ("Waiting in the
//! queue", "Generating layers: step 12/40…"), Esc cancels it on the server too, and the result is
//! added as one undo step.
//!
//! * `generative`: the model's own RGBA layers, including content completed behind occluders. The
//!   colours are synthesised, so the stack only approximates the photo.
//! * `mask`: the model's alpha masks over copies of the original pixel layer (edges snapped to the
//!   image with Refine Edge), so no pixel is invented.
//!
//! The layers arrive in a group above the source layer, which is hidden (not deleted), so Undo or
//! unhiding it brings the original back. Nothing is sent unless the user set a service up
//! (Preferences › AI Integrations); see [`crate::remote`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use photocraft_algo::matting::{self, RefineParams};
use photocraft_algo::segment::scale_for;
use photocraft_algo::selection::Region;
use photocraft_doc::{Document, Layer, LayerContent, LayerId, LayerMask};
use photocraft_geom::Rect;
use photocraft_ml::{DecomposeMode, DecomposeParams, Decomposition, JobState, RemoteClient, RemoteConfig};
use photocraft_color::PixelFormat;
use photocraft_raster::{Surface, from_rgba_into};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::jobs::JobCtx;
use crate::smartselect_cmds::with_doc_sampler;
use crate::{EngineError, Result, Session};

const CMD: &str = "layer.decompose";
const LABEL: &str = "Decompose into Layers";

/// Working-resolution budget (pixels) of the image uploaded (the service resizes it to its
/// generation size anyway).
const UPLOAD_PX: usize = 1_600_000;
/// A job that has not finished by then is given up on (and cancelled on the server).
const DEADLINE: Duration = Duration::from_secs(40 * 60);
/// Consecutive failed status checks tolerated before giving up (a network blip is not a failure).
pub(crate) const MAX_POLL_FAILURES: u32 = 6;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

pub(crate) fn remote_error(e: photocraft_ml::Error) -> EngineError {
    EngineError::Other(format!("AI service: {e}"))
}

/// How often the job is polled (fast in tests).
pub(crate) fn poll_interval() -> Duration {
    // Unit tests poll a loopback mock quickly; a live test against a real service must not.
    if cfg!(test) && std::env::var("PHOTOCRAFT_REMOTE_TEST").as_deref() != Ok("1") { Duration::from_millis(5) } else { Duration::from_millis(1500) }
}

/// Why Decompose into Layers cannot run now, in words for the user.
fn unavailable() -> Option<String> {
    if !crate::remote::settings().use_for_layers {
        return Some(format!("{LABEL} is switched off in Preferences › AI Integrations"));
    }
    crate::remote::unavailable_reason().map(|why| format!("{LABEL}: {why}"))
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    s.active().ok_or("no document open")?;
    unavailable().map_or(Ok(()), Err)
}

/// What the worker hands to the apply step: finished layers, back to front.
struct Done {
    layers: Vec<Layer>,
    mode: DecomposeMode,
    composite_error: Option<f32>,
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let source = layer_param(s, p)?;
    let cfg = crate::remote::layers_config().ok_or_else(|| EngineError::Other(unavailable().unwrap_or_else(|| format!("{LABEL} is not available"))))?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = Arc::clone(&d.doc);
    let src = doc.layer(source).ok_or(EngineError::NoLayer(source))?;
    let (source_name, source_is_raster) = (src.name.clone(), matches!(src.content, LayerContent::Raster(_)));

    let a = &s.prefs().integrations;
    let num = |k: &str, default: u32, lo: u32, hi: u32| -> Result<u32> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v.as_u64().filter(|n| (u64::from(lo)..=u64::from(hi)).contains(n)).map(|n| n as u32).ok_or_else(|| bad(format!("`{k}` must be a whole number from {lo} to {hi}"))),
        }
    };
    let layers = num("layers", a.decompose_layers, 2, 10)?;
    let steps = num("steps", a.decompose_steps, 10, 60)?;
    let resolution = match p.get("resolution") {
        None | Some(Value::Null) => a.decompose_resolution.name().parse::<u32>().unwrap_or(640),
        Some(v) => v.as_u64().filter(|r| matches!(r, 640 | 1024)).map(|r| r as u32).ok_or_else(|| bad("`resolution` must be 640 or 1024"))?,
    };
    let mode = match p.get("mode") {
        None | Some(Value::Null) => DecomposeMode::parse(a.decompose_mode.name()).unwrap_or_default(),
        Some(v) => v.as_str().and_then(DecomposeMode::parse).ok_or_else(|| bad("`mode` must be generative or mask"))?,
    };
    let prompt = match p.get("prompt") {
        None | Some(Value::Null) => String::new(),
        Some(v) => v.as_str().filter(|t| t.chars().count() <= 500).ok_or_else(|| bad("`prompt` must be text of at most 500 characters"))?.to_string(),
    };
    let seed = match p.get("seed") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().ok_or_else(|| bad("`seed` must be a whole number"))?),
    };
    let flag = |k: &str, default: bool| p.get(k).and_then(Value::as_bool).unwrap_or(default);
    let (all_layers, hide_original) = (flag("sampleAllLayers", false), flag("hideOriginal", true));
    if mode == DecomposeMode::Mask && !source_is_raster {
        return Err(bad("mask mode needs a pixel layer (rasterize the layer first)"));
    }
    let params = DecomposeParams { layers, resolution, steps, cfg: 4.0, seed, prompt, mode };

    crate::jobs::run(
        s,
        LABEL,
        true,
        move |ctx| work(ctx, &cfg, &doc, source, all_layers, &params),
        move |s, done| apply(s, done, source, &source_name, hide_original),
    )
}

/// The worker: upload, wait for the service (reporting progress), download, build the layers.
fn work(ctx: &JobCtx, cfg: &RemoteConfig, doc: &Arc<Document>, source: LayerId, all_layers: bool, params: &DecomposeParams) -> Result<Done> {
    ctx.progress(0.0, "Preparing the image");
    let window = doc.bounds();
    let step = scale_for(window, UPLOAD_PX);
    let img = with_doc_sampler(doc, Some(source), all_layers, |smp, _| smp.rgb_scaled(window, step));
    let rgb: Vec<u8> = img.px.iter().flat_map(|p| p.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
    ctx.check()?;
    let jpeg = photocraft_codecs::web::encode_jpeg_rgb8(img.w as u32, img.h as u32, &rgb, 95, false, false, None, None, None).map_err(|e| EngineError::Other(format!("could not prepare the image: {e}")))?;

    let client = RemoteClient::new(cfg.clone());
    ctx.progress(0.01, "Sending the image to the AI service");
    let job = client.submit_decompose(&jpeg, "image/jpeg", params).map_err(remote_error)?;
    log::info!("decompose job submitted ({} layers, {} px, {} steps, {})", params.layers, params.resolution, params.steps, params.mode.name());
    ctx.progress(0.03, "Waiting for the AI service");

    let started = Instant::now();
    let mut failures = 0;
    loop {
        if ctx.cancelled() {
            // Free the server's GPU: a cancelled job would otherwise run to the end.
            let _ = client.cancel_job(&job);
            return Err(EngineError::Cancelled);
        }
        if started.elapsed() > DEADLINE {
            let _ = client.cancel_job(&job);
            return Err(EngineError::Other("the AI service took too long (over 40 minutes); the job was cancelled".into()));
        }
        match client.job_status(&job) {
            Ok(st) => {
                failures = 0;
                match st.state {
                    JobState::Queued => {
                        let ahead = st.queue_position.map(|n| format!(" ({n} ahead)")).unwrap_or_default();
                        ctx.progress(0.03, &format!("Waiting in the AI service's queue{ahead}"));
                    }
                    JobState::Running => {
                        let frac = if st.steps > 0 { st.step as f32 / st.steps as f32 } else { 0.0 };
                        let eta = st.eta_seconds.filter(|e| *e >= 1.0).map(|e| format!(", about {} left", human_duration(e))).unwrap_or_default();
                        ctx.progress(0.05 + 0.85 * frac.clamp(0.0, 1.0), &format!("Generating layers: step {}/{}{eta}", st.step, st.steps));
                    }
                    JobState::Succeeded => break,
                    JobState::Failed => return Err(EngineError::Other(format!("the AI service could not decompose the image{}", st.error.map(|e| format!(": {e}")).unwrap_or_default()))),
                    JobState::Cancelled | JobState::Expired => return Err(EngineError::Other("the AI service stopped the job".into())),
                }
            }
            Err(e) => {
                failures += 1;
                log::warn!("decompose status check failed ({failures}/{MAX_POLL_FAILURES}): {e}");
                if failures >= MAX_POLL_FAILURES {
                    return Err(remote_error(e));
                }
            }
        }
        // Sleep in short slices so Esc is noticed quickly.
        let until = Instant::now() + poll_interval();
        while Instant::now() < until && !ctx.cancelled() {
            std::thread::sleep(Duration::from_millis(5).min(poll_interval()));
        }
    }

    ctx.progress(0.9, "Downloading the layers");
    let result = client.fetch_decomposition(&job).map_err(remote_error)?;
    // The result is ours now; let the server forget it.
    let _ = client.cancel_job(&job);
    ctx.check()?;
    ctx.progress(0.94, "Building the layers");
    let layers = build_layers(ctx, doc, source, all_layers, window, &result)?;
    Ok(Done { layers, mode: result.mode, composite_error: result.composite_error })
}

pub(crate) fn human_duration(secs: f32) -> String {
    let s = secs.round() as u64;
    if s >= 90 { format!("{} min", s.div_ceil(60)) } else { format!("{s} s") }
}

/// A layer's name: the model's caption, else its place in the stack.
fn layer_name(caption: Option<&str>, z: i32, index: usize) -> String {
    match caption {
        Some(c) => c.chars().take(40).collect(),
        None if index == 0 && z <= 0 => "Background".to_string(),
        None => format!("Layer {}", index + 1),
    }
}

/// Bounding box `[x0, y0, x1, y1)` of the pixels of a `w × h` image whose `channel` of `stride`
/// interleaved channels is above zero; `None` when there are none.
fn content_box(data: &[u8], w: usize, h: usize, stride: usize, channel: usize) -> Option<[usize; 4]> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        let row = data.get(y * w * stride..(y + 1) * w * stride)?;
        for x in 0..w {
            if row[x * stride + channel] > 0 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
}

/// Bilinear sample of an RGBA8 image at generated-pixel position `(fx, fy)` (pixel centres at
/// +0.5), as straight RGBA in `0..=1`. Colours blend premultiplied, so a transparent neighbour's
/// colour never bleeds into an edge.
pub(crate) fn sample_rgba(data: &[u8], w: usize, h: usize, fx: f32, fy: f32) -> [f32; 4] {
    let (fx, fy) = ((fx - 0.5).clamp(0.0, (w - 1) as f32), (fy - 0.5).clamp(0.0, (h - 1) as f32));
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let mut acc = [0.0f32; 4];
    for (x, y, wt) in [(x0, y0, (1.0 - tx) * (1.0 - ty)), (x1, y0, tx * (1.0 - ty)), (x0, y1, (1.0 - tx) * ty), (x1, y1, tx * ty)] {
        let i = (y * w + x) * 4;
        let a = f32::from(data[i + 3]) / 255.0;
        for c in 0..3 {
            acc[c] += f32::from(data[i + c]) / 255.0 * a * wt;
        }
        acc[3] += a * wt;
    }
    if acc[3] > 1e-6 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3].min(1.0)] } else { [0.0; 4] }
}

/// A raster layer holding `rgba` (generated `gw × gh`) stretched over `window`, written band by
/// band so a large document never needs one huge buffer. Only the part around the content is
/// written; the rest stays empty.
fn generative_layer(name: String, fmt: PixelFormat, window: Rect, rgba: &[u8], gw: usize, gh: usize) -> Layer {
    let mut layer = Layer::raster(name, fmt);
    let Some([bx0, by0, bx1, by1]) = content_box(rgba, gw, gh, 4, 3) else { return layer };
    let (ww, wh) = (window.width() as f32, window.height() as f32);
    let to_x = |g: usize, up: bool| window.x0 + if up { (g as f32 * ww / gw as f32).ceil() } else { (g as f32 * ww / gw as f32).floor() } as i32;
    let to_y = |g: usize, up: bool| window.y0 + if up { (g as f32 * wh / gh as f32).ceil() } else { (g as f32 * wh / gh as f32).floor() } as i32;
    // One generated pixel of margin for the bilinear fringe.
    let area = Rect::new(to_x(bx0.saturating_sub(1), false), to_y(by0.saturating_sub(1), false), to_x((bx1 + 1).min(gw), true), to_y((by1 + 1).min(gh), true)).intersect(&window);
    if area.is_empty() {
        return layer;
    }
    let n = fmt.channels();
    let w = area.width() as usize;
    let Some(surface) = layer.surface_mut() else { return layer };
    const BAND: i32 = 128;
    let mut y = area.y0;
    while y < area.y1 {
        let yb = (y + BAND).min(area.y1);
        let mut buf = vec![0.0f32; w * (yb - y) as usize * n];
        let fill = |(row, out): (usize, &mut [f32])| {
            let fy = ((y + row as i32 - window.y0) as f32 + 0.5) * gh as f32 / wh;
            for (x, px) in out.chunks_exact_mut(n).enumerate() {
                let fx = ((area.x0 + x as i32 - window.x0) as f32 + 0.5) * gw as f32 / ww;
                from_rgba_into(&fmt, sample_rgba(rgba, gw, gh, fx, fy), px);
            }
        };
        {
            use rayon::prelude::*;
            buf.par_chunks_mut(w * n).enumerate().for_each(fill);
        }
        surface.write_region(Rect::new(area.x0, y, area.x1, yb), &buf);
        y = yb;
    }
    layer
}

/// The model's 8-bit alpha mask (`gw × gh`) stretched over `window` as a coverage [`Region`]
/// around its content (`None` when it is empty).
fn mask_region_of(grey: &[u8], gw: usize, gh: usize, window: Rect) -> Option<Region> {
    let [bx0, by0, bx1, by1] = content_box(grey, gw, gh, 1, 0)?;
    let (ww, wh) = (window.width() as f32, window.height() as f32);
    let x0 = window.x0 + ((bx0.saturating_sub(1)) as f32 * ww / gw as f32).floor() as i32;
    let y0 = window.y0 + ((by0.saturating_sub(1)) as f32 * wh / gh as f32).floor() as i32;
    let x1 = window.x0 + (((bx1 + 1).min(gw)) as f32 * ww / gw as f32).ceil() as i32;
    let y1 = window.y0 + (((by1 + 1).min(gh)) as f32 * wh / gh as f32).ceil() as i32;
    let bbox = Rect::new(x0, y0, x1, y1).intersect(&window);
    if bbox.is_empty() {
        return None;
    }
    let w = bbox.width() as usize;
    let mut mask = vec![0u8; w * bbox.height() as usize];
    {
        use rayon::prelude::*;
        mask.par_chunks_mut(w).enumerate().for_each(|(row, out)| {
            let fy = ((bbox.y0 + row as i32 - window.y0) as f32 + 0.5) * gh as f32 / wh;
            for (x, o) in out.iter_mut().enumerate() {
                let fx = ((bbox.x0 + x as i32 - window.x0) as f32 + 0.5) * gw as f32 / ww;
                // Bilinear on the single channel (RGBA with the grey as alpha, colour ignored).
                let (gx, gy) = ((fx - 0.5).clamp(0.0, (gw - 1) as f32), (fy - 0.5).clamp(0.0, (gh - 1) as f32));
                let (ix, iy) = (gx.floor() as usize, gy.floor() as usize);
                let (jx, jy) = ((ix + 1).min(gw - 1), (iy + 1).min(gh - 1));
                let (tx, ty) = (gx - ix as f32, gy - iy as f32);
                let at = |x: usize, y: usize| f32::from(grey[y * gw + x]);
                let v = (at(ix, iy) * (1.0 - tx) + at(jx, iy) * tx) * (1.0 - ty) + (at(ix, jy) * (1.0 - tx) + at(jx, jy) * tx) * ty;
                *o = v.round().clamp(0.0, 255.0) as u8;
            }
        });
    }
    Some(Region { bbox, mask })
}

/// Turns the service's layers into document layers, back to front.
fn build_layers(ctx: &JobCtx, doc: &Arc<Document>, source: LayerId, all_layers: bool, window: Rect, d: &Decomposition) -> Result<Vec<Layer>> {
    let fmt = doc.pixel_format();
    let total = d.layers.len().max(1) as f32;
    let mut out = Vec::with_capacity(d.layers.len());
    for (i, l) in d.layers.iter().enumerate() {
        ctx.check()?;
        ctx.progress(0.94 + 0.06 * i as f32 / total, "Building the layers");
        let img = photocraft_codecs::decode(&l.png).map_err(|e| EngineError::Other(format!("AI service: a returned layer could not be read: {e}")))?;
        let (gw, gh) = (img.width() as usize, img.height() as usize);
        if gw == 0 || gh == 0 || gw > 16384 || gh > 16384 {
            return Err(EngineError::Other("AI service: a returned layer has an unusable size".into()));
        }
        let rgba = img.to_rgba8();
        let name = layer_name(l.caption.as_deref(), l.z, i);
        match d.mode {
            DecomposeMode::Generative => out.push(generative_layer(name, fmt, window, &rgba, gw, gh)),
            DecomposeMode::Mask => {
                let grey: Vec<u8> = rgba.chunks_exact(4).map(|p| p[0]).collect();
                let src = doc.layer(source).ok_or(EngineError::NoLayer(source))?;
                let mut layer = src.duplicate();
                layer.name = name;
                layer.mask = match mask_region_of(&grey, gw, gh, window) {
                    Some(region) => {
                        // The generated mask is only as exact as one generated pixel: snap its edge
                        // to the photo, as Object Selection does.
                        let cell = window.width().max(window.height()) as f32 / gw.max(gh) as f32;
                        let params = RefineParams { radius: (cell * 0.75).clamp(2.0, 24.0), smart_radius: true, smooth: 0.0, feather: 0.0, contrast: 0.0, shift_edge: 0.0 };
                        let refined = with_doc_sampler(doc, Some(source), all_layers, |smp, dd| matting::refine_mask(smp, &matting::region_reader(&region), region.bbox, dd.bounds(), &params));
                        let region = refined.unwrap_or(region);
                        Some(LayerMask { surface: matting::region_surface(&region), ..LayerMask::reveal_all() })
                    }
                    // Nothing in this layer: hide everything.
                    None => Some(LayerMask { surface: Surface::new(PixelFormat::GRAY8), ..LayerMask::reveal_all() }),
                };
                out.push(layer);
            }
        }
    }
    Ok(out)
}

/// Adds the layers as a group above the source (one undo step) and hides the source.
fn apply(s: &mut Session, done: Done, source: LayerId, source_name: &str, hide_original: bool) -> Result<Value> {
    let (count, mode, composite_error) = (done.layers.len(), done.mode, done.composite_error);
    let group = Layer::group(format!("{source_name} layers"), done.layers);
    let gid = s.edit(LABEL, move |doc, active| {
        let gid = doc.insert_above(Some(source), group);
        if hide_original && let Some(l) = doc.layer_mut(source) {
            l.visible = false;
        }
        *active = Some(gid);
        Ok(gid)
    })?;
    Ok(json!({ "group": gid.0, "layers": count, "mode": mode.name(), "compositeError": composite_error }))
}

/// Decompose into Layers command spec (a Properties Quick Action; no menu item).
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: LABEL,
        menu: &[],
        shortcut: None,
        params: r##"{"layer":id?,"sampleAllLayers":bool=false,"layers":2..10=prefs,"resolution":640|1024=prefs,"steps":10..60=prefs,"mode":"generative"|"mask"=prefs,"prompt":str?,"seed":int?,"hideOriginal":bool=true} → {group,layers,mode,compositeError} (needs an AI service in Preferences › AI Integrations; a background job taking minutes; adds a group of layers above the source and hides the source)"##,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn png_of(w: u32, h: u32, layout: photocraft_codecs::ChannelLayout, data: Vec<u8>) -> Vec<u8> {
        let img = photocraft_codecs::Image::from_u8(w, h, layout, data).unwrap();
        photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap()
    }

    fn zip_of(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    /// Generated 32 × 24 layers: an opaque blue background and a red disc (RGBA), as the service
    /// returns them in `generative` mode; `mask` mode returns greyscale alpha instead.
    fn result_zip(mode: &str) -> Vec<u8> {
        let (w, h) = (32usize, 24usize);
        let disc = |x: usize, y: usize| ((x as f32 + 0.5 - 16.0).powi(2) + (y as f32 + 0.5 - 12.0).powi(2)).sqrt() < 7.0;
        let (bg, fg) = if mode == "mask" {
            (png_of(32, 24, photocraft_codecs::ChannelLayout::Gray, vec![255; w * h]), png_of(32, 24, photocraft_codecs::ChannelLayout::Gray, (0..w * h).map(|i| if disc(i % w, i / w) { 255 } else { 0 }).collect()))
        } else {
            (
                png_of(32, 24, photocraft_codecs::ChannelLayout::Rgba, (0..w * h).flat_map(|_| [20, 40, 200, 255]).collect()),
                png_of(32, 24, photocraft_codecs::ChannelLayout::Rgba, (0..w * h).flat_map(|i| if disc(i % w, i / w) { [220, 30, 30, 255] } else { [0, 0, 0, 0] }).collect()),
            )
        };
        let manifest = format!(
            r#"{{"mode":"{mode}","generated":{{"width":32,"height":24}},"composite_error":0.02,"layers":[
                {{"file":"layer_00.png","z":0,"bbox":[0,0,32,24],"alpha_coverage":1.0}},
                {{"file":"layer_01.png","z":1,"bbox":[9,5,23,19],"alpha_coverage":0.2,"caption":"red disc"}}]}}"#
        );
        zip_of(&[("manifest.json", manifest.into_bytes()), ("layer_00.png", bg), ("layer_01.png", fg)])
    }

    struct Reply(&'static str, &'static str, Vec<u8>);

    fn json_reply(status: &'static str, body: &str) -> Reply {
        Reply(status, "application/json", body.as_bytes().to_vec())
    }

    fn serve(replies: Vec<Reply>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        serve_with(replies, |_| {})
    }

    /// A loopback server answering the next requests in order; returns each request line.
    /// `on_request` sees each request line before its answer is sent. A request that never
    /// arrives ends the server after 10 s (a failing test must not hang).
    fn serve_with(replies: Vec<Reply>, on_request: impl Fn(&str) + Send + 'static) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.set_nonblocking(true).unwrap();
        let url = format!("http://{}/photocraft/v1", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for Reply(status, ct, body) in replies {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut s = loop {
                    match l.accept() {
                        Ok((s, _)) => break s,
                        Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
                        Err(_) => return seen,
                    }
                };
                s.set_nonblocking(false).unwrap();
                let mut got = Vec::new();
                let mut buf = [0u8; 8192];
                let head_end = loop {
                    if let Some(p) = got.windows(4).position(|w| w == b"\r\n\r\n") {
                        break p + 4;
                    }
                    let n = s.read(&mut buf).unwrap();
                    if n == 0 {
                        break got.len();
                    }
                    got.extend(&buf[..n]);
                };
                let head = String::from_utf8_lossy(&got[..head_end]).to_string();
                let want = head.to_ascii_lowercase().lines().find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                while got.len() < head_end + want {
                    let n = s.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    got.extend(&buf[..n]);
                }
                let line = head.lines().next().unwrap_or("").to_string();
                on_request(&line);
                write!(s, "HTTP/1.1 {status}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                s.write_all(&body).unwrap();
                seen.push(line);
            }
            seen
        });
        (url, h)
    }

    /// A 64 × 48 document whose background is mid grey.
    fn doc_session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        s.edit("paint", |doc, a| {
            let surf = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            surf.fill_rect(Rect::new(0, 0, 64, 48), &[0.5, 0.5, 0.5, 1.0]);
            Ok(())
        })
        .unwrap();
        s
    }

    fn configure(url: &str) {
        crate::remote::set_test_config(RemoteConfig::new(url, "pc_test_key"));
    }

    fn happy_path_replies(mode: &str) -> Vec<Reply> {
        vec![
            json_reply("202 Accepted", r#"{"job_id":"01JTEST","state":"queued"}"#),
            json_reply("200 OK", r#"{"state":"queued","queue_position":1}"#),
            json_reply("200 OK", r#"{"state":"running","step":10,"steps":40,"eta_seconds":30}"#),
            json_reply("200 OK", r#"{"state":"succeeded","step":40,"steps":40}"#),
            Reply("200 OK", "application/zip", result_zip(mode)),
            json_reply("204 No Content", ""),
        ]
    }

    #[test]
    fn generative_layers_land_in_a_group_above_a_hidden_source_and_undo_cleanly() {
        let (url, h) = serve(happy_path_replies("generative"));
        configure(&url);
        let mut s = doc_session();
        let before = s.active().unwrap().doc.layers.len();
        let r = s.execute(CMD, json!({"layers": 2})).unwrap();
        assert_eq!((r["layers"].as_u64(), r["mode"].as_str()), (Some(2), Some("generative")));
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.layers.len(), before + 1, "one group was added");
        let group = doc.layers.iter().find(|l| l.is_group()).unwrap();
        let kids = group.children().unwrap();
        assert_eq!(kids.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), vec!["Background", "red disc"]);
        assert!(!doc.layers[0].visible, "the source is hidden, not deleted");
        // The disc layer is red at the centre and empty at a corner; the background is blue.
        let px = |l: &Layer, x: i32, y: i32| {
            let mut o = [[0.0f32; 4]; 1];
            l.surface().unwrap().read_rgba_into(Rect::new(x, y, x + 1, y + 1), &mut o);
            o[0]
        };
        let disc = px(&kids[1], 32, 24);
        assert!(disc[0] > 0.8 && disc[1] < 0.2 && disc[3] > 0.99, "{disc:?}");
        assert!(px(&kids[1], 2, 2)[3] < 0.01);
        let bg = px(&kids[0], 2, 2);
        assert!(bg[2] > 0.7 && bg[3] > 0.99, "{bg:?}");
        let reqs = h.join().unwrap();
        assert!(reqs[0].starts_with("POST /photocraft/v1/decompose") && reqs[4].contains("/result") && reqs[5].starts_with("DELETE"), "{reqs:?}");
        // One undo step brings the original back.
        s.execute("edit.undo", json!({})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.layers.len(), before);
        assert!(doc.layers[0].visible);
        crate::remote::set_test_config(None);
    }

    #[test]
    fn mask_mode_masks_copies_of_the_original_pixels() {
        let (url, h) = serve(happy_path_replies("mask"));
        configure(&url);
        let mut s = doc_session();
        s.execute(CMD, json!({"layers": 2, "mode": "mask"})).unwrap();
        let doc = &s.active().unwrap().doc;
        let group = doc.layers.iter().find(|l| l.is_group()).unwrap();
        let kids = group.children().unwrap();
        assert_eq!(kids.len(), 2);
        // Copies of the grey original (no pixel invented), with the disc's mask on the second.
        let mut o = [[0.0f32; 4]; 1];
        kids[1].surface().unwrap().read_rgba_into(Rect::new(32, 24, 33, 25), &mut o);
        assert!((o[0][0] - 0.5).abs() < 0.02, "{o:?}");
        let m = &kids[1].mask.as_ref().unwrap().surface;
        let at = |x: i32, y: i32| m.sample_channel(x, y, 0);
        assert!(at(32, 24) > 0.9 && at(2, 2) < 0.1, "{} {}", at(32, 24), at(2, 2));
        h.join().unwrap();
        crate::remote::set_test_config(None);
    }

    #[test]
    fn a_rejected_key_is_a_clean_error_and_the_document_is_unchanged() {
        let (url, h) = serve(vec![json_reply("401 Unauthorized", r#"{"title":"Invalid API key","status":401}"#)]);
        configure(&url);
        let mut s = doc_session();
        let e = s.execute(CMD, json!({})).unwrap_err().to_string();
        assert!(e.contains("401") && e.contains("Invalid API key") && !e.contains("pc_test_key"), "{e}");
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        assert!(s.active().unwrap().doc.layers[0].visible);
        h.join().unwrap();
        crate::remote::set_test_config(None);
    }

    #[test]
    fn a_failed_job_reports_the_services_reason() {
        let (url, h) = serve(vec![json_reply("202 Accepted", r#"{"job_id":"J1"}"#), json_reply("200 OK", r#"{"state":"failed","error":{"title":"Out of memory"}}"#)]);
        configure(&url);
        let mut s = doc_session();
        let e = s.execute(CMD, json!({})).unwrap_err().to_string();
        assert!(e.contains("Out of memory"), "{e}");
        h.join().unwrap();
        crate::remote::set_test_config(None);
    }

    #[test]
    fn cancelling_stops_the_server_job_too() {
        let ctx = JobCtx::new();
        let hook = ctx.clone();
        // The user presses Esc right after the job is submitted.
        let (url, h) = serve_with(vec![json_reply("202 Accepted", r#"{"job_id":"J1"}"#), json_reply("204 No Content", "")], move |line| {
            if line.starts_with("POST") {
                hook.cancel();
            }
        });
        let s = doc_session();
        let doc = Arc::clone(&s.active().unwrap().doc);
        let id = doc.layers[0].id;
        let r = work(&ctx, &RemoteConfig::new(&url, "pc_test_key").unwrap(), &doc, id, false, &DecomposeParams::default());
        assert!(matches!(r, Err(EngineError::Cancelled)), "{:?}", r.map(|d| d.layers.len()));
        let seen = h.join().unwrap();
        assert!(seen.len() == 2 && seen[0].starts_with("POST") && seen[1].starts_with("DELETE /photocraft/v1/jobs/J1"), "{seen:?}");
    }

    #[test]
    fn it_is_disabled_and_explains_when_no_service_is_set_up() {
        crate::remote::set_test_config(None);
        let s = doc_session();
        let why = enabled(&s).unwrap_err();
        assert!(why.contains("AI Integrations"), "{why}");
        let mut s = s;
        let e = s.execute(CMD, json!({})).unwrap_err().to_string();
        assert!(e.contains("AI Integrations"), "{e}");
    }

    #[test]
    fn bad_params_are_rejected_before_anything_is_sent() {
        configure("http://127.0.0.1:1/photocraft/v1");
        let mut s = doc_session();
        for p in [json!({"layers": 1}), json!({"layers": 11}), json!({"steps": 5}), json!({"resolution": 800}), json!({"mode": "magic"}), json!({"seed": -1}), json!({"prompt": 5})] {
            assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
        }
        crate::remote::set_test_config(None);
    }

    /// Runs Decompose into Layers against the real service and dumps each layer for viewing.
    /// `PHOTOCRAFT_REMOTE_TEST=1 PHOTOCRAFT_REMOTE_URL=… PHOTOCRAFT_REMOTE_KEY=… PHOTOCRAFT_TEST_IMAGE=photo.jpg
    /// PHOTOCRAFT_TEST_OUT=dir [PHOTOCRAFT_TEST_MODE=mask] cargo test -p photocraft-engine --release --lib decompose_live -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a remote AI service and a photo"]
    fn decompose_live_dump() {
        let (Ok(img), Ok(out)) = (std::env::var("PHOTOCRAFT_TEST_IMAGE"), std::env::var("PHOTOCRAFT_TEST_OUT")) else { return };
        let mode = std::env::var("PHOTOCRAFT_TEST_MODE").unwrap_or_else(|_| "generative".into());
        let mut s = Session::new();
        s.execute("file.openAs", json!({"path": img})).unwrap();
        let t = Instant::now();
        let r = s.execute(CMD, json!({"layers": 4, "steps": 10, "resolution": 640, "mode": mode, "seed": 1})).unwrap();
        eprintln!("decompose: {r} in {:.0} s", t.elapsed().as_secs_f32());
        std::fs::create_dir_all(&out).unwrap();
        let doc = &s.active().unwrap().doc;
        let b = doc.bounds();
        let dump = |name: &str, d: &Document| {
            let px = photocraft_compose::render(d, b).px;
            let mut bytes = Vec::new();
            bytes.extend(b.width().to_le_bytes());
            bytes.extend(b.height().to_le_bytes());
            for p in &px {
                bytes.extend(p.iter().map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
            }
            std::fs::write(std::path::Path::new(&out).join(name), bytes).unwrap();
        };
        let group = doc.layers.iter().find(|l| l.is_group()).unwrap();
        eprintln!("group `{}`: {:?}", group.name, group.children().unwrap().iter().map(|k| k.name.clone()).collect::<Vec<_>>());
        // Each child alone, then the stack.
        for i in 0..group.children().unwrap().len() {
            let mut solo = (**doc).clone();
            for l in &mut solo.layers {
                if let Some(kids) = l.children_mut() {
                    for (j, k) in kids.iter_mut().enumerate() {
                        k.visible = j == i;
                    }
                } else {
                    l.visible = false;
                }
            }
            dump(&format!("layer_{i}.raw"), &solo);
        }
        dump("stack.raw", doc);
    }

    #[test]
    fn resampling_keeps_colours_clean_at_soft_edges() {
        // A red pixel next to a fully transparent black one: the edge stays red, never dark.
        let data = [255, 0, 0, 255, 0, 0, 0, 0];
        let p = sample_rgba(&data, 2, 1, 1.0, 0.5);
        assert!(p[0] > 0.99 && p[1] < 0.01 && (p[3] - 0.5).abs() < 0.01, "{p:?}");
        assert_eq!(sample_rgba(&[0, 0, 0, 0], 1, 1, 0.5, 0.5), [0.0; 4]);
        assert_eq!(content_box(&[0, 0, 0, 255, 0, 0, 0, 0], 2, 1, 4, 3), Some([0, 0, 1, 1]));
        assert_eq!(content_box(&[0; 8], 2, 1, 4, 3), None);
    }
}
