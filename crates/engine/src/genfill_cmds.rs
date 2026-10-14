//! Generative Fill: replace the selected area (an object, the background, a blemish) with what a
//! prompt describes, through the image-generation provider the user chose (`photocraft-ml`).
//!
//! * `edit.generativeFill` sends a crop of the composite around the selection plus a mask to the
//!   provider, and puts the answer on a new pixel layer whose alpha is the selection, so the
//!   original is untouched and the result can be hidden, masked, moved or undone. An empty prompt
//!   removes the selected content.
//! * `ai.settings` reads and changes the provider and its settings. The provider choice, server
//!   URL and model are ordinary preferences (`dialogs["ai.generativeFill"]`). **Credentials are
//!   not preferences**: API keys and account ids live in a separate file in the user's config
//!   folder ([`enable_credentials_file`]; in memory only until an app enables it), so no
//!   preferences view, export or serialisation can carry them, and no command returns them.
//!   Environment variables `PHOTOCRAFT_<PROVIDER>_API_KEY` / `_ACCOUNT_ID` are used when nothing
//!   is saved.
//!
//! Only the crop around the selection is uploaded, and only when the command runs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::SystemTime;

use photocraft_algo::segment::RgbImage;
use photocraft_algo::selection::{expand, mask_from_surface};
use photocraft_doc::{Document, Layer};
use photocraft_geom::Rect;
use photocraft_ml::genfill::{self, FillRequest, Provider, ProviderConfig};
use photocraft_ml::{MlError, models};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::jobs::JobCtx;
use crate::smartselect_cmds::with_doc_sampler;
use crate::{EngineError, Result, Session};

const CMD: &str = "edit.generativeFill";
const SETTINGS_CMD: &str = "ai.settings";
const SETTINGS_KEY: &str = "ai.generativeFill";

/// Most pixels the window around the selection may have (it is read at full resolution).
const MAX_WINDOW_PX: u64 = 120_000_000;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn convert(e: MlError) -> EngineError {
    match e {
        MlError::Cancelled => EngineError::Cancelled,
        MlError::Msg(m) => EngineError::Other(m),
    }
}

// ------------------------------------------------------------------ settings

/// The saved provider choice and each provider's configuration.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub provider: Option<Provider>,
    pub providers: BTreeMap<String, ProviderConfig>,
}

impl Settings {
    /// The provider to use: the chosen one, else the free Cloudflare tier.
    pub fn active(&self) -> Provider {
        self.provider.unwrap_or(Provider::Cloudflare)
    }

    pub fn config(&self, p: Provider) -> ProviderConfig {
        self.providers.get(p.id()).cloned().unwrap_or_default()
    }
}

// ------------------------------------------------------------------ credentials

/// One provider's secrets: kept out of the preferences.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Secret {
    api_key: String,
    account_id: String,
}

type SecretMap = BTreeMap<String, Secret>;

static CREDENTIALS_FILE: OnceLock<PathBuf> = OnceLock::new();

/// Where credentials are stored: `PHOTOCRAFT_CONFIG_DIR`, else the platform's user config folder.
pub fn default_credentials_path() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let dir = env("PHOTOCRAFT_CONFIG_DIR").or_else(|| {
        #[cfg(target_os = "windows")]
        let base = env("APPDATA");
        #[cfg(target_os = "macos")]
        let base = env("HOME").map(|h| h.join("Library").join("Application Support"));
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let base = env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")));
        base.map(|b| b.join("photocraft"))
    })?;
    Some(dir.join("ai-credentials.json"))
}

/// Persist credentials to [`default_credentials_path`]. Interactive apps call this once at
/// start-up; without it (tests, the CLI, scripts) saved credentials live in memory for the
/// session only, and the environment variables still work.
pub fn enable_credentials_file() {
    if let Some(p) = default_credentials_path() {
        let _ = CREDENTIALS_FILE.set(p);
    }
}

fn read_secrets(path: &Path) -> SecretMap {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write_secrets(path: &Path, map: &SecretMap) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(map).map_err(|e| EngineError::Other(format!("cannot encode the credentials: {e}")))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| EngineError::Other(format!("cannot create {}: {e}", dir.display())))?;
    }
    photocraft_format::atomic_write(path, &bytes).map_err(|e| EngineError::Other(format!("cannot save the credentials to {}: {e}", path.display())))?;
    // Owner-only where the platform has permission bits (Windows keeps them in the user profile).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Serialises every credentials read-modify-write in this process, so two sessions saving
