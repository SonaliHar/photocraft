//! Distinct mixed-only invert identities; no singleton source epoch alias.
use photocraft_xcf::Effect;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Linear,
    Gamma,
}
/// Zero arguments means no typed configuration, graph or literal allocation.
/// Envelope, geometry, model and actual source proof remain caller obligations.
pub(crate) fn kind<P>(effect: &Effect<P>) -> Option<Kind> {
    if effect.version.as_deref() != Some("0:0") || !effect.arguments.is_empty() {
        return None;
    }
    match effect.operation.as_str() {
        "gegl:invert-linear" => Some(Kind::Linear),
        "gegl:invert-gamma" => Some(Kind::Gamma),
        _ => None,
    }
}
