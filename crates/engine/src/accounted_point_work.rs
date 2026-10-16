//! Bounded work routing only. This never validates a source, recipe or command.
use photocraft_color::SampleType;
use photocraft_doc::{ColorMode, LayerContent, SampleEncoding, accounted_document::AccountedDocument, item_identity::ItemRef};
use photocraft_raster::backing::{BackingError, ReadControl};
/// Select finite standard service for existing Box or the narrowly qualified
/// implicit Point routes. Full current/native/command validation remains mandatory.
pub fn is_current_standard_work_candidate(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<bool, BackingError> {
    c.check(1)?;
    if owner.is_readonly_filtered_flat_point_storage() || owner.is_readonly_masked_filtered_flat_point_storage() { return Ok(true); }
    if !(owner.is_readonly_point_storage() || owner.is_readonly_assigned_point_storage()) {
        return Ok(false);
    }
    if super::is_current_box_work_candidate(owner.document(), c)? {
        return Ok(true);
    }
    is_current_pure_point_work_candidate(owner, c)
}
/// Separate from Box so registered command selection retains every old Box route.
pub fn is_current_pure_point_work_candidate(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<bool, BackingError> {
    c.check(1)?;
    if owner.is_readonly_filtered_flat_point_storage() || owner.is_readonly_masked_filtered_flat_point_storage() { return Ok(true); }
    if !owner.is_readonly_point_storage() {
        return Ok(false);
    }
    let d = owner.document();
    let [layer] = d.layers.as_slice() else { return Ok(false) };
    let Some(state) = d.xcf_state.as_ref() else { return Ok(false) };
    let [stack] = state.filter_stacks.as_slice() else { return Ok(false) };
    if stack.owner != ItemRef::Layer(layer.id) {
        return Ok(false);
    }
    match stack.nodes.as_slice() {
        [node] => {
            c.check(node.operation.name.len() as u64)?;
            c.check(node.operation.schema_epoch.len() as u64)?;
            let compat = node.composition.as_ref().map(|p| p.compatibility_epoch.as_str());
            if let Some(value) = compat {
                c.check(value.len() as u64)?;
            }
            let epoch =
                |v: &str| matches!(v, photocraft_compose::SEMI_FLATTEN_EPOCH | photocraft_compose::HUE_CHROMA_EPOCH | photocraft_compose::VIBRANCE_EPOCH);
            let existing = matches!(node.operation.name.as_str(), "gimp:semi-flatten" | "gegl:hue-chroma" | "gegl:vibrance")
                || epoch(&node.operation.schema_epoch)
                || compat.is_some_and(epoch)
                // These nine existing roles share their old Replace compatibility
                // epoch. Require the exact operation/schema/compatibility triple;
                // names alone would include unrelated original/geometry policies.
                || node.operation.stored_version == "0:0"
                    && compat == Some("xcf-22-stock-point-replace-fields-v1")
                    && matches!((node.operation.name.as_str(),node.operation.schema_epoch.as_str()),
                        ("gegl:absolute","xcf-22-stock-absolute-linear-precision-v1")
                        | ("gegl:channel-mixer","xcf-22-stock-channel-mixer-precision-v1")
                        | ("gegl:mono-mixer","xcf-22-stock-mono-mixer-signed-precision-v1")
                        | ("gegl:color-overlay","xcf-22-stock-color-overlay-six-precision-v1")
                        | ("gegl:component-extract","xcf-22-stock-component-extract-four-precision-v1")
                        | ("gegl:component-extract","xcf-22-stock-component-extract-cmyk-precision-v1")
                        | ("gegl:component-extract","xcf-22-stock-component-extract-ycbcr-precision-v1")
                        | ("gegl:alpha-clip","xcf22-stock-alpha-clip-precision-v1")
                        | ("gegl:alpha-clip","xcf22-stock-alpha-clip-finite-precision-v1"));
            // New loaded Point policy is exact and never selects mixed storage.
            if node.operation.name=="gegl:exposure" && node.operation.stored_version=="1:0"
                && node.operation.schema_epoch==photocraft_compose::EXPOSURE_LOADED_POINT_EPOCH
                && compat==Some(photocraft_compose::EXPOSURE_LOADED_POINT_EPOCH){return Ok(true)}
            // Masked Point uses its own exact triple, never an Exposure name grant.
            if node.operation.name=="gegl:exposure" && node.operation.stored_version=="1:0"
                && node.operation.schema_epoch==photocraft_compose::EXPOSURE_MASKED_EPOCH
                && compat==Some(photocraft_compose::EXPOSURE_MASKED_EPOCH){return Ok(true)}
            if node.operation.stored_version=="0:0" && compat==Some(node.operation.schema_epoch.as_str())
                && matches!((node.operation.name.as_str(),node.operation.schema_epoch.as_str()),
                    ("gegl:invert-linear",photocraft_compose::INVERT_LINEAR_MASKED_EPOCH)
                    |("gegl:invert-gamma",photocraft_compose::INVERT_GAMMA_MASKED_EPOCH)
                    |("gegl:brightness-contrast",photocraft_compose::BRIGHTNESS_CONTRAST_MASKED_EPOCH)
                    |("gegl:levels",photocraft_compose::GEGL_LEVELS_MASKED_EPOCH)){return Ok(true)}
            // Existing positive routes retain their exact prior checks/debits.
            if existing {
                return Ok(true);
            }
            // The seven residual/configured roles use their own compatibility
            // epochs, never the generic PointReplace epoch or a names-only OR.
            // Pay fixed bounded comparisons before this additive fallback.
            c.check(7 * 128)?;
            Ok(node.operation.stored_version == "0:0"
                && compat == Some(node.operation.schema_epoch.as_str())
                && matches!((node.operation.name.as_str(), node.operation.schema_epoch.as_str()),
                    ("gimp:desaturate", "xcf-22-stock-desaturate-precision-v1")
                    | ("gegl:saturation", "xcf-22-stock-saturation-precision-v1")
                    | ("gegl:saturation", "xcf-22-stock-gray-saturation-precision-v1")
                    | ("gegl:saturation", "xcf-22-stock-rgb-yuv-saturation-precision-v1")
                    | ("gegl:value-invert", "xcf-22-stock-value-invert-precision-v1")
                    | ("gimp:color-balance", "xcf-22-stock-color-balance-precision-v1")
                    | ("gimp:color-balance", "xcf-22-stock-color-balance-depth-v1")))
        }
        [] => {
            c.check(128)?;
            let Some(source) = d.source_color_context.as_ref() else { return Ok(false) };
            if d.mode != ColorMode::Rgb
                || d.sample_encoding != Some(SampleEncoding::NonlinearProfile)
                || !matches!((d.depth, source.source_precision), (SampleType::F32, 650) | (SampleType::F64, 750))
                || source.original_icc.is_some()
                || !state.filter_masks.is_empty()
            {
                return Ok(false);
            }
            let [item] = state.items.as_slice() else { return Ok(false) };
            if item.item != ItemRef::Layer(layer.id) {
                return Ok(false);
            }
            let LayerContent::BackedRaster(backed) = &layer.content else { return Ok(false) };
            let r = backed.operative_bounds();
            Ok(r.x0 == 0
                && r.y0 == 0
                && r.x1 > 0
                && r.y1 > 0
                && r == d.bounds()
                && r.size() == d.size
                && r.width() <= 262144
                && r.height() <= 262144
                && u64::from(r.width()).checked_mul(u64::from(r.height())).is_some_and(|n| n <= (1u64 << 30))
                && item.raster_bounds == Some(r))
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
#[path = "accounted_point_work_tests.rs"]
mod tests;
