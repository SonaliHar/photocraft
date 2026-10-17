//! UNRUN actual selected owned import; no resident-backed positive shortcut.
use super::super::staged_accounted_group::tests::{read, records};
use super::*;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaLimits},
};
use std::sync::atomic::{AtomicU64, Ordering};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn limits() -> photocraft_xcf::ResourceLimits {
    photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap()
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
fn layer<'a>(layers: &'a [Layer], name: &str) -> &'a Layer {
    for current in layers {
        if current.name == name {
            return current;
        }
        if let LayerContent::Group(group) = &current.content
            && let Some(child) = group.children.iter().find(|child| child.name == name)
        {
            return child;
        }
    }
    panic!("original layer identity remains present")
}
fn exact(doc: &Document, original: &XcfFile, quota: &BackingQuota) {
    assert_eq!(doc.layers.len(), 2);
    let group = layer(&doc.layers, &original.layers[0].name);
    assert!(matches!(&group.content,LayerContent::Group(g) if g.children.len()==2));
    assert!(group.surface().is_none());
    for source in &original.layers {
        let actual = layer(&doc.layers, &source.name);
        assert_eq!(actual.name, source.name);
        assert_eq!(actual.visible, source.visible);
        assert_eq!(actual.opacity.to_bits(), source.opacity.to_bits());
        if let Some(plane) = &source.plane {
            let LayerContent::BackedRaster(backed) = &actual.content else { panic!("real charged native-backed leaf") };
            assert!(backed.quota().shares_ledger(quota));
            let view = backed.checked_view();
            let rect = bounds(source).unwrap();
            let bpp = view.format().bytes_per_pixel();
            let mut output = vec![0; plane.bytes.len()];
            for y in 0..source.height {
                for x in 0..source.width {
                    let at = (y as usize * source.width as usize + x as usize) * bpp;
                    view.read_logical_pixel_native_into(rect.x0 + x as i32, rect.y0 + y as i32, &mut output[at..at + bpp], &ctl()).unwrap();
                }
            }
            assert_eq!(output, plane.bytes);
        }
    }
    if let Some(mask) = &original.layers[0].mask {
        let actual = group.mask.as_ref().unwrap();
        let rect = bounds(&original.layers[0]).unwrap();
        let plane = mask.plane.as_ref().unwrap();
        assert_eq!(actual.surface.try_read_interleaved(rect).unwrap(), plane.bytes);
        let reference = Surface::try_from_interleaved_with(actual.surface.format(), rect, &plane.bytes, &Interrupt::NONE).unwrap();
        assert_eq!(actual.surface.default_bytes(), reference.default_bytes());
        assert_eq!(actual.surface.tile_count(), reference.tile_count());
        for (coord, tile) in reference.tiles() {
            assert_eq!(actual.surface.tile(*coord).unwrap().bytes(), tile.bytes());
        }
    } else {
        assert!(group.mask.is_none());
    }
    assert_eq!(doc.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap().as_slice(), original.icc);
    assert_eq!(doc.icc_profile.as_ref().unwrap().as_slice(), original.icc);
}
#[test]
#[ignore = "UNRUN:24 pinned original files, actual quota/native promotion, retained title copy and exact nested/native/mask custody"]
fn all_originals_promote_real_three_leaf_owners_and_keep_original_root() {
    let (root, rows) = records();
    for row in rows {
        let bytes = read(&root, &row);
        let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let q = quota();
        let before = q.usage().unwrap();
        let starts = AtomicU64::new(0);
        let progress = |v: f32| {
            if v.to_bits() == 0.05f32.to_bits() {
                starts.fetch_add(1, Ordering::Relaxed);
            }
        };
        let options = DecodeOptions { progress: Some(&progress), ..Default::default() };
        assert!(crate::import_owned_staged_plain_bytes_in_quota("strict.xcf", bytes.clone(), &Default::default(), &limits(), quota(), &ctl()).is_err());
        let selected = crate::import_owned_xcf_bytes_in_quota("original-group.xcf", bytes.clone(), &options, &limits(), q.clone(), &ctl()).unwrap();
        assert_eq!(starts.load(Ordering::Relaxed), 2, "immutable metadata preplan plus selected decode, without family retry");
        let crate::OwnedXcfImport::IsolatedNormalGroup(imported) = selected else { panic!("sealed isolated group family") };
        assert!(imported.source_read_only());
        assert!(!imported.preview_only());
        let owner = imported.document();
        owner.validate_assigned_isolated_normal_group_storage(&ctl()).unwrap();
        exact(owner.document(), &original, &q);
        let root_owner = owner.clone();
        let renamed = owner.try_with_document_name("Retained group.pcraft", &ctl()).unwrap();
        assert_eq!(renamed.document().name, "Retained group.pcraft");
        assert_eq!(renamed.document().xcf_state, owner.document().xcf_state);
        exact(renamed.document(), &original, &q);
        drop(imported);
        drop(root_owner);
        exact(renamed.document(), &original, &q);
        drop(renamed);
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(std::fs::read(root.join(row["source"]["file"].as_str().unwrap())).unwrap().as_slice(), bytes.as_slice());
    }
}
#[test]
#[ignore = "UNRUN:real work/cancel/full quota denial leaves source and enclosing ledger unchanged"]
fn failed_promotion_is_private_and_refunds_enclosing_catalog() {
    let (root, rows) = records();
    let bytes = read(&root, &rows[0]);
    for mode in 0..3 {
        let q = quota();
        let before = q.usage().unwrap();
        let stop = || mode == 0;
        let interrupt = Interrupt::cancel_only(&stop);
        let work = |_| if mode == 1 { Err(photocraft_raster::backing::BackingError::Work) } else { Ok(()) };
        let control = ReadControl { interrupt: &interrupt, work: &work };
        let block = if mode == 2 { Some(q.reserve(photocraft_raster::backing::quota::QuotaKind::Resident, 256 << 20).unwrap()) } else { None };
        let blocked = q.usage().unwrap();
        assert!(crate::import_owned_xcf_bytes_in_quota("group.xcf", bytes.clone(), &Default::default(), &limits(), q.clone(), &control).is_err());
        assert_eq!(q.usage().unwrap(), blocked);
        drop(block);
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(bytes.len() as u64, rows[0]["source"]["bytes"].as_u64().unwrap());
    }
}
