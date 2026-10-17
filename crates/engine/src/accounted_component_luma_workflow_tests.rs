//! UNRUN legacy/modern component and luma continuation on pinned original/edit source packets.
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
        mode: 11,
        legacy: true,
        raw: [-2, -2, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::ProfileRgb,
        file: "composition-matrix/mode-hsv-hue-legacy.xcf",
        sha: "199a4e935c0a41c269c37e96034fe10c1cd25da37bf5dc429e77d4365d0cd29c",
        len: 5425,
        state: "composition-matrix/mode-hsv-hue-legacy.fresh-source_state.json",
        state_sha: "7ab434a9c40bca88fbc32c4b3922073267ce8676011b2a3968aed12ffeee5fbf",
    },
    Case {
        mode: 11,
        legacy: true,
        raw: [-2, -2, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::ProfileRgb,
        file: "composition-matrix/mode-hsv-hue-legacy.edit.xcf",
        sha: "e6a0b51f5c066affb7410db75b346d77adea642e20d430b13c0db465089a0356",
        len: 5425,
        state: "composition-matrix/mode-hsv-hue-legacy.fresh-edit_state.json",
        state_sha: "505f44ac74d65b65fa668e4034a7ebe6e880757211db0c4e5daf9e4d74d344f0",
    },
    Case {
        mode: 12,
        legacy: true,
        raw: [-2, -2, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::ProfileRgb,
        file: "composition-matrix/mode-hsv-saturation-legacy.xcf",
        sha: "d08ef0c7acef8cc8c7e5c4724ef76a3c5b74e619c672f1b7b8c327c554faafb6",
        len: 5425,
        state: "composition-matrix/mode-hsv-saturation-legacy.fresh-source_state.json",
        state_sha: "1a993c7ddd063122ee59361abed31a96f0ef3701d8441e0df09e094479f81310",
    },
    Case {
        mode: 12,
        legacy: true,
        raw: [-2, -2, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::ProfileRgb,
        file: "composition-matrix/mode-hsv-saturation-legacy.edit.xcf",
        sha: "79254653da78290a7d58938b05d62bc3e8f7d9efee127e90228e11f6ca217237",
        len: 5425,
        state: "composition-matrix/mode-hsv-saturation-legacy.fresh-edit_state.json",
        state_sha: "8817214636bc58612976974dd9de7ea7484551a6aa7746e97ceed17361b18749",
    },
    Case {
        mode: 13,
        legacy: true,
        raw: [-2, -2, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::ProfileRgb,
        file: "composition-matrix/mode-hsl-color-legacy.xcf",
        sha: "f44a49fb38ba3a574e5c778b6625f1ee0f4b85f3077fb817123499e59431cb02",
        len: 5425,
        state: "composition-matrix/mode-hsl-color-legacy.fresh-source_state.json",
        state_sha: "89cd0624d665a8a89dd4ae70823a4bca7316e769bacd48c2a856360483bcb4f0",
    },
    Case {
        mode: 13,
        legacy: true,
        raw: [-2, -2, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::ProfileRgb,
        file: "composition-matrix/mode-hsl-color-legacy.edit.xcf",
        sha: "c153bb5711c96835e25e95e617e40b0a5a2bd948392c354b3c5a16c08e33d2a5",
        len: 5425,
        state: "composition-matrix/mode-hsl-color-legacy.fresh-edit_state.json",
        state_sha: "7f5a8c5800483323eb22a145c28e65780b73f73cafc915aa6d310e55c6fd76e0",
    },
    Case {
        mode: 14,
        legacy: true,
        raw: [-2, -2, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::ProfileRgb,
        file: "composition-matrix/mode-hsv-value-legacy.xcf",
        sha: "96e77593f731a6048110e523e84d2dc6bbf5ef92c18bf11cd1d75fdbe2b85331",
        len: 5425,
        state: "composition-matrix/mode-hsv-value-legacy.fresh-source_state.json",
        state_sha: "87fbd0faad2b3fd8865e1c2d3e8a3e8e48cbaa0a316ede534dc0c9f1ecba0d64",
    },
    Case {
        mode: 14,
        legacy: true,
        raw: [-2, -2, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::ProfileRgb,
        file: "composition-matrix/mode-hsv-value-legacy.edit.xcf",
        sha: "c927a2f12dda9060456154077c3e29e31940c910f483d07d3d8c0a4f758017e4",
        len: 5425,
        state: "composition-matrix/mode-hsv-value-legacy.fresh-edit_state.json",
        state_sha: "93ac16b18782aca250380d4da9908e6bdb42448f2e2af939ec2f1de05e76c6f5",
    },
    Case {
        mode: 37,
        legacy: false,
        raw: [-2, -1, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-hsv-hue.xcf",
        sha: "c3c7222d1c04228c4424ae6dbe904c5d5f97f838edab0df321dd2b4b64f0202e",
        len: 5425,
        state: "composition-matrix/mode-hsv-hue.fresh-source_state.json",
        state_sha: "57b153c62aea4add7f32ec8ffde6c5ae4c8c1a9e07f470bceaf53fc77f8a91cd",
    },
    Case {
        mode: 37,
        legacy: false,
        raw: [-2, -1, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-hsv-hue.edit.xcf",
        sha: "92bf51dc0e56a8fe79eca0bb1cb3b744d875b60f9772f0fee007fa8828d3e73f",
        len: 5425,
        state: "composition-matrix/mode-hsv-hue.fresh-edit_state.json",
        state_sha: "1cdc28eacefe4789490f6015ac63975fc904964425878d518b6f39080f001e12",
    },
    Case {
        mode: 38,
        legacy: false,
        raw: [-2, -1, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-hsv-saturation.xcf",
        sha: "371826088442cf3c034d00abf10c8e91cdbcb502b24ffa836c11775629549d94",
        len: 5425,
        state: "composition-matrix/mode-hsv-saturation.fresh-source_state.json",
        state_sha: "510f0baa5bcb2203dd6388f1b7e351c261f075dd4e91ca8eaa6f7063ec5367d9",
    },
    Case {
        mode: 38,
        legacy: false,
        raw: [-2, -1, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-hsv-saturation.edit.xcf",
        sha: "720861856428a4eb0b138ab9e8e5a50f3fa61ae236196af9391f3dbd14e72f17",
        len: 5425,
        state: "composition-matrix/mode-hsv-saturation.fresh-edit_state.json",
        state_sha: "b25914a6fb3692bea7bef264e1d91c4770d6d20000cbc86fde8a8ee1d9997701",
    },
    Case {
        mode: 39,
        legacy: false,
        raw: [-2, -1, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-hsl-color.xcf",
        sha: "205e9879b61586f15d39c33ad88f6521aad496b1ae563442119c35dab406bc53",
        len: 5425,
        state: "composition-matrix/mode-hsl-color.fresh-source_state.json",
        state_sha: "0da91e5f1ecd317a7b2761d90b7e5817dbd1635a0466106eae5a94d81dc61797",
    },
    Case {
        mode: 39,
        legacy: false,
        raw: [-2, -1, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-hsl-color.edit.xcf",
        sha: "df1755d7b6427bc620f9bee536156dc833e36d6ac3446ca73e8eb69d5a0349f2",
        len: 5425,
        state: "composition-matrix/mode-hsl-color.fresh-edit_state.json",
        state_sha: "3763d6b0882590ef9d90cc2407c5fb8c05f7061714c6ff2df80010aa57b63fcf",
    },
    Case {
        mode: 40,
        legacy: false,
        raw: [-2, -1, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-hsv-value.xcf",
        sha: "601898d0fe3152aa58480a8ac4e892eafe7118ba579e96a10bcd2df505ddd696",
        len: 5425,
        state: "composition-matrix/mode-hsv-value.fresh-source_state.json",
        state_sha: "b8796328c6dd740c759a08a0c4668ea5a4a23fc27269c4d5626fe1a37181d696",
    },
    Case {
        mode: 40,
        legacy: false,
        raw: [-2, -1, -2],
        blend: GimpSpace::ProfileRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-hsv-value.edit.xcf",
        sha: "63db78fc8007a36d9b151c6116288890b3aabe5511f6e9d2992ff24899b35762",
        len: 5425,
        state: "composition-matrix/mode-hsv-value.fresh-edit_state.json",
        state_sha: "812c26bed26289c7bee80e302fcc775308d958e0519be0e56bf0acc638feda6a",
    },
    Case {
        mode: 54,
        legacy: false,
        raw: [-1, -1, -2],
        blend: GimpSpace::LinearRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-luma-darken-only.xcf",
        sha: "82168822453034ad39ba38bbdc2b161c05062757e22744cb5cf6ce0a5a114f5a",
        len: 5425,
        state: "composition-matrix/mode-luma-darken-only.fresh-source_state.json",
        state_sha: "da73e62690818e01c13bf198a9666a805457936fcfb4d17e565dee00cfc33674",
    },
    Case {
        mode: 54,
        legacy: false,
        raw: [-1, -1, -2],
        blend: GimpSpace::LinearRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-luma-darken-only.edit.xcf",
        sha: "2f2a65180e2e9e6f918c02b5892d7cc2528c7de6a8f5d0cb1770b407deecc918",
        len: 5425,
        state: "composition-matrix/mode-luma-darken-only.fresh-edit_state.json",
        state_sha: "a1d92f291164eb5d2d1bb11af9cbe6df412b1c1729cf455fb723fdfd6861699f",
    },
    Case {
        mode: 55,
        legacy: false,
        raw: [-1, -1, -2],
        blend: GimpSpace::LinearRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-luma-lighten-only.xcf",
        sha: "d987678275e33745283f2914543fda48c037f96a7da6781a58e5e5ba387ab21c",
        len: 5425,
        state: "composition-matrix/mode-luma-lighten-only.fresh-source_state.json",
        state_sha: "c81e5b95403d47bf25aa1b96ae5423811880c958ee3d9f2d3e6968faaa9792f4",
    },
    Case {
        mode: 55,
        legacy: false,
        raw: [-1, -1, -2],
        blend: GimpSpace::LinearRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-luma-lighten-only.edit.xcf",
        sha: "394fd6f196f67a6d5ad4ff6299042383c6b392c4591685eb8f7f32b5cf02e76c",
        len: 5425,
        state: "composition-matrix/mode-luma-lighten-only.fresh-edit_state.json",
        state_sha: "75b096a282b3f58e2ee63c1ae5e5440c6738df28dd9e18800e60f6419a8ef359",
    },
    Case {
        mode: 56,
        legacy: false,
        raw: [-1, -1, -2],
        blend: GimpSpace::LinearRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-luminance.xcf",
        sha: "df61e7f07f9a15200cedca0e4ec620786ea6fe9b65f56529e2ed6388976a312b",
        len: 5425,
        state: "composition-matrix/mode-luminance.fresh-source_state.json",
        state_sha: "9e7802c0c72eb07ba5f541560e43916b95e602e9387116afac6097ac00c63ba5",
    },
    Case {
        mode: 56,
        legacy: false,
        raw: [-1, -1, -2],
        blend: GimpSpace::LinearRgb,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-luminance.edit.xcf",
        sha: "110603b2ee7c17aac0cfb944fd1e8123d51da61c80b55a88f115b8ec9b1dbd11",
        len: 5425,
        state: "composition-matrix/mode-luminance.fresh-edit_state.json",
        state_sha: "47b15efb5404595f5c4b2b54e7f2930f9acbfa63bf2a07c68e2115463bb3d51c",
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
    s.add_owned_separable_blend_xcf_import(&source, "Component original", Some(case.file), &ctl()).unwrap();
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
    assert_eq!(doc.mode, photocraft_doc::ColorMode::Rgb);
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
    assert_eq!(resolved.operator, GimpOperator::ClipToBackdrop);
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
    let state: Value = serde_json::from_slice(&checked(case.state, case.state_sha)).unwrap();
    assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
    assert!(state["assigned_icc_sha256"].is_null());
    let captures = native(s);
    for (i, row) in state["layers"].as_array().unwrap().iter().rev().enumerate() {
        let blob = &row["native"];
        assert_eq!(blob["byte_order"], "little");
        let file = format!("composition-matrix/{}", blob["file"].as_str().unwrap());
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
    reopened.add_owned_native_separable_blend_archive(archive, "continued", Some("actual/Legacy.pcraft"), &ctl()).unwrap();
    reopened
}
fn wire(s: &Session, mode: i32) -> Value {
    let state = s.active().unwrap();
    json!({"layerId":ids(s)[1].0.to_string(),"sourceMode":mode,"legacy":false,
        "blendSpace":"profile-rgb","compositeSpace":"profile-rgb","operator":"clip-to-backdrop",
        "documentId":state.doc.id.0.to_string(),"expectedRevision":state.revision.to_string()})
}
#[test]
#[ignore = "UNRUN: all22 pinned legacy/modern component and luma original/edit native/raw/profile policies, paid discovery/title/native; own CURRENT persistence display, no new GIMP numerical claim"]
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
        let next = save_reopen(&mut s, "ComponentTitle.pcraft", &q);
        assert_eq!(next.active().unwrap().doc.name, "ComponentTitle.pcraft");
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
#[ignore = "UNRUN: each component/luma mode both IDs compound rawzero metadata, oneundo/noopredo and continued native current state; no new source grant"]
fn metadata_raw_zero_undo_and_native_continuation() {
    for case in CASES.iter().step_by(2) {
        for target in 0..2 {
            let (mut s, q) = session(case);
            let id = ids(&s)[target];
            let before = s.active().unwrap().doc.clone();
            let raw = native(&s);
            let source = s.active().unwrap().source.clone();
            let revision = s.active().unwrap().revision;
            s.execute("layer.setProps", json!({"layer":id.0,"name":"Component current","visible":false,"opacity":-0.0})).unwrap();
            let current = s.active().unwrap();
            assert_eq!(current.revision, revision + 1);
            assert_eq!(current.history.past_len(), 1);
            let changed = current.doc.layers.iter().find(|l| l.id == id).unwrap();
            assert_eq!(changed.opacity.to_bits(), (-0.0f32).to_bits());
            assert!(!changed.visible);
            assert_eq!(changed.name, "Component current");
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
            let mut next = save_reopen(&mut s, "ComponentMetadata.pcraft", &q);
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
#[ignore = "UNRUN: actual component/luma singleton/bulk Move logical words/default/view, checked canonical bounds, zero redo/overflow atomicity/native continuation; no producer Move oracle"]
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
        let mut next = save_reopen(&mut s, "ComponentMove.pcraft", &q);
        assert_eq!(native(&next), moved);
        let next_top = ids(&next)[1];
        next.execute("layer.translate", json!({"layer":next_top.0,"dx":1,"dy":-1})).unwrap();
        assert_eq!(native(&next)[1].1, original[1].1);
    }
}
#[test]
#[ignore = "UNRUN: component/luma raw policy -> existing modern explicit wire only, onehistory/undo/noopredo/native continued metadata; no component/luma/legacy setter grant"]
fn components_to_existing_modern_composition_transition() {
    for case in CASES.iter().step_by(2) {
        let (mut s, q) = session(case);
        let original = s.active().unwrap().doc.clone();
        let raw = native(&s);
        let rev = s.active().unwrap().revision;
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
        let mut next = save_reopen(&mut s, "ComponentTransition.pcraft", &q);
        assert_eq!(native(&next), raw);
        assert_eq!(display(&next), pixels);
        let next_top = ids(&next)[1];
        next.execute("layer.setProps", json!({"layer":next_top.0,"name":"Modern continued"})).unwrap();
        assert_eq!(native(&next), raw);
    }
}
#[test]
#[ignore = "UNRUN: actual component/luma finite work/cancel/quota/stale fullhistory+view/channel fences never publish; derived pure lower luma tie/zeroY refusals are not producer evidence"]
fn transaction_and_permission_failures_are_atomic() {
    let (mut s, q) = session(&CASES[0]);
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
        let (mut s, _) = session(&CASES[0]);
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
    // Derived pure CURRENT lower contract, NOT captured GIMP source/evidence.
    // Positive alpha/opacity reaches the actual luminance branch; using stored
    // Linear makes transfer identity and preserves the commutative equal-Y tie.
    let profile = photocraft_cms::Builtin::Srgb.profile();
    let weights = profile.matrix.as_ref().unwrap()[1];
    assert!(weights.iter().all(|v| v.is_finite() && *v > 0.0 && *v <= 1.0));
    let working = photocraft_compose::gimp_composite::WorkingSpace::new(profile, photocraft_doc::SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    let backdrop = [weights[1], 0.0, 0.0, 1.0];
    let source = [0.0, weights[0], 0.0, 1.0];
    for mode in [54, 55] {
        assert!(working.separable(&working, backdrop, source, 1.0, GimpOperator::ClipToBackdrop, photocraft_compose::gimp_blend::mode(mode).unwrap()).is_err());
    }
    assert!(
        working
            .separable(
                &working,
                [0.0, 0.0, 0.0, 1.0],
                [0.5, 0.25, 0.75, 1.0],
                1.0,
                GimpOperator::ClipToBackdrop,
                photocraft_compose::gimp_blend::mode(56).unwrap()
            )
            .is_err()
    );
}
