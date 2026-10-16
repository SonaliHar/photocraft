//! Channels: alpha and spot channels, Select › Save / Load Selection, Quick Mask, channel
//! targeting and visibility (the Channels panel), Image › Apply Image / Calculations, and the
//! Channels panel menu (Split / Merge Channels, Merge Spot Channel).
//!
//! Alpha channels live in [`Document::channels`] (white = selected). Quick Mask is a temporary
//! channel in [`Document::quick_mask`], so entering and leaving it are history steps like in
//! Photoshop. Which channel is targeted and which are visible is view state on
//! [`DocState::channel_view`] (not history), like the active layer.
//!
//! Pixel commands (`paint.*`, `filter.*`, `image.adjustments.*`, `edit.fill`) take
//! `"target": {"channel": i}` or `"target": "quickMask"`; [`Session::execute`] fills that in from the
//! targeted channel when the caller didn't pass a target. Targeting a single colour channel keeps
//! those commands from touching the other colour channels of the layer ([`restrict_to_color`]).

use photocraft_algo::selection as sel;
#[cfg(test)]
use photocraft_algo::selection::SelectionMode;
use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, blend::blend_channel};
use photocraft_doc::{AlphaChannel, ColorIndicates, Document, Layer, LayerContent, LayerId};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Surface, from_rgba, to_rgba};
use serde::Serialize;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{DocState, EngineError, Result, Session};
mod split_controlled;
mod merge_controlled;
mod merge_spot_controlled;
pub(crate) fn merge_spot_dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    merge_spot_controlled::dispatch_preflight(session, params, background)
}
pub(crate) fn split_dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<bool> {
    split_controlled::dispatch_preflight(session, params, background)
}
pub(crate) fn merge_dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<bool> {
    merge_controlled::dispatch_preflight(session, params, background)
}

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------------------------
// View state

/// What pixel commands edit (Channels panel click / ⌘2…⌘9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "index")]
pub enum ChannelTarget {
    /// The layers (all colour channels). In Quick Mask mode this edits the Quick Mask.
    #[default]
    Composite,
    /// One colour channel of the active layer (index into the pixel format's colour channels).
    Color(usize),
    /// An alpha or spot channel (index into [`Document::channels`]).
    Alpha(usize),
    /// The active layer's layer mask.
    LayerMask,
    /// The active layer's vector mask.
    VectorMask,
}

/// Per-document Channels panel state: the target and the eye toggles. Not part of history.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelView {
    pub target: ChannelTarget,
    /// Hidden colour channels (missing entries are visible).
    pub color_hidden: Vec<bool>,
    /// Visible alpha channels (missing entries are hidden).
    pub alpha_visible: Vec<bool>,
    pub quick_mask_hidden: bool,
    /// ⌥-click view of the active layer's mask (#196); see [`crate::mask_view_cmds`].
    pub layer_mask: Option<crate::mask_view_cmds::LayerMaskView>,
    /// Pixel editing target independent of showing a mask on the canvas.
    pub editing_mask: Option<LayerId>,
}

/// Restore imported saved-channel visibility and one active alpha-channel target.
/// The runtime has one target; multiple selected alpha channels remain canonical source
/// state and are not reduced to an arbitrary editing target.
pub fn imported_view(doc: &Document) -> ChannelView {
    imported_view_with_interrupt(doc, &photocraft_raster::Interrupt::NONE).unwrap_or_default()
}
/// Controlled view restoration; no source/raster validation or permissions added.
pub fn imported_view_with_interrupt(doc: &Document, control: &photocraft_raster::Interrupt<'_>) -> Result<ChannelView> {
    let poll = || control.check().map_err(|_| EngineError::Cancelled);
    poll()?;
    let Some(state) = &doc.xcf_state else {
        return Ok(ChannelView::default());
    };
    if state.items.len() > 10_000 || state.selection.selected.len() > 10_000 || doc.channels.len() > 100_000 {
        return Err(EngineError::Other("imported channel view exceeds structural limits".into()));
    }
    let mut visible = std::collections::HashSet::new();
    visible.try_reserve(state.items.len()).map_err(|_| EngineError::Other("imported channel visibility allocation".into()))?;
    for item in &state.items {
        poll()?;
        if let photocraft_doc::item_identity::ItemRef::Channel(id) = item.item
            && item.visible
        {
            visible.insert(id);
        }
    }
    let mut positions = std::collections::HashMap::new();
    positions.try_reserve(doc.channels.len()).map_err(|_| EngineError::Other("imported channel identity allocation".into()))?;
    let mut alpha_visible = Vec::new();
    poll()?;
    alpha_visible.try_reserve_exact(doc.channels.len()).map_err(|_| EngineError::Other("imported channel view allocation".into()))?;
    poll()?;
    for (index, channel) in doc.channels.iter().enumerate() {
        poll()?;
        positions.entry(channel.id).or_insert(index);
        alpha_visible.push(visible.contains(&channel.id));
    }
    let mut selected_alpha_count = 0usize;
    for item in &state.selection.selected {
        poll()?;
        if let photocraft_doc::item_identity::ItemRef::Channel(id) = item
            && positions.contains_key(id)
        {
            selected_alpha_count += 1;
            if selected_alpha_count > 1 {
                break;
            }
        }
    }
    let target = match state.selection.active {
        Some(photocraft_doc::item_identity::ItemRef::Channel(id)) if selected_alpha_count <= 1 => {
            positions.get(&id).copied().map_or(ChannelTarget::Composite, ChannelTarget::Alpha)
        }
        _ => ChannelTarget::Composite,
    };
    let editing_mask = if target == ChannelTarget::Composite { imported_editing_mask_with_interrupt(doc, control)? } else { None };
    poll()?;
    Ok(ChannelView { target, alpha_visible, editing_mask, ..Default::default() })
}
/// Imported explicit mask target takes precedence; otherwise exactly one editing flag
/// is admitted. Conflicting flags retain canonical state but do not choose a runtime target.
/// Filter selections and channel-owned masks are separate semantics and are not layer masks.
pub fn imported_editing_mask(doc: &Document) -> Option<LayerId> {
    imported_editing_mask_with_interrupt(doc, &photocraft_raster::Interrupt::NONE).unwrap_or(None)
}
pub fn imported_editing_mask_with_interrupt(doc: &Document, control: &photocraft_raster::Interrupt<'_>) -> Result<Option<LayerId>> {
    use photocraft_doc::item_identity::ItemRef;
    let poll = || control.check().map_err(|_| EngineError::Cancelled);
    poll()?;
    let Some(state) = &doc.xcf_state else {
        return Ok(None);
    };
    if state.items.len() > 10_000 || state.filter_masks.len() > 10_000 {
        return Err(EngineError::Other("imported mask view exceeds structural limits".into()));
    }
    let mut layers = std::collections::HashMap::new();
    photocraft_doc::resident_structure::visit_layers_with(doc, control, |layer| {
        layers.try_reserve(1).map_err(|_| photocraft_raster::RasterInputError::AllocationFailed)?;
        // Preserve native layer()'s first occurrence for malformed duplicate IDs.
        layers.entry(layer.id).or_insert(layer.mask.is_some());
        Ok(true)
    })?;
    let mut filters = std::collections::HashSet::new();
    filters.try_reserve(state.filter_masks.len()).map_err(|_| EngineError::Other("imported filter mask view allocation".into()))?;
    for mask in &state.filter_masks {
        poll()?;
        filters.insert(mask.id);
    }
    let mut first_masks = std::collections::HashMap::new();
    first_masks.try_reserve(state.items.len()).map_err(|_| EngineError::Other("imported mask identity allocation".into()))?;
    for item in &state.items {
        poll()?;
        let ItemRef::Mask(id) = item.item else {
            continue;
        };
        let owner = item.mask.as_ref().and_then(|mask| {
            let ItemRef::Layer(layer) = mask.owner else {
                return None;
            };
            (mask.editing && layers.get(&layer) == Some(&true)).then_some(layer)
        });
        first_masks.entry(id).or_insert(owner);
    }
    let owner = |reference| {
        let ItemRef::Mask(id) = reference else {
            return None;
        };
        if filters.contains(&id) {
            return None;
        }
        first_masks.get(&id).copied().flatten()
    };
    if let Some(reference) = state.selection.mask_target {
        let selected = owner(reference);
        poll()?;
        return Ok(selected);
    }
    let mut first = None;
    for item in &state.items {
        poll()?;
        if let Some(candidate) = owner(item.item) {
            if first.is_some() {
                poll()?;
                return Ok(None);
            }
            first = Some(candidate);
        }
    }
    poll()?;
    Ok(first)
}
/// The hidden layer mask editing target while its owner remains the active masked layer.
pub fn mask_editing_target(state: &DocState) -> Option<LayerId> {
    let layer = state.channel_view.editing_mask?;
    (state.active_layer == Some(layer) && state.doc.layer(layer).is_some_and(|layer| layer.mask.is_some())).then_some(layer)
}

impl ChannelView {
    pub fn color_visible(&self, k: usize) -> bool {
        !self.color_hidden.get(k).copied().unwrap_or(false)
    }
    pub fn alpha_shown(&self, i: usize) -> bool {
        self.alpha_visible.get(i).copied().unwrap_or(false)
    }
    pub fn set_color_visible(&mut self, k: usize, v: bool) {
        if self.color_hidden.len() <= k {
            self.color_hidden.resize(k + 1, false);
        }
        self.color_hidden[k] = !v;
    }
    pub fn set_alpha_visible(&mut self, i: usize, v: bool) {
        if self.alpha_visible.len() <= i {
            self.alpha_visible.resize(i + 1, false);
        }
        self.alpha_visible[i] = v;
    }
    /// Number of visible colour channels out of `colors`.
    pub fn visible_colors(&self, colors: usize) -> usize {
        (0..colors).filter(|k| self.color_visible(*k)).count()
    }
    /// True when the canvas shows just the normal composite (nothing extra to draw).
    pub fn is_plain(&self, doc: &Document) -> bool {
        let colors = color_count(doc);
        self.visible_colors(colors) == colors
            && !(0..doc.channels.len()).any(|i| self.alpha_shown(i))
            && (doc.quick_mask.is_none() || self.quick_mask_hidden)
            && self.shown_layer_mask(doc).is_none()
    }
    /// The layer mask shown on the canvas (#196) and how, if its layer still has a mask.
    pub fn shown_layer_mask<'a>(&self, doc: &'a Document) -> Option<(&'a photocraft_doc::LayerMask, crate::mask_view_cmds::MaskViewMode)> {
        let v = self.layer_mask?;
        Some((doc.layer(v.layer)?.mask.as_ref()?, v.mode))
    }
}

/// Session-wide Quick Mask Options (double-click the Quick Mask button in Photoshop).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickMaskOptions {
    pub color: Color,
    pub opacity: f32,
    pub indicates: ColorIndicates,
}

impl Default for QuickMaskOptions {
    fn default() -> Self {
        Self { color: AlphaChannel::DEFAULT_COLOR, opacity: 0.5, indicates: ColorIndicates::MaskedAreas }
    }
}

/// Keep the view valid after the document changed (undo, deleted channels, mode changes).
pub(crate) fn fix_view(st: &mut DocState) {
    let n = st.doc.channels.len();
    let colors = color_count(&st.doc);
    let quick = st.doc.quick_mask.is_some();
    if st.channel_view.editing_mask.is_some() && mask_editing_target(st).is_none() {
        st.channel_view.editing_mask = None;
    }
    let v = &mut st.channel_view;
    v.alpha_visible.truncate(n);
    v.color_hidden.truncate(colors);
    match v.target {
        ChannelTarget::Alpha(i) if i >= n => v.target = ChannelTarget::Composite,
        ChannelTarget::Color(k) if k >= colors || colors < 2 => v.target = ChannelTarget::Composite,
        _ => {}
    }
    crate::mask_view_cmds::fix(st);
    // An alpha/spot eye, Quick Mask, or the active layer's mask can be the only thing on the
    // canvas (so R, G and B can all be off). Otherwise the colour channels come back.
    let active = st.active_layer;
    let shown = {
        let doc = &st.doc;
        let v = &st.channel_view;
        (0..n).any(|i| v.alpha_shown(i)) || (quick && !v.quick_mask_hidden) || active.is_some_and(|id| doc.layer(id).is_some_and(|l| l.mask.is_some()))
    };
    if st.channel_view.visible_colors(colors) == 0 && !shown {
        st.channel_view.color_hidden.clear();
    }
}

/// Bump the revision for a view-only change without dirtying a clean document.
/// `"layer"` param of a source: a layer id, a layer name (the topmost of that name, as a loaded
/// Photoshop action refers to it), or `"merged"` (default). An unknown name is an error.

fn touch(st: &mut DocState) {
    let clean = st.saved_revision == st.revision;
    st.revision += 1;
    st.view_revision += 1;
    if clean {
        st.saved_revision = st.revision;
    }
    st.last_damage = None;
}

// ---------------------------------------------------------------------------------------------
// Channel names and formats

/// Colour channel names of a document mode (the rows under the composite).
pub fn color_names(mode: ColorMode) -> &'static [&'static str] {
    match mode {
        ColorMode::Cmyk => &["Cyan", "Magenta", "Yellow", "Black"],
        ColorMode::Lab => &["Lightness", "a", "b"],
        ColorMode::Grayscale => &["Gray"],
        _ => &["Red", "Green", "Blue"],
    }
}

/// Name of the composite row.
pub fn composite_name(mode: ColorMode) -> &'static str {
    match mode {
        ColorMode::Cmyk => "CMYK",
        ColorMode::Lab => "Lab",
        ColorMode::Grayscale => "Gray",
        _ => "RGB",
    }
}

/// The Curves dialog's Channel popup lists only this channel when the Channels panel has a
/// single non-composite target (a colour component, layer mask, saved channel or Quick Mask).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsolatedCurveChannel {
    /// Channels-panel name, including a custom alpha name or "{layer} Mask".
    pub name: String,
    /// Curves param key: empty = composite `points`; otherwise `red`/`green`/…
    pub key: &'static str,
    /// Index into the tone-space histograms (composite = 0 for RGB/CMYK).
    pub hist: usize,
    /// Colour components keep their curve tint; masks and alphas do not.
    pub tint: bool,
    /// Built-in labels (colour channels, Quick Mask) follow the interface language; a custom
    /// alpha name or a "{layer} Mask" keeps the layer name, so it stays exactly as entered (#5335).
    pub translate: bool,
    /// The gray plane this target edits, whatever the document's colour mode; its surface is
    /// what the dialog's histogram must describe (#5335). None for colour components.
    pub gray: Option<GraySurface>,
}

/// A grayscale plane Curves can target: the dialog parses and previews its curves in gray levels
/// even in Lab and CMYK documents (#5335).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraySurface {
    /// The active layer's layer mask.
    LayerMask,
    /// A saved channel, as an index into [`Document::channels`].
    Alpha(usize),
    /// The Quick Mask.
    QuickMask,
}

/// Isolated Curves channel for the active document, matching [`inject_target`].
pub fn isolated_curve_channel(st: &DocState) -> Option<IsolatedCurveChannel> {
    isolated_curve_channel_for(st, false)
}

/// [`isolated_curve_channel`] with the Layers panel's mask target added: clicking a mask
/// thumbnail targets it without touching the Channels panel, and both ways of selecting the mask
/// must give the same Curves dropdown (#5335).
pub fn isolated_curve_channel_for(st: &DocState, layers_panel_mask: bool) -> Option<IsolatedCurveChannel> {
    let doc = &st.doc;
    let mode = edit_mode(doc);
    match st.channel_view.target {
        ChannelTarget::Color(k) => {
            let names = color_names(mode);
            let keys = crate::adjust_params::channel_keys(crate::adjust_params::default_space(mode));
            let name = names.get(k)?.to_string();
            let key = *keys.get(k)?;
            let hist = if matches!(mode, ColorMode::Lab) { k } else { k.saturating_add(1) };
            Some(IsolatedCurveChannel { name, key, hist, tint: true, translate: true, gray: None })
        }
        ChannelTarget::Alpha(i) => doc.channels.get(i).map(|c| IsolatedCurveChannel {
            name: c.name.clone(),
            key: "",
            hist: 0,
            tint: false,
            translate: false,
            gray: Some(GraySurface::Alpha(i)),
        }),
        ChannelTarget::Composite if doc.quick_mask.is_some() => {
            Some(IsolatedCurveChannel { name: "Quick Mask".into(), key: "", hist: 0, tint: false, translate: true, gray: Some(GraySurface::QuickMask) })
        }
        ChannelTarget::LayerMask => layer_mask_curve_channel(st),
        ChannelTarget::Composite if crate::mask_view_cmds::current(st).is_some() || layers_panel_mask => layer_mask_curve_channel(st),
        _ => None,
    }
}

