//! Worker-only native mesh bridge. Registered capture/publication stays in
//! selection_transform_controlled; this module does not mutate a Session.
use crate::{EngineError, Result};
use photocraft_algo::wide_geometry::GeometryLimits;
use photocraft_geom::{Rect, warp::Warp};
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use serde_json::Value;
const ID: &str = "select.transformSelection";
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: message.into() }
}
pub(crate) fn policy(p: &Value) -> Result<()> {
    if !matches!(p.get("interpolation").and_then(Value::as_str), Some("nearest" | "nearestNeighbor")) {
        return Err(bad("native selection mesh requires explicit nearest"));
    }
    Ok(())
}
pub(crate) fn prepare(source: &Surface, bounds: Rect, warp: &Warp, limits: GeometryLimits, c: &Interrupt<'_>) -> Result<Surface> {
    c.check().map_err(|_| EngineError::Cancelled)?;
    photocraft_algo::selection_mesh_nearest_controlled::try_warp_gray_nearest_mesh_with_limits(source, bounds, warp, limits, c)
        .map_err(|e| if matches!(e, RasterInputError::Cancelled) || c.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) })
}
