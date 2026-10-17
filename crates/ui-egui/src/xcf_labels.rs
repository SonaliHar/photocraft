//! Source mode names without inventing Photoshop blend-mode equivalence.
use photocraft_doc::item_identity::ItemRef;
use photocraft_doc::{Document, Layer};

fn mode_label(mode: i32) -> Option<&'static str> {
    Some(match mode {
        11 => "GIMP HSV Hue (legacy)",
        12 => "GIMP HSV Saturation (legacy)",
        13 => "GIMP HSL Color (legacy)",
        14 => "GIMP HSV Value (legacy)",
        20 => "GIMP Grain Extract (legacy)",
        21 => "GIMP Grain Merge (legacy)",
        24 => "GIMP LCh Hue",
        25 => "GIMP LCh Chroma",
        26 => "GIMP LCh Color",
        27 => "GIMP LCh Lightness",
        37 => "GIMP HSV Hue",
        38 => "GIMP HSV Saturation",
        39 => "GIMP HSL Color",
        40 => "GIMP HSV Value",
        46 => "GIMP Grain Extract",
        47 => "GIMP Grain Merge",
        54 => "GIMP Luma Darken",
        55 => "GIMP Luma Lighten",
        56 => "GIMP Luminance",
        57 => "GIMP Color Erase",
        58 => "GIMP Erase",
        59 => "GIMP Merge",
        60 => "GIMP Split",
        _ => return None,
    })
}
pub(crate) fn source_blend_label(doc: &Document, layer: &Layer) -> Option<&'static str> {
    let state = doc.xcf_state.as_ref()?;
    let item = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id))?;
    mode_label(item.composition.as_ref()?.source_mode)
}
#[cfg(test)]
pub(crate) fn readonly_blend(ui: &mut egui::Ui, label: &str, width: f32) {
    ui.add_enabled_ui(false, |ui| {
        egui::ComboBox::from_id_salt("xcf-source-blend").selected_text(tl!(label)).width(width).show_ui(ui, |_| {});
    })
    .response
    .on_hover_text(tl!("Changing this imported GIMP mode is not supported yet."));
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, PixelFormat, SampleType, Size};
    fn document(mode: i32) -> Document {
        let mut doc = Document::new("source blend", Size::new(64, 48), ColorMode::Rgb, SampleType::U8);
        let layer = Layer::raster("source layer", PixelFormat::RGBA8);
        let item = ItemRef::Layer(layer.id);
        doc.layers.push(layer);
        let mut state = photocraft_doc::XcfState::default();
        state.provenance.format = "xcf".into();
        state.provenance.format_version = 24;
        state.items.push(photocraft_doc::xcf_state::ItemState {
            item,
            name: "source layer".into(),
            custom_name: false,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: Some(1201),
            source_flags: 0,
            raster_bounds: None,
            mask: None,
            composition: Some(photocraft_doc::import_compositing::GimpComposition {
                compatibility_epoch: "ui-source-label-only".into(),
                source_mode: mode,
                legacy: (11..=14).contains(&mode),
                stored_blend_space: -1,
                stored_composite_space: -1,
                stored_operator: 0,
                resolved: None,
                pass_through: false,
            }),
        });
        doc.xcf_state = Some(state);
        doc
    }
    #[test]
    fn distinct_source_color_models_never_claim_normal_or_photoshop_equivalence() {
        for mode in [11, 12, 13, 14, 20, 21, 24, 25, 26, 27, 37, 38, 39, 40, 46, 47, 54, 55, 56, 57, 58, 59, 60] {
            let label = mode_label(mode).unwrap();
            assert!(label.starts_with("GIMP "));
            assert!(!label.contains("Normal"));
            assert_eq!(label.contains("legacy"), matches!(mode, 11..=14 | 20 | 21));
        }
        assert_eq!(mode_label(24), Some("GIMP LCh Hue"));
        assert_eq!(mode_label(37), Some("GIMP HSV Hue"));
        assert_ne!(mode_label(26), mode_label(39));
        for mode in [0, 28, 30, 61, -1, i32::MAX] {
            assert_eq!(mode_label(mode), None);
        }
    }
    #[test]
    fn canonical_owner_identity_controls_label_and_native_reopen_retains_it() {
        let doc = document(24);
        assert_eq!(doc.layers[0].blend, photocraft_color::BlendMode::Normal);
        assert_eq!(source_blend_label(&doc, &doc.layers[0]), Some("GIMP LCh Hue"));
        let unrelated = Layer::raster("other", PixelFormat::RGBA8);
        assert_eq!(source_blend_label(&doc, &unrelated), None);
        let raw = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&raw).unwrap();
        assert_eq!(source_blend_label(&reopened, &reopened.layers[0]), Some("GIMP LCh Hue"));
        assert_eq!(reopened.xcf_state, doc.xcf_state);
        let mut plain = doc.clone();
        plain.xcf_state = None;
        assert_eq!(source_blend_label(&plain, &plain.layers[0]), None);
    }
    #[test]
    fn source_mode_widget_is_disabled_without_disabling_neighbor_opacity() {
        use egui_kittest::kittest::{NodeT, Queryable};
        let mut opacity = 1f32;
        let mut h = egui_kittest::Harness::new_ui(|ui| {
            ui.horizontal(|ui| {
                readonly_blend(ui, "GIMP HSV Hue", 180.);
                ui.add(egui::Slider::new(&mut opacity, 0.0..=1.0).text("Opacity"));
            });
        });
        h.run_steps(2);
        assert!(h.get_by_role(egui::accesskit::Role::ComboBox).accesskit_node().is_disabled());
        assert!(!h.get_by_role_and_label(egui::accesskit::Role::Slider, "Opacity").accesskit_node().is_disabled());
    }
}
