//! Ignored UNRUN format-only preallocation/floor contracts. Uses paid fixture
//! metadata and external malformed manifests; no Source/evaluator grant.
use super::*;
use std::cell::Cell;
struct NoPayload<'a> {
    calls: &'a Cell<usize>,
}
impl crate::convert::Fetch for NoPayload<'_> {
    fn finite_raster(&mut self, _: &crate::finite_raster::FiniteRasterM) -> Result<photocraft_doc::BackedLayer> {
        self.calls.set(self.calls.get() + 1);
        Err(FormatError::Unsupported("payload must not be reached".into()))
    }
    fn tile(&mut self, _: &str, _: usize) -> Result<Vec<u8>> {
        self.calls.set(self.calls.get() + 1);
        Err(FormatError::Unsupported("payload must not be reached".into()))
    }
    fn blob(&mut self, _: &str) -> Result<Arc<Vec<u8>>> {
        self.calls.set(self.calls.get() + 1);
        Err(FormatError::Unsupported("payload must not be reached".into()))
    }
}
fn selected_load(m: &crate::manifest::DocM, q: &BackingQuota, calls: &Cell<usize>) -> Result<photocraft_doc::Document> {
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
    loader.document_with_mask_set_control(m, &ctl(), q)
}
#[test]
#[ignore = "UNRUN/uncompiled: SurfaceM descriptor cap rejects before ANY fetch/surface allocation; admissible physical descriptor alone is not a valid CURRENT proof"]
fn selected_manifest_preflight_precedes_all_payload_reads() {
    let q = quota();
    let original = point(&q, SampleType::F64, true);
    let memory = save_accounted_point_f32_coverage_document_archive(&original, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    // Real external metadata envelope precedes malformed DocM/map/String clones;
    // shared hash references are not new mask payload allocations.
    let _harness = q.reserve(QuotaKind::Index, 4 << 20).unwrap();
    let mut m = manifest(&memory).document;
    let template = m.xcf_filter_masks[0].clone();
    let tile = template.surface.tiles[0].clone();
    let calls = Cell::new(0);
    m.xcf_filter_masks[0].surface.tiles.resize(32, tile.clone());
    assert!(selected_load(&m, &q, &calls).is_err());
    assert_eq!(calls.get(), 0); //16MiB+default >permask
    m.xcf_filter_masks = vec![template.clone(); 5];
    for mask in &mut m.xcf_filter_masks {
        mask.surface.tiles.resize(31, tile.clone());
    }
    assert!(selected_load(&m, &q, &calls).is_err());
    assert_eq!(calls.get(), 0); //5*15.5MiB >64MiB
    m.xcf_filter_masks.truncate(4);
    crate::masked_filtered_flat::preflight(&m, &ctl()).unwrap();
    assert!(selected_load(&m, &q, &calls).is_err());
    assert!(calls.get() > 0); //preflight admits62MiB, actual later payload still refuses
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(crate::masked_filtered_flat::preflight(&m, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    let work = |_| Err(BackingError::Work);
    assert!(crate::masked_filtered_flat::preflight(&m, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
}
#[test]
#[ignore = "UNRUN/uncompiled: original format/minreader12 checked before migration, no invention from11, and unmarked archives cannot handoff into newclass"]
fn original_floor_and_unmarked_intent_remain_closed() {
    let q = quota();
    let original = point(&q, SampleType::F64, true);
    let memory = save_accounted_point_f32_coverage_document_archive(&original, Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    for (format, minimum) in [(11, 11), (11, 12), (12, 11)] {
        let forged = rewritten_header(&memory, &q, |v| {
            v["format_version"] = format.into();
            v["minimum_reader_version"] = minimum.into();
            v["current_storage_hint"] = "masked_filtered_flat_point".into();
        });
        assert!(load(forged, &q).is_err());
    }
    // Explicit selected marker is only structural intent; standalone point data
    // may fit CURRENT structure, but supplies no mixed Source history or program.
    let marked = rewritten_header(&memory, &q, |v| {
        v["format_version"] = 12.into();
        v["minimum_reader_version"] = 12.into();
        v["current_storage_hint"] = "masked_filtered_flat_point".into();
    });
    assert!(crate::load_from_bytes(&marked.bytes).is_err());
    assert!(load(marked.clone(), &q).unwrap().into_accounted_readonly_point_storage(&ctl()).is_err());
    let current = load(marked, &q).unwrap().into_accounted_readonly_masked_filtered_flat_storage(&ctl()).unwrap();
    assert!(current.is_readonly_masked_filtered_flat_point_storage());
    assert!(!current.is_readonly_filtered_flat_point_storage());
    let empty = current_empty(&q, SampleType::F32);
    let ordinary = save_current_document_archive(empty.document(), Writer::new(&q), Default::default(), &q, &ctl()).unwrap().finish();
    let v11 = rewritten_header(&ordinary, &q, |v| {
        v["format_version"] = 11.into();
        v["current_storage_hint"] = serde_json::Value::Null;
    });
    let archive = load(v11, &q).unwrap();
    assert_eq!(archive.current_storage_hint(), None);
    assert!(archive.into_accounted_readonly_masked_filtered_flat_storage(&ctl()).is_err());
    let mut raw = serde_json::json!({"format_version":11,"minimum_reader_version":11,"document":{}});
    crate::migrate::migrate(&mut raw).unwrap();
    assert_eq!(raw["format_version"], 12);
    assert!(raw.get("current_storage_hint").is_none());
}
