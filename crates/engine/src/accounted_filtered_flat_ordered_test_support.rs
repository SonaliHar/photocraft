//! Test-only actual storage bindings and prospectively paid CURRENT subset.
//! No engine functions, source grant, numeric or operative-order oracle.
#![allow(dead_code)]
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "accounted_filtered_flat_ordered_fixture.rs"]
pub mod data;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "../../ui-egui/src/filtered_flat_fixture.rs"]
#[allow(dead_code)]
mod native;
use photocraft_doc::{
    Document, LayerContent, SampleEncoding,
    accounted_document::AccountedDocument,
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterValue},
};
use photocraft_raster::backing::ReadControl;
use sha2::{Digest, Sha256};
use std::sync::Arc;
pub fn ctl() -> ReadControl<'static> {
    native::ctl()
}
pub fn workspace() -> std::path::PathBuf {
    native::workspace()
}
pub fn archive(o: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    native::archive(o)
}
pub fn encoded(o: &AccountedDocument) -> Vec<u8> {
    native::encoded(o)
}
pub fn same(a: &Document, b: &Document) {
    native::same(a, b)
}
pub fn storage(a: &Document, b: &Document) {
    native::storage(a, b)
}
pub fn bytes(binding: data::Binding) -> Vec<u8> {
    let b = std::fs::read(workspace().join(binding.path)).unwrap();
    assert_eq!(b.len(), binding.bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&b)), binding.sha256);
    b
}
pub fn import(case: &data::Case) -> photocraft_io::OwnedXcfImport {
    let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
        resident: 256 << 20,
        index: 64 << 20,
        scratch: 0,
    });
    let value = try_import_with_resident(case, 128 << 20, q).unwrap();
    assert!(matches!(value, photocraft_io::OwnedXcfImport::FilteredFlat(_)));
    observed(value.document().document(), case);
    value
}
pub fn try_import_with_resident(
    case: &data::Case,
    resident: u64,
    q: photocraft_raster::backing::quota::BackingQuota,
) -> Result<photocraft_io::OwnedXcfImport, photocraft_io::IoError> {
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", resident), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let mut options = photocraft_xcf::DecodeOptions::default();
    options.limits.resident_bytes = usize::try_from(resident).unwrap();
    options.limits.input_bytes = 16 << 20;
    photocraft_io::import_owned_xcf_bytes_in_quota("ordered.xcf", Arc::new(bytes(case.source)), &options, &limits, q, &ctl())
}

