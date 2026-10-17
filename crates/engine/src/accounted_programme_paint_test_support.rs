//! Test-only delegating genuine Source / explicitly paid synthetic CURRENT cases.
//! No source qualification, operation epoch or producer evidence is manufactured.
#![allow(dead_code, unused_imports)]
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "accounted_normal_hierarchy_test_support.rs"]
mod hierarchy;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "accounted_masked_filtered_flat_test_support.rs"]
mod masked;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "accounted_masked_normal_hierarchy_test_support.rs"]
mod masked_tree;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "accounted_filtered_flat_ordered_test_support.rs"]
mod mixed;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "accounted_masked_scalar_test_support.rs"]
mod point;
// Each consumer parent supplies Session (engine crate or photocraft_engine).
use super::Session;
pub use mixed::{ctl, workspace};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, accounted_document::AccountedDocument, item_identity::ItemRef};
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits},
    },
};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    ThresholdAlpha,
    HistogramThreshold,
    Point,
    AssignedPoint,
    GaussianGroup,
    FlatComposite,
    FilteredFlat,
    MaskedFilteredFlat,
    Hierarchy,
    MaskedHierarchyCurrent,
}
pub const CASES: [Case; 10] = [
    Case::ThresholdAlpha,
    Case::HistogramThreshold,
    Case::Point,
    Case::AssignedPoint,
    Case::GaussianGroup,
    Case::FlatComposite,
    Case::FilteredFlat,
    Case::MaskedFilteredFlat,
    Case::Hierarchy,
    Case::MaskedHierarchyCurrent,
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence {
    GenuineSource,
    PaidSyntheticCurrent,
}
impl Case {
    pub fn evidence(self) -> Evidence {
        if self == Self::MaskedHierarchyCurrent { Evidence::PaidSyntheticCurrent } else { Evidence::GenuineSource }
    }
}
pub fn cases() -> &'static [Case] {
    &CASES
}
pub fn owner(input: &Input) -> &AccountedDocument {
    input.owner()
}
pub enum Input {
    Source(photocraft_io::OwnedXcfImport),
    Current(AccountedDocument),
}
impl Input {
    pub fn owner(&self) -> &AccountedDocument {
        match self {
            Self::Source(s) => s.document(),
            Self::Current(s) => s,
        }
    }
}
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn q() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 512 << 20, index: 64 << 20, scratch: 0 })
}
fn corpus() -> PathBuf {
    std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap().into()
}
fn read_source(path: &str, sha: &str, len: usize) -> photocraft_io::OwnedXcfImport {
    let b = std::fs::read(corpus().join(path)).unwrap();
    assert_eq!(b.len(), len);
    assert_eq!(hash(&b), sha);
    photocraft_io::import_owned_xcf_bytes_in_quota(
        path,
        Arc::new(b),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
            .unwrap(),
        q(),
        &ctl(),
    )
    .unwrap()
}
fn threshold(alpha: bool) -> photocraft_io::OwnedXcfImport {
    let root = corpus().join(if alpha { "alpha-threshold-precision-evidence" } else { "threshold-precision-evidence" });
    let b = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(
        hash(&b),
        if alpha {
            "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263"
        } else {
            "23015e327f45c490ed6dbeaa6f4a9541b13dbe8526e9abfff26da13d38c56142"
        }
    );
    let pins: serde_json::Value = serde_json::from_slice(&b).unwrap();
    let proof = std::fs::read(root.join("fresh-verification.json")).unwrap();
    assert_eq!(hash(&proof), pins["fresh-verification.json"].as_str().unwrap());
    let proof: serde_json::Value = serde_json::from_slice(&proof).unwrap();
    for row in proof["checks"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        assert!(id.len() < 4096 && !id.contains(['/', '\\']));
        if !alpha && id != "rgb-float_non_linear.channel0" {
            continue;
        }
        let name = format!("{id}.xcf");
        let b = std::fs::read(root.join(&name)).unwrap();
        assert!(b.len() <= 16 << 20);
        assert_eq!(hash(&b), pins[&name].as_str().unwrap());
        if alpha {
            let raw = photocraft_xcf::decode(&b, &Default::default()).unwrap();
            if raw.model != photocraft_xcf::Model::Rgb
                || raw.sample != photocraft_xcf::Sample::F64
                || raw.precision != 750
                || !raw.layers.first().and_then(|l| l.effects.first()).is_some_and(|e| e.operation == "gimp:threshold-alpha" && e.state.enabled == Some(true))
            {
                continue;
            }
        }
        return photocraft_io::import_owned_xcf_bytes_in_quota(
            &name,
            Arc::new(b),
            &Default::default(),
            &photocraft_xcf::ResourceLimits::for_policy(Default::default())
                .unwrap()
                .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
                .unwrap(),
            q(),
            &ctl(),
        )
        .unwrap();
    }
    panic!("qualified pinned threshold fixture")
}
pub fn load(case: Case) -> Input {
    let s = match case {
        Case::ThresholdAlpha => threshold(true),
        Case::HistogramThreshold => threshold(false),
        Case::Point => point::import(&point::cases()[0]),
        Case::AssignedPoint => {
            read_source("box-raster-color-evidence/assigned-unit-f32.source.xcf", "e4f31ba998692bb7f57ebc82433a4c8d90a7aeaeb2d67a9276079bb594d4cec8", 4817)
        }
        Case::GaussianGroup => {
            read_source("composition-matrix/group-28-group-mask.xcf", "740064c2b008a9785801b9de4425f685dc2618a7fcc938d37728723dd10ccc49", 10313)
        }
        Case::FlatComposite => read_source("composition-matrix/mode-screen.xcf", "5d938125138a0006dac2aa1045893973602891d9795e3a52df91acc75722fdeb", 5542),
        Case::FilteredFlat => mixed::import(&mixed::data::CASES[0]),
        Case::MaskedFilteredFlat => masked::import(&masked::cases()[0]),
        Case::Hierarchy => {
            let c = hierarchy::cases().into_iter().find(|c| c["source_version"].as_u64() == Some(22)).unwrap();
            hierarchy::import(&c)
        }
        Case::MaskedHierarchyCurrent => return Input::Current(masked_tree::build(&masked::cases()[0])),
    };
    let owner = s.document();
    match case {
        Case::ThresholdAlpha => assert!(owner.is_readonly_threshold_alpha_storage()),
        Case::HistogramThreshold => assert!(owner.is_readonly_threshold_storage()),
        Case::Point => assert!(owner.is_readonly_point_storage()),
        Case::AssignedPoint => assert!(owner.is_readonly_assigned_point_storage()),
        Case::GaussianGroup => assert!(owner.is_readonly_gaussian_group_storage()),
        Case::FlatComposite => assert!(owner.is_readonly_flat_composite_storage()),
        Case::FilteredFlat => assert!(owner.is_readonly_filtered_flat_point_storage()),
        Case::MaskedFilteredFlat => assert!(owner.is_readonly_masked_filtered_flat_point_storage()),
        Case::Hierarchy => assert!(owner.is_readonly_normal_hierarchy_storage()),
        _ => panic!("case"),
    };
    Input::Source(s)
}
pub fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let b = photocraft_io::accounted_current_native::save_accounted_current_archive(
        owner,
        std::io::Cursor::new(Vec::new()),
        Default::default(),
        owner.quota(),
        &ctl(),
    )
    .unwrap()
    .into_inner();
    photocraft_io::accounted_native::open_accounted_native_from_reader(
        &mut std::io::Cursor::new(&b),
        b.len() as u64,
        Default::default(),
        owner.quota().clone(),
        &ctl(),
    )
    .unwrap()
}
pub fn reopen(s: &mut Session, case: Case, a: photocraft_format::owned_document::OwnedDocumentArchive) -> usize {
    match case {
        Case::ThresholdAlpha => s.add_owned_native_threshold_alpha_archive(a, "programme", None, &ctl()),
        Case::HistogramThreshold => s.add_owned_native_threshold_archive(a, "programme", None, &ctl()),
        Case::Point | Case::AssignedPoint => s.add_owned_native_point_archive(a, "programme", None, &ctl()),
        Case::GaussianGroup => s.add_owned_native_isolated_gaussian_group_archive(a, "programme", None, &ctl()),
        Case::FlatComposite => s.add_owned_native_separable_blend_archive(a, "programme", None, &ctl()),
        Case::FilteredFlat => s.add_owned_native_filtered_flat_archive(a, "programme", None, &ctl()),
        Case::MaskedFilteredFlat => s.add_owned_native_masked_filtered_flat_archive(a, "programme", None, &ctl()),
        Case::Hierarchy | Case::MaskedHierarchyCurrent => s.add_owned_native_normal_hierarchy_archive(a, "programme", None, &ctl()),
    }
    .unwrap()
}
pub fn adopt(s: &mut Session, case: Case, input: &Input) -> usize {
    let Input::Source(i) = input else {
        return reopen(s, case, archive(input.owner()));
    };
    match i {
        photocraft_io::OwnedXcfImport::ThresholdAlpha(v) => s.add_owned_threshold_alpha_xcf_import(v, "programme", Some("protected-original.xcf"), &ctl()),
        photocraft_io::OwnedXcfImport::HistogramThreshold(v) => {
            s.add_owned_histogram_threshold_xcf_import(v, "programme", Some("protected-original.xcf"), &ctl())
        }
        photocraft_io::OwnedXcfImport::Point(v) => s.add_owned_point_xcf_import(v, "programme", Some("protected-original.xcf"), &ctl()),
        photocraft_io::OwnedXcfImport::IsolatedGaussianGroup(v) => {
            s.add_owned_isolated_gaussian_group_xcf_import(v, "programme", Some("protected-original.xcf"), &ctl())
        }
        photocraft_io::OwnedXcfImport::SeparableBlend(v) => s.add_owned_separable_blend_xcf_import(v, "programme", Some("protected-original.xcf"), &ctl()),
        photocraft_io::OwnedXcfImport::FilteredFlat(v) => s.add_owned_filtered_flat_xcf_import(v, "programme", Some("protected-original.xcf"), &ctl()),
        photocraft_io::OwnedXcfImport::MaskedFilteredFlat(v) => {
            s.add_owned_masked_filtered_flat_xcf_import(v, "programme", Some("protected-original.xcf"), &ctl())
        }
        photocraft_io::OwnedXcfImport::NormalHierarchy(v) => s.add_owned_normal_hierarchy_xcf_import(v, "programme", Some("protected-original.xcf"), &ctl()),
        _ => panic!("not selected programme Source"),
    }
    .unwrap()
}
fn walk<'a>(layers: &'a [Layer], f: &mut impl FnMut(&'a Layer), depth: usize) {
    assert!(depth <= 32);
    for l in layers {
        f(l);
        if let LayerContent::Group(g) = &l.content {
            walk(&g.children, f, depth + 1)
        }
    }
}
pub fn target(doc: &Document) -> LayerId {
    let mut result = None;
    walk(
        &doc.layers,
        &mut |l| {
            if matches!(l.content, LayerContent::BackedRaster(_)) && result.is_none() {
                result = Some(l.id)
            }
        },
        0,
    );
    let fallback = result.unwrap();
    let Some(s) = &doc.xcf_state else { return fallback };
    let mut filtered = None;
    walk(
        &doc.layers,
        &mut |l| {
            if matches!(l.content, LayerContent::BackedRaster(_)) && s.filter_stacks.iter().any(|s| s.owner == ItemRef::Layer(l.id) && !s.nodes.is_empty()) {
                filtered.get_or_insert(l.id);
            }
        },
        0,
    );
    filtered.unwrap_or(fallback)
}
pub fn layer(doc: &Document, id: LayerId) -> &Layer {
    let mut found = None;
    walk(
        &doc.layers,
        &mut |l| {
            if l.id == id {
                found = Some(l)
            }
        },
        0,
    );
    found.unwrap()
}
pub fn raw(doc: &Document, id: LayerId) -> Vec<u8> {
    hierarchy::raw(layer(doc, id))
}
pub fn patch(doc: &Document, id: LayerId) -> serde_json::Value {
    let LayerContent::BackedRaster(b) = &layer(doc, id).content else { panic!("raster target") };
    let r = b.operative_bounds();
    serde_json::json!({"layer":id.0,"points":[[f64::from(r.x0)+2.0,f64::from(r.y0)+2.0,0.7],[f64::from(r.x0)+3.0,f64::from(r.y0)+3.0,1.0]],"size":2.0,"hardness":0.6,"spacing":0.3,"color":[0.4,0.3,0.2,0.5],"seed":53})
}
/// Borrowed programme observation: retaining the real owner shares immutable
/// model/native/mask providers, with no deep snapshot or fake incoming receipts.
fn compare_surface(a: &photocraft_raster::Surface, b: &photocraft_raster::Surface, c: &ReadControl<'_>) {
    c.check(64).unwrap();
    assert_eq!(a.format(), b.format());
    assert_eq!(a.default_bytes(), b.default_bytes());
    assert_eq!(a.tile_count(), b.tile_count());
    // HashMap coordinate lookup is unavailable as a borrowed public API; these
    // tiny fixture masks use one complete iterator and pay each repeated join.
    for (coord, tile) in a.tiles() {
        c.check((b.tile_count() as u64).checked_mul(16).unwrap().checked_add((tile.bytes().len() as u64).checked_mul(2).unwrap()).unwrap()).unwrap();
        let other = b.tiles().find(|(k, _)| *k == coord).unwrap().1;
        assert_eq!(tile.bytes(), other.bytes());
    }
}
pub struct Witness {
    owner: AccountedDocument,
    _fee: QuotaLease,
}
impl Witness {
    pub fn capture(owner: &AccountedDocument, c: &ReadControl<'_>) -> Self {
        c.check(1).unwrap();
        let fee = owner.quota().reserve(QuotaKind::Index, 65536).unwrap();
        Self { owner: owner.clone(), _fee: fee }
    }
    pub fn before(&self) -> &Document {
        self.owner.document()
    }
    pub fn preserved(&self, after: &Document, target: LayerId, c: &ReadControl<'_>) {
        // Independent fixture comparison prepay, never engine/preparation credit.
        // No deep snapshot: immutable model and pixel providers remain borrowed.
        c.check(1 << 20).unwrap();
        let a = self.before();
        let measured = photocraft_io::retained_native_model_census(a, Default::default(), c.interrupt).unwrap();
        c.check((measured.retained_bytes() as u64).checked_mul(16).unwrap().checked_add(65536).unwrap()).unwrap();
        assert_eq!(a.icc_profile, after.icc_profile);
        assert_eq!(a.source_color_context, after.source_color_context);
        assert_eq!(a.depth, after.depth);
        assert_eq!(a.mode, after.mode);
        assert_eq!(a.sample_encoding, after.sample_encoding);
        assert_eq!(a.working_profile_interpretation, after.working_profile_interpretation);
        if let Some(state) = &a.xcf_state {
            for mask in &state.filter_masks {
                c.check((state.filter_masks.len() as u64).checked_mul(16).unwrap()).unwrap();
                let other = after.xcf_state.as_ref().unwrap().filter_masks.iter().find(|m| m.id == mask.id).unwrap();
                compare_surface(&mask.surface, &other.surface, c);
            }
        }
        assert_eq!(a.xcf_state, after.xcf_state);
        fn compare(a: &[Layer], b: &[Layer], target: LayerId, c: &ReadControl<'_>, depth: usize) {
            assert!(depth <= 32);
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                c.check(128).unwrap();
                assert_eq!(a.id, b.id);
                assert_eq!(a.name, b.name);
                assert_eq!(a.opacity.to_bits(), b.opacity.to_bits());
                assert_eq!(a.visible, b.visible);
                assert_eq!(a.blend, b.blend);
                assert_eq!(a.locks, b.locks);
                assert_eq!(a.import_compositing, b.import_compositing);
                match (&a.mask, &b.mask) {
                    (Some(x), Some(y)) => compare_surface(&x.surface, &y.surface, c),
                    (None, None) => {}
                    _ => panic!("layer mask retained"),
                };
                assert_eq!(a.mask, b.mask);
                match (&a.content, &b.content) {
                    (LayerContent::Group(x), LayerContent::Group(y)) => {
                        assert_eq!(x.expanded, y.expanded);
                        compare(&x.children, &y.children, target, c, depth + 1)
                    }
                    (LayerContent::BackedRaster(x), LayerContent::BackedRaster(y)) => {
                        assert_eq!(x.descriptor(), y.descriptor());
                        assert_eq!(x.default_bytes(), y.default_bytes());
                        if a.id != target {
                            for coord in x.operative_bounds().tiles() {
                                let px = x.checked_view().pin_tile(coord, c).unwrap();
                                let py = y.checked_view().pin_tile(coord, c).unwrap();
                                match (px.encoded(), py.encoded()) {
                                    (
                                        photocraft_raster::checked_view::NativeTileBytes::FullTile(a),
                                        photocraft_raster::checked_view::NativeTileBytes::FullTile(b),
                                    ) => {
                                        c.check(a.len() as u64).unwrap();
                                        assert_eq!(a, b)
                                    }
                                    (
                                        photocraft_raster::checked_view::NativeTileBytes::DefaultPixel(a),
                                        photocraft_raster::checked_view::NativeTileBytes::DefaultPixel(b),
                                    ) => {
                                        c.check(a.len() as u64).unwrap();
                                        assert_eq!(a, b)
                                    }
                                    _ => panic!("retained sibling representation"),
                                }
                            }
                        }
                    }
                    _ => panic!("retained tree"),
                }
            }
        }
        compare(&a.layers, &after.layers, target, c, 0);
        c.check(1).unwrap();
    }
}
