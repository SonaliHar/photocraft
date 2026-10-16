//! Imported filter controls dispatch the same checked commands as automation.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{
    LayerId,
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
};
use serde_json::json;
mod alpha_clip;
mod alpha_clip_finite;
mod assigned_box;
mod observed_box;
mod semi_flatten;
mod filtered_flat_point;
mod hue_chroma;
mod vibrance;
mod channel_mixer;
mod color_overlay;
mod color_rotate;
mod component_extract;
mod current_items;
pub(crate) mod composition;
mod editing;
mod filter_stack;
mod filter_insert;
mod filter_mask;
mod graph_inspect;
mod gaussian_group;
pub(crate) mod source_inspect;
mod item_sets;
mod indexed_palette;
mod linked_image;
mod layer_mask;
mod mask_geometry;
mod literal_color;
mod mono_mixer_signed;
pub(crate) mod path_name;
mod red_eye;
mod semantic_selection;
mod text_name;
mod text_view_cache;
mod vector_style;
mod vector_width;
mod underline;
#[cfg(test)]
mod accounted_threshold_alpha_tests;
#[cfg(test)]
mod accounted_threshold_tests;

pub(crate) fn mask_storage_params(app:&PhotocraftApp,layer:LayerId,on_set:bool,command:&str)->Result<serde_json::Value,String> {
    layer_mask::queued_params(app,layer,on_set,command)
}

fn run_checked(app: &mut PhotocraftApp, captured: &photocraft_engine::document_snapshot::DocumentSnapshot, revision: u64, command: &str, params: serde_json::Value) {
    if !app.session.active().is_some_and(|current| current.doc.id == captured.id && current.revision == revision && current.doc.shares_state(captured)) {
        app.ui.status = "xcf.properties.stale-target".into();
        app.ui.status_error = true;
        return;
    }
    if let Err(error) = app.run(command, params) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

pub(crate) fn linked_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    linked_image::properties(app, ui, layer);
}

pub(crate) fn editing_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    current_items::properties(app, ui, layer);
    composition::properties(app, ui, layer);
    source_inspect::properties(app, ui, layer);
    indexed_palette::properties(app, ui, layer);
    layer_mask::properties(app, ui, layer);
    mask_geometry::properties(app, ui, layer);
    semantic_selection::properties(app, ui, layer);
    editing::properties(app, ui, layer);
    vector_style::properties(app, ui, layer);
    vector_width::properties(app, ui, layer);
}

pub(crate) fn has_item_set_properties(document: &photocraft_doc::Document, layer: LayerId) -> bool {
    item_sets::has_matching(&document, layer) || item_sets::has_creation(&document, layer)
}

pub(crate) fn item_set_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    item_sets::properties(app, ui, layer);
}

pub(crate) fn text_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    text_name::properties(app, ui, layer);
    let Some((document, revision)) = app.session.active().map(|state| (state.doc.clone(), state.revision)) else { return };
    let Some(state) = &document.xcf_state else { return };
    let hints = match text_view_cache::get(ui.ctx(), &document, revision, layer) {
        Ok(hints) => hints,
        Err(error) => { ui.label(error.to_string()); return; }
    };
    let Some(source) = hints.source_index.and_then(|index| state.source_items.get(index)).filter(|source| source.owner == ItemRef::Layer(layer)) else { return };
    let photocraft_doc::xcf_items::EditableSource::Text { content, .. } = &source.source else { return };
    let live = app.ui.transform.is_none() && !app.session.has_jobs() && app.session.active().is_some_and(|active|
        active.floating.is_none() && active.doc.xcf_state.as_ref().is_none_or(|state| state.floating.is_none()));
    let same_target =
        app.ui.xcf_text_draft.as_ref().is_some_and(|draft| draft.document == document.id.0 && draft.layer == layer.0 && draft.revision == revision);
    if !same_target {
        app.ui.xcf_text_draft = Some(crate::state::XcfTextDraft {
            document: document.id.0,
            layer: layer.0,
            revision,
            content: content.chars().take(256).collect(),
            ..Default::default()
        });
    }
    let Some(draft) = app.ui.xcf_text_draft.as_mut() else { return };
    // Restored view-state booleans cannot replace this snapshot-derived bundle.
    draft.editable = hints.content_editable;
    draft.font_size = hints.font_size;
    draft.validated = true;
    // View state restored through automation is still bounded and carries no authority.
    if draft.content.chars().nth(256).is_some() {
        draft.content = draft.content.chars().take(256).collect();
    }
    let ordinary = document.effective_locks(layer);
    let unlocked =
        !ordinary.all && !ordinary.pixels && !ordinary.position && state.effective_locks(source.owner).is_ok_and(|locks| !locks.content && !locks.position);
    let editable = draft.editable && unlocked && live;
    let current_color = hints.color;
    let color_editable = hints.color_editable && unlocked && live;
    let current_spacing = hints.spacing;
    let spacing_editable = hints.spacing_editable && unlocked && live;
    let current_line_spacing = hints.line_spacing;
    let line_spacing_editable = hints.line_spacing_editable && unlocked && live;
    let current_origin = hints.origin;
    let position_editable = hints.position_editable && unlocked && live;
    let mut color_change = None;
    let tokens = Tokens::get(ui.ctx());
    ui.separator();
    ui.label(egui::RichText::new(tl!("GIMP Text")).color(tokens.text));
    let label = ui.label(tl!("Text content"));
    let content_editor = if current_line_spacing.is_some() {
        egui::TextEdit::multiline(&mut draft.content).char_limit(9).desired_rows(2)
    } else {
        egui::TextEdit::singleline(&mut draft.content).char_limit(8)
    };
    ui.add_enabled(editable, content_editor.desired_width(f32::INFINITY)).labelled_by(label.id);
    let mut size_change = None;
    if let Some(current_size) = draft.font_size {
        let mut selected_size = current_size;
        ui.add_enabled_ui(editable, |ui| {
            egui::ComboBox::from_id_salt("xcf_text_font_size").selected_text(format!("{current_size} px")).show_ui(ui, |ui| {
                for size in [16, 24, 32, 40] {
                    if size == 24 && current_spacing.is_some_and(|spacing| spacing != 0.) {
                        continue;
                    }
                    ui.selectable_value(&mut selected_size, size, format!("{size} px"));
                }
            });
            ui.label(tl!("Font size"));
        });
        if selected_size != current_size {
            size_change = Some(selected_size);
        }
    }
    let mut spacing_change = None;
    if let Some(current) = current_spacing
        && matches!(current, 0. | 2. | 4.)
    {
        let mut selected = current as u32;
        ui.add_enabled_ui(spacing_editable, |ui| {
            egui::ComboBox::from_id_salt("xcf_text_letter_spacing").selected_text(format!("{current} px")).show_ui(ui, |ui| {
                for spacing in [0, 2, 4] {
                    ui.selectable_value(&mut selected, spacing, format!("{spacing} px"));
                }
            });
            ui.label(tl!("Letter spacing"));
        });
        if selected as f32 != current {
            spacing_change = Some(selected);
        }
    }
    let mut line_spacing_change = None;
    if let Some(current) = current_line_spacing
        && matches!(current, 0. | 2. | 4.)
    {
        let mut selected = current as u32;
        ui.add_enabled_ui(line_spacing_editable, |ui| {
            let selector = egui::ComboBox::from_id_salt("xcf_text_line_spacing").selected_text(format!("{current} px")).show_ui(ui, |ui| {
                for spacing in [0, 2, 4] {
                    ui.selectable_value(&mut selected, spacing, format!("{spacing} px"));
                }
            });
            let label = ui.label(tl!("Line spacing"));
            selector.response.labelled_by(label.id);
        });
        if selected as f32 != current {
            line_spacing_change = Some(selected);
        }
    }
    if let Some(color) = current_color
        && let Some(mut rgb) = hints.display_rgb
    {
        ui.add_enabled_ui(color_editable, |ui| {
            ui.horizontal(|ui| {
                ui.label(tl!("Text color"));
                if ui.color_edit_button_rgb(&mut rgb).changed() {
                    color_change = text_color_patch(&document, layer, color, rgb);
                }
            });
        });
    }
    let mut position_change = None;
    if let Some(current) = current_origin {
        let mut origin = current;
        ui.add_enabled_ui(position_editable, |ui| {
            ui.horizontal(|ui| {
                let [x, y] = &mut origin;
                let label = ui.label(tl!("Text X"));
                ui.add(egui::DragValue::new(x).speed(1).range(-(1 << 29)..=(1 << 29)).suffix(" px")).labelled_by(label.id);
                let label = ui.label(tl!("Text Y"));
                ui.add(egui::DragValue::new(y).speed(1).range(-(1 << 29)..=(1 << 29)).suffix(" px")).labelled_by(label.id);
            });
        });
        if origin != current {
            position_change = Some(origin);
        }
    }
    let mut apply = false;
    ui.horizontal(|ui| {
        apply = ui.add_enabled(editable && draft.content != *content, egui::Button::new(tl!("Apply Text"))).clicked();
        if ui.add_enabled(editable && draft.content != *content, egui::Button::new(tl!("Reset Text"))).clicked() {
            draft.content = content.chars().take(256).collect();
        }
    });
    if !editable {
        ui.label(egui::RichText::new(tl!("Text content editing is unavailable for this layer.")).color(tokens.text_dim));
    }
    let proposed = apply.then(|| draft.content.clone());
    if let Some(size) = size_change {
        run_checked(app, &document, revision, "xcf.text.fontSize.set", json!({"layerId":layer.0.to_string(),"sizePx":size}));
    }
    if let Some(patch) = color_change {
        run_checked(app, &document, revision, "xcf.text.color.set", patch);
    }
    if let Some(spacing) = spacing_change {
        run_checked(app, &document, revision, "xcf.text.letterSpacing.set", json!({"layerId":layer.0.to_string(),"letterSpacingPx":spacing}));
    }
    if let Some(spacing) = line_spacing_change {
        run_checked(app, &document, revision, "xcf.text.lineSpacing.set", json!({"layerId":layer.0.to_string(),"lineSpacingPx":spacing}));
    }
    if let Some(origin) = position_change {
        run_checked(app, &document, revision, "xcf.text.position.set", json!({"layerId":layer.0.to_string(),"originPx":origin}));
    }
    if let Some(content) = proposed {
        run_checked(app, &document, revision, "xcf.text.content.set", json!({"layerId":layer.0.to_string(),"content":content}));
    }
    underline::properties(app, ui, layer);
}