/// different providers cannot overwrite each other's entry.
static IO_LOCK: Mutex<()> = Mutex::new(());

/// What was last read from the credentials file: its stamp (modified time, length) and content.
type FileCache = Option<((SystemTime, u64), SecretMap)>;

/// The session's credentials. With a credentials file the file is the single source of truth:
/// every read looks at it (re-reading only when it changed) and every save re-reads, merges its
/// own provider and writes under [`IO_LOCK`], so nothing is cached that was not persisted and
/// independent sessions merge instead of clobbering. Without a file the map lives in memory.
#[derive(Default)]
pub struct Secrets {
    /// Overrides the app-wide file (tests).
    file: Option<PathBuf>,
    memory: Mutex<SecretMap>,
    cache: Mutex<FileCache>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secrets(..)")
    }
}

impl Secrets {
    fn path(&self) -> Option<&Path> {
        self.file.as_deref().or_else(|| CREDENTIALS_FILE.get().map(PathBuf::as_path))
    }

    /// The credentials as they are now.
    fn snapshot(&self) -> SecretMap {
        let Some(path) = self.path() else {
            return self.memory.lock().unwrap_or_else(PoisonError::into_inner).clone();
        };
        let stamp = std::fs::metadata(path).ok().and_then(|m| Some((m.modified().ok()?, m.len())));
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let (Some(stamp), Some((seen, map))) = (stamp, cache.as_ref())
            && *seen == stamp
        {
            return map.clone();
        }
        let map = read_secrets(path);
        *cache = stamp.map(|st| (st, map.clone()));
        map
    }

    /// Set (or, with a default [`Secret`], remove) one provider's credentials. The in-memory
    /// state changes only if the file write succeeded, so a retry really retries.
    fn store(&self, id: &str, secret: Secret) -> Result<()> {
        let _io = IO_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let apply = |map: &mut SecretMap| {
            if secret == Secret::default() {
                map.remove(id);
            } else {
                map.insert(id.to_string(), secret.clone());
            }
        };
        match self.path() {
            None => {
                apply(&mut self.memory.lock().unwrap_or_else(PoisonError::into_inner));
                Ok(())
            }
            Some(path) => {
                let mut map = read_secrets(path);
                apply(&mut map);
                write_secrets(path, &map)?;
                *self.cache.lock().unwrap_or_else(PoisonError::into_inner) = None;
                Ok(())
            }
        }
    }
}

// ------------------------------------------------------------------ settings

/// The saved settings with the credentials filled in from the credentials store (defaults when
/// none, or when the saved value is unreadable). Never serialise the result into the
/// preferences: use [`save_settings`].
pub fn settings(s: &Session) -> Settings {
    let mut st: Settings = s.prefs().dialogs.get(SETTINGS_KEY).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    let secrets = s.ai_secrets.snapshot();
    for p in Provider::ALL {
        if let Some(secret) = secrets.get(p.id()) {
            let cfg = st.providers.entry(p.id().to_string()).or_default();
            cfg.api_key.clone_from(&secret.api_key);
            cfg.account_id.clone_from(&secret.account_id);
        }
    }
    st
}

/// Save `st`, which differs from the stored settings in the provider choice and/or `changed`'s
/// configuration. The credentials of `changed` go to the credentials store first (nothing is
/// recorded if that write fails); the preferences get everything except credentials.
fn save_settings(s: &mut Session, st: &Settings, changed: Option<Provider>) -> Result<()> {
    if let Some(p) = changed {
        let cfg = st.config(p);
        s.ai_secrets.store(p.id(), Secret { api_key: cfg.api_key, account_id: cfg.account_id })?;
    }
    let mut public = st.clone();
    for cfg in public.providers.values_mut() {
        cfg.api_key.clear();
        cfg.account_id.clear();
    }
    let value = serde_json::to_value(&public).unwrap_or(Value::Null);
    s.prefs.edit(|p| p.dialogs.insert(SETTINGS_KEY.into(), value));
    Ok(())
}

