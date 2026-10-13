#![cfg(feature = "corpus")]
//! Current geometry contracts are independent of immutable source provenance.
use photocraft_compose::gimp_render::PreparedRenderer;
use photocraft_doc::{SampleEncoding, item_identity::ItemRef};
use photocraft_geom::Size;
use photocraft_raster::Interrupt;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[test]
#[ignore = "requires reviewed unpublished Normal geometry originals"]
fn current_geometry_bounds_masks_profiles_and_cancellation_are_checked() {
    let base = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let mut count = 0;
    for variant in ["left-top", "right-bottom"] {
        for model in ["rgb", "gray"] {
            for depth in ["f32", "f64"] {
                let path = base.join(format!("normal-mask-geometry-holdout-evidence/{variant}/{model}-{depth}-op2.xcf"));
                let bytes = fs::read(&path).unwrap();
                let original = photocraft_io::import("geometry.xcf", &bytes).unwrap().document;
                PreparedRenderer::new(&original).unwrap();
                let owner = original.layers.iter().find(|layer| layer.mask.is_some()).unwrap().id;
                let mut inert = original.clone();
                inert.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![1, 2, 3]));
                PreparedRenderer::new(&inert).unwrap();
                for alteration in 0..6 {
                    let mut wrong = original.clone();
                    match alteration {
                        0 => wrong.icc_profile = Some(Arc::new(vec![1, 2, 3])),
                        1 => wrong.sample_encoding = Some(SampleEncoding::Linear),
                        2 => wrong.size = Size::new(65, 65),
                        3 => {
                            let state = wrong.xcf_state.as_mut().unwrap();
                            let metadata =
                                state.items.iter_mut().find_map(|item| item.mask.as_mut().filter(|mask| mask.owner == ItemRef::Layer(owner))).unwrap();
                            metadata.bounds.x0 -= 1;
                        }
                        4 => {
                            let base_id = wrong.layers[0].id;
                            wrong
                                .xcf_state
                                .as_mut()
                                .unwrap()
                                .items
                                .iter_mut()
                                .find(|item| item.item == ItemRef::Layer(base_id))
                                .unwrap()
                                .raster_bounds
                                .as_mut()
                                .unwrap()
                                .x0 += 1;
                        }
                        5 => wrong.layers.iter_mut().find(|layer| layer.id == owner).unwrap().mask.as_mut().unwrap().enabled = false,
                        _ => unreachable!(),
                    }
                    let before = wrong.clone();
                    assert!(PreparedRenderer::new(&wrong).is_err(), "{variant}/{model}/{depth}: current alteration {alteration}");
                    assert_eq!(wrong, before);
                }
                let polls = AtomicUsize::new(0);
                let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 5;
                assert!(PreparedRenderer::new_with_interrupt(&original, &Interrupt::cancel_only(&cancel)).is_err());
                assert!(polls.load(Ordering::Relaxed) >= 6);
                assert!(PreparedRenderer::new_with_interrupt(&original, &Interrupt::cancel_only(&|| true)).is_err());
                let native = photocraft_format::save_to_bytes(&original, &Default::default()).unwrap();
                assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), original);
                assert_eq!(fs::read(path).unwrap(), bytes);
                count += 1;
            }
        }
    }
    assert_eq!(count, 8);
}
