//! Generated Source-first all-depth hierarchy fixture. Not GIMP evidence.
//! Encoded/native/comparison buffers are external harness data; genuine reader
//! owners retain actual decoder/model/native quota receipts.
#![allow(dead_code, unused_imports)]
#[path = "../../io/src/xcf_import/plain_quantized_hierarchy_fixture.rs"]
pub mod data;
#[path = "accounted_normal_hierarchy_test_support.rs"]
mod old;
pub use old::{archive, ctl, encoded, raw, same, storage};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, SampleType, item_identity::ItemRef};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits, QuotaUsage};
use std::sync::Arc;
pub fn sample(depth: usize) -> SampleType {
    match depth {
        0 => SampleType::U8,
        1 => SampleType::U16,
        2 => SampleType::U32,
        3 => SampleType::F32,
        4 => SampleType::F64,
        _ => panic!("fixture depth"),
    }
}
pub fn precision(depth: usize) -> u32 {
    match depth {
        0 => 150,
        1 => 250,
        2 => 350,
        3 => 650,
        4 => 750,
        _ => panic!("fixture depth"),
    }
}
pub fn bytes(depth: usize) -> Vec<u8> {
    data::image_paint(depth)
}
pub fn import(depth: usize) -> photocraft_io::OwnedXcfImport {
    let q =
        BackingQuota::new_with_aggregate_initial(QuotaLimits { resident: 512 << 20, index: 64 << 20, scratch: 0 }, 512 << 20, QuotaUsage::default()).unwrap().0;
    let imported = photocraft_io::import_owned_xcf_bytes_in_quota(
        "generated-plain-hierarchy.xcf",
        Arc::new(bytes(depth)),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        q,
        &ctl(),
    )
    .unwrap();
    assert!(matches!(imported, photocraft_io::OwnedXcfImport::NormalHierarchy(_)));
    observed(imported.document().document(), depth);
    imported
}
pub fn layers(doc: &Document) -> Vec<&Layer> {
    let mut todo: Vec<&Layer> = doc.layers.iter().collect();
    let mut out = Vec::new();
    while let Some(l) = todo.pop() {
        out.push(l);
        if let LayerContent::Group(g) = &l.content {
            todo.extend(g.children.iter());
        }
    }
    out
}
pub fn named<'a>(doc: &'a Document, name: &str) -> &'a Layer {
    layers(doc).into_iter().find(|l| l.name == name).unwrap()
}
pub fn target_leaf(doc: &Document) -> LayerId {
    named(doc, "LeafA").id
}
pub fn ids(doc: &Document) -> (LayerId, LayerId, LayerId) {
    (named(doc, "Group").id, target_leaf(doc), named(doc, "Empty").id)
}
/// Per-actual-leaf native bytes; ordering/IDs remain actual immutable tree data.
pub fn payload(doc: &Document) -> Vec<(LayerId, Vec<u8>)> {
    layers(doc).into_iter().filter(|l| matches!(l.content, LayerContent::BackedRaster(_))).map(|l| (l.id, raw(l))).collect()
}
pub fn source_payload(a: &Document, b: &Document) {
    old::payload(a, b)
}
pub fn observed(doc: &Document, depth: usize) {
    assert_eq!(doc.mode, photocraft_doc::ColorMode::Rgb);
    assert_eq!(doc.depth, sample(depth));
    assert_eq!(doc.size, photocraft_geom::Size::new(257, 259));
    assert_eq!(doc.sample_encoding, Some(photocraft_doc::SampleEncoding::NonlinearProfile));
    let c = doc.source_color_context.as_ref().unwrap();
    assert_eq!(c.source_precision, precision(depth));
    assert!(c.original_icc.is_none());
    let s = doc.xcf_state.as_ref().unwrap();
    assert_eq!(s.provenance.format_version, 12);
    assert!(s.filter_masks.is_empty());
    assert!(s.filter_stacks.iter().all(|s| s.nodes.is_empty()));
    let all = layers(doc);
    assert_eq!(all.len(), 6);
    for l in all {
        let row = s.items.iter().find(|r| r.item == ItemRef::Layer(l.id)).unwrap();
        if let LayerContent::Group(g) = &l.content {
            assert!(row.raster_bounds.is_none());
            if l.name == "Empty" {
                assert!(g.children.is_empty());
            }
        } else {
            let LayerContent::BackedRaster(b) = &l.content else { panic!("fixture native") };
            assert_eq!(b.format().sample, sample(depth));
            assert_eq!(b.operative_bounds(), doc.bounds());
            let leaf = match l.name.as_str() {
                "LeafA" => 0,
                "LeafB" => 1,
                "Background" => 2,
                _ => panic!("fixture leaf"),
            };
            assert_eq!(raw(l), data::native(depth, 2, leaf));
        }
    }
    let a = named(doc, "LeafA");
    assert!(a.visible);
    assert_eq!(a.opacity.to_bits(), 0.375f32.to_bits());
    let inner = named(doc, "Inner");
    assert_eq!(s.items.iter().find(|r| r.item == ItemRef::Layer(a.id)).unwrap().parent, Some(ItemRef::Layer(inner.id)));
    assert_ne!(doc.layers.first().unwrap().id, a.id);
    assert!(!named(doc, "LeafB").visible);
}

/// Explicit paid CURRENT copy only: retain an empty target stack revision anchor.
/// The actual Source helper above does not invent a stack/source receipt.
pub fn anchored(owner: &photocraft_doc::accounted_document::AccountedDocument) -> photocraft_doc::accounted_document::AccountedDocument {
    use photocraft_doc::xcf_filters::FilterStack;
    use photocraft_raster::backing::quota::QuotaKind;
    let q = owner.quota().clone();
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mut d = owner.document().clone();
    let leaf = target_leaf(&d);
    let s = d.xcf_state.as_mut().unwrap();
    assert!(s.filter_stacks.iter().all(|s| s.nodes.is_empty()));
    if let Some(stack) = s.filter_stacks.iter_mut().find(|s| s.owner == ItemRef::Layer(leaf)) {
        stack.revision = 7;
    } else {
        s.filter_stacks.push(FilterStack { owner: ItemRef::Layer(leaf), nodes: Vec::new(), revision: 7 });
    }
    photocraft_doc::accounted_document::AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(
        d,
        Arc::new((owner.clone(), index, resident)),
        q,
        &ctl(),
    )
    .unwrap()
}