/// Where a credential comes from, for display (never the credential itself).
fn source(saved: &str, env_set: bool) -> &'static str {
    if !saved.trim().is_empty() {
        "saved"
    } else if env_set {
        "environment"
    } else {
        "none"
    }
}

/// Everything the settings UI needs, without any secret.
pub fn settings_state(s: &Session) -> Value {
    let st = settings(s);
    let providers: Vec<Value> = Provider::ALL
        .into_iter()
        .map(|p| {
            let cfg = st.config(p);
            json!({
                "id": p.id(),
                "label": p.label(),
                "needsKey": p.needs_key(),
                "needsAccount": p.needs_account(),
                "configured": cfg.missing(p).is_none(),
                "missing": cfg.missing(p),
                "keySource": source(&cfg.api_key, !cfg.key(p).is_empty()),
                "accountSource": source(&cfg.account_id, !cfg.account(p).is_empty()),
                "baseUrl": cfg.base_url,
                "model": cfg.model,
                "maxSide": cfg.max_side,
                "defaults": { "baseUrl": p.default_base_url(), "model": p.default_model(), "maxSide": p.default_side() },
            })
        })
        .collect();
    let subject = &models::SUBJECT;
    json!({
        "provider": st.active().id(),
        "providers": providers,
        "subjectModel": {
            "id": subject.id,
            "licence": subject.licence,
            "bytes": subject.bytes,
            "present": models::is_present(subject),
            "path": models::path_of(subject).map(|p| p.display().to_string()),
        },
    })
}

fn valid_url(u: &str) -> bool {
    u.is_empty() || ((u.starts_with("https://") || u.starts_with("http://")) && !u.contains(char::is_whitespace))
}

fn set_settings(s: &mut Session, p: &Value) -> Result<Value> {
    let mut st = settings(s);
    let mut changed = false;
    let mut configured: Option<Provider> = None;
    if let Some(id) = p.get("provider").and_then(Value::as_str) {
        let prov = Provider::parse(id).ok_or_else(|| bad(SETTINGS_CMD, format!("unknown provider `{id}`")))?;
        changed |= st.provider != Some(prov);
        st.provider = Some(prov);
    }
    if let Some(c) = p.get("configure").filter(|v| !v.is_null()) {
        let id = c
            .get("provider")
            .and_then(Value::as_str)
            .or_else(|| p.get("provider").and_then(Value::as_str))
            .ok_or_else(|| bad(SETTINGS_CMD, "configure needs a provider"))?;
        let prov = Provider::parse(id).ok_or_else(|| bad(SETTINGS_CMD, format!("unknown provider `{id}`")))?;
        let mut cfg = st.config(prov);
        let text = |k: &str| -> Result<Option<String>> {
            match c.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v.as_str().map(|t| Some(t.trim().to_string())).ok_or_else(|| bad(SETTINGS_CMD, format!("`{k}` must be a string"))),
            }
        };
        if let Some(v) = text("apiKey")? {
            cfg.api_key = v;
        }
        if let Some(v) = text("accountId")? {
            cfg.account_id = v;
        }
        if let Some(v) = text("baseUrl")? {
            if !valid_url(&v) {
                return Err(bad(SETTINGS_CMD, "baseUrl must start with http:// or https://"));
            }
            cfg.base_url = v;
        }
        if let Some(v) = text("model")? {
            cfg.model = v;
        }
        if let Some(v) = c.get("maxSide").filter(|v| !v.is_null()) {
            let n = v.as_u64().filter(|n| *n == 0 || (64..=2048).contains(n)).ok_or_else(|| bad(SETTINGS_CMD, "maxSide must be 0 (default) or 64..2048"))?;
            cfg.max_side = n as u32;
        }
        if st.config(prov) != cfg {
            changed = true;
            configured = Some(prov);
        }
        st.providers.insert(prov.id().to_string(), cfg);
    }
    if changed {
        save_settings(s, &st, configured)?;
    }
    Ok(settings_state(s))
}

// ------------------------------------------------------------------ the fill

fn has_selection(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.selection.is_none() {
        return Err("make a selection first: the area to generate".into());
    }
    Ok(())
}

