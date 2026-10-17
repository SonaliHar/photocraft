//! Bounded foreign-format loss diagnostics for canonical XCF-specific state.
//! No resource resolution, payload reads, full serialization or format guesses.
//! PSD maps ordinary names, path geometry, channel samples, DPI axes and native
//! layer locks/mask enabled+linked flags; those are not blanket loss warnings.
use photocraft_doc::{Document, LayerContent, PhysicalUnit, item_identity::ItemRef, xcf_items::EditableSource};

const ROWS: usize = 256;
const VISITS: usize = 10_000;
fn label(name: &str) -> String {
    let mut characters = name.chars();
    let mut value: String = characters.by_ref().take(128).collect();
    if characters.next().is_some() {
        value.push('…');
    }
    value
}
fn item(reference: ItemRef) -> String {
    let kind = match reference {
        ItemRef::Layer(_) => "Layer",
        ItemRef::Channel(_) => "Channel",
        ItemRef::Mask(_) => "Mask",
        ItemRef::Path(_) => "Path",
    };
    format!("{kind} {}", reference.raw())
}
struct Warnings {
    rows: Vec<String>,
    visits: usize,
    omitted: bool,
}
impl Warnings {
    fn new() -> Self {
        Self { rows: Vec::new(), visits: 0, omitted: false }
    }
    fn visit(&mut self) -> bool {
        if self.visits >= VISITS {
            self.omitted = true;
            return false;
        }
        self.visits += 1;
        true
    }
    fn push(&mut self, row: impl FnOnce() -> String) -> bool {
        // Reserve a final row for an explicit omission summary, even at overflow.
        if self.rows.len() >= ROWS - 1 {
            self.omitted = true;
            return false;
        }
        self.rows.push(row());
        true
    }
    fn finish(mut self) -> Vec<String> {
        if self.omitted {
            self.rows.push("Additional XCF export-loss diagnostics were omitted because the bounded row or traversal limit was reached; save as .pcraft to retain canonical state".into());
        }
        self.rows
    }
}

// Mirror the existing DisplayInfo RGB16/whole-percent wire quantization only.
// No profile conversion is performed merely to produce a diagnostic.
fn display_component_exact(value: f32, scale: f32) -> bool {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return false;
    }
    let wire = (value * scale).round() as u16;
    (f32::from(wire) / scale).to_bits() == value.to_bits()
}
fn display_color_exact(color: &photocraft_doc::Color) -> bool {
    color.mode == photocraft_doc::ColorMode::Rgb
        && color.alpha.to_bits() == 1.0f32.to_bits()
        && color.c.get(3).is_some_and(|v| v.to_bits() == 0.0f32.to_bits())
        && color.c.iter().take(3).all(|v| display_component_exact(*v, 65535.0))
}
fn same_color_bits(a: &photocraft_doc::Color, b: &photocraft_doc::Color) -> bool {
    a.mode == b.mode && a.alpha.to_bits() == b.alpha.to_bits() && a.c.iter().zip(&b.c).all(|(a, b)| a.to_bits() == b.to_bits())
}

// Flat codecs receive a rendered Image plus image metadata, not these editable
// Document objects. Inspect metadata presence only; never sample or decode it.
fn flat_objects(document: &Document, out: &mut Warnings) {
    for path in &document.paths {
        if !out.visit() {
            break;
        }
        if !out.push(|| {
            format!("Saved path {} {:?}: flat raster export does not retain editable path geometry, name or path ordering", path.id.0, label(&path.name))
        }) {
            break;
        }
    }
    if document.work_path.is_some() {
        out.push(|| "Work path: flat raster export does not retain editable work-path geometry".into());
    }
    if document.clipping_path.is_some() {
        out.push(|| "Clipping path: flat raster export does not retain the document's named clipping-path binding and flatness setting".into());
    }
    for channel in &document.channels {
        if !out.visit() {
            break;
        }
        let kind = if channel.spot.is_some() { "spot" } else { "alpha" };
        if !out.push(|| format!("Channel {} {:?}: flat raster export does not retain the independent {kind} channel, coverage samples, name or overlay/ink options; image alpha is a separate rendered output", channel.id.0, label(&channel.name))) { break; }
    }
    if document.selection.is_some() || document.selection_channel_id.is_some() {
        out.push(|| "Selection: flat raster export does not retain the active selection coverage surface or its stable channel identity".into());
    }
    if !document.guides.horizontal.is_empty() || !document.guides.vertical.is_empty() {
        out.push(|| "Guides: flat raster export does not retain editable horizontal/vertical guide positions".into());
    }
}

