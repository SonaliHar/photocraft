//! Bounded derived Paths-panel rows; never an edit or resource capability.
use crate::vector_ui::{PathEntry, PathRow};
use photocraft_doc::{DocId, Document, Layer, LayerContent, LayerId, Path, Subpath};
use photocraft_engine::document_snapshot::{DocumentSnapshot, WeakDocumentSnapshot};
use std::sync::Arc;

const MAX_PATHS: usize = 10_000;
const MAX_COMPONENTS: usize = 100_000;
const MAX_KNOTS: usize = 100_000;
const MAX_NAME: usize = 4096;
const MAX_NAMES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CacheError {
    Limit,
    Allocation,
    Geometry,
}
impl std::fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Limit => "paths.rows.limit",
            Self::Allocation => "paths.rows.allocation",
            Self::Geometry => "paths.rows.geometry",
        })
    }
}
#[derive(Clone)]
struct Cache {
    document: DocId,
    revision: u64,
    active: Option<LayerId>,
    snapshot: WeakDocumentSnapshot,
    result: Result<Arc<Vec<PathEntry>>, CacheError>,
}
impl Cache {
    fn matches(&self, snapshot: &DocumentSnapshot, revision: u64, active: Option<LayerId>) -> bool {
        self.document == snapshot.id
            && self.revision == revision
            && self.active == active
            && self.snapshot.upgrade().is_some_and(|original| original.shares_state(snapshot))
    }
}
#[derive(Default)]
struct Census {
    components: usize,
    knots: usize,
    names: usize,
}
impl Census {
    fn path(&mut self, path: &Path) -> Result<Path, CacheError> {
        self.components = self.components.checked_add(path.subpaths.len()).filter(|n| *n <= MAX_COMPONENTS).ok_or(CacheError::Limit)?;
        let mut subpaths = Vec::new();
        subpaths.try_reserve_exact(path.subpaths.len()).map_err(|_| CacheError::Allocation)?;
        for subpath in &path.subpaths {
            self.knots = self.knots.checked_add(subpath.knots.len()).filter(|n| *n <= MAX_KNOTS).ok_or(CacheError::Limit)?;
            if subpath
                .knots
                .iter()
                .any(|knot| [knot.anchor.x, knot.anchor.y, knot.in_ctrl.x, knot.in_ctrl.y, knot.out_ctrl.x, knot.out_ctrl.y].iter().any(|v| !v.is_finite()))
            {
                return Err(CacheError::Geometry);
            }
            let mut knots = Vec::new();
            knots.try_reserve_exact(subpath.knots.len()).map_err(|_| CacheError::Allocation)?;
            knots.extend_from_slice(&subpath.knots);
            subpaths.push(Subpath { closed: subpath.closed, knots, op: subpath.op });
        }
        Ok(Path { subpaths, fill_rule: path.fill_rule, inverted: path.inverted })
    }
    fn name(&mut self, prefix: &str, suffix: &str) -> Result<String, CacheError> {
        let bytes = prefix.len().checked_add(suffix.len()).filter(|n| *n <= MAX_NAME).ok_or(CacheError::Limit)?;
        self.names = self.names.checked_add(bytes).filter(|n| *n <= MAX_NAMES).ok_or(CacheError::Limit)?;
        let mut name = String::new();
        name.try_reserve_exact(bytes).map_err(|_| CacheError::Allocation)?;
        name.push_str(prefix);
        name.push_str(suffix);
        Ok(name)
    }
}
fn active_layer<'a>(layers: &'a [Layer], id: LayerId, depth: usize, visited: &mut usize, found: &mut Option<&'a Layer>) -> Result<(), CacheError> {
    if depth > 100 {
        return Err(CacheError::Limit);
    }
    for layer in layers {
        *visited = visited.checked_add(1).filter(|n| *n <= MAX_PATHS).ok_or(CacheError::Limit)?;
        if layer.id == id {
            if found.is_some() {
                return Err(CacheError::Geometry);
            }
            *found = Some(layer);
        }
        if let Some(children) = layer.children() {
            active_layer(children, id, depth + 1, visited, found)?;
        }
    }
    Ok(())
}
fn prepare(document: &Document, active: Option<LayerId>) -> Result<Vec<PathEntry>, CacheError> {
    if document.paths.len() > MAX_PATHS {
        return Err(CacheError::Limit);
    }
    let mut selected = None;
    if let Some(id) = active {
        let mut visited = 0;
        active_layer(&document.layers, id, 0, &mut visited, &mut selected)?;
    }
    let layer_path = selected.and_then(|layer| match &layer.content {
        LayerContent::Shape(shape) => Some((layer.name.as_str(), " Shape Path", &shape.path)),
        _ => layer.vector_mask.as_ref().map(|mask| (layer.name.as_str(), " Vector Mask", &mask.path)),
    });
    let count = document
        .paths
        .len()
        .checked_add(usize::from(document.work_path.is_some()))
        .and_then(|n| n.checked_add(usize::from(layer_path.is_some())))
        .filter(|n| *n <= MAX_PATHS)
        .ok_or(CacheError::Limit)?;
    let mut rows = Vec::new();
    rows.try_reserve_exact(count).map_err(|_| CacheError::Allocation)?;
    let mut census = Census::default();
    for saved in &document.paths {
        rows.push(PathEntry { id: Some(saved.id), name: census.name(&saved.name, "")?, path: census.path(&saved.path)?, kind: PathRow::Saved });
    }
    if let Some(path) = &document.work_path {
        rows.push(PathEntry { id: None, name: census.name("Work Path", "")?, path: census.path(path)?, kind: PathRow::Work });
    }
    if let Some((name, suffix, path)) = layer_path {
        rows.push(PathEntry { id: None, name: census.name(name, suffix)?, path: census.path(path)?, kind: PathRow::Layer });
    }
    Ok(rows)
}

