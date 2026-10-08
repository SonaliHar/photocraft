//! Sensor development commands; deliberately independent of pixel Camera Raw filters.
use crate::{
    EngineError, Result, Session,
    commands::{CommandSpec, layer_param},
};
use photocraft_doc::{Document, LayerContent, LayerId, RawDevelopment, SmartObject};
use serde_json::{Value, json};
use std::sync::Mutex;

type DevelopedEntry = ([u8; 32], crate::smart_cmds::SourceImage, usize);
static DEVELOPED: Mutex<Vec<DevelopedEntry>> = Mutex::new(Vec::new());
const CACHE_BYTES: usize = 256 * 1024 * 1024;

pub fn source_image(doc: &Document, name: &str, bytes: &[u8], settings: &RawDevelopment) -> Result<crate::smart_cmds::SourceImage> {
    settings.validate().map_err(bad)?;
    let profile = crate::color_cmds::composite_profile(doc);
    let mut hash = blake3::Hasher::new();
    hash.update(bytes);
    hash.update(&serde_json::to_vec(settings).map_err(|e| bad(e.to_string()))?);
    hash.update(&profile.to_bytes());
    hash.update(format!("{:?}", doc.pixel_format()).as_bytes());
    let key = *hash.finalize().as_bytes();
    if let Some(image) = DEVELOPED.lock().ok().and_then(|mut cache| {
        let i = cache.iter().position(|(k, _, _)| *k == key)?;
        let entry = cache.remove(i);
        let image = entry.1.clone();
        cache.push(entry);
        Some(image)
    }) {
        return Ok(image);
    }
    let imported = photocraft_io::raw_smart::develop(name, bytes, settings).map_err(|e| EngineError::Other(e.to_string()))?;
    let source = &imported.document;
    let surface = source.layers.first().and_then(|l| l.surface()).ok_or_else(|| EngineError::Other("RAW development produced no pixels".into()))?;
    let transform = photocraft_cms::cached(&crate::color_cmds::composite_profile(source), &crate::color_cmds::composite_profile(doc), Default::default())
        .map_err(|e| EngineError::Other(e.to_string()))?;
    let surface = crate::color_cmds::convert_surface(surface, source.mode, doc.pixel_format(), &transform).convert(doc.pixel_format());
    let size = surface.tiles().map(|(_, tile)| tile.bytes().len()).sum::<usize>();
    let image = crate::smart_cmds::SourceImage { surface: std::sync::Arc::new(surface), bounds: source.bounds() };
    if size <= CACHE_BYTES
        && let Ok(mut cache) = DEVELOPED.lock()
    {
        cache.retain(|(k, _, _)| *k != key);
        cache.push((key, image.clone(), size));
        while cache.len() > 2 || cache.iter().map(|(_, _, size)| size).sum::<usize>() > CACHE_BYTES {
            cache.remove(0);
        }
    }
    Ok(image)
}

pub fn smart(doc: &Document, layer: LayerId) -> Result<&SmartObject> {
    match doc.layer(layer).map(|l| &l.content) {
        Some(LayerContent::Smart(sm))
            if sm.raw_development.is_some()
                && matches!(sm.source, photocraft_doc::SmartSource::Embedded { .. })
                && sm.raw_decode_source.as_ref().is_none_or(|source| matches!(source, photocraft_doc::SmartSource::Embedded { .. })) =>
        {
            Ok(sm)
        }
        _ => Err(EngineError::Other("the layer is not a RAW-backed smart object".into())),
    }
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    let state = s.active().ok_or("no document")?;
    smart(&state.doc, state.active_layer.ok_or("no active layer")?).map(|_| ()).map_err(|e| e.to_string())
}

fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let sm = smart(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
    Ok(json!({"layer": id.0, "settings": sm.raw_development, "sensorBacked": true}))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let settings: RawDevelopment =
        serde_json::from_value(p.get("settings").cloned().ok_or_else(|| bad("missing settings"))?).map_err(|e| bad(e.to_string()))?;
    settings.validate().map_err(bad)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let sm = smart(doc, id)?;
    if sm.raw_development.as_ref() == Some(&settings) {
        return Ok(json!({"layer": id.0, "unchanged": true}));
    }
    crate::jobs::edit_job(
        s,
        "RAW Development",
        move |doc, _, ctx| {
            ctx.check()?;
            let layer = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            let LayerContent::Smart(sm) = &mut layer.content else { return Err(bad("not a smart object")) };
            sm.raw_development = Some(settings);
            sm.psd_raw = None;
            layer.psd_blocks.retain(|(key, _)| !matches!(key, b"SoLd" | b"PlLd" | b"SoLE"));
            if !crate::smart_cmds::refresh(doc, id)? {
                return Err(bad("RAW sensor source is unavailable; settings were not applied"));
            }
            ctx.check()?;
            Ok(id)
        },
        |id| json!({"layer": id.0}),
    )
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "layer.rawDevelopment.set".into(), msg: msg.into() }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "layer.rawDevelopment.inspect",
            label: "Inspect RAW Development",
            menu: &[],
            shortcut: None,
            params: "{layer:id?}",
            enabled,
            run: inspect,
            journal: false,
        },
        CommandSpec {
            id: "layer.rawDevelopment.set",
            label: "RAW Development",
            menu: &[],
            shortcut: None,
            params: "{layer:id?,settings:{version:1,exposure:number,whiteBalance:{mode:asShot|auto|multipliers,rgb:[r,g,b]?},demosaic:ahd|mhc|bilinear}}",
            enabled,
            run: set,
            journal: true,
        },
    ]
}
