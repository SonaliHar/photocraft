//! UNRUN RGB/Gray alpha/color continuation on pinned original/edit source packets.
//! Encoded/native/reference buffers are external harness data. CURRENT display
//! comparisons below establish persistence consistency, never a new GIMP oracle.
use crate::{EngineError, Session, accounted_commands::PreparedAccountedCommand, accounted_view::AccountedView};
use photocraft_doc::{
    LayerContent, LayerId,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, NativePlaneDescriptor, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits},
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
struct Case {
    mode: i32,
    model: photocraft_doc::ColorMode,
    record: Option<&'static str>,
    operator: GimpOperator,
    legacy: bool,
    raw: [i32; 3],
    blend: GimpSpace,
    composite: GimpSpace,
    file: &'static str,
    sha: &'static str,
    len: usize,
    state: &'static str,
    state_sha: &'static str,
}
const CASES: &[Case] = &[
    Case {
        mode: 57,
        model: photocraft_doc::ColorMode::Rgb,
        record: None,
        legacy: false,
        raw: [-1, -1, -2],
        blend: GimpSpace::LinearRgb,
        composite: GimpSpace::LinearRgb,
        operator: GimpOperator::ClipToBackdrop,
        file: "composition-matrix/mode-color-erase.xcf",
        sha: "12e80e12cb37b06cc624e0a0a3971b74d905a8686792ecf0661293c94fa9e949",
        len: 5425,
        state: "composition-matrix/mode-color-erase.fresh-source_state.json",
        state_sha: "a0019ab536d149fedf1d1992143d1e76edb5ffed108a5c762760dccb0d16d53a",
    },
    Case {
        mode: 57,
        model: photocraft_doc::ColorMode::Rgb,
        record: None,
        legacy: false,
        raw: [-1, -1, -2],
        blend: GimpSpace::LinearRgb,
        composite: GimpSpace::LinearRgb,
        operator: GimpOperator::ClipToBackdrop,
        file: "composition-matrix/mode-color-erase.edit.xcf",
        sha: "21414630eb9d9cf6f785e5849db724c723d64b0e367eb077b08bcc34f3b3fc41",
        len: 5425,
        state: "composition-matrix/mode-color-erase.fresh-edit_state.json",
        state_sha: "657f6a022f9007afd4362415d646deaaf24375684840ad7e571377d78fac07d7",
    },
    Case {
        mode: 58,
        model: photocraft_doc::ColorMode::Rgb,
        record: None,
        legacy: false,
        raw: [0, -1, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        operator: GimpOperator::ClipToBackdrop,
        file: "composition-matrix/mode-erase.xcf",
        sha: "34b7a66065b6c95d721e07c6702e7cc683a06e420ea0cc3a0a6015a99a9043a9",
        len: 5425,
        state: "composition-matrix/mode-erase.fresh-source_state.json",
        state_sha: "ba6143c6ff8993d7487ebd0d914562419b261c49cc065d5ba97428ad8e6f7cd2",
    },
    Case {
        mode: 58,
        model: photocraft_doc::ColorMode::Rgb,
        record: None,
        legacy: false,
        raw: [0, -1, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        operator: GimpOperator::ClipToBackdrop,
        file: "composition-matrix/mode-erase.edit.xcf",
        sha: "8fc7771f5165aa24406a4d6b598b469608d1cbd3fa43d96dd9fe4907d94d4953",
        len: 5425,
        state: "composition-matrix/mode-erase.fresh-edit_state.json",
        state_sha: "3c4b0aea31c103aa1bda1d5bd68cdb3ca2db6c777c4f09d57bc386a84b065b77",
    },
    Case {
        mode: 59,
        model: photocraft_doc::ColorMode::Rgb,
        record: None,
        legacy: false,
        raw: [0, -1, -1],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        operator: GimpOperator::Union,
        file: "composition-matrix/mode-merge.xcf",
        sha: "0f47d8bd893192f72373c18277e2f7963a0a8cf96c035eae7237ad191d6a92c6",
        len: 5425,
        state: "composition-matrix/mode-merge.fresh-source_state.json",
        state_sha: "208b151436fa440b018ec54baf10976029d760d2f010f17afb2da7d60e37cfaa",
    },
    Case {
        mode: 59,
        model: photocraft_doc::ColorMode::Rgb,
        record: None,
        legacy: false,
        raw: [0, -1, -1],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        operator: GimpOperator::Union,
        file: "composition-matrix/mode-merge.edit.xcf",
        sha: "39ef3113c93105d28b1831afe76d57eeaa1afc0203481fdedae5efd55880ac8a",
        len: 5425,
        state: "composition-matrix/mode-merge.fresh-edit_state.json",
        state_sha: "01413633431f5d2c659abf254d0e68cc8d21001a8cad9519aa751653b0ba41d5",
    },
    Case {
        mode: 60,
        model: photocraft_doc::ColorMode::Rgb,
        record: None,
        legacy: false,
        raw: [0, 0, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::ProfileRgb,
        operator: GimpOperator::ClipToBackdrop,
        file: "composition-matrix/mode-split.xcf",
        sha: "b7a06e3b0b55af7c070726a5b4e1773d4878012f098fcaeb24ef445621e2cc98",
        len: 5425,
        state: "composition-matrix/mode-split.fresh-source_state.json",
        state_sha: "4c2cd099d0a5e0127fdd506cedca034b1c3c876fcf2b2abf0c48b6b957619a42",
    },
    Case {
        mode: 60,
        model: photocraft_doc::ColorMode::Rgb,
        record: None,
        legacy: false,
        raw: [0, 0, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::ProfileRgb,
        operator: GimpOperator::ClipToBackdrop,
        file: "composition-matrix/mode-split.edit.xcf",
        sha: "dd49fc7598f045fcaef77a73ea37e7d6693312c366fefa0df5e4869e7c400bc0",
        len: 5425,
        state: "composition-matrix/mode-split.fresh-edit_state.json",
        state_sha: "f6e0b213bb33830233aecea524d3d24133f5a18de4e5c07fc096c13382247388",
    },
    Case {
        mode: 57,
        model: photocraft_doc::ColorMode::Grayscale,
        record: Some("mode-57-source"),
        legacy: false,
        raw: [-1, -1, -2],
        blend: GimpSpace::LinearRgb,
        composite: GimpSpace::LinearRgb,
        operator: GimpOperator::ClipToBackdrop,
        file: "alpha-gray-evidence/mode-57.xcf",
        sha: "e5d2daa85de2ded2009b3930798be7854fb88c16b3a7489d18aab1c18e289c87",
        len: 3153,
        state: "alpha-gray-evidence/observations.json",
        state_sha: "697cfca7d1390b46b3ef319bc76f716c5ff90eb2010d8ef70ac107dce823ac10",
    },
    Case {
        mode: 57,
        model: photocraft_doc::ColorMode::Grayscale,
        record: Some("mode-57-edit"),
        legacy: false,
        raw: [-1, -1, -2],
        blend: GimpSpace::LinearRgb,
        composite: GimpSpace::LinearRgb,
        operator: GimpOperator::ClipToBackdrop,
        file: "alpha-gray-evidence/mode-57.edit.xcf",
        sha: "7749efea7978a9b1746cd6e13fa790fbd1311345e3e9163503f8300720dbc872",
        len: 3153,
        state: "alpha-gray-evidence/observations.json",
        state_sha: "697cfca7d1390b46b3ef319bc76f716c5ff90eb2010d8ef70ac107dce823ac10",
    },
    Case {
        mode: 58,
        model: photocraft_doc::ColorMode::Grayscale,
        record: Some("mode-58-source"),
        legacy: false,
        raw: [0, -1, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        operator: GimpOperator::ClipToBackdrop,
        file: "alpha-gray-evidence/mode-58.xcf",
        sha: "4db2455e4a677850d22851b70139f98ee447b7faaeebaf3aa08fdcffdc9f8284",
        len: 3153,
        state: "alpha-gray-evidence/observations.json",
        state_sha: "697cfca7d1390b46b3ef319bc76f716c5ff90eb2010d8ef70ac107dce823ac10",
    },
    Case {
        mode: 58,
        model: photocraft_doc::ColorMode::Grayscale,
        record: Some("mode-58-edit"),
        legacy: false,
        raw: [0, -1, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        operator: GimpOperator::ClipToBackdrop,
        file: "alpha-gray-evidence/mode-58.edit.xcf",
        sha: "69917d9f09ac58127090e8cd4a54438540204c5cf4bb034d1189759f36741ab8",
        len: 3153,
        state: "alpha-gray-evidence/observations.json",
        state_sha: "697cfca7d1390b46b3ef319bc76f716c5ff90eb2010d8ef70ac107dce823ac10",
    },
    Case {
        mode: 59,
        model: photocraft_doc::ColorMode::Grayscale,
        record: Some("mode-59-source"),
        legacy: false,
        raw: [0, -1, -1],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        operator: GimpOperator::Union,
        file: "alpha-gray-evidence/mode-59.xcf",
        sha: "91aab8b6e5e208fb1bd74f4cf118e88a569250d35e1d88a1042f6111c68ee411",
        len: 3153,
        state: "alpha-gray-evidence/observations.json",
        state_sha: "697cfca7d1390b46b3ef319bc76f716c5ff90eb2010d8ef70ac107dce823ac10",
    },
    Case {
        mode: 59,
        model: photocraft_doc::ColorMode::Grayscale,
        record: Some("mode-59-edit"),
        legacy: false,
        raw: [0, -1, -1],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        operator: GimpOperator::Union,
        file: "alpha-gray-evidence/mode-59.edit.xcf",
        sha: "58c51251598a839ffe178cf12e7a60cc529b1910f6a012f9ea9a38e308f43adc",
        len: 3153,
        state: "alpha-gray-evidence/observations.json",
        state_sha: "697cfca7d1390b46b3ef319bc76f716c5ff90eb2010d8ef70ac107dce823ac10",
    },
    Case {
        mode: 60,
        model: photocraft_doc::ColorMode::Grayscale,
        record: Some("mode-60-source"),
        legacy: false,
        raw: [0, 0, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::ProfileRgb,
        operator: GimpOperator::ClipToBackdrop,
        file: "alpha-gray-evidence/mode-60.xcf",
        sha: "e07c92d5c54a8a5d6f2864480cdd1d09abce3002f44ef7eb7ab03bb76846c687",
        len: 3153,
        state: "alpha-gray-evidence/observations.json",
        state_sha: "697cfca7d1390b46b3ef319bc76f716c5ff90eb2010d8ef70ac107dce823ac10",
    },
    Case {
        mode: 60,
        model: photocraft_doc::ColorMode::Grayscale,
        record: Some("mode-60-edit"),
        legacy: false,
        raw: [0, 0, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::ProfileRgb,
        operator: GimpOperator::ClipToBackdrop,
        file: "alpha-gray-evidence/mode-60.edit.xcf",
        sha: "53754c96455d25cd8e9dc17f28b602bb2ba2c5fc568f89869a8f031d1faa516c",
        len: 3153,
        state: "alpha-gray-evidence/observations.json",
        state_sha: "697cfca7d1390b46b3ef319bc76f716c5ff90eb2010d8ef70ac107dce823ac10",
    },
];
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn digest(raw: &[u8]) -> String {
    Sha256::digest(raw).iter().map(|b| format!("{b:02x}")).collect()
}
fn root() -> PathBuf {
    PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("configured reviewed corpus"))
}
fn checked(file: &str, pin: &str) -> Vec<u8> {
    let raw = std::fs::read(root().join(file)).unwrap();
    assert_eq!(digest(&raw), pin);
    raw
}
fn session(case: &Case) -> (Session, BackingQuota) {
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let raw = checked(case.file, case.sha);
    assert_eq!(raw.len(), case.len);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let source = photocraft_io::import_owned_xcf_from_reader_in_quota(
        case.file,
        &mut Cursor::new(&raw),
        raw.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::SeparableBlend(source) = source else { panic!("one strict selected route, no Plain retry") };
    let mut s = Session::new();
    s.add_owned_separable_blend_xcf_import(&source, "Alpha original", Some(case.file), &ctl()).unwrap();
    assert!(s.active().unwrap().doc.accounted().unwrap().is_readonly_flat_composite_storage());
    (s, q)
}
// Full target-tile padding is intentionally NOT a Move oracle. These external
// captures preserve native row-major operative words and descriptor/defaults.
fn native(s: &Session) -> Vec<(NativePlaneDescriptor, Vec<u8>)> {
    s.active()
        .unwrap()
        .doc
        .layers
        .iter()
        .map(|l| {
            let LayerContent::BackedRaster(backed) = &l.content else { panic!("actual native leaf") };
            let r = backed.operative_bounds();
            let n = backed.format().bytes_per_pixel();
            let mut bytes = vec![0; r.width() as usize * r.height() as usize * n];
            for coordinate in r.tiles() {
                let pin = backed.checked_view().pin_tile(coordinate, &ctl()).unwrap();
                let part = coordinate.rect().intersect(&r);
                for y in part.y0..part.y1 {
                    for x in part.x0..part.x1 {
                        let i = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * n;
                        pin.read_pixel_native_into(x, y, &mut bytes[i..i + n], &ctl()).unwrap();
                    }
                }
            }
            (backed.descriptor(), bytes)
        })
        .collect()
}
fn ids(s: &Session) -> [LayerId; 2] {
    [s.active().unwrap().doc.layers[0].id, s.active().unwrap().doc.layers[1].id]
}
fn policy(s: &Session, i: usize) -> photocraft_doc::import_compositing::GimpComposition {
    let state = s.active().unwrap();
    let id = state.doc.layers[i].id;
    state.doc.xcf_state.as_ref().unwrap().items.iter().find(|r| r.item == ItemRef::Layer(id)).unwrap().composition.clone().unwrap()
}
fn original(case: &Case, s: &Session) {
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(doc.depth, photocraft_doc::SampleType::F32);
    assert_eq!(doc.mode, case.model);
    let basis = match case.model {
        photocraft_doc::ColorMode::Rgb => photocraft_cms::Builtin::Srgb.profile(),
        photocraft_doc::ColorMode::Grayscale => photocraft_cms::Builtin::SGray.profile(),
        _ => panic!("closed model"),
    };
    assert_eq!(doc.icc_profile.as_deref().map(Vec::as_slice), basis.encoded_bytes());
    match case.model {
        photocraft_doc::ColorMode::Rgb => {
            assert!(matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. }))
        }
        photocraft_doc::ColorMode::Grayscale => {
            assert!(matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSGray { .. }))
        }
        _ => panic!("closed model"),
    }
    // Eight Gray producer effectiveICC files are absent locally: HOLD. This
    // proves the CURRENT strict SGray basis, never equality to missing bytes.
    assert_eq!(doc.sample_encoding, Some(photocraft_doc::SampleEncoding::NonlinearProfile));
    assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, 650);
    assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
    assert_eq!(doc.xcf_state.as_ref().unwrap().provenance.format_version, 12);
    let top = policy(s, 1);
    assert_eq!(top.source_mode, case.mode);
    assert_eq!(top.legacy, case.legacy);
    assert_eq!(top.compatibility_epoch, "xcf-12-authored-blend-auto");
    assert_eq!([top.stored_blend_space, top.stored_composite_space, top.stored_operator], case.raw);
    let resolved = top.resolved.unwrap();
    assert_eq!(resolved.blend_space, case.blend);
    assert_eq!(resolved.composite_space, case.composite);
    assert_eq!(resolved.operator, case.operator);
    let base = policy(s, 0);
    assert_eq!(base.source_mode, 28);
    assert!(!base.legacy);
    assert_eq!(base.compatibility_epoch, "xcf-12-normal");
    assert_eq!((base.stored_blend_space, base.stored_composite_space, base.stored_operator), (0, -1, -1));
    let resolved = base.resolved.unwrap();
    assert_eq!(resolved.blend_space, GimpSpace::LinearRgb);
    assert_eq!(resolved.composite_space, GimpSpace::LinearRgb);
    assert_eq!(resolved.operator, GimpOperator::Union);
    // State is SHA-bound first. Its native files have their own exact length/hash
    // bindings; compare actual importer words against producer readback, not self.
    let capture: Value = serde_json::from_slice(&checked(case.state, case.state_sha)).unwrap();
    let state = if let Some(id) = case.record { capture["records"].as_array().unwrap().iter().find(|v| v["id"] == id).unwrap() } else { &capture };
    assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
    assert!(state.get("assigned_icc_sha256").or_else(|| state.get("assigned_icc")).unwrap().is_null());
    let captures = native(s);
    for (i, row) in state["layers"].as_array().unwrap().iter().rev().enumerate() {
        let blob = &row["native"];
        assert_eq!(blob.get("byte_order").or_else(|| row.get("byte_order")).unwrap(), "little");
        let folder = if case.record.is_some() { "alpha-gray-evidence" } else { "composition-matrix" };
        let file = format!("{folder}/{}", blob["file"].as_str().unwrap());
        let bytes = checked(&file, blob["sha256"].as_str().unwrap());
        assert_eq!(bytes.len() as u64, blob["bytes"].as_u64().unwrap());
        let native: Vec<u8> = bytes.as_chunks::<4>().0.iter().flat_map(|w| f32::from_le_bytes(*w).to_ne_bytes()).collect();
        assert_eq!(captures[i].1, native);
        assert_eq!(doc.layers[i].name, row["name"].as_str().unwrap());
        let bounds = captures[i].0.bounds();
        assert_eq!([bounds.x0, bounds.y0], [row["offset"][0].as_i64().unwrap() as i32, row["offset"][1].as_i64().unwrap() as i32]);
    }
}
fn display(s: &Session) -> Vec<[u64; 4]> {
    let owner = s.active().unwrap().doc.accounted().unwrap();
    let prepared = photocraft_compose::backed_separable_document::PreparedBackedSeparableDocument::new(owner, Default::default(), &ctl()).unwrap();
    let band = prepared.render_band(owner.document().bounds(), &ctl()).unwrap();
    band.pixels().iter().map(|p| p.map(f64::to_bits)).collect()
}
fn save_reopen(s: &mut Session, title: &str, q: &BackingQuota) -> Session {
    let bytes = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(s, title, &ctl())
        .unwrap()
        .write_and_publish(s, &ctl(), |doc, _| {
            photocraft_format::owned_document::save_current_document_archive(doc, Cursor::new(Vec::new()), Default::default(), q, &ctl())
                .map(|w| w.into_inner())
                .map_err(|e| EngineError::Other(e.to_string()))
        })
        .unwrap();
    let archive =
        photocraft_io::accounted_native::open_accounted_native_from_reader(&mut Cursor::new(&bytes), bytes.len() as u64, Default::default(), q.clone(), &ctl())
            .unwrap();
    let mut reopened = Session::new();
    reopened.add_owned_native_separable_blend_archive(archive, "continued", Some("actual/Alpha.pcraft"), &ctl()).unwrap();
    reopened
}
// A genuine preheld metadata copy of an admitted Gray owner. No source
// eligibility is inferred: full typed CURRENT recipe is validated before this
// test-only publication, retaining the actual original codec/native receipts.
fn gray_explicit_normal_control(case: &Case) -> (Session, BackingQuota) {
    let (s, q) = session(case);
    let original = s.active().unwrap().doc.accounted().unwrap().clone();
    let index = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let payload = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    let mut doc = original.document().clone();
    let base = doc.layers[0].id;
    let physical = doc.layers[0].import_compositing.as_mut().unwrap();
    physical.blend_encoding = photocraft_doc::SampleEncoding::NonlinearProfile;
    physical.composite_encoding = photocraft_doc::SampleEncoding::NonlinearProfile;
    let policy = doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(base)).unwrap().composition.as_mut().unwrap();
    policy.stored_blend_space = 2;
    policy.stored_composite_space = 2;
    policy.stored_operator = 2;
    policy.resolved = Some(photocraft_doc::import_compositing::ResolvedGimpComposition {
        blend_space: GimpSpace::ProfileRgb,
        composite_space: GimpSpace::ProfileRgb,
        operator: GimpOperator::ClipToBackdrop,
    });
    let census = photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert!(census.retained_bytes() as u64 <= index.bytes());
    assert!(doc.icc_profile.as_ref().map_or(0, |p| p.capacity()) as u64 <= payload.bytes());
    let owner = photocraft_doc::accounted_document::AccountedDocument::from_owned_readonly_flat_composite(
        doc,
        std::sync::Arc::new((original, index, payload)),
        q.clone(),
        &ctl(),
    )
    .unwrap();
    crate::accounted_xcf_open::separable_blend::validate_current_separable_blend_storage(&owner, &ctl()).unwrap();
    let mut out = Session::new();
    let state = crate::DocState::new_accounted(owner, None, 64, &ctl()).unwrap();
    out.publish_document_state(state, &ctl()).unwrap();
    (out, q)
}
fn wire(s: &Session, mode: i32) -> Value {
    let state = s.active().unwrap();
    json!({"layerId":ids(s)[1].0.to_string(),"sourceMode":mode,"legacy":false,
        "blendSpace":"profile-rgb","compositeSpace":"profile-rgb","operator":"clip-to-backdrop",
        "documentId":state.doc.id.0.to_string(),"expectedRevision":state.revision.to_string()})
}
#[test]
#[ignore = "UNRUN: all16 pinned RGB/Gray alpha original/edit native/raw/profile policies, paid discovery/title/native; own CURRENT persistence display, no new GIMP numerical claim"]
fn originals_title_native_and_paid_inspection() {
    for case in CASES {
        let (mut s, q) = session(case);
        original(case, &s);
        let before = s.active().unwrap().doc.clone();
        let raw = native(&s);
        let pixels = display(&s);
        let usage = q.usage().unwrap();
        let reply = crate::accounted_xcf_inspect::prepare(&s, &json!({}), &ctl()).unwrap();
        assert!(reply.quota().shares_ledger(&q));
        assert!(reply.charged_index_bytes() > 0);
        assert!(q.usage().unwrap().index > usage.index);
        assert_eq!(reply.value()["items"]["total"], 2);
        assert_eq!(reply.value()["filters"]["total"], 0);
        assert_eq!(reply.value()["filterStacks"]["total"], 0);
        drop(reply);
        assert_eq!(q.usage().unwrap(), usage);
        assert!(crate::accounted_filter_inspect::prepare(&s, &json!({"filterId":"1"}), &ctl()).is_err());
        let next = save_reopen(&mut s, "AlphaTitle.pcraft", &q);
        assert_eq!(next.active().unwrap().doc.name, "AlphaTitle.pcraft");
        assert_eq!(native(&next), raw);
        assert_eq!(display(&next), pixels);
        assert_eq!(next.active().unwrap().doc.xcf_state, before.xcf_state);
        assert_eq!(next.active().unwrap().doc.icc_profile, before.icc_profile);
        assert_eq!(next.active().unwrap().doc.source_color_context, before.source_color_context);
        assert_eq!(next.active().unwrap().doc.working_profile_interpretation, before.working_profile_interpretation);
        assert!(next.active().unwrap().doc.accounted().unwrap().is_readonly_flat_composite_storage());
    }
}
#[test]
#[ignore = "UNRUN: each RGB/Gray alpha mode both IDs compound rawzero metadata, oneundo/noopredo and continued native current state; no new source grant"]
fn metadata_raw_zero_undo_and_native_continuation() {
    for case in CASES.iter().step_by(2) {
        for target in 0..2 {
            let (mut s, q) = session(case);
            let id = ids(&s)[target];
            let before = s.active().unwrap().doc.clone();
            let raw = native(&s);
            let source = s.active().unwrap().source.clone();
            let revision = s.active().unwrap().revision;
            s.execute("layer.setProps", json!({"layer":id.0,"name":"Alpha current","visible":false,"opacity":-0.0})).unwrap();
            let current = s.active().unwrap();
            assert_eq!(current.revision, revision + 1);
            assert_eq!(current.history.past_len(), 1);
            let changed = current.doc.layers.iter().find(|l| l.id == id).unwrap();
            assert_eq!(changed.opacity.to_bits(), (-0.0f32).to_bits());
            assert!(!changed.visible);
            assert_eq!(changed.name, "Alpha current");
            let row = current.doc.xcf_state.as_ref().unwrap().items.iter().find(|r| r.item == ItemRef::Layer(id)).unwrap();
            assert_eq!(row.name, changed.name);
            assert_eq!(row.visible, changed.visible);
            assert!(row.custom_name);
            assert_eq!(
                policy(&s, 0),
                before.xcf_state.as_ref().unwrap().items.iter().find(|r| r.item == ItemRef::Layer(ids(&s)[0])).unwrap().composition.clone().unwrap()
            );
            assert_eq!(native(&s), raw);
            assert_eq!(current.source, source);
            assert_eq!(current.doc.icc_profile, before.icc_profile);
            assert!(s.undo_with_control(&ctl()).unwrap());
            assert!(s.active().unwrap().doc.shares_state(&before));
            let history = s.active().unwrap().history.clone();
            let revision = s.active().unwrap().revision;
            s.execute("layer.setProps", json!({"layer":id.0,"name":false,"visible":"ignored","opacity":null})).unwrap();
            assert_eq!(s.active().unwrap().revision, revision);
            assert!(s.active().unwrap().history.shares_accounted_history(&history));
            assert!(s.active().unwrap().history.can_redo());
            assert!(s.redo_with_control(&ctl()).unwrap());
            let pixels = display(&s);
            let mut next = save_reopen(&mut s, "AlphaMetadata.pcraft", &q);
            assert_eq!(native(&next), raw);
            assert_eq!(display(&next), pixels);
            assert_eq!(next.active().unwrap().doc.xcf_state, s.active().unwrap().doc.xcf_state);
            let next_id = ids(&next)[target];
            next.execute("layer.setProps", json!({"layer":next_id.0,"visible":true,"opacity":0.5})).unwrap();
            assert_eq!(native(&next), raw);
        }
    }
}
fn select_both(s: &mut Session) {
    let target = ids(s);
    let state = s.active().unwrap();
    let view = AccountedView::new(state.doc.accounted().unwrap(), Some(target[1]), &target, Some(target[1]), &ctl()).unwrap();
    let state = s.active_mut().unwrap();
    state.accounted_view = Some(view);
    state.active_layer = Some(target[1]);
    state.layer_anchor = Some(target[1]);
    state.selected_layers.clear();
    state.view_revision += 1;
}
#[test]
#[ignore = "UNRUN: actual RGB/Gray alpha singleton/bulk Move logical words/default/view, checked canonical bounds, zero redo/overflow atomicity/native continuation; no producer Move oracle"]
fn current_move_is_distinct_from_original_source() {
    for case in CASES.iter().step_by(2) {
        let (mut s, q) = session(case);
        let original = native(&s);
        let raw_state = s.active().unwrap().doc.xcf_state.clone();
        let profile = s.active().unwrap().doc.icc_profile.clone();
        let source_color = s.active().unwrap().doc.source_color_context.clone();
        let top = ids(&s)[1];
        s.execute("layer.translate", json!({"layer":top.0,"dx":-259,"dy":257})).unwrap();
        let moved = native(&s);
        assert_eq!(moved[0], original[0]);
        assert_eq!(moved[1].1, original[1].1);
        assert_eq!(moved[1].0.default_bytes(), original[1].0.default_bytes());
        assert_eq!(moved[1].0.bounds(), original[1].0.bounds().translate(-259, 257));
        assert_eq!(s.active().unwrap().doc.icc_profile, profile);
        assert_eq!(s.active().unwrap().doc.source_color_context, source_color);
        for i in 0..2 {
            assert_eq!(
                policy(&s, i),
                raw_state.as_ref().unwrap().items.iter().find(|r| r.item == ItemRef::Layer(ids(&s)[i])).unwrap().composition.clone().unwrap()
            );
        }
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        assert!(s.undo_with_control(&ctl()).unwrap());
        assert_eq!(native(&s), original);
        let history = s.active().unwrap().history.clone();
        s.execute("layer.translate", json!({"layer":top.0,"dx":0,"dy":0})).unwrap();
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert!(s.active().unwrap().history.can_redo());
        select_both(&mut s);
        s.execute("layer.translate", json!({"dx":-3,"dy":5})).unwrap();
        let moved = native(&s);
        for i in 0..2 {
            assert_eq!(moved[i].1, original[i].1);
            assert_eq!(moved[i].0.bounds(), original[i].0.bounds().translate(-3, 5));
        }
        let state = s.active().unwrap();
        for (i, l) in state.doc.layers.iter().enumerate() {
            assert_eq!(
                state.doc.xcf_state.as_ref().unwrap().items.iter().find(|r| r.item == ItemRef::Layer(l.id)).unwrap().raster_bounds,
                Some(moved[i].0.bounds())
            );
        }
        let before = state.doc.clone();
        let revision = state.revision;
        let journal = s.journal.clone();
        assert!(s.execute("layer.translate", json!({"dx":2147483647,"dy":1})).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.journal, journal);
        let mut next = save_reopen(&mut s, "AlphaMove.pcraft", &q);
        assert_eq!(native(&next), moved);
        let next_top = ids(&next)[1];
        next.execute("layer.translate", json!({"layer":next_top.0,"dx":1,"dy":-1})).unwrap();
        assert_eq!(native(&next)[1].1, original[1].1);
    }
}
#[test]
#[ignore = "UNRUN: RGB alpha -> existing modern explicit wire; Gray transition atomic refusal, onehistory/undo/noopredo/native continued metadata; no alpha/Gray setter grant"]
fn rgb_transition_and_gray_refusal() {
    for case in CASES.iter().step_by(2) {
        let (mut s, q) = session(case);
        let original = s.active().unwrap().doc.clone();
        let raw = native(&s);
        let rev = s.active().unwrap().revision;
        if case.model == photocraft_doc::ColorMode::Grayscale {
            let history = s.active().unwrap().history.clone();
            let journal = s.journal.clone();
            for target in ids(&s) {
                for mode in [28, 30, 31, 57, 58, 59, 60] {
                    let mut p = wire(&s, mode);
                    p["layerId"] = json!(target.0.to_string());
                    assert!(s.execute("xcf.composition.set", p).is_err());
                    assert!(s.active().unwrap().doc.shares_state(&original));
                    assert_eq!(s.active().unwrap().revision, rev);
                    assert!(s.active().unwrap().history.shares_accounted_history(&history));
                    assert_eq!(s.journal, journal);
                    assert_eq!(native(&s), raw);
                }
            }
            // Prepaid derived CURRENT control, not authored producer state:
            // base Normal is already explicit so a model-only guard after raw
            // no-op would incorrectly let the command through.
            let (mut control, _) = gray_explicit_normal_control(case);
            let before = control.active().unwrap().doc.clone();
            let h = control.active().unwrap().history.clone();
            let mut p = wire(&control, 28);
            p["layerId"] = json!(ids(&control)[0].0.to_string());
            assert!(control.execute("xcf.composition.set", p).is_err());
            let patch = photocraft_doc::accounted_document::FlatCompositeCompositionPatch {
                layer: ids(&control)[0],
                source_mode: 28,
                blend_space: GimpSpace::ProfileRgb,
                composite_space: GimpSpace::ProfileRgb,
                operator: GimpOperator::ClipToBackdrop,
            };
            assert!(before.accounted().unwrap().try_with_flat_composite_composition_patch(patch, &ctl()).is_err());
            assert!(control.active().unwrap().doc.shares_state(&before));
            assert!(control.active().unwrap().history.shares_accounted_history(&h));
            let mut next = save_reopen(&mut s, "GrayRefusal.pcraft", &q);
            assert_eq!(native(&next), raw);
            let top = ids(&next)[1];
            next.execute("layer.setProps", json!({"layer":top.0,"name":"Gray continued"})).unwrap();
            assert_eq!(native(&next), raw);
            continue;
        }
        let p = wire(&s, 30);
        s.execute("xcf.composition.set", p).unwrap();
        let changed = s.active().unwrap().doc.clone();
        let top = policy(&s, 1);
        assert_eq!(top.source_mode, 30);
        assert!(!top.legacy);
        assert_eq!(top.compatibility_epoch, "xcf-12-authored-blend-auto");
        assert_eq!((top.stored_blend_space, top.stored_composite_space, top.stored_operator), (2, 2, 2));
        assert_eq!(native(&s), raw);
        assert_eq!(s.active().unwrap().doc.icc_profile, original.icc_profile);
        assert_eq!(s.active().unwrap().doc.source_color_context, original.source_color_context);
        assert_eq!(s.active().unwrap().doc.working_profile_interpretation, original.working_profile_interpretation);
        assert_eq!(s.active().unwrap().revision, rev + 1);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        let p = wire(&s, 31);
        s.execute("xcf.composition.set", p).unwrap();
        assert!(s.undo_with_control(&ctl()).unwrap());
        let history = s.active().unwrap().history.clone();
        let revision = s.active().unwrap().revision;
        let p = wire(&s, 30);
        assert_eq!(s.execute("xcf.composition.set", p).unwrap()["updated"], true);
        assert!(s.active().unwrap().doc.shares_state(&changed));
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert!(s.active().unwrap().history.can_redo());
        assert!(s.undo_with_control(&ctl()).unwrap());
        assert!(s.active().unwrap().doc.shares_state(&original));
        assert_eq!(policy(&s, 1).legacy, case.legacy);
        assert!(s.redo_with_control(&ctl()).unwrap());
        for bad_mode in [case.mode, 20, 21] {
            let p = wire(&s, bad_mode);
            let before = s.active().unwrap().doc.clone();
            assert!(s.execute("xcf.composition.set", p).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
        }
        let mut p = wire(&s, 30);
        p["legacy"] = json!(true);
        assert!(s.execute("xcf.composition.set", p).is_err());
        let pixels = display(&s);
        let mut next = save_reopen(&mut s, "AlphaTransition.pcraft", &q);
        assert_eq!(native(&next), raw);
        assert_eq!(display(&next), pixels);
        let next_top = ids(&next)[1];
        next.execute("layer.setProps", json!({"layer":next_top.0,"name":"Modern continued"})).unwrap();
        assert_eq!(native(&next), raw);
    }
}
#[test]
#[ignore = "UNRUN: actual RGB/Gray alpha finite work/cancel/quota/stale fullhistory+view/channel fences never publish; derived pure lower alpha branches are not producer evidence"]
fn transaction_and_permission_failures_are_atomic() {
    for case in [&CASES[0], &CASES[8]] {
        let (mut s, q) = session(case);
        let owner = s.active().unwrap().doc.accounted().unwrap().clone();
        let mut rejected = Session::new();
        assert!(rejected.add_accounted_document(owner, None, &ctl()).is_err());
        assert!(rejected.docs.is_empty());
        let id = ids(&s)[1];
        let p = json!({"layer":id.0,"name":"Never published"});
        let before = s.active().unwrap().doc.clone();
        let rev = s.active().unwrap().revision;
        let work = |_| Err(BackingError::Work);
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        assert!(PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &c).is_err());
        let left = std::sync::atomic::AtomicU64::new(32);
        let debit = |n| {
            left.try_update(std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Relaxed, |v| v.checked_sub(n))
                .map(|_| ())
                .map_err(|_| BackingError::Work)
        };
        let finite = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
        assert!(PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &finite).is_err());
        let cancelled = || true;
        let interrupt = Interrupt::cancel_only(&cancelled);
        let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(PreparedAccountedCommand::prepare(&s, "layer.translate", &json!({"layer":id.0,"dx":1}), &c).is_err());
        let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
        assert!(s.execute("layer.setProps", p.clone()).is_err());
        drop(hold);
        for (command, params) in [
            ("xcf.mask.enabled.set", json!({"maskId":"1","enabled":false})),
            ("xcf.filter.enable", json!({"filterId":"1","enabled":false})),
            ("layer.setProps", json!({"blend":"multiply"})),
        ] {
            assert!(s.execute(command, params).is_err());
        }
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.active().unwrap().revision, rev);
        for fault in 0..3 {
            let (mut s, _) = session(case);
            let p = json!({"layer":ids(&s)[1].0,"name":"Stale"});
            let prepared = PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &ctl()).unwrap();
            match fault {
                0 => s.active_mut().unwrap().history.max_states += 1,
                1 => s.active_mut().unwrap().view_revision += 1,
                _ => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
            };
            let before = s.active().unwrap().doc.clone();
            let revision = s.active().unwrap().revision;
            let journal = s.journal.clone();
            assert!(prepared.execute(&mut s, &ctl()).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
            assert_eq!(s.active().unwrap().revision, revision);
            assert_eq!(s.journal, journal);
        }
    }
    // Derived pure lower controls only, NOT new GIMP source/numerical evidence.
    // StoredLinear makes transfer identity; Gray samples are exact replicated
    // colors, while original native Gray planes above remain two-channel raw.
    for builtin in [photocraft_cms::Builtin::Srgb, photocraft_cms::Builtin::SGray] {
        let working =
            photocraft_compose::gimp_composite::WorkingSpace::new(builtin.profile(), photocraft_doc::SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
        let back = [-0.0, -0.0, -0.0, 0.75];
        for mode in 57..=60 {
            let operator = if mode == 59 { GimpOperator::Union } else { GimpOperator::ClipToBackdrop };
            let result = working.separable(&working, back, [0.5, 0.5, 0.5, 0.0], 1.0, operator, photocraft_compose::gimp_blend::mode(mode).unwrap()).unwrap();
            assert_eq!(result.map(f64::to_bits), back.map(f64::to_bits));
        }
        let mode = |n| photocraft_compose::gimp_blend::mode(n).unwrap();
        let erased = working.separable(&working, [0.25, 0.25, 0.25, 0.75], [0.75, 0.75, 0.75, 0.5], 1.0, GimpOperator::ClipToBackdrop, mode(58)).unwrap();
        assert_eq!(erased.map(f64::to_bits), [0.25, 0.25, 0.25, 0.375].map(f64::to_bits));
        for (n, b, s) in [(57, [0.5, 0.5, 0.5, 0.75], [0.5, 0.5, 0.5, 1.0]), (60, [0.25, 0.25, 0.25, 0.25], [0.75, 0.75, 0.75, 0.5])] {
            assert_eq!(working.separable(&working, b, s, 1.0, GimpOperator::ClipToBackdrop, mode(n)).unwrap().map(f64::to_bits), [0u64; 4]);
        }
        // Coverage sum exceeds1: existing Merge preserves its min(backdrop,
        // 1-source) weighting, rather than normal source-over interpolation.
        let merged = working.separable(&working, [0.25, 0.25, 0.25, 0.75], [0.75, 0.75, 0.75, 0.5], 1.0, GimpOperator::Union, mode(59)).unwrap();
        assert_eq!(merged.map(f64::to_bits), [0.5, 0.5, 0.5, 1.0].map(f64::to_bits));
    }
}