/// Single context-local entry; unchanged frames share row geometry. Errors are
/// cached too, so rejected metadata is not rescanned every frame. Arc/egui's
/// small ownership allocations remain framework overhead, not an RSS claim.
pub(crate) fn rows(context: &egui::Context, snapshot: &DocumentSnapshot, revision: u64, active: Option<LayerId>) -> Result<Arc<Vec<PathEntry>>, CacheError> {
    let key = egui::Id::new("vector_path_rows_cache");
    if let Some(cache) = context.data_mut(|data| data.get_temp::<Cache>(key))
        && cache.matches(snapshot, revision, active)
    {
        return cache.result;
    }
    let result = prepare(snapshot, active).map(Arc::new);
    let cache = Cache { document: snapshot.id, revision, active, snapshot: snapshot.downgrade(), result: result.clone() };
    context.data_mut(|data| data.insert_temp(key, cache));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> Document {
        Document::new("Own cache", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8)
    }
    #[test]
    fn matching_snapshot_shares_geometry_revision_and_active_invalidate() {
        let mut doc = document();
        doc.paths.push(photocraft_doc::NamedPath::new("Own", Path::new(vec![Subpath::polyline(&[(0., 0.), (1., 1.)])])));
        let snapshot = DocumentSnapshot::from_resident(Arc::new(doc)).unwrap();
        let ctx = egui::Context::default();
        let first = rows(&ctx, &snapshot, 1, None).unwrap();
        assert!(Arc::ptr_eq(&first, &rows(&ctx, &snapshot, 1, None).unwrap()));
        assert!(!Arc::ptr_eq(&first, &rows(&ctx, &snapshot, 2, None).unwrap()));
        assert!(!Arc::ptr_eq(&first, &rows(&ctx, &snapshot, 1, Some(LayerId(9))).unwrap()));
    }
    #[test]
    fn oversized_metadata_refuses_without_mutating_source() {
        let mut doc = document();
        doc.paths.push(photocraft_doc::NamedPath::new("x".repeat(MAX_NAME + 1), Path::default()));
        let before = doc.clone();
        assert!(matches!(prepare(&doc, None), Err(CacheError::Limit)));
        assert_eq!(doc, before);
        let mut census = Census::default();
        let mut path = Path::new(vec![Subpath::default()]);
        path.subpaths[0].knots = vec![photocraft_doc::Knot::corner(0., 0.); MAX_KNOTS + 1];
        assert!(matches!(census.path(&path), Err(CacheError::Limit)));
    }
}
