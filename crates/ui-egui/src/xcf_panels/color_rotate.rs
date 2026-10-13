//! Destination-only view draft for independently witnessed Color Rotate pairs.
use super::*;
use photocraft_doc::xcf_filters::FilterNode;
pub(super) const EPOCH: &str = "xcf-22-stock-color-rotate-perceptual-v1";
pub(super) fn valid_node(node: &FilterNode) -> bool {
    if node.operation.name != "gegl:color-rotate" || node.operation.stored_version != "0:0" || node.operation.schema_epoch != EPOCH {
        return false;
    }
    let Ok(op) = photocraft_compose::gimp_color_rotate::typed_arguments(&node.arguments) else { return false };
    let Some(raw) = photocraft_compose::gimp_color_rotate::witnessed_source_triplet(op) else { return false };
    node.composition.as_ref().is_some_and(|p| (p.stored_blend_space, p.stored_composite_space, p.stored_operator) == (raw, raw, raw))
        && !op.destination.clockwise
        && valid_pair_for_raw(raw, op.destination.from_degrees, op.destination.to_degrees)
}
pub(super) fn valid_pair(from: f32, to: f32) -> bool {
    (from.to_bits() == 30f32.to_bits() && to.to_bits() == 210f32.to_bits()) || (from.to_bits() == 210f32.to_bits() && to.to_bits() == 330f32.to_bits())
}
fn valid_pair_for_raw(raw: i32, from: f32, to: f32) -> bool {
    match raw {
        -1567849903 => valid_pair(from, to),
        -352812463 => {
            (from.to_bits() == 33.125f32.to_bits() && to.to_bits() == 211.75f32.to_bits())
                || (from.to_bits() == 207.5f32.to_bits() && to.to_bits() == 337.25f32.to_bits())
        }
        _ => false,
    }
}
fn parse_from(text: &str) -> Option<f64> {
    parse(text, &[30., 210.])
}
fn parse_to(text: &str) -> Option<f64> {
    parse(text, &[210., 330.])
}
fn parse_boundary_from(text: &str) -> Option<f64> {
    parse(text, &[33.125, 207.5])
}
fn parse_boundary_to(text: &str) -> Option<f64> {
    parse(text, &[211.75, 337.25])
}
fn parse(text: &str, values: &[f32]) -> Option<f64> {
    if text.len() > 32 {
        return None;
    }
    let value = text.trim().parse::<f64>().ok()?;
    values.iter().any(|v| f64::from(*v).to_bits() == value.to_bits()).then_some(value)
}
fn angle(ui: &mut egui::Ui, label: &str, value: &mut f32, editable: bool, parse: fn(&str) -> Option<f64>) {
    ui.horizontal(|ui| {
        let label = ui.label(label);
        ui.add_enabled(
            editable,
            egui::DragValue::new(value).speed(0.0).custom_parser(parse).custom_formatter(|value, _| (value as f32).to_string()).update_while_editing(false),
        )
        .labelled_by(label.id);
    });
}
pub(super) fn controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, document: u64, revision: u64, node: &FilterNode, editable: bool) {
    let Ok(op) = photocraft_compose::gimp_color_rotate::typed_arguments(&node.arguments) else { return };
    let Some(raw) = photocraft_compose::gimp_color_rotate::witnessed_source_triplet(op) else { return };
    let identity = |d: &crate::state::XcfColorRotateDraft| d.document == document && d.revision == revision && d.filter == node.id.0;
    if app.ui.xcf_color_rotate_draft.as_ref().is_none_or(|d| !identity(d)) {
        app.ui.xcf_color_rotate_draft =
            Some(crate::state::XcfColorRotateDraft { document, revision, filter: node.id.0, from: op.destination.from_degrees, to: op.destination.to_degrees });
    }
    let Some(draft) = app.ui.xcf_color_rotate_draft.as_mut() else { return };
    if !draft.from.is_finite() || !draft.to.is_finite() {
        ui.label(tl!("Filter editing is unavailable"));
        return;
    }
    let from_parser: fn(&str) -> Option<f64> = if raw == -352812463 { parse_boundary_from } else { parse_from };
    let to_parser: fn(&str) -> Option<f64> = if raw == -352812463 { parse_boundary_to } else { parse_to };
    angle(ui, tl!("Destination start angle"), &mut draft.from, editable, from_parser);
    angle(ui, tl!("Destination end angle"), &mut draft.to, editable, to_parser);
    let changed = draft.from.to_bits() != op.destination.from_degrees.to_bits() || draft.to.to_bits() != op.destination.to_degrees.to_bits();
    let valid = valid_pair_for_raw(raw, draft.from, draft.to);
    let apply = ui.add_enabled(editable && changed && valid, egui::Button::new(tl!("Apply"))).clicked();
    if ui.add_enabled(changed, egui::Button::new(tl!("Reset"))).clicked() {
        draft.from = op.destination.from_degrees;
        draft.to = op.destination.to_degrees;
    }
    let values = apply.then_some((draft.from.to_bits(), draft.to.to_bits()));
    if let Some((from, to)) = values {
        let _=app.run("xcf.filter.set",json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"dest-from","value":{"Number":{"Float32Bits":from}}},{"name":"dest-to","value":{"Number":{"Float32Bits":to}}}]}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn witnessed_pairs_and_decimal_parsers_do_not_normalize() {
        assert!(valid_pair(30., 210.));
        assert!(valid_pair(210., 330.));
        for pair in [(210., 210.), (30., 330.), (f32::NAN, 210.), (30., f32::INFINITY)] {
            assert!(!valid_pair(pair.0, pair.1));
        }
        assert_eq!(parse_from("30"), Some(30.));
        assert_eq!(parse_to("330"), Some(330.));
        for text in ["30.00000001", "-0", "NaN", "inf", "360", "-1"] {
            assert!(parse_from(text).is_none());
        }
        assert!(parse_to("330.00000001").is_none());
        assert!(parse_from(&"1".repeat(33)).is_none());
        assert!(valid_pair_for_raw(-352812463, 33.125, 211.75));
        assert!(valid_pair_for_raw(-352812463, 207.5, 337.25));
        assert!(!valid_pair_for_raw(-352812463, 30., 210.));
        assert!(!valid_pair_for_raw(-1567849903, 33.125, 211.75));
        assert!(!valid_pair_for_raw(-352812463, 33.125, 337.25));
        assert!(!valid_pair_for_raw(0, 33.125, 211.75));
        assert_eq!(parse_boundary_from("33.125"), Some(33.125));
        assert_eq!(parse_boundary_to("337.25"), Some(337.25));
        assert!(parse_boundary_from("33.125000001").is_none());
        assert!(parse_boundary_to("337.250000001").is_none());
    }
}
