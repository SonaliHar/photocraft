//! Saved path geometry mapping. Source evidence is inert and retained alongside canonical IDs.
use crate::IoError;
use photocraft_doc::{
    Knot, NamedPath, Path, PathOp, Subpath,
    item_identity::{ItemRef, PathId},
    xcf_provenance::OpaqueRecord,
    xcf_state::{ItemLocks, ItemState},
};
use photocraft_geom::Point;
use photocraft_raster::Interrupt;
use photocraft_xcf::{PathPoint, VectorPath};

pub struct MappedPaths {
    pub paths: Vec<NamedPath>,
    pub items: Vec<ItemState>,
    pub selected: Vec<ItemRef>,
    pub memberships: Vec<(u32, ItemRef)>,
    pub tattoos: Vec<(u32, PathId)>,
    pub opaque: Vec<OpaqueRecord>,
}
fn unsupported(message: &str) -> IoError {
    IoError::Unsupported(format!("XCF saved path: {message}"))
}
fn reserve<T>(out: &mut Vec<T>, count: usize) -> Result<(), IoError> {
    out.try_reserve_exact(count).map_err(|_| unsupported("allocation failed"))
}
fn coordinate(point: &PathPoint) -> Result<Point, IoError> {
    if point.legacy_integer_coordinates.is_some() {
        return Err(unsupported("unqualified legacy integer geometry"));
    }
    if !(2..=6).contains(&point.components.len()) {
        return Err(unsupported("point component count"));
    }
    let x = point.components.first().copied().ok_or_else(|| unsupported("missing x"))?;
    let y = point.components.get(1).copied().ok_or_else(|| unsupported("missing y"))?;
    let x = f32::from_bits(x);
    let y = f32::from_bits(y);
    if !point.components.iter().all(|bits| f32::from_bits(*bits).is_finite()) {
        return Err(unsupported("nonfinite point"));
    }
    if x.abs() > (1u32 << 29) as f32 || y.abs() > (1u32 << 29) as f32 {
        return Err(unsupported("point outside safe editable coordinate bounds"));
    }
    Ok(Point::new(f64::from(x), f64::from(y)))
}
fn copy_string(value: &str) -> Result<String, IoError> {
    let mut out = String::new();
    out.try_reserve_exact(value.len()).map_err(|_| unsupported("name allocation"))?;
    out.push_str(value);
    Ok(out)
}
// Derived evidence, deliberately not presented as original GIMP record bytes.
// PCXPATH1, source-layout u32 (0 modern/1 VECTORS/2 PATHS), legacy-format u32,
// legacy-state u32 (MAX absent), stroke-count u32; each stroke has closed/count
// u32, each point kind/axis-count/integer-present u32, optional exact i32 xy,
// then the original f32 axis bits. All integers are big endian.
fn geometry_evidence(source: &VectorPath, ctl: &Interrupt<'_>, charge: &mut dyn FnMut(usize) -> Result<(), IoError>) -> Result<Vec<u8>, IoError> {
    let mut size = 24usize;
    for stroke in &source.strokes {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        size = size.checked_add(8).ok_or_else(|| unsupported("geometry evidence overflow"))?;
        for point in &stroke.points {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let bytes = point
                .components
                .len()
                .checked_mul(4)
                .and_then(|n| n.checked_add(12 + if point.legacy_integer_coordinates.is_some() { 8 } else { 0 }))
                .ok_or_else(|| unsupported("geometry evidence overflow"))?;
            size = size.checked_add(bytes).ok_or_else(|| unsupported("geometry evidence overflow"))?;
        }
    }
    if size > 64 << 20 {
        return Err(unsupported("geometry evidence size limit"));
    }
    charge(size.checked_add(std::mem::size_of::<OpaqueRecord>() + 128).ok_or_else(|| unsupported("geometry evidence overflow"))?)?;
    let mut bytes = Vec::new();
    reserve(&mut bytes, size)?;
    bytes.extend_from_slice(b"PCXPATH1");
    let layout = if source.legacy_format.is_some() {
        2u32
    } else if source.raw_record.is_empty() {
        1
    } else {
        0
    };
    for value in [
        layout,
        source.legacy_format.unwrap_or(0),
        source.legacy_state.map(u32::from).unwrap_or(u32::MAX),
        u32::try_from(source.strokes.len()).map_err(|_| unsupported("stroke count overflow"))?,
    ] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    for stroke in &source.strokes {
        for value in [u32::from(stroke.closed), u32::try_from(stroke.points.len()).map_err(|_| unsupported("point count overflow"))?] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        for point in &stroke.points {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            for value in [
                point.kind,
                u32::try_from(point.components.len()).map_err(|_| unsupported("axis count overflow"))?,
                u32::from(point.legacy_integer_coordinates.is_some()),
            ] {
                bytes.extend_from_slice(&value.to_be_bytes());
            }
            if let Some(xy) = point.legacy_integer_coordinates {
                for value in xy {
                    bytes.extend_from_slice(&value.to_be_bytes());
                }
            }
            for bits in &point.components {
                bytes.extend_from_slice(&bits.to_be_bytes());
            }
        }
    }
    Ok(bytes)
}
/// The charge callback shares the import transaction's cumulative resident budget.
/// No source tattoo or source array index becomes a canonical identity.
pub fn map_paths(
    paths: Vec<VectorPath>,
    version: u32,
    ctl: &Interrupt<'_>,
    charge: &mut dyn FnMut(usize) -> Result<(), IoError>,
) -> Result<MappedPaths, IoError> {
    if paths.len() > 10000 {
        return Err(unsupported("path count limit"));
    }
    let mut out = MappedPaths { paths: Vec::new(), items: Vec::new(), selected: Vec::new(), memberships: Vec::new(), tattoos: Vec::new(), opaque: Vec::new() };
    let mut seen = std::collections::HashSet::new();
    let mut total_points = 0usize;
    for (order, source) in paths.into_iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if !source.unsupported.is_empty() {
            return Err(unsupported("unknown required source fields"));
        }
        if source.legacy_format.is_some_and(|v| v != 3) {
            return Err(unsupported("unqualified legacy path epoch"));
        }
        if source.name.len() > 1 << 20 || source.name.contains('\0') {
            return Err(unsupported("name limit"));
        }
        if source.strokes.len() > 10000 || source.item_sets.len() > 10000 {
            return Err(unsupported("stroke or membership count"));
        }
        charge(1024usize.checked_add(source.name.len().saturating_mul(4)).ok_or_else(|| unsupported("resident overflow"))?)?;
        if let Some(tattoo) = source.tattoo {
            seen.try_reserve(1).map_err(|_| unsupported("tattoo allocation"))?;
            if !seen.insert(tattoo) {
                return Err(unsupported("duplicate source tattoo"));
            }
        }
        let mut subpaths = Vec::new();
        charge(source.strokes.len().checked_mul(std::mem::size_of::<Subpath>()).ok_or_else(|| unsupported("stroke bytes overflow"))?)?;
        reserve(&mut subpaths, source.strokes.len())?;
        for stroke in &source.strokes {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            if source.legacy_format == Some(3) && stroke.points.iter().any(|point| point.components.len() != 2) {
                return Err(unsupported("legacy PATHS format 3 requires exactly XY"));
            }
            total_points = total_points.checked_add(stroke.points.len()).ok_or_else(|| unsupported("point count overflow"))?;
            let legacy = source.legacy_format == Some(3);
            let complete = if legacy && !stroke.closed {
                stroke.points.len() >= 2 && stroke.points.len().checked_add(1).is_some_and(|n| n.is_multiple_of(3))
            } else {
                stroke.points.len().is_multiple_of(3)
            };
            if total_points > 1_000_000 || !complete {
                return Err(unsupported("point count or incomplete control/anchor triple"));
            }
            let knots_count = if legacy && !stroke.closed { stroke.points.len() / 3 + 1 } else { stroke.points.len() / 3 };
            charge(
                knots_count
                    .checked_mul(std::mem::size_of::<Knot>())
                    .and_then(|v| v.checked_add(std::mem::size_of::<Subpath>()))
                    .ok_or_else(|| unsupported("geometry bytes overflow"))?,
            )?;
            let mut knots = Vec::new();
            reserve(&mut knots, knots_count)?;
            for index in 0..knots_count {
                ctl.check().map_err(|_| IoError::Cancelled)?;
                let base = index.checked_mul(3).ok_or_else(|| unsupported("point index overflow"))?;
                let (incoming, center, outgoing, expected) = if legacy {
                    // Observed PATHS format3 is anchor/out/next-in. An open stroke
                    // omits the first incoming handle; fresh GIMP exposes it at the anchor.
                    let center = stroke.points.get(base).ok_or_else(|| unsupported("missing anchor"))?;
                    let outgoing = stroke.points.get(base + 1).ok_or_else(|| unsupported("missing outgoing control"))?;
                    let incoming = if index > 0 {
                        stroke.points.get(base - 1)
                    } else if stroke.closed {
                        stroke.points.last()
                    } else {
                        Some(center)
                    }
                    .ok_or_else(|| unsupported("missing incoming control"))?;
                    (incoming, center, outgoing, (if index == 0 && !stroke.closed { 1 } else { 2 }, 1, 2))
                } else {
                    (
                        stroke.points.get(base).ok_or_else(|| unsupported("missing incoming control"))?,
                        stroke.points.get(base + 1).ok_or_else(|| unsupported("missing anchor"))?,
                        stroke.points.get(base + 2).ok_or_else(|| unsupported("missing outgoing control"))?,
                        (1, 0, 1),
                    )
                };
                if (incoming.kind, center.kind, outgoing.kind) != expected {
                    return Err(unsupported("unqualified point-kind ordering"));
                }
                knots.push(Knot { in_ctrl: coordinate(incoming)?, anchor: coordinate(center)?, out_ctrl: coordinate(outgoing)?, smooth: false });
            }
            subpaths.push(Subpath { closed: stroke.closed, knots, op: PathOp::Combine });
        }
        let named = NamedPath::new(copy_string(&source.name)?, Path::new(subpaths));
        let id = named.id;
        let item = ItemRef::Path(id);
        let geometry = geometry_evidence(&source, ctl, charge)?;
        let mut geometry_hash = blake3::Hasher::new();
        for chunk in geometry.chunks(4096) {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            geometry_hash.update(chunk);
        }
        reserve(&mut out.opaque, 1)?;
        out.opaque.push(OpaqueRecord {
            owner: Some(item),
            namespace: "photocraft/xcf/derived-path-geometry-v1".into(),
            identity: copy_string(&source.name)?,
            source_version: version,
            order: u32::try_from(order).map_err(|_| unsupported("source order overflow"))?,
            flags: 0,
            bytes: std::sync::Arc::new(geometry),
            blake3: geometry_hash.finalize().to_hex().to_string(),
            rendering_required: false,
        });
        reserve(&mut out.paths, 1)?;
        reserve(&mut out.items, 1)?;
        out.items.push(ItemState {
            item,
            name: copy_string(&source.name)?,
            custom_name: true,
            visible: source.visible,
            locks: ItemLocks { content: source.lock_content, position: source.lock_position, visibility: source.lock_visibility, ..Default::default() },
            parent: None,
            source_tattoo: source.tattoo,
            source_flags: u32::from(source.legacy_linked),
            raster_bounds: None,
            mask: None,
            composition: None,
        });
        if source.selected {
            reserve(&mut out.selected, 1)?;
            out.selected.push(item);
        }
        if let Some(tattoo) = source.tattoo {
            reserve(&mut out.tattoos, 1)?;
            out.tattoos.push((tattoo, id));
        }
        charge(source.item_sets.len().checked_mul(std::mem::size_of::<(u32, ItemRef)>()).ok_or_else(|| unsupported("membership bytes overflow"))?)?;
        for index in &source.item_sets {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            reserve(&mut out.memberships, 1)?;
            out.memberships.push((*index, item));
        }
        // Modern raw bytes contain original axes/kinds/parasites; legacy complete
        // bytes remain in the import's image PATHS/VECTORS property evidence.
        if !source.raw_record.is_empty() {
            if source.raw_record.len() > 64 << 20 {
                return Err(unsupported("raw record limit"));
            }
            let mut hash = blake3::Hasher::new();
            for chunk in source.raw_record.chunks(4096) {
                ctl.check().map_err(|_| IoError::Cancelled)?;
                hash.update(chunk);
            }
            reserve(&mut out.opaque, 1)?;
            out.opaque.push(OpaqueRecord {
                owner: Some(item),
                namespace: "gimp/xcf/source-path".into(),
                identity: copy_string(&source.name)?,
                source_version: version,
                order: u32::try_from(order).map_err(|_| unsupported("source order overflow"))?,
                flags: 0,
                bytes: std::sync::Arc::new(source.raw_record),
                blake3: hash.finalize().to_hex().to_string(),
                rendering_required: false,
            });
        }
        out.paths.push(named);
    }
    Ok(out)
}