fn layer_mask_curve_channel(st: &DocState) -> Option<IsolatedCurveChannel> {
    let layer = st.active_layer.and_then(|id| st.doc.layer(id))?;
    layer.mask.as_ref()?;
    let name = format!("{} Mask", layer.name);
    Some(IsolatedCurveChannel { name, key: "", hist: 0, tint: false, translate: false, gray: Some(GraySurface::LayerMask) })
}

fn edit_mode(doc: &Document) -> ColorMode {
    doc.pixel_format().mode
}

pub fn color_count(doc: &Document) -> usize {
    edit_mode(doc).color_channels()
}

/// Pixel format of alpha channels: grayscale at the document depth, no transparency.
pub fn channel_format(doc: &Document) -> PixelFormat {
    PixelFormat::new(ColorMode::Grayscale, doc.depth, false)
}

/// Next free "`base` N" name (Alpha 1, Spot Color 2, …).
pub fn next_alpha_name(doc: &Document, base: &str) -> String {
    // `channels.len() + 1` candidates always include a free one.
    (1..=doc.channels.len() + 1).map(|n| format!("{base} {n}")).find(|n| !doc.channels.iter().any(|c| &c.name == n)).unwrap_or_else(|| base.to_string())
}

// ---------------------------------------------------------------------------------------------
// Planes: one grayscale value per canvas pixel

fn read_plane(s: &Surface, r: Rect) -> Vec<f32> {
    let mut v = Vec::new();
    s.read_region_into(r, &mut v);
    let ch = s.channels();
    if ch > 1 { v.chunks_exact(ch).map(|p| p[0]).collect() } else { v }
}

fn plane_surface(vals: &[f32], r: Rect, fmt: PixelFormat) -> Surface {
    let mut s = Surface::new(fmt);
    let clamped: Vec<f32> = vals.iter().map(|v| v.clamp(0.0, 1.0)).collect();
    s.write_region(r, &clamped);
    s.prune();
    s
}

/// Make every canvas pixel of `s` explicit so filters and adjustments that work on the content
/// bounds also see the untouched (default-valued) parts of a channel.
fn materialize(s: &mut Surface, r: Rect) {
    let v = s.read_region(r);
    s.write_region(r, &v);
}

fn luminosity(rgba: [f32; 4]) -> f32 {
    0.299 * rgba[0] + 0.587 * rgba[1] + 0.114 * rgba[2]
}

// ---------------------------------------------------------------------------------------------
// Channel references

/// A channel named in params: `"composite"`, a colour name (`"red"`, `"cyan"`, `"lightness"`…),
/// a stable `{ "channelId": id }`, alpha index (number) or name, `"quickMask"`, `"selection"`, `"transparency"`, `"mask"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChanRef {
    Composite,
    Color(usize),
    Alpha(usize),
    QuickMask,
    Selection,
    Transparency,
    LayerMask,
    /// The layer's vector mask, rasterized (⌘-click its thumbnail, #196).
    VectorMask,
}

fn parameter_ref<'a>(params: &'a Value, key: &str) -> Option<&'a Value> {
    if key == "channel" && params.get("channelId").is_some() { Some(params) } else { params.get(key) }
}

fn channel_identity(value: &Value) -> Option<u64> {
    let id = match value {
        Value::Number(number) => number.as_u64()?,
        Value::String(text) => {
            if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let id = text.parse::<u64>().ok()?;
            if id.to_string() != *text {
                return None;
            }
            id
        }
        _ => return None,
    };
    (id != 0).then_some(id)
}