fn text_color_profiles(
    document: &photocraft_doc::Document,
    color: photocraft_text::xcf::TextColor,
) -> Option<(photocraft_cms::Profile, photocraft_cms::Profile)> {
    use photocraft_cms::xcf_color::{Encoding, effective_implicit_profile, effective_profile_from_definition};
    let definition = effective_implicit_profile(document.icc_profile.as_ref()?, photocraft_cms::ColorSpace::Rgb, Encoding::NonlinearProfile).ok()?;
    let encoding = match color.encoding {
        photocraft_text::xcf::TextColorEncoding::Linear => Encoding::Linear,
        photocraft_text::xcf::TextColorEncoding::Nonlinear => Encoding::NonlinearProfile,
    };
    let declared = effective_profile_from_definition(&definition, encoding).ok()?;
    // egui's float RGB picker accepts linear RGB. The admitted text context
    // binds sRGB primaries; its declared transfer function remains explicit.
    let picker = effective_profile_from_definition(&definition, Encoding::Linear).ok()?;
    Some((declared, picker))
}

fn text_color_rgb(document: &photocraft_doc::Document, color: photocraft_text::xcf::TextColor) -> Option<[f32; 3]> {
    let (declared, display) = text_color_profiles(document, color)?;
    let transform = photocraft_cms::Transform::new(&declared, &display, photocraft_cms::Intent::RelativeColorimetric, false).ok()?;
    let input = color.rgba_f32_bits.map(|bits| f64::from(f32::from_bits(bits)));
    let mut rgb = [0.0; 3];
    transform.try_eval_f64(input.get(..3)?, &mut rgb).ok()?;
    rgb.iter().all(|value| value.is_finite()).then(|| rgb.map(|value| value.clamp(0.0, 1.0) as f32))
}

fn text_color_patch(document: &photocraft_doc::Document, layer: LayerId, color: photocraft_text::xcf::TextColor, rgb: [f32; 3]) -> Option<serde_json::Value> {
    if rgb.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
        return None;
    }
    let (declared, display) = text_color_profiles(document, color)?;
    let transform = photocraft_cms::Transform::new(&display, &declared, photocraft_cms::Intent::RelativeColorimetric, false).ok()?;
    let mut converted = [0.0; 3];
    transform.try_eval_f64(&rgb.map(f64::from), &mut converted).ok()?;
    if converted.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let mut bits = color.rgba_f32_bits;
    for (output, value) in bits.iter_mut().take(3).zip(converted) {
        *output = (value.clamp(0.0, 1.0) as f32).to_bits();
    }
    let encoding = match color.encoding {
        photocraft_text::xcf::TextColorEncoding::Linear => "linear",
        photocraft_text::xcf::TextColorEncoding::Nonlinear => "nonlinear",
    };
    Some(json!({"layerId":layer.0.to_string(),"encoding":encoding,"rgbaF32Bits":bits.map(|bits| format!("{bits:08x}"))}))
}

