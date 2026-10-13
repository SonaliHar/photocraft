//! Closed admission for legacy f32 advanced brush equations.
//! This rejects silent native-wide narrowing and delegates only the explicit
//! native Mixer/Color Replacement lanes without adding resource authority.
use crate::{EngineError,Result,Session};
use photocraft_color::SampleType;
use photocraft_doc::{Document,Layer,LayerContent};
use photocraft_raster::Interrupt;
use serde_json::Value;

fn poll(control:&Interrupt<'_>)->Result<()> {control.check().map_err(|_|EngineError::Cancelled)}
fn wide(sample:SampleType)->bool {matches!(sample,SampleType::U32|SampleType::F64)}
fn unsupported(command:&str)->EngineError {
    super::bad(command,"native U32/F64 advanced brush arithmetic is unsupported; the legacy f32 brush cannot retain these native samples")
}
/// Call before brush loading/parameter clones and again at the actual handler
/// boundary. Target storage is authoritative, not Document.depth or a locator.
/// Ordinary Gray masks/channels on a wide document remain ordinary targets.
pub(super) fn target_preflight(session:&Session,command:&str,params:&Value,control:&Interrupt<'_>)->Result<()> {
    poll(control)?;
    if !matches!(command,"paint.mixerBrush"|"paint.colorReplacement") {return Ok(());}
    let state=session.active().ok_or(EngineError::NoDocument)?;
    use crate::channel_cmds::{ChannelTarget,Target};
    // Color Replacement is not channel-routed by inject_target and accepts
    // only absent target or explicit pixels, even when a channel is viewed.
    let target=if command=="paint.colorReplacement" {
        if params.get("target").is_some_and(|value|value.as_str()!=Some("pixels")) {
            return Err(super::bad(command,"Color Replacement paints layer pixels only"));
        }
        Target::Pixels
    } else if params.get("target").is_some() {
        crate::channel_cmds::target_of(params)
    } else {
        // Exact borrowed counterpart of inject_target; never clone JSON to
        // synthesize a target before this pre-normalization admission.
        match state.channel_view.target {
            ChannelTarget::Alpha(index) if index<state.doc.channels.len()=>Target::Alpha(index),
            ChannelTarget::Composite if state.doc.quick_mask.is_some()=>Target::QuickMask,
            ChannelTarget::LayerMask=>Target::Mask,
            ChannelTarget::Composite if crate::mask_view_cmds::current(state).is_some()=>Target::Mask,
            _=>Target::Pixels,
        }
    };
    let surface=match target {
        Target::Alpha(index)=>state.doc.channels.get(index).map(|channel|&channel.surface),
        Target::QuickMask=>state.doc.quick_mask.as_ref().map(|channel|&channel.surface),
        Target::Pixels|Target::Mask=>{
            let id=super::layer_id(session,params)?;
            state.doc.layer(id).and_then(|layer|if target==Target::Mask {layer.mask.as_ref().map(|mask|mask.surface.as_ref())} else {layer.surface()})
        },
    }.ok_or_else(||super::bad(command,"target has no editable pixels"))?;
    if wide(surface.format().sample) {
        if command=="paint.colorReplacement" && super::native_replace_controlled::dispatch_preflight(session,params,control)? {return Ok(());}
        if command=="paint.mixerBrush" && super::native_mixer_controlled::dispatch_preflight(session,params,control)? {return Ok(());}
        return Err(unsupported(command));
    }
    poll(control)
}
/// Complete early command admission, before float normalization. Resolves only
/// the relevant borrowed Mixer field, not a BrushSettings clone or tip load.
pub(super) fn dispatch_preflight(session:&Session,command:&str,params:&Value,control:&Interrupt<'_>)->Result<()> {
    target_preflight(session,command,params,control)?;
    if command=="paint.mixerBrush" {
        let sample_all=resolved_sample_all_layers(session,params,control)?;
        let state=session.active().ok_or(EngineError::NoDocument)?;
        mixer_source_preflight(&state.doc,sample_all,control)?;
    }
    poll(control)
}
fn resolved_sample_all_layers(session:&Session,params:&Value,control:&Interrupt<'_>)->Result<bool> {
    poll(control)?;
    let mut value=session.tools.brush.mixer.sample_all_layers;
    if let Some(name)=params.get("preset").and_then(Value::as_str) {
        // picked_over preserves smoothing/locked tip sections, but never the
        // Mixer section; PresetCaptures does not override it either.
        if name.len()>65_536||session.tools.presets.len()>100_000 {return Err(super::bad("paint.mixerBrush","preset lookup limit"));}
        let mut seen_bytes=0usize;let mut found=None;
        for preset in &session.tools.presets {
            poll(control)?;
            seen_bytes=seen_bytes.checked_add(preset.name.len()).filter(|bytes|*bytes<=8<<20).ok_or_else(||super::bad("paint.mixerBrush","preset lookup name limit"))?;
            if preset.name.eq_ignore_ascii_case(name) {found=Some(preset.brush.mixer.sample_all_layers);break;}
        }
        value=found.ok_or_else(||super::bad("paint.mixerBrush","no such brush preset"))?;
    }
    if let Some(patch)=params.get("brush").filter(|value|value.is_object())
        && let Some(mixer)=patch.get("mixer") {
        let mixer=mixer.as_object().ok_or_else(||super::bad("paint.mixerBrush","invalid brush Mixer settings"))?;
        if let Some(field)=mixer.get("sampleAllLayers") {
            value=field.as_bool().ok_or_else(||super::bad("paint.mixerBrush","invalid brush Mixer sampleAllLayers"))?;
        }
    }
    // Existing scalar flag semantics: a non-boolean scalar leaves the resolved
    // brush value intact; no new default or invalid-value normalization.
    if let Some(scalar)=params.get("sampleAllLayers").and_then(Value::as_bool) {value=scalar;}
    poll(control)?;Ok(value)
}
/// Handler supplies its already resolved Mixer setting (preset/brush/scalar
/// overrides included), before reservoir preparation and composite sampling.
/// No scan occurs for the single-target source, which target_preflight checked.
pub(super) fn mixer_source_preflight(doc:&Document,sample_all_layers:bool,control:&Interrupt<'_>)->Result<()> {
    poll(control)?;
    if !sample_all_layers {return Ok(());}
    // Legacy Sample All Layers always requests an RGBA32F flattened buffer.
    // A native-wide composite output has no qualified f32 conversion here.
    if wide(doc.depth) {return Err(unsupported("paint.mixerBrush"));}
    let mut visits=0usize;
    visible_sources(&doc.layers,0,&mut visits,control)?;
    poll(control)
}
fn visible_sources(layers:&[Layer],depth:usize,visits:&mut usize,control:&Interrupt<'_>)->Result<()> {
    if depth>100 {return Err(super::bad("paint.mixerBrush","composite source hierarchy limit"));}
    // Bound each slice before iteration as well as the aggregate, including
    // hidden nodes. No allocation or native pixel reads occur in this census.
    if layers.len()>10_000 {return Err(super::bad("paint.mixerBrush","composite source count limit"));}
    for layer in layers {
        poll(control)?;
        *visits=visits.checked_add(1).filter(|n|*n<=10_000).ok_or_else(||super::bad("paint.mixerBrush","composite source count limit"))?;
        if !layer.visible {continue;}
        if layer.surface().is_some_and(|surface|wide(surface.format().sample))
            ||layer.fill_cache.as_ref().is_some_and(|cache|wide(cache.surface.format().sample))
            ||layer.mask.as_ref().is_some_and(|mask|mask.enabled&&wide(mask.surface.format().sample)) {
            return Err(unsupported("paint.mixerBrush"));
        }
        match &layer.content {
            LayerContent::BackedRaster(source) if wide(source.format().sample)=>return Err(unsupported("paint.mixerBrush")),
            LayerContent::Group(group)=>visible_sources(&group.children,depth+1,visits,control)?,
            _=>{},
        }
    }
    Ok(())
}