/// The actual serializer lane, not merely a file extension. A TIFF carrying
/// Photoshop layer data is Layered; an ordinary TIFF raster is Flat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Destination {
    Native,
    Layered,
    Flat,
}

/// Compatibility entry for the established PSD/layered diagnostics.
pub(crate) fn warnings(document: &Document) -> Vec<String> {
    warnings_for(document, Destination::Layered)
}

// Flat TIFF reconstructs exactly this conventional layer metadata when the actual
// encoded Image has no alpha. Runtime LayerId is freshly minted on foreign import.
fn canonical_background(document: &Document) -> bool {
    if document.xcf_state.is_some() || document.source_color_context.is_some() || document.sample_encoding.is_some() {
        return false;
    }
    let [layer] = document.layers.as_slice() else {
        return false;
    };
    let photocraft_doc::Layer {
        indexed_source,
        import_compositing,
        id: _,
        name,
        visible,
        locks,
        blend,
        opacity,
        fill_opacity,
        clipped,
        mask,
        vector_mask,
        effects,
        label,
        content,
        psd_blocks,
        psd_id,
        fill_cache,
        link_group,
        excluded_channels,
        blend_if,
        advanced,
        video,
    } = layer;
    let photocraft_doc::Effects { enabled, items, psd_raw, reference } = effects;
    indexed_source.is_none()
        && import_compositing.is_none()
        && name == "Background"
        && *visible
        && *locks == photocraft_doc::Locks { transparency: true, position: true, ..Default::default() }
        && *blend == photocraft_doc::BlendMode::Normal
        && opacity.to_bits() == 1.0f32.to_bits()
        && fill_opacity.to_bits() == 1.0f32.to_bits()
        && !*clipped
        && mask.is_none()
        && vector_mask.is_none()
        && *enabled
        && items.is_empty()
        && psd_raw.is_none()
        && reference.is_none()
        && *label == photocraft_doc::LabelColor::None
        && matches!(content, LayerContent::Raster(surface) if surface.format() == document.pixel_format())
        && psd_blocks.is_empty()
        && psd_id.is_none()
        && fill_cache.is_none()
        && link_group.is_none()
        && *excluded_channels == 0
        && *blend_if == Default::default()
        && *advanced == Default::default()
        && video.is_none()
}

pub(crate) fn warnings_for_flat_tiff(document: &Document, actual_opaque_image: bool) -> Vec<String> {
    warnings_for_impl(document, Destination::Flat, actual_opaque_image && canonical_background(document))
}

