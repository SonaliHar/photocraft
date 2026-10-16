//! Deferred generated U32/350 Source continuity; no GIMP/numerical oracle.
use super::*;
#[path = "accounted_plain_integer_hierarchy_test_support.rs"]
mod f;
use std::{
    io::{Seek, Write},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
struct Writer {
    bytes: std::io::Cursor<Vec<u8>>,
    writes: Arc<AtomicU64>,
}
impl Write for Writer {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.bytes.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.bytes.flush()
    }
}
impl Seek for Writer {
    fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
        self.bytes.seek(p)
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: actual generated U32/350 Source portable writer early Work/no output/refund, native13 marker-only handoff and retained-empty anchors across repeated saves; not producer acceptance"]
fn u32_plain_portable_writer_and_empty_anchor_continuation() {
    for depth in [2usize] {
        let source = f::import(depth);
        let owner = source.document();
        f::observed(owner.document(), depth);
        let before = owner.quota().usage().unwrap();
        let work = |_| Err(BackingError::Work);
        let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &work };
        let writes = Arc::new(AtomicU64::new(0));
        let error = photocraft_io::accounted_current_native::save_accounted_current_archive(
            owner,
            Writer { bytes: std::io::Cursor::new(Vec::new()), writes: writes.clone() },
            Default::default(),
            owner.quota(),
            &c,
        )
        .err()
        .unwrap();
        let photocraft_format::FormatError::Io(error) = error else { panic!("typed backing error") };
        assert_eq!(error.get_ref().and_then(|e| e.downcast_ref::<BackingError>()), Some(&BackingError::Work));
        assert_eq!(writes.load(Ordering::Relaxed), 0);
        assert_eq!(owner.quota().usage().unwrap(), before);
        let mut current = f::anchored(owner);
        let original = current.clone();
        for _ in 0..2 {
            let archive = f::archive(&current);
            assert!(is_owned_current_normal_hierarchy_candidate(&archive, &f::ctl()).unwrap());
            assert!(!is_owned_current_filtered_flat_candidate(&archive, &f::ctl()).unwrap());
            let mut wrong = Session::new();
            assert!(wrong.add_owned_native_point_archive(f::archive(&current), "u32.pcraft", None, &f::ctl()).is_err());
            assert!(wrong.add_owned_native_masked_filtered_flat_archive(f::archive(&current), "u32.pcraft", None, &f::ctl()).is_err());
            assert!(
                wrong
                    .add_owned_native_archive(f::archive(&current), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "u32.pcraft", None, &f::ctl())
                    .is_err()
            );
            assert!(wrong.documents().is_empty());
            current = archive.into_accounted_readonly_normal_hierarchy_storage(&f::ctl()).unwrap();
            assert!(current.is_readonly_normal_hierarchy_storage());
            assert!(!current.is_legacy_storage());
            f::same(original.document(), current.document());
            f::observed(current.document(), depth);
            let stacks = &current.document().xcf_state.as_ref().unwrap().filter_stacks;
            assert_eq!(stacks.len(), 1);
            assert_eq!(stacks[0].revision, 7);
            assert!(stacks[0].nodes.is_empty());
        }
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: generated U32/350 hierarchy general renderer basis/actual-class routing, dedicated word continuity and tiny band quantum; internal consistency only"]
fn u32_plain_general_renderer_preserves_basis_and_tiny_quantum() {
    use photocraft_compose::{
        backed_document::PreparedBackedDocument, backed_normal::NormalBandLimits, backed_normal_hierarchy_document::PreparedBackedNormalHierarchyDocument,
    };
    for depth in [2usize] {
        let source = f::import(depth);
        let owner = source.document();
        // Warm actual backing caches before comparing temporary-owner refunds.
        let warm = PreparedBackedNormalHierarchyDocument::new(owner, Default::default(), &f::ctl()).unwrap();
        warm.validate_operative(&f::ctl()).unwrap();
        drop(warm);
        let before = owner.quota().usage().unwrap();
        assert!(PreparedBackedDocument::new_accounted(owner, &photocraft_cms::Builtin::DisplayP3.profile(), Default::default(), &f::ctl()).is_err());
        assert_eq!(owner.quota().usage().unwrap(), before);
        {
            let general = PreparedBackedDocument::new_accounted(owner, &photocraft_cms::Builtin::Srgb.profile(), Default::default(), &f::ctl()).unwrap();
            let direct = PreparedBackedNormalHierarchyDocument::new(owner, Default::default(), &f::ctl()).unwrap();
            let bounds = owner.document().bounds();
            let a = general.render_band(bounds, &f::ctl()).unwrap();
            let b = direct.render_band(bounds, &f::ctl()).unwrap();
            assert_eq!(a.rect(), b.rect());
            assert_eq!(a.pixels().len(), b.pixels().len());
            for (a, b) in a.pixels().iter().zip(b.pixels()) {
                assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
            }
            let tiny = PreparedBackedNormalHierarchyDocument::new(owner, NormalBandLimits { max_rows: 1, max_pixels: 1 }, &f::ctl()).unwrap();
            let x = bounds.x0 + 64;
            let y = bounds.y0 + 64;
            let one = tiny.render_band(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &f::ctl()).unwrap();
            let index = (y - bounds.y0) as usize * bounds.width() as usize + (x - bounds.x0) as usize;
            assert_eq!(one.pixels()[0].map(f64::to_bits), b.pixels()[index].map(f64::to_bits));
        }
        assert_eq!(owner.quota().usage().unwrap(), before);
    }
}
