//! Existing genuine Source bytes imported with true selection scratch headroom.
//! Encoded/comparison inputs are external harness allocations, not receipts.
#![allow(dead_code, unused_imports)]
use crate::Session;
#[path = "accounted_programme_paint_test_support.rs"]
pub mod base;
#[path = "accounted_normal_hierarchy_test_support.rs"]
mod hierarchy;
#[path = "accounted_masked_filtered_flat_test_support.rs"]
mod masked;
#[path = "accounted_filtered_flat_ordered_test_support.rs"]
mod mixed;
#[path = "accounted_masked_scalar_test_support.rs"]
mod point;
pub use base::{Case, Input, Witness, adopt, archive, ctl, layer, patch, raw, reopen};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
pub const CASES: [Case; 4] = [Case::Point, Case::FilteredFlat, Case::MaskedFilteredFlat, Case::Hierarchy];

pub fn load(case: Case, depth: usize) -> Input {
    assert!(depth < 2);
    let precision = if depth == 0 { 650 } else { 750 };
    let bytes = match case {
        Case::Point => {
            let c = point::cases().into_iter().find(|c| c["source_precision"].as_u64() == Some(precision) && c["operation"] == "gegl:invert-linear").unwrap();
            point::bytes(&c)
        }
        Case::FilteredFlat => mixed::bytes(mixed::data::CASES[depth].source),
        Case::MaskedFilteredFlat => {
            let c = masked::cases().into_iter().find(|c| c["source_precision"].as_u64() == Some(precision)).unwrap();
            masked::bytes(&c)
        }
        Case::Hierarchy => {
            let c =
                hierarchy::cases().into_iter().find(|c| c["source_precision"].as_u64() == Some(precision) && c["source_version"].as_u64() == Some(22)).unwrap();
            hierarchy::bytes(&c)
        }
        _ => panic!("four explicitly selected genuine programme Source classes"),
    };
    let quota = BackingQuota::new(QuotaLimits { resident: 512 << 20, index: 64 << 20, scratch: 16 << 20 });
    let source = photocraft_io::import_owned_xcf_bytes_in_quota(
        "selection-programme-source.xcf",
        std::sync::Arc::new(bytes),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        quota,
        &ctl(),
    )
    .unwrap();
    let o = source.document();
    assert_eq!(u64::from(o.document().source_color_context.as_ref().unwrap().source_precision), precision);
    assert!(match case {
        Case::Point => o.is_readonly_point_storage(),
        Case::FilteredFlat => o.is_readonly_filtered_flat_point_storage(),
        Case::MaskedFilteredFlat => o.is_readonly_masked_filtered_flat_point_storage(),
        Case::Hierarchy => o.is_readonly_normal_hierarchy_storage(),
        _ => false,
    });
    Input::Source(source)
}

pub fn same_selection(a: &photocraft_doc::Document, b: &photocraft_doc::Document) {
    let (a, b) = (a.selection.as_ref().unwrap(), b.selection.as_ref().unwrap());
    assert_eq!(a.format(), b.format());
    assert_eq!(a.default_bytes(), b.default_bytes());
    assert_eq!(a.tile_count(), b.tile_count());
    for (coord, tile) in a.tiles() {
        ctl().check((b.tile_count() as u64) * 32 + (tile.bytes().len() as u64) * 2).unwrap();
        assert_eq!(tile.bytes(), b.tiles().find(|(k, _)| *k == coord).unwrap().1.bytes());
    }
}