pub fn parse_ref(v: &Value, doc: &Document) -> Option<ChanRef> {
    let mode = edit_mode(doc);
    match v {
        Value::Number(n) => n.as_u64().map(|i| ChanRef::Alpha(i as usize)),
        Value::Object(o) => {
            if let Some(id) = o.get("channelId") {
                let id = channel_identity(id)?;
                return doc.channels.iter().position(|channel| channel.id.0 == id).map(ChanRef::Alpha);
            }
            if let Some(k) = o.get("color").and_then(Value::as_u64) {
                Some(ChanRef::Color(k as usize))
            } else {
                o.get("alpha").and_then(Value::as_u64).map(|i| ChanRef::Alpha(i as usize))
            }
        }
        Value::String(s) => {
            let l = s.to_ascii_lowercase();
            match l.as_str() {
                "composite" | "merged" => return Some(ChanRef::Composite),
                "quickmask" | "quick mask" => return Some(ChanRef::QuickMask),
                "selection" => return Some(ChanRef::Selection),
                "transparency" => return Some(ChanRef::Transparency),
                "mask" | "layermask" => return Some(ChanRef::LayerMask),
                "vectormask" | "vector mask" => return Some(ChanRef::VectorMask),
                _ => {}
            }
            if l == composite_name(mode).to_ascii_lowercase() {
                return Some(ChanRef::Composite);
            }
            if let Some(k) = color_names(mode).iter().position(|n| n.eq_ignore_ascii_case(&l)) {
                return Some(if color_names(mode).len() == 1 { ChanRef::Composite } else { ChanRef::Color(k) });
            }
            doc.channels.iter().position(|c| c.name == *s).map(ChanRef::Alpha)
        }
        _ => None,
    }
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn doc_index(s: &Session, p: &Value, key: &str) -> Result<usize> {
    match crate::commands::int(p, key).and_then(|v| u64::try_from(v).ok()) {
        Some(i) if (i as usize) < s.documents().len() => Ok(i as usize),
        Some(i) => Err(EngineError::Other(format!("no open document {i}"))),
        None => s.active_index().ok_or(EngineError::NoDocument),
    }
}

/// Alpha channel index from `p[key]` (number or name), checked against the document.
fn alpha_index(doc: &Document, p: &Value, key: &str, cmd: &str) -> Result<usize> {
    match parameter_ref(p, key).and_then(|v| parse_ref(v, doc)) {
        Some(ChanRef::Alpha(i)) if i < doc.channels.len() => Ok(i),
        Some(ChanRef::Alpha(i)) => Err(bad(cmd, format!("no alpha channel {i} (document has {})", doc.channels.len()))),
        _ => Err(bad(cmd, format!("`{key}` must name an alpha channel (index or name)"))),
    }
}

// ---------------------------------------------------------------------------------------------
// Sources (Load Selection, Apply Image, Calculations)

/// Native pixel values of a layer or of the merged image over the canvas.
fn native_pixels(doc: &Document, layer: Option<LayerId>) -> Result<(Vec<f32>, usize)> {
    let r = doc.bounds();
    let fmt = doc.pixel_format();
    let n = fmt.channels();
    match layer {
        Some(id) => {
            let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            let s = l.surface().ok_or_else(|| EngineError::Other(format!("layer \"{}\" has no pixels", l.name)))?;
            if s.format() == fmt {
                return Ok((s.read_region(r), n));
            }
            let raw = s.read_region(r);
            let sf = s.format();
            Ok((raw.chunks_exact(sf.channels()).flat_map(|p| from_rgba(&fmt, to_rgba(&sf, p))).collect(), n))
        }
        None => {
            let buf = photocraft_compose::render(doc, r)?;
            Ok((buf.px.iter().flat_map(|p| from_rgba(&fmt, *p)).collect(), n))
        }
    }
}

/// Planes of channel `r` from document `doc` (`layer` = None: merged). A composite gives one
/// plane per colour channel; everything else gives one plane.
fn ref_planes(doc: &Document, layer: Option<LayerId>, active: Option<LayerId>, r: ChanRef) -> Result<Vec<Vec<f32>>> {
    let area = doc.bounds();
    let colors = color_count(doc);
    Ok(match r {
        ChanRef::Composite | ChanRef::Color(_) => {
            let (px, n) = native_pixels(doc, layer)?;
            let planes: Vec<Vec<f32>> = (0..colors).map(|c| px.chunks_exact(n).map(|p| p[c]).collect()).collect();
            match r {
                ChanRef::Color(k) => vec![planes.get(k).cloned().ok_or_else(|| EngineError::Other(format!("no colour channel {k}")))?],
                _ => planes,
            }
        }
        ChanRef::Alpha(i) => vec![read_plane(&doc.channels.get(i).ok_or_else(|| EngineError::Other(format!("no alpha channel {i}")))?.surface, area)],
        ChanRef::QuickMask => vec![read_plane(&doc.quick_mask.as_ref().ok_or_else(|| EngineError::Other("not in Quick Mask mode".into()))?.surface, area)],
        ChanRef::Selection => vec![sel::mask_from_surface(doc.selection.as_ref(), area)],
        ChanRef::Transparency => {
            let id = layer.or(active).ok_or_else(|| EngineError::Other("transparency needs a layer".into()))?;
            let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            match l.surface() {
                Some(s) => {
                    let mut px = vec![[0.0f32; 4]; area.width() as usize * area.height() as usize];
                    s.read_rgba_into(area, &mut px);
                    vec![px.iter().map(|p| p[3]).collect()]
                }
                // A shape can have no pixel cache yet (for example after loading a document).
                // Use its existing renderer rather than treating it as a canvas-wide fill.
                None => match &l.content {
                    LayerContent::Shape(shape) => vec![
                        photocraft_vector::CompiledShape::new(shape, photocraft_vector::DEFAULT_TOLERANCE, area)
                            .render_rgba(area)
                            .into_iter()
                            .map(|p| p[3])
                            .collect(),
                    ],
                    // Adjustment and fill layers cover the whole canvas.
                    _ => vec![vec![1.0; area.width() as usize * area.height() as usize]],
                },
            }
        }
        ChanRef::LayerMask => {
            let id = layer.or(active).ok_or_else(|| EngineError::Other("layer mask needs a layer".into()))?;
            let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            let m = l.mask.as_ref().ok_or_else(|| EngineError::Other(format!("layer \"{}\" has no mask", l.name)))?;
            vec![read_plane(&m.surface, area)]
        }
        ChanRef::VectorMask => {
            let id = layer.or(active).ok_or_else(|| EngineError::Other("vector mask needs a layer".into()))?;
            let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            let m = l.vector_mask.as_ref().ok_or_else(|| EngineError::Other(format!("layer \"{}\" has no vector mask", l.name)))?;
            // The path's shape, whether or not the mask is enabled (as Photoshop loads it).
            let shape = photocraft_doc::VectorMask { enabled: true, density: 1.0, feather: 0.0, ..m.clone() };
            vec![photocraft_vector::vector_mask_values(&shape, area)]
        }
    })
}

/// One grayscale plane (composites reduce to luminosity, as in Calculations).
fn gray_plane(doc: &Document, layer: Option<LayerId>, active: Option<LayerId>, r: ChanRef) -> Result<Vec<f32>> {
    let mut planes = ref_planes(doc, layer, active, r)?;
    if planes.len() == 1
        && let Some(only) = planes.pop()
    {
        return Ok(only);
    }
    let fmt = doc.pixel_format();
    let n = planes.first().map_or(0, Vec::len);
    let mut px = vec![0.0f32; fmt.channels()];
    Ok((0..n)
        .map(|i| {
            for (c, p) in planes.iter().enumerate() {
                px[c] = p[i];
            }
            if fmt.alpha {
                px[planes.len()] = 1.0;
            }
            luminosity(to_rgba(&fmt, &px))
        })
        .collect())
}

fn source_poll(control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn source_buffer(length: usize, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<f32>> {
    source_poll(control)?;
    let mut out = Vec::new();
    out.try_reserve_exact(length).map_err(|_| EngineError::Other("channel plane allocation failed".into()))?;
    if out.capacity() > length.checked_mul(2).ok_or_else(|| EngineError::Other("channel plane length overflow".into()))? {
        return Err(EngineError::Other("channel plane capacity exceeded".into()));
    }
    while out.len() < length {
        source_poll(control)?;
        out.resize(out.len().saturating_add(4096).min(length), 0.0);
    }
    Ok(out)
}
fn source_dimensions(doc: &Document, cmd: &str) -> Result<(Rect, usize)> {
    let width = i32::try_from(doc.size.width).map_err(|_| bad(cmd, "source width is too large"))?;
    let height = i32::try_from(doc.size.height).map_err(|_| bad(cmd, "source height is too large"))?;
    if width <= 0 || height <= 0 {
        return Err(bad(cmd, "source dimensions must be nonzero"));
    }
    let area = Rect::new(0, 0, width, height);
    let bytes = photocraft_raster::checked_region_byte_len(area, PixelFormat::RGBA32F)?;
    Ok((area, bytes / 16))
}
fn source_format(format: PixelFormat) -> Result<()> {
    if matches!(format.sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {
        return Err(EngineError::Other("channel arithmetic does not support U32/F64 source samples".into()));
    }
    if !(1..=5).contains(&format.channels()) {
        return Err(EngineError::Other("unsupported channel source format".into()));
    }
    Ok(())
}
fn source_owner<'a>(doc: &'a Document, id: Option<LayerId>, merged: bool, control: &photocraft_raster::Interrupt<'_>) -> Result<Option<&'a Layer>> {
    let mut stack = Vec::new();
    let mut found = None;
    let mut count = 0usize;
    if doc.layers.len() > 100000 {
        return Err(EngineError::Other("source layer count exceeds preparation limit".into()));
    }
    stack.try_reserve(doc.layers.len()).map_err(|_| EngineError::Other("source layer allocation failed".into()))?;
    stack.extend(doc.layers.iter().map(|layer| (layer, 0usize)));
    while let Some((layer, depth)) = stack.pop() {
        source_poll(control)?;
        count = count.checked_add(1).filter(|n| *n <= 100000).ok_or_else(|| EngineError::Other("source layer count exceeds preparation limit".into()))?;
        if depth >= 100 {
            return Err(EngineError::Other("source layer depth exceeds preparation limit".into()));
        }
        if Some(layer.id) == id {
            if found.is_some() {
                return Err(EngineError::Other("ambiguous source layer identity".into()));
            }
            found = Some(layer);
        }
        if merged {
            if let Some(surface) = layer.surface() {
                source_format(surface.format())?;
            }
            if let Some(mask) = &layer.mask {
                source_format(mask.surface.format())?;
            }
            if let Some(cache) = &layer.fill_cache {
                source_format(cache.surface.format())?;
            }
        }
        if let Some(children) = layer.children() {
            if count.checked_add(stack.len()).and_then(|n| n.checked_add(children.len())).is_none_or(|n| n > 100000) {
                return Err(EngineError::Other("source layer count exceeds preparation limit".into()));
            }
            stack.try_reserve(children.len()).map_err(|_| EngineError::Other("source layer allocation failed".into()))?;
            if stack.capacity() > 200000 {
                return Err(EngineError::Other("source layer capacity exceeds preparation limit".into()));
            }
            stack.extend(children.iter().map(|child| (child, depth + 1)));
        }
    }
    if let Some(id) = id {
        return found.map(Some).ok_or(EngineError::NoLayer(id));
    }
    Ok(None)
}
fn source_vector(mask: &photocraft_doc::VectorMask, area: Rect, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<f32>> {
    let limits = photocraft_vector::VectorPreparationLimits::default();
    if mask.path.subpaths.len() > limits.max_components {
        return Err(EngineError::Other("source vector mask count exceeds limit".into()));
    }
    let mut knots = 0usize;
    for path in &mask.path.subpaths {
        source_poll(control)?;
        knots = knots
            .checked_add(path.knots.len())
            .filter(|n| *n <= limits.max_edges)
            .ok_or_else(|| EngineError::Other("source vector mask knot count exceeds limit".into()))?;
    }
    let bytes = knots
        .checked_mul(std::mem::size_of::<photocraft_doc::Knot>())
        .and_then(|n| n.checked_add(mask.path.subpaths.len().checked_mul(std::mem::size_of::<photocraft_doc::Subpath>())?))
        .ok_or_else(|| EngineError::Other("source vector mask size overflow".into()))?;
    if bytes > limits.max_bytes / 2 {
        return Err(EngineError::Other("source vector mask copy exceeds limit".into()));
    }
    let mut subpaths = Vec::new();
    subpaths.try_reserve_exact(mask.path.subpaths.len()).map_err(|_| EngineError::Other("source vector mask allocation failed".into()))?;
    for path in &mask.path.subpaths {
        source_poll(control)?;
        let mut copy = Vec::new();
        copy.try_reserve_exact(path.knots.len()).map_err(|_| EngineError::Other("source vector mask allocation failed".into()))?;
        if copy.capacity() > path.knots.len().saturating_mul(2) {
            return Err(EngineError::Other("source vector mask capacity exceeded".into()));
        }
        for chunk in path.knots.chunks(256) {
            source_poll(control)?;
            copy.extend_from_slice(chunk);
        }
        subpaths.push(photocraft_doc::Subpath { closed: path.closed, knots: copy, op: path.op });
    }
    let shape = photocraft_doc::VectorMask {
        path: photocraft_doc::Path { subpaths, fill_rule: mask.path.fill_rule, inverted: mask.path.inverted },
        enabled: true,
        linked: mask.linked,
        density: 1.0,
        feather: 0.0,
    };
    let compiled = photocraft_vector::CompiledVectorMask::try_new_with_interrupt(&shape, limits, control)?;
    Ok(compiled.try_render_with_interrupt(area, limits, control)?)
}
fn source_native(surface: &Surface, area: Rect, control: &photocraft_raster::Interrupt<'_>, mut visit: impl FnMut(usize, &[f32]) -> Result<()>) -> Result<()> {
    source_format(surface.format())?;
    let read = photocraft_raster::backing::ReadControl { interrupt: control, work: &|_| Ok(()) };
    let view = photocraft_raster::checked_view::CheckedRasterView::resident_with_control(surface, area, &read)
        .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })?;
    let format = surface.format();
    let mut raw = [0u8; 40];
    let mut values = [0.0f32; 5];
    let mut index = 0usize;
    for y in area.y0..area.y1 {
        source_poll(control)?;
        for x in area.x0..area.x1 {
            if index.is_multiple_of(256) {
                source_poll(control)?;
            }
            let encoded = raw.get_mut(..format.bytes_per_pixel()).ok_or_else(|| EngineError::Other("invalid source pixel size".into()))?;
            view.read_logical_pixel_native_into(x, y, encoded, &read)
                .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })?;
            for (channel, value) in
                values.get_mut(..format.channels()).ok_or_else(|| EngineError::Other("invalid source channels".into()))?.iter_mut().enumerate()
            {
                *value = photocraft_color::read_sample(encoded, format.sample, channel);
                if !value.is_finite() {
                    return Err(EngineError::Other("nonfinite channel source sample".into()));
                }
            }
            visit(index, values.get(..format.channels()).ok_or_else(|| EngineError::Other("invalid source channels".into()))?)?;
            index = index.checked_add(1).ok_or_else(|| EngineError::Other("channel source index overflow".into()))?;
        }
    }
    source_poll(control)
}
fn read_plane_with_interrupt(surface: &Surface, area: Rect, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<f32>> {
    let bytes = photocraft_raster::checked_region_byte_len(area, PixelFormat::RGBA32F)?;
    let mut plane = source_buffer(bytes / 16, control)?;
    source_native(surface, area, control, |index, pixel| {
        *plane.get_mut(index).ok_or_else(|| EngineError::Other("source plane length mismatch".into()))? =
            *pixel.first().ok_or_else(|| EngineError::Other("empty source pixel".into()))?;
        Ok(())
    })?;
    Ok(plane)
}
fn source_reference(value: &Value, doc: &Document, cmd: &str) -> Result<ChanRef> {
    match value {
        Value::String(text) if text.len() <= 4096 => {}
        Value::Number(number) if number.as_u64().and_then(|n| usize::try_from(n).ok()).is_some() => {}
        Value::Object(map) if map.len() <= 2 => {
            for (key, value) in map {
                if key == "channelId" {
                    if channel_identity(value).is_none() {
                        return Err(bad(cmd, "invalid channel identity"));
                    }
                } else if matches!(key.as_str(), "color" | "alpha") && value.as_u64().and_then(|n| usize::try_from(n).ok()).is_some() {
                } else {
                    return Err(bad(cmd, "invalid channel reference"));
                }
            }
        }
        _ => return Err(bad(cmd, "invalid channel reference")),
    }
    parse_ref(value, doc).ok_or_else(|| bad(cmd, "unknown channel reference"))
}
fn source_with_interrupt(s: &Session, p: &Value, cmd: &str, gray: bool, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<Vec<f32>>> {
    source_poll(control)?;
    if !p.is_object() && !p.is_null() {
        return Err(bad(cmd, "source must be an object"));
    }
    if let Some(value) = p.get("document") {
        if !value.is_null()
            && value.as_u64().and_then(|n| usize::try_from(n).ok()).is_none()
            && value.as_f64().filter(|n| n.is_finite() && n.fract() == 0.0 && *n >= 0.0 && *n < s.documents().len() as f64).is_none()
        {
            return Err(bad(cmd, "invalid source document index"));
        }
    }
    let di = doc_index(s, p, "document")?;
    let st = s.documents().get(di).ok_or_else(|| bad(cmd, "invalid source document index"))?;
    let target = s.active().ok_or(EngineError::NoDocument)?;
    // Only the actually selected source is fenced. Unrelated inactive previews
    // are not a source-preparation policy; persisted saved attachments are not
    // runtime floating previews and are left to existing source consumers.
    if st.floating.is_some() || st.coalesce.is_some() || s.command_has_job_on(st.doc.id) {
        return Err(bad(cmd, "finish the selected source document's preview, gesture or job first"));
    }
    source_dimensions(&st.doc, cmd)?;
    source_dimensions(&target.doc, cmd)?;
    if st.doc.size != target.doc.size {
        return Err(bad(cmd, "source dimensions must match the target"));
    }
    source_format(st.doc.pixel_format())?;
    if st.doc.channels.len() > 100000 {
        return Err(bad(cmd, "source channel count exceeds preparation limit"));
    }
    let reference = match parameter_ref(p, "channel") {
        Some(value) => source_reference(value, &st.doc, cmd)?,
        None => ChanRef::Composite,
    };
    let layer = match p.get("layer") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.is_empty() || text == "merged" => None,
        Some(value) => Some(LayerId(value.as_u64().filter(|id| *id != 0).ok_or_else(|| bad(cmd, "source layer must be an ID or merged"))?)),
    };
    let invert = match p.get("invert") {
        None | Some(Value::Null) => false,
        Some(value) => value.as_bool().ok_or_else(|| bad(cmd, "invert must be boolean"))?,
    };
    source_from_snapshot_with_interrupt(&st.doc, st.active_layer, SnapshotSource { layer, channel: reference, gray, invert }, cmd, control)
}

/// Parsed, borrowed source selectors; no UI/runtime focus or authority is inferred.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SnapshotSource {
    pub layer: Option<LayerId>,
    pub channel: ChanRef,
    pub gray: bool,
    pub invert: bool,
}
/// Extract from a caller-held frozen resident document without a temporary
/// Session or document clone. The caller fences runtime previews/jobs/target
/// revision; this helper revalidates current source data and admission limits.
pub(crate) fn source_from_snapshot_with_interrupt(
    doc: &Document,
    active_layer: Option<LayerId>,
    source: SnapshotSource,
    cmd: &str,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<Vec<Vec<f32>>> {
    source_poll(control)?;
    let (area, pixels) = source_dimensions(doc, cmd)?;
    source_format(doc.pixel_format())?;
    if doc.channels.len() > 100000 {
        return Err(bad(cmd, "source channel count exceeds preparation limit"));
    }
    let SnapshotSource { layer, channel: reference, gray, invert } = source;
    if layer.is_some_and(|id| id.0 == 0) {
        return Err(bad(cmd, "source layer must be nonzero"));
    }
    let colors = color_count(doc);
    if colors == 0 || colors > 4 {
        return Err(bad(cmd, "unsupported source color channels"));
    }
    if let ChanRef::Color(channel) = reference {
        if channel >= colors {
            return Err(bad(cmd, "source color channel out of range"));
        }
    }
    match reference {
        ChanRef::Alpha(index) => {
            source_format(doc.channels.get(index).ok_or_else(|| bad(cmd, "source alpha channel out of range"))?.surface.format())?;
        }
        ChanRef::QuickMask => {
            source_format(doc.quick_mask.as_ref().ok_or_else(|| bad(cmd, "not in Quick Mask mode"))?.surface.format())?;
        }
        ChanRef::Selection => {
            if let Some(surface) = &doc.selection {
                source_format(surface.format())?;
            }
        }
        _ => {}
    }
    let needed_layer = if matches!(reference, ChanRef::LayerMask | ChanRef::Transparency | ChanRef::VectorMask) {
        Some(layer.or(active_layer).ok_or_else(|| bad(cmd, "source needs a layer"))?)
    } else {
        layer
    };
    let checked_owner = if needed_layer.is_some() || matches!(reference, ChanRef::Composite | ChanRef::Color(_)) {
        source_owner(doc, needed_layer, needed_layer.is_none() && matches!(reference, ChanRef::Composite | ChanRef::Color(_)), control)?
    } else {
        None
    };
    if let Some(owner) = checked_owner {
        if matches!(reference, ChanRef::Composite | ChanRef::Color(_) | ChanRef::Transparency) {
            if let Some(surface) = owner.surface() {
                source_format(surface.format())?;
            }
        }
        if reference == ChanRef::LayerMask {
            source_format(owner.mask.as_ref().ok_or_else(|| bad(cmd, "source layer has no mask"))?.surface.format())?;
        }
        if reference == ChanRef::VectorMask && owner.vector_mask.is_none() {
            return Err(bad(cmd, "source layer has no vector mask"));
        }
    }
    let count = if reference == ChanRef::Composite && !gray { colors } else { 1 };
    // All output capacity, grayscale reduction and a whole compositor temporary
    // are admitted together before any canvas-sized allocation (2x Vec slack).
    let peak = (pixels as u64)
        .checked_mul((count as u64).checked_mul(8).and_then(|n| n.checked_add(32)).ok_or_else(|| bad(cmd, "plane budget overflow"))?)
        .ok_or_else(|| bad(cmd, "plane budget overflow"))?;
    let metadata = 100000u64 * std::mem::size_of::<(&Layer, usize)>() as u64 * 2;
    let vector_extra = if reference == ChanRef::VectorMask { 128u64 << 20 } else { 0 };
    if peak.checked_add(metadata).and_then(|n| n.checked_add(vector_extra)).is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES) {
        return Err(bad(cmd, "source plane preparation exceeds resident limit"));
    }
    let mut planes = Vec::new();
    planes.try_reserve_exact(count).map_err(|_| bad(cmd, "plane allocation failed"))?;
    for _ in 0..count {
        planes.push(source_buffer(pixels, control)?);
    }
    let fmt = doc.pixel_format();
    let mut store = |index: usize, pixel: &[f32]| -> Result<()> {
        let value = match reference {
            ChanRef::Color(channel) => *pixel.get(channel).ok_or_else(|| bad(cmd, "source channel shape mismatch"))?,
            _ => *pixel.first().ok_or_else(|| bad(cmd, "empty source pixel"))?,
        };
        if reference == ChanRef::Composite && gray && colors > 1 {
            let mut native = [0.0f32; 5];
            native
                .get_mut(..colors)
                .ok_or_else(|| bad(cmd, "source channel shape mismatch"))?
                .copy_from_slice(pixel.get(..colors).ok_or_else(|| bad(cmd, "source channel shape mismatch"))?);
            if fmt.alpha {
                *native.get_mut(colors).ok_or_else(|| bad(cmd, "source alpha shape mismatch"))? = 1.0;
            }
            *planes.first_mut().and_then(|plane| plane.get_mut(index)).ok_or_else(|| bad(cmd, "source plane length mismatch"))? =
                luminosity(to_rgba(&fmt, &native));
        } else if reference == ChanRef::Composite && !gray {
            for (channel, plane) in planes.iter_mut().enumerate() {
                *plane.get_mut(index).ok_or_else(|| bad(cmd, "source plane length mismatch"))? =
                    *pixel.get(channel).ok_or_else(|| bad(cmd, "source channel shape mismatch"))?;
            }
        } else {
            *planes.first_mut().and_then(|plane| plane.get_mut(index)).ok_or_else(|| bad(cmd, "source plane length mismatch"))? = value;
        }
        Ok(())
    };
    match reference {
        ChanRef::Composite | ChanRef::Color(_) => match layer {
            Some(id) => {
                let owner = checked_owner.ok_or(EngineError::NoLayer(id))?;
                let surface = owner.surface().ok_or_else(|| bad(cmd, "source layer has no pixels"))?;
                source_native(surface, area, control, |index, pixel| {
                    if surface.format() == fmt {
                        store(index, pixel)
                    } else {
                        let rgba = to_rgba(&surface.format(), pixel);
                        let mut native = [0.0; 5];
                        photocraft_raster::from_rgba_into(&fmt, rgba, &mut native);
                        store(index, &native)
                    }
                })?;
            }
            None => {
                // Preserve the existing imported legacy display route; it has
                // boundary polling only, not a newly qualified canonical route.
                let buffer = if doc.xcf_state.is_some() {
                    source_poll(control)?;
                    let buffer = photocraft_compose::render(doc, area)?;
                    source_poll(control)?;
                    buffer
                } else {
                    photocraft_compose::render_with_interrupt(doc, area, control)?
                };
                if buffer.px.len() != pixels {
                    return Err(bad(cmd, "composite source length mismatch"));
                }
                for (index, rgba) in buffer.px.iter().enumerate() {
                    if index.is_multiple_of(256) {
                        source_poll(control)?;
                    }
                    if !rgba.iter().all(|value| value.is_finite()) {
                        return Err(bad(cmd, "nonfinite composite source"));
                    }
                    let mut native = [0.0; 5];
                    photocraft_raster::from_rgba_into(&fmt, *rgba, &mut native);
                    store(index, &native)?;
                }
            }
        },
        ChanRef::Alpha(index) => {
            let plane =
                read_plane_with_interrupt(&doc.channels.get(index).ok_or_else(|| bad(cmd, "source alpha channel out of range"))?.surface, area, control)?;
            planes.clear();
            planes.push(plane);
        }
        ChanRef::QuickMask => {
            let plane = read_plane_with_interrupt(&doc.quick_mask.as_ref().ok_or_else(|| bad(cmd, "not in Quick Mask mode"))?.surface, area, control)?;
            planes.clear();
            planes.push(plane);
        }
        ChanRef::Selection => {
            if let Some(surface) = &doc.selection {
                let plane = read_plane_with_interrupt(surface, area, control)?;
                planes.clear();
                planes.push(plane);
            }
        }
        ChanRef::LayerMask | ChanRef::Transparency | ChanRef::VectorMask => {
            let id = needed_layer.ok_or_else(|| bad(cmd, "source needs a layer"))?;
            let owner = checked_owner.ok_or(EngineError::NoLayer(id))?;
            match reference {
                ChanRef::LayerMask => {
                    let plane = read_plane_with_interrupt(&owner.mask.as_ref().ok_or_else(|| bad(cmd, "source layer has no mask"))?.surface, area, control)?;
                    planes.clear();
                    planes.push(plane);
                }
                ChanRef::Transparency => {
                    if let Some(surface) = owner.surface() {
                        source_native(surface, area, control, |index, pixel| {
                            let value = to_rgba(&surface.format(), pixel)[3];
                            *planes.first_mut().and_then(|plane| plane.get_mut(index)).ok_or_else(|| bad(cmd, "source plane length mismatch"))? = value;
                            Ok(())
                        })?;
                    } else {
                        for chunk in planes.first_mut().ok_or_else(|| bad(cmd, "missing source plane"))?.chunks_mut(4096) {
                            source_poll(control)?;
                            chunk.fill(1.0);
                        }
                    }
                }
                ChanRef::VectorMask => {
                    let mask = owner.vector_mask.as_ref().ok_or_else(|| bad(cmd, "source layer has no vector mask"))?;
                    let plane = source_vector(mask, area, control)?;
                    planes.clear();
                    planes.push(plane);
                }
                _ => return Err(bad(cmd, "invalid source reference")),
            }
        }
    }
    for plane in &mut planes {
        if plane.len() != pixels {
            return Err(bad(cmd, "source plane length mismatch"));
        }
        for (index, value) in plane.iter_mut().enumerate() {
            if index.is_multiple_of(256) {
                source_poll(control)?;
            }
            if !value.is_finite() {
                return Err(bad(cmd, "nonfinite source plane"));
            }
            if invert {
                *value = 1.0 - *value;
                if !value.is_finite() {
                    return Err(bad(cmd, "source inversion overflow"));
                }
            }
        }
    }
    source_poll(control)?;
    Ok(planes)
}

#[cfg(test)]
mod snapshot_source_tests {
    use super::*;
    #[test]
    fn frozen_ordinary_source_matches_existing_luminosity_and_color_planes() {
        for sample in [photocraft_color::SampleType::U8, photocraft_color::SampleType::U16, photocraft_color::SampleType::F32] {
            let mut doc = Document::new("Own source", photocraft_doc::Size::new(2, 1), photocraft_color::ColorMode::Rgb, sample);
            let mut surface = Surface::new(doc.pixel_format());
            surface.try_write_region_f64(doc.bounds(), &[0.2, 0.4, 0.8, 1.0, 0.9, 0.1, 0.3, 0.25]).unwrap();
            let layer = Layer::new("Own pixels", LayerContent::Raster(surface));
            let id = layer.id;
            doc.layers.push(layer);
            let before = doc.clone();
            let request = SnapshotSource { layer: Some(id), channel: ChanRef::Composite, gray: true, invert: false };
            let actual = source_from_snapshot_with_interrupt(&doc, Some(id), request, "owned.snapshot", &photocraft_raster::Interrupt::NONE).unwrap();
            assert_eq!(actual, vec![gray_plane(&doc, Some(id), Some(id), ChanRef::Composite).unwrap()]);
            let color = SnapshotSource { channel: ChanRef::Color(1), gray: false, ..request };
            assert_eq!(
                source_from_snapshot_with_interrupt(&doc, Some(id), color, "owned.snapshot", &photocraft_raster::Interrupt::NONE).unwrap(),
                ref_planes(&doc, Some(id), Some(id), ChanRef::Color(1)).unwrap()
            );
            let inverted = source_from_snapshot_with_interrupt(
                &doc,
                Some(id),
                SnapshotSource { invert: true, ..request },
                "owned.snapshot",
                &photocraft_raster::Interrupt::NONE,
            )
            .unwrap();
            assert_eq!(inverted, actual.into_iter().map(|plane| plane.into_iter().map(|value| 1.0 - value).collect::<Vec<_>>()).collect::<Vec<_>>());
            assert_eq!(doc, before);
        }
    }
    #[test]
    fn snapshot_source_refuses_bad_bounds_references_wide_and_cancel_without_mutation() {
        let doc = Document::new("Own source", photocraft_doc::Size::new(2, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let request = SnapshotSource { layer: None, channel: ChanRef::Color(3), gray: true, invert: false };
        assert!(source_from_snapshot_with_interrupt(&doc, None, request, "owned.snapshot", &photocraft_raster::Interrupt::NONE).is_err());
        assert!(matches!(
            source_from_snapshot_with_interrupt(&doc, None, request, "owned.snapshot", &photocraft_raster::Interrupt::cancel_only(&|| true)),
            Err(EngineError::Cancelled)
        ));
        let mut wide = doc.clone();
        wide.depth = photocraft_color::SampleType::F64;
        assert!(
            source_from_snapshot_with_interrupt(
                &wide,
                None,
                SnapshotSource { channel: ChanRef::Composite, ..request },
                "owned.snapshot",
                &photocraft_raster::Interrupt::NONE
            )
            .is_err()
        );
        let mut malformed = doc.clone();
        malformed.size.width = 0;
        assert!(source_from_snapshot_with_interrupt(&malformed, None, request, "owned.snapshot", &photocraft_raster::Interrupt::NONE).is_err());
    }
}

// ---------------------------------------------------------------------------------------------
// Blending (Apply Image / Calculations)

/// Apply Image / Calculations blending: the separable layer modes plus Add and Subtract with
/// scale and offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Blending {
    Mode(BlendMode),
    Add { scale: f32, offset: f32 },
    Subtract { scale: f32, offset: f32 },
}

impl Blending {
    pub fn parse(p: &Value) -> std::result::Result<Self, String> {
        let name = p.get("blending").and_then(Value::as_str).unwrap_or("multiply");
        let scale = p.get("scale").and_then(Value::as_f64).unwrap_or(1.0).clamp(1.0, 2.0) as f32;
        let offset = p.get("offset").and_then(Value::as_f64).unwrap_or(0.0).clamp(-255.0, 255.0) as f32 / 255.0;
        match name.to_ascii_lowercase().as_str() {
            "add" => Ok(Blending::Add { scale, offset }),
            "subtract" => Ok(Blending::Subtract { scale, offset }),
            other => match crate::commands::blend_from_str(other) {
                Some(m) if m.is_separable() && !matches!(m, BlendMode::Dissolve | BlendMode::PassThrough) => Ok(Blending::Mode(m)),
                _ => Err(format!("unsupported blending `{other}`")),
            },
        }
    }

    /// Blend `top` (the source) onto `base` (the target).
    pub fn apply(self, base: f32, top: f32) -> f32 {
        match self {
            Blending::Mode(m) => blend_channel(m, base, top),
            Blending::Add { scale, offset } => (base + top) / scale + offset,
            Blending::Subtract { scale, offset } => (base - top) / scale + offset,
        }
        .clamp(0.0, 1.0)
    }
}

/// A source spec from `p[key]`, or from flat `<prefix>Document` / `<prefix>Layer` /
/// `<prefix>Channel` / `<prefix>Invert` keys (what the generic dialog form sends).
fn source_spec(p: &Value, key: &str, prefix: &str) -> Option<Value> {
    if let Some(v) = p.get(key) {
        return Some(v.clone());
    }
    let mut m = serde_json::Map::new();
    for (k, field) in [("document", "Document"), ("layer", "Layer"), ("channel", "Channel"), ("invert", "Invert")] {
        match p.get(format!("{prefix}{field}")) {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if s.is_empty() || (k == "layer" && s == "merged") => {}
            Some(v) => {
                m.insert(k.into(), v.clone());
            }
        }
    }
    (!m.is_empty()).then_some(Value::Object(m))
}

fn mask_wire(value: &Value, cmd: &str) -> Result<()> {
    match value {
        Value::String(text) if text.len() <= 4096 => Ok(()),
        Value::Number(_) | Value::Bool(_) | Value::Null => Ok(()),
        Value::Object(map) if map.len() <= 2 => {
            for (key, value) in map {
                if key.len() > 32 || value.is_array() || value.is_object() {
                    return Err(bad(cmd, "invalid nested mask parameter"));
                }
                mask_wire(value, cmd)?;
            }
            Ok(())
        }
        _ => Err(bad(cmd, "mask parameter exceeds bounded scalar shape")),
    }
}
fn mask_plane_with_interrupt(s: &Session, p: &Value, cmd: &str, control: &photocraft_raster::Interrupt<'_>) -> Result<Option<Vec<f32>>> {
    source_poll(control)?;
    let owned;
    let spec = if let Some(value) = p.get("mask") {
        value
    } else {
        let mut map = serde_json::Map::new();
        for (key, field) in [("document", "maskDocument"), ("layer", "maskLayer"), ("channel", "maskChannel"), ("invert", "maskInvert")] {
            if let Some(value) = p.get(field) {
                mask_wire(value, cmd)?;
                if value.is_null() || value.as_str().is_some_and(|text| text.is_empty() || (key == "layer" && text == "merged")) {
                    continue;
                }
                map.insert(key.into(), value.clone());
            }
        }
        if map.is_empty() {
            return Ok(None);
        }
        owned = Value::Object(map);
        &owned
    };
    match spec {
        Value::Null | Value::Bool(false) => Ok(None),
        Value::Object(map) if map.len() <= 8 => {
            for (key, value) in map {
                if key.len() > 32 {
                    return Err(bad(cmd, "mask parameter key exceeds limit"));
                }
                mask_wire(value, cmd)?;
            }
            if spec.get("channel").is_none_or(|channel| channel == "none") {
                return Ok(None);
            }
            let planes = source_with_interrupt(s, spec, cmd, true, control)?;
            let plane = planes.into_iter().next().ok_or_else(|| bad(cmd, "mask source has no plane"))?;
            source_poll(control)?;
            Ok(Some(plane))
        }
        _ => Err(bad(cmd, "mask source must be a bounded source object or false")),
    }
}

// Controlled source/mask regression drafts, intentionally unrun.
#[cfg(test)]
mod checked_source_drafts {
    use super::*;
    use photocraft_color::SampleType;
    use photocraft_raster::Interrupt;
    #[test]
    fn supported_native_source_and_luminosity_match_existing_math() {
        for depth in [8, 16, 32] {
            let mut session = Session::new();
            session.execute("file.new", json!({"width":3,"height":2,"depth":depth})).unwrap();
            let active = session.active().unwrap();
            let id = active.active_layer.unwrap();
            for gray in [false, true] {
                let expected = if gray {
                    vec![gray_plane(&active.doc, Some(id), active.active_layer, ChanRef::Composite).unwrap()]
                } else {
                    ref_planes(&active.doc, Some(id), active.active_layer, ChanRef::Composite).unwrap()
                };
                let actual = source_with_interrupt(&session, &json!({"layer":id.0}), "own-source-test", gray, &Interrupt::NONE).unwrap();
                assert_eq!(actual, expected);
            }
        }
    }
    #[test]
    fn source_indices_mask_shapes_and_early_cancel_fail_gracefully() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        for params in [
            json!({"document":1e100}),
            json!({"document":-1}),
            json!({"layer":{}}),
            json!({"channel":{"alpha":usize::MAX}}),
            json!({"channel":{"color":9}}),
            json!({"invert":"yes"}),
        ] {
            assert!(source_with_interrupt(&session, &params, "own-source-test", false, &Interrupt::NONE).is_err());
        }
        assert!(mask_plane_with_interrupt(&session, &json!({"mask":{"channel":{"alpha":{"nested":true}}}}), "own-mask-test", &Interrupt::NONE).is_err());
        assert!(mask_plane_with_interrupt(&session, &json!({"mask":true}), "own-mask-test", &Interrupt::NONE).is_err());
        assert!(mask_plane_with_interrupt(&session, &json!({"mask":false}), "own-mask-test", &Interrupt::NONE).unwrap().is_none());
        let cancel = || true;
        assert!(matches!(source_with_interrupt(&session, &json!({}), "own-source-test", true, &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
    }
    #[test]
    fn checked_plane_refuses_wide_nonfinite_and_reversed_geometry() {
        for depth in [SampleType::U32, SampleType::F64] {
            let surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
            assert!(read_plane_with_interrupt(&surface, Rect::new(0, 0, 1, 1), &Interrupt::NONE).is_err());
        }
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        let area = Rect::new(0, 0, 1, 1);
        surface.try_write_interleaved(area, &f32::NAN.to_ne_bytes()).unwrap();
        let before = surface.clone();
        assert!(read_plane_with_interrupt(&surface, area, &Interrupt::NONE).is_err());
        assert_eq!(surface, before);
        assert!(read_plane_with_interrupt(&surface, Rect::new(2, 0, 1, 1), &Interrupt::NONE).is_err());
    }
    #[test]
    fn only_selected_source_gestures_and_runtime_previews_block_preparation() {
        let mut session = Session::new();
        for _ in 0..3 {
            session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        }
        session.docs[0].coalesce = Some("unrelated inactive gesture".into());
        let params = json!({"document":1});
        assert!(source_with_interrupt(&session, &params, "own-source-test", true, &Interrupt::NONE).is_ok());
        session.docs[1].coalesce = Some("selected source gesture".into());
        assert!(source_with_interrupt(&session, &params, "own-source-test", true, &Interrupt::NONE).is_err());
        assert!(mask_plane_with_interrupt(&session, &json!({"mask":{"document":1,"channel":"composite"}}), "own-mask-test", &Interrupt::NONE).is_err());
        session.docs[1].coalesce = None;
        session.docs[0].coalesce = None;
        session.active = Some(0);
        session.execute("select.all", json!({})).unwrap();
        session.execute("select.float", json!({})).unwrap();
        assert!(session.docs[0].floating.is_some());
        session.active = Some(2);
        assert!(source_with_interrupt(&session, &params, "own-source-test", true, &Interrupt::NONE).is_ok());
        assert!(source_with_interrupt(&session, &json!({"document":0}), "own-source-test", true, &Interrupt::NONE).is_err());
    }
}

// ---------------------------------------------------------------------------------------------
// Targets for pixel commands

/// Where a pixel command writes, from its `"target"` param.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Pixels,
    Mask,
    Alpha(usize),
    QuickMask,
}

pub(crate) fn target_of(p: &Value) -> Target {
    match p.get("target") {
        Some(Value::String(s)) => match s.as_str() {
            "mask" => Target::Mask,
            "quickMask" => Target::QuickMask,
            _ => Target::Pixels,
        },
        Some(Value::Object(o)) => o.get("channel").and_then(Value::as_u64).map_or(Target::Pixels, |i| Target::Alpha(i as usize)),
        _ => Target::Pixels,
    }
}

/// The target is an alpha channel or the Quick Mask (no layer involved).
pub(crate) fn is_channel_target(p: &Value) -> bool {
    matches!(target_of(p), Target::Alpha(_) | Target::QuickMask)
}

/// The surface a pixel command writes to and whether the layer's transparency is locked.
pub(crate) fn target_surface<'a>(doc: &'a mut Document, layer: Option<LayerId>, p: &Value) -> Result<(&'a mut Surface, bool)> {
    match target_of(p) {
        Target::Alpha(i) => {
            let n = doc.channels.len();
            let id = doc.channels.get(i).ok_or_else(|| EngineError::Other(format!("no alpha channel {i} (document has {n})")))?.id;
            crate::xcf_item_locks::require_channel_content(doc, id)?;
            doc.channels.get_mut(i).map(|c| (&mut c.surface, false)).ok_or_else(|| EngineError::Other(format!("no alpha channel {i} (document has {n})")))
        }
        Target::QuickMask => doc.quick_mask.as_mut().map(|c| (&mut c.surface, false)).ok_or_else(|| EngineError::Other("not in Quick Mask mode".into())),
        t => {
            let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            let lock = t == Target::Pixels && crate::xcf_item_locks::transparency_locked(doc, id)?;
            Ok((crate::commands::paint_surface(doc, id, p)?, lock))
        }
    }
}

pub(crate) fn read_target_surface<'a>(doc: &'a Document, layer: Option<LayerId>, p: &Value) -> Result<&'a Surface> {
    match target_of(p) {
        Target::Alpha(i) => {
            let n = doc.channels.len();
            doc.channels.get(i).map(|c| &c.surface).ok_or_else(|| EngineError::Other(format!("no alpha channel {i} (document has {n})")))
        }
        Target::QuickMask => doc.quick_mask.as_ref().map(|c| &c.surface).ok_or_else(|| EngineError::Other("not in Quick Mask mode".into())),
        Target::Pixels => {
            let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            doc.layer(id).and_then(Layer::surface).ok_or(EngineError::NoLayer(id))
        }
        Target::Mask => {
            let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            doc.layer(id).and_then(|l| l.mask.as_ref()).map(|m| m.surface.as_ref()).ok_or(EngineError::NoLayer(id))
        }
    }
}

