//! Preflight for original qualified linked drawable appearance. No resource reads.
use crate::{IoError, xcf_modified_sources::ModifiedSourceContext};
use photocraft_doc::{PixelFormat, Rect, xcf_items::TransformState};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Drawable, Encoding, Model};
fn refused() -> IoError {
    IoError::Unsupported("XCF linked source requires an independently qualified stored appearance and explicit dependency recovery".into())
}
pub fn validate<P>(source: &Drawable<P>, context: &ModifiedSourceContext, control: &Interrupt<'_>) -> Result<(), IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    let linked = source.link.as_ref().ok_or_else(refused)?;
    let transform = source.transform.as_ref().ok_or_else(refused)?;
    if context.mapping.source_version != 25
        || context.source_precision != 150
        || context.source_encoding != Encoding::NonlinearProfile
        || context.mapping.format != PixelFormat::RGBA8
        || source.model != Model::Rgb
        || !source.alpha
        || source.text.is_some()
        || source.vector.is_some()
        // XCF's known bits control automatic naming and monitoring, not pixels.
        // Both are retained as inert source data; this importer never monitors.
        || linked.flags > 3
        || linked.path.is_empty()
        || source.group
        || source.mask.is_some()
        || !source.effects.is_empty()
        || source.floating_attachment.is_some()
        || source.mode != 28
        || (source.composite_mode, source.composite_space, source.blend_space) != (-1, -1, 0)
        || source.opacity.to_bits() != 1.0f32.to_bits()
        || source.unsupported.iter().any(|reason| reason != "linked layer/transform")
    {
        return Err(refused());
    }
    let x1 = i64::from(source.offset.0).checked_add(i64::from(source.width)).ok_or_else(refused)?;
    let y1 = i64::from(source.offset.1).checked_add(i64::from(source.height)).ok_or_else(refused)?;
    let bound = 1i64 << 29;
    if i64::from(source.offset.0) < -bound || i64::from(source.offset.1) < -bound || x1 > bound || y1 > bound {
        return Err(refused());
    }
    let bounds = Rect::new(source.offset.0, source.offset.1, i32::try_from(x1).map_err(|_| refused())?, i32::try_from(y1).map_err(|_| refused())?);
    let state = TransformState {
        matrix_f64_bits: transform.matrix.map(|value| f64::from(value).to_bits()),
        interpolation: i32::try_from(transform.interpolation).map_err(|_| refused())?,
        pre_offset: [transform.offset.0, transform.offset.1],
    };
    photocraft_compose::gimp_link_items::qualify_linked_geometry(linked.width, linked.height, &state, bounds).map_err(|_| refused())?;
    Ok(())
}
