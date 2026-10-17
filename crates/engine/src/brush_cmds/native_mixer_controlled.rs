//! Closed full-load native Mixer lane. Legacy tool well receives only the
//! original f32 parameter reservoir, exact amount one and absent pickup.
use crate::{DocumentSnapshot, EngineError, Result, Session, WeakDocumentSnapshot};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_paint::mixer::{MixerSettings, MixerState};
use photocraft_paint::native_mixer_controlled::{self, NativeMixerSettings};
use photocraft_paint::{BrushSettings, StrokeError, StrokeLimits, StrokePoint, StrokeRenderer};
use photocraft_raster::{Interrupt, Surface};
use serde_json::{Value, json};
fn bad(message: impl Into<String>) -> EngineError {
    super::bad("paint.mixerBrush", message)
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| EngineError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| bad("native Mixer accounting overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(|| bad("native Mixer accounting overflow"))
}
fn paint(e: StrokeError) -> EngineError {
    if e == StrokeError::Cancelled { EngineError::Cancelled } else { bad(e.to_string()) }
}
fn io(e: photocraft_io::IoError, c: &Interrupt<'_>) -> EngineError {
    if matches!(e, photocraft_io::IoError::Cancelled) || c.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) }
}
fn boolean(p: &Value, key: &str, default: bool) -> Result<bool> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| bad(format!("{key} must be boolean"))),
    }
}
fn number(v: &Value, key: &str) -> Result<f32> {
    let n = v.as_f64().filter(|n| n.is_finite() && (*n as f32).is_finite()).ok_or_else(|| bad(format!("{key} must be finite and representable")))?;
    Ok(n as f32)
}
fn load_color(s: &Session, p: &Value) -> Result<[f32; 4]> {
    match p.get("color") {
        None => {}
        Some(Value::Array(values)) if matches!(values.len(), 3 | 4) => {
            for value in values {
                let v = number(value, "color")?;
                if !(0.0..=1.0).contains(&v) {
                    return Err(bad("Mixer color must be unit RGBA"));
                }
            }
        }
        Some(Value::String(value)) => {
            let hex = value.strip_prefix('#').unwrap_or(value);
            if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|v| v.is_ascii_hexdigit()) {
                return Err(bad("Mixer color must be #rrggbb or #rrggbbaa"));
            }
        }
        _ => return Err(bad("Mixer color must be finite f32 RGB(A) parameters or hex")),
    }
    let color = super::color(p.get("color"), s.tools.foreground);
    if color.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err(bad("native Mixer load color must be finite unit RGBA"));
    }
    Ok(color)
}
fn same_well(a: &MixerState, b: &MixerState) -> bool {
    fn color(a: Option<[f32; 4]>, b: Option<[f32; 4]>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a.into_iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits()),
            _ => false,
        }
    }
    a.amount.to_bits() == b.amount.to_bits() && color(a.reservoir, b.reservoir) && color(a.pickup, b.pickup)
}
/// Borrowed preset/Mixer property precedence, without resolving/loading tips.
fn settings(s: &Session, p: &Value, c: &Interrupt<'_>) -> Result<MixerSettings> {
    poll(c)?;
    let mut base = &s.tools.brush.mixer;
    if let Some(value) = p.get("preset") {
        let name = value.as_str().ok_or_else(|| bad("preset must be a name"))?;
        if name.len() > 65_536 || s.tools.presets.len() > 100_000 {
            return Err(bad("preset lookup limit"));
        }
        let mut bytes = 0usize;
        let mut found = None;
        for preset in &s.tools.presets {
            poll(c)?;
            bytes = bytes.checked_add(preset.name.len()).filter(|n| *n <= 8 << 20).ok_or_else(|| bad("preset name work limit"))?;
            if preset.name.eq_ignore_ascii_case(name) {
                found = Some(&preset.brush.mixer);
                break;
            }
        }
        base = found.ok_or_else(|| bad("no such brush preset"))?;
    }
    let mut m = MixerSettings { wet: base.wet, load: base.load, mix: base.mix, flow: base.flow, sample_all_layers: base.sample_all_layers };
    if let Some(v) = p.get("brush") {
        let patch = v.as_object().ok_or_else(|| bad("brush must be an object"))?;
        if let Some(v) = patch.get("mixer") {
            let patch = v.as_object().ok_or_else(|| bad("brush Mixer must be an object"))?;
            for (key, target) in [("wet", &mut m.wet), ("load", &mut m.load), ("mix", &mut m.mix), ("flow", &mut m.flow)] {
                if let Some(v) = patch.get(key) {
                    *target = number(v, key)?;
                }
            }
            if let Some(v) = patch.get("sampleAllLayers") {
                m.sample_all_layers = v.as_bool().ok_or_else(|| bad("sampleAllLayers must be boolean"))?;
            }
        }
    }
    // Exactly the ordinary percent schedule: f32 multiply/default then /100.
    for (key, target) in [("wet", &mut m.wet), ("load", &mut m.load), ("mix", &mut m.mix), ("flow", &mut m.flow)] {
        let n = match p.get(key) {
            Some(v) => number(v, key)?,
            None => *target * 100.0,
        };
        if !n.is_finite() {
            return Err(bad("Mixer percentage overflow"));
        }
        *target = n.clamp(0.0, 100.0) / 100.0;
    }
    m.sample_all_layers = boolean(p, "sampleAllLayers", m.sample_all_layers)?;
    Ok(m)
}
fn target<'a>(s: &'a Session, p: &Value) -> Result<(crate::channel_cmds::Target, Option<LayerId>, &'a Surface)> {
    use crate::channel_cmds::{ChannelTarget, Target};
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let route = if p.get("target").is_some() {
        crate::channel_cmds::target_of(p)
    } else {
        match state.channel_view.target {
            ChannelTarget::Alpha(i) if i < state.doc.channels.len() => Target::Alpha(i),
            ChannelTarget::Composite if state.doc.quick_mask.is_some() => Target::QuickMask,
            ChannelTarget::LayerMask => Target::Mask,
            ChannelTarget::Composite if crate::mask_view_cmds::current(state).is_some() => Target::Mask,
            _ => Target::Pixels,
        }
    };
    let id = if matches!(route, Target::Pixels | Target::Mask) { Some(super::layer_id(s, p)?) } else { None };
    let surface = match route {
        Target::Alpha(i) => state.doc.channels.get(i).map(|v| &v.surface),
        Target::QuickMask => state.doc.quick_mask.as_ref().map(|v| &v.surface),
        Target::Pixels | Target::Mask => id
            .and_then(|id| state.doc.layer(id))
            .and_then(|layer| if route == Target::Mask { layer.mask.as_ref().map(|mask| mask.surface.as_ref()) } else { layer.surface() }),
    }
    .ok_or_else(|| bad("target has no editable pixels"))?;
    Ok((route, id, surface))
}
/// Native admission BEFORE normalization. false is the genuine narrow lane only.
pub(super) fn dispatch_preflight(s: &Session, p: &Value, c: &Interrupt<'_>) -> Result<bool> {
    poll(c)?;
    let (route, id, surface) = target(s, p)?;
    let fmt = surface.format();
    if !matches!(fmt.sample, SampleType::U32 | SampleType::F64) {
        return Ok(false);
    }
    super::stroke_controlled::dispatch_preflight(s, "paint.mixerBrush", p, false)?;
    if p.get("target").is_some_and(|v| v.as_str() != Some("pixels")) {
        return Err(bad("native Mixer requires literal pixel target"));
    }
    if route != crate::channel_cmds::Target::Pixels {
        return Err(bad("native Mixer supports layer pixels only"));
    }
    let id = id.ok_or_else(|| bad("missing native Mixer layer"))?;
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let doc = &state.doc;
    let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if fmt.mode != ColorMode::Rgb
        || doc.mode != ColorMode::Rgb
        || doc.depth != fmt.sample
        || !matches!(layer.content, LayerContent::Raster(_))
        || doc.xcf_state.is_some()
        || doc.source_color_context.is_some()
        || doc.color_table.is_some()
        || layer.indexed_source.is_some()
        || layer.video.is_some()
        || layer.import_compositing.is_some()
    {
        return Err(bad("native Mixer requires source-free resident RGB"));
    }
    if state.floating.is_some() || state.coalesce.is_some() || s.coalesce_request.is_some() || s.color_restrict.is_some() {
        return Err(bad("finish transient work before native Mixer"));
    }
    let m = settings(s, p, c)?;
    load_color(s, p)?;
    if m.sample_all_layers
        || m.load.to_bits() != 1.0f32.to_bits()
        || !boolean(p, "loadAfterStroke", true)?
        || !boolean(p, "cleanAfterStroke", true)?
        || s.tools.mixer.pickup.is_some()
    {
        return Err(bad("native Mixer requires current-layer sampling, full load, auto-load, clean-after and empty pickup"));
    }
    crate::commands::check_pixels_unlocked(doc, id)?;
    crate::native_linear_context::validate(doc, c).map_err(|e| if c.cancelled() { EngineError::Cancelled } else { bad(e) })?;
    if doc.selection.as_ref().is_some_and(|mask| mask.format().mode != ColorMode::Grayscale || mask.format().alpha) {
        return Err(bad("native Mixer selection must be Gray coverage without alpha"));
    }
    poll(c)?;
    Ok(true)
}
struct Capture {
    snapshot: DocumentSnapshot,
    weak: WeakDocumentSnapshot,
    revision: u64,
    active: Option<LayerId>,
    id: LayerId,
    format: PixelFormat,
    lock: bool,
    view: crate::channel_cmds::ChannelView,
    well: MixerState,
}
struct Input {
    capture: Capture,
    points: Vec<StrokePoint>,
    brush: BrushSettings,
    settings: MixerSettings,
    legacy_reservoir: [f32; 4],
}
struct Candidate {
    capture: Capture,
    output: Surface,
    damage: Rect,
    well: MixerState,
    response: Value,
}
fn capture(s: &Session, p: &Value, c: &Interrupt<'_>) -> Result<Option<Input>> {
    if !dispatch_preflight(s, p, c)? {
        return Ok(None);
    }
    let (_, id, _) = target(s, p)?;
    let id = id.ok_or_else(|| bad("missing Mixer target"))?;
    let state = s.active().ok_or(EngineError::NoDocument)?;
    if state.channel_view.color_hidden.len() > 16 || state.channel_view.alpha_visible.len() > 10_000 {
        return Err(bad("native Mixer view capture limit"));
    }
    let snapshot = state.doc.clone();
    let fmt = snapshot.layer(id).and_then(|l| l.surface()).ok_or_else(|| bad("missing Mixer target"))?.format();
    let capture = Capture {
        weak: snapshot.downgrade(),
        revision: state.revision,
        active: state.active_layer,
        id,
        format: fmt,
        lock: snapshot.effective_locks(id).transparency,
        view: state.channel_view.clone(),
        well: s.tools.mixer.clone(),
        snapshot,
    };
    let points = super::parse_points(p, "paint.mixerBrush")?;
    let mut brush = super::resolve_brush(s, p, "paint.mixerBrush")?;
    let settings = settings(s, p, c)?;
    // Preserve upstream explicit stroke override before generator construction.
    brush.opacity = 1.0;
    // Parameter-domain f32 colors remain the exact legacy well metadata. Native
    // CMS conversion happens separately in worker; no computed WORK is stored here.
    let legacy_reservoir = load_color(s, p)?;
    poll(c)?;
    Ok(Some(Input { capture, points, brush, settings, legacy_reservoir }))
}
fn prepare(input: Input, c: &Interrupt<'_>) -> Result<Candidate> {
    poll(c)?;
    let snapshot = &input.capture.snapshot;
    let native = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(snapshot, native, c).map_err(|e| io(e, c))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, native, c).map_err(|e| io(e, c))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(snapshot, c)?;
    let points = mul(input.points.capacity() as u64, std::mem::size_of::<StrokePoint>() as u64)?;
    let views = add(input.capture.view.color_hidden.capacity() as u64, input.capture.view.alpha_visible.capacity() as u64)?;
    let held = add(payload, add(mul(model, 3)?, add(mul(raster.clone_metadata_bytes, 3)?, add(points, add(views, 512)?)?)?)?)?;
    let standard = StrokeLimits::default();
    let remaining = standard.resident_bytes.checked_sub(held).ok_or_else(|| bad("native Mixer document/input headroom"))?;
    crate::native_linear_context::validate(snapshot, c).map_err(|e| if c.cancelled() { EngineError::Cancelled } else { bad(e) })?;
    let reservoir = super::native_wide_color(snapshot, input.capture.format, &Value::Null, input.legacy_reservoir.map(f64::from))?;
    poll(c)?;
    let settings = NativeMixerSettings {
        reservoir: [reservoir[0], reservoir[1], reservoir[2]],
        wet: f64::from(input.settings.wet),
        mix: f64::from(input.settings.mix),
        flow: f64::from(input.settings.flow),
    };
    let generation = StrokeLimits { resident_bytes: remaining / 4, work_units: standard.work_units / 16 };
    let mut renderer = StrokeRenderer::try_new_with_interrupt(&input.brush, None, 1.0, generation, c).map_err(paint)?.record_dabs();
    renderer.push_with_interrupt(&input.points, generation, c).map_err(paint)?;
    renderer.finish_with_interrupt(generation, c).map_err(paint)?;
    let req = renderer.incremental_clone_requirements(generation, c).map_err(paint)?;
    let source = snapshot.layer(input.capture.id).and_then(|l| l.surface()).ok_or_else(|| bad("native Mixer PRE missing"))?;
    let lower = StrokeLimits {
        resident_bytes: remaining.checked_sub(req.retained_bytes).ok_or_else(|| bad("Mixer renderer headroom"))?,
        work_units: standard.work_units - standard.work_units / 4,
    };
    let output =
        native_mixer_controlled::try_apply_full_load(source, &renderer, settings, snapshot.selection.as_ref(), input.capture.lock, lower, c).map_err(paint)?;
    let well = MixerState { reservoir: Some(input.legacy_reservoir), amount: 1.0, pickup: None };
    let damage = output.damage;
    let response = if damage.is_empty() {
        json!({"damage":[0,0,0,0],"nativeMixer":true})
    } else {
        json!({"damage":[damage.x0,damage.y0,damage.width(),damage.height()],"nativeMixer":true})
    };
    poll(c)?;
    Ok(Candidate { capture: input.capture, output: output.surface, damage, well, response })
}
fn apply(s: &mut Session, candidate: Candidate, c: &Interrupt<'_>) -> Result<Value> {
    poll(c)?;
    let captured = &candidate.capture;
    let state = s.active().ok_or(EngineError::NoDocument)?;
    if state.doc.id != captured.snapshot.id
        || state.revision != captured.revision
        || state.active_layer != captured.active
        || !captured.weak.is_current(&state.doc)
        || !state.doc.shares_state(&captured.snapshot)
        || state.channel_view != captured.view
        || !same_well(&s.tools.mixer, &captured.well)
        || state.floating.is_some()
        || state.coalesce.is_some()
        || s.coalesce_request.is_some()
        || s.color_restrict.is_some()
    {
        return Err(bad("captured native Mixer target/view/well changed"));
    }
    crate::commands::check_pixels_unlocked(&state.doc, captured.id)?;
    if state.doc.effective_locks(captured.id).transparency != captured.lock {
        return Err(bad("native Mixer alpha lock changed"));
    }
    crate::native_linear_context::validate(&state.doc, c).map_err(|e| if c.cancelled() { EngineError::Cancelled } else { bad(e) })?;
    let id = captured.id;
    let fmt = captured.format;
    let lock = captured.lock;
    let damage = candidate.damage;
    poll(c)?;
    if !damage.is_empty() {
        s.edit_with_interrupt("Mixer Brush", c, |doc, _| {
            poll(c)?;
            crate::commands::check_pixels_unlocked(doc, id)?;
            if doc.effective_locks(id).transparency != lock {
                return Err(bad("Mixer alpha lock changed"));
            }
            let target = doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or_else(|| bad("Mixer target changed"))?;
            if target.format() != fmt {
                return Err(bad("Mixer format changed"));
            }
            *target = candidate.output;
            Ok(damage)
        })?;
    }
    // Single synchronous publication fence: no poll/fallible work/allocation
    // between successful document commit and prepared exact scalar well install.
    s.tools.mixer = candidate.well;
    Ok(candidate.response)
}
pub(super) fn try_run_job(s: &mut Session, p: &Value) -> Result<Option<Value>> {
    let Some(input) = capture(s, p, &Interrupt::NONE)? else {
        return Ok(None);
    };
    let value = crate::jobs::run(
        s,
        "Mixer Brush",
        true,
        move |ctx| ctx.stage(0.0, 0.95, "Native Mixer", |c| prepare(input, c)).map(|candidate| (candidate, ctx.clone())),
        move |session, (candidate, ctx)| ctx.stage(0.95, 1.0, "Apply native Mixer", |c| apply(session, candidate, c)),
    )?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(sample: SampleType) -> Session {
        let mut doc = photocraft_doc::Document::new("Native Mixer", photocraft_doc::Size::new(8, 8), ColorMode::Rgb, sample);
        doc.sample_encoding = Some(photocraft_doc::SampleEncoding::Linear);
        doc.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
        doc.layers.push(photocraft_doc::Layer::new("Pixels", LayerContent::Raster(Surface::with_default(doc.pixel_format(), &[0.5, 0.5, 0.5, 0.75]))));
        let mut s = Session::new();
        s.add_document(doc, None).unwrap();
        s.tools.brush.pressure_size = false;
        s.tools.brush.smoothing.enabled = false;
        s
    }
    fn params() -> Value {
        json!({"points":[[2,2,1],[5,5,1]],"seed":17,"color":[0.8,0.25,0.5,0.5],"size":4,"flow":100,"wet":80,"mix":50,"load":100,"sampleAllLayers":false,"cleanAfterStroke":true,"loadAfterStroke":true})
    }
    #[test]
    fn registered_native_mixer_has_exact_well_and_one_history_step() {
        for sample in [SampleType::U32, SampleType::F64] {
            let mut s = fixture(sample);
            let before = s.active().unwrap().doc.clone();
            let revision = s.active().unwrap().revision;
            s.execute("paint.mixerBrush", params()).unwrap();
            assert_eq!(s.active().unwrap().revision, revision + 1);
            assert_eq!(s.tools.mixer.amount.to_bits(), 1.0f32.to_bits());
            assert_eq!(s.tools.mixer.reservoir, Some([0.8, 0.25, 0.5, 0.5]));
            assert!(s.tools.mixer.pickup.is_none());
            let edited = s.active().unwrap().doc.clone();
            assert!(s.undo());
            assert_eq!(&*s.active().unwrap().doc, &*before);
            assert!(s.redo());
            assert_eq!(&*s.active().unwrap().doc, &*edited);
        }
    }
    #[test]
    fn refused_well_composite_and_cancellation_keep_document_redo_and_well() {
        let mut s = fixture(SampleType::F64);
        s.edit("Redo", |doc, _| {
            doc.name = "Redo".into();
            Ok(())
        })
        .unwrap();
        assert!(s.undo());
        let before = s.active().unwrap().doc.clone();
        let well = s.tools.mixer.clone();
        for (key, value) in [("sampleAllLayers", json!(true)), ("cleanAfterStroke", json!(false)), ("loadAfterStroke", json!(false)), ("load", json!(50))] {
            let mut p = params();
            p[key] = value;
            assert!(s.execute("paint.mixerBrush", p).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
            assert!(same_well(&s.tools.mixer, &well));
            assert!(s.active().unwrap().history.can_redo());
        }
        let ctx = crate::jobs::JobCtx::default();
        ctx.cancel();
        s.set_inline_job_ctx(Some(ctx));
        assert!(matches!(s.execute("paint.mixerBrush", params()), Err(EngineError::Cancelled)));
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(same_well(&s.tools.mixer, &well));
    }
    #[test]
    fn captured_well_change_refuses_prepared_publication() {
        let mut s = fixture(SampleType::U32);
        let input = capture(&s, &params(), &Interrupt::NONE).unwrap().unwrap();
        let candidate = prepare(input, &Interrupt::NONE).unwrap();
        let original = s.active().unwrap().doc.clone();
        s.tools.mixer.pickup = Some([0.1; 4]);
        let changed = s.tools.mixer.clone();
        assert!(apply(&mut s, candidate, &Interrupt::NONE).is_err());
        assert!(s.active().unwrap().doc.shares_state(&original));
        assert!(same_well(&s.tools.mixer, &changed));
    }
}