pub(crate) fn filter_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    if app.session.active().and_then(|s|s.doc.accounted()).is_some_and(|o|o.is_readonly_filtered_flat_point_storage()) {
        filtered_flat_point::properties(app, ui, layer);
        return;
    }
    filter_mask::properties(app, ui, layer);
    filter_insert::properties(app, ui, layer);
    filter_stack::properties(app, ui, layer);
    let Some((document, revision)) = app.session.active().map(|state| (state.doc.clone(), state.revision)) else { return };
    let Some(state) = document.xcf_state.as_ref() else { return };
    let Some(stack) = state.filter_stacks.iter().find(|stack| stack.owner == ItemRef::Layer(layer)) else { return };
    if stack.nodes.is_empty() {
        return;
    }
    let tokens = Tokens::get(ui.ctx());
    let ordinary = document.effective_locks(layer);
    let unlocked = !ordinary.all && !ordinary.pixels && state.effective_locks(stack.owner).is_ok_and(|locks| !locks.content);
    ui.separator();
    ui.label(egui::RichText::new(tl!("GIMP Filters")).color(tokens.text));
    // Retained stacks can be large; keep the panel bounded without hiding that fact.
    for node in stack.nodes.iter().take(64) {
        ui.push_id(node.id.0, |ui| {
            let color_exchange = node.operation.name == "gegl:color-exchange";
            let color_to_alpha = node.operation.name == "gegl:color-to-alpha";
            let color_overlay = node.operation.name == "gegl:color-overlay";
            let channel_mixer = node.operation.name == "gegl:channel-mixer";
            let color_rotate = node.operation.name == "gegl:color-rotate";
            let mono_signed = node.operation.name == "gegl:mono-mixer" && node.operation.schema_epoch == mono_mixer_signed::EPOCH;
            let component_extract = node.operation.name == "gegl:component-extract";
            let alpha_clip = node.operation.name == "gegl:alpha-clip";
            let red_eye = node.operation.name == "gegl:red-eye-removal";
            let alpha_clip_finite = alpha_clip && node.operation.schema_epoch == alpha_clip_finite::EPOCH;
            let literal_colors = color_exchange || color_to_alpha || color_overlay;
            let assigned = document.accounted().is_some_and(|owner| owner.is_readonly_assigned_point_storage());
            let editable = unlocked && !assigned && photocraft_engine::xcf_filter_cmds::scalar_edit_candidate(node)
                && (!color_exchange || literal_color::valid_arguments(&node.arguments))
                && (!red_eye || node.operation.stored_version == "0:0"
                    && matches!(node.operation.schema_epoch.as_str(), red_eye::EPOCH | photocraft_compose::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH)
                    && red_eye::valid_arguments(&node.arguments))
                && (!color_rotate || color_rotate::valid_node(node))
                && (!channel_mixer || node.operation.stored_version == "0:0" && node.operation.schema_epoch == channel_mixer::EPOCH && channel_mixer::valid_arguments(&node.arguments))
                && (!mono_signed || node.operation.stored_version == "0:0" && mono_mixer_signed::valid_arguments(&node.arguments))
                && (!color_overlay || node.operation.stored_version == "0:0" && node.operation.schema_epoch == color_overlay::EPOCH && color_overlay::valid_arguments(&node.arguments))
                && (!alpha_clip || node.operation.stored_version == "0:0" && match node.operation.schema_epoch.as_str() { alpha_clip::EPOCH => alpha_clip::valid_arguments(&node.arguments), alpha_clip_finite::EPOCH => alpha_clip_finite::valid_arguments(&node.arguments), _ => false })
                && (!color_to_alpha || node.operation.stored_version == "0:0" && literal_color::valid_color_to_alpha_arguments(&node.arguments))
                && (!component_extract || node.operation.stored_version == "0:0" && component_extract::valid_arguments_for_epoch(&node.operation.schema_epoch, &node.arguments));
            let caption: String = node.source_display_name.as_deref().filter(|name| !name.is_empty()).unwrap_or(&node.operation.name).chars().take(128).collect();
            if observed_box::scope(&document, node) {
                observed_box::controls(app, ui, &document, revision, layer, node, unlocked, &caption);
                return;
            }
            if semi_flatten::scope(&document, node) {
                semi_flatten::controls(app, ui, &document, revision, layer, node, unlocked, &caption);
                return;
            }
            if hue_chroma::scope(&document, node) {
                hue_chroma::controls(app, ui, &document, revision, layer, node, unlocked, &caption);
                return;
            }
            if vibrance::scope(&document, node) {
                vibrance::controls(app, ui, &document, revision, layer, node, unlocked, &caption);
                return;
            }
            ui.horizontal_wrapped(|ui| {
                let mut enabled = node.enabled;
                let activation_editable = if assigned {
                    unlocked && assigned_box::candidate(&document, layer, node)
                } else { editable || (unlocked
                    && document.layer(layer).is_some_and(|owner| matches!(owner.content, photocraft_doc::LayerContent::Group(_)))
                    && photocraft_engine::xcf_filter_cmds::activation_edit_candidate(&document, node)) };
                if ui.add_enabled(activation_editable, egui::Checkbox::new(&mut enabled, caption)).changed() {
                    run_checked(app, &document, revision, "xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":enabled}));
                }
            });
            if document.accounted().is_some_and(|owner| owner.is_readonly_gaussian_group_storage()) {
                if gaussian_group::candidate(&document, layer, node) {
                    gaussian_group::controls(app, ui, &document, revision, node, unlocked);
                } else {
                    ui.label(tl!("Filter editing is unavailable"));
                }
                return;
            }
            if color_rotate {
                color_rotate::controls(app, ui, &document, revision, node, editable);
                if !editable {
                    ui.label(if unlocked { tl!("Filter editing is unavailable") } else { tl!("Layer content is locked") });
                }
                return;
            }
            if node.operation.name == "gimp:posterize" {
                for argument in node.arguments.iter().take(64) {
                    if let FilterValue::Number(ExactNumber::Signed { width: 32, value }) = argument.value
                        && argument.name == "levels" {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("{}: {value}", tl!("Levels")));
                            for (label, next) in [(tl!("Decrease Levels"), value.checked_sub(1)), (tl!("Increase Levels"), value.checked_add(1))] {
                                let admissible = next.filter(|value| (2..=256).contains(value));
                                if ui.add_enabled(editable && admissible.is_some(), egui::Button::new(label)).clicked()
                                    && let Some(next) = admissible {
                                    run_checked(app, &document, revision, "xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"levels","value":{"Number":{"Signed":{"width":32,"value":next}}}}]}));
                                }
                            }
                        });
                    }
                }
            }
            if node.operation.name == "gimp:color-balance"
                && let Some(FilterValue::Config(ranges)) = node.arguments.iter().find(|argument| argument.name == "config").map(|argument| &argument.value)
                && ranges.len() == 3
            {
                for (range_index, (range, name)) in ranges.iter().zip(["shadows", "midtones", "highlights"]).enumerate() {
                    let caption = match name { "shadows" => tl!("Shadows"), "midtones" => tl!("Midtones"), _ => tl!("Highlights") };
                    if range.name != name { continue; }
                    let FilterValue::Config(fields) = &range.value else { continue };
                    if fields.len() != 3 { continue; }
                    ui.label(caption);
                    for (axis_index, (field, name)) in fields.iter().zip(["cyan-red", "magenta-green", "yellow-blue"]).enumerate() {
                        let FilterValue::Number(ExactNumber::Float64Bits(bits)) = field.value else { continue };
                        let value = f64::from_bits(bits);
                        if field.name != name || !value.is_finite() || !(-1.0..=1.0).contains(&value) { continue; }
                        let label = match name { "cyan-red" => tl!("Cyan / Red"), "magenta-green" => tl!("Magenta / Green"), _ => tl!("Yellow / Blue") };
                        let mut percent = value * 100.0;
                        if ui.add_enabled(editable, egui::DragValue::new(&mut percent).speed(0.25).range(-100.0..=100.0).suffix("%").prefix(format!("{label}: "))).changed()
                            && let Some(wire) = color_balance_patch(ranges, range_index, axis_index, percent / 100.0)
                        {
                            run_checked(app, &document, revision, "xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"config","value":wire}]}));
                        }
                    }
                }
            }
            let alien_hsl = node.operation.name == "gegl:alien-map" && node.arguments.iter().any(|argument| {
                argument.name == "color-model" && matches!(&argument.value, FilterValue::Enum { domain, value: 1 } if domain == "gegl:alien-map/color-model")
            });
            for argument in node.arguments.iter().take(64) {
                let fallback = argument.name.chars().take(64).collect::<String>();
                let channel_label = match argument.name.as_str() {
                    "rr-gain" => Some(format!("{} / {}", tl!("Red"), tl!("Red"))),
                    "rg-gain" => Some(format!("{} / {}", tl!("Red"), tl!("Green"))),
                    "rb-gain" => Some(format!("{} / {}", tl!("Red"), tl!("Blue"))),
                    "gr-gain" => Some(format!("{} / {}", tl!("Green"), tl!("Red"))),
                    "gg-gain" => Some(format!("{} / {}", tl!("Green"), tl!("Green"))),
                    "gb-gain" => Some(format!("{} / {}", tl!("Green"), tl!("Blue"))),
                    "br-gain" => Some(format!("{} / {}", tl!("Blue"), tl!("Red"))),
                    "bg-gain" => Some(format!("{} / {}", tl!("Blue"), tl!("Green"))),
                    "bb-gain" => Some(format!("{} / {}", tl!("Blue"), tl!("Blue"))),
                    _ => None,
                };
                let label = match argument.name.as_str() {
                    "component" if component_extract => tl!("Component"),
                    "threshold" if red_eye => tl!("Threshold"),
                    "invert" if component_extract => tl!("Invert"),
                    "linear" if component_extract => tl!("Linear"),
                    "color" if color_to_alpha => tl!("Color"),
                    "value" if color_overlay => tl!("Color"),
                    "transparency-threshold" if color_to_alpha => tl!("Transparency threshold"),
                    "opacity-threshold" if color_to_alpha => tl!("Opacity threshold"),
                    "from-color" if color_exchange => tl!("From color"),
                    "to-color" if color_exchange => tl!("To color"),
                    "red-threshold" if color_exchange => tl!("Red threshold"),
                    "green-threshold" if color_exchange => tl!("Green threshold"),
                    "blue-threshold" if color_exchange => tl!("Blue threshold"),
                    "cpn-1-frequency" if node.operation.name == "gegl:alien-map" => if alien_hsl { tl!("Hue frequency") } else { tl!("Red frequency") },
                    "cpn-2-frequency" if node.operation.name == "gegl:alien-map" => if alien_hsl { tl!("Saturation frequency") } else { tl!("Green frequency") },
                    "cpn-3-frequency" if node.operation.name == "gegl:alien-map" => if alien_hsl { tl!("Lightness frequency") } else { tl!("Blue frequency") },
                    "cpn-1-phaseshift" if node.operation.name == "gegl:alien-map" => if alien_hsl { tl!("Hue phase shift") } else { tl!("Red phase shift") },
                    "cpn-2-phaseshift" if node.operation.name == "gegl:alien-map" => if alien_hsl { tl!("Saturation phase shift") } else { tl!("Green phase shift") },
                    "cpn-3-phaseshift" if node.operation.name == "gegl:alien-map" => if alien_hsl { tl!("Lightness phase shift") } else { tl!("Blue phase shift") },
                    "cpn-1-keep" if node.operation.name == "gegl:alien-map" => if alien_hsl { tl!("Keep hue") } else { tl!("Keep red") },
                    "cpn-2-keep" if node.operation.name == "gegl:alien-map" => if alien_hsl { tl!("Keep saturation") } else { tl!("Keep green") },
                    "cpn-3-keep" if node.operation.name == "gegl:alien-map" => if alien_hsl { tl!("Keep lightness") } else { tl!("Keep blue") },
                    "scale" if node.operation.name == "gegl:sepia" => tl!("Strength"),
                    "srgb" if node.operation.name == "gegl:sepia" || color_overlay => tl!("Process in sRGB"),
                    "clip-low" => tl!("Clip low"),
                    "clip-high" => tl!("Clip high"),
                    "low-limit" => tl!("Lower limit"),
                    "high-limit" => tl!("Upper limit"),
                    "red" => tl!("Red weight"),
                    "green" => tl!("Green weight"),
                    "blue" => tl!("Blue weight"),
                    "preserve-luminosity" => tl!("Preserve luminosity"),
                    "value" if node.operation.name == "gimp:threshold-alpha" => tl!("Alpha threshold"),
                    "original-temperature" => tl!("Original temperature"),
                    "intended-temperature" => tl!("Intended temperature"),
                    _ => fallback.as_str(),
                };
                let label = if channel_mixer { channel_label.as_deref().unwrap_or(label) } else { label };
                let changed = match argument.value {
                    FilterValue::Graph(_) => {
                        graph_inspect::properties(app, ui, &document, revision, node.id, &argument.name);
                        None
                    }
                    FilterValue::Color { .. } if color_overlay && argument.name == "value" => {
                        let mut changed = None;
                        ui.add_enabled_ui(editable, |ui| {
                            ui.horizontal(|ui| {
                                let label_response = ui.label(label);
                                changed = color_overlay::edit(ui, &argument.value, label_response.id, label);
                            });
                        });
                        changed
                    }
                    FilterValue::Color { .. } if color_exchange && matches!(argument.name.as_str(), "from-color" | "to-color") || color_to_alpha && argument.name == "color" => {
                        let mut changed = None;
                        ui.add_enabled_ui(editable, |ui| {
                            ui.horizontal(|ui| {
                                let label_response = ui.label(label);
                                changed = literal_color::edit(ui, &argument.value, label_response.id, label);
                            });
                        });
                        changed
                    }
                    FilterValue::Enum { ref domain, value }
                        if component_extract && argument.name == "component" && component_extract::allowed_component(&node.operation.schema_epoch, domain, value) =>
                    {
                        let choices: &[(i32, &str)] = match node.operation.schema_epoch.as_str() {
                            component_extract::EPOCH => &[(0, tl!("Red")), (1, tl!("Green")), (2, tl!("Blue")), (20, tl!("Alpha"))],
                            component_extract::CMYK_EPOCH => &[(8, tl!("Cyan")), (9, tl!("Magenta")), (10, tl!("Yellow")), (11, tl!("Black"))],
                            component_extract::YCBCR_EPOCH => &[(12, tl!("Y'CbCr Y'")), (13, tl!("Y'CbCr Cb")), (14, tl!("Y'CbCr Cr"))],
                            component_extract::GEOMETRY_EPOCH => &[(0, tl!("Red")), (1, tl!("Green")), (2, tl!("Blue")), (8, tl!("Cyan")), (9, tl!("Magenta")), (10, tl!("Yellow")), (11, tl!("Black")), (12, tl!("Y'CbCr Y'")), (13, tl!("Y'CbCr Cb")), (14, tl!("Y'CbCr Cr")), (20, tl!("Alpha"))],
                            _ => &[],
                        };
                        let selected = choices.iter().find_map(|(choice, label)| (*choice == value).then_some(*label)).unwrap_or("");
                        let mut current = value;
                        ui.add_enabled_ui(editable, |ui| {
                            egui::ComboBox::from_label(tl!("Component")).selected_text(selected).show_ui(ui, |ui| {
                                for (choice, label) in choices { ui.selectable_value(&mut current, *choice, *label); }
                            });
                        });
                        (current != value).then(|| FilterValue::Enum { domain: domain.clone(), value: current })
                    }
                    FilterValue::Enum { ref domain, value }
                        if node.operation.name == "gegl:saturation"
                            && argument.name == "colorspace"
                            && domain == "GeglSaturationType"
                            && (0..=2).contains(&value) =>
                    {
                        let choices = [(0, tl!("Native")), (1, tl!("CIE Lab")), (2, tl!("CIE Yuv"))];
                        let selected = choices.iter().find_map(|(choice, label)| (*choice == value).then_some(*label)).unwrap_or("");
                        let yuv = document.mode == photocraft_doc::ColorMode::Grayscale
                            && node.operation.schema_epoch == "xcf-22-stock-gray-saturation-precision-v1"
                            || document.mode == photocraft_doc::ColorMode::Rgb
                                && matches!(document.depth, photocraft_color::SampleType::U16 | photocraft_color::SampleType::F32)
                                && matches!(node.operation.schema_epoch.as_str(),
                                    "xcf-22-stock-saturation-precision-v1" | "xcf-22-stock-rgb-yuv-saturation-precision-v1");
                        let mut current = value;
                        ui.add_enabled_ui(editable, |ui| {
                            egui::ComboBox::from_label(tl!("Saturation colorspace")).selected_text(selected).show_ui(ui, |ui| {
                                for (choice, label) in choices {
                                    ui.add_enabled_ui(choice != 2 || yuv, |ui| {
                                        ui.selectable_value(&mut current, choice, label);
                                    });
                                }
                            });
                        });
                        (current != value).then(|| FilterValue::Enum { domain: domain.clone(), value: current })
                    }
                    FilterValue::Enum { ref domain, value }
                        if node.operation.name == "gimp:threshold"
                            && argument.name == "channel"
                            && domain == "GimpHistogramChannel"
                            && (0..=5).contains(&value) =>
                    {
                        let choices = [(0, tl!("Value")), (1, tl!("Red")), (2, tl!("Green")), (3, tl!("Blue")), (4, tl!("Alpha")), (5, tl!("Luminance"))];
                        let selected = choices.iter().find_map(|(choice, label)| (*choice == value).then_some(*label)).unwrap_or("");
                        let mut current = value;
                        ui.add_enabled_ui(editable, |ui| {
                            egui::ComboBox::from_label(tl!("Histogram channel")).selected_text(selected).show_ui(ui, |ui| {
                                for (choice, label) in choices {
                                    ui.selectable_value(&mut current, choice, label);
                                }
                            });
                        });
                        (current != value).then(|| FilterValue::Enum { domain: domain.clone(), value: current })
                    }
                    FilterValue::Enum { ref domain, value }
                        if node.operation.name == "gimp:desaturate"
                            && argument.name == "mode"
                            && domain == "GimpDesaturateMode"
                            && (0..=4).contains(&value) =>
                    {
                        let choices = [(0, tl!("Lightness")), (1, tl!("Luma")), (2, tl!("Average")), (3, tl!("Luminance")), (4, tl!("Value"))];
                        let selected = choices.iter().find_map(|(choice, label)| (*choice == value).then_some(*label)).unwrap_or("");
                        let mut current = value;
                        ui.add_enabled_ui(editable, |ui| {
                            egui::ComboBox::from_label(tl!("Desaturate mode")).selected_text(selected).show_ui(ui, |ui| {
                                for (choice, label) in choices {
                                    ui.selectable_value(&mut current, choice, label);
                                }
                            });
                        });
                        (current != value).then(|| FilterValue::Enum { domain: domain.clone(), value: current })
                    }
                    FilterValue::Bool(value) => {
                        let mut current = value;
                        ui.add_enabled(editable, egui::Checkbox::new(&mut current, label)).changed().then_some(FilterValue::Bool(current))
                    }
                    FilterValue::Number(ref number) if channel_mixer => {
                        if let Some(value) = channel_mixer::edit_gain(ui, label, editable, number) {
                            run_checked(app, &document, revision, "xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":argument.name,"value":value}]}));
                        }
                        None
                    }
                    FilterValue::Number(ref number) if mono_signed && matches!(argument.name.as_str(), "red" | "green" | "blue") => {
                        if let Some(value) = mono_mixer_signed::edit_weight(ui, label, editable, number) {
                            run_checked(app, &document, revision, "xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":argument.name,"value":value}]}));
                        }
                        None
                    }
                    FilterValue::Number(ExactNumber::Float32Bits(bits)) if red_eye => red_eye::edit(ui, label, editable, bits),
                    FilterValue::Number(ExactNumber::Float32Bits(bits)) if f32::from_bits(bits).is_finite() => {
                        let mut current = f32::from_bits(bits);
                        let changed = if literal_colors || alpha_clip {
                            ui.horizontal(|ui| {
                                let label = ui.label(label);
                                let drag = egui::DragValue::new(&mut current).speed(0.01);
                                let drag = if alpha_clip_finite { drag } else { drag.range(0.0..=1.0) };
                                ui.add_enabled(editable, drag).labelled_by(label.id).changed()
                            }).inner
                        } else {
                            ui.add_enabled(editable, egui::DragValue::new(&mut current).speed(0.01).prefix(format!("{label}: "))).changed()
                        };
                        changed.then_some(FilterValue::Number(ExactNumber::Float32Bits(current.to_bits())))
                    }
                    _ => None,
                };
                if let Some(value) = changed {
                    // Keep the stored scalar type. Engine schema/range checks and
                    // authoritative rendering decide whether the edit can commit.
                    run_checked(app, &document, revision, "xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":argument.name,"value":value}]}));
                }
            }
            if !editable {
                let message = if !unlocked { tl!("Layer content is locked") }
                    else if assigned && assigned_box::candidate(&document, layer, node) { tl!("Filter parameters are read-only") }
                    else { tl!("Filter editing is unavailable") };
                ui.label(egui::RichText::new(message).color(tokens.text_dim));
            }
        });
    }
    if stack.nodes.len() > 64 {
        ui.label(egui::RichText::new(tl!("Additional filters are available through command inspection")).color(tokens.text_dim));
    }
}