/// The grayscale surface filters and adjustments edit instead of the layer's pixels (made explicit
/// over the canvas), if the params target one: an alpha channel, the Quick Mask, or the mask of
/// `layer`.
pub(crate) fn channel_surface_for_filter<'a>(doc: &'a mut Document, layer: Option<LayerId>, p: &Value) -> Result<Option<&'a mut Surface>> {
    if target_of(p) == Target::Pixels {
        return Ok(None);
    }
    let b = doc.bounds();
    let (surf, _) = target_surface(doc, layer, p)?;
    materialize(surf, b);
    Ok(Some(surf))
}

/// Commands that edit a targeted layer mask in place of the layer: the Image › Adjustments table,
/// the filters and the fills (Edit › Fill and its keys). A mask target enables them on any layer
/// with a mask, so ⌘I inverts an adjustment layer's mask (#780) and Fill fills it (#2456) as in
/// Photoshop.
fn edits_mask(id: &str) -> bool {
    id.strip_prefix("image.adjustments.").is_some_and(|kind| crate::adjust_params::default_for(kind, ColorMode::Rgb).is_ok())
        || crate::filters::params_for(id, &Value::Null).is_some()
        || id == "edit.fill"
        // Copying a targeted mask (#2928).
        || id == "edit.copy"
        || crate::fill_key_cmds::IDS.contains(&id)
}

