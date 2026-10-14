//! The per-photo Camera Raw recipe of the Camera Raw workspace: the photo's settings kept beside it, as
//! Photoshop keeps them in an XMP sidecar, and a screen-sized render of them for a host to show.
//!
//! The recipe belongs to the photo, not to a document: Camera Raw opens the original raw with it every
//! time, and what the full editor makes of the photo is a separate document. Leaving a photo ("Done")
//! writes the recipe, and renders it at screen size in the background so the photo can be shown with its
//! adjustments the moment it is visited again. Both are written through the host's write authority, so
//! nothing reaches outside the folder the host granted.
//!
//! The recipe is the edit itself: it is written before the dialog lets go of the settings, and a write that
//! fails is an error for the caller, which leaves the dialog as it was. The render is a cache: its failures
//! are only logged.

use photocraft_algo::camera_raw::CameraRaw;
use photocraft_doc::{DocId, Document};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Version of the recipe file.
pub const VERSION: u64 = 1;
/// Longest side and JPEG quality of the render.
pub const RENDER_SIDE: u32 = 2560;
pub const RENDER_QUALITY: u8 = 90;
/// The render is a cache, so it waits for the photo being opened next to get out of the way: it starts this long
/// after the photo is left, one at a time, on one thread, and is dropped if a newer render of the same photo was asked for.
#[cfg(test)]
const RENDER_DELAY_MS: u64 = 0;
#[cfg(not(test))]
const RENDER_DELAY_MS: u64 = 1500;

/// Where a host wants a photo's recipe and render written (paths under its write root).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Targets {
    pub recipe: Option<String>,
    pub display: Option<String>,
    /// The photo's render manifest: the render is then written under a name of its own and published in it
    /// (see `render_manifest`).
    pub manifest: Option<crate::render_manifest::Request>,
}

fn recipe_bytes(params: &CameraRaw) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(&to_json(params)).map_err(|e| e.to_string())
}

/// The recipe file's content for `params`: only the settings that differ from the defaults.
///
/// `identity` says whether the settings change the photo at all (some, such as a sharpening radius with no amount, do
/// not on their own), so a host can tell an edited photo from one whose recipe is only the defaults without knowing
/// the settings' rules. Readers of this version that do not know it ignore it.
pub fn to_json(params: &CameraRaw) -> Value {
    json!({"version": VERSION, "cameraRaw": crate::camera_raw_ui::non_defaults(params), "identity": params.is_identity()})
}

/// The settings in a recipe file's content. Unknown versions, wrong types and out-of-range values are
/// errors, never silently replaced by defaults.
pub fn from_json(value: &Value) -> Result<CameraRaw, String> {
    match value.get("version").and_then(Value::as_u64) {
        Some(VERSION) => {}
        Some(other) => return Err(format!("unsupported Camera Raw recipe version {other}")),
        None => return Err("a Camera Raw recipe needs a version".into()),
    }
    let settings = value.get("cameraRaw").ok_or("a Camera Raw recipe needs cameraRaw settings")?;
    if !settings.is_object() {
        return Err("cameraRaw must be an object".into());
    }
    let params: CameraRaw = serde_json::from_value(settings.clone()).map_err(|e| format!("Camera Raw recipe: {e}"))?;
    params.validate_changes(&CameraRaw::default())?;
    Ok(params)
}

/// The settings of `params` that act on the sensor data while a raw is developed (Open's develop, and
/// `app.render`'s).
pub fn tuning(params: &CameraRaw) -> photocraft_io::raw::RawTuning {
    photocraft_io::raw::RawTuning {
        temperature: params.temperature,
        tint: params.tint,
        exposure: params.exposure,
        shadows: params.shadows,
        highlights: params.highlights,
        clarity: params.clarity,
        texture: params.texture,
        dehaze: params.dehaze,
    }
}

/// What is left of `params` to apply as one Camera Raw step once the raw was developed with [`tuning`]: everything but the
/// settings the develop applied (white balance among them only when the develop could apply it, `white_balanced`).
pub fn after_develop(params: &CameraRaw, white_balanced: bool) -> CameraRaw {
    let mut rest = params.clone();
    rest.exposure = 0.0;
    rest.shadows = 0.0;
    rest.highlights = 0.0;
    rest.clarity = 0.0;
    rest.texture = 0.0;
    rest.dehaze = 0.0;
    if white_balanced {
        rest.temperature = 0.0;
        rest.tint = 0.0;
    }
    rest
}

