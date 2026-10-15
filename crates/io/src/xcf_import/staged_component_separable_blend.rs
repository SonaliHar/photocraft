//! Exact existing component/luma original source rule, separate from modern Auto.
use super::*;
pub(super) fn mode(mode: i32) -> bool {
    matches!(mode,37..=40|54..=56)
}
pub(super) fn domain<P: ExactParsedPlane>(f: &XcfFile<P>) -> bool {
    legacy::geometry(f)
        && f.layers.first().is_some_and(|top| {
            photocraft_doc::import_compositing::flat_component_source_rule(f.version, top.mode, top.blend_space, top.composite_space, top.composite_mode)
                .is_some()
        })
}
pub(super) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, ctl: &Interrupt<'_>, budget: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    if !domain(f) {
        return Ok(false);
    }
    legacy::native(f, ctl, budget)
}
