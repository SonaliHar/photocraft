//! Ignored UNRUN/uncompiled format-only hierarchy admission. Malformed metadata
//! clones are paid external harnesses, not producer or renderer evidence.
use super::*;
use std::cell::Cell;
struct NoPayload<'a> {
    calls: &'a Cell<usize>,
}
impl crate::convert::Fetch for NoPayload<'_> {
    fn finite_raster(&mut self, _: &crate::finite_raster::FiniteRasterM) -> Result<photocraft_doc::BackedLayer> {
        self.calls.set(self.calls.get() + 1);
        Err(FormatError::Unsupported("payload reached".into()))
    }
    fn tile(&mut self, _: &str, _: usize) -> Result<Vec<u8>> {
        self.calls.set(self.calls.get() + 1);
        Err(FormatError::Unsupported("payload reached".into()))
    }
    fn blob(&mut self, _: &str) -> Result<Arc<Vec<u8>>> {
        self.calls.set(self.calls.get() + 1);
        Err(FormatError::Unsupported("payload reached".into()))
    }
}
fn selected(m: &crate::manifest::DocM, q: &BackingQuota, calls: &Cell<usize>) -> Result<Document> {
    let mut fetch = NoPayload { calls };
    let mut loader = crate::convert::Loader {
        fetch: &mut fetch,
        preserve_ids: true,
        max_id: 0,
        id_map: Default::default(),
        channel_map: Default::default(),
        path_map: Default::default(),
        contents_id_map: Default::default(),
        legacy_linked_contents: Default::default(),
        tiles: Default::default(),
    };
    loader.document_with_normal_hierarchy_control(m, &ctl(), q)
}
fn group(mut layer: crate::manifest::LayerM, id: u64, children: Vec<crate::manifest::LayerM>) -> crate::manifest::LayerM {
    layer.id = id;
    layer.name = format!("group{id}");
    layer.content = crate::manifest::ContentM::Group { children, expanded: false, artboard: None };
    layer
}
#[test]
#[ignore = "UNRUN/uncompiled: selected13 SurfaceM16/64MiB/depth admission before any payload, old generic path unchanged"]
fn mask_aggregate_and_tree_header_precede_payload() {
    let q = quota();
    let original = point(&q, SampleType::F64, true);
    let memory = save_accounted_point_f32_coverage_document_archive(&original, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    let _external = q.reserve(QuotaKind::Index, 4 << 20).unwrap();
    let mut m = manifest(&memory).document;
    let leaf = m.layers[0].clone();
    m.layers = vec![group(leaf.clone(), 99, vec![leaf.clone()])];
    let mask = m.xcf_filter_masks[0].clone();
    let tile = mask.surface.tiles[0].clone();
    let calls = Cell::new(0);
    m.xcf_filter_masks[0].surface.tiles.resize(32, tile.clone());
    assert!(selected(&m, &q, &calls).is_err());
    assert_eq!(calls.get(), 0);
    m.xcf_filter_masks = vec![mask.clone(); 5];
    for m in &mut m.xcf_filter_masks {
        m.surface.tiles.resize(31, tile.clone());
    }
    assert!(selected(&m, &q, &calls).is_err());
    assert_eq!(calls.get(), 0);
    m.xcf_filter_masks = vec![mask];
    let mut chain = leaf.clone();
    for id in 100..133 {
        chain = group(leaf.clone(), id, vec![chain]);
    }
    m.layers = vec![chain];
    assert!(selected(&m, &q, &calls).is_err());
    assert_eq!(calls.get(), 0);
    m.layers = vec![group(leaf.clone(), 99, vec![leaf])];
    crate::normal_hierarchy::preflight(&m, &ctl()).unwrap();
    assert!(selected(&m, &q, &calls).is_err());
    assert!(calls.get() > 0);
}
#[test]
#[ignore = "UNRUN/uncompiled: original13 floors before migration,12 no-invention, raw/unmarked purpose refusal"]
fn original_floor_and_no_invention() {
    let q = quota();
    let owner = current_empty(&q, SampleType::F32);
    let memory = save_current_document_archive(owner.document(), Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    for (format, minimum) in [(12, 12), (12, 13), (13, 12)] {
        let bad = rewritten_header(&memory, &q, |v| {
            v["format_version"] = format.into();
            v["minimum_reader_version"] = minimum.into();
            v["current_storage_hint"] = "normal_hierarchy".into();
        });
        assert!(load(bad, &q).is_err());
    }
    let v12 = rewritten_header(&memory, &q, |v| {
        v["format_version"] = 12.into();
        v["minimum_reader_version"] = 5.into();
        v.as_object_mut().unwrap().remove("current_storage_hint");
    });
    let archive = load(v12, &q).unwrap();
    assert_eq!(archive.current_storage_hint(), None);
    assert!(archive.into_accounted_readonly_normal_hierarchy_storage(&ctl()).is_err());
    let marked = rewritten_header(&memory, &q, |v| {
        v["format_version"] = 13.into();
        v["minimum_reader_version"] = 13.into();
        v["current_storage_hint"] = "normal_hierarchy".into();
    });
    assert!(crate::load_from_bytes(&marked.bytes).is_err()); // raw loader never guesses selected class
    let mut raw = serde_json::json!({"format_version":12,"minimum_reader_version":5,"document":{}});
    crate::migrate::migrate(&mut raw).unwrap();
    assert_eq!(raw["format_version"], 13);
    assert!(raw.get("current_storage_hint").is_none());
}
