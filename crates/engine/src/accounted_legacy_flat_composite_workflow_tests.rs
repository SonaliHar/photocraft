//! UNRUN legacy14 continuation on pinned original/edit source packets.
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
    file: &'static str,
    sha: &'static str,
    len: usize,
    state: &'static str,
    state_sha: &'static str,
}
const CASES: &[Case] = &[
    Case {
        mode: 3,
        file: "composition-matrix/mode-multiply-legacy.xcf",
        sha: "5e360b5ef372f417cb6d9d7bdf4d1ae45ee8404f81c6c3515a770cdfdf260724",
        len: 5425,
        state: "composition-matrix/mode-multiply-legacy.fresh-source_state.json",
        state_sha: "5f2d576a1c1f7166c428145b9e87c9e27e692761f00eb2f9e307372b9fe98331",
    },
    Case {
        mode: 3,
        file: "composition-matrix/mode-multiply-legacy.edit.xcf",
        sha: "e688b4f351ea2cceac7f8aa91c13a8fe171e7b38a9029289ec3fc34e68f1200f",
        len: 5425,
        state: "composition-matrix/mode-multiply-legacy.fresh-edit_state.json",
        state_sha: "5cd1b769cb14957d69f31d4613374b60db3ea4f816a5c2e2ed88e7a92f005376",
    },
    Case {
        mode: 4,
        file: "composition-matrix/mode-screen-legacy.xcf",
        sha: "5549e0664cb0c5fe4d789f7051df82e5166c827b5cda5399de358e0f77f5f65b",
        len: 5425,
        state: "composition-matrix/mode-screen-legacy.fresh-source_state.json",
        state_sha: "6dc513e6040235d66777fbf90da4bb597b9c06b06f547397996f6c42509fa4e4",
    },
    Case {
        mode: 4,
        file: "composition-matrix/mode-screen-legacy.edit.xcf",
        sha: "2bd243b2c860674bd27b98508b537cb1905cd073f64503ca3065cc3db78a5ef2",
        len: 5425,
        state: "composition-matrix/mode-screen-legacy.fresh-edit_state.json",
        state_sha: "2ada33d5b69a5e81b058732f906d27fc8d2498de4e786152b848f5fe1f95fd9f",
    },
    Case {
        mode: 6,
        file: "composition-matrix/mode-difference-legacy.xcf",
        sha: "631a21065ee78a11e27fb30b94cd5e4d6de498cb473ac24c393d957e78955c10",
        len: 5425,
        state: "composition-matrix/mode-difference-legacy.fresh-source_state.json",
        state_sha: "3bf5af0bc9d71a09462d535aaeb96e9d88d4c7602f32df7cb3ed77f42929a39d",
    },
    Case {
        mode: 6,
        file: "composition-matrix/mode-difference-legacy.edit.xcf",
        sha: "25ae92dcddb975ffe28ecd52ca7f6a2e973c15b0bb33ced4a6e7503250db39cc",
        len: 5425,
        state: "composition-matrix/mode-difference-legacy.fresh-edit_state.json",
        state_sha: "b9610a9da2210600fcbcd5e43aadce2a9686ab5386df4ca948f72c3a2264509c",
    },
    Case {
        mode: 7,
        file: "composition-matrix/mode-addition-legacy.xcf",
        sha: "331cf71d7c4190361dc0669587d6db185e6805c599aa89bc5c5264a364dccfe2",
        len: 5425,
        state: "composition-matrix/mode-addition-legacy.fresh-source_state.json",
        state_sha: "b9089bb492ea451c560cdc4d85707d689222758e808615394df45f8d93edd805",
    },
    Case {
        mode: 7,
        file: "composition-matrix/mode-addition-legacy.edit.xcf",
        sha: "c8de95694fac48d388fc2784f2af18d278e99fcd81ec58df3d85de281c73d858",
        len: 5425,
        state: "composition-matrix/mode-addition-legacy.fresh-edit_state.json",
        state_sha: "34fb58da31875a35ab29f49e64674afe00c3c8f1a4c7fd3a9ee9e63dfcf7b91a",
    },
    Case {
        mode: 8,
        file: "composition-matrix/mode-subtract-legacy.xcf",
        sha: "a1d3576a425099b68c08e6509b219abae91049036b1f1d02c764c308db50a93a",
        len: 5425,
        state: "composition-matrix/mode-subtract-legacy.fresh-source_state.json",
        state_sha: "f9e60de686cb5f87c44c475e5ab7057771d2d7005dc025be314ebba328af6006",
    },
    Case {
        mode: 8,
        file: "composition-matrix/mode-subtract-legacy.edit.xcf",
        sha: "43700e9b6f15513ed5f3978270e53a81190a0827169dd7fc679bd94a1ad271af",
        len: 5425,
        state: "composition-matrix/mode-subtract-legacy.fresh-edit_state.json",
        state_sha: "f143c35c15345614550621c95a8ecef10c16b15be9905f9ad9622090ae19f6a1",
    },
    Case {
        mode: 9,
        file: "composition-matrix/mode-darken-only-legacy.xcf",
        sha: "f131a3acdebcdf21d094113e8de49f11323fac02f1a6b1b2bd2eaac215b60cef",
        len: 5425,
        state: "composition-matrix/mode-darken-only-legacy.fresh-source_state.json",
        state_sha: "b51ba9ab5a472eb81d20c7f74f7199ca4bb985148323fdf9a31f5582d81cf99a",
    },
    Case {
        mode: 9,
        file: "composition-matrix/mode-darken-only-legacy.edit.xcf",
        sha: "e7af316d808acdf0c644d9147d3b43c484e337371003dd0682f61d9eec507f79",
        len: 5425,
        state: "composition-matrix/mode-darken-only-legacy.fresh-edit_state.json",
        state_sha: "23959169853693fc9f495014b02a28d170bd1ecef17de8fe852ff032c305add8",
    },
    Case {
        mode: 10,
        file: "composition-matrix/mode-lighten-only-legacy.xcf",
        sha: "c1227beafcc354d039ad4a1acad97e420aa74bb4b330d729ec8367d5e9a6d63e",
        len: 5425,
        state: "composition-matrix/mode-lighten-only-legacy.fresh-source_state.json",
        state_sha: "04e8755838068d61bed41eef35759d8d9640547956f94b98a68ff5ab83bbbf3d",
    },
    Case {
        mode: 10,
        file: "composition-matrix/mode-lighten-only-legacy.edit.xcf",
        sha: "acc87218736dbc47856d2156477898d724e4cb34cf5e05ba671643ea5a245655",
        len: 5425,
        state: "composition-matrix/mode-lighten-only-legacy.fresh-edit_state.json",
        state_sha: "8c1e3ad6354311c48d3317492dd800a16c26a92a8cb86264f4bb289f71592abc",
    },
    Case {
        mode: 15,
        file: "composition-matrix/mode-divide-legacy.xcf",
        sha: "4123a81300930c61dcb9d87b3c1b0e345413e12970e4e0aedd077b5b00ef85ad",
        len: 5425,
        state: "composition-matrix/mode-divide-legacy.fresh-source_state.json",
        state_sha: "9c20b4a7d2c045d00567eaa7504f84ab6a10a8449d1a774f65854a1700e4d422",
    },
    Case {
        mode: 15,
        file: "composition-matrix/mode-divide-legacy.edit.xcf",
        sha: "6ef837a41f158cd07181bf99e011c94791d9dd450c26c5e4c0796f4630e93fe2",
        len: 5425,
        state: "composition-matrix/mode-divide-legacy.fresh-edit_state.json",
        state_sha: "e206d0c36f974e3ea21fe7e10150023b09a1e664d9533c8bf5a784c06ac124c5",
    },
    Case {
        mode: 16,
        file: "composition-matrix/mode-dodge-legacy.xcf",
        sha: "04a5a327080689e01e912d6ba5b80d35f43740957424e0fdcfc78415cfa00dd2",
        len: 5425,
        state: "composition-matrix/mode-dodge-legacy.fresh-source_state.json",
        state_sha: "b723a5b655d6fa1cd4884f37e9330d35a5331fdf9d856491e98767d3ac958933",
    },
    Case {
        mode: 16,
        file: "composition-matrix/mode-dodge-legacy.edit.xcf",
        sha: "d701e5871058041b5e85a4bbbe638fb19b57958039e2d84bf9a428afeb2061c4",
        len: 5425,
        state: "composition-matrix/mode-dodge-legacy.fresh-edit_state.json",
        state_sha: "deb07f2dea152044aa68bec0d0cd4ce9eb9bdbb75c8d3283b605b3bb9d3157de",
    },
    Case {
        mode: 17,
        file: "composition-matrix/mode-burn-legacy.xcf",
        sha: "3353f9add2f61f63de5472fb2a05b5715e77b79cd1ad5064879603014827d303",
        len: 5425,
        state: "composition-matrix/mode-burn-legacy.fresh-source_state.json",
        state_sha: "f40e7ab19bc3621885eac19ca8b5ece98e79ee0b4bd03fe0261c301399613cd7",
    },
    Case {
        mode: 17,
        file: "composition-matrix/mode-burn-legacy.edit.xcf",
        sha: "c42e0514144502977746f75d89c497263eb84e4fe704c8e4dcedd9bb18d52e32",
        len: 5425,
        state: "composition-matrix/mode-burn-legacy.fresh-edit_state.json",
        state_sha: "7bc9bcf98f37d656b9cb9b65d1820415930313257fe004a76621036f0a40c703",
    },
    Case {
        mode: 18,
        file: "composition-matrix/mode-hardlight-legacy.xcf",
        sha: "8efcdc27963ee1a9c8de0cb0b707b69f8de9f108c419acfbdf3ee2ba13336aaf",
        len: 5425,
        state: "composition-matrix/mode-hardlight-legacy.fresh-source_state.json",
        state_sha: "7a09c1d0aee3e6834b0c85d04d559964a2428533e0bb18aa2932469acdd46dbb",
    },
    Case {
        mode: 18,
        file: "composition-matrix/mode-hardlight-legacy.edit.xcf",
        sha: "5d77d1c54445c281ba9b8d4e8de91f73df62b1e97e082ee830697d3c045b5952",
        len: 5425,
        state: "composition-matrix/mode-hardlight-legacy.fresh-edit_state.json",
        state_sha: "a7c5f19ce9635b9184ca800e7ae01070064924586d908233c3b0d1522a0deeac",
    },
    Case {
        mode: 19,
        file: "composition-matrix/mode-softlight-legacy.xcf",
        sha: "e55f2a8270fed8c422c074c267eabce53be7381acc917be56c98de4195ec4d26",
        len: 5425,
        state: "composition-matrix/mode-softlight-legacy.fresh-source_state.json",
        state_sha: "a5c2b93849fc9f83bba8f952ba5b2a6eadf3568f296152172f7f012484597e76",
    },
    Case {
        mode: 19,
        file: "composition-matrix/mode-softlight-legacy.edit.xcf",
        sha: "9a6deffbbd711673870bfc663d4fc991feb7ca2c1e249bb1a15b31c1c8fdef97",
        len: 5425,
        state: "composition-matrix/mode-softlight-legacy.fresh-edit_state.json",
        state_sha: "0f63d2abbe27fda94f90653eda050e992dc030a56e73747d6e397b4f2e8dbb1b",
    },
    Case {
        mode: 20,
        file: "composition-matrix/mode-grain-extract-legacy.xcf",
        sha: "fe51d57e6edcb8b7e89a0ec8561b856a5395799b99640a32323a24bda019c060",
        len: 5425,
        state: "composition-matrix/mode-grain-extract-legacy.fresh-source_state.json",
        state_sha: "342e97d7fd8e6603407e16fbac40768935c564b8b6a82025bebf578679548431",
    },
    Case {
        mode: 20,
        file: "composition-matrix/mode-grain-extract-legacy.edit.xcf",
        sha: "ea3dcb513399a814fa1cd066b889e3735a13338f1a9a5a61eaae9878275c4263",
        len: 5425,
        state: "composition-matrix/mode-grain-extract-legacy.fresh-edit_state.json",
        state_sha: "3b34179bde8e751d831214e6a18b636a8739b04dfef0a90a292611e977140dd3",
    },
    Case {
        mode: 21,
        file: "composition-matrix/mode-grain-merge-legacy.xcf",
        sha: "c9d3e10c9f938508b581fc1e3f3fbc1b75ecf8783c3b1d6ac999f95a795d9f40",
        len: 5425,
        state: "composition-matrix/mode-grain-merge-legacy.fresh-source_state.json",
        state_sha: "af5a3266141855cd7e54afac961c656336966bb05f60133f3637a95e95a71e65",
    },
    Case {
        mode: 21,
        file: "composition-matrix/mode-grain-merge-legacy.edit.xcf",
        sha: "9128f7d994b69971ab8d50c78a2a0fd420bf37014cc58600bf139c515101249b",
        len: 5425,
        state: "composition-matrix/mode-grain-merge-legacy.fresh-edit_state.json",
        state_sha: "aa3f9fb505aa050801eba0aa3c00d36960d1c39d88b790b4228c4716f94fe641",
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
    s.add_owned_separable_blend_xcf_import(&source, "Legacy original", Some(case.file), &ctl()).unwrap();
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
    assert!(top.legacy);
    assert_eq!(top.compatibility_epoch, "xcf-12-authored-blend-auto");
    assert_eq!((top.stored_blend_space, top.stored_composite_space, top.stored_operator), (-2, -2, -2));
    let resolved = top.resolved.unwrap();
    assert_eq!(resolved.blend_space, GimpSpace::ProfileRgb);
    assert_eq!(resolved.composite_space, GimpSpace::ProfileRgb);
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
#[ignore = "UNRUN: all28 pinned legacy14 original/edit native/raw/profile policies, paid discovery/title/native; own CURRENT persistence display, no new GIMP numerical claim"]
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
        let next = save_reopen(&mut s, "LegacyTitle.pcraft", &q);
        assert_eq!(next.active().unwrap().doc.name, "LegacyTitle.pcraft");
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
#[ignore = "UNRUN: each legacy mode both IDs compound rawzero metadata, oneundo/noopredo and continued native current state; no new source grant"]
fn metadata_raw_zero_undo_and_native_continuation() {
    for case in CASES.iter().step_by(2) {
        for target in 0..2 {
            let (mut s, q) = session(case);
            let id = ids(&s)[target];
            let before = s.active().unwrap().doc.clone();
            let raw = native(&s);
            let source = s.active().unwrap().source.clone();
            let revision = s.active().unwrap().revision;
            s.execute("layer.setProps", json!({"layer":id.0,"name":"Legacy current","visible":false,"opacity":-0.0})).unwrap();
            let current = s.active().unwrap();
            assert_eq!(current.revision, revision + 1);
            assert_eq!(current.history.past_len(), 1);
            let changed = current.doc.layers.iter().find(|l| l.id == id).unwrap();
            assert_eq!(changed.opacity.to_bits(), (-0.0f32).to_bits());
            assert!(!changed.visible);
            assert_eq!(changed.name, "Legacy current");
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
            let mut next = save_reopen(&mut s, "LegacyMetadata.pcraft", &q);
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
#[ignore = "UNRUN: actual legacy singleton/bulk Move logical words/default/view, checked canonical bounds, zero redo/overflow atomicity/native continuation; no producer Move oracle"]
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
        let mut next = save_reopen(&mut s, "LegacyMove.pcraft", &q);
        assert_eq!(native(&next), moved);
        let next_top = ids(&next)[1];
        next.execute("layer.translate", json!({"layer":next_top.0,"dx":1,"dy":-1})).unwrap();
        assert_eq!(native(&next)[1].1, original[1].1);
    }
}
#[test]
#[ignore = "UNRUN: legacy14 raw policy -> existing modern explicit wire only, onehistory/undo/noopredo/native continued metadata; no legacy setter grant"]
fn legacy_to_existing_modern_composition_transition() {
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
        assert!(policy(&s, 1).legacy);
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
        let mut next = save_reopen(&mut s, "LegacyTransition.pcraft", &q);
        assert_eq!(native(&next), raw);
        assert_eq!(display(&next), pixels);
        let next_top = ids(&next)[1];
        next.execute("layer.setProps", json!({"layer":next_top.0,"name":"Modern continued"})).unwrap();
        assert_eq!(native(&next), raw);
    }
}
#[test]
#[ignore = "UNRUN: actual legacy finite work/cancel/quota/stale fullhistory+view/channel fences and generic grant closure never publish a prefix"]
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
}
