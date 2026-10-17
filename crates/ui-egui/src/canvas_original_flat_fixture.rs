//! Six independent GIMP captures, used as lifecycle inputs only. Effective-ICC
//! byte HOLDs, Normal getter Auto normalization and derived signed-zero/RGB
//! differences remain unresolved. No GIMP display/numerical acceptance oracle.
//! All file/native/reference buffers here are external harness allocations.
use photocraft_doc::{
    Document, LayerContent, Rect,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
#[path = "canvas_flat_current_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod current;
pub(super) fn ctl() -> photocraft_raster::backing::ReadControl<'static> {
    current::ctl()
}
pub(super) fn archive(owner: &photocraft_doc::accounted_document::AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    current::archive(owner)
}
pub(super) fn page(value: &Value, doc: &Document, offset: usize, limit: usize) {
    current::page(value, doc, offset, limit)
}
pub(super) fn payload(a: &Document, b: &Document) {
    current::payload(a, b)
}
pub(super) fn same_quota(owner: &photocraft_doc::accounted_document::AccountedDocument) {
    current::same_quota(owner)
}
pub(super) fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
pub(super) struct Case {
    pub path: PathBuf,
    pub input: Vec<u8>,
    pub state: Value,
    pub raw: Vec<[i32; 4]>,
    pub resolved: Vec<[i32; 3]>,
}
fn checked(path: &Path, sha: &str) -> Vec<u8> {
    let b = std::fs::read(path).unwrap();
    assert_eq!(hash(&b), sha);
    b
}
pub(super) fn cases() -> Vec<Case> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_FLAT_MULTILAYER_RESEARCH").expect("explicit local research root required"));
    for (file, sha) in [
        ("six-source-bindings-v1.json", "c87d9d9e828e541c7b0edc41e90bd4bf872df72cddb273810fbe12f15c3cd59c"),
        ("observation-receipt-v2-explicit-limit.json", "10588c4cca3c27ae168acb1983b1374ab5ad3697939f453c2081a8afa4f342e0"),
        ("sixth-observation-receipt-v1.json", "14d59ce5fd969b1ef95d535304a83aa4a1ea04e1375cceaef8e47aee5cff94ce"),
    ] {
        checked(&root.join(file), sha);
    }
    let rows = [
        (
            "author-v1",
            "three-auto",
            "547fccca9b828174af95882ac7ffd5c367fdfee1425f557b59d59168b00c0497",
            "a785ca8f86878b03734b9c1a04fc4036fad62bde6375dfccfba3cd89a5f9a457",
            0,
        ),
        (
            "author-v1",
            "three-hidden",
            "bb99313c91ea121d7c8b5e436718ddfdbdde43f00610f8599747c4933acab0b9",
            "869256e1f8b7e3f97e0009013ba298eb8a468b3054604ee6d9185609c17bcbb7",
            0,
        ),
        (
            "author-v1",
            "three-zero",
            "3cd559eb1756e88a12f9c1adb54c5de34436d466a21b75d028f5279577815085",
            "95ffcf390b59a38985fc97e03a05c8ba1c5aaa1eadae73809b298739eb5f2749",
            0,
        ),
        (
            "author-v1",
            "four-auto",
            "48667ae4bbdaabbaceab109e2a2d237ee5f21e725633227476b91c0046c9382e",
            "e1e2798ef4469321dc8fe984550b0798cfc564b4a371cee2875999e5fa8ae5b7",
            1,
        ),
        (
            "author-v1",
            "four-explicit",
            "14360b4c79e1b5804ef0911c1a754079173ada764c9e534b698053231a4ce9b0",
            "b94b3c1a9d6510e52efaa2e2fb17f7398ef1141f836e461590a97440da082515",
            2,
        ),
        (
            "sixth-author-v1",
            "sixth-overlapping-explicit",
            "8ec81353a220f3f1a671aae05b90c59c704f92b779d944b8e3c47c478317a113",
            "4b765685b37084ac17988c22d9ba1657128c1cbbb0a3f90b4dae80aea97b1165",
            2,
        ),
    ];
    rows.into_iter()
        .map(|(phase, id, sha, state_sha, kind)| {
            let dir = root.join(phase).join("outputs");
            let path = dir.join(format!("{id}.xcf"));
            let input = checked(&path, sha);
            let state: Value = serde_json::from_slice(&checked(&dir.join(format!("{id}.case.json")), state_sha)).unwrap();
            assert_eq!(state["same_process"]["precision"], 650);
            assert!(state["same_process"]["profiles"]["assigned"].is_null());
            for row in state["raw_saved"]["layers"].as_array().unwrap() {
                let p = &row["native_blob"];
                let b = checked(&dir.join(p["file"].as_str().unwrap()), p["sha256"].as_str().unwrap());
                assert_eq!(b.len() as u64, p["bytes"].as_u64().unwrap());
            }
            let (raw, resolved) = match kind {
                0 => (vec![[31, -4, -1, -2], [30, -1, -1, -2], [28, 0, -1, -1]], vec![[4, 1, 2], [1, 1, 2], [1, 1, 1]]),
                1 => (vec![[36, 0, -1, -2], [34, -1, -1, -2], [23, -4, -1, -2], [28, 0, -1, -1]], vec![[2, 1, 2], [1, 1, 2], [4, 1, 2], [1, 1, 1]]),
                _ => (vec![[44, 1, 4, 4], [31, 4, 2, 3], [30, 2, 1, 2], [28, 0, 1, 1]], vec![[1, 4, 4], [4, 2, 3], [2, 1, 2], [1, 1, 1]]),
            };
            Case { path, input, state, raw, resolved }
        })
        .collect()
}
pub(super) fn import(case: &Case) -> photocraft_io::OwnedXcfImport {
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    photocraft_io::import_owned_xcf_from_reader_in_quota(
        "Independent original.xcf",
        &mut std::io::Cursor::new(&case.input),
        case.input.len() as u64,
        &Default::default(),
        &limits,
        q,
        &ctl(),
    )
    .unwrap()
}
fn space(s: GimpSpace) -> i32 {
    match s {
        GimpSpace::LinearRgb => 1,
        GimpSpace::ProfileRgb => 2,
        GimpSpace::Lab => 3,
        GimpSpace::PerceptualRgb => 4,
    }
}
fn operator(s: GimpOperator) -> i32 {
    match s {
        GimpOperator::Union => 1,
        GimpOperator::ClipToBackdrop => 2,
        GimpOperator::ClipToLayer => 3,
        GimpOperator::Intersection => 4,
    }
}
pub(super) fn observed(case: &Case, doc: &Document) {
    let rows = case.state["same_process"]["layers"].as_array().unwrap();
    let rawrows = case.state["raw_saved"]["layers"].as_array().unwrap();
    assert_eq!(doc.layers.len(), rows.len());
    assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
    assert!(matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. }));
    assert_eq!(doc.icc_profile.as_deref().unwrap().as_slice(), photocraft_cms::Builtin::Srgb.profile().encoded_bytes().unwrap());
    // Generated CURRENT builtin bytes are not the GIMP effective-profile witness.
    assert_eq!(doc.size.width as u64, case.state["same_process"]["size"][0].as_u64().unwrap());
    assert_eq!(doc.size.height as u64, case.state["same_process"]["size"][1].as_u64().unwrap());
    for (i, (layer, row)) in doc.layers.iter().rev().zip(rows).enumerate() {
        assert_eq!(layer.name, row["name"].as_str().unwrap());
        assert_eq!(layer.visible, row["visible"].as_bool().unwrap());
        let bounds = Rect::from_xywh(
            row["offset"][0].as_i64().unwrap() as i32,
            row["offset"][1].as_i64().unwrap() as i32,
            row["size"][0].as_u64().unwrap() as u32,
            row["size"][1].as_u64().unwrap() as u32,
        );
        let item = doc.xcf_state.as_ref().unwrap().items.iter().find(|v| v.item == ItemRef::Layer(layer.id)).unwrap();
        assert_eq!(item.raster_bounds, Some(bounds));
        let p = item.composition.as_ref().unwrap();
        assert_eq!([p.source_mode, p.stored_blend_space, p.stored_composite_space, p.stored_operator], case.raw[i]);
        // Literal canonical-contract expectations, never generated using new source rule.
        let r = p.checked_resolved().unwrap();
        assert_eq!([space(r.blend_space), space(r.composite_space), operator(r.operator)], case.resolved[i]);
        let LayerContent::BackedRaster(b) = &layer.content else { panic!("unflattened original native") };
        assert_eq!(b.operative_bounds(), bounds);
        let mut native = vec![0; bounds.width() as usize * bounds.height() as usize * 16];
        for coord in bounds.tiles() {
            let pin = b.checked_view().pin_tile(coord, &ctl()).unwrap();
            for y in coord.rect().intersect(&bounds).y0..coord.rect().intersect(&bounds).y1 {
                for x in coord.rect().intersect(&bounds).x0..coord.rect().intersect(&bounds).x1 {
                    let at = ((y - bounds.y0) as usize * bounds.width() as usize + (x - bounds.x0) as usize) * 16;
                    pin.read_pixel_native_into(x, y, &mut native[at..at + 16], &ctl()).unwrap();
                }
            }
        }
        if cfg!(target_endian = "big") {
            for word in native.as_chunks_mut::<4>().0 {
                word.reverse();
            }
        }
        assert_eq!(hash(&native), rawrows[i]["native_sha256"].as_str().unwrap());
    }
}
/// Only the independently recorded public-format payload offset is changed.
/// This is a synthetic malformed negative, never an authored GIMP variant.
pub(super) fn malformed(case: &Case) -> Vec<u8> {
    let property = case.state["raw_saved"]["layers"][0]["properties"].as_array().unwrap().iter().find(|p| p["tag"] == 36).unwrap();
    let at = property["offset"].as_u64().unwrap() as usize;
    let expected = i32::from_be_bytes(case.input[at..at + 4].try_into().unwrap());
    assert_eq!(expected, case.raw[0][2]);
    let mut bytes = case.input.clone();
    bytes[at..at + 4].copy_from_slice(&99i32.to_be_bytes());
    bytes
}
pub(super) fn refuses(bytes: &[u8]) {
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    assert!(
        photocraft_io::import_owned_xcf_from_reader_in_quota(
            "Malformed original.xcf",
            &mut std::io::Cursor::new(bytes),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            q,
            &ctl()
        )
        .is_err()
    );
}