/// The precondition of `id` when `p` targets the active layer's mask (`"target":"mask"`): the
/// layer needs a mask. None when another precondition applies.
pub(crate) fn mask_target_enabled(s: &Session, id: &str, p: &Value) -> Option<std::result::Result<(), String>> {
    if target_of(p) != Target::Mask || !edits_mask(id) {
        return None;
    }
    Some(crate::active_layer_of(s).and_then(|l| if l.mask.is_some() { Ok(()) } else { Err("the active layer has no layer mask".into()) }))
}

/// Whether command `id` edits the targeted channel or mask when its params name no `"target"`
/// (filters, adjustments, paint and fill commands); the shell adds its mask target to these.
pub fn follows_target(id: &str) -> bool {
    // Moving the selected pixels (⌘-drag, Move tool) moves the targeted mask's, not the layer's.
    // Moving a layer with its unlinked mask targeted moves the mask alone (`layer.translate`).
    routed(id) || id == "select.float" || id == "layer.translate"
}

/// Commands whose target follows the Channels panel.
fn routed(id: &str) -> bool {
    id.starts_with("filter.")
        || id == "plugin.run"
        || id.starts_with("image.adjustments.")
        || matches!(id, "paint.stroke" | "paint.pencil" | "paint.bucket" | "paint.gradient" | "paint.mixerBrush" | "edit.fill" | "image.applyImage")
        || crate::fill_key_cmds::IDS.contains(&id)
        // Pasting into a targeted mask or channel (#1035); Copy takes a targeted mask (#2928).
        || matches!(id, "edit.copy" | "edit.paste" | "edit.pasteSpecial.pasteInPlace" | "edit.pasteSpecial.pasteInto" | "edit.pasteSpecial.pasteOutside")
        // Edit › Transform › Rotate/Flip turn a targeted mask or channel (#2835).
        || matches!(
            id,
            "edit.transform.rotate180"
                | "edit.transform.rotate90Cw"
                | "edit.transform.rotate90Ccw"
                | "edit.transform.flipHorizontal"
                | "edit.transform.flipVertical"
        )
        || matches!(
            id,
            "paint.cloneStamp"
                | "paint.patternStamp"
                | "paint.healingBrush"
                | "paint.spotHealing"
                | "paint.dodge"
                | "paint.burn"
                | "paint.sponge"
                | "paint.blur"
                | "paint.sharpen"
                | "paint.smudge"
                | "paint.historyBrush"
                | "paint.redEye"
                | "paint.remove"
        )
}

/// Fill in `"target"` from the targeted channel (or Quick Mask mode) when the caller gave none.
pub(crate) fn inject_target(s: &Session, id: &str, params: Value) -> Value {
    if !routed(id) || params.get("target").is_some() {
        return params;
    }
    let Some(st) = s.active() else { return params };
    let t = match st.channel_view.target {
        ChannelTarget::Alpha(i) if i < st.doc.channels.len() => json!({ "channel": i }),
        ChannelTarget::Composite if st.doc.quick_mask.is_some() => json!("quickMask"),
        ChannelTarget::LayerMask => json!("mask"),
        // Viewing the active layer's mask (⌥-click): pixel commands edit the mask.
        ChannelTarget::Composite if crate::mask_view_cmds::current(st).is_some() => json!("mask"),
        _ => return params,
    };
    match params {
        Value::Object(mut m) => {
            m.insert("target".into(), t);
            Value::Object(m)
        }
        _ => json!({ "target": t }),
    }
}

/// The colour channel pixel commands are limited to, if a single colour channel is targeted.
pub(crate) fn color_restriction(s: &Session, id: &str, params: &Value) -> Option<usize> {
    if !routed(id) || is_channel_target(params) || crate::commands::is_mask_target(params) {
        return None;
    }
    match s.active()?.channel_view.target {
        ChannelTarget::Color(k) => Some(k),
        _ => None,
    }
}

/// After a pixel edit with colour channel `k` targeted, put the other channels (and the layer's
/// transparency) of layer `id` back as they were. Only changed tiles are touched.
pub(crate) fn restrict_to_color(before: &Document, after: &mut Document, id: LayerId, k: usize) {
    let Some(old) = before.layer(id).and_then(Layer::surface) else { return };
    let Some(new) = after.layer_mut(id).and_then(Layer::surface_mut) else { return };
    if old.format() != new.format() {
        return;
    }
    let mut coords: Vec<TileCoord> = new.tiles().filter(|(c, t)| old.tile(**c).is_none_or(|o| !std::sync::Arc::ptr_eq(o, t))).map(|(c, _)| *c).collect();
    coords.extend(old.tiles().filter(|(c, _)| new.tile(**c).is_none()).map(|(c, _)| *c));
    let n = new.channels();
    for c in coords {
        let r = c.rect();
        let o = old.read_region(r);
        let mut v = new.read_region(r);
        for (pn, po) in v.chunks_exact_mut(n).zip(o.chunks_exact(n)) {
            for ch in 0..n {
                if ch != k {
                    pn[ch] = po[ch];
                }
            }
        }
        new.write_region(r, &v);
    }
    new.prune();
}

/// The colour channel Clear is limited to: the Channels panel targets a single one.
pub(crate) fn targeted_color(s: &Session) -> Option<usize> {
    match s.active()?.channel_view.target {
        ChannelTarget::Color(k) => Some(k),
        _ => None,
    }
}

/// Clear (Delete) with colour channel `k` of layer `id` targeted (#2720): fill the selected area
/// (or the canvas) of that channel only with the background colour's value for it, as Photoshop
/// does. A channel has no transparency of its own, so the other channels and the layer's alpha
/// stay; partly selected pixels blend.
pub(crate) fn clear_color_channel(doc: &mut Document, id: LayerId, k: usize, background: [f32; 4]) -> Result<()> {
    let canvas = doc.bounds();
    let sel = doc.selection.clone();
    let area = sel.as_ref().map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
    let fmt = surf.format();
    let n = fmt.channels();
    // The background colour in the layer's colour model, so `k` picks the right component.
    let Some(v) = from_rgba(&fmt, [background[0], background[1], background[2], 1.0]).get(k).copied().filter(|_| k < n) else {
        return Err(EngineError::Other(format!("no colour channel {k} in this layer")));
    };
    if area.is_empty() {
        return Ok(());
    }
    let cover =
        sel.as_ref().map(|m| crate::allocation::read_region(m, area, "clearing a channel selection").map(|v| (v, m.format().channels().max(1)))).transpose()?;
    let mut px = crate::allocation::read_region(surf, area, "clearing a channel")?;
    for (i, p) in px.chunks_exact_mut(n).enumerate() {
        let w = cover.as_ref().map_or(1.0, |(m, mk)| m.get(i * mk).copied().unwrap_or(0.0));
        if let Some(c) = p.get_mut(k) {
            *c += (v - *c) * w.clamp(0.0, 1.0);
        }
    }
    surf.try_write_region(area, &px)?;
    surf.prune();
    Ok(())
}

/// The alpha channel or Quick Mask the Channels panel targets, as `"target"` params: Clear
/// (Delete) fills it instead of the layer (#2734).
pub(crate) fn targeted_channel(s: &Session) -> Option<Value> {
    let st = s.active()?;
    let t = match st.channel_view.target {
        ChannelTarget::Alpha(i) if i < st.doc.channels.len() => json!({ "channel": i }),
        ChannelTarget::Composite if st.doc.quick_mask.is_some() => json!("quickMask"),
        _ => return None,
    };
    Some(json!({ "target": t }))
}

/// Clear (Delete) with an alpha channel or the Quick Mask targeted (`target` from
/// [`targeted_channel`]): fill the selected area (or the canvas) of it with the background colour,
/// as gray, as Photoshop does. Partly selected pixels blend.
pub(crate) fn clear_channel(doc: &mut Document, target: &Value, background: [f32; 4]) -> Result<()> {
    let canvas = doc.bounds();
    let sel = doc.selection.clone();
    let area = sel.as_ref().map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    if area.is_empty() {
        return Ok(());
    }
    let (surf, _) = target_surface(doc, None, target)?;
    crate::fill_cmds::fill_color(surf, area, [background[0], background[1], background[2], 1.0], sel.as_ref())
}

/// The active document targets an alpha channel or is in Quick Mask mode, so pixel commands
/// work without a pixel layer.
pub(crate) fn edits_channel(s: &Session) -> bool {
    s.active().is_some_and(|st| {
        st.doc.quick_mask.is_some() && st.channel_view.target == ChannelTarget::Composite
            || matches!(st.channel_view.target, ChannelTarget::Alpha(i) if i < st.doc.channels.len())
    })
}

// ---------------------------------------------------------------------------------------------
// Enabled predicates

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn has_channels(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.active().is_some_and(|d| !d.doc.channels.is_empty()) { Ok(()) } else { Err("the document has no alpha channels".into()) }
}
fn has_selection(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.active().is_some_and(|d| d.doc.selection.is_some()) { Ok(()) } else { Err("no selection".into()) }
}
fn has_spot(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.active().is_some_and(|d| d.doc.channels.iter().any(|c| c.spot.is_some())) { Ok(()) } else { Err("the document has no spot channels".into()) }
}
fn can_split(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if color_count(&d.doc) + d.doc.channels.len() < 2 { Err("only one channel to split".into()) } else { Ok(()) }
}
fn can_merge(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if mergeable(s).len() >= 2 { Ok(()) } else { Err("Merge Channels needs at least two open grayscale documents of the same size".into()) }
}

// ---------------------------------------------------------------------------------------------
// Save / Load Selection

mod selection_storage_controlled;
mod save_selection_controlled;

pub(crate) fn save_selection_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    save_selection_controlled::dispatch_preflight(s, "select.saveSelection", p, background).map(|_| ())
}

fn save_selection(s: &mut Session, p: &Value) -> Result<Value> {
    save_selection_controlled::save_selection(s, p)
}

mod load_selection_controlled;

/// Conservative metadata-only raw Load candidates for one captured same-document dialog.
/// Dispatch still validates actual canonical storage/locks and captured revision; these IDs
/// provide no source qualification, mutation permission, profile alias, or resource grant.
pub fn raw_load_storage_hints(document:&crate::DocumentSnapshot,control:&photocraft_raster::Interrupt<'_>)->Result<Vec<photocraft_doc::item_identity::ChannelId>> {
    load_selection_controlled::canonical_selection_storage::hints(document,control)
}

/// Conservative same-document raw Save replacement candidates, computed only from metadata.
/// The existing canonical selection is the source; returned saved IDs grant neither imported
/// creation nor renaming, and dispatch still validates storage/locks/revision independently.
pub fn raw_save_storage_hints(document:&crate::DocumentSnapshot,control:&photocraft_raster::Interrupt<'_>)->Result<Vec<photocraft_doc::item_identity::ChannelId>> {
    load_selection_controlled::canonical_selection_storage::save_hints(document,control)
}

/// Once-per-dialog metadata hint for explicit ordinary New Channel raw selection storage.
/// This allocates no identity and reads no samples; full frozen command admission still applies.
pub fn raw_new_selection_storage_hint(document:&crate::DocumentSnapshot,control:&photocraft_raster::Interrupt<'_>)->Result<bool> {
    load_selection_controlled::canonical_selection_storage::new_hint(document,control)
}

pub(crate) fn load_selection_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    load_selection_controlled::dispatch_preflight(s, p, background)
}

pub(crate) use load_selection_controlled::stored_load_publication::StoredLoadOffer;

/// Dispatch Load Selection with an independently bound original source held by the host.
/// The binding is used only for this attempt and never becomes serialized authority.
pub fn load_stored_selection_with_binding(
    session: &mut Session,
    binding: &crate::xcf_channel_consumer::SourceBinding<'_>,
    limits: crate::xcf_channel_stored_load::Limits,
    context: &crate::jobs::JobCtx,
) -> Result<Value> {
    load_selection_controlled::stored_load_publication::load_stored_selection_with_binding(session, binding, limits, context)
}

fn load_selection(s: &mut Session, p: &Value) -> Result<Value> {
    load_selection_controlled::run(s, p)
}

// ---------------------------------------------------------------------------------------------
// Quick Mask

/// Quick Mask channel for the current selection: white where selected; no selection = an empty
/// mask (nothing masked), as in Photoshop.
#[cfg(test)]
fn quick_mask_from(doc: &Document, o: &QuickMaskOptions) -> AlphaChannel {
    let area = doc.bounds();
    let fmt = channel_format(doc);
    let surface = match &doc.selection {
        Some(s) => plane_surface(&sel::mask_from_surface(Some(s), area), area, fmt),
        None => unselected_surface(area, fmt, o.indicates),
    };
    AlphaChannel { color: o.color, opacity: o.opacity, indicates: o.indicates, ..AlphaChannel::new("Quick Mask", surface) }
}

/// The selection a Quick Mask channel turns back into (an untouched or all-white mask = none).
#[cfg(test)]
fn selection_from(doc: &Document, ch: &AlphaChannel) -> Option<Surface> {
    let area = doc.bounds();
    let vals = read_plane(&ch.surface, area);
    if doc.quick_mask_unselected && ch.indicates != ColorIndicates::SelectedAreas && uniform(&vals, 1.0) {
        return None;
    }
    sel::combine(None, &vals, area, SelectionMode::Replace)
}

mod quick_mask_controlled;

pub(crate) fn quick_mask_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    quick_mask_controlled::dispatch_preflight(s, p, background)
}

fn quick_mask(s: &mut Session, p: &Value) -> Result<Value> {
    quick_mask_controlled::run(s, p)
}

// ---------------------------------------------------------------------------------------------
// Channel management

fn indicates_param(v: Option<&Value>) -> Option<ColorIndicates> {
    match v?.as_str()? {
        "masked" | "maskedAreas" => Some(ColorIndicates::MaskedAreas),
        "selected" | "selectedAreas" => Some(ColorIndicates::SelectedAreas),
        _ => None,
    }
}

mod new_controlled;

pub(crate) fn new_channel_dispatch_preflight(s: &Session, p: &Value, spot: bool) -> Result<()> {
    new_controlled::dispatch_preflight(s, p, spot)
}

fn new_channel(s: &mut Session, p: &Value) -> Result<Value> {
    new_controlled::run(s, p, false)
}

fn new_spot(s: &mut Session, p: &Value) -> Result<Value> {
    new_controlled::run(s, p, true)
}

mod duplicate_controlled;

pub(crate) fn duplicate_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    duplicate_controlled::dispatch_preflight(s, p, background)
}

fn duplicate_channel(s: &mut Session, p: &Value) -> Result<Value> {
    duplicate_controlled::run(s, p)
}

/// A one-layer grayscale document whose pixels are `plane` (Split Channels, Duplicate Channel
/// to a new document).
fn gray_document(name: &str, like: &Document, plane: &[f32]) -> Result<Document> {
    let mut d = Document::new(name, like.size, ColorMode::Grayscale, like.depth);
    d.set_physical_resolution(like.effective_resolution()).map_err(|e| EngineError::Other(e.to_string()))?;
    let fmt = d.pixel_format();
    let mut bg = Layer::raster("Background", fmt);
    bg.locks.transparency = true;
    bg.locks.position = true;
    if let Some(s) = bg.surface_mut() {
        let data: Vec<f32> = plane.iter().flat_map(|v| [v.clamp(0.0, 1.0), 1.0]).collect();
        s.write_region(like.bounds(), &data);
        s.prune();
    }
    d.layers.push(bg);
    Ok(d)
}

mod management_controlled;

pub(crate) fn management_dispatch_preflight(s: &Session, command: &str, p: &Value, background: bool) -> Result<()> {
    management_controlled::dispatch_preflight(s, command, p, background)
}

fn delete_channel(s: &mut Session, p: &Value) -> Result<Value> {
    management_controlled::run(s, "channel.delete", p)
}

fn fix_view_now(st: &mut DocState) {
    fix_view(st);
    touch(st);
}

fn rename_channel(s: &mut Session, p: &Value) -> Result<Value> {
    management_controlled::run(s, "channel.rename", p)
}

fn move_channel(s: &mut Session, p: &Value) -> Result<Value> {
    management_controlled::run(s, "channel.move", p)
}