pub(crate) fn warnings_for(document: &Document, destination: Destination) -> Vec<String> {
    warnings_for_impl(document, destination, false)
}
fn warnings_for_impl(document: &Document, destination: Destination, reconstructs_background: bool) -> Vec<String> {
    if destination == Destination::Native {
        return Vec::new();
    }
    let mut out = Warnings::new();
    // Preserve legacy strings/order relied on by callers and existing tests.
    if document.xcf_state.as_ref().is_some_and(|state| !state.filter_stacks.is_empty()) {
        out.push(|| "Live GIMP filter state is not retained by this format; save as .pcraft to keep it".into());
    }
    if document.sample_encoding.is_some() || document.source_color_context.is_some() {
        out.push(|| "Original GIMP sample interpretation and color provenance are not retained by this format; save as .pcraft to keep them".into());
    }
    let mut frames: [Option<std::slice::Iter<'_, photocraft_doc::Layer>>; photocraft_doc::MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    if let Some(frame) = frames.first_mut() {
        *frame = Some(document.layers.iter());
    }
    let mut depth = 0usize;
    loop {
        let next = frames.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else {
            if depth == 0 {
                break;
            }
            depth -= 1;
            continue;
        };
        if !out.visit() {
            break;
        }
        if destination == Destination::Flat && !reconstructs_background {
            let content = match &layer.content {
                LayerContent::Group(_) => "group hierarchy and child layer records",
                LayerContent::Text(_) => "editable text layer content",
                LayerContent::Shape(_) => "editable shape layer content",
                LayerContent::Smart(_) => "smart-object layer content and source binding",
                LayerContent::Adjustment(_) => "editable adjustment layer content",
                LayerContent::Fill(_) => "editable fill layer content",
                LayerContent::Raster(_) | LayerContent::BackedRaster(_) => "independent raster layer content",
            };
            if !out.push(|| format!("Layer {} {:?}: flat raster export does not retain {content}, layer name/visibility/opacity/blend/locks or independent layer editing; only the rendered composite is exported", layer.id.0, label(&layer.name))) { break; }
            if layer.mask.is_some() && !out.push(|| format!("Layer {} {:?}: flat raster export does not retain the independent layer mask, its samples or editing/enabled/link state; mask appearance may contribute to the rendered composite", layer.id.0, label(&layer.name))) { break; }
            if layer.vector_mask.is_some() && !out.push(|| format!("Layer {} {:?}: flat raster export does not retain editable vector-mask geometry or mask policy; mask appearance may contribute to the rendered composite", layer.id.0, label(&layer.name))) { break; }
        }
        if layer.indexed_source.is_some()
            && !out
                .push(|| format!("Layer {:?}: original GIMP palette indices are not retained by this format; save as .pcraft to keep them", label(&layer.name)))
        {
            break;
        }
        if layer.import_compositing.is_some()
            && !out.push(|| {
                format!("Layer {:?}: original GIMP compositing settings are not retained by this format; save as .pcraft to keep them", label(&layer.name))
            })
        {
            break;
        }
        if let LayerContent::Group(group) = &layer.content {
            let Some(next_depth) = depth.checked_add(1).filter(|value| *value <= photocraft_doc::MAX_GROUP_DEPTH) else {
                out.omitted = true;
                break;
            };
            depth = next_depth;
            if let Some(frame) = frames.get_mut(depth) {
                *frame = Some(group.children.iter());
            } else {
                out.omitted = true;
                break;
            }
        }
    }
    if let Some(resolution) = &document.physical_resolution {
        match &resolution.unit {
            PhysicalUnit::Custom(unit) => {
                out.push(|| format!("Custom physical unit {:?}: name, factor, digits, symbol and abbreviation are not retained by this format; numeric DPI axes are exported separately", label(&unit.name)));
            }
            PhysicalUnit::Inch => {}
            _ => {
                out.push(|| "Physical display-unit preference is not retained by this format; numeric DPI axes are exported separately".into());
            }
        }
    }
    if destination == Destination::Flat {
        flat_objects(document, &mut out);
    }
    let Some(state) = &document.xcf_state else {
        return out.finish();
    };
    for indexed in &state.editable_indexed {
        if !out.visit() {
            break;
        }
        if !out.push(|| format!("Layer {}: current editable GIMP index plane, alpha plane and palette binding are not retained as editable indexed state by this format; save as .pcraft to keep them", indexed.owner.0)) { break; }
    }
    for source in &state.source_items {
        if !out.visit() {
            break;
        }
        let semantics = match &source.source {
            EditableSource::Text { .. } => "editable GIMP text/markup, source configuration, font binding and source-cache identity",
            EditableSource::Vector { .. } => "editable GIMP vector-source configuration, path binding and source-cache identity",
            EditableSource::Linked { .. } => "editable GIMP linked-source configuration, resource binding, current verification and source-cache identity",
        };
        if !out.push(|| {
            format!(
                "{}: {semantics} are not retained by this format; current pixels do not retain these source semantics; save as .pcraft to keep them",
                item(source.owner)
            )
        }) {
            break;
        }
    }
    for resource in &state.resources {
        if !out.visit() {
            break;
        }
        if !out.push(|| format!("Resource {} {:?}: original/current GIMP resource identity, binding and resolution status are not retained as canonical resource state by this format; save as .pcraft to keep them", resource.id.0, label(&resource.name))) { break; }
    }
    for stack in &state.filter_stacks {
        if !out.visit() {
            break;
        }
        for node in &stack.nodes {
            if !out.visit() {
                break;
            }
            if !out.push(|| format!("{} filter {} {:?}: typed operation/version/configuration, enabled/opacity/clipping/region, composition, mask and auxiliary bindings are not retained by this format; save as .pcraft to keep them", item(stack.owner), node.id.0, label(&node.operation.name))) { break; }
        }
    }
    for mask in &state.filter_masks {
        if !out.visit() {
            break;
        }
        if !out.push(|| format!("Filter mask {}: canonical filter-owner binding and native mask state are not retained as a live GIMP filter mask by this format; save as .pcraft to keep them", mask.id.0)) { break; }
    }
    for current in &state.items {
        if !out.visit() {
            break;
        }
        if (current.source_flags != 0 || current.source_tattoo.is_some())
            && !out.push(|| {
                format!(
                    "{} {:?}: original GIMP item flags/tattoo and their typed identity binding are not retained by this format; save as .pcraft to keep them",
                    item(current.item),
                    label(&current.name)
                )
            })
        {
            break;
        }
        if !current.custom_name
            && !out.push(|| {
                format!(
                    "{} {:?}: current GIMP automatic/custom-name policy flag is not retained by this format; the literal name may export independently",
                    item(current.item),
                    label(&current.name)
                )
            })
        {
            break;
        }
        let locks = current.locks;
        let loses_locks = locks.visibility || (!matches!(current.item, ItemRef::Layer(_)) && (locks.content || locks.position || locks.alpha));
        if loses_locks
            && !out.push(|| {
                format!(
                    "{}: GIMP visibility lock or non-layer typed locks are not retained by this format; ordinary native layer locks may export independently",
                    item(current.item)
                )
            })
        {
            break;
        }
        if let Some(mask) = &current.mask {
            let outside = f64::from_bits(mask.outside_f64_bits);
            // PSD's mask default is an 8-bit value; enabled/link flags and mask
            // samples already map. Only extra UI state/exact default is warned.
            let exact_default = outside.is_finite() && (0.0..=1.0).contains(&outside) && ((outside * 255.0).round() / 255.0).to_bits() == outside.to_bits();
            if (mask.shown || mask.editing || !exact_default) && !out.push(|| format!("{}: GIMP mask shown/editing state or exact outside-default policy is not retained by this format; mask samples/enabled/link state may export independently", item(current.item))) { break; }
        }
    }
    // These canonical current policies have no PSD/flat representation, even
    // when an ordinary native name, blend mode or channel sample exports.
    for current in &state.items {
        if !out.visit() {
            break;
        }
        if current.composition.is_some() && !out.push(|| format!("{}: current GIMP composition compatibility epoch, stored mode/spaces/operator and resolved policy are not retained by this format; a native blend mode may export independently", item(current.item))) { break; }
        if !matches!(current.item, ItemRef::Layer(_))
            && current.visible
            && !out.push(|| {
                format!(
                    "{}: current GIMP non-layer visibility state is not retained by this format; native layer visibility may export independently",
                    item(current.item)
                )
            })
        {
            break;
        }
    }
    for channel in document.channels.iter().filter(|_| destination == Destination::Layered) {
        if !out.visit() {
            break;
        }
        let loses_display = match channel.spot {
            Some((ink, solidity)) => !display_color_exact(&ink) || !display_component_exact(solidity, 100.0),
            None => !display_color_exact(&channel.color) || !display_component_exact(channel.opacity, 100.0),
        };
        if loses_display && !out.push(|| format!("Channel {} {:?}: exact native overlay/spot color model, component bits or opacity/solidity bits are not retained by this format; PSD DisplayInfo exports RGB16 color and whole-percent opacity independently", channel.id.0, label(&channel.name))) { break; }
        // Spot DisplayInfo serializes ink/solidity, not its separate alpha-overlay
        // fallback color, opacity and selected/masked-areas choice.
        if channel.spot.is_some()
            && (!same_color_bits(&channel.color, &photocraft_doc::AlphaChannel::DEFAULT_COLOR)
                || channel.opacity.to_bits() != 0.5f32.to_bits()
                || channel.indicates != photocraft_doc::ColorIndicates::MaskedAreas)
            && !out.push(|| format!("Channel {} {:?}: separate native spot-channel fallback overlay color, opacity and selected/masked-area interpretation are not retained by this format; spot ink/solidity may export independently", channel.id.0, label(&channel.name))) { break; }
    }
    for set in &state.item_sets {
        if !out.visit() {
            break;
        }
        if !out.push(|| {
            format!(
                "Item set {} {:?}: typed GIMP family/rule/member bindings are not retained by this format; save as .pcraft to keep them",
                set.id.0,
                label(&set.name)
            )
        }) {
            break;
        }
    }
    if !state.selection.selected.is_empty()
        || state.selection.active.is_some()
        || !state.selection.selected_paths.is_empty()
        || state.selection.mask_target.is_some()
    {
        out.push(|| "GIMP typed selected/active items, selected paths and mask edit target are not retained by this format; selection/channel samples may export independently".into());
    }
    if let Some(attachment) = &state.floating {
        out.push(|| {
            format!(
                "Floating layer {}: typed attachment to {} is not retained by this format; save as .pcraft to keep it",
                attachment.layer.0,
                item(attachment.target)
            )
        });
    }
    if !state.opaque.is_empty() {
        out.push(|| "Original opaque GIMP property records are not retained by this format; save as .pcraft to keep them".into());
    }
    let provenance = &state.provenance;
    if !provenance.format.is_empty()
        || provenance.format_version != 0
        || !provenance.container.is_empty()
        || provenance.producer.is_some()
        || provenance.release_id.is_some()
        || !provenance.source_label.is_empty()
        || provenance.source_hash.is_some()
        || provenance.original_locator.is_some()
        || !provenance.protected_source_fingerprints.is_empty()
    {
        // Locator contents are deliberately not copied into user-facing warnings.
        out.push(|| "Original GIMP project/container/producer identity, source hash/locator and retained source-protection fingerprints are not retained as canonical provenance by this format; save as .pcraft to keep them".into());
    }
    out.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Layer, Size};
    #[test]
    fn legacy_strings_and_unicode_labels_remain_bounded() {
        let mut doc = Document::new("test", Size::new(1, 1), ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let mut layer = Layer::raster("🎨".repeat(10_000), doc.pixel_format());
        layer.import_compositing = Some(photocraft_doc::ImportCompositing {
            blend_encoding: photocraft_doc::SampleEncoding::NonlinearProfile,
            composite_encoding: photocraft_doc::SampleEncoding::NonlinearProfile,
            pass_through: false,
        });
        doc.layers.push(layer);
        let rows = warnings(&doc);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains('…'));
        assert!(rows[0].len() < 1024);
        assert!(rows[0].contains("original GIMP compositing settings are not retained"));
    }
    #[test]
    fn warning_overflow_is_explicit_and_plain_native_equivalents_are_quiet() {
        let mut doc = Document::new("test", Size::new(1, 1), ColorMode::Rgb, photocraft_doc::SampleType::U8);
        assert!(warnings(&doc).is_empty());
        for _ in 0..300 {
            let mut layer = Layer::raster("layer", doc.pixel_format());
            layer.import_compositing = Some(photocraft_doc::ImportCompositing {
                blend_encoding: photocraft_doc::SampleEncoding::Linear,
                composite_encoding: photocraft_doc::SampleEncoding::Linear,
                pass_through: false,
            });
            doc.layers.push(layer);
        }
        let rows = warnings(&doc);
        assert_eq!(rows.len(), ROWS);
        assert!(rows.last().unwrap().contains("omitted"));
    }
    #[test]
    fn current_indexed_pixels_warn_without_immutable_indexed_source() {
        let mut doc = Document::new("test", Size::new(1, 1), ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let layer = Layer::raster("current", doc.pixel_format());
        let owner = layer.id;
        doc.layers.push(layer);
        let mut state = photocraft_doc::xcf_state::XcfState::default();
        state.editable_indexed.push(photocraft_doc::EditableIndexedLayer {
            owner,
            raster: photocraft_doc::EditableIndexedRaster::try_new(
                photocraft_doc::Rect { x0: 0, y0: 0, x1: 1, y1: 1 },
                std::sync::Arc::new(vec![0]),
                None,
                vec![[1, 2, 3]],
            )
            .unwrap(),
        });
        doc.xcf_state = Some(state);
        let rows = warnings(&doc);
        assert!(rows.iter().any(|row| row.contains("current editable GIMP index plane")));
        assert!(!rows.iter().any(|row| row.contains("original GIMP palette indices")));
    }
    #[test]
    fn current_composition_and_non_layer_visibility_do_not_need_source_flags() {
        use photocraft_doc::{
            item_identity::ChannelId,
            xcf_state::{ItemLocks, ItemState, XcfState},
        };
        let mut doc = Document::new("current", Size::new(1, 1), ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let mut state = XcfState::default();
        state.items.push(ItemState {
            item: ItemRef::Channel(ChannelId(37)),
            name: "current channel".into(),
            custom_name: true,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: None,
            mask: None,
            composition: Some(photocraft_doc::import_compositing::GimpComposition {
                compatibility_epoch: "diagnostic-only".into(),
                source_mode: 0,
                legacy: false,
                stored_blend_space: 0,
                stored_composite_space: 0,
                stored_operator: 0,
                resolved: None,
                pass_through: false,
            }),
        });
        doc.xcf_state = Some(state);
        let before = doc.clone();
        let rows = warnings(&doc);
        assert!(rows.iter().any(|r| r.contains("Channel 37: current GIMP composition compatibility epoch")));
        assert!(rows.iter().any(|r| r.contains("Channel 37: current GIMP non-layer visibility")));
        assert!(!rows.iter().any(|r| r.contains("flags/tattoo")));
        assert_eq!(doc, before);
    }
    #[test]
    fn representable_channel_defaults_and_indicates_are_not_blanket_losses() {
        let mut doc = Document::new("channels", Size::new(1, 1), ColorMode::Rgb, photocraft_doc::SampleType::U8);
        doc.xcf_state = Some(Default::default());
        let mut channel = photocraft_doc::AlphaChannel::new("saved", photocraft_doc::Surface::new(photocraft_doc::PixelFormat::GRAY8));
        channel.indicates = photocraft_doc::ColorIndicates::SelectedAreas;
        doc.channels.push(channel);
        assert!(warnings(&doc).is_empty());
        doc.channels[0].opacity = f32::from_bits(0.5f32.to_bits() + 1);
        let before = doc.clone();
        let rows = warnings(&doc);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("whole-percent opacity"));
        assert_eq!(doc, before);
        doc.channels[0].opacity = 0.5;
        doc.channels[0].color = photocraft_doc::Color::rgb(-0.0, 0.0, 0.0);
        assert!(warnings(&doc).iter().any(|r| r.contains("component bits")));
        doc.channels[0].color = photocraft_doc::AlphaChannel::DEFAULT_COLOR;
        doc.channels[0].spot = Some((photocraft_doc::Color::rgb(0.0, 1.0, 0.0), 0.5));
        let rows = warnings(&doc);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("separate native spot-channel fallback"));
    }
    #[test]
    fn provenance_warning_keeps_locator_contents_inert_and_bounded() {
        let mut doc = Document::new("provenance", Size::new(1, 1), ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let mut state = photocraft_doc::xcf_state::XcfState::default();
        state.provenance.original_locator = Some("inert-private-source-location".repeat(10000));
        state.provenance.protected_source_fingerprints.push("a".repeat(64));
        doc.xcf_state = Some(state);
        let before = doc.clone();
        let rows = warnings(&doc);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("source-protection fingerprints"));
        assert!(!rows[0].contains("inert-private-source-location"));
        assert!(rows[0].len() < 1024);
        assert_eq!(doc, before);
    }

    #[test]
    fn tiff_background_metadata_exemption_never_hides_source_state_or_other_losses() {
        let mut doc = Document::new("own", Size::new(1, 1), ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let mut layer = photocraft_doc::Layer::raster("Background", doc.pixel_format());
        layer.locks.transparency = true;
        layer.locks.position = true;
        doc.layers.push(layer);
        assert!(warnings_for_flat_tiff(&doc, true).is_empty());
        assert!(warnings_for_flat_tiff(&doc, false).iter().any(|row| row.contains("independent raster layer")));
        assert!(
            warnings_for(&doc, Destination::Flat).iter().any(|row| row.contains("independent raster layer")),
            "other flat formats retain their prior diagnostics"
        );
        // Classifier-only malformed/empty source state is not a Source-admission fixture.
        doc.xcf_state = Some(Default::default());
        assert!(warnings_for_flat_tiff(&doc, true).iter().any(|row| row.contains("independent raster layer")));
        doc.xcf_state = None;
        doc.guides.horizontal.push(1.0);
        let rows = warnings_for_flat_tiff(&doc, true);
        assert!(!rows.iter().any(|row| row.contains("independent raster layer")));
        assert!(rows.iter().any(|row| row.contains("Guides:")), "the layer exception does not suppress document losses");
    }
}
