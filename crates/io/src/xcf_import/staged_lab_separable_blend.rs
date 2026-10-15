//! Exact old RGB LCh source policy; the paid Lab owner is separate authority.
use super::*;
pub(super) fn mode(mode: i32) -> bool {
    matches!(mode, 24..=27)
}
pub(super) fn domain<P: ExactParsedPlane>(f: &XcfFile<P>) -> bool {
    legacy::geometry(f)
        && f.layers.first().is_some_and(|top| {
            photocraft_doc::import_compositing::flat_lab_source_rule(f.version, top.mode, top.blend_space, top.composite_space, top.composite_mode).is_some()
        })
}
pub(super) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, ctl: &Interrupt<'_>, budget: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    if !domain(f) {
        return Ok(false);
    }
    legacy::native(f, ctl, budget)
}