/// Make `t` the edit target and show it the way Photoshop does when its row is clicked.
fn target(s: &mut Session, t: ChannelTarget) {
    let Some(st) = s.active_mut() else { return };
    let colors = color_count(&st.doc);
    let n = st.doc.channels.len();
    let v = &mut st.channel_view;
    v.target = t;
    v.editing_mask = None;
    match t {
        ChannelTarget::Composite => {
            v.color_hidden.clear();
            v.alpha_visible.clear();
        }
        ChannelTarget::LayerMask | ChannelTarget::VectorMask => v.color_hidden.clear(),
        ChannelTarget::Color(k) => {
            v.color_hidden = (0..colors).map(|c| c != k).collect();
            v.alpha_visible.clear();
        }
        ChannelTarget::Alpha(i) => {
            v.color_hidden = vec![true; colors];
            v.alpha_visible = (0..n).map(|c| c == i).collect();
        }
    }
    fix_view_now(st);

    s.tools.target_mask(t == ChannelTarget::LayerMask);
}

mod view_controlled;

pub(crate) fn view_dispatch_preflight(s: &Session, command: &str, p: &Value, background: bool) -> Result<()> {
    view_controlled::dispatch_preflight(s, command, p, background)
}

fn target_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    view_controlled::run(s, "channel.target", p)
}

const SLOT_KEY: &str = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };

fn target_slot(s: &mut Session, slot: usize) -> Result<Value> {
    view_controlled::target_slot(s, slot)
}

fn set_visible(s: &mut Session, p: &Value) -> Result<Value> {
    view_controlled::run(s, "channel.setVisible", p)
}

mod options_controlled;

pub(crate) fn options_dispatch_preflight(s: &Session, p: &Value, background: bool) -> Result<()> {
    options_controlled::dispatch_preflight(s, p, background)
}

fn options(s: &mut Session, p: &Value) -> Result<Value> {
    options_controlled::run(s, p)
}

/// Merge Spot Channel: flattens the image and prints the spot ink into the colour channels.
fn merge_spot(s: &mut Session, p: &Value) -> Result<Value> {
    merge_spot_controlled::run(s, p)
}
/// Split Channels: one grayscale document per colour and alpha channel; closes the original.
fn split(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(value) = split_controlled::try_run(s, p)? { return Ok(value); }
    let idx = s.active_index().ok_or(EngineError::NoDocument)?;
    let doc = s.documents()[idx].doc.clone();
    let area = doc.bounds();
    let (px, n) = native_pixels(&doc, None)?;
    let mode = edit_mode(&doc);
    let names = color_names(mode);
    let short = |name: &str| name.chars().next().map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default();
    let mut docs: Vec<Document> = Vec::new();
    for (c, name) in names.iter().enumerate() {
        let plane: Vec<f32> = px.chunks_exact(n).map(|p| p[c]).collect();
        docs.push(gray_document(&format!("{}_{}", doc.name, short(name)), &doc, &plane)?);
    }
    for ch in &doc.channels {
        docs.push(gray_document(&format!("{}_{}", doc.name, ch.name), &doc, &read_plane(&ch.surface, area))?);
    }
    if p.get("closeOriginal").and_then(Value::as_bool).unwrap_or(true) {
        s.close(idx);
    }
    let made: Vec<usize> = docs.into_iter().map(|d| s.add_document(d, None)).collect::<Result<_>>()?;
    Ok(json!({ "documents": made }))
}

/// Open grayscale documents with the active document's size, active first.
fn mergeable(s: &Session) -> Vec<usize> {
    let Some(a) = s.active() else { return Vec::new() };
    let ok = |d: &DocState| d.doc.mode == ColorMode::Grayscale && d.doc.size == a.doc.size;
    let mut v: Vec<usize> = s.active_index().into_iter().filter(|_| ok(a)).collect();
    v.extend(s.documents().iter().enumerate().filter(|(i, d)| Some(*i) != s.active_index() && ok(d)).map(|(i, _)| i));
    v
}

/// Merge Channels: grayscale documents of one size become the channels of an RGB, CMYK or Lab
/// document (extra documents become alpha channels); the sources close.
fn merge(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(value) = merge_controlled::try_run(s, p)? { return Ok(value); }
    let cmd = "channel.merge";
    let mode = match p.get("mode").and_then(Value::as_str).unwrap_or("rgb") {
        "rgb" => ColorMode::Rgb,
        "cmyk" => ColorMode::Cmyk,
        "lab" => ColorMode::Lab,
        other => return Err(bad(cmd, format!("unknown mode `{other}`"))),
    };
    let srcs: Vec<usize> = match p.get("documents").and_then(Value::as_array) {
        Some(a) => a
            .iter()
            .map(|v| v.as_u64().map(|i| i as usize).filter(|i| *i < s.documents().len()).ok_or_else(|| bad(cmd, "bad document index")))
            .collect::<Result<_>>()?,
        None => mergeable(s).into_iter().take(mode.color_channels()).collect(),
    };
    if srcs.len() < mode.color_channels() {
        return Err(bad(cmd, format!("{mode:?} needs {} grayscale documents, got {}", mode.color_channels(), srcs.len())));
    }
    // Each source closes by its *current* index, so a repeated index would shift the later
    // removals onto a document that was never named (#934). Refuse it rather than guess.
    let mut seen = std::collections::HashSet::new();
    if let Some(dup) = srcs.iter().find(|i| !seen.insert(**i)) {
        return Err(bad(cmd, format!("document {dup} is listed more than once")));
    }
    let Some(first) = srcs.first().and_then(|&i| s.documents().get(i)).map(|d| d.doc.clone()) else {
        return Err(bad(cmd, "bad document index"));
    };
    let mut planes = Vec::new();
    for &i in &srcs {
        let d = &s.documents()[i].doc;
        if d.size != first.size || d.mode != ColorMode::Grayscale {
            return Err(bad(cmd, format!("\"{}\" is not a grayscale document of the same size", d.name)));
        }
        planes.push(gray_plane(d, None, None, ChanRef::Composite)?);
    }
    let mut out = Document::new(p.get("name").and_then(Value::as_str).unwrap_or("Untitled"), first.size, mode, first.depth);
    out.set_physical_resolution(first.effective_resolution()).map_err(|e| bad(cmd, e.to_string()))?;
    let fmt = out.pixel_format();
    let cc = mode.color_channels();
    let npx = planes[0].len();
    let mut data = Vec::with_capacity(npx * fmt.channels());
    for i in 0..npx {
        for pl in &planes[..cc] {
            data.push(pl[i].clamp(0.0, 1.0));
        }
        data.push(1.0);
    }
    let mut bg = Layer::raster("Background", fmt);
    bg.locks.transparency = true;
    bg.locks.position = true;
    if let Some(sf) = bg.surface_mut() {
        sf.write_region(out.bounds(), &data);
        sf.prune();
    }
    out.layers.push(bg);
    let area = out.bounds();
    let cf = channel_format(&out);
    for (k, pl) in planes[cc..].iter().enumerate() {
        out.channels.push(AlphaChannel::new(format!("Alpha {}", k + 1), plane_surface(pl, area, cf)));
    }
    let mut close: Vec<usize> = srcs.clone();
    close.sort_unstable();
    for i in close.into_iter().rev() {
        s.close(i);
    }
    let i = s.add_document(out, None)?;
    Ok(json!({ "document": i }))
}

// ---------------------------------------------------------------------------------------------
// Apply Image / Calculations

#[path = "channel_cmds/native_calculations.rs"]
mod native_calculations;