/// Bilinear resample of a single plane.
fn resize_plane(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(dw * dh);
    let (fx, fy) = (sw as f32 / dw as f32, sh as f32 / dh as f32);
    for y in 0..dh {
        let sy = ((y as f32 + 0.5) * fy - 0.5).clamp(0.0, (sh - 1) as f32);
        let (y0, ty) = (sy.floor() as usize, sy.fract());
        let y1 = (y0 + 1).min(sh - 1);
        for x in 0..dw {
            let sx = ((x as f32 + 0.5) * fx - 0.5).clamp(0.0, (sw - 1) as f32);
            let (x0, tx) = (sx.floor() as usize, sx.fract());
            let x1 = (x0 + 1).min(sw - 1);
            let at = |xx: usize, yy: usize| src.get(yy * sw + xx).copied().unwrap_or(0.0);
            let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
            let bot = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
            out.push(top + (bot - top) * ty);
        }
    }
    out
}

/// Bilinear resample of an RGB image into `dw x dh` planar-interleaved `[r, g, b]`.
fn resize_rgb(src: &[[f32; 3]], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<[f32; 3]> {
    let planes: Vec<Vec<f32>> = (0..3).map(|c| resize_plane(&src.iter().map(|p| p[c]).collect::<Vec<_>>(), sw, sh, dw, dh)).collect();
    (0..dw * dh).map(|i| [0, 1, 2].map(|c| planes[c].get(i).copied().unwrap_or(0.0))).collect()
}

/// What the worker hands back: the new layer's pixels over `window` (straight RGBA, alpha = the
/// selection's coverage).
struct Generated {
    window: Rect,
    rgba: Vec<[f32; 4]>,
    provider: Provider,
}

/// Tight bounds of coverage above `min` in `m` (`w` wide) placed at `origin`.
fn coverage_bounds(m: &[f32], w: usize, origin: Rect, min: f32) -> Option<Rect> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for (y, row) in m.chunks_exact(w.max(1)).enumerate() {
        let Some(first) = row.iter().position(|v| *v > min) else { continue };
        let last = row.iter().rposition(|v| *v > min).unwrap_or(first);
        x0 = x0.min(first);
        x1 = x1.max(last + 1);
        y0 = y0.min(y);
        y1 = y + 1;
    }
    (x0 != usize::MAX).then(|| Rect::new(origin.x0 + x0 as i32, origin.y0 + y0 as i32, origin.x0 + x1 as i32, origin.y0 + y1 as i32))
}

