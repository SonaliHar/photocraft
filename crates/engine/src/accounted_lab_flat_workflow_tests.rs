//! UNRUN physical-Lab LCh continuation on pinned original/edit source packets.
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
        mode: 24,
        legacy: false,
        raw: [-3, -1, -2],
        blend: GimpSpace::Lab,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-lch-hue.xcf",
        sha: "1a8e021a09d6e3c1c2ca6d011921d5a2b4220ef0ca63d59fa2ffd20b9a7e6226",
        len: 5425,
        state: "composition-matrix/mode-lch-hue.fresh-source_state.json",
        state_sha: "c50972a6d68e393c7b39dda301b0c257aca7d487138a613390cb70488c834d15",
    },
    Case {
        mode: 24,
        legacy: false,
        raw: [-3, -1, -2],
        blend: GimpSpace::Lab,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-lch-hue.edit.xcf",
        sha: "db26f5a2f89d2b7c49796c42da1973392619b4ecc600205b560ea4ccdc6a9618",
        len: 5425,
        state: "composition-matrix/mode-lch-hue.fresh-edit_state.json",
        state_sha: "531b3d2d26cdd43b38966ef993135a046ef058371e6d7f9af40ffd5c25f39d0f",
    },
    Case {
        mode: 25,
        legacy: false,
        raw: [-3, -1, -2],
        blend: GimpSpace::Lab,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-lch-chroma.xcf",
        sha: "5f98eddc14e34fb9a23daed205c12b7f5acef7f02192bd3431196a4aacba7a1f",
        len: 5425,
        state: "composition-matrix/mode-lch-chroma.fresh-source_state.json",
        state_sha: "9a119e05433fa1f104c711d45364ae959062769edee6c8140e66735904f05adc",
    },
    Case {
        mode: 25,
        legacy: false,
        raw: [-3, -1, -2],
        blend: GimpSpace::Lab,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-lch-chroma.edit.xcf",
        sha: "c109c81bc3d6644ee4821f46aac2c174850e5205a4ec3fa166ef42f3473ad6a8",
        len: 5425,
        state: "composition-matrix/mode-lch-chroma.fresh-edit_state.json",
        state_sha: "1386419216612fa0f870b964841ad39620ac4f156ffea2cfdcbb45533a88cdf1",
    },
    Case {
        mode: 26,
        legacy: false,
        raw: [-3, -1, -2],
        blend: GimpSpace::Lab,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-lch-color.xcf",
        sha: "656e0ac4097e84fbb716046cf51a7a42fdc504d836c0f2306ebbde7c360b6230",
        len: 5425,
        state: "composition-matrix/mode-lch-color.fresh-source_state.json",
        state_sha: "f680add52ecd9e4e2dedccdb5de7622f539c104a0b38450fad58ee480123ede9",
    },
    Case {
        mode: 26,
        legacy: false,
        raw: [-3, -1, -2],
        blend: GimpSpace::Lab,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-lch-color.edit.xcf",
        sha: "31d3339f01dcd990a3c1fdcdf3dfc0b41e4f4cee582f13d59140c56b01e3448c",
        len: 5425,
        state: "composition-matrix/mode-lch-color.fresh-edit_state.json",
        state_sha: "f638006ebf915730ff9215e088bc0a6537182f60f6a0984e1f59eff558d7e3e9",
    },
    Case {
        mode: 27,
        legacy: false,
        raw: [-3, -1, -2],
        blend: GimpSpace::Lab,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-lch-lightness.xcf",
        sha: "7b87e8941d447077f6a9b8c568051bb43efd0491da8b76e88cacac16d4dcd9aa",
        len: 5425,
        state: "composition-matrix/mode-lch-lightness.fresh-source_state.json",
        state_sha: "904d4fdee775722ad083efd7b91b86044d78b90d2fc67419893cfa7a35919a35",
    },
    Case {
        mode: 27,
        legacy: false,
        raw: [-3, -1, -2],
        blend: GimpSpace::Lab,
        composite: GimpSpace::LinearRgb,
        file: "composition-matrix/mode-lch-lightness.edit.xcf",
        sha: "617e6fad645831455a3832b6e97a36c44fc4f1269052c5f7596648b7cb64eaf4",
        len: 5425,
        state: "composition-matrix/mode-lch-lightness.fresh-edit_state.json",
        state_sha: "29957c9892ae52d760172fd50ff85a10361c885a80133023168213a50fe074c5",
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
    s.add_owned_separable_blend_xcf_import(&source, "LCh original", Some(case.file), &ctl()).unwrap();
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
    assert_eq!(doc.icc_profile.as_deref().map(Vec::as_slice), photocraft_cms::Builtin::Srgb.profile().encoded_bytes());
    assert!(matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. }));
    // Lab is canonical physical CIE Lab. ImportCompositing carries only RGB
    // transfer encodings, so the original foreground has no physical surrogate.
    assert!(doc.layers[1].import_compositing.is_none());
    assert!(doc.layers[0].import_compositing.is_some());

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
    reopened.add_owned_native_separable_blend_archive(archive, "continued", Some("actual/LCh.pcraft"), &ctl()).unwrap();
    reopened
}
fn wire(s: &Session, mode: i32) -> Value {
    let state = s.active().unwrap();
    json!({"layerId":ids(s)[1].0.to_string(),"sourceMode":mode,"legacy":false,
        "blendSpace":"profile-rgb","compositeSpace":"profile-rgb","operator":"clip-to-backdrop",
        "documentId":state.doc.id.0.to_string(),"expectedRevision":state.revision.to_string()})
}
#[test]
#[ignore = "UNRUN: all8 pinned physical-Lab LCh original/edit native/raw/profile policies, paid discovery/title/native; own CURRENT persistence display, no new GIMP numerical claim"]
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
        let next = save_reopen(&mut s, "LChTitle.pcraft", &q);
        assert_eq!(next.active().unwrap().doc.name, "LChTitle.pcraft");
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
#[ignore = "UNRUN: each LCh mode both IDs compound rawzero metadata, oneundo/noopredo and continued native current state; no new source grant"]
fn metadata_raw_zero_undo_and_native_continuation() {
    for case in CASES.iter().step_by(2) {
        for target in 0..2 {
            let (mut s, q) = session(case);
            let id = ids(&s)[target];
            let before = s.active().unwrap().doc.clone();
            let raw = native(&s);
            let source = s.active().unwrap().source.clone();
            let revision = s.active().unwrap().revision;
            s.execute("layer.setProps", json!({"layer":id.0,"name":"LCh current","visible":false,"opacity":-0.0})).unwrap();
            let current = s.active().unwrap();
            assert_eq!(current.revision, revision + 1);
            assert_eq!(current.history.past_len(), 1);
            let changed = current.doc.layers.iter().find(|l| l.id == id).unwrap();
            assert_eq!(changed.opacity.to_bits(), (-0.0f32).to_bits());
            assert!(!changed.visible);
            assert_eq!(changed.name, "LCh current");
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
            let mut next = save_reopen(&mut s, "LChMetadata.pcraft", &q);
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
#[ignore = "UNRUN: actual LCh singleton/bulk Move logical words/default/view, checked canonical bounds, zero redo/overflow atomicity/native continuation; no producer Move oracle"]
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
        let mut next = save_reopen(&mut s, "LChMove.pcraft", &q);
        assert_eq!(native(&next), moved);
        let next_top = ids(&next)[1];
        next.execute("layer.translate", json!({"layer":next_top.0,"dx":1,"dy":-1})).unwrap();
        assert_eq!(native(&next)[1].1, original[1].1);
    }
}
#[test]
#[ignore = "UNRUN: LCh raw policy -> existing modern explicit wire only, onehistory/undo/noopredo/native continued metadata; no Lab/LCh setter grant"]
fn lab_to_existing_modern_composition_transition() {
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
        assert!(s.active().unwrap().doc.layers[1].import_compositing.is_some());
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
        assert!(s.active().unwrap().doc.layers[1].import_compositing.is_none());
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
        let mut next = save_reopen(&mut s, "LChTransition.pcraft", &q);
        assert_eq!(native(&next), raw);
        assert_eq!(display(&next), pixels);
        let next_top = ids(&next)[1];
        next.execute("layer.setProps", json!({"layer":next_top.0,"name":"Modern continued"})).unwrap();
        assert_eq!(native(&next), raw);
    }
}
#[test]
#[ignore = "UNRUN: actual LCh finite work/cancel/quota/stale fullhistory+view/channel fences and typed native cancellation never publish a prefix"]
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
    // Typed native prepublication cancellation is tested with the real archive
    // and actual same ledger, never via generic/fabricated class adoption.
    let (s, q) = session(&CASES[0]);
    let bytes =
        photocraft_format::owned_document::save_current_document_archive(&s.active().unwrap().doc, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
            .unwrap()
            .into_inner();
    let archive =
        photocraft_io::accounted_native::open_accounted_native_from_reader(&mut Cursor::new(&bytes), bytes.len() as u64, Default::default(), q.clone(), &ctl())
            .unwrap();
    let mut refused = Session::new();
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(refused.add_owned_native_separable_blend_archive(archive, "cancelled", None, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert!(refused.docs.is_empty());
}
