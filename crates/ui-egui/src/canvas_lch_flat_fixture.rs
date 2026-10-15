//! LCh24..27 original packets (eight source/edit XCFs), no Lab normalization.
//! All files/native/reference buffers are external harness captures. Changed
//! CURRENT metadata/geometry/modern policy is not authored GIMP edit evidence.
use photocraft_doc::{
    Document, LayerContent,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
#[path = "canvas_flat_translate_fixture.rs"]
#[allow(dead_code)]
mod current;
pub(super) fn ctl() -> photocraft_raster::backing::ReadControl<'static> {
    current::ctl()
}
pub(super) fn native(layer: &photocraft_doc::Layer) -> Vec<u8> {
    current::native(layer)
}
pub(super) fn archive(owner: &photocraft_doc::accounted_document::AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    current::archive(owner)
}
pub(super) fn reference(original: &Document, current_doc: &Document) -> photocraft_compose::Buffer {
    current::reference(original, current_doc)
}
pub(super) fn moved(before: &Document, after: &Document, id: photocraft_doc::LayerId, dx: i32, dy: i32) {
    current::exact(before, after, &[(id, dx, dy)])
}
pub(super) struct Case {
    pub file: String,
    pub mode: i32,
    pub state: Value,
    pub png: String,
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
pub(super) fn cases() -> (PathBuf, Vec<Case>) {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("composition-matrix");
    let mut pins = serde_json::Map::new();
    for raw in [include_bytes!("../../compose/tests/gimp_lch_hashes.json").as_slice()] {
        let values: Value = serde_json::from_slice(raw).unwrap();
        for (file, sha) in values.as_object().unwrap() {
            assert_eq!(hash(&std::fs::read(root.join(file)).unwrap()), sha.as_str().unwrap());
            pins.insert(file.clone(), sha.clone());
        }
    }
    let mut cases = Vec::new();
    for (mode, name) in [(24, "lch-hue"), (25, "lch-chroma"), (26, "lch-color"), (27, "lch-lightness")] {
        for edit in [false, true] {
            let stem = format!("mode-{name}");
            let file = format!("{stem}{}.xcf", if edit { ".edit" } else { "" });
            let phase = if edit { "edit" } else { "source" };
            let state_file = format!("{stem}.fresh-{phase}_state.json");
            let png = format!("{stem}.fresh-{phase}.ref.png");
            assert!(pins.contains_key(&file));
            assert!(pins.contains_key(&state_file));
            assert!(pins.contains_key(&png));
            let state: Value = serde_json::from_slice(&std::fs::read(root.join(state_file)).unwrap()).unwrap();
            assert_eq!(state["size"], serde_json::json!([16, 12]));
            assert!(state["assigned_icc_sha256"].is_null());
            assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
            for row in state["layers"].as_array().unwrap() {
                let p = &row["native"];
                let b = std::fs::read(root.join(p["file"].as_str().unwrap())).unwrap();
                assert_eq!(hash(&b), p["sha256"].as_str().unwrap());
                assert_eq!(b.len() as u64, p["bytes"].as_u64().unwrap());
                assert_eq!(p["byte_order"], "little");
            }
            cases.push(Case { file, mode, state, png });
        }
    }
    assert_eq!(cases.len(), 8);
    (root, cases)
}
pub(super) fn input(root: &Path, case: &Case) -> Vec<u8> {
    std::fs::read(root.join(&case.file)).unwrap()
}
pub(super) fn import(bytes: &[u8]) -> photocraft_io::OwnedXcfImport {
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    photocraft_io::import_owned_xcf_from_reader_in_quota(
        "Original LCh.xcf",
        &mut std::io::Cursor::new(bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q,
        &ctl(),
    )
    .unwrap()
}
pub(super) fn observed(root: &Path, case: &Case, doc: &Document) {
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(doc.mode, photocraft_doc::ColorMode::Rgb);
    assert_eq!(doc.depth, photocraft_doc::SampleType::F32);
    assert_eq!(doc.sample_encoding, Some(photocraft_doc::SampleEncoding::NonlinearProfile));
    assert!(matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. }));
    assert_eq!(doc.xcf_state.as_ref().unwrap().items.len(), 2);
    assert_eq!(doc.size.width, 16);
    assert_eq!(doc.size.height, 12);
    assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
    assert_eq!(doc.icc_profile.as_deref().unwrap().as_slice(), photocraft_cms::Builtin::Srgb.profile().encoded_bytes().unwrap());
    for (index, (layer, row)) in doc.layers.iter().zip(case.state["layers"].as_array().unwrap().iter().rev()).enumerate() {
        assert_eq!(layer.name, row["name"].as_str().unwrap());
        assert!(layer.visible);
        assert!(layer.mask.is_none());
        let LayerContent::BackedRaster(b) = &layer.content else { panic!("native") };
        assert!(doc.xcf_state.as_ref().unwrap().filter_stacks.is_empty());
        let r = b.operative_bounds();
        assert_eq!([r.x0, r.y0], [row["offset"][0].as_i64().unwrap() as i32, row["offset"][1].as_i64().unwrap() as i32]);
        assert_eq!([r.width(), r.height()], [row["size"][0].as_u64().unwrap() as u32, row["size"][1].as_u64().unwrap() as u32]);
        let item = doc.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).unwrap();
        assert_eq!(item.raster_bounds, Some(r));
        let p = item.composition.as_ref().unwrap();
        let resolved = p.checked_resolved().unwrap();
        if index == 0 {
            assert_eq!((p.source_mode, p.stored_blend_space, p.stored_composite_space, p.stored_operator), (28, 0, -1, -1));
            assert!(!p.legacy);
            assert_eq!(p.compatibility_epoch, "xcf-12-normal");
            assert_eq!((resolved.blend_space, resolved.composite_space, resolved.operator), (GimpSpace::LinearRgb, GimpSpace::LinearRgb, GimpOperator::Union));
        } else {
            assert_eq!((p.source_mode, p.stored_blend_space, p.stored_composite_space, p.stored_operator), (case.mode, -3, -1, -2));
            assert!(!p.legacy);
            assert!(layer.import_compositing.is_none(), "physical storage cannot represent Lab transfer");
            assert_eq!(p.compatibility_epoch, "xcf-12-authored-blend-auto");
            assert_eq!(
                (resolved.blend_space, resolved.composite_space, resolved.operator),
                (GimpSpace::Lab, GimpSpace::LinearRgb, GimpOperator::ClipToBackdrop)
            );
        }
        let mut expected = std::fs::read(root.join(row["native"]["file"].as_str().unwrap())).unwrap();
        if cfg!(target_endian = "big") {
            for word in expected.chunks_exact_mut(4) {
                word.reverse();
            }
        }
        assert_eq!(native(layer), expected);
    }
}
pub(super) fn raw(before: &Document, after: &Document) {
    assert_eq!(before.layers.len(), after.layers.len());
    for (old, now) in before.layers.iter().zip(&after.layers) {
        assert_eq!(old.id, now.id);
        let LayerContent::BackedRaster(a) = &old.content else { panic!("native") };
        let LayerContent::BackedRaster(b) = &now.content else { panic!("native") };
        assert_eq!(a.operative_bounds(), b.operative_bounds());
        assert_eq!(a.format(), b.format());
        assert_eq!(a.default_bytes(), b.default_bytes());
        assert_eq!(native(old), native(now));
    }
    assert_eq!(before.icc_profile, after.icc_profile);
    assert_eq!(before.source_color_context, after.source_color_context);
    assert_eq!(before.working_profile_interpretation, after.working_profile_interpretation);
    assert_eq!(before.sample_encoding, after.sample_encoding);
    assert_eq!(before.xcf_state.as_ref().unwrap().opaque, after.xcf_state.as_ref().unwrap().opaque);
}
pub(super) fn transition(doc: &Document, revision: u64) -> Value {
    serde_json::json!({"layerId":doc.layers[1].id.0.to_string(),"documentId":doc.id.0.to_string(),"expectedRevision":revision.to_string(),"sourceMode":31,"legacy":false,"blendSpace":"linear-rgb","compositeSpace":"linear-rgb","operator":"union"})
}
pub(super) fn transitioned(doc: &Document) {
    let p = doc.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(doc.layers[1].id)).unwrap().composition.as_ref().unwrap();
    assert_eq!((p.source_mode, p.stored_blend_space, p.stored_composite_space, p.stored_operator), (31, 1, 1, 1));
    assert!(!p.legacy);
}

pub(super) fn same_quota(owner: &photocraft_doc::accounted_document::AccountedDocument) {
    owner.validate_readonly_flat_composite_storage(&ctl()).unwrap();
    for layer in &owner.document().layers {
        let LayerContent::BackedRaster(b) = &layer.content else { panic!("native") };
        assert!(owner.quota().shares_ledger(b.quota()));
    }
}