fn generate(doc: &Document, provider: Provider, cfg: &ProviderConfig, prompt: &str, context: f32, ctx: &JobCtx) -> Result<Generated> {
    ctx.progress(0.02, "Preparing the selection");
    let area = doc.bounds();
    let sel = doc.selection.as_ref().ok_or_else(|| EngineError::Other("make a selection first: the area to generate".into()))?;
    // An inverted selection has a non-empty default; otherwise only the tiles it touches matter.
    let inverted = sel.default_pixel().first().is_some_and(|v| *v > 0.0);
    let scan = if inverted { area } else { sel.content_bounds().intersect(&area) };
    if scan.is_empty() {
        return Err(EngineError::Other("the selection is empty".into()));
    }
    if scan.width() as u64 * scan.height() as u64 > MAX_WINDOW_PX {
        return Err(EngineError::Other("the selection is too large for Generative Fill".into()));
    }
    let scanned = mask_from_surface(Some(sel), scan);
    let sel_rect = coverage_bounds(&scanned, scan.width() as usize, scan, 0.02).ok_or_else(|| EngineError::Other("the selection is empty".into()))?;
    drop(scanned);
    // Context around the selection helps the model match light, colour and perspective.
    let pad = ((sel_rect.width().max(sel_rect.height()) as f32 * context).round() as i32).max(16);
    let window = sel_rect.inflate(pad).intersect(&area);
    let (ww, wh) = (window.width() as usize, window.height() as usize);
    if ww as u64 * wh as u64 > MAX_WINDOW_PX {
        return Err(EngineError::Other("the area around the selection is too large for Generative Fill".into()));
    }
    ctx.check()?;
    let (cw, ch) = genfill::canvas_for(provider, cfg, ww as u32, wh as u32);
    let (cw, ch) = (cw as usize, ch as usize);

    ctx.progress(0.06, "Reading the image");
    let step = (ww.max(wh) / cw.max(ch)).max(1);
    let img: RgbImage = with_doc_sampler(doc, None, true, |smp, _| smp.rgb_scaled(window, step));
    let rgb = resize_rgb(&img.px, img.w, img.h, cw, ch);
    let coverage = mask_from_surface(Some(sel), window);
    ctx.check()?;
    // The model repaints a little beyond the edge so no halo of the old content survives.
    let wanted = resize_plane(&coverage, ww, wh, cw, ch);
    let grown = expand(&wanted, cw, ch, (cw.max(ch) / 128).max(2) as f32);
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let rgb_bytes: Vec<u8> = rgb.iter().flat_map(|p| p.map(to_u8)).collect();
    let mask_bytes: Vec<u8> = grown.iter().map(|v| if *v >= 0.5 { 255 } else { 0 }).collect();

    ctx.progress(0.12, &format!("Waiting for {}", provider.label()));
    let request = FillRequest { prompt, width: cw as u32, height: ch as u32, rgb: &rgb_bytes, mask: &mask_bytes };
    let bytes = genfill::generate(provider, cfg, &request, &|| !ctx.cancelled()).map_err(convert)?;
    ctx.check()?;

    ctx.progress(0.9, "Placing the result");
    let image = photocraft_codecs::decode(&bytes)
        .map_err(|e| EngineError::Other(format!("could not read the image {} returned: {e}", provider.label())))?
        .convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
    let (rw, rh) = (image.width() as usize, image.height() as usize);
    if rw == 0 || rh == 0 || image.data().len() != rw * rh * 4 {
        return Err(EngineError::Other(format!("{} returned an empty image", provider.label())));
    }
    let px: Vec<[f32; 3]> =
        image.data().as_chunks::<4>().0.iter().map(|p| [f32::from(p[0]) / 255.0, f32::from(p[1]) / 255.0, f32::from(p[2]) / 255.0]).collect();
    let fitted = if (rw, rh) == (ww, wh) { px } else { resize_rgb(&px, rw, rh, ww, wh) };
    let rgba = fitted.iter().zip(&coverage).map(|(c, a)| [c[0], c[1], c[2], *a]).collect();
    Ok(Generated { window, rgba, provider })
}

fn layer_name(prompt: &str) -> String {
    let p: String = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if p.is_empty() {
        return "Generative Fill (removal)".into();
    }
    let short: String = p.chars().take(40).collect();
    if short.chars().count() < p.chars().count() { format!("Generative Fill: {short}…") } else { format!("Generative Fill: {short}") }
}

