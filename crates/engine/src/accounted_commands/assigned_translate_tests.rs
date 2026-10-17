//! UNRUN linked CURRENT controls derived from genuine assigned source packets.
//! Setting link flags is a prepaid CURRENT-only fixture, not GIMP linkage or
//! translation evidence. Native/source captures and raw references are external
//! harness data; production owners retain their actual ledger receipts.
use super::*;
use crate::{DocumentSnapshot, accounted_commands::PreparedAccountedCommand};
use photocraft_doc::{LayerContent, accounted_document::AccountedDocument, item_identity::ItemRef};
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaLimits},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
const CASES: [(&str, &str, usize); 64] = [
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op1.xcf", "fd97c954dbfee8fa69695a69ce1e400499f2854a4f5af2acf75bb4a99d708b9b", 9339),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op2.xcf", "ab0f494fef20f85d0d1e93b81790dba410c914be2d75e5860667b973ae75ef1e", 9339),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op3.xcf", "6280cb35f6495f2d6113cfaf6a1befb28042ad9734d03b3a38727261ee62559d", 9339),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op4.xcf", "43bcfecb913670e4aa3d738daa71cc3a69f1b39ca80c61a1bbb9adcf0802a81d", 9339),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op1.xcf", "d09764e320785b11fe20ad38ae29a0a228b8422891e9dd2e6c114762297a9788", 13710),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op2.xcf", "a40f9837d56038faed84e09cf5b5127f31843154ecb15ca8762901932386ca02", 13710),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op3.xcf", "d8358732abedfbc7e9a5261349a60f3446aac514d3cab4533f02e75f6c5f5b31", 13710),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op4.xcf", "605ebdf92089ef77927d3a39581db3d84b741ad9df57fd7b7a930f350c93f6cb", 13710),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op1.xcf", "b356d0639fbc7dff8a81fd8a8444356586088233cc8f423f7cc516a205af2461", 5247),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op2.xcf", "c88f868af205e468030cd8ecc4d479f0ecbf36a50cafbe3a43d80b5d01b13642", 5247),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op3.xcf", "c54efc051531d8372f56444cb6408eacda687232f1e8519899e21f0a5e590d3c", 5247),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op4.xcf", "64163f76c08404904803fed411f44f424efe7f91c6888529a28413a115695e31", 5247),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op1.xcf", "e4f455f58216921cffc088d0116b21040fae4bc232a20e073d0ac94aa5c5d81c", 6690),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op2.xcf", "dcf0b6ddbe2c3d9a899a844224a85b3a5411864cd1ec730ae2bac069a624276f", 6690),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op3.xcf", "c85f4a32e60d4fa50eb012e3a1f625dce6c9ed5f452c132e06a21e7b8ede29ca", 6690),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op4.xcf", "cb9dad1b89275d258d1789bfe3ebb45a817a647bd0df50b01be32517ec272a1f", 6690),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op1.xcf", "4250f944696df427e92e3e4568aede8f7ec49ff8d9fbf26c429cc2210282d5bf", 6409),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op2.xcf", "298f7b56c94b3a9acdd4634f8eacc1643b5c361403947b11adb80fa6fea541dd", 6409),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op3.xcf", "6f99f6247e017b7fd260d6b815695476e44657e30f9294798b2f54f54222d045", 6409),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op4.xcf", "6cc984c03caefe3697ca0765787259b2943fbc10cc6a300620bbfd8cdd8f3787", 6409),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op1.xcf", "b7f941d9e7956fd4cbde707da299d92b9c5ad76131613e9e8589ead1a10894b7", 10477),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op2.xcf", "7852a3a531c05ba6085f713fee3d2bde70344563dc67ce0962857c4ee95e2d1a", 10477),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op3.xcf", "32a893d94ff713708cb9ca0182e9d60f018b9b00e4963acc7f8fc9bd8ed95847", 10477),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op4.xcf", "b41aa7e93a35ad401ee23f58237e181db262adbf7af1e9e4bedeee73bb7f226e", 10477),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op1.xcf", "b39d43a1d3a4de272d655b526c104d90f67bbce6764f5865fe78bec5ab4e31e6", 4261),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op2.xcf", "f906a7defebfce50480c523c2d544535f73b3a2310f12eb26c33364280aeaed3", 4261),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op3.xcf", "87b1fb3edcf289fc3452d838a1ff3bb79709e6728babb3de9d48248343e1fdaf", 4261),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op4.xcf", "2715b93c6b22d8314c47978fa98bfb346efec3a7198d39629dd9b713423a3422", 4261),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op1.xcf", "0853c2ff39fc77dcb7aa88a2ab5411789fa08bc202141a9df6c8243e39ed6c4c", 6385),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op2.xcf", "551fcac24063d1ce685404ce00ddade0dad2814c7d511d551a59b76235936f9b", 6385),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op3.xcf", "2598fe1aaad8a130c40040254d44708128ec17be0cd1ea101cb0bd578029c102", 6385),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op4.xcf", "4e3f10f96df65ec064d0e2139838387312b0b328b62ba35617ffb9d48abbc047", 6385),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op1.xcf", "ff29148ffec4a0c845441d44d26da3aa666e277a77731faf16285cfc69923974", 9339),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op2.xcf", "0d96b8736f3d71b547f062a3aaf6852fbbe035f182b3b0af745f413abba6ba47", 9339),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op3.xcf", "ed64ad0823b55057b91db1df92b7f2949fbe21ebf06db219c407f6efad6ddf9c", 9339),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op4.xcf", "8132f936a8d8cf2051f9904e7f5bc99e5aaad75de2114baa23d61db5dae84d70", 9339),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op1.xcf", "45a731c7dec5f4004ce4ce95cc08a5d7158643f5f92df45fbb7f8a22565277a8", 13710),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op2.xcf", "0f58ccb432193ad8d8b2e598a85edcd0d786bdeec38ae2ef4fc55b085bee25d3", 13710),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op3.xcf", "ea80f910a08de501fb969803d8fdc348bd73d971e62969e7bdc2dc0dbaae2477", 13710),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op4.xcf", "707c317381ab0fc1b390beb5f1e377e0a3c2d61bf0f6ec99eec1f98cef01f02f", 13710),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op1.xcf", "2e67b171d0b5d46ca601721611af65287fd897d82c19b74d09038cd647baebba", 5247),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op2.xcf", "3da1d9cad5307c60059779186028a77626b8194a8e139c601a86d06448c3053a", 5247),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op3.xcf", "5fa48ea2f673977f960d8ae5f43f77beeeab821c6648929d1e469695da765f34", 5247),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op4.xcf", "3f84c611ed7cf7e2e4556c047c2438d104330f38f4e3cafe36eeb9a45508b8fb", 5247),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op1.xcf", "fc144090592f149c139cc6f980acdd03664746a4e17568ad64f814a4a0299468", 6690),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op2.xcf", "dee52b5be4b63f26a100af6413cf970836a100e55866092158b7d8dbdb72c361", 6690),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op3.xcf", "cdbdd0080b0451399d64cfa80427ddd3ab4518fb6f38ff27f8e5940d7f52661d", 6690),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op4.xcf", "8e43e6815c87bac3374b2aeaf1905d6da8a242d04878e5341d68587800d27fc5", 6690),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op1.xcf", "3451e9edfcf702864dc8529a48bbd5ec5ac12c5472998d79a1537cdeeb381790", 9339),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op2.xcf", "69e7957c4b6e2b7351c9605519cae1d2dfc0f4c00c8456d850d7a3db7cf942b0", 9339),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op3.xcf", "0e78ed8b0bd8a4803c18f8d2ec2c775711a523aabd53c458d40f873a4eca23d2", 9339),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op4.xcf", "0a825021886e1d2f7b064e01bbd20c0581ff522178e9ce7744fce66ee33c44db", 9339),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op1.xcf", "a811e5a2f8f3e46251479b0ae5ea855fa84b6b7d14c9c47da6ed1e1969ba75ec", 13710),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op2.xcf", "2d886b9e924c57f0ee05c1ab35e6cceac13eae0f90f6017f47149d34b353aad0", 13710),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op3.xcf", "d3ded016dc359693fce24312db211df66bcdac6442998349205d578e2bbcaa29", 13710),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op4.xcf", "ffa5d4e0152466684f3f6dec172b4005c2c484a7a2a0a45956f88377602f7ef7", 13710),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op1.xcf", "826f043bb684df4b353d28ad9d19ddb488c8984b4569099b856ddf2c7cfc4251", 5247),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op2.xcf", "2cd715306242b1be2701057706d1426b9ff2a3425495913f9b92b57cf70a81ef", 5247),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op3.xcf", "be32290b027725c05f76ad4a654e25c5bc75fac66e82efe92a2b4bb168be1809", 5247),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op4.xcf", "bd6c03a429ccedb8c4afcad4c12563184eb1ab776c58a37028f9c7d2c691dcd9", 5247),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op1.xcf", "d6882a35bf13dca9366e57370afa502ee271eaf7ff3adb24c9c53bf163c6963e", 6690),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op2.xcf", "bac3fdca4ca0bd427fdf39ed3549788c0b74e3c70298b460c981fe120736e05c", 6690),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op3.xcf", "106040db4a6de949c0116cd625bd706734d118721184a4bea8d38e94c2d44eb1", 6690),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op4.xcf", "885b21f43b3461074eeb34f77e8509ec0a415a95f6e98fff6e77677317b71a00", 6690),
];
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn linked(case: usize, enabled: bool) -> Session {
    fixture(case, enabled, true)
}
fn fixture(case: usize, enabled: bool, is_linked: bool) -> Session {
    let (path, pin, len) = CASES[case];
    let bytes = std::fs::read(PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(path)).unwrap();
    assert_eq!(bytes.len(), len);
    assert_eq!(hash(&bytes), pin);
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let result = photocraft_io::import_owned_xcf_from_reader_in_quota(
        path,
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::AnalyticGeometry(imported) = result else { panic!("actual assigned source") };
    let source = imported.document().clone();
    let fee = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let mut doc = source.document().clone();
    let fg = doc.layers[1].id;
    let mask = doc.layers[1].mask.as_mut().unwrap();
    mask.linked = is_linked;
    mask.enabled = enabled;
    let metadata = doc.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|item| item.mask.as_mut().filter(|m| m.owner == ItemRef::Layer(fg))).unwrap();
    metadata.linked = is_linked;
    metadata.enabled = enabled;
    let owner = AccountedDocument::from_owned(doc, Arc::new((source, fee)), q.clone(), &ctl()).unwrap();
    let snapshot = DocumentSnapshot::from_accounted(owner).unwrap();
    let archive = save(&snapshot, &q);
    let mut out = Session::new();
    out.add_owned_native_analytic_geometry_archive(archive, "linked-current.pcraft", Some("actual/linked-current.pcraft"), &ctl()).unwrap();
    out
}
fn save(doc: &DocumentSnapshot, q: &BackingQuota) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let bytes =
        photocraft_format::owned_document::save_current_document_archive(doc, Cursor::new(Vec::new()), Default::default(), q, &ctl()).unwrap().into_inner();
    photocraft_io::accounted_native::open_accounted_native_from_reader(&mut Cursor::new(&bytes), bytes.len() as u64, Default::default(), q.clone(), &ctl())
        .unwrap()
}
fn foreground(s: &Session) -> LayerId {
    s.active().unwrap().doc.layers[1].id
}
fn raw(s: &Session) -> (photocraft_geom::Rect, Vec<u8>) {
    let LayerContent::BackedRaster(b) = &s.active().unwrap().doc.layers[1].content else { panic!() };
    let area = b.operative_bounds();
    let n = b.format().bytes_per_pixel();
    let mut out = vec![0; area.width() as usize * area.height() as usize * n];
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            let i = ((y - area.y0) as usize * area.width() as usize + (x - area.x0) as usize) * n;
            b.checked_view().read_logical_pixel_native_into(x, y, &mut out[i..i + n], &ctl()).unwrap();
        }
    }
    (area, out)
}
fn mask_raw(s: &Session) -> (Vec<u8>, Vec<(photocraft_geom::TileCoord, Vec<u8>)>) {
    let mask = &s.active().unwrap().doc.layers[1].mask.as_ref().unwrap().surface;
    (mask.default_bytes().to_vec(), mask.tiles().map(|(c, t)| (*c, t.bytes().to_vec())).collect())
}
fn mask_logical(s: &Session) -> Vec<u8> {
    let doc = &s.active().unwrap().doc;
    let extent = doc.xcf_state.as_ref().unwrap().items.iter().find_map(|i| i.mask.as_ref().map(|m| m.bounds)).unwrap();
    doc.layers[1].mask.as_ref().unwrap().surface.try_read_interleaved_with(extent, &Interrupt::NONE).unwrap()
}
fn assert_original_mask_values(s: &Session, original: &(Vec<u8>, Vec<(photocraft_geom::TileCoord, Vec<u8>)>)) {
    let mask = &s.active().unwrap().doc.layers[1].mask.as_ref().unwrap().surface;
    assert_eq!(mask.default_bytes(), original.0);
    for (coordinate, bytes) in &original.1 {
        assert_eq!(mask.try_read_interleaved_with(coordinate.rect(), &Interrupt::NONE).unwrap(), *bytes);
    }
    // Native serialization may retain additional all-default padding tiles.
    // Check their values rather than treating a map layout as source evidence.
    for (coordinate, tile) in mask.tiles() {
        if !original.1.iter().any(|(old, _)| old == coordinate) {
            assert!(tile.bytes().chunks_exact(original.0.len()).all(|pixel| pixel == original.0));
        }
    }
}
#[test]
#[ignore = "UNRUN:64 genuine original controls, alloperators/encodings/models/depths, linked enabled/disabled move native+mask bounds and inverse raw padding"]
fn assigned_linked_moves_the_foreground_and_its_complete_mask_without_source_changes() {
    for case in 0..CASES.len() {
        for enabled in [true, false] {
            let mut s = linked(case, enabled);
            let before = s.active().unwrap().doc.clone();
            let source = s.active().unwrap().source.clone();
            let (old, words) = raw(&s);
            let original_mask = mask_raw(&s);
            let logical_mask = mask_logical(&s);
            let fg = foreground(&s);
            assert_eq!(s.execute("layer.translate", json!({"layer":fg.0,"target":"mask","dx":3,"dy":-2})).unwrap(), Value::Null);
            assert_eq!(raw(&s), (old.translate(3, -2), words.clone()));
            assert_eq!(mask_logical(&s), logical_mask);
            let state = s.active().unwrap();
            assert_eq!(state.source, source);
            assert_eq!(state.doc.icc_profile, before.icc_profile);
            assert_eq!(state.doc.source_color_context, before.source_color_context);
            let canonical = state.doc.xcf_state.as_ref().unwrap();
            let m = canonical.items.iter().find_map(|i| i.mask.as_ref()).unwrap();
            assert_eq!(m.bounds, old.translate(3, -2));
            assert_eq!(m.offset, [m.bounds.x0, m.bounds.y0]);
            assert_eq!(m.enabled, enabled);
            assert!(m.linked);
            let q = state.doc.quota().unwrap();
            let archive = save(&state.doc, q);
            let mut reopened = Session::new();
            reopened.add_owned_native_analytic_geometry_archive(archive, "Moved.pcraft", Some("actual/Moved.pcraft"), &ctl()).unwrap();
            assert_eq!(raw(&reopened), raw(&s));
            assert_eq!(mask_raw(&reopened), mask_raw(&s));
            reopened.execute("layer.translate", json!({"layer":fg.0,"dx":-3,"dy":2})).unwrap();
            assert_eq!(raw(&reopened), (old, words));
            assert_original_mask_values(&reopened, &original_mask);
            assert!(s.undo_with_control(&ctl()).unwrap());
            let history = s.active().unwrap().history.clone();
            let revision = s.active().unwrap().revision;
            s.execute("layer.translate", json!({"layer":{},"dx":0,"target":"mask"})).unwrap();
            assert!(s.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(s.active().unwrap().revision, revision);
            assert!(s.active().unwrap().history.can_redo());
            assert!(s.redo_with_control(&ctl()).unwrap());
        }
    }
}
#[test]
#[ignore = "UNRUN: mixed assigned metadata/Move key keeps first label, both-target .mask moves whole foreground, no-op redo/native continuation"]
fn assigned_translation_reuses_the_existing_opaque_gesture_anchor() {
    for case in [0, 8, 16, 24, 32, 40, 48, 56] {
        let mut s = linked(case, false);
        let fg = foreground(&s);
        let old = raw(&s);
        s.execute("layer.setProps", json!({"layer":fg.0,"name":"Current linked","coalesce":"\0linked"})).unwrap();
        s.execute("layer.translate", json!({"layer":fg.0,"dx":2,"coalesce":"\0linked"})).unwrap();
        s.execute("layer.setProps", json!({"layer":fg.0,"opacity":0.375,"coalesce":"\0linked"})).unwrap();
        s.execute("layer.translate", json!({"layer":fg.0,"target":"mask","dy":-1,"coalesce":"\0linked"})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Layer Properties"));
        assert_eq!(raw(&s), (old.0.translate(2, -1), old.1));
        assert!(s.undo_with_control(&ctl()).unwrap());
        let history = s.active().unwrap().history.clone();
        s.execute("layer.translate", json!({"layer":999999,"dx":0,"coalesce":"other"})).unwrap();
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert!(s.active().unwrap().history.can_redo());
        assert!(s.redo_with_control(&ctl()).unwrap());
    }
}
#[test]
#[ignore = "UNRUN: exact assigned foreground restriction, unlinked refusal, real history/source/prefs/channel fences and cumulative quota/cancel rollback"]
fn assigned_translation_cannot_publish_a_partial_or_stale_recipe() {
    let mut unlinked = fixture(0, true, false);
    let unchanged = unlinked.active().unwrap().doc.clone();
    let target = foreground(&unlinked);
    for mask_target in [false, true] {
        let params = if mask_target { json!({"layer":target.0,"target":"mask","dx":1}) } else { json!({"layer":target.0,"dx":1}) };
        assert!(unlinked.execute("layer.translate", params).is_err());
        assert!(unlinked.active().unwrap().doc.shares_state(&unchanged));
    }
    for change in 0..6 {
        let mut s = linked(0, true);
        let p = json!({"layer":foreground(&s).0,"dx":1});
        let prepared = PreparedAccountedCommand::prepare(&s, "layer.translate", &p, &ctl()).unwrap();
        match change {
            0 => s.active_mut().unwrap().source.as_mut().unwrap().label = "stale source".into(),
            1 => s.active_mut().unwrap().path = Some("stale.pcraft".into()),
            2 => s.active_mut().unwrap().history.max_states += 1,
            3 => s.prefs.edit(|p| p.performance.history_states = p.performance.history_states.saturating_add(1)),
            4 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
            _ => s.journal.push(("harness".into(), Value::Null)),
        };
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.clone();
        let journal = s.journal.clone();
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.journal, journal);
    }
    let mut s = linked(0, true);
    let fg = foreground(&s);
    let backdrop = s.active().unwrap().doc.layers[0].id;
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    for p in [json!({"layer":backdrop.0,"dx":1}), json!({"layer":fg.0,"dx":1,"target":"quickMask"}), json!({"layer":fg.0,"dx":2147483647,"dy":1})] {
        assert!(s.execute("layer.translate", p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    let spent = AtomicU64::new(0);
    let work = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    super::current(before.accounted().unwrap(), &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap();
    let cost = spent.load(Ordering::Relaxed);
    let spent = AtomicU64::new(0);
    let work = |n| if spent.fetch_add(n, Ordering::Relaxed).saturating_add(n) > cost + cost / 4 { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(s.translate_accounted_imported_layer(fg, 1, 1, None, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert!(spent.load(Ordering::Relaxed) > cost);
    assert!(s.active().unwrap().doc.shares_state(&before));
    let q = before.quota().unwrap();
    let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.execute("layer.translate", json!({"layer":fg.0,"dx":1})).is_err());
    drop(hold);
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal, journal);
}
