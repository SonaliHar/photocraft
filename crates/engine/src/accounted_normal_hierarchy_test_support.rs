//! Deferred host fixture: genuine retained native leaves wrapped in a synthetic
//! CURRENT tree under prospective metadata guards. Never a hierarchy Source claim.
#![allow(dead_code, unused_imports)]
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "accounted_filtered_flat_ordered_test_support.rs"]
mod base;
pub use base::{ctl, workspace};
use photocraft_doc::{
    BlendMode, Document, Layer, LayerContent, LayerId, SampleEncoding,
    accounted_document::AccountedDocument,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ImportCompositing, ResolvedGimpComposition},
    item_identity::ItemRef,
    xcf_state::ItemState,
};
use photocraft_raster::backing::quota::QuotaKind;
use std::sync::Arc;
pub const OUTER: LayerId = LayerId(900001);
pub const INNER: LayerId = LayerId(900002);
pub const EMPTY: LayerId = LayerId(900003);
fn group(id: LayerId, name: &str, children: Vec<Layer>) -> Layer {
    let mut l = Layer::group(name, children);
    l.id = id;
    l.blend = BlendMode::Normal;
    l.import_compositing = Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    l
}
fn composition() -> GimpComposition {
    GimpComposition {
        compatibility_epoch: "xcf-22-normal".into(),
        source_mode: 28,
        legacy: false,
        stored_blend_space: 0,
        stored_composite_space: -1,
        stored_operator: -1,
        resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
        pass_through: false,
    }
}
fn rows(layers: &[Layer], parent: Option<ItemRef>, template: &ItemState, bounds: photocraft_geom::Rect, out: &mut Vec<ItemState>) {
    for l in layers {
        let mut r = template.clone();
        r.item = ItemRef::Layer(l.id);
        r.name = l.name.clone();
        r.visible = l.visible;
        r.parent = parent;
        r.raster_bounds = if l.is_group() { None } else { Some(bounds) };
        r.mask = None;
        r.source_flags = 0;
        r.composition = Some(composition());
        out.push(r);
        if let LayerContent::Group(g) = &l.content {
            rows(&g.children, Some(ItemRef::Layer(l.id)), template, bounds, out)
        }
    }
}
pub fn build(depth: usize) -> AccountedDocument {
    let imported = base::import(base::data::CASES.get(depth).unwrap());
    let original = imported.document().clone();
    let q = original.quota().clone();
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mut d = original.document().clone();
    let mut leaves = std::mem::take(&mut d.layers);
    assert_eq!(leaves.len(), 4);
    let plain = leaves.remove(0);
    let first = leaves.remove(0);
    let inner = group(INNER, "Inner", leaves);
    let empty = group(EMPTY, "Empty", vec![]);
    let outer = group(OUTER, "Outer", vec![first, inner, empty]);
    d.layers = vec![plain, outer];
    let bounds = d.bounds();
    let s = d.xcf_state.as_mut().unwrap();
    let template = s.items.iter().find(|r| matches!(r.item, ItemRef::Layer(_))).unwrap().clone();
    let mut canonical = Vec::new();
    rows(&d.layers, None, &template, bounds, &mut canonical);
    canonical.extend(s.items.iter().filter(|r| matches!(r.item, ItemRef::Mask(_))).cloned());
    s.items = canonical;
    s.selection.selected = vec![ItemRef::Layer(OUTER)];
    s.selection.active = Some(ItemRef::Layer(OUTER));
    drop(template);
    AccountedDocument::new_readonly_normal_hierarchy_current_storage(d, Arc::new((original, index, resident)), q, &ctl()).unwrap()
}
pub fn changed(owner: &AccountedDocument, f: impl FnOnce(&mut Document)) -> AccountedDocument {
    let q = owner.quota().clone();
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mut d = owner.document().clone();
    f(&mut d);
    AccountedDocument::new_readonly_normal_hierarchy_current_storage(d, Arc::new((owner.clone(), index, resident)), q, &ctl()).unwrap()
}
// Encoded Cursor<Vec> and comparison archives are external test harness output;
// the subsequent public native reader creates its own genuine paid receipts.
pub fn encoded(owner: &AccountedDocument) -> Vec<u8> {
    photocraft_io::accounted_current_native::save_accounted_current_archive(owner, std::io::Cursor::new(Vec::new()), Default::default(), owner.quota(), &ctl())
        .unwrap()
        .into_inner()
}
pub fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let b = encoded(owner);
    let a = photocraft_io::accounted_native::open_accounted_native_from_reader(
        &mut std::io::Cursor::new(&b),
        b.len() as u64,
        Default::default(),
        owner.quota().clone(),
        &ctl(),
    )
    .unwrap();
    assert_eq!(a.current_storage_hint(), Some(photocraft_format::NativeCurrentStorageHint::NormalHierarchy));
    a
}
pub fn payload(a: &Document, b: &Document) {
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.source_color_context, b.source_color_context);
    let x = a.xcf_state.as_ref().unwrap();
    let y = b.xcf_state.as_ref().unwrap();
    assert_eq!(x.provenance, y.provenance);
    assert_eq!(x.opaque, y.opaque);
}
pub fn layer(owner: &AccountedDocument) -> LayerId {
    match owner.document().xcf_state.as_ref().unwrap().filter_stacks.last().unwrap().owner {
        ItemRef::Layer(id) => id,
        _ => panic!("fixture layer owner"),
    }
}