fn place(s: &mut Session, g: Generated, prompt: &str) -> Result<Value> {
    let w = g.window;
    let id = s.edit("Generative Fill", |doc, active| {
        let mut layer = Layer::raster(layer_name(prompt), doc.pixel_format());
        let surface = layer.surface_mut().ok_or_else(|| EngineError::Other("a new pixel layer has no pixels".into()))?;
        let format = surface.format();
        let ch = format.channels();
        let mut data = vec![0.0f32; g.rgba.len().saturating_mul(ch)];
        for (px, out) in g.rgba.iter().zip(data.chunks_exact_mut(ch)) {
            photocraft_raster::from_rgba_into(&format, *px, out);
        }
        surface.write_region(w, &data);
        surface.prune();
        let id = doc.insert_above(*active, layer);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(
        json!({ "layer": id.0, "provider": g.provider.id(), "bounds": [w.x0, w.y0, w.width(), w.height()], "status": format!("Generative Fill done ({})", g.provider.label()) }),
    )
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let prompt = match p.get("prompt") {
        None | Some(Value::Null) => String::new(),
        Some(v) => v.as_str().ok_or_else(|| bad(CMD, "`prompt` must be a string"))?.trim().to_string(),
    };
    if prompt.chars().count() > 1000 {
        return Err(bad(CMD, "`prompt` is longer than 1000 characters"));
    }
    let st = settings(s);
    let provider = match p.get("provider").and_then(Value::as_str) {
        None => st.active(),
        Some(id) => Provider::parse(id).ok_or_else(|| bad(CMD, format!("unknown provider `{id}`")))?,
    };
    let cfg = st.config(provider);
    if let Some(why) = cfg.missing(provider) {
        return Err(EngineError::Other(why));
    }
    let context = match p.get("context") {
        None | Some(Value::Null) => 0.5,
        Some(v) => v.as_f64().filter(|c| c.is_finite() && (0.0..=2.0).contains(c)).ok_or_else(|| bad(CMD, "`context` must be a number from 0 to 2"))? as f32,
    };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    if d.doc.selection.is_none() {
        return Err(EngineError::Other("make a selection first: the area to generate".into()));
    }
    let doc = Arc::clone(&d.doc);
    let apply_prompt = prompt.clone();
    crate::jobs::run(s, "Generative Fill", true, move |ctx| generate(&doc, provider, &cfg, &prompt, context, ctx), move |s, g| place(s, g, &apply_prompt))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: CMD,
            label: "Generative Fill…",
            menu: &["Edit"],
            shortcut: None,
            params: r##"{"prompt":string="" (what to generate; empty removes the selected content),"provider":"cloudflare|sdwebui|gemini|openai|stability"=the saved choice,"context":0..2=0.5 (how much surrounding image is sent, as a fraction of the selection's size)} → {layer,provider,bounds} (a crop around the selection is uploaded to the provider; the result is a new pixel layer masked by the selection; runs as a background job)"##,
            enabled: has_selection,
            run,
            journal: true,
        },
        CommandSpec {
            id: SETTINGS_CMD,
            label: "AI Settings",
            menu: &[],
            shortcut: None,
            params: r##"{"provider":id?,"configure":{"provider":id,"apiKey":string?,"accountId":string?,"baseUrl":string?,"model":string?,"maxSide":0|64..2048?}?} → {provider,providers:[{id,label,needsKey,needsAccount,configured,missing,keySource,accountSource,baseUrl,model,maxSide,defaults}],subjectModel:{id,licence,bytes,present,path}} (credentials are stored in a separate credentials file in the user config folder, never in the preferences, and never returned; environment variables PHOTOCRAFT_<PROVIDER>_API_KEY and _ACCOUNT_ID are used when none is saved)"##,
            enabled: always,
            run: set_settings,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_raster::Surface;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// One-shot HTTP server answering a PNG; sends back the request head it saw.
    fn serve_png(body: Vec<u8>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let Ok((mut s, _)) = listener.accept() else { return };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 16384];
            let (head_end, len) = loop {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..i]).to_ascii_lowercase();
                    let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                    break (i + 4, len);
                }
            };
            while buf.len() < head_end + len {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let _ = tx.send(String::from_utf8_lossy(&buf[..head_end]).into_owned());
            let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(reply.as_bytes());
            let _ = s.write_all(&body);
        });
        (format!("http://{addr}"), rx)
    }

    fn red_png(w: u32, h: u32) -> Vec<u8> {
        let data: Vec<u8> = (0..w * h).flat_map(|_| [220u8, 20, 20]).collect();
        let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgb, data).unwrap();
        photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()
    }

    fn session_with_selection() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        s.edit("select", |doc, _| {
            let mut sel = Surface::new(PixelFormat::GRAY8);
            sel.fill_rect(Rect::new(20, 20, 40, 40), &[1.0]);
            doc.selection = Some(sel);
            Ok(())
        })
        .unwrap();
        s
    }

    #[test]
    fn settings_round_trip_and_never_reveal_keys() {
        let mut s = Session::new();
        let r = s.execute(SETTINGS_CMD, json!({})).unwrap();
        assert_eq!(r["provider"], "cloudflare");
        let r =
            s.execute(SETTINGS_CMD, json!({"provider": "gemini", "configure": {"provider": "gemini", "apiKey": "sekret-123", "model": "gemini-x"}})).unwrap();
        assert_eq!(r["provider"], "gemini");
        let gemini = r["providers"].as_array().unwrap().iter().find(|p| p["id"] == "gemini").unwrap();
        assert_eq!(gemini["configured"], true);
        assert_eq!(gemini["keySource"], "saved");
        assert_eq!(gemini["model"], "gemini-x");
        assert!(!r.to_string().contains("sekret-123"), "the key must not come back");
        assert_eq!(settings(&s).config(Provider::Gemini).api_key, "sekret-123");
        let e = s.execute(SETTINGS_CMD, json!({"configure": {"provider": "gemini", "baseUrl": "file:///etc"}}));
        assert!(e.is_err());
        let e = s.execute(SETTINGS_CMD, json!({"provider": "dalle"}));
        assert!(e.is_err());
    }

    /// The dummy key must not appear in any view or serialisation of the preferences, in the
    /// command results, or in the journal; it still reaches the provider config.
    #[test]
    fn credentials_never_reach_the_preferences_or_other_views() {
        const KEY: &str = "dummy-key-7c1e9f";
        const ACCOUNT: &str = "dummy-account-42b";
        let mut s = Session::new();
        let r = s
            .execute(
                SETTINGS_CMD,
                json!({"provider": "cloudflare", "configure": {"provider": "cloudflare", "apiKey": KEY, "accountId": ACCOUNT, "model": "m"}}),
            )
            .unwrap();
        let views = [
            ("ai.settings result", r.to_string()),
            ("prefs_to_json", s.prefs_to_json()),
            ("prefs_value", s.prefs_value().to_string()),
            ("prefs serde", serde_json::to_string(s.prefs()).unwrap()),
            ("prefs dialogs", serde_json::to_string(&s.prefs().dialogs).unwrap()),
            ("journal", format!("{:?}", s.journal)),
            ("session debug", format!("{:?}", s.ai_secrets)),
            ("ai.settings again", s.execute(SETTINGS_CMD, json!({})).unwrap().to_string()),
        ];
        for (what, text) in views {
            assert!(!text.contains(KEY) && !text.contains(ACCOUNT), "{what} leaks a credential");
        }
        // Non-secret settings are ordinary preferences; the credentials are still usable.
        assert!(s.prefs_to_json().contains("\"model\""));
        let cfg = settings(&s).config(Provider::Cloudflare);
        assert_eq!((cfg.api_key.as_str(), cfg.account_id.as_str()), (KEY, ACCOUNT));
        // Reloading the preferences from their own JSON keeps the credentials (they are separate).
        let json = s.prefs_to_json();
        s.load_prefs_json(&json).unwrap();
        assert_eq!(settings(&s).config(Provider::Cloudflare).api_key, KEY);
        // Clearing removes them.
        s.execute(SETTINGS_CMD, json!({"configure": {"provider": "cloudflare", "apiKey": "", "accountId": ""}})).unwrap();
        assert_eq!(settings(&s).config(Provider::Cloudflare).api_key, "", "an empty string clears it");
    }

    /// Review finding: sessions that load the file independently must merge, not clobber.
    #[test]
    fn independent_sessions_merge_their_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai-credentials.json");
        let mut a = Session::new();
        let mut b = Session::new();
        a.ai_secrets.file = Some(path.clone());
        b.ai_secrets.file = Some(path.clone());
        // Both read the (still missing) file before either writes.
        assert!(settings(&a).config(Provider::Gemini).api_key.is_empty());
        assert!(settings(&b).config(Provider::OpenAi).api_key.is_empty());
        a.execute(SETTINGS_CMD, json!({"configure": {"provider": "gemini", "apiKey": "dummy-gemini"}})).unwrap();
        b.execute(SETTINGS_CMD, json!({"configure": {"provider": "openai", "apiKey": "dummy-openai"}})).unwrap();
        let on_disk = read_secrets(&path);
        assert_eq!(on_disk.get("gemini").map(|s| s.api_key.as_str()), Some("dummy-gemini"));
        assert_eq!(on_disk.get("openai").map(|s| s.api_key.as_str()), Some("dummy-openai"));
        // Each session sees the other's change without restarting.
        assert_eq!(settings(&a).config(Provider::OpenAi).api_key, "dummy-openai");
        assert_eq!(settings(&b).config(Provider::Gemini).api_key, "dummy-gemini");
    }

    /// Review finding: a failed write must not be remembered as saved, so retrying retries.
    #[test]
    fn a_failed_credentials_write_is_retried() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai-credentials.json");
        std::fs::create_dir(&path).unwrap(); // an obstruction: the target is a directory
        let mut s = Session::new();
        s.ai_secrets.file = Some(path.clone());
        let params = json!({"configure": {"provider": "gemini", "apiKey": "dummy-key"}});
        assert!(s.execute(SETTINGS_CMD, params.clone()).is_err(), "the write must fail");
        assert!(settings(&s).config(Provider::Gemini).api_key.is_empty(), "a failed save is not remembered");
        std::fs::remove_dir(&path).unwrap();
        s.execute(SETTINGS_CMD, params).unwrap();
        assert_eq!(read_secrets(&path).get("gemini").map(|s| s.api_key.as_str()), Some("dummy-key"), "the retry wrote the file");
        assert!(!s.prefs_to_json().contains("dummy-key"));
    }

    #[test]
    fn the_credentials_file_round_trips_and_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("ai-credentials.json");
        let mut map = SecretMap::new();
        map.insert("gemini".into(), Secret { api_key: "k".into(), account_id: String::new() });
        write_secrets(&path, &map).unwrap();
        assert_eq!(read_secrets(&path), map);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0, "owner only");
        }
        assert!(read_secrets(&dir.path().join("missing.json")).is_empty());
        std::fs::write(&path, b"not json").unwrap();
        assert!(read_secrets(&path).is_empty(), "a corrupt file is ignored, not a panic");
    }

    #[test]
    fn needs_a_selection_and_credentials() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        assert!(!s.is_enabled(CMD), "no selection, no Generative Fill");
        let mut s = session_with_selection();
        assert!(s.is_enabled(CMD));
        let e = s.execute(CMD, json!({"prompt": "a dog", "provider": "openai"})).unwrap_err().to_string();
        assert!(e.contains("API key"), "{e}");
        assert_eq!(s.active().unwrap().doc.layers.len(), 1, "nothing was added");
    }

    #[test]
    fn fills_the_selection_on_a_new_layer() {
        let (base, rx) = serve_png(red_png(64, 64));
        let mut s = session_with_selection();
        s.execute(SETTINGS_CMD, json!({"configure": {"provider": "stability", "apiKey": "k", "baseUrl": base}})).unwrap();
        let r = s.execute(CMD, json!({"prompt": "a red patch", "provider": "stability"})).unwrap();
        let head = rx.recv().unwrap();
        assert!(head.starts_with("POST /v2beta/stable-image/edit/inpaint "), "{head}");
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.layers.len(), 2);
        let layer = doc.layers.last().unwrap();
        assert!(layer.name.starts_with("Generative Fill: a red patch"), "{}", layer.name);
        assert_eq!(r["layer"], layer.id.0);
        let surf = layer.surface().unwrap();
        let inside = surf.rgba(30, 30);
        assert!(inside[0] > 0.7 && inside[1] < 0.3 && inside[3] > 0.99, "inside {inside:?}");
        assert!(surf.rgba(5, 5)[3] < 0.01, "outside the selection stays transparent");
        // One undo removes it.
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    }

    #[test]
    fn provider_failures_leave_the_document_alone() {
        let mut s = session_with_selection();
        // Nothing listens on this port.
        s.execute(SETTINGS_CMD, json!({"configure": {"provider": "sdwebui", "baseUrl": "http://127.0.0.1:9"}})).unwrap();
        let e = s.execute(CMD, json!({"prompt": "x", "provider": "sdwebui"}));
        assert!(e.is_err());
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    }

    #[test]
    fn bad_parameters_are_errors() {
        let mut s = session_with_selection();
        assert!(s.execute(CMD, json!({"prompt": 5})).is_err());
        assert!(s.execute(CMD, json!({"prompt": "x".repeat(1001)})).is_err());
        assert!(s.execute(CMD, json!({"context": 9})).is_err());
        assert!(s.execute(CMD, json!({"provider": "nope"})).is_err());
    }

    #[test]
    fn layer_names_are_short_and_single_line() {
        assert_eq!(layer_name(""), "Generative Fill (removal)");
        assert_eq!(layer_name("  a\n red   balloon "), "Generative Fill: a red balloon");
        assert!(layer_name(&"word ".repeat(30)).ends_with('…'));
    }

    #[test]
    fn coverage_bounds_are_tight() {
        let mut m = vec![0.0f32; 10 * 8];
        m[2 * 10 + 3] = 1.0;
        m[5 * 10 + 7] = 0.5;
        let r = coverage_bounds(&m, 10, Rect::new(100, 200, 110, 208), 0.02).unwrap();
        assert_eq!(r, Rect::new(103, 202, 108, 206));
        assert!(coverage_bounds(&[0.0; 4], 2, Rect::new(0, 0, 2, 2), 0.02).is_none());
    }
}
