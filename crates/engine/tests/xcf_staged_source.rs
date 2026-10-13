//! Actual staged-source documents preserve native/source state through Session edits.
#![cfg(feature = "corpus")]
use photocraft_engine::Session;
use photocraft_raster::Interrupt;
use photocraft_xcf::{Budget, DecodeOptions, PlainSourceAccounting, ResourceLimits, ResourcePolicy, Source, XcfError};
use serde_json::json;
struct Bytes(Vec<u8>);
impl Source for Bytes {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8], _: &Budget<'_>) -> Result<(), XcfError> {
        let start = offset as usize;
        out.copy_from_slice(&self.0[start..start + out.len()]);
        Ok(())
    }
}
#[test]
fn staged_original_rgb_gray_current_edit_native_reopen_and_undo_are_exact() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf");
    for name in ["rgb-u8_non_linear.xcf", "gray-u8_non_linear.xcf"] {
        let bytes = std::fs::read(root.join(name)).expect("run cargo xtask corpus --xcf");
        let encoded_hash = blake3::hash(&bytes);
        let mut source = Bytes(bytes);
        let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
        let imported = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
            name,
            &mut source,
            &Interrupt::NONE,
            &DecodeOptions::default(),
            &budget,
            PlainSourceAccounting::BorrowedMemory,
        )
        .unwrap();
        let mut session = Session::new();
        session.open_document(imported.document, None).unwrap();
        let id = session.active().unwrap().doc.layers[0].id;
        session.active_mut().unwrap().active_layer = Some(id);
        let before = session.active().unwrap().doc.clone();
        let source_state = before.xcf_state.clone();
        let source_color = before.source_color_context.clone();
        session.execute("image.adjustments.invert", json!({})).unwrap();
        let current = &session.active().unwrap().doc;
        assert_ne!(current.layers[0].surface(), before.layers[0].surface());
        assert_eq!(current.xcf_state, source_state);
        assert_eq!(current.source_color_context, source_color);
        assert_eq!(blake3::hash(&source.0), encoded_hash);
        let native = photocraft_format::save_to_bytes(current, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **current);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, before);
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *before);
    }
}