// Comparison buffers are external harness observations, never owner receipts.
pub fn raw(layer: &Layer) -> Vec<u8> {
    let LayerContent::BackedRaster(b) = &layer.content else { panic!("native fixture leaf") };
    let r = b.operative_bounds();
    let bpp = b.format().bytes_per_pixel();
    let mut out = vec![0; r.width() as usize * r.height() as usize * bpp];
    for coord in r.tiles() {
        let pin = b.checked_view().pin_tile(coord, &ctl()).unwrap();
        let rect = coord.rect().intersect(&r);
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * bpp;
                pin.read_pixel_native_into(x, y, &mut out[at..at + bpp], &ctl()).unwrap()
            }
        }
    }
    out
}
pub fn storage(a: &Document, b: &Document) {
    assert_eq!(a.size, b.size);
    assert_eq!(a.mode, b.mode);
    assert_eq!(a.depth, b.depth);
    assert_eq!(a.sample_encoding, b.sample_encoding);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    payload(a, b);
    assert_eq!(a.xcf_state, b.xcf_state);
    fn layers(a: &[Layer], b: &[Layer]) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.name, b.name);
            assert_eq!(a.visible, b.visible);
            assert_eq!(a.opacity.to_bits(), b.opacity.to_bits());
            match (&a.content, &b.content) {
                (LayerContent::Group(x), LayerContent::Group(y)) => {
                    assert_eq!(x.expanded, y.expanded);
                    layers(&x.children, &y.children)
                }
                (LayerContent::BackedRaster(x), LayerContent::BackedRaster(y)) => {
                    assert_eq!(x.descriptor(), y.descriptor());
                    assert_eq!(x.default_bytes(), y.default_bytes());
                    assert_eq!(raw(a), raw(b))
                }
                _ => panic!("fixture topology"),
            }
        }
    }
    layers(&a.layers, &b.layers)
}
pub fn same(a: &Document, b: &Document) {
    assert_eq!(a.name, b.name);
    storage(a, b)
}

