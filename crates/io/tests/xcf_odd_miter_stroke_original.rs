#![cfg(feature = "corpus")]
//! Own independently reopened odd-miter-source cache, canonical and native pipeline witnesses.
use photocraft_doc::{item_identity::ItemRef, xcf_items::EditableSource};
use photocraft_geom::Rect;
use sha2::{Digest, Sha256};
fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("reviewed local full corpus required"))
        .join("vector-odd-miter-stroke-evidence")
}
fn checked_packet() -> std::path::PathBuf {
    let root = root();
    let bytes = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(sha(&bytes), "c80b51eb9ed7fd2fbd13ef66706d5cb7ee0fa47c3e669a0ce80cf8a57eb5ab5a");
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let files = manifest.as_object().unwrap();
    assert_eq!(files.len(), 130);
    for (path, hash) in files {
        let bytes = std::fs::read(root.join(path)).unwrap();
        assert_eq!(sha(&bytes), hash.as_str().unwrap());
    }
    root
}
#[test]
#[ignore = "requires own unpublished odd-width noAA source/edit witnesses"]
fn noaa_source_edit_import_cache_and_native_match_stock() {
    check_family("noaa");
}
#[test]
#[ignore = "requires own reversed negative-origin odd-width witnesses"]
fn negative_source_edit_import_cache_and_native_match_stock() {
    check_family("noaa-negative");
}
#[test]
#[ignore = "requires own AA widths1/7 source/edit witnesses"]
fn aa_source_edit_import_cache_and_native_match_stock() {
    check_family("aa");
}
fn check_family(family: &str) {
    let root = checked_packet();
    let base = root.join(family);
    for edit in [false, true] {
        let phase = if edit { "edit" } else { "source" };
        let bytes = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        let native = std::fs::read(base.join(format!("{phase}-reopened.layer.native.bin"))).unwrap();
        let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
        assert_eq!(native, std::fs::read(base.join(format!("round2/{phase}-reopened.layer.native.bin"))).unwrap());
        assert_eq!(expected, std::fs::read(base.join(format!("round2/{phase}-reopened.projection.native.bin"))).unwrap());
        let rect = match (family, edit) {
            ("noaa", false) => Rect::from_xywh(6, 8, 43, 31),
            ("noaa", true) => Rect::from_xywh(7, 6, 45, 33),
            ("noaa-negative", false) => Rect::from_xywh(-13, -11, 43, 31),
            ("noaa-negative", true) => Rect::from_xywh(-12, -13, 45, 33),
            ("aa", false) => Rect::from_xywh(9, 11, 37, 25),
            ("aa", true) => Rect::from_xywh(8, 7, 43, 31),
            _ => panic!("unknown own family"),
        };
        let imported = photocraft_io::import("round.xcf", &bytes).unwrap();
        assert!(imported.origin.is_read_only());
        let doc = imported.document;
        doc.validate_xcf_state().unwrap();
        let before = doc.clone();
        let state = doc.xcf_state.as_ref().unwrap();
        let source = &state.source_items[0];
        let EditableSource::Vector { path, source_modified: false, configuration } = &source.source else { panic!("missing canonical editable round source") };
        assert_eq!(source.owner, ItemRef::Layer(doc.layers[0].id));
        assert_eq!(*path, doc.paths[0].id);
        assert_ne!(path.0, 0);
        let photocraft_doc::xcf_filters::FilterValue::Config(stroke) = &configuration.iter().find(|a| a.name == "stroke").unwrap().value else { panic!() };
        assert_eq!(
            stroke.iter().find(|a| a.name == "join-style").unwrap().value,
            photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Unsigned { width: 32, value: 0 })
        );
        let cache = source.cache.as_ref().unwrap();
        cache.validate().unwrap();
        assert_eq!(cache.bytes.as_slice(), native);
        assert_eq!([cache.width, cache.height], [rect.width() as u32, rect.height() as u32]);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(rect).unwrap(), native);
        assert!(state.resources.iter().all(|r| r.owned_bytes.is_none()));
        let rendered = photocraft_compose::gimp_render::try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
        let display: Vec<u8> = rendered.px.iter().flat_map(|p| p.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8)).collect();
        assert_eq!(display, expected);
        assert_eq!(doc, before, "regeneration must not mutate original cache/raster/source evidence");
        let encoded = photocraft_io::export(&doc, "round.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("round.pcraft", &encoded.bytes).unwrap().document;
        assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "round.pcraft"));
        let fresh = photocraft_compose::gimp_render::try_render(&reopened, reopened.bounds(), &mut |_| Ok(())).unwrap();
        let display: Vec<u8> = fresh.px.iter().flat_map(|p| p.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8)).collect();
        assert_eq!(display, expected);
    }
}