fn calculation_poll(ctl: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn calculation_error(error: photocraft_raster::RasterInputError) -> EngineError {
    if error == photocraft_raster::RasterInputError::Cancelled { EngineError::Cancelled } else { EngineError::Other(error.to_string()) }
}
fn calculation_shape(value: &Value, depth: usize, total: &mut usize, visits: &mut usize, cmd: &str) -> Result<()> {
    if depth > 4 {
        return Err(bad(cmd, "parameter depth limit"));
    }
    *visits = visits.checked_add(1).filter(|count| *count <= 4096).ok_or_else(|| bad(cmd, "parameter visit limit"))?;
    *total = total.checked_add(std::mem::size_of::<Value>()).ok_or_else(|| bad(cmd, "parameter size overflow"))?;
    if *total > 4 << 20 {
        return Err(bad(cmd, "parameter byte limit"));
    }
    match value {
        Value::String(text) => {
            *total = total.checked_add(text.len()).ok_or_else(|| bad(cmd, "parameter size overflow"))?;
            if text.len() > 1 << 20 || *total > 4 << 20 || text.contains('\0') {
                return Err(bad(cmd, "bounded UTF-8 without NUL required"));
            }
        }
        Value::Object(object) => {
            if object.len() > 40 {
                return Err(bad(cmd, "parameter count limit"));
            }
            for (key, value) in object {
                if key.len() > 256 {
                    return Err(bad(cmd, "parameter key limit"));
                }
                // Account every map entry and key before descending, including
                // all-numeric maps with no string-value payload at all.
                *total = total.checked_add(64).and_then(|bytes| bytes.checked_add(key.len())).ok_or_else(|| bad(cmd, "parameter size overflow"))?;
                if *total > 4 << 20 {
                    return Err(bad(cmd, "parameter byte limit"));
                }
                calculation_shape(value, depth + 1, total, visits, cmd)?;
            }
        }
        Value::Array(_) => return Err(bad(cmd, "unexpected array parameter")),
        _ => {}
    }
    Ok(())
}
fn calculation_number(p: &Value, key: &str, default: f64, cmd: &str) -> Result<f64> {
    match p.get(key) {
        None => Ok(default),
        Some(value) => value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, format!("{key} must be finite numeric"))),
    }
}
fn calculation_blend(p: &Value, cmd: &str) -> Result<(Blending, f32)> {
    if let Some(value) = p.get("blending") {
        if value.as_str().is_none_or(|v| v.len() > 256) {
            return Err(bad(cmd, "blending must be a bounded string"));
        }
    }
    calculation_number(p, "scale", 1.0, cmd)?;
    calculation_number(p, "offset", 0.0, cmd)?;
    let opacity = calculation_number(p, "opacity", 100.0, cmd)?.clamp(0.0, 100.0) as f32 / 100.0;
    Ok((Blending::parse(p).map_err(|e| bad(cmd, e))?, opacity))
}
/// Root may call this before floating normalization, job setup and params cloning.
pub(crate) fn calculation_dispatch_preflight(s: &Session, cmd: &str, p: &Value, background: bool) -> Result<()> {
    if !matches!(cmd, "image.applyImage" | "image.calculations") {
        return Err(bad(cmd, "unknown calculation command"));
    }
    if !p.is_object() && !p.is_null() {
        return Err(bad(cmd, "object or null parameters required"));
    }
    calculation_shape(p, 0, &mut 0, &mut 0, cmd)?;
    calculation_blend(p, cmd)?;
    let native = match p.get("nativeCoverage") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        _ => return Err(bad(cmd, "nativeCoverage must be boolean")),
    };
    if native && cmd != "image.calculations" {
        return Err(bad(cmd, "nativeCoverage is supported only by Calculations"));
    }
    if p.get("preserveTransparency").is_some_and(|v| !v.is_boolean()) {
        return Err(bad(cmd, "preserveTransparency must be boolean"));
    }
    if p.get("name").is_some_and(|v| !v.is_string()) {
        return Err(bad(cmd, "name must be a string"));
    }
    if p.get("result").is_some_and(|v| v.as_str().is_none_or(|v| !matches!(v, "newChannel" | "newDocument" | "selection"))) {
        return Err(bad(cmd, "unknown result"));
    }
    if let Some(target) = p.get("target") {
        match target {
            Value::String(v) if matches!(v.as_str(), "pixels" | "layer" | "mask" | "quickMask") => {}
            Value::Object(o) if o.len() == 1 && o.get("channel").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).is_some() => {}
            _ => return Err(bad(cmd, "invalid target")),
        }
    }
    if background || s.command_is_background() || s.command_has_pending_job() || s.has_jobs() || s.coalesce_request.is_some() {
        return Err(bad(cmd, "synchronous idle calculation required"));
    }
    let active = s.active().ok_or(EngineError::NoDocument)?;
    if active.floating.is_some() || active.coalesce.is_some() || active.doc.xcf_state.as_ref().is_some_and(|st| st.floating.is_some()) {
        return Err(bad(cmd, "finish floating/coalesced editing first"));
    }
    active.doc.validate_resident()?;
    if !native && matches!(active.doc.depth, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {
        return Err(bad(cmd, "wide native calculation unsupported"));
    }
    if active.revision == u64::MAX {
        return Err(bad(cmd, "revision headroom exhausted"));
    }
    if native {
        native_calculations::preflight(s, p, &photocraft_raster::Interrupt::NONE)?;
    }
    Ok(())
}
fn calculation_pixels(area: Rect) -> Result<usize> {
    photocraft_raster::checked_region_byte_len(area, PixelFormat::GRAY8).map_err(calculation_error)
}
fn calculation_buffer<T: Default + Clone>(length: usize, ctl: &photocraft_raster::Interrupt<'_>) -> Result<Vec<T>> {
    calculation_poll(ctl)?;
    let mut data = Vec::new();
    data.try_reserve_exact(length).map_err(|_| EngineError::Other("calculation allocation limit".into()))?;
    if data.capacity() != length {
        return Err(EngineError::Other("calculation capacity limit".into()));
    }
    let chunk = (32768 / std::mem::size_of::<T>().max(1)).max(1);
    while data.len() < length {
        calculation_poll(ctl)?;
        data.resize(data.len().checked_add(chunk).ok_or_else(|| EngineError::Other("calculation size overflow".into()))?.min(length), T::default());
    }
    calculation_poll(ctl)?;
    Ok(data)
}
fn calculation_admit(s: &Session, specs: &[&Value], pixels: usize, cmd: &str, ctl: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    let active = s.active_index().ok_or(EngineError::NoDocument)?;
    let mut indices = [None; 4];
    indices[0] = Some(active);
    let mut count = 1;
    for spec in specs {
        calculation_poll(ctl)?;
        let index = match spec.get("document") {
            None | Some(Value::Null) => active,
            Some(v) => {
                usize::try_from(v.as_u64().ok_or_else(|| bad(cmd, "source document must be integer"))?).map_err(|_| bad(cmd, "source document overflow"))?
            }
        };
        if !indices.iter().flatten().any(|old| *old == index) {
            *indices.get_mut(count).ok_or_else(|| bad(cmd, "source document count limit"))? = Some(index);
            count += 1;
        }
    }
    let mut resident = 0u64;
    let mut metadata = 0u64;
    for index in indices.into_iter().flatten() {
        calculation_poll(ctl)?;
        let doc = &s.documents().get(index).ok_or_else(|| bad(cmd, "missing source document"))?.doc;
        if matches!(doc.depth, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {
            return Err(bad(cmd, "wide native source unsupported"));
        }
        let footprint = crate::DocumentSnapshot::resident_raster_footprint_of(doc, ctl)?;
        resident = resident.checked_add(footprint.retained_bytes).ok_or_else(|| bad(cmd, "retained byte overflow"))?;
        metadata = metadata.checked_add(footprint.clone_metadata_bytes).ok_or_else(|| bad(cmd, "COW metadata overflow"))?;
    }
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let area = doc.bounds();
    let tiles = if area.is_empty() {
        0
    } else {
        let span = |a: i32, b: i32| {
            u64::try_from(i64::from(b.div_euclid(photocraft_geom::TILE_SIZE)) - i64::from(a.div_euclid(photocraft_geom::TILE_SIZE)) + 1)
                .map_err(|_| bad(cmd, "tile count overflow"))
        };
        span(area.x0, area.x1.checked_sub(1).ok_or_else(|| bad(cmd, "tile edge overflow"))?)?
            .checked_mul(span(area.y0, area.y1.checked_sub(1).ok_or_else(|| bad(cmd, "tile edge overflow"))?)?)
            .ok_or_else(|| bad(cmd, "tile count overflow"))?
    };
    let output = tiles
        .checked_mul(65536)
        .and_then(|n| n.checked_mul(doc.pixel_format().bytes_per_pixel().max(4) as u64))
        .and_then(|n| n.checked_add(tiles.checked_mul(512)?))
        .ok_or_else(|| bad(cmd, "output charge overflow"))?;
    // Raster/COW plus simultaneously live native planes, decoded planes and output.
    // Non-raster metadata/blob retention is not total allocator/RSS protection.
    let peak = resident
        .checked_mul(2)
        .and_then(|n| n.checked_add(metadata.checked_mul(3)?))
        .and_then(|n| n.checked_add((pixels as u64).checked_mul(64)?))
        .and_then(|n| n.checked_add(output.checked_mul(3)?))
        .and_then(|n| n.checked_add((4 << 20) + 32768))
        .ok_or_else(|| bad(cmd, "calculation peak overflow"))?;
    if peak > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(bad(cmd, "calculation raster/planes/COW limit"));
    }
    calculation_poll(ctl)
}
fn calculation_values(values: &[f32], pixels: usize, unit: bool, cmd: &str, ctl: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    if values.len() != pixels {
        return Err(bad(cmd, "plane length mismatch"));
    }
    for chunk in values.chunks(4096) {
        calculation_poll(ctl)?;
        if chunk.iter().any(|v| !v.is_finite() || unit && !(0.0..=1.0).contains(v)) {
            return Err(bad(cmd, "nonfinite or invalid coverage sample"));
        }
    }
    Ok(())
}
fn calculation_planes(planes: &[Vec<f32>], pixels: usize, cmd: &str, ctl: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    if planes.is_empty() || planes.len() > 4 {
        return Err(bad(cmd, "invalid source plane count"));
    }
    for plane in planes {
        calculation_values(plane, pixels, false, cmd, ctl)?;
    }
    Ok(())
}
fn calculation_read(surface: &Surface, area: Rect, ctl: &photocraft_raster::Interrupt<'_>) -> Result<Vec<f32>> {
    let format = surface.format();
    if matches!(format.sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {
        return Err(EngineError::Other("wide target unsupported".into()));
    }
    let length = photocraft_raster::checked_region_byte_len(area, format).map_err(calculation_error)?;
    let mut raw = calculation_buffer::<u8>(length, ctl)?;
    surface.try_read_interleaved_into_with(area, &mut raw, ctl).map_err(calculation_error)?;
    let mut values = calculation_buffer::<f32>(
        calculation_pixels(area)?.checked_mul(format.channels()).ok_or_else(|| EngineError::Other("sample count overflow".into()))?,
        ctl,
    )?;
    for (index, target) in values.iter_mut().enumerate() {
        if index % 4096 == 0 {
            calculation_poll(ctl)?;
        }
        photocraft_color::sample_byte_range(format.sample, index, raw.len()).map_err(|e| EngineError::Other(e.to_string()))?;
        *target = photocraft_color::read_sample(&raw, format.sample, index);
        if !target.is_finite() {
            return Err(EngineError::Other("nonfinite target sample".into()));
        }
    }
    Ok(values)
}
fn calculation_write(surface: &mut Surface, area: Rect, values: &[f32], ctl: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    let format = surface.format();
    if matches!(format.sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {
        return Err(EngineError::Other("wide target unsupported".into()));
    }
    let length = photocraft_raster::checked_region_byte_len(area, format).map_err(calculation_error)?;
    if values.len() != calculation_pixels(area)?.checked_mul(format.channels()).ok_or_else(|| EngineError::Other("output count overflow".into()))? {
        return Err(EngineError::Other("output length mismatch".into()));
    }
    let mut bytes = calculation_buffer::<u8>(length, ctl)?;
    for (index, &value) in values.iter().enumerate() {
        if index % 4096 == 0 {
            calculation_poll(ctl)?;
        }
        if !value.is_finite() {
            return Err(EngineError::Other("nonfinite output".into()));
        }
        photocraft_color::sample_byte_range(format.sample, index, bytes.len()).map_err(|e| EngineError::Other(e.to_string()))?;
        photocraft_color::write_sample(&mut bytes, format.sample, index, value);
    }
    surface.try_write_interleaved_with(area, &bytes, ctl).map_err(calculation_error)?;
    surface.try_prune_with(ctl).map_err(calculation_error)?;
    calculation_poll(ctl)
}
fn calculation_gray(values: &[f32], area: Rect, format: PixelFormat, ctl: &photocraft_raster::Interrupt<'_>) -> Result<Surface> {
    if format.channels() != 1 {
        return Err(EngineError::Other("one native target plane required".into()));
    }
    let mut out = calculation_buffer::<f32>(values.len(), ctl)?;
    for (index, (target, &value)) in out.iter_mut().zip(values).enumerate() {
        if index % 4096 == 0 {
            calculation_poll(ctl)?;
        }
        if !value.is_finite() {
            return Err(EngineError::Other("nonfinite output".into()));
        }
        *target = value.clamp(0.0, 1.0);
    }
    let mut surface = Surface::new(format);
    calculation_write(&mut surface, area, &out, ctl)?;
    Ok(surface)
}
fn calculation_weight(index: usize, opacity: f32, mask: Option<&[f32]>, selection: Option<&[f32]>) -> Result<f32> {
    let at = |p: Option<&[f32]>| -> Result<f32> {
        match p {
            None => Ok(1.0),
            Some(values) => values.get(index).copied().ok_or_else(|| EngineError::Other("coverage length mismatch".into())),
        }
    };
    let weight = opacity * at(mask)? * at(selection)?;
    if !weight.is_finite() || !(0.0..=1.0).contains(&weight) {
        return Err(EngineError::Other("invalid blend weight".into()));
    }
    Ok(weight)
}
fn calculation_mask(s: &Session, p: &Value, pixels: usize, cmd: &str, ctl: &photocraft_raster::Interrupt<'_>) -> Result<Option<Vec<f32>>> {
    let mask = mask_plane_with_interrupt(s, p, cmd, ctl)?;
    if let Some(values) = &mask {
        calculation_values(values, pixels, true, cmd, ctl)?;
    }
    Ok(mask)
}
fn apply_image(s: &mut Session, p: &Value) -> Result<Value> {
    apply_image_with_interrupt(s, p, &photocraft_raster::Interrupt::NONE)
}
pub fn apply_image_with_interrupt(s: &mut Session, p: &Value, ctl: &photocraft_raster::Interrupt<'_>) -> Result<Value> {
    const CMD: &str = "image.applyImage";
    calculation_dispatch_preflight(s, CMD, p, false)?;
    calculation_poll(ctl)?;
    let (blending, opacity) = calculation_blend(p, CMD)?;
    let preserve = p.get("preserveTransparency").and_then(Value::as_bool).unwrap_or(false);
    let spec = source_spec(p, "source", "source").unwrap_or_else(|| json!({}));
    let maskspec = source_spec(p, "mask", "mask").unwrap_or_else(|| json!({}));
    let area = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
    let pixels = calculation_pixels(area)?;
    if maskspec.get("channel").is_some_and(|channel| channel != "none") {
        calculation_admit(s, &[&spec, &maskspec], pixels, CMD, ctl)?;
    } else {
        calculation_admit(s, &[&spec], pixels, CMD, ctl)?;
    }
    let source = source_with_interrupt(s, &spec, CMD, false, ctl)?;
    calculation_planes(&source, pixels, CMD, ctl)?;
    let mask = calculation_mask(s, p, pixels, CMD, ctl)?;
    let layer = s.active().and_then(|d| d.active_layer);
    s.edit_with_interrupt("Apply Image", ctl, |doc, _| {
        let selection = match doc.selection.as_ref() {
            None => None,
            Some(surface) => {
                let mut values = read_plane_with_interrupt(surface, area, ctl)?;
                calculation_values(&values, pixels, false, CMD, ctl)?;
                for chunk in values.chunks_mut(4096) {
                    calculation_poll(ctl)?;
                    for v in chunk {
                        *v = v.clamp(0.0, 1.0);
                    }
                }
                Some(values)
            }
        };
        if is_channel_target(p) {
            let mut srcp = calculation_buffer::<f32>(pixels, ctl)?;
            for (index, target) in srcp.iter_mut().enumerate() {
                if index % 4096 == 0 {
                    calculation_poll(ctl)?;
                }
                if source.len() == 1 {
                    *target = *source.first().and_then(|plane| plane.get(index)).ok_or_else(|| bad(CMD, "source length mismatch"))?;
                } else {
                    let mut sum = 0f32;
                    for plane in &source {
                        sum += *plane.get(index).ok_or_else(|| bad(CMD, "source length mismatch"))?;
                    }
                    *target = sum / source.len() as f32;
                }
                if !target.is_finite() {
                    return Err(bad(CMD, "nonfinite source mean"));
                }
            }
            let (surface, _) = target_surface(doc, None, p)?;
            let base = read_plane_with_interrupt(surface, area, ctl)?;
            calculation_values(&base, pixels, false, CMD, ctl)?;
            let mut out = calculation_buffer::<f32>(pixels, ctl)?;
            for (index, (&base, target)) in base.iter().zip(&mut out).enumerate() {
                calculation_poll(ctl)?;
                let top = *srcp.get(index).ok_or_else(|| bad(CMD, "source length mismatch"))?;
                *target = base + (blending.apply(base, top) - base) * calculation_weight(index, opacity, mask.as_deref(), selection.as_deref())?;
                if !target.is_finite() {
                    return Err(bad(CMD, "nonfinite blend output"));
                }
            }
            let replacement = calculation_gray(&out, area, surface.format(), ctl)?;
            calculation_poll(ctl)?;
            *surface = replacement;
            return Ok(());
        }
        let id = layer.ok_or_else(|| bad(CMD, "no active layer"))?;
        let format = doc.pixel_format();
        let surface = crate::commands::paint_surface(doc, id, &Value::Null)?;
        if surface.format() != format {
            return Err(bad(CMD, "target format mismatch"));
        }
        let channels = format.channels();
        let colors = format.mode.color_channels();
        let mut values = calculation_read(surface, area, ctl)?;
        for (index, pixel) in values.chunks_exact_mut(channels).enumerate() {
            calculation_poll(ctl)?;
            let base_alpha = if format.alpha { pixel.get(colors).copied().ok_or_else(|| bad(CMD, "alpha length mismatch"))?.clamp(0.0, 1.0) } else { 1.0 };
            let mut weight = calculation_weight(index, opacity, mask.as_deref(), selection.as_deref())?;
            if preserve {
                weight *= base_alpha;
            }
            if weight <= 0.0 {
                continue;
            }
            let output_alpha = if preserve { base_alpha } else { weight + base_alpha * (1.0 - weight) };
            if !output_alpha.is_finite() || !preserve && output_alpha <= 0.0 {
                return Err(bad(CMD, "invalid alpha divisor"));
            }
            for channel in 0..colors {
                let plane = source.get(channel).or_else(|| source.first()).ok_or_else(|| bad(CMD, "missing source plane"))?;
                let top = *plane.get(index).ok_or_else(|| bad(CMD, "source length mismatch"))?;
                let base = pixel.get_mut(channel).ok_or_else(|| bad(CMD, "target length mismatch"))?;
                let blended = top + (blending.apply(*base, top) - top) * base_alpha;
                *base = if preserve { *base + (blended - *base) * weight } else { (weight * blended + base_alpha * (1.0 - weight) * *base) / output_alpha };
                if !base.is_finite() {
                    return Err(bad(CMD, "nonfinite blend output"));
                }
            }
            if format.alpha {
                *pixel.get_mut(colors).ok_or_else(|| bad(CMD, "alpha length mismatch"))? = output_alpha;
            }
        }
        calculation_write(surface, area, &values, ctl)
    })?;
    Ok(Value::Null)
}
fn calculations(s: &mut Session, p: &Value) -> Result<Value> {
    calculations_with_interrupt(s, p, &photocraft_raster::Interrupt::NONE)
}
pub fn calculations_with_interrupt(s: &mut Session, p: &Value, ctl: &photocraft_raster::Interrupt<'_>) -> Result<Value> {
    const CMD: &str = "image.calculations";
    calculation_poll(ctl)?;
    calculation_dispatch_preflight(s, CMD, p, false)?;
    if p.get("nativeCoverage").and_then(Value::as_bool) == Some(true) {
        return native_calculations::run(s, p, ctl);
    }
    calculation_poll(ctl)?;
    let (blending, opacity) = calculation_blend(p, CMD)?;
    let spec1 = source_spec(p, "source1", "source1").unwrap_or_else(|| json!({}));
    let spec2 = source_spec(p, "source2", "source2").unwrap_or_else(|| json!({}));
    let maskspec = source_spec(p, "mask", "mask").unwrap_or_else(|| json!({}));
    let area = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
    let pixels = calculation_pixels(area)?;
    if maskspec.get("channel").is_some_and(|channel| channel != "none") {
        calculation_admit(s, &[&spec1, &spec2, &maskspec], pixels, CMD, ctl)?;
    } else {
        calculation_admit(s, &[&spec1, &spec2], pixels, CMD, ctl)?;
    }
    let first = source_with_interrupt(s, &spec1, CMD, true, ctl)?;
    let second = source_with_interrupt(s, &spec2, CMD, true, ctl)?;
    calculation_planes(&first, pixels, CMD, ctl)?;
    calculation_planes(&second, pixels, CMD, ctl)?;
    if first.len() != 1 || second.len() != 1 {
        return Err(bad(CMD, "one plane per calculation source required"));
    }
    let first = first.first().ok_or_else(|| bad(CMD, "missing first source"))?;
    let second = second.first().ok_or_else(|| bad(CMD, "missing second source"))?;
    let mask = calculation_mask(s, p, pixels, CMD, ctl)?;
    let mut out = calculation_buffer::<f32>(pixels, ctl)?;
    for (index, ((&base, &top), target)) in second.iter().zip(first).zip(&mut out).enumerate() {
        calculation_poll(ctl)?;
        calculation_weight(index, opacity, mask.as_deref(), None)?;
        let coverage = match mask.as_deref() {
            None => 1.0,
            Some(values) => *values.get(index).ok_or_else(|| bad(CMD, "mask length mismatch"))?,
        };
        *target = base + (blending.apply(base, top) - base) * opacity * coverage;
        if !target.is_finite() {
            return Err(bad(CMD, "nonfinite calculation output"));
        }
    }
    let name = match p.get("name").and_then(Value::as_str) {
        None => None,
        Some(text) => {
            let mut owned = String::new();
            owned.try_reserve_exact(text.len()).map_err(|_| bad(CMD, "name allocation limit"))?;
            owned.push_str(text);
            Some(owned)
        }
    };
    match p.get("result").and_then(Value::as_str).unwrap_or("newChannel") {
        "newChannel" => {
            let index = s.edit_with_interrupt("Calculations", ctl, |doc, _| {
                let name = name.unwrap_or_else(|| next_alpha_name(doc, "Alpha"));
                let surface = calculation_gray(&out, area, channel_format(doc), ctl)?;
                doc.channels.try_reserve(1).map_err(|_| bad(CMD, "channel allocation limit"))?;
                calculation_poll(ctl)?;
                doc.channels.push(AlphaChannel::new(name, surface));
                Ok(doc.channels.len() - 1)
            })?;
            Ok(json!({"channel":index}))
        }
        "selection" => {
            let selected = s.edit_with_interrupt("Calculations", ctl, |doc, _| {
                let mut bytes = calculation_buffer::<u8>(pixels, ctl)?;
                let mut nonempty = false;
                for (index, (byte, &value)) in bytes.iter_mut().zip(&out).enumerate() {
                    if index % 4096 == 0 {
                        calculation_poll(ctl)?;
                    }
                    *byte = (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    nonempty |= *byte != 0;
                }
                let selection = if !nonempty {
                    None
                } else {
                    let mut surface = Surface::new(PixelFormat::GRAY8);
                    surface.try_write_interleaved_with(area, &bytes, ctl).map_err(calculation_error)?;
                    surface.try_prune_with(ctl).map_err(calculation_error)?;
                    Some(surface)
                };
                calculation_poll(ctl)?;
                doc.selection = selection;
                Ok(doc.selection.is_some())
            })?;
            Ok(json!({"selected":selected}))
        }
        "newDocument" => {
            let like = &s.active().ok_or(EngineError::NoDocument)?.doc;
            let mut doc = Document::new(name.as_deref().unwrap_or("Untitled"), like.size, ColorMode::Grayscale, like.depth);
            doc.set_physical_resolution(like.effective_resolution()).map_err(|e| bad(CMD, e.to_string()))?;
            let format = doc.pixel_format();
            let mut data = calculation_buffer::<f32>(pixels.checked_mul(format.channels()).ok_or_else(|| bad(CMD, "gray output count overflow"))?, ctl)?;
            for (index, pixel) in data.chunks_exact_mut(format.channels()).enumerate() {
                calculation_poll(ctl)?;
                *pixel.first_mut().ok_or_else(|| bad(CMD, "missing gray sample"))? =
                    out.get(index).copied().ok_or_else(|| bad(CMD, "output length mismatch"))?.clamp(0.0, 1.0);
                if format.alpha {
                    *pixel.get_mut(1).ok_or_else(|| bad(CMD, "missing alpha sample"))? = 1.0;
                }
            }
            let mut background = Layer::raster("Background", format);
            background.locks.transparency = true;
            background.locks.position = true;
            calculation_write(background.surface_mut().ok_or_else(|| bad(CMD, "no background raster"))?, area, &data, ctl)?;
            doc.layers.try_reserve(1).map_err(|_| bad(CMD, "layer allocation limit"))?;
            doc.layers.push(background);
            calculation_poll(ctl)?;
            let index = s.add_document_with_interrupt(doc, None, ctl)?;
            Ok(json!({"document":index}))
        }
        _ => Err(bad(CMD, "unknown result")),
    }
}
#[cfg(test)]
#[path = "channel_cmds/xcf_calculation_safety_tests.rs"]
mod xcf_calculation_safety_tests;
// ---------------------------------------------------------------------------------------------
// State as JSON

/// The Channels panel rows and view state of a document (also part of `inspect`).
pub fn channels_json(st: &DocState) -> Value {
    let doc = &st.doc;
    let mode = edit_mode(doc);
    let v = &st.channel_view;
    let colors = color_count(doc);
    json!({
        "composite": composite_name(mode),
        "colors": color_names(mode).iter().enumerate().map(|(k, n)| json!({ "name": n, "visible": v.color_visible(k) })).collect::<Vec<_>>(),
        "alpha": doc.channels.iter().enumerate().map(|(i, c)| json!({
            "index": i,
            "channelId": c.id.0,
            "channelIdString": c.id.0.to_string(),
            "name": c.name,
            "visible": v.alpha_shown(i),
            "spot": c.spot.map(|(col, sol)| json!({ "color": col.to_rgb(), "solidity": sol })),
            "color": c.color.to_rgb(),
            "opacity": c.opacity,
            "indicates": c.indicates,
        })).collect::<Vec<_>>(),
        "quickMask": doc.quick_mask.as_ref().map(|q| json!({ "visible": !v.quick_mask_hidden, "color": q.color.to_rgb(), "opacity": q.opacity, "indicates": q.indicates })),
        "target": v.target,
        "compositeVisible": v.visible_colors(colors) == colors,
        "layerMaskView": crate::mask_view_cmds::view_json(st),
    })
}

fn list(s: &Session) -> Value {
    s.active().map(channels_json).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------------------------
// Registry

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "select.saveSelection",
            "Save Selection…",
            ["Select"],
            None,
            r##"{"name":str?,"channel":"new"|index|name="new","document":index?,"operation":"new|replace|add|subtract|intersect"="new","rawStorage":bool=false (same-depth Gray new/replace; preserve default and all off-canvas raw tiles, no arithmetic/source-carrier contexts),"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_selection,
            save_selection
        ),
        spec!(
            "select.loadSelection",
            "Load Selection…",
            ["Select"],
            None,
            r##"{"channel":index|name|"composite"|"red|green|blue|…"|"transparency"|"mask"|"vectorMask"|"quickMask"|"selection"|"targeted" (the Channels panel's target),"layer":id?,"document":index?,"invert":bool=false,"operation":"new|add|subtract|intersect"="new","rawStorage":bool=false (plain same-depth saved Gray channel; replace without invert; preserve all raw storage),"storedSource":bool=false (same-document stored read; requires fresh host-held original binding; no filtered appearance),"nativeCoverage":bool=false,"coverageContext":"unit"?}"##,
            has_doc,
            load_selection
        ),
        spec!(
            "select.editInQuickMaskMode",
            "Edit in Quick Mask Mode",
            ["Select"],
            Some("Q"),
            r##"{"on":bool?=toggle,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_doc,
            quick_mask
        ),
        spec!(
            "image.applyImage",
            "Apply Image…",
            ["Image"],
            None,
            r##"{"source":{"document":index?,"layer":id|name|"merged"="merged","channel":"composite"|"red|…"|index|"selection"|"transparency"|"mask"="composite","invert":bool=false},"blending":"normal|multiply|screen|overlay|softLight|hardLight|colorDodge|colorBurn|darken|lighten|difference|exclusion|linearBurn|linearDodge|add|subtract|…"="multiply","opacity":0..100=100,"scale":1..2=1,"offset":-255..255=0,"preserveTransparency":bool=false,"mask":{"document","layer","channel","invert"}?,"sourceChannel|sourceDocument|sourceLayer|sourceInvert|maskChannel|…":flat form of source/mask?}"##,
            has_apply_target,
            apply_image
        ),
        spec!(
            "image.calculations",
            "Calculations…",
            ["Image"],
            None,
            r##"{"source1":{"document","layer","channel","channelId":canonical-decimal-str?,"invert"},"source2":{…},"blending":"multiply|…"="multiply","opacity":0..100=100,"scale":1..2=1,"offset":-255..255=0,"mask":{…}?,"result":"newChannel|newDocument|selection"="newChannel","name":str?,"nativeCoverage":bool=false,"coverageContext":"unit"?,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_doc,
            calculations
        ),
        spec!(
            "channel.new",
            "New Channel…",
            [],
            None,
            r##"{"name":str?,"fill":"black|white|selection"="black","rawStorage":bool=false (fill selection only; identical native Gray depth; preserve default/all off-canvas raw tiles without arithmetic),"color":"#rrggbb"="#ff0000","opacity":0..100=50,"indicates":"masked|selected"="masked","documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_doc,
            new_channel
        ),
        spec!(
            "channel.newSpot",
            "New Spot Channel…",
            [],
            None,
            r##"{"name":str?,"color":"#rrggbb"="#ff0000","solidity":0..100=0,"fromSelection":bool=true,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_doc,
            new_spot
        ),
        spec!(
            "channel.duplicate",
            "Duplicate Channel…",
            [],
            None,
            r##"{"channel":index|name|{channelId:id}|"red|…"|"composite"|"quickMask","name":str?,"document":index|"new"?=active,"invert":bool=false,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_doc,
            duplicate_channel
        ),
        spec!(
            "channel.delete",
            "Delete Channel",
            [],
            None,
            r##"{"channel":index|name|{channelId:id}?=targeted,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_channels,
            delete_channel
        ),
        spec!(
            "channel.rename",
            "Rename Channel",
            [],
            None,
            r##"{"channel":index|name|{channelId:id},"name":str,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_channels,
            rename_channel
        ),
        spec!(
            "channel.move",
            "Reorder Channel",
            [],
            None,
            r##"{"channel":index|name|{channelId:id},"to":index,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_channels,
            move_channel
        ),
        spec!(
            "channel.target",
            "Target Channel",
            [],
            None,
            r##"{"channel":"composite"|"red|green|blue|…"|index|name="composite","documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_doc,
            target_cmd
        ),
        spec!(
            "channel.setVisible",
            "Channel Visibility",
            [],
            None,
            r##"{"channel":"composite"|"red|…"|index|name|"quickMask","visible":bool?=toggle,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_doc,
            set_visible
        ),
        spec!(
            "channel.options",
            "Channel Options…",
            [],
            None,
            r##"{"channel":index|name|{channelId:id}|"quickMask","name":str?,"indicates":"masked|selected|spot"?,"color":"#rrggbb"?,"opacity":0..100?,"solidity":0..100?,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?}"##,
            has_doc,
            options
        ),
        spec!("channel.mergeSpot", "Merge Spot Channel", [], None, r##"{"channel":index|name|{channelId:id}?=targeted or first spot,"documentId":decimal-string?,"expectedRevision":decimal-string? (captured pair)}"##, has_spot, merge_spot),
        spec!("channel.split", "Split Channels", [], None, r##"{"closeOriginal":bool=true,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?} (native routes validate the captured pair)"##, can_split, split),
        spec!(
            "channel.merge",
            "Merge Channels…",
            [],
            None,
            r##"{"mode":"rgb|cmyk|lab"="rgb","documents":[index,…]?=open grayscale docs of the active size,"name":str?,"documentId":canonical-decimal-str?,"expectedRevision":canonical-decimal-str?} (native routes validate the captured pair)"##,
            can_merge,
            merge
        ),
        CommandSpec {
            id: "channel.list",
            label: "Channels",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: has_doc,
            run: |s, _| Ok(list(s)),
            journal: false,
        },
        spec!("channel.target.composite", "Target Composite Channel", [], Some("Cmd+2"), "{}", has_doc, |s, _| target_slot(s, 2)),
        // ⌘\ targets the active layer's mask, as clicking its thumbnail does.
        spec!("channel.target.layerMask", "Target Layer Mask", [], Some("Cmd+\\"), "{}", has_layer_mask, |s, _| target_cmd(
            s,
            &serde_json::json!({"channel": "mask"})
        )),
        // ⌘⌥2 loads the image's luminosity as a selection (⌘-click on the composite channel).
        spec!("select.loadLuminosity", "Load Luminosity", [], Some("Cmd+Alt+2"), "{}", has_doc, |s, _| load_selection(
            s,
            &serde_json::json!({"channel": "composite"})
        )),
        spec!("channel.target.slot3", "Target Channel 3", [], Some("Cmd+3"), "{}", slot_ok::<3>, |s, _| target_slot(s, 3)),
        spec!("channel.target.slot4", "Target Channel 4", [], Some("Cmd+4"), "{}", slot_ok::<4>, |s, _| target_slot(s, 4)),
        spec!("channel.target.slot5", "Target Channel 5", [], Some("Cmd+5"), "{}", slot_ok::<5>, |s, _| target_slot(s, 5)),
        spec!("channel.target.slot6", "Target Channel 6", [], Some("Cmd+6"), "{}", slot_ok::<6>, |s, _| target_slot(s, 6)),
        spec!("channel.target.slot7", "Target Channel 7", [], Some("Cmd+7"), "{}", slot_ok::<7>, |s, _| target_slot(s, 7)),
        spec!("channel.target.slot8", "Target Channel 8", [], Some("Cmd+8"), "{}", slot_ok::<8>, |s, _| target_slot(s, 8)),
        spec!("channel.target.slot9", "Target Channel 9", [], Some("Cmd+9"), "{}", slot_ok::<9>, |s, _| target_slot(s, 9)),
    ]
}

/// Target Layer Mask: the active layer has a mask.
fn has_layer_mask(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    let layer = st.active_layer.and_then(|id| st.doc.layer(id)).ok_or("no active layer")?;
    layer.mask.as_ref().map(|_| ()).ok_or_else(|| "the active layer has no layer mask".to_string())
}

fn slot_ok<const N: usize>(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let colors = color_count(&d.doc);
    let shown = if colors > 1 { colors } else { 0 };
    if N - 3 < shown + d.doc.channels.len() { Ok(()) } else { Err(format!("no channel for {SLOT_KEY}{N}")) }
}

fn has_apply_target(s: &Session) -> std::result::Result<(), String> {
    if edits_channel(s) {
        return Ok(());
    }
    let d = s.active().ok_or("no document open")?;
    let l = d.active_layer.and_then(|id| d.doc.layer(id)).ok_or("no active layer")?;
    if matches!(l.content, LayerContent::Raster(_)) {
        Ok(())
    } else {
        Err(format!("Apply Image needs a pixel layer (active layer is {} {} layer)", l.content.article(), l.content.kind_name()))
    }
}

#[cfg(test)]
mod physical_axes_tests {
    use super::*;
    #[test]
    fn split_then_merge_preserves_custom_axes() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":2,"height":1})).unwrap();
        let resolution = photocraft_doc::PhysicalResolution {
            x_dpi: 144.0,
            y_dpi: 288.0,
            unit: photocraft_doc::PhysicalUnit::Custom(photocraft_doc::CustomUnit {
                name: "Own unit".into(),
                factor: 2.5,
                digits: 3,
                symbol: "u".into(),
                abbreviation: "ou".into(),
            }),
        };
        s.active_mut().unwrap().doc.try_resident_mut().unwrap().set_physical_resolution(resolution.clone()).unwrap();
        s.execute("channel.split", json!({})).unwrap();
        for d in s.documents() {
            assert_eq!(d.doc.effective_resolution(), resolution);
        }
        s.set_active(0);
        s.execute("channel.merge", json!({"mode":"rgb","documents":[0,1,2]})).unwrap();
        assert_eq!(s.active().unwrap().doc.effective_resolution(), resolution);
    }
    #[test]
    fn extracted_gray_preserves_custom_axes_through_native_save() {
        let mut d = Document::new("axes", photocraft_doc::Size::new(2, 1), ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let resolution = photocraft_doc::PhysicalResolution {
            x_dpi: 144.0,
            y_dpi: 288.0,
            unit: photocraft_doc::PhysicalUnit::Custom(photocraft_doc::CustomUnit {
                name: "Own unit \u{03a9}".into(),
                factor: 2.5,
                digits: 3,
                symbol: "u".into(),
                abbreviation: "ou".into(),
            }),
        };
        d.set_physical_resolution(resolution.clone()).unwrap();
        let extracted = gray_document("gray", &d, &[0.25, 0.75]).unwrap();
        assert_eq!(extracted.effective_resolution(), resolution);
        let bytes = photocraft_format::save_to_bytes(&extracted, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap().effective_resolution(), resolution);
    }
}

#[cfg(test)]
mod canonical_channel_identity_tests {
    use super::*;

    fn document() -> Document {
        let mut doc = Document::new("channel identities", photocraft_doc::Size::new(1, 1), ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let format = channel_format(&doc);
        for id in [9_007_199_254_740_993, 9_007_199_254_740_994] {
            let mut channel = AlphaChannel::new("same name", Surface::new(format));
            channel.id = photocraft_doc::item_identity::ChannelId(id);
            doc.channels.push(channel);
        }
        doc
    }

    #[test]
    fn canonical_strings_select_exact_identity_above_json_safe_integer() {
        let doc = document();
        assert_eq!(parse_ref(&json!({"channelId":"9007199254740994"}), &doc), Some(ChanRef::Alpha(1)));
        assert_eq!(parse_ref(&json!({"channelId":9_007_199_254_740_994_u64}), &doc), Some(ChanRef::Alpha(1)));
        assert_eq!(parse_ref(&json!(0), &doc), Some(ChanRef::Alpha(0)));
        assert_eq!(parse_ref(&json!("same name"), &doc), Some(ChanRef::Alpha(0)));
    }

    #[test]
    fn malformed_identity_never_falls_back_to_alpha_index_or_name() {
        let doc = document();
        for value in [
            json!(""),
            json!("0"),
            json!("01"),
            json!("+1"),
            json!(" 1"),
            json!("1.0"),
            json!("１"),
            json!("18446744073709551616"),
            json!("100000000000000000000"),
            json!(true),
            json!([]),
            json!({}),
            json!(null),
        ] {
            assert_eq!(parse_ref(&json!({"channelId":value,"alpha":0,"color":0}), &doc), None);
        }
    }

    #[test]
    fn inspection_retains_numeric_id_and_adds_exact_string_for_targeting() {
        let mut session = Session::new();
        session.add_document(document(), None).unwrap();
        let state = session.active().unwrap();
        let view = channels_json(state);
        assert_eq!(view["alpha"][1]["channelId"], json!(9_007_199_254_740_994_u64));
        assert_eq!(view["alpha"][1]["channelIdString"], "9007199254740994");
        let source = state.doc.clone();
        let revision = state.revision;
        session.execute("channel.target", json!({"channelId":"9007199254740994"})).unwrap();
        assert_eq!(session.active().unwrap().channel_view.target, ChannelTarget::Alpha(1));
        assert_eq!(session.active().unwrap().doc, source);
        assert_eq!(session.active().unwrap().revision, revision);
        session.execute("channel.target", json!({"channelId":"01","channel":0})).unwrap_err();
        assert_eq!(session.active().unwrap().channel_view.target, ChannelTarget::Alpha(1));
        assert_eq!(session.active().unwrap().doc, source);
    }
}

#[cfg(test)]
fn unselected_surface(area: Rect, fmt: PixelFormat, indicates: ColorIndicates) -> Surface {
    let mut s = Surface::new(fmt);
    if indicates != ColorIndicates::SelectedAreas {
        s.fill_rect(area, &[1.0]);
    }
    s
}

#[cfg(test)]
fn uniform(vals: &[f32], target: f32) -> bool {
    vals.iter().all(|v| (v - target).abs() <= 0.5 / 255.0)
}