/// How the photo under a recipe was developed: what the dialog's picture of the recipe depends on besides the settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Developed {
    /// The raw's tone curve: the settings that act on the sensor data are applied along it, as a raw open applies them.
    pub curve: Option<Vec<[f32; 2]>>,
    /// The width of the photo's full development, when the document is a reduced stand-in for it (sizes in the settings,
    /// such as Clarity's radius, are relative to the photo).
    pub full_width: Option<u32>,
}

/// The dialog's picture of `params`, from the photo's pixels as developed with the defaults (`px`, `w` x `h`, at `pixel_scale` of
/// the photo's full size): the settings that act on the sensor data (Shadows, Highlights, Clarity, Texture, Dehaze) along the raw's
/// tone curve, as a raw open applies them, then the rest as the Camera Raw filter. One definition for the dialog's preview and the
/// recipe's render, so a photo browsed past looks as it did in the dialog.
pub fn apply(px: &mut [[f32; 4]], w: usize, h: usize, params: &CameraRaw, raw_curve: Option<&[[f32; 2]]>, pixel_scale: f32, float: bool) {
    let mut p = params.clone();
    p.pixel_scale = pixel_scale;
    if let Some(curve) = raw_curve {
        photocraft_io::raw::retone(px, w, h, curve, crate::camera_raw_ui::raw_stage(&p));
        (p.shadows, p.highlights, p.clarity, p.texture, p.dehaze) = (0.0, 0.0, 0.0, 0.0, 0.0);
    }
    photocraft_algo::camera_raw::develop(px, w, h, &p, float);
}

/// `doc` with `params` applied as [`apply`] does, at the size of the render.
fn render_pixels(doc: &Document, params: &CameraRaw, developed: &Developed) -> photocraft_compose::Buffer {
    let buffer = photocraft_compose::thumbnail_buffer(doc, RENDER_SIDE);
    let (w, h) = (buffer.rect.width() as usize, buffer.rect.height() as usize);
    let mut px = buffer.px;
    let full_width = developed.full_width.unwrap_or(doc.size.width).max(1);
    let float = doc.depth == photocraft_color::SampleType::F32;
    apply(&mut px, w, h, params, developed.curve.as_deref(), (w as f32 / full_width as f32).min(1.0), float);
    photocraft_compose::Buffer { rect: buffer.rect, px }
}

#[cfg(test)]
pub(crate) fn render_pixels_for_test(doc: &Document, params: &CameraRaw, developed: &Developed) -> photocraft_compose::Buffer {
    render_pixels(doc, params, developed)
}

/// `doc` with `params` applied, as a screen-sized sRGB JPEG: the picture the dialog shows ([`apply`]), from a reduced copy of
/// the composite, so it costs a fraction of a full-size pass.
pub fn render_display(doc: &Document, params: &CameraRaw, developed: &Developed) -> Result<Vec<u8>, String> {
    let adjusted = render_pixels(doc, params, developed);
    let icc = if matches!(doc.mode, photocraft_color::ColorMode::Cmyk | photocraft_color::ColorMode::Lab) {
        None
    } else {
        doc.icc_profile.as_ref().map(|i| i.to_vec())
    };
    photocraft_io::display_jpeg_from_buffer(&adjusted, icc, RENDER_QUALITY).map(|r| r.bytes).map_err(|e| e.to_string())
}

/// Writes `params` as the photo's recipe, if the host asked for one. A failed write is returned, never
/// only logged: the settings exist nowhere else once the dialog lets them go.
pub fn write_recipe(app: &PhotocraftApp, targets: &Targets, params: &CameraRaw) -> Result<(), String> {
    let Some(path) = &targets.recipe else { return Ok(()) };
    let write = app.services.background_automation_write.clone().ok_or("background automation write authority is not configured")?;
    let bytes = recipe_bytes(params)?;
    write(path, &bytes).map_err(|error| format!("could not keep the Camera Raw settings in {path}: {error}"))
}