fn color_balance_patch(ranges: &[FilterArgument], selected_range: usize, selected_axis: usize, value: f64) -> Option<serde_json::Value> {
    if ranges.len() != 3 || selected_range >= 3 || selected_axis >= 3 || !value.is_finite() || !(-1.0..=1.0).contains(&value) {
        return None;
    }
    let mut output = Vec::new();
    for (range_index, (range, name)) in ranges.iter().zip(["shadows", "midtones", "highlights"]).enumerate() {
        let FilterValue::Config(fields) = &range.value else { return None };
        if range.name != name || fields.len() != 3 {
            return None;
        }
        let mut decoded = Vec::new();
        for (axis_index, (field, name)) in fields.iter().zip(["cyan-red", "magenta-green", "yellow-blue"]).enumerate() {
            let FilterValue::Number(ExactNumber::Float64Bits(bits)) = field.value else { return None };
            if field.name != name || !f64::from_bits(bits).is_finite() || !(-1.0..=1.0).contains(&f64::from_bits(bits)) {
                return None;
            }
            let bits = if (selected_range, selected_axis) == (range_index, axis_index) { value.to_bits() } else { bits };
            decoded.push(json!({"name":name,"value":{"Number":{"Float64Bits":format!("0x{bits:016x}")}}}));
        }
        output.push(json!({"name":name,"value":{"Config":decoded}}));
    }
    Some(json!({"Config":output}))
}