fn hex(s: &str) -> Vec<u8> {
    s.as_bytes().as_chunks::<2>().0.iter().map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect()
}
fn word(b: &[u8], at: &mut usize) -> u32 {
    let v = u32::from_be_bytes(b.get(*at..*at + 4).unwrap().try_into().unwrap());
    *at += 4;
    v
}
fn text<'a>(b: &'a [u8], at: &mut usize) -> &'a str {
    let n = word(b, at) as usize;
    let v = b.get(*at..*at + n).unwrap();
    *at += n;
    assert_eq!(v.last(), Some(&0));
    std::str::from_utf8(v.get(..n - 1).unwrap()).unwrap()
}
fn argument(raw: &[u8]) -> (String, FilterValue) {
    let mut at = 0;
    let name = text(raw, &mut at).to_owned();
    let kind = word(raw, &mut at);
    let value = match kind {
        3 => FilterValue::Number(ExactNumber::Float32Bits(word(raw, &mut at))),
        8 => {
            let format = text(raw, &mut at);
            assert_eq!(word(raw, &mut at), 16);
            let mut components = Vec::new();
            for _ in 0..4 {
                let b = raw.get(at..at + 4).unwrap();
                at += 4;
                components.push(ExactNumber::Float32Bits(u32::from_le_bytes(b.try_into().unwrap())));
            }
            assert_eq!(word(raw, &mut at), 0);
            let encoding = match format {
                "RGBA float" => SampleEncoding::Linear,
                "R'G'B'A float" => SampleEncoding::NonlinearProfile,
                _ => panic!("actual serialized color format"),
            };
            FilterValue::Color { components, encoding, profile: None }
        }
        _ => panic!("literal frozen stored argument type"),
    };
    assert_eq!(at, raw.len());
    (name, value)
}
pub fn observed(doc: &Document, case: &data::Case) {
    assert_eq!(doc.layers.len(), 4);
    assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, case.precision);
    assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
    let raw: serde_json::Value = serde_json::from_slice(&bytes(case.capture)).unwrap();
    let roots = raw["raw"]["roots"].as_array().unwrap();
    let state = doc.xcf_state.as_ref().unwrap();
    assert_eq!(state.filter_stacks.len(), 3);
    assert_eq!(state.filter_masks.len(), 9);
    let directory = workspace().join(case.capture.path).parent().unwrap().to_owned();
    for (index, (layer, row)) in doc.layers.iter().zip(roots.iter().rev()).enumerate() {
        assert_eq!(layer.name, data::ROOT_NAMES[index]);
        assert_eq!(layer.name, row["name"].as_str().unwrap());
        let blob = &row["stored_source"];
        let b = std::fs::read(directory.join(blob["file"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&b)), blob["sha256"].as_str().unwrap());
        assert_eq!(native::raw(layer), b);
        let effects = row["effects"].as_array().unwrap();
        assert_eq!(effects.len(), data::OPERATIONS[index].len());
        let stacks = state.filter_stacks.iter().filter(|s| s.owner == ItemRef::Layer(layer.id)).collect::<Vec<_>>();
        if effects.is_empty() {
            assert!(stacks.is_empty());
            continue;
        }
        assert_eq!(stacks.len(), 1);
        let stack = stacks[0];
        assert_eq!(stack.nodes.len(), effects.len());
        for (node_index, (node, effect)) in stack.nodes.iter().zip(effects).enumerate() {
            assert_eq!(node.operation.name, data::OPERATIONS[index][node_index]);
            assert_eq!(node.operation.name, effect["strings"][2].as_str().unwrap());
            let properties = effect["properties"].as_array().unwrap();
            for property in properties {
                let b = hex(property["hex"].as_str().unwrap());
                let identity = format!("{}@{}:{}", property["tag"].as_u64().unwrap(), property["offset"].as_u64().unwrap(), b.len());
                let record = state
                    .opaque
                    .iter()
                    .find(|r| r.owner == Some(ItemRef::Layer(layer.id)) && r.namespace == "gimp/xcf/source-property" && r.identity == identity)
                    .unwrap();
                assert_eq!(record.bytes.as_slice(), b);
            }
            let args = properties.iter().filter(|p| p["tag"] == 45).map(|p| argument(&hex(p["hex"].as_str().unwrap()))).collect::<Vec<_>>();
            assert_eq!(args.len(), node.arguments.len());
            for ((name, value), actual) in args.iter().zip(&node.arguments) {
                assert_eq!(name, &actual.name);
                assert_eq!(value, &actual.value);
            }
            let blob = &effect["mask"]["stored_native"];
            let b = std::fs::read(directory.join(blob["file"].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&b)), blob["sha256"].as_str().unwrap());
            let mask = state.filter_masks.iter().find(|m| m.owner == node.id).unwrap();
            assert_eq!(mask.surface.try_read_interleaved(doc.bounds()).unwrap(), b);
        }
    }
}
pub fn payload(a: &Document, b: &Document) {
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    let (a_state, b_state) = (a.xcf_state.as_ref().unwrap(), b.xcf_state.as_ref().unwrap());
    assert_eq!(a_state.opaque, b_state.opaque);
    assert_eq!(a_state.filter_masks, b_state.filter_masks);
    for (a, b) in a.layers.iter().zip(&b.layers) {
        assert_eq!(a.id, b.id);
        assert_eq!(native::raw(a), native::raw(b));
        let (LayerContent::BackedRaster(a), LayerContent::BackedRaster(b)) = (&a.content, &b.content) else { panic!("backed") };
        assert_eq!(a.descriptor(), b.descriptor());
        assert_eq!(a.default_bytes(), b.default_bytes());
    }
}
pub fn single_root_two(owner: &AccountedDocument) -> AccountedDocument {
    // External CURRENT topology fixture. Preheld Index+Resident and whole original
    // source carrier are retained by native::current BEFORE copying/mutating.
    native::current(owner, |doc| {
        let root = doc.layers.last().unwrap().id;
        doc.layers.retain(|l| l.id == root);
        let state = doc.xcf_state.as_mut().unwrap();
        state.filter_stacks.retain(|s| s.owner == ItemRef::Layer(root));
        let stack = &mut state.filter_stacks[0];
        assert_eq!(stack.nodes.len(), 3);
        stack.nodes.truncate(2);
        let keep = [stack.nodes[0].mask.unwrap(), stack.nodes[1].mask.unwrap()];
        state.filter_masks.retain(|m| keep.contains(&ItemRef::Mask(m.id)));
        state.items.retain(|i| i.item == ItemRef::Layer(root) || keep.contains(&i.item));
        state.opaque.retain(|r| r.owner.is_none() || r.owner == Some(ItemRef::Layer(root)) || r.owner.is_some_and(|i| keep.contains(&i)));
        state.selection.selected.retain(|r| *r == ItemRef::Layer(root));
        assert_eq!(state.selection.selected, vec![ItemRef::Layer(root)]);
        state.selection.active = Some(ItemRef::Layer(root));
    })
}
pub fn malformed(owner: &AccountedDocument) -> AccountedDocument {
    native::current(owner, |d| {
        let n = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[1];
        n.operation.name = "unsupported:ordered-fixture".into();
        n.operation.schema_epoch = "unsupported-ordered-v1".into();
        n.composition.as_mut().unwrap().compatibility_epoch = "unsupported-ordered-v1".into();
    })
}
