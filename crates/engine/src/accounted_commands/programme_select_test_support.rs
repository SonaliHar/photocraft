//! Test-only genuine multiroot Source loaders; no new Source or pixel grant.
#![allow(dead_code, unused_imports)]
use super::Session;
#[path = "../accounted_masked_filtered_flat_test_support.rs"]
mod masked;
#[path = "../accounted_filtered_flat_ordered_test_support.rs"]
mod ordered;
#[path = "../accounted_programme_paint_test_support.rs"]
pub mod programmes;
pub use programmes::{Witness, ctl};
pub fn session(mask: bool, depth: usize) -> Session {
    let input = if mask {
        let id = if depth == 0 { "f32-two-roots" } else { "f64-two-roots" };
        let c = masked::cases().into_iter().find(|c| c["id"].as_str() == Some(id)).unwrap();
        masked::import(&c)
    } else {
        ordered::import(&ordered::data::CASES[depth])
    };
    let mut s = Session::new();
    let case = if mask { programmes::Case::MaskedFilteredFlat } else { programmes::Case::FilteredFlat };
    programmes::adopt(&mut s, case, &programmes::Input::Source(input));
    s
}
pub fn roots(d: &photocraft_doc::Document) -> (photocraft_doc::LayerId, photocraft_doc::LayerId) {
    let id = |name: &str| d.layers.iter().find(|l| l.name == name).unwrap().id;
    let a = id("Own root 0");
    let b = id("Own root 1");
    assert_ne!(a, b);
    (a, b)
}