#[cfg(test)]
mod alpha_clip_finite_tests;
#[cfg(test)]
mod alpha_clip_tests;
#[cfg(test)]
mod color_exchange_tests;
#[cfg(test)]
mod color_overlay_tests;
#[cfg(test)]
mod color_to_alpha_tests;
#[cfg(test)]
mod component_extract_extended_tests;
#[cfg(test)]
mod component_extract_tests;
#[cfg(test)]
mod component_geometry_tests;
#[cfg(test)]
mod mono_mixer_signed_tests;
#[cfg(test)]
mod multiline_tests;
#[cfg(test)]
mod red_eye_tests;
#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

    #[test]
    fn rejected_property_action_surfaces_error_without_document_or_history_change() {
        let document = photocraft_doc::Document::new("Own rejected action", photocraft_doc::Size::new(2, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let mut session = photocraft_engine::Session::new();
        session.open_document(document, None).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.entry_labels().count();
        let revision = app.session.active().unwrap().revision;
        run_checked(&mut app, &before, revision, "xcf.filter.enable", json!({"filterId":"0","enabled":true}));
        assert!(app.ui.status_error);
        assert!(!app.ui.status.is_empty());
        assert_eq!(app.session.active().unwrap().doc, before);
        assert_eq!(app.session.active().unwrap().history.entry_labels().count(), history);
    }

    #[test]
    fn property_target_guard_refuses_replaced_snapshot_even_with_equal_revision() {
        let mut session = photocraft_engine::Session::new();
        session.open_document(photocraft_doc::Document::new("Original", photocraft_doc::Size::new(2, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8), None).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        let before = app.session.active().unwrap().doc.clone();
        app.session.edit("Own intermediate edit", |document, _| { document.name = "Replaced".into(); Ok(()) }).unwrap();
        let installed = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let count = app.session.active().unwrap().history.entry_labels().count();
        run_checked(&mut app, &before, revision, "document.rename", json!({"name":"Wrong target"}));
        assert_eq!(app.ui.status, "xcf.properties.stale-target");
        assert_eq!(app.session.active().unwrap().doc, installed);
        assert_eq!(app.session.active().unwrap().history.entry_labels().count(), count);
    }

    #[test]
    #[ignore = "requires independent exact-font color originals"]
    fn text_color_picker_maps_declared_encoding_and_preserves_stored_alpha() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("text-color-evidence");
        let bytes = std::fs::read(root.join("aa-16-primary-source.xcf")).unwrap();
        let original = photocraft_io::import("text.xcf", &bytes).unwrap().document;
        let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join("aa-16-primary-edit.xcf")).unwrap()).unwrap().document;
        let layer = original.layers[0].id;
        let color = photocraft_engine::xcf_text_cmds::current_text_color(&original, layer).unwrap();
        let target = photocraft_engine::xcf_text_cmds::current_text_color(&reference, reference.layers[0].id).unwrap();
        let target_rgb = text_color_rgb(&reference, target).unwrap();
        let patch = text_color_patch(&original, layer, color, target_rgb).unwrap();
        assert_eq!(patch["rgbaF32Bits"][3], format!("{:08x}", color.rgba_f32_bits[3]));
        assert!(text_color_patch(&original, layer, color, [f32::NAN, 0.0, 0.0]).is_none());
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.run("xcf.text.color.set", patch).unwrap();
        let edited = &app.session.active().unwrap().doc;
        assert_eq!(edited.layers, original.layers);
        assert_eq!(edited.xcf_state.as_ref().unwrap().opaque, original.xcf_state.as_ref().unwrap().opaque);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap(),
            photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap()
        );
        let saved = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), **edited);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(*app.session.active().unwrap().doc, original);
        let alpha_doc = photocraft_io::import("alpha.xcf", &std::fs::read(root.join("alpha-only/aa-40-alpha-only-edit.xcf")).unwrap()).unwrap().document;
        let alpha_layer = alpha_doc.layers[0].id;
        let alpha = photocraft_engine::xcf_text_cmds::current_text_color(&alpha_doc, alpha_layer).unwrap();
        assert_eq!(alpha.rgba_f32_bits[3], 0.25f32.to_bits());
        assert_eq!(text_color_patch(&alpha_doc, alpha_layer, alpha, [0.0, 1.0, 0.0]).unwrap()["rgbaF32Bits"][3], "3e800000");
        let fractional_doc = photocraft_io::import("fractional.xcf", &std::fs::read(root.join("aa-32-fractional-source.xcf")).unwrap()).unwrap().document;
        let fractional = photocraft_engine::xcf_text_cmds::current_text_color(&fractional_doc, fractional_doc.layers[0].id).unwrap();
        assert_eq!(fractional.encoding, photocraft_text::xcf::TextColorEncoding::Linear);
        let picker = text_color_rgb(&fractional_doc, fractional).unwrap();
        for (actual, expected) in picker.iter().zip(fractional.rgba_f32_bits.iter().take(3)) {
            assert!((*actual - f32::from_bits(*expected)).abs() < 2e-6, "picker floats must remain linear");
        }
        let state = app.session.active_mut().unwrap();
        state.doc.try_resident_mut().unwrap().layers[0].locks.pixels = true;
        let mut harness = Harness::builder().with_size(egui::vec2(420.0, 360.0)).build_ui_state(move |ui, app| text_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        harness.get_by_label("Text color");
        let doc = &harness.state().session.active().unwrap().doc;
        assert!(photocraft_engine::xcf_text_cmds::current_text_color(doc, layer).is_some());
        assert!(!photocraft_engine::xcf_text_cmds::color_edit_candidate(doc, layer));
        assert_eq!(std::fs::read(root.join("aa-16-primary-source.xcf")).unwrap(), bytes);
    }

    #[test]
    #[ignore = "requires local unpublished independent exact-font text originals"]
    fn text_content_control_preserves_cache_and_uses_checked_apply_history() {
        text_content_case("text-stem-evidence", "simple/source.xcf", "simple/edit.xcf");
    }
    #[test]
    #[ignore = "requires local unpublished independent antialiased exact-font text originals"]
    fn antialiased_text_content_control_preserves_cache_native_and_history() {
        text_content_case("text-rectilinear-aa-evidence", "shape-source.xcf", "shape-edit.xcf");
        text_content_case("text-rectilinear-aa-evidence", "kern-source.xcf", "kern-edit.xcf");
    }

    #[test]
    #[ignore = "requires local unpublished independent antialiased font-size originals"]
    fn text_font_size_control_matches_independent_edit_without_replacing_original_cache() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("text-size-evidence");
        let path = root.join("aa-size-source.xcf");
        let bytes = std::fs::read(&path).unwrap();
        let original = photocraft_io::import("text.xcf", &bytes).unwrap().document;
        let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join("aa-size-edit.xcf")).unwrap()).unwrap().document;
        let layer = original.layers[0].id;
        assert!(photocraft_engine::xcf_text_cmds::font_size_edit_candidate(&original, layer));
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(360.0, 460.0)).build_ui_state(move |ui, app| text_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        let source_layout = harness.state_mut().cached_layer_bounds(&original.layers[0]).unwrap();
        assert_eq!(source_layout, photocraft_geom::Rect::new(8, 4, 49, 24));
        harness.query_all_by_role(egui::accesskit::Role::ComboBox).next().expect("font-size control").click();
        harness.run_steps(2);
        harness.get_by_label("24 px").click();
        harness.run_steps(2);
        let current_layout = harness.state_mut().cached_layer_bounds(&original.layers[0]).unwrap();
        assert_eq!(current_layout, photocraft_geom::Rect::new(8, 4, 69, 34));
        let active = harness.state().session.active().unwrap();
        assert_eq!(active.history.past_len(), 1);
        let edited = &active.doc;
        let current = edited.xcf_state.as_ref().unwrap();
        let source = original.xcf_state.as_ref().unwrap();
        assert_eq!(current.source_items[0].source, reference.xcf_state.as_ref().unwrap().source_items[0].source);
        assert_eq!(current.source_items[0].cache, source.source_items[0].cache);
        assert_eq!(current.resources, source.resources);
        assert_eq!(current.opaque, source.opaque);
        assert_eq!(edited.layers, original.layers);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap(),
            photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap()
        );
        let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        harness.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(photocraft_engine::xcf_text_cmds::current_font_size(&harness.state().session.active().unwrap().doc, layer), Some(24));
        let current = harness.state_mut().session.active_mut().unwrap();
        current
            .doc
            .try_resident_mut()
            .unwrap()
            .xcf_state
            .as_mut()
            .unwrap()
            .items
            .iter_mut()
            .find(|item| item.item == ItemRef::Layer(layer))
            .unwrap()
            .locks
            .position = true;
        harness.run_steps(2);
        assert!(harness.query_all_by_role(egui::accesskit::Role::ComboBox).next().expect("font-size control").accesskit_node().is_disabled());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    #[test]
    #[ignore = "requires independent canonical letter-spacing originals"]
    fn text_letter_spacing_control_preserves_source_and_updates_current_geometry() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("text-spacing-evidence");
        let path = root.join("aa-32-HEFT-source.xcf");
        let bytes = std::fs::read(&path).unwrap();
        let original = photocraft_io::import("text.xcf", &bytes).unwrap().document;
        let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join("aa-32-HEFT-edit.xcf")).unwrap()).unwrap().document;
        let layer = original.layers[0].id;
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(420.0, 540.0)).build_ui_state(move |ui, app| text_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        harness.query_all_by_role(egui::accesskit::Role::ComboBox).nth(1).expect("letter-spacing control").click();
        harness.run_steps(2);
        harness.get_by_label("4 px").click();
        harness.run_steps(2);
        let active = harness.state().session.active().unwrap();
        assert_eq!(active.history.past_len(), 1);
        let edited = &active.doc;
        assert_eq!(edited.xcf_state.as_ref().unwrap().source_items[0].source, reference.xcf_state.as_ref().unwrap().source_items[0].source);
        assert_eq!(edited.layers, original.layers);
        assert_eq!(edited.xcf_state.as_ref().unwrap().source_items[0].cache, original.xcf_state.as_ref().unwrap().source_items[0].cache);
        assert_eq!(edited.xcf_state.as_ref().unwrap().resources, original.xcf_state.as_ref().unwrap().resources);
        assert_eq!(edited.xcf_state.as_ref().unwrap().opaque, original.xcf_state.as_ref().unwrap().opaque);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap(),
            photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap()
        );
        let expected = photocraft_engine::transform_cmds::transform_bounds(&reference, &reference.layers[0]);
        assert_eq!(harness.state_mut().cached_layer_bounds(&original.layers[0]), Some(expected));
        let edited = &harness.state().session.active().unwrap().doc;
        let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
        harness.query_all_by_role(egui::accesskit::Role::ComboBox).next().unwrap().click();
        harness.run_steps(2);
        assert!(harness.query_by_label("24 px").is_none(), "unqualified spaced size must not be offered");
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        let active = harness.state_mut().session.active_mut().unwrap();
        active.doc.try_resident_mut().unwrap().layers[0].locks.pixels = true;
        active.revision += 1;
        harness.run_steps(2);
        assert!(harness.query_all_by_role(egui::accesskit::Role::ComboBox).nth(1).unwrap().accesskit_node().is_disabled());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    #[test]
    #[ignore = "requires independent canonical zero-spacing originals"]
    fn zero_letter_spacing_controls_follow_independent_edits_and_remove_the_field() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("text-spacing-evidence");
        let path = root.join("aa-32-HEFT-control.xcf");
        let bytes = std::fs::read(&path).unwrap();
        let original = photocraft_io::import("zero.xcf", &bytes).unwrap().document;
        let layer = original.layers[0].id;
        assert_eq!(photocraft_engine::xcf_text_cmds::current_letter_spacing(&original, layer), Some(0.));
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(420.0, 540.0)).build_ui_state(move |ui, app| text_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        for (step, (spacing, file)) in
            [(2, "aa-32-HEFT-source.xcf"), (0, "aa-32-HEFT-control.xcf"), (4, "aa-32-HEFT-edit.xcf"), (0, "aa-32-HEFT-control.xcf")].into_iter().enumerate()
        {
            harness.query_all_by_role(egui::accesskit::Role::ComboBox).nth(1).expect("zero-spacing control").click();
            harness.run_steps(2);
            harness.get_by_label(&format!("{spacing} px")).click();
            harness.run_steps(2);
            let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join(file)).unwrap()).unwrap().document;
            let active = harness.state().session.active().unwrap();
            assert_eq!(active.history.past_len(), step + 1);
            let edited = &active.doc;
            assert_eq!(edited.xcf_state.as_ref().unwrap().source_items[0].source, reference.xcf_state.as_ref().unwrap().source_items[0].source);
            assert_eq!(edited.layers, original.layers);
            assert_eq!(edited.xcf_state.as_ref().unwrap().resources, original.xcf_state.as_ref().unwrap().resources);
            assert_eq!(edited.xcf_state.as_ref().unwrap().opaque, original.xcf_state.as_ref().unwrap().opaque);
            assert_eq!(edited.xcf_state.as_ref().unwrap().source_items[0].cache, original.xcf_state.as_ref().unwrap().source_items[0].cache);
            assert_eq!(
                photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap(),
                photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap()
            );
            let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
        }
        for _ in 0..4 {
            harness.state_mut().run("edit.undo", json!({})).unwrap();
        }
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    #[test]
    #[ignore = "requires independent canonical text placement originals"]
    fn position_controls_move_current_text_and_preserve_original_cache() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
        let path = root.join("text-spacing-evidence/none-32-HEFT-control.xcf");
        let bytes = std::fs::read(&path).unwrap();
        let original = photocraft_io::import("text.xcf", &bytes).unwrap().document;
        let layer = original.layers[0].id;
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(420.0, 600.0)).build_ui_state(move |ui, app| text_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        for (index, key, repeats) in [(0, egui::Key::ArrowUp, 3), (1, egui::Key::ArrowDown, 2)] {
            harness.query_all_by_role(egui::accesskit::Role::SpinButton).nth(index).expect("current text origin control").click();
            harness.run_steps(2);
            for _ in 0..repeats {
                harness.key_press(key);
                harness.run_steps(2);
            }
        }
        let active = harness.state().session.active().unwrap();
        let edited = &active.doc;
        assert_eq!(photocraft_engine::xcf_text_cmds::current_text_origin(edited, layer), Some([11, 2]));
        assert_eq!(edited.layers, original.layers);
        let before = original.xcf_state.as_ref().unwrap();
        let after = edited.xcf_state.as_ref().unwrap();
        assert_eq!(after.items, before.items);
        assert_eq!(after.resources, before.resources);
        assert_eq!(after.opaque, before.opaque);
        assert_eq!(after.source_items[0].cache, before.source_items[0].cache);
        let actual = photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap();
        let expected = std::fs::read(root.join("text-placement-evidence/none-32-HEFT-control-shift.projection.bin")).unwrap();
        let rendered: Vec<_> = actual.px.iter().flat_map(|p| p.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8)).collect();
        assert_eq!(rendered, expected);
        let saved = photocraft_io::export(edited, "placed.pcraft", &Default::default()).unwrap();
        assert_eq!(photocraft_io::import("placed.pcraft", &saved.bytes).unwrap().document, **edited);
        let expected_bounds = photocraft_geom::Rect::new(11, 2, 94, 41);
        assert_eq!(harness.state_mut().cached_layer_bounds(&original.layers[0]), Some(expected_bounds));
        let steps = harness.state().session.active().unwrap().history.past_len();
        assert!(steps > 0);
        for _ in 0..steps {
            harness.state_mut().run("edit.undo", json!({})).unwrap();
        }
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        harness.state_mut().run("layer.setProps", json!({"layer":layer.0,"locks":{"position":true}})).unwrap();
        harness.run_steps(2);
        assert!(harness.query_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap().accesskit_node().is_disabled());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    fn text_content_case(packet: &str, source_file: &str, reference_file: &str) {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(packet);
        let bytes = std::fs::read(root.join(source_file)).unwrap();
        let original = photocraft_io::import("text.xcf", &bytes).unwrap().document;
        let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join(reference_file)).unwrap()).unwrap().document;
        let photocraft_doc::xcf_items::EditableSource::Text { content: ref proposed, .. } = reference.xcf_state.as_ref().unwrap().source_items[0].source else {
            panic!("independent text reference")
        };
        let layer = original.layers[0].id;
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(420.0, 500.0)).build_ui_state(move |ui, app| text_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        assert!(!harness.get_by_role(egui::accesskit::Role::TextInput).accesskit_node().is_disabled(), "checked current text candidate must be enabled");
        harness.get_by_role(egui::accesskit::Role::TextInput).click();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_role(egui::accesskit::Role::TextInput).type_text(proposed);
        harness.run_steps(2);
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
        harness.get_by_label("Apply Text").click();
        harness.run_steps(2);
        let state = harness.state().session.active().unwrap();
        assert_eq!(state.history.past_len(), 1);
        let edited = &state.doc;
        assert_eq!(edited.layers, original.layers);
        assert_eq!(edited.xcf_state.as_ref().unwrap().resources, original.xcf_state.as_ref().unwrap().resources);
        assert_eq!(edited.xcf_state.as_ref().unwrap().source_items[0].cache, original.xcf_state.as_ref().unwrap().source_items[0].cache);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap(),
            photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap()
        );
        let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        harness.run_steps(2);
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        harness.get_by_role(egui::accesskit::Role::TextInput).click();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_role(egui::accesskit::Role::TextInput).type_text("A");
        harness.run_steps(2);
        harness.get_by_label("Apply Text").click();
        harness.run_steps(2);
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
        let state = harness.state_mut().session.active_mut().unwrap();
        state
            .doc
            .try_resident_mut()
            .unwrap()
            .xcf_state
            .as_mut()
            .unwrap()
            .items
            .iter_mut()
            .find(|item| item.item == ItemRef::Layer(layer))
            .unwrap()
            .locks
            .content = true;
        harness.run_steps(2);
        assert!(harness.get_by_role(egui::accesskit::Role::TextInput).accesskit_node().is_disabled());
        assert!(harness.get_by_label("Apply Text").accesskit_node().is_disabled());
        assert_eq!(std::fs::read(root.join(source_file)).unwrap(), bytes);
    }

    #[test]
    #[ignore = "requires local unpublished independent Desaturate originals"]
    fn desaturate_mode_control_uses_typed_checked_history_and_source_preservation() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("desaturate-precision-evidence");
        let bytes = std::fs::read(root.join("rgb-float_non_linear.lightness.xcf")).unwrap();
        let original = photocraft_io::import("desaturate.xcf", &bytes).unwrap().document;
        let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join("rgb-float_non_linear.average.xcf")).unwrap()).unwrap().document;
        let layer = original.layers[0].id;
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(420.0, 500.0)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        harness.query_all_by_role(egui::accesskit::Role::ComboBox).next().expect("Desaturate mode selector").click();
        harness.run_steps(2);
        harness.get_by_label("Average").click();
        harness.run_steps(2);
        let state = harness.state().session.active().unwrap();
        let edited = &state.doc;
        assert_eq!(state.history.past_len(), 1);
        assert_eq!(edited.layers, original.layers);
        assert_eq!(edited.source_color_context, original.source_color_context);
        let current = edited.xcf_state.as_ref().unwrap();
        let source = original.xcf_state.as_ref().unwrap();
        assert_eq!(current.opaque, source.opaque);
        assert_eq!(current.filter_masks, source.filter_masks);
        assert_eq!(current.filter_stacks[0].nodes[0].arguments, reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap(),
            photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap()
        );
        let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        let state = harness.state_mut().session.active_mut().unwrap();
        let doc = state.doc.try_resident_mut().unwrap();
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(layer)).unwrap().locks.content = true;
        harness.run_steps(2);
        assert!(harness.query_all_by_role(egui::accesskit::Role::ComboBox).next().unwrap().accesskit_node().is_disabled());
        assert_eq!(std::fs::read(root.join("rgb-float_non_linear.lightness.xcf")).unwrap(), bytes);
    }

    #[test]
    #[ignore = "requires local unpublished independent Saturation originals"]
    fn saturation_colorspace_control_uses_typed_checked_history_and_source_preservation() {
        saturation_selector_case("rgb-float_non_linear", "cielab", "CIE Lab", false);
        saturation_selector_case("rgb-float_non_linear", "cieyuv", "CIE Yuv", false);
        saturation_selector_case("rgb-u16_non_linear", "cieyuv", "CIE Yuv", false);
        saturation_selector_from("rgb-float_non_linear", "cieyuv", "cielab", "CIE Lab", false);
        saturation_selector_case("rgb-double_non_linear", "cielab", "CIE Lab", true);
        saturation_selector_case("gray-float_non_linear", "cieyuv", "CIE Yuv", false);
    }
    fn saturation_selector_case(case: &str, target: &str, label: &str, yuv_disabled: bool) {
        saturation_selector_from(case, "native", target, label, yuv_disabled);
    }
    fn saturation_selector_from(case: &str, source_choice: &str, target: &str, label: &str, yuv_disabled: bool) {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("saturation-precision-evidence");
        let bytes = std::fs::read(root.join(format!("{case}.{source_choice}-half.xcf"))).unwrap();
        let original = photocraft_io::import("desaturate.xcf", &bytes).unwrap().document;
        let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join(format!("{case}.{target}-half.xcf"))).unwrap()).unwrap().document;
        let layer = original.layers[0].id;
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(420.0, 500.0)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        harness.query_all_by_role(egui::accesskit::Role::ComboBox).next().expect("Saturation colorspace selector").click();
        harness.run_steps(2);
        assert_eq!(harness.get_by_label("CIE Yuv").accesskit_node().is_disabled(), yuv_disabled);
        harness.get_by_label(label).click();
        harness.run_steps(2);
        let state = harness.state().session.active().unwrap();
        let edited = &state.doc;
        assert_eq!(state.history.past_len(), 1);
        assert_eq!(edited.layers, original.layers);
        assert_eq!(edited.source_color_context, original.source_color_context);
        let current = edited.xcf_state.as_ref().unwrap();
        let source = original.xcf_state.as_ref().unwrap();
        assert_eq!(current.opaque, source.opaque);
        assert_eq!(current.filter_masks, source.filter_masks);
        assert_eq!(current.filter_stacks[0].nodes[0].arguments, reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap(),
            photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap()
        );
        let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        let state = harness.state_mut().session.active_mut().unwrap();
        let doc = state.doc.try_resident_mut().unwrap();
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(layer)).unwrap().locks.content = true;
        harness.run_steps(2);
        assert!(harness.query_all_by_role(egui::accesskit::Role::ComboBox).next().unwrap().accesskit_node().is_disabled());
        assert_eq!(std::fs::read(root.join(format!("{case}.{source_choice}-half.xcf"))).unwrap(), bytes);
    }

    #[test]
    #[ignore = "requires local unpublished independent Threshold originals"]
    fn threshold_channel_control_uses_typed_checked_history_and_source_preservation() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("threshold-precision-evidence");
        let bytes = std::fs::read(root.join("rgb-float_non_linear.channel0.xcf")).unwrap();
        let original = photocraft_io::import("threshold.xcf", &bytes).unwrap().document;
        let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join("rgb-float_non_linear.channel2.xcf")).unwrap()).unwrap().document;
        let layer = original.layers[0].id;
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(420.0, 500.0)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        harness.query_all_by_role(egui::accesskit::Role::ComboBox).next().expect("Threshold mode selector").click();
        harness.run_steps(2);
        harness.get_by_label("Green").click();
        harness.run_steps(2);
        let state = harness.state().session.active().unwrap();
        let edited = &state.doc;
        assert_eq!(state.history.past_len(), 1);
        assert_eq!(edited.layers, original.layers);
        assert_eq!(edited.source_color_context, original.source_color_context);
        let current = edited.xcf_state.as_ref().unwrap();
        let source = original.xcf_state.as_ref().unwrap();
        assert_eq!(current.opaque, source.opaque);
        assert_eq!(current.filter_masks, source.filter_masks);
        assert_eq!(current.filter_stacks[0].nodes[0].arguments, reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap(),
            photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap()
        );
        let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        let state = harness.state_mut().session.active_mut().unwrap();
        let doc = state.doc.try_resident_mut().unwrap();
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(layer)).unwrap().locks.content = true;
        harness.run_steps(2);
        assert!(harness.query_all_by_role(egui::accesskit::Role::ComboBox).next().unwrap().accesskit_node().is_disabled());
        assert_eq!(std::fs::read(root.join("rgb-float_non_linear.channel0.xcf")).unwrap(), bytes);
    }

    #[test]
    fn color_balance_axis_patch_preserves_other_exact_values_and_rejects_bad_shapes() {
        let ranges: Vec<_> = ["shadows", "midtones", "highlights"]
            .into_iter()
            .map(|name| FilterArgument {
                name: name.into(),
                value: FilterValue::Config(
                    ["cyan-red", "magenta-green", "yellow-blue"]
                        .into_iter()
                        .map(|name| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits((-0.0_f64).to_bits())) })
                        .collect(),
                ),
            })
            .collect();
        let patched = color_balance_patch(&ranges, 1, 2, 0.125).unwrap();
        for range in 0..3 {
            for axis in 0..3 {
                let expected = if (range, axis) == (1, 2) { "0x3fc0000000000000" } else { "0x8000000000000000" };
                assert_eq!(patched["Config"][range]["value"]["Config"][axis]["value"]["Number"]["Float64Bits"], expected);
            }
        }
        for (range, axis, value) in [(3, 0, 0.0), (0, 3, 0.0), (0, 0, f64::NAN), (0, 0, 1.001)] {
            assert!(color_balance_patch(&ranges, range, axis, value).is_none());
        }
        let mut malformed = ranges.clone();
        malformed[1].name = "shadows".into();
        assert!(color_balance_patch(&malformed, 0, 0, 0.0).is_none());
        assert!(color_balance_patch(&ranges[..2], 0, 0, 0.0).is_none());
    }

    #[test]
    #[ignore = "requires local unpublished independent Color Balance originals"]
    fn color_balance_luminosity_control_preserves_source_and_uses_checked_history() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("color-balance-evidence");
        let bytes = std::fs::read(root.join("rgb-float_non_linear.coupled.xcf")).unwrap();
        let original = photocraft_io::import("balance.xcf", &bytes).unwrap().document;
        let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join("rgb-float_non_linear.preserve.xcf")).unwrap()).unwrap().document;
        let layer = original.layers[0].id;
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(340.0, 760.0)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        harness.get_by_label("Preserve luminosity").click();
        harness.run_steps(2);
        let state = harness.state().session.active().unwrap();
        let edited = &state.doc;
        assert_eq!(state.history.past_len(), 1);
        assert_eq!(edited.layers, original.layers);
        assert_eq!(edited.source_color_context, original.source_color_context);
        assert_eq!(
            edited.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments,
            reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments
        );
        let actual = photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(actual, photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap());
        let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        assert_eq!(std::fs::read(root.join("rgb-float_non_linear.coupled.xcf")).unwrap(), bytes);
    }

    #[test]
    #[ignore = "requires reviewed unpublished GIMP Posterize original"]
    fn imported_filter_buttons_use_checked_history_and_preserve_source() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("operations/gimp-posterize");
        let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("probe.json")).unwrap()).unwrap();
        let path = root.join(proof["cases"][0]["input"].as_str().unwrap());
        let source = std::fs::read(&path).unwrap();
        let original = photocraft_io::import("posterize.xcf", &source).unwrap().document;
        let layer = original.layers[0].id;
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(700.0, 300.0)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        harness.get_by_label("Increase Levels").click();
        harness.run_steps(2);
        let doc = &harness.state().session.active().unwrap().doc;
        assert_eq!(
            doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[0].value,
            FilterValue::Number(ExactNumber::Signed { width: 32, value: 4 })
        );
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 1);
        assert_eq!(doc.layers[0].surface(), original.layers[0].surface());
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        harness.get_by_label("Remove Filter").click();
        harness.run_steps(2);
        assert!(harness.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks.iter().all(|stack| stack.nodes.is_empty()));
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        let state = harness.state_mut().session.active_mut().unwrap();
        let doc = state.doc.try_resident_mut().unwrap();
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(layer)).unwrap().locks.content = true;
        harness.run_steps(2);
        assert!(harness.get_by_label("Increase Levels").accesskit_node().is_disabled());
        assert!(harness.get_by_label("Remove Filter").accesskit_node().is_disabled());
        assert_eq!(std::fs::read(path).unwrap(), source);
    }

    #[test]
    #[ignore = "requires reviewed unpublished HDR RGB Clip original"]
    fn typed_clip_boolean_uses_shared_validation_history_and_exact_source() {
        boolean_filter_case("rgb-clip-hdr-evidence", "clip.source.xcf", "clip.low-disabled.xcf", "Clip low");
    }
    #[test]
    #[ignore = "requires local unpublished independent RGB Sepia originals"]
    fn sepia_encoding_checkbox_preserves_typed_native_source_and_history() {
        boolean_filter_case("sepia-precision-evidence", "rgb-float_non_linear.coupled.xcf", "rgb-float_non_linear.linear.xcf", "Process in sRGB");
    }
    #[test]
    #[ignore = "requires local unpublished independent RGB Alien Map originals"]
    fn alien_map_scalar_and_keep_controls_match_independent_edit_and_preserve_history() {
        alien_map_control_case("alien-map-precision-evidence", ["Keep red", "Keep blue"]);
    }
    #[test]
    #[ignore = "requires local unpublished independent HSL Alien Map originals"]
    fn alien_map_hsl_controls_match_independent_edit_and_preserve_history() {
        alien_map_control_case("alien-map-hsl-precision-evidence", ["Keep hue", "Keep lightness"]);
    }
    fn alien_map_control_case(packet: &str, keep_labels: [&str; 2]) {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(packet);
        let path = root.join("rgb-float_non_linear.source.xcf");
        let bytes = std::fs::read(&path).unwrap();
        let original = photocraft_io::import("alien-map.xcf", &bytes).unwrap().document;
        let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join("rgb-float_non_linear.kept.xcf")).unwrap()).unwrap().document;
        let layer = original.layers[0].id;
        let reference_arguments = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments;
        let values: Vec<_> = original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]
            .arguments
            .iter()
            .filter(|argument| matches!(argument.value, FilterValue::Number(ExactNumber::Float32Bits(_))))
            .map(|argument| {
                let expected = reference_arguments.iter().find(|expected| expected.name == argument.name).unwrap();
                let FilterValue::Number(ExactNumber::Float32Bits(bits)) = expected.value else { panic!("independent Float32 argument") };
                f32::from_bits(bits).to_string()
            })
            .collect();
        assert_eq!(values.len(), 6);
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(340.0, 900.0)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        for (index, value) in values.iter().enumerate() {
            harness.query_all_by_role(egui::accesskit::Role::SpinButton).nth(index).unwrap().click();
            harness.run_steps(2);
            harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            harness.query_all_by_role(egui::accesskit::Role::SpinButton).find(|node| node.is_focused()).unwrap().type_text(value);
            harness.key_press(egui::Key::Enter);
            harness.run_steps(2);
            assert_eq!(harness.state().session.active().unwrap().history.past_len(), index + 1);
        }
        for name in keep_labels {
            harness.get_by_label(name).click();
            harness.run_steps(2);
        }
        let active = harness.state().session.active().unwrap();
        assert_eq!(active.history.past_len(), 8);
        let edited = &active.doc;
        assert_eq!(&edited.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments, reference_arguments);
        assert_eq!(edited.layers, original.layers);
        assert_eq!(edited.source_color_context, original.source_color_context);
        assert_eq!(edited.xcf_state.as_ref().unwrap().opaque, original.xcf_state.as_ref().unwrap().opaque);
        assert_eq!(edited.xcf_state.as_ref().unwrap().filter_masks, original.xcf_state.as_ref().unwrap().filter_masks);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap(),
            photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap()
        );
        let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
        for _ in 0..8 {
            harness.state_mut().run("edit.undo", json!({})).unwrap();
        }
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        for _ in 0..8 {
            harness.state_mut().run("edit.redo", json!({})).unwrap();
        }
        assert_eq!(&harness.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments, reference_arguments);
        let current = harness.state_mut().session.active_mut().unwrap();
        current
            .doc
            .try_resident_mut()
            .unwrap()
            .xcf_state
            .as_mut()
            .unwrap()
            .items
            .iter_mut()
            .find(|item| item.item == ItemRef::Layer(layer))
            .unwrap()
            .locks
            .content = true;
        harness.run_steps(2);
        assert!(harness.query_all_by_role(egui::accesskit::Role::SpinButton).all(|node| node.accesskit_node().is_disabled()));
        for name in keep_labels {
            assert!(harness.get_by_label(name).accesskit_node().is_disabled());
        }
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    fn boolean_filter_case(packet: &str, source_file: &str, reference_file: &str, label: &str) {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(packet);
        let bytes = std::fs::read(root.join(source_file)).unwrap();
        let original = photocraft_io::import("clip.xcf", &bytes).unwrap().document;
        let reference = photocraft_io::import("clip.xcf", &std::fs::read(root.join(reference_file)).unwrap()).unwrap().document;
        let layer = original.layers[0].id;
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(300.0, 480.0)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        if packet == "sepia-precision-evidence" {
            let FilterValue::Number(ExactNumber::Float32Bits(bits)) =
                reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.iter().find(|argument| argument.name == "scale").unwrap().value
            else {
                panic!("independent scale")
            };
            harness.get_by_role(egui::accesskit::Role::SpinButton).click();
            harness.run_steps(2);
            harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            let input = harness.get_by_role(egui::accesskit::Role::SpinButton);
            assert!(input.is_focused());
            input.type_text(&f32::from_bits(bits).to_string());
            harness.key_press(egui::Key::Enter);
            harness.run_steps(2);
            assert_eq!(harness.state().session.active().unwrap().history.past_len(), 1);
        }
        let before_boolean = harness.state().session.active().unwrap().doc.clone();
        let prior_history = harness.state().session.active().unwrap().history.past_len();
        harness.get_by_label(label).click();
        harness.run_steps(2);
        let active = harness.state().session.active().unwrap();
        let edited = &active.doc;
        assert_eq!(active.history.past_len(), prior_history + 1);
        assert_eq!(edited.layers, original.layers);
        let state = edited.xcf_state.as_ref().unwrap();
        assert_eq!(state.filter_masks, original.xcf_state.as_ref().unwrap().filter_masks);
        assert_eq!(state.filter_stacks[0].nodes[0].arguments, reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments);
        let actual = photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap();
        let expected = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(actual, expected);
        let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*harness.state().session.active().unwrap().doc, *before_boolean);
        for _ in 0..prior_history {
            harness.state_mut().run("edit.undo", json!({})).unwrap();
        }
        assert_eq!(*harness.state().session.active().unwrap().doc, original);
        let state = harness.state_mut().session.active_mut().unwrap();
        let doc = state.doc.try_resident_mut().unwrap();
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().locks.content = true;
        harness.run_steps(2);
        assert!(harness.get_by_label(label).accesskit_node().is_disabled());
        assert_eq!(std::fs::read(root.join(source_file)).unwrap(), bytes);
    }
}

#[cfg(test)]
mod channel_mixer_tests;

#[cfg(test)]
mod color_rotate_tests;

#[cfg(test)]
mod color_rotate_boundary_tests;

#[cfg(test)]
mod red_eye_geometry_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "xcf_panels/gaussian_group_activation_tests.rs"]
mod gaussian_group_activation_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "xcf_panels/pure_point_geometry_tests.rs"]
mod pure_point_geometry_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "xcf_panels/nine_point_geometry_tests.rs"]
mod nine_point_geometry_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "xcf_panels/residual_point_geometry_tests.rs"]
mod residual_point_geometry_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "xcf_panels/filtered_flat_tests.rs"]
mod filtered_flat_tests;