// Genuine author packets stay separate from the synthetic CURRENT builder above.
fn digest(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(b))
}
pub fn cases() -> Vec<serde_json::Value> {
    let p = workspace().join("temp/sol1-normal-hierarchy-gimp-readiness-20261016/literal-index-v1.json");
    let b = std::fs::read(p).unwrap();
    assert!(b.len() < 128 << 10);
    assert_eq!(digest(&b), "852e72bec7dcef9714c4a1fc84f3f2708b7edec598fb170e8ebcc0aa226bb2e9");
    let d: serde_json::Value = serde_json::from_slice(&b).unwrap();
    let v = d["cases"].as_array().unwrap().clone();
    assert_eq!(v.len(), 12);
    v
}
fn binding(v: &serde_json::Value) -> Vec<u8> {
    let b = std::fs::read(v["path"].as_str().unwrap()).unwrap();
    assert_eq!(b.len() as u64, v["bytes"].as_u64().unwrap());
    assert_eq!(digest(&b), v["sha256"].as_str().unwrap());
    b
}
pub fn path(c: &serde_json::Value) -> std::path::PathBuf {
    c["source"]["path"].as_str().unwrap().into()
}
pub fn bytes(c: &serde_json::Value) -> Vec<u8> {
    binding(&c["source"])
}
pub fn import(c: &serde_json::Value) -> photocraft_io::OwnedXcfImport {
    let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 256 << 20,
        index: 64 << 20,
        scratch: 0,
    });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
        .unwrap();
    let mut options = photocraft_xcf::DecodeOptions::default();
    options.limits.resident_bytes = 256 << 20;
    options.limits.input_bytes = 16 << 20;
    let imported = photocraft_io::import_owned_xcf_bytes_in_quota("hierarchy.xcf", Arc::new(bytes(c)), &options, &limits, q, &ctl()).unwrap();
    assert!(matches!(imported, photocraft_io::OwnedXcfImport::NormalHierarchy(_)));
    observed(imported.document().document(), c);
    imported
}
pub fn observed(d: &Document, c: &serde_json::Value) {
    assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, u32::try_from(c["source_precision"].as_u64().unwrap()).unwrap());
    assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
    assert_eq!(d.xcf_state.as_ref().unwrap().provenance.format_version, u32::try_from(c["source_version"].as_u64().unwrap()).unwrap());
    let capture: serde_json::Value = serde_json::from_slice(&binding(&c["capture"])).unwrap();
    let packet = binding(&c["packet"]);
    let x = d.xcf_state.as_ref().unwrap();
    let mut actual = Vec::new();
    fn visit<'a>(layers: &'a [Layer], parent: Option<LayerId>, out: &mut Vec<(&'a Layer, Option<LayerId>)>) {
        for l in layers.iter().rev() {
            out.push((l, parent));
            if let LayerContent::Group(g) = &l.content {
                visit(&g.children, Some(l.id), out)
            }
        }
    }
    visit(&d.layers, None, &mut actual);
    let expected = c["expected_top_first"].as_array().unwrap();
    assert_eq!(actual.len(), expected.len());
    for ((l, parent), row) in actual.iter().zip(expected) {
        assert_eq!(l.name, row["name"].as_str().unwrap());
        assert_eq!(l.visible, row["visible"].as_bool().unwrap());
        assert_eq!(l.opacity.to_bits(), ((row["opacity_percent"].as_f64().unwrap() / 100.0f64) as f32).to_bits());
        let item = x.items.iter().find(|i| i.item == ItemRef::Layer(l.id)).unwrap();
        assert_eq!(item.parent, parent.map(ItemRef::Layer));
        assert_eq!(item.raster_bounds, if l.is_group() { None } else { Some(d.bounds()) });
        let rawrow = capture["raw"]["layers"].as_array().unwrap().iter().find(|r| r["name"] == l.name).unwrap();
        assert_eq!(l.is_group(), rawrow["is_group"].as_bool().unwrap());
        for property in rawrow["properties"].as_array().unwrap() {
            let tag = property["tag"].as_u64().unwrap();
            let offset = property["offset"].as_u64().unwrap();
            let bytes = property["bytes"].as_u64().unwrap();
            let identity = format!("{tag}@{offset}:{bytes}");
            let record =
                x.opaque.iter().find(|r| r.owner == Some(ItemRef::Layer(l.id)) && r.namespace == "gimp/xcf/source-property" && r.identity == identity).unwrap();
            let hex = property["hex"].as_str().unwrap();
            let expected: Vec<u8> =
                hex.as_bytes().as_chunks::<2>().0.iter().map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect();
            assert_eq!(record.bytes.as_slice(), expected);
            assert!(!record.rendering_required);
            assert_eq!(record.source_version, x.provenance.format_version);
        }
        if !l.is_group() {
            let span = &rawrow["plane"]["native"];
            let at = span["offset"].as_u64().unwrap() as usize;
            let n = span["bytes"].as_u64().unwrap() as usize;
            let expected = &packet[at..at + n];
            assert_eq!(digest(expected), span["sha256"].as_str().unwrap());
            assert_eq!(raw(l), expected)
        }
    }
}
