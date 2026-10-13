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

fn prepared<'a>(
    doc: &'a photocraft_doc::Document,
    ctl: &Interrupt<'_>,
) -> photocraft_compose::gimp_composite::Result<photocraft_compose::gimp_render::scene_budget::ScenePrepared<'a>> {
    photocraft_compose::gimp_render::scene_budget::prepare(
        doc,
        ctl,
        photocraft_compose::gimp_render::scene_budget::SceneLimits { resident_bytes: 512 << 20, work_units: 36_000_000 * 512 },
    )
}
#[test]
#[ignore = "requires reviewed unpublished ordinary scene originals"]
fn current_ordinary_scene_bounds_masks_profiles_and_cancellation_are_checked() {
    let base = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let mut count = 0;
    {
        let variant = "medium";
        for model in ["rgb", "gray"] {
            for depth in ["u16", "u32", "f32", "f64"] {
                let path = base.join(format!("normal-mask-ordinary-scene-evidence/{variant}/{model}-{depth}-op2.xcf"));
                let bytes = fs::read(&path).unwrap();
                let original = photocraft_io::import("geometry.xcf", &bytes).unwrap().document;
                assert!(PreparedRenderer::new(&original).is_err());
                prepared(&original, &Interrupt::NONE).unwrap();
                let epoch = "xcf12-analytic-ordinary-normal-mask-v1";
                for policy in original.xcf_state.as_ref().unwrap().items.iter().filter_map(|item| item.composition.as_ref()) {
                    assert_eq!(policy.compatibility_epoch, epoch);
                }
                for (command, parameters) in
                    [("edit.assignProfile", serde_json::json!({"profile":"none"})), ("color.profileMismatch", serde_json::json!({"action":"discard"}))]
                {
                    let mut session = photocraft_engine::Session::new();
                    session.open_document(original.clone(), None).unwrap();
                    let before = session.active().unwrap().doc.clone();
                    assert!(session.execute(command, parameters).is_err(), "integer epoch cannot discard its required current interpretation");
                    assert_eq!(*session.active().unwrap().doc, *before);
                    assert!(!session.undo());
                }
                let temporary =
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp").join(format!("sol4-ordinary-scene-current-{}", std::process::id()));
                fs::create_dir_all(&temporary).unwrap();
                let identity_profile = temporary.join(format!("{variant}-{model}-{depth}.icc"));
                fs::write(&identity_profile, original.icc_profile.as_ref().unwrap().as_slice()).unwrap();
                let mut session = photocraft_engine::Session::new();
                session.open_document(original.clone(), None).unwrap();
                session.execute("edit.convertToProfile", serde_json::json!({"profile":identity_profile,"intent":"relative","bpc":false})).unwrap();
                let converted = (*session.active().unwrap().doc).clone();
                for layer in &original.layers {
                    let after = converted.layer(layer.id).unwrap();
                    assert_eq!(after.surface(), layer.surface(), "identity conversion preserves low native integer codes/defaults");
                    assert_eq!(after.mask, layer.mask);
                }
                assert_eq!(converted.source_color_context, original.source_color_context);
                prepared(&converted, &Interrupt::NONE).unwrap();
                assert!(session.undo());
                assert_eq!(*session.active().unwrap().doc, original);
                assert!(session.redo());
                assert_eq!(*session.active().unwrap().doc, converted);
                for other in [
                    photocraft_doc::import_compositing::XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH,
                    photocraft_doc::import_compositing::XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH,
                    photocraft_doc::import_compositing::XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH,
                ] {
                    let mut wrong_epoch = original.clone();
                    for policy in wrong_epoch.xcf_state.as_mut().unwrap().items.iter_mut().filter_map(|item| item.composition.as_mut()) {
                        policy.compatibility_epoch = other.into();
                    }
                    assert!(PreparedRenderer::new(&wrong_epoch).is_err(), "integer planes cannot borrow a floating geometry epoch");
                }
                let owner = original.layers.iter().find(|layer| layer.mask.is_some()).unwrap().id;
                let mut inert = original.clone();
                inert.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![1, 2, 3]));
                prepared(&inert, &Interrupt::NONE).unwrap();
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
                    assert!(prepared(&wrong, &Interrupt::NONE).is_err(), "{variant}/{model}/{depth}: current alteration {alteration}");
                    assert_eq!(wrong, before);
                }
                let polls = AtomicUsize::new(0);
                let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 5;
                assert!(prepared(&original, &Interrupt::cancel_only(&cancel)).is_err());
                assert!(polls.load(Ordering::Relaxed) >= 6);
                assert!(prepared(&original, &Interrupt::cancel_only(&|| true)).is_err());
                let native = photocraft_format::save_to_bytes(&original, &Default::default()).unwrap();
                assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), original);
                assert_eq!(fs::read(path).unwrap(), bytes);
                count += 1;
            }
        }
    }
    assert_eq!(count, 8);
}
