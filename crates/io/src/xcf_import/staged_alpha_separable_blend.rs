//! Exact existing RGB/Gray alpha-mode source domain, not a general Gray grant.
use super::*;
pub(super) fn mode(mode: i32) -> bool {
    matches!(mode, 57..=60)
}
pub(super) fn domain<P: ExactParsedPlane>(f: &XcfFile<P>) -> bool {
    matches!(f.model, Model::Rgb | Model::Gray)
        && legacy::geometry_model(f, f.model)
        && f.layers.first().is_some_and(|top| {
            photocraft_doc::import_compositing::flat_alpha_source_rule(f.version, top.mode, top.blend_space, top.composite_space, top.composite_mode).is_some()
        })
}
pub(super) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, ctl: &Interrupt<'_>, budget: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    if !domain(f) {
        return Ok(false);
    }
    legacy::native_layout(f, if f.model == Model::Gray { 2 } else { 4 }, ctl, budget)
}
