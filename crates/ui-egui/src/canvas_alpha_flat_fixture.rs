//! Alpha57..60 RGB/Gray source/edit lifecycle fixtures. Missing eight Gray
//! producer effective ICC captures stay a byte-evidence HOLD. Generated CURRENT
//! SGray is never asserted equal to those absent bytes (and is not Gamma22).
//! File/native/model/reference vectors below are external harness captures.
use photocraft_doc::{
    Document, LayerContent, Rect,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits},
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
pub(super) fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
pub(super) struct Case {
    pub root: PathBuf,
    pub file: String,
    pub mode: i32,
    pub gray: bool,
    pub state: Value,
    pub png: String,
}
fn checked(root: &Path, blob: &Value) -> Vec<u8> {
    let b = std::fs::read(root.join(blob["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&b), blob["sha256"].as_str().unwrap());
    assert_eq!(b.len() as u64, blob["bytes"].as_u64().unwrap());
    b
}
fn bindings(root: &Path, value: &Value, missing: &mut usize) {
    match value {
        Value::Object(map) => {
            if let (Some(file), Some(_), Some(_)) = (map.get("file").and_then(Value::as_str), map.get("sha256"), map.get("bytes")) {
                if file.ends_with(".effective.icc") {
                    assert!(!root.join(file).exists(), "review evidence before claiming newly available producer ICC");
                    assert_eq!(value["sha256"], "4e1c5f686d5639e33a9d3a31d564a45c8fca2a5eb1e00314bd3b0098c60fe8f2");
                    *missing += 1;
                } else {
                    checked(root, value);
                }
            } else {
                for v in map.values() {
                    bindings(root, v, missing)
                }
            }
        }
        Value::Array(rows) => {
            for v in rows {
                bindings(root, v, missing)
            }
        }
        _ => {}
    }
}
pub(super) fn cases() -> Vec<Case> {
    let full = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let root = full.join("composition-matrix");
    let mut cases = Vec::new();
    let mut pins = serde_json::Map::new();
    for raw in
        [include_bytes!("../../compose/tests/gimp_color_erase_hashes.json").as_slice(), include_bytes!("../../compose/tests/gimp_alpha_hashes.json").as_slice()]
    {
        let v: Value = serde_json::from_slice(raw).unwrap();
        for (file, sha) in v.as_object().unwrap() {
            assert_eq!(hash(&std::fs::read(root.join(file)).unwrap()), sha.as_str().unwrap());
            pins.insert(file.clone(), sha.clone());
        }
    }
    for (mode, name) in [(57, "color-erase"), (58, "erase"), (59, "merge"), (60, "split")] {
        for edit in [false, true] {
            let stem = format!("mode-{name}");
            let file = format!("{stem}{}.xcf", if edit { ".edit" } else { "" });
            let phase = if edit { "edit" } else { "source" };
            let state_file = format!("{stem}.fresh-{phase}_state.json");
            let png = format!("{stem}.fresh-{phase}.ref.png");
            assert!(pins.contains_key(&file));
            let state: Value = serde_json::from_slice(&std::fs::read(root.join(state_file)).unwrap()).unwrap();
            assert!(state["assigned_icc_sha256"].is_null());
            for row in state["layers"].as_array().unwrap() {
                assert_eq!(row["native"]["byte_order"], "little");
                checked(&root, &row["native"]);
            }
            cases.push(Case { root: root.clone(), file, mode, gray: false, state, png });
        }
    }
    let root = full.join("alpha-gray-evidence");
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    let pin: Value = serde_json::from_slice(include_bytes!("../../compose/tests/gimp_alpha_gray_hashes.json")).unwrap();
    assert_eq!(hash(&bytes), pin["observations.json"].as_str().unwrap());
    let obs: Value = serde_json::from_slice(&bytes).unwrap();
    let mut missing = 0;
    for row in obs["records"].as_array().unwrap() {
        let file = row["source_file"].as_str().unwrap().to_owned();
        assert_eq!(hash(&std::fs::read(root.join(&file)).unwrap()), row["source_sha256"].as_str().unwrap());
        bindings(&root, row, &mut missing);
        assert!(row["assigned_icc"].is_null());
        assert_eq!(row["canvas"], json!([16, 12]));
        let mode = row["layers"][0]["mode"].as_i64().unwrap() as i32;
        assert!((57..=60).contains(&mode));
        for layer in row["layers"].as_array().unwrap() {
            assert_eq!(layer["native_format"], "Y'A float");
            assert_eq!(layer["byte_order"], "little");
        }
        cases.push(Case { root: root.clone(), file, mode, gray: true, state: row.clone(), png: row["reference"]["file"].as_str().unwrap().to_owned() });
    }
    assert_eq!(missing, 8);
    assert_eq!(cases.len(), 16);
    cases
}
pub(super) fn input(case: &Case) -> Vec<u8> {
    std::fs::read(case.root.join(&case.file)).unwrap()
}
pub(super) fn import(bytes: &[u8]) -> photocraft_io::OwnedXcfImport {
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    photocraft_io::import_owned_xcf_from_reader_in_quota(
        "Original alpha.xcf",
        &mut std::io::Cursor::new(bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q,
        &ctl(),
    )
    .unwrap()
}
fn backed(layer: &photocraft_doc::Layer) -> &photocraft_doc::BackedLayer {
    let LayerContent::BackedRaster(b) = &layer.content else { panic!("native leaf") };
    b
}
pub(super) fn native(layer: &photocraft_doc::Layer) -> Vec<u8> {
    let b = backed(layer);
    let r = b.operative_bounds();
    let stride = b.format().bytes_per_pixel();
    let mut out = vec![0; r.width() as usize * r.height() as usize * stride];
    for coord in r.tiles() {
        let pin = b.checked_view().pin_tile(coord, &ctl()).unwrap();
        let part = coord.rect().intersect(&r);
        for y in part.y0..part.y1 {
            for x in part.x0..part.x1 {
                let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * stride;
                pin.read_pixel_native_into(x, y, &mut out[at..at + stride], &ctl()).unwrap();
            }
        }
    }
    out
}
pub(super) fn observed(case: &Case, doc: &Document) {
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(doc.size.width, 16);
    assert_eq!(doc.size.height, 12);
    assert_eq!(doc.mode, if case.gray { photocraft_doc::ColorMode::Grayscale } else { photocraft_doc::ColorMode::Rgb });
    assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
    let profile = if case.gray {
        assert!(matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSGray { .. }));
        photocraft_cms::Builtin::SGray.profile()
    } else {
        assert!(matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { .. }));
        photocraft_cms::Builtin::Srgb.profile()
    };
    assert_eq!(doc.icc_profile.as_deref().unwrap().as_slice(), profile.encoded_bytes().unwrap());
    for (index, (layer, row)) in doc.layers.iter().zip(case.state["layers"].as_array().unwrap().iter().rev()).enumerate() {
        assert_eq!(layer.name, row["name"].as_str().unwrap());
        assert!(layer.visible);
        assert!(layer.mask.is_none());
        let b = backed(layer);
        assert_eq!(b.format().bytes_per_pixel(), if case.gray { 8 } else { 16 });
        let r = b.operative_bounds();
        assert_eq!([r.x0, r.y0], [row["offset"][0].as_i64().unwrap() as i32, row["offset"][1].as_i64().unwrap() as i32]);
        assert_eq!([r.width(), r.height()], [row["size"][0].as_u64().unwrap() as u32, row["size"][1].as_u64().unwrap() as u32]);
        let item = doc.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).unwrap();
        assert_eq!(item.raster_bounds, Some(r));
        let p = item.composition.as_ref().unwrap();
        assert!(!p.legacy);
        let resolved = p.checked_resolved().unwrap();
        let (raw, blend, composite, operator, epoch) = if index == 0 {
            ([0, -1, -1], GimpSpace::LinearRgb, GimpSpace::LinearRgb, GimpOperator::Union, "xcf-12-normal")
        } else {
            match case.mode {
                57 => ([-1, -1, -2], GimpSpace::LinearRgb, GimpSpace::LinearRgb, GimpOperator::ClipToBackdrop, "xcf-12-authored-blend-auto"),
                58 => ([0, -1, -2], GimpSpace::ProfileRgb, GimpSpace::LinearRgb, GimpOperator::ClipToBackdrop, "xcf-12-authored-blend-auto"),
                59 => ([0, -1, -1], GimpSpace::ProfileRgb, GimpSpace::LinearRgb, GimpOperator::Union, "xcf-12-authored-blend-auto"),
                60 => ([0, 0, -2], GimpSpace::ProfileRgb, GimpSpace::ProfileRgb, GimpOperator::ClipToBackdrop, "xcf-12-authored-blend-auto"),
                _ => panic!("closedcase"),
            }
        };
        assert_eq!(p.source_mode, if index == 0 { 28 } else { case.mode });
        assert_eq!([p.stored_blend_space, p.stored_composite_space, p.stored_operator], raw);
        assert_eq!((resolved.blend_space, resolved.composite_space, resolved.operator), (blend, composite, operator));
        assert_eq!(p.compatibility_epoch, epoch);
        let mut expected = checked(&case.root, &row["native"]);
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
    for (a, b) in before.layers.iter().zip(&after.layers) {
        assert_eq!(a.id, b.id);
        assert_eq!(backed(a).operative_bounds(), backed(b).operative_bounds());
        assert_eq!(backed(a).format(), backed(b).format());
        assert_eq!(backed(a).default_bytes(), backed(b).default_bytes());
        assert_eq!(native(a), native(b));
    }
    assert_eq!(before.icc_profile, after.icc_profile);
    assert_eq!(before.sample_encoding, after.sample_encoding);
    assert_eq!(before.source_color_context, after.source_color_context);
    assert_eq!(before.working_profile_interpretation, after.working_profile_interpretation);
    assert_eq!(before.xcf_state.as_ref().unwrap().opaque, after.xcf_state.as_ref().unwrap().opaque);
}
pub(super) fn moved(before: &Document, after: &Document, id: photocraft_doc::LayerId, dx: i32, dy: i32) {
    let mut state = before.xcf_state.clone().unwrap();
    for old in &before.layers {
        let now = after.layer(old.id).unwrap();
        let (x, y) = if old.id == id { (dx, dy) } else { (0, 0) };
        let r = backed(old).operative_bounds();
        let shifted = Rect::new(r.x0 + x, r.y0 + y, r.x1 + x, r.y1 + y);
        assert_eq!(backed(now).operative_bounds(), shifted);
        assert_eq!(native(old), native(now));
        assert_eq!(backed(old).format(), backed(now).format());
        assert_eq!(backed(old).default_bytes(), backed(now).default_bytes());
        state.items.iter_mut().find(|i| i.item == ItemRef::Layer(old.id)).unwrap().raster_bounds = Some(shifted);
    }
    assert_eq!(Some(state), after.xcf_state);
    assert_eq!(before.icc_profile, after.icc_profile);
    assert_eq!(before.source_color_context, after.source_color_context);
}
pub(super) fn archive(owner: &photocraft_doc::accounted_document::AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let q = owner.quota();
    let bytes =
        photocraft_format::owned_document::save_current_document_archive(owner.document(), std::io::Cursor::new(Vec::new()), Default::default(), q, &ctl())
            .unwrap()
            .into_inner();
    photocraft_io::accounted_native::open_accounted_native_from_reader(
        &mut std::io::Cursor::new(&bytes),
        bytes.len() as u64,
        Default::default(),
        q.clone(),
        &ctl(),
    )
    .unwrap()
}
pub(super) fn transition(doc: &Document, revision: u64) -> Value {
    json!({"layerId":doc.layers[1].id.0.to_string(),"documentId":doc.id.0.to_string(),"expectedRevision":revision.to_string(),"sourceMode":31,"legacy":false,"blendSpace":"linear-rgb","compositeSpace":"linear-rgb","operator":"union"})
}
pub(super) fn reference(case: &Case, doc: &Document) -> photocraft_compose::Buffer {
    use photocraft_compose::{gimp_blend, gimp_composite::WorkingSpace};
    let inputs: Vec<_> = case.state["layers"].as_array().unwrap().iter().rev().map(|row| checked(&case.root, &row["native"])).collect();
    let profile = if case.gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
    let mut px = Vec::new();
    for y in 0..12 {
        for x in 0..16 {
            let mut bg = [0.; 4];
            for (index, layer) in doc.layers.iter().enumerate() {
                if !layer.visible {
                    continue;
                }
                let b = backed(layer);
                let r = b.operative_bounds();
                let inside = r.contains(x, y);
                let pixel = if inside {
                    let n = b.format().bytes_per_pixel();
                    let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * n;
                    let sample = |c| f64::from(f32::from_le_bytes(inputs[index][at + c * 4..at + c * 4 + 4].try_into().unwrap()));
                    if case.gray {
                        let v = sample(0);
                        [v, v, v, sample(1)]
                    } else {
                        std::array::from_fn(sample)
                    }
                } else {
                    [0.; 4]
                };
                let p = doc.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).unwrap().composition.as_ref().unwrap();
                let resolved = p.checked_resolved().unwrap();
                let space = WorkingSpace::new(profile, doc.sample_encoding.unwrap(), resolved.composite_space).unwrap();
                let blend = WorkingSpace::new(profile, doc.sample_encoding.unwrap(), resolved.blend_space).unwrap();
                let opacity = if inside { f64::from(layer.opacity) } else { 0. };
                bg = if p.source_mode == 28 {
                    space.normal(bg, pixel, opacity, resolved.operator).unwrap()
                } else {
                    space.separable(&blend, bg, pixel, opacity, resolved.operator, gimp_blend::mode(p.source_mode).unwrap()).unwrap()
                };
            }
            px.push(bg.map(|v| v as f32));
        }
    }
    photocraft_compose::Buffer { rect: doc.bounds(), px }
}
pub(super) fn page(value: &Value, doc: &Document, offset: usize, limit: usize) {
    let state = doc.xcf_state.as_ref().unwrap();
    assert_eq!(value["items"]["total"], state.items.len());
    assert_eq!(value["filters"]["total"], 0);
    assert_eq!(value["filterStacks"]["total"], 0);
    let rows = value["items"]["entries"].as_array().unwrap();
    assert_eq!(rows.len(), state.items.len().saturating_sub(offset).min(limit));
    for (row, item) in rows.iter().zip(state.items.iter().skip(offset).take(limit)) {
        assert_eq!(row["item"], json!({"kind":"layer","id":item.item.raw().to_string()}));
        let p = item.composition.as_ref().unwrap();
        assert_eq!(row["composition"]["sourceMode"], p.source_mode);
        assert_eq!(row["composition"]["storedBlendSpace"], p.stored_blend_space);
        assert_eq!(row["composition"]["storedCompositeSpace"], p.stored_composite_space);
        assert_eq!(row["composition"]["storedOperator"], p.stored_operator);
    }
}
pub(super) fn same_quota(owner: &photocraft_doc::accounted_document::AccountedDocument) {
    owner.validate_readonly_flat_composite_storage(&ctl()).unwrap();
    for layer in &owner.document().layers {
        assert!(owner.quota().shares_ledger(backed(layer).quota()));
    }
}
