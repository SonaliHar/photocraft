#![cfg(feature = "corpus")]
use photocraft_doc::{item_identity::ItemRef, xcf_items::EditableSource};
use sha2::{Digest, Sha256};
fn sha(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|b| format!("{b:02x}")).collect()
}
fn packet() -> std::path::PathBuf {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("local independent corpus required"))
        .join("vector-monotone-arch-evidence");
    let b = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(sha(&b), "17d6b424d3598de363941d2928d269333fbbb02e6f63a8985666a82fe92a5875");
    let m: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(m.as_object().unwrap().len(), 191);
    for (p, h) in m.as_object().unwrap() {
        assert_eq!(sha(&std::fs::read(root.join(p)).unwrap()), h.as_str().unwrap());
    }
    root
}
fn projection(doc: &photocraft_doc::Document) -> Vec<u8> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(()))
        .unwrap()
        .px
        .iter()
        .flat_map(|p| p.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8))
        .collect()
}
#[test]
#[ignore = "requires own independent monotone cubic stock source/edit witnesses"]
fn all_eight_arch_sources_import_preserve_cache_and_native_exactly() {
    let root = packet();
    for name in ["basic", "asymmetric-fractional", "wide", "reversed"] {
        for phase in ["source", "edit"] {
            let d = root.join(name);
            let bytes = std::fs::read(d.join(format!("{phase}.xcf"))).unwrap();
            let parsed = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert_eq!((parsed.version, parsed.precision), (24, 150));
            let source = parsed.layers.iter().find(|l| l.vector.is_some()).unwrap();
            let bounds = photocraft_geom::Rect::from_xywh(source.offset.0, source.offset.1, source.width, source.height);
            let native = std::fs::read(d.join(format!("{phase}-reopened.layer.native.bin"))).unwrap();
            assert_eq!(native, source.plane.as_ref().unwrap().bytes);
            assert_eq!(native, std::fs::read(d.join(format!("round2/{phase}-reopened.layer.native.bin"))).unwrap());
            let expected = std::fs::read(d.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
            let imported = photocraft_io::import("arch.xcf", &bytes).unwrap();
            assert!(imported.origin.is_read_only());
            let doc = imported.document;
            doc.validate_xcf_state().unwrap();
            let before = doc.clone();
            let state = doc.xcf_state.as_ref().unwrap();
            let item = &state.source_items[0];
            let EditableSource::Vector { path, source_modified: false, .. } = &item.source else { panic!("missing editable source") };
            assert_eq!(*path, doc.paths[0].id);
            assert_eq!(item.owner, ItemRef::Layer(doc.layers[0].id));
            assert_eq!(doc.paths[0].path.subpaths[0].knots.len(), 2);
            assert_eq!(item.cache.as_ref().unwrap().bytes.as_slice(), native);
            item.cache.as_ref().unwrap().validate().unwrap();
            assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap(), native);
            assert!(state.resources.iter().all(|r| r.owned_bytes.is_none()));
            assert_eq!(projection(&doc), expected, "{name}/{phase}");
            assert_eq!(doc, before);
            let encoded = photocraft_io::export(&doc, "arch.pcraft", &Default::default()).unwrap();
            let reopened = photocraft_io::import("arch.pcraft", &encoded.bytes).unwrap().document;
            assert_eq!(reopened, *photocraft_io::named_for_file(&doc, "arch.pcraft"));
            assert_eq!(projection(&reopened), expected);
        }
    }
}
