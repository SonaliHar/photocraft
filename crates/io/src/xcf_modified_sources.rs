//! Narrow source-raster admission backed by modified-buffer declarations.
//! Text/vector regeneration and resource resolution are deliberately separate consumers.
use crate::{
    IoError,
    xcf_sources::{self, MappedSource, SourceContext},
};
use photocraft_doc::{PixelFormat, item_identity::ResourceId};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Drawable, Encoding, Model, Plane, Sample};
pub struct ModifiedSourceContext {
    pub mapping: SourceContext,
    pub source_precision: u32,
    pub source_encoding: Encoding,
    pub assigned_profile_present: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifiedSourceKind {
    Text,
    Vector,
    Linked,
}
pub struct ModifiedSource {
    pub kind: ModifiedSourceKind,
    pub mapped: MappedSource,
    pub warning: &'static str,
}
impl std::fmt::Debug for ModifiedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModifiedSource").field("kind", &self.kind).field("warning", &self.warning).finish_non_exhaustive()
    }
}
fn unsupported(reason: &str) -> IoError {
    IoError::Unsupported(format!("XCF modified source: {reason}"))
}
/// Only the explicit modified-buffer declaration selects cached raster authority.
/// The original source cache remains immutable provenance; ordinary pixel editing acts
/// on the layer raster. Editing source parameters requires a qualified regeneration
/// consumer, and must fail explicitly until one is available.
pub fn validate_modified_source(source: &Drawable, context: &ModifiedSourceContext, ctl: &Interrupt<'_>) -> Result<ModifiedSourceKind, IoError> {
    validate_modified_source_with_storage(source, context, ctl)
}
pub fn validate_modified_source_with_storage<P>(
    source: &Drawable<P>,
    context: &ModifiedSourceContext,
    ctl: &Interrupt<'_>,
) -> Result<ModifiedSourceKind, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let kind = match (source.text.as_ref(), source.vector.as_ref(), source.link.as_ref()) {
        (Some(_), None, None) => {
            if source.text_flags & !3 != 0 {
                return Err(unsupported("unknown text flags"));
            }
            if !source.text_source_modified() {
                return Err(unsupported("text requires font-qualified regeneration; an unmodified cache is not a renderer"));
            }
            if context.mapping.source_version != 19 {
                return Err(unsupported("unqualified modified-text source epoch"));
            }
            if context.mapping.vector_path.is_some() {
                return Err(unsupported("text has an unexpected vector dependency"));
            }
            ModifiedSourceKind::Text
        }
        (None, Some(_), None) => return Err(unsupported("vector regeneration is required; its modified flag does not prove saved-cache raster authority")),
        (None, None, Some(_)) => {
            crate::xcf_link_sources::validate(source,context,ctl)?;
            return Ok(ModifiedSourceKind::Linked);
        }
        _ => return Err(unsupported("missing or conflicting source kind")),
    };
    if source.group || source.mask.is_some() || !source.effects.is_empty() || source.transform.is_some() || source.floating_attachment.is_some() {
        return Err(unsupported("unqualified source mask/effect/transform/group/floating interaction"));
    }
    if context.source_precision != 150
        || context.source_encoding != Encoding::NonlinearProfile
        || context.assigned_profile_present
        || source.model != Model::Rgb
        || !source.alpha
        || context.mapping.format != PixelFormat::RGBA8
    {
        return Err(unsupported("unqualified modified-source sample/profile encoding"));
    }
    if source.mode != 28 || (source.composite_mode, source.composite_space, source.blend_space) != (-1, -1, 0) || source.opacity.to_bits() != 1.0f32.to_bits() {
        return Err(unsupported("unqualified modified-source composition"));
    }
    let bound = 1i64 << 29;
    if source.width == 0 || source.height == 0 || source.width > 262144 || source.height > 262144 {
        return Err(unsupported("source dimension limit"));
    }
    for (offset, length) in [(source.offset.0, source.width), (source.offset.1, source.height)] {
        let start = i64::from(offset);
        let end = start.checked_add(i64::from(length)).ok_or_else(|| unsupported("source extent overflow"))?;
        if start < -bound || end > bound {
            return Err(unsupported("source coordinate limit"));
        }
    }
    let marker = match kind {
        ModifiedSourceKind::Text => "editable GIMP text layer",
        ModifiedSourceKind::Vector => "editable GIMP vector layer",
        ModifiedSourceKind::Linked => "linked layer/transform",
    };
    if source.unsupported.iter().any(|reason| reason != marker) {
        return Err(unsupported("additional unsupported source rendering state"));
    }
    Ok(kind)
}
/// Transfers the original plane without cloning; the source's plane field may already
/// be None after take(). Its consumed descriptor and byte count are checked independently.
pub fn map_modified_source(
    source: &Drawable,
    cache: Plane,
    context: ModifiedSourceContext,
    ctl: &Interrupt<'_>,
    fresh: &mut dyn FnMut() -> ResourceId,
    charge: &mut dyn FnMut(usize) -> Result<(), IoError>,
) -> Result<ModifiedSource, IoError> {
    map_modified_source_with_storage(source, cache, context, ctl, fresh, charge)
}
pub fn map_modified_source_with_storage<P>(
    source: &Drawable<P>,
    cache: Plane,
    context: ModifiedSourceContext,
    ctl: &Interrupt<'_>,
    fresh: &mut dyn FnMut() -> ResourceId,
    charge: &mut dyn FnMut(usize) -> Result<(), IoError>,
) -> Result<ModifiedSource, IoError> {
    let kind = validate_modified_source_with_storage(source, &context, ctl)?;
    if cache.sample != Sample::U8 {
        return Err(unsupported("cache sample descriptor mismatch"));
    }
    let mut mapped = xcf_sources::map_source_with_storage(source, Some(cache), context.mapping, ctl, fresh, charge)?
        .ok_or_else(|| unsupported("source mapping disappeared"))?;
    // The validated modified flag selects the saved painted pixels as raster authority.
    // These known, typed source records remain immutable regeneration evidence; they
    // are not unknown rendering dependencies of that current painted raster.
    for record in &mut mapped.opaque {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if kind != ModifiedSourceKind::Linked || (record.namespace=="gimp/xcf/source-property" && matches!(record.identity.as_str(),"48"|"49") && record.source_version==25) {
            record.rendering_required = false;
        }
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let warning = match kind {
        ModifiedSourceKind::Text => "Modified GIMP text pixels are retained with their text and font references. Text regeneration is unavailable.",
        ModifiedSourceKind::Vector => "Modified GIMP vector pixels are retained with their path and fill/stroke settings. Vector regeneration is unavailable.",
        ModifiedSourceKind::Linked => "The original qualified linked appearance is retained. Its image dependency remains unresolved; explicit authorized recovery is required before regeneration.",
    };
    Ok(ModifiedSource { kind, mapped, warning })
}