/// Writes `params` as the photo's recipe ([`write_recipe`]) and, once it is kept, starts rendering it for
/// `document`. Only the recipe can fail the call; the render's failures are logged (it is a cache).
pub fn persist(app: &mut PhotocraftApp, document: DocId, targets: &Targets, params: &CameraRaw, developed: &Developed) -> Result<(), String> {
    write_recipe(app, targets, params)?;
    render_in_background(app, document, targets, params, developed);
    Ok(())
}

fn render_in_background(app: &PhotocraftApp, document: DocId, targets: &Targets, params: &CameraRaw, developed: &Developed) {
    let Some(write) = app.services.background_automation_write.clone() else { return };
    let Some(display) = targets.display.clone() else { return };
    let Some(doc) = app.session.documents().iter().find(|st| st.doc.id == document).map(|st| st.doc.clone()) else { return };
    // With a manifest: numbered now, so a save of the photo asked for after this render wins over it whenever either lands, and
    // made against the recipe as written (a render without a recipe written has nothing to be checked against).
    let published = match (&targets.manifest, &targets.recipe, recipe_bytes(params)) {
        (Some(request), Some(_), Ok(bytes)) => {
            let generation = crate::render_manifest::generation();
            Some((request.clone(), generation, crate::render_manifest::Made::Recipe { sha256: crate::render_manifest::sha256_hex(&bytes) }))
        }
        _ => None,
    };
    let params = params.clone();
    let developed = developed.clone();
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::{Mutex, OnceLock, PoisonError};
        use std::time::Duration;

        static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);
        static LATEST: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());
        static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
        static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();

        let ticket = NEXT_TICKET.fetch_add(1, Ordering::Relaxed);
        {
            let mut latest = LATEST.lock().unwrap_or_else(PoisonError::into_inner);
            latest.retain(|(path, _)| *path != display);
            latest.push((display.clone(), ticket));
            // Photos browsed past and never asked for again do not accumulate.
            if latest.len() > 64 {
                latest.remove(0);
            }
        }
        let spawned = std::thread::Builder::new().name("camera-raw-render".into()).spawn(move || {
            std::thread::sleep(Duration::from_millis(RENDER_DELAY_MS));
            let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
            let current = LATEST.lock().unwrap_or_else(PoisonError::into_inner).iter().any(|(path, t)| *path == display && *t == ticket);
            if !current {
                return;
            }
            // One worker thread for the whole job (the compositor and the Camera Raw pass parallelise through
            // rayon): the photo being opened next keeps the other cores.
            let pool = POOL.get_or_init(|| rayon::ThreadPoolBuilder::new().num_threads(1).thread_name(|_| "camera-raw-render-worker".into()).build().ok());
            let rendered = match pool {
                Some(pool) => pool.install(|| render_display(&doc, &params, &developed)),
                None => render_display(&doc, &params, &developed),
            };
            match rendered {
                Ok(bytes) => {
                    let target = published.as_ref().map_or_else(|| display.clone(), |(_, n, _)| crate::render_manifest::versioned(&display, *n));
                    if let Err(error) = write(&target, &bytes) {
                        log::warn!("could not write the Camera Raw render {target}: {error}");
                        return;
                    }
                    if let Some((request, n, made)) = &published {
                        let manifest = crate::render_manifest::content(*n, made, request.source.as_deref(), &[("display", target.as_str(), bytes.as_slice())]);
                        if let Err(error) = crate::render_manifest::publish(&write, request, *n, &manifest) {
                            log::warn!("could not publish the Camera Raw render: {error}");
                        }
                    }
                }
                Err(error) => log::warn!("could not render the Camera Raw recipe: {error}"),
            }
        });
        if let Err(error) = spawned {
            log::warn!("could not start the Camera Raw render: {error}");
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (doc, params, developed, display, write, published);
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Layer, LayerContent};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;
    use std::sync::{Arc, Mutex};

    fn grey_document(value: f32) -> Document {
        let mut doc = Document::new("p", Size::new(64, 48), ColorMode::Rgb, SampleType::U8);
        let mut surface = Surface::new(doc.pixel_format());
        surface.fill_rect(Rect::new(0, 0, 64, 48), &[value, value, value, 1.0]);
        doc.layers.push(Layer::new("bg", LayerContent::Raster(surface)));
        doc
    }

    fn centre_red(jpeg: &[u8]) -> u8 {
        let image = photocraft_codecs::decode(jpeg).expect("decodes");
        let (w, h) = image.dimensions();
        let channels = image.data().len() / (w as usize * h as usize);
        image.data()[((h / 2) as usize * w as usize + (w / 2) as usize) * channels]
    }

    #[test]
    fn a_recipe_round_trips_and_keeps_only_what_differs() {
        let params = CameraRaw { exposure: 1.5, contrast: -20.0, ..Default::default() };
        let json = to_json(&params);
        assert_eq!(json["version"], 1);
        assert_eq!(json["cameraRaw"].as_object().unwrap().len(), 2, "defaults are not stored: {json}");
        assert_eq!(json["identity"], false);
        assert_eq!(from_json(&json).unwrap(), params);
        assert_eq!(from_json(&to_json(&CameraRaw::default())).unwrap(), CameraRaw::default());
        assert_eq!(to_json(&CameraRaw::default())["identity"], true);
        let inert = CameraRaw { sharpen_radius: 2.0, ..Default::default() };
        assert_eq!(to_json(&inert)["identity"], true, "a stored setting that changes nothing on its own: {}", to_json(&inert));
    }

    #[test]
    fn a_bad_recipe_is_an_error_not_defaults() {
        assert!(from_json(&json!({})).unwrap_err().contains("version"));
        assert!(from_json(&json!({"version": 99, "cameraRaw": {}})).unwrap_err().contains("99"));
        assert!(from_json(&json!({"version": 1})).unwrap_err().contains("cameraRaw"));
        assert!(from_json(&json!({"version": 1, "cameraRaw": 3})).is_err());
        assert!(from_json(&json!({"version": 1, "cameraRaw": {"exposure": "bright"}})).is_err());
    }

    #[test]
    fn the_render_applies_the_recipe_to_a_screen_sized_copy() {
        let doc = grey_document(0.25);
        let plain = render_display(&doc, &CameraRaw::default(), &Developed::default()).unwrap();
        let brighter = CameraRaw { exposure: 2.0, ..Default::default() };
        let lifted = render_display(&doc, &brighter, &Developed::default()).unwrap();
        assert!(plain.starts_with(&[0xFF, 0xD8]) && lifted.starts_with(&[0xFF, 0xD8]));
        assert!(centre_red(&lifted) > centre_red(&plain) + 40, "{} vs {}", centre_red(&lifted), centre_red(&plain));
    }

    #[test]
    fn persist_writes_the_recipe_and_renders_it() {
        let writes = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
        let recorded = writes.clone();
        let services = crate::Services {
            background_automation_write: Some(Arc::new(move |path, bytes| {
                recorded.lock().unwrap().push((path.to_string(), bytes.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let doc = grey_document(0.25);
        let id = doc.id;
        app.session.add_document(doc, None);
        let params = CameraRaw { exposure: 1.0, ..Default::default() };
        let targets = Targets { recipe: Some("a.cameraraw.json".into()), display: Some("a.display.jpg".into()), manifest: None };
        persist(&mut app, id, &targets, &params, &Developed::default()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while writes.lock().unwrap().len() < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let writes = writes.lock().unwrap();
        let recipe = writes.iter().find(|(path, _)| path == "a.cameraraw.json").expect("recipe written");
        assert_eq!(from_json(&serde_json::from_slice(&recipe.1).unwrap()).unwrap(), params);
        assert!(writes.iter().any(|(path, bytes)| path == "a.display.jpg" && bytes.starts_with(&[0xFF, 0xD8])), "render written");
    }

    #[test]
    fn a_recipe_render_is_published_in_the_manifest_with_the_recipe_s_fingerprint() {
        let writes = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
        let recorded = writes.clone();
        let services = crate::Services {
            background_automation_write: Some(Arc::new(move |path, bytes| {
                recorded.lock().unwrap().push((path.to_string(), bytes.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let doc = grey_document(0.25);
        let id = doc.id;
        app.session.add_document(doc, None);
        let manifest = format!("m{}.photocraft.renders.json", crate::render_manifest::generation());
        let targets = Targets {
            recipe: Some("m.cameraraw.json".into()),
            display: Some("m.photocraft.display.jpg".into()),
            manifest: Some(crate::render_manifest::Request { manifest: manifest.clone(), source: Some("1:2".into()) }),
        };
        persist(&mut app, id, &targets, &CameraRaw { exposure: 1.0, ..Default::default() }, &Developed::default()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !writes.lock().unwrap().iter().any(|(p, _)| *p == manifest) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let writes = writes.lock().unwrap();
        let published: Value = serde_json::from_slice(&writes.iter().find(|(p, _)| *p == manifest).expect("published").1).unwrap();
        let recipe = &writes.iter().find(|(p, _)| p == "m.cameraraw.json").unwrap().1;
        assert_eq!(published["kind"], "recipe");
        assert_eq!(published["recipe"]["sha256"], crate::render_manifest::sha256_hex(recipe).as_str());
        assert_eq!(published["source"], "1:2");
        let n = published["generation"].as_u64().unwrap();
        let file = format!("m.photocraft.display.g{n}.jpg");
        assert_eq!(published["outputs"]["display"]["file"], file.as_str());
        assert!(writes.iter().any(|(p, b)| *p == file && b.starts_with(&[0xFF, 0xD8])));
        assert!(!writes.iter().any(|(p, _)| p == "m.photocraft.display.jpg"), "never the fixed name");
    }

    #[test]
    fn a_newer_render_of_the_same_photo_replaces_a_queued_one() {
        let writes = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
        let recorded = writes.clone();
        let services = crate::Services {
            background_automation_write: Some(Arc::new(move |path, bytes| {
                recorded.lock().unwrap().push((path.to_string(), bytes.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let doc = grey_document(0.25);
        let id = doc.id;
        app.session.add_document(doc, None);
        let targets = Targets { recipe: None, display: Some("coalesce.display.jpg".into()), manifest: None };
        // Both are requested before either can run (the first holds the turn while it renders).
        for exposure in [0.5f32, 2.0] {
            persist(&mut app, id, &targets, &CameraRaw { exposure, ..Default::default() }, &Developed::default()).unwrap();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !writes.lock().unwrap().iter().any(|(p, _)| p == "coalesce.display.jpg") && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        let writes = writes.lock().unwrap();
        let rendered: Vec<_> = writes.iter().filter(|(p, _)| p == "coalesce.display.jpg").collect();
        assert!(!rendered.is_empty() && rendered.len() <= 2, "{} renders", rendered.len());
        let last = rendered.last().unwrap();
        // The final write is always the newest recipe's (+2 EV is far brighter than +0.5 EV on the same grey).
        let older = render_display(&grey_document(0.25), &CameraRaw { exposure: 0.5, ..Default::default() }, &Developed::default()).unwrap();
        assert!(centre_red(&last.1) > centre_red(&older) + 20, "the newest recipe is the one that ends up on disk");
    }

    #[test]
    fn a_recipe_that_cannot_be_written_is_the_caller_s_error_and_nothing_is_rendered() {
        let attempts = Arc::new(Mutex::new(Vec::<String>::new()));
        let recorded = attempts.clone();
        let services = crate::Services {
            background_automation_write: Some(Arc::new(move |path, _| {
                recorded.lock().unwrap().push(path.to_string());
                Err("disk full".into())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let doc = grey_document(0.5);
        let id = doc.id;
        app.session.add_document(doc, None);
        let targets = Targets { recipe: Some("a.json".into()), display: Some("a.display.jpg".into()), manifest: None };
        let error = persist(&mut app, id, &targets, &CameraRaw { exposure: 1.0, ..Default::default() }, &Developed::default()).unwrap_err();
        assert!(error.contains("disk full") && error.contains("a.json"), "{error}");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(*attempts.lock().unwrap(), ["a.json"], "no render of settings that were not kept");
    }

    #[test]
    fn a_render_that_cannot_be_written_is_not_an_error_for_the_caller() {
        let services = crate::Services { background_automation_write: Some(Arc::new(|_, _| Err("disk full".into()))), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let doc = grey_document(0.5);
        let id = doc.id;
        app.session.add_document(doc, None);
        persist(
            &mut app,
            id,
            &Targets { recipe: None, display: Some("blocked-render.display.jpg".into()), manifest: None },
            &CameraRaw::default(),
            &Developed::default(),
        )
        .unwrap();
    }

    #[test]
    fn a_recipe_without_write_authority_is_an_error() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(write_recipe(&app, &Targets { recipe: Some("a.json".into()), display: None, manifest: None }, &CameraRaw::default()).is_err());
        write_recipe(&app, &Targets::default(), &CameraRaw::default()).unwrap();
    }
}
