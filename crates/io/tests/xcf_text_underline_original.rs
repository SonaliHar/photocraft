#![cfg(feature = "corpus")]
use sha2::{Digest, Sha256};
#[test]
#[ignore = "private independently authored public-GIMP underline controls"]
fn original_underline_run_source_edit_and_native_pipeline() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-underline-evidence");
    let rows: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in rows["rows"].as_array().unwrap().iter().filter(|r| r["negative"] == false && !(r["size_px"] == 16 && r["antialias"] == false)) {
        for state in row["states"].as_array().unwrap() {
            let name = state["xcf_file"].as_str().unwrap();
            let bytes = std::fs::read(root.join(name)).unwrap();
            assert_eq!(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), state["source_sha256"].as_str().unwrap());
            let doc = photocraft_io::import(name, &bytes).unwrap_or_else(|e| panic!("{name}: actual original import: {e}")).document;
            let source = &doc.xcf_state.as_ref().unwrap().source_items[0];
            assert_eq!(source.cache.as_ref().unwrap().bytes.as_slice(), std::fs::read(root.join(state["state"]["native"]["file"].as_str().unwrap())).unwrap());
            let render = photocraft_compose::gimp_render::try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
            let expected = std::fs::read(root.join(state["state"]["projection"]["file"].as_str().unwrap())).unwrap();
            let actual: Vec<u8> = render.px.iter().flat_map(|p| p.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8)).collect();
            assert_eq!(actual.len(), expected.len());
            let max = actual.iter().zip(&expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            assert!(max <= 2, "{name}: {max}/255");
            let saved = photocraft_io::export(&doc, "underline.pcraft", &Default::default()).unwrap();
            assert_eq!(photocraft_io::import("underline.pcraft", &saved.bytes).unwrap().document, *photocraft_io::named_for_file(&doc, "underline.pcraft"));
            assert_eq!(std::fs::read(root.join(name)).unwrap(), bytes);
            count += 1;
        }
    }
    assert_eq!(count, 18);
}
#[test]
#[ignore = "private independently authored public-GIMP underline controls"]
fn actual_underline_canonical_classifier_crosschecks_raw_and_runs() {
    use photocraft_doc::{
        LayerId,
        item_identity::{ItemRef, ResourceId},
    };
    use photocraft_raster::Interrupt;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-underline-evidence");
    let rows: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut accepted = 0;
    let mut rejected = 0;
    for row in rows["rows"].as_array().unwrap() {
        for state in row["states"].as_array().unwrap() {
            let bytes = std::fs::read(root.join(state["xcf_file"].as_str().unwrap())).unwrap();
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let mut layer = file.layers.into_iter().find(|l| l.text.is_some()).unwrap();
            let plane = layer.plane.take();
            let mapped = photocraft_io::xcf_sources::map_source(
                &layer,
                plane,
                photocraft_io::xcf_sources::SourceContext {
                    owner: ItemRef::Layer(LayerId(99)),
                    vector_path: None,
                    format: photocraft_doc::PixelFormat::RGBA8,
                    source_version: file.version,
                },
                &Interrupt::NONE,
                &mut || ResourceId(100),
                &mut |_| Ok(()),
            )
            .unwrap()
            .unwrap();
            let result = photocraft_text::xcf::qualify_underlined_source(&mapped.item, &mapped.resources[0], [8, 4], &Interrupt::NONE);
            if row["negative"] == true || row["size_px"] == 16 && row["antialias"] == false {
                assert!(result.is_err());
                assert!(photocraft_io::import("underline-negative.xcf", &bytes).is_err(), "{} must not publish a document", state["xcf_file"]);
                rejected += 1;
            } else {
                let q = result.unwrap_or_else(|e| panic!("{}: {e}", state["xcf_file"]));
                assert_eq!(q.request.content, "HEFT");
                accepted += 1;
            }
            assert!(photocraft_text::xcf::qualify_scaled_rectilinear_source(&mapped.item, &mapped.resources[0], [8, 4], &Interrupt::NONE).is_err());
        }
    }
    assert_eq!((accepted, rejected), (18, 14));
}
#[test]
#[ignore = "private independently authored public-GIMP underline controls"]
fn actual_underline_own_glyph_and_run_kernel_projects_originals() {
    use photocraft_doc::{
        LayerId,
        item_identity::{ItemRef, ResourceId},
    };
    use photocraft_raster::Interrupt;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-underline-evidence");
    let rows: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let font = photocraft_text::xcf::ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    let mut count = 0;
    for row in rows["rows"].as_array().unwrap().iter().filter(|r| r["negative"] == false && !(r["size_px"] == 16 && r["antialias"] == false)) {
        for state in row["states"].as_array().unwrap() {
            let bytes = std::fs::read(root.join(state["xcf_file"].as_str().unwrap())).unwrap();
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let mut layer = file.layers.into_iter().find(|l| l.text.is_some()).unwrap();
            let plane = layer.plane.take();
            let mapped = photocraft_io::xcf_sources::map_source(
                &layer,
                plane,
                photocraft_io::xcf_sources::SourceContext {
                    owner: ItemRef::Layer(LayerId(99)),
                    vector_path: None,
                    format: photocraft_doc::PixelFormat::RGBA8,
                    source_version: file.version,
                },
                &Interrupt::NONE,
                &mut || ResourceId(100),
                &mut |_| Ok(()),
            )
            .unwrap()
            .unwrap();
            let q = photocraft_text::xcf::qualify_underlined_source(&mapped.item, &mapped.resources[0], [8, 4], &Interrupt::NONE).unwrap();
            let mut work_units = 0_u64;
            let mut work_calls = 0;
            let prepared =
                photocraft_text::xcf::underline::prepare_underlined_with_charge(&font, q, 8 * 1024 * 1024, &Interrupt::NONE, &mut |_| Ok(()), &mut |n| {
                    work_units += n;
                    work_calls += 1;
                    Ok(())
                })
                .unwrap();
            println!("UNDERLINE_WORK {} {} {}", state["xcf_file"].as_str().unwrap(), work_units, work_calls);
            let rendered = prepared.render_mode_with_charge(q.antialias, 8 * 1024 * 1024, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
            let rect = rendered.rect;
            let pixels = rendered.surface.try_read_interleaved(rect).unwrap();
            let mut actual = vec![0u8; 128 * 64 * 4];
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    if (0..128).contains(&x) && (0..64).contains(&y) {
                        let src = usize::try_from(((y - rect.y0) * rect.width() as i32 + x - rect.x0) * 4).unwrap();
                        let dst = usize::try_from((y * 128 + x) * 4).unwrap();
                        actual[dst..dst + 4].copy_from_slice(&pixels[src..src + 4]);
                    }
                }
            }
            let expected = std::fs::read(root.join(state["state"]["projection"]["file"].as_str().unwrap())).unwrap();
            let max = actual.iter().zip(&expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            println!("{}: {max}/255", state["xcf_file"]);
            assert!(max <= 2);
            count += 1;
        }
    }
    assert_eq!(count, 18);
}

#[test]
#[ignore = "private independently authored public-GIMP underline controls"]
fn underline_shared_renderer_context_resident_and_cancel_guards() {
    use photocraft_compose::{gimp_composite::CompositionError, gimp_text_items as text};
    use photocraft_raster::{Interrupt, RasterInputError};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-underline-evidence");
    let bytes = std::fs::read(root.join("aa-32-whole.source.xcf")).unwrap();
    let doc = photocraft_io::import("text.xcf", &bytes).unwrap().document;
    let source = &doc.xcf_state.as_ref().unwrap().source_items[0];
    assert!(text::qualify_current_source(&doc, source, &Interrupt::NONE).is_ok());
    assert!(text::qualify_layout_source(&doc, source, &Interrupt::NONE).is_err());
    let denied = CompositionError::Invalid("owned underline quota sentinel");
    for limit in [0, 411640] {
        let mut charged = 0;
        let result = text::render_source(&doc, source, &Interrupt::NONE, &mut |n| {
            charged += n;
            if charged > limit { Err(denied.clone()) } else { Ok(()) }
        });
        assert!(matches!(result,Err(ref e) if *e==denied));
    }
    assert!(matches!(
        text::render_source(&doc, source, &Interrupt::cancel_only(&|| true), &mut |_| Ok(())),
        Err(CompositionError::Raster(RasterInputError::Cancelled))
    ));
    let mut mismatched = doc.clone();
    mismatched.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == source.owner).unwrap().raster_bounds.as_mut().unwrap().x0 += 1;
    assert!(text::qualify_current_source(&mismatched, &mismatched.xcf_state.as_ref().unwrap().source_items[0], &Interrupt::NONE).is_err());
    let mut partial = doc.clone();
    partial.size.height = 128;
    assert!(text::qualify_current_source(&partial, &partial.xcf_state.as_ref().unwrap().source_items[0], &Interrupt::NONE).is_err());
    assert_eq!(std::fs::read(root.join("aa-32-whole.source.xcf")).unwrap(), bytes);
}

#[test]
#[ignore = "private independently authored public-GIMP underline controls"]
fn underline_lowered_work_budget_is_not_reset_or_reclassified_as_resident() {
    use photocraft_xcf::{Budget, BudgetKind, DecodeOptions, ErrorKind, ResourceLimits, ResourcePolicy, SourceAccounting};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-underline-evidence");
    let bytes = std::fs::read(root.join("aa-16-whole.source.xcf")).unwrap();
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("sampleWorkUnits", 16 << 20)]).unwrap();
    let budget = Budget::new(limits, None);
    let denied = photocraft_io::xcf_import::import_candidate_with_budget(
        "underline.xcf",
        &bytes,
        &photocraft_raster::Interrupt::NONE,
        &DecodeOptions::default(),
        &budget,
        SourceAccounting::Borrowed,
    )
    .unwrap_err();
    assert!(matches!(denied,photocraft_io::IoError::Xcf(ref e) if e.kind==ErrorKind::Limit && e.message.contains("XCF underline preparation")), "{denied}");
    let consumed = budget.used(BudgetKind::Work);
    assert!(consumed > 0);
    assert!(consumed <= 16 << 20);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
    assert!(
        photocraft_io::xcf_import::import_candidate_with_budget(
            "underline.xcf",
            &bytes,
            &photocraft_raster::Interrupt::NONE,
            &DecodeOptions::default(),
            &budget,
            SourceAccounting::Borrowed
        )
        .is_err()
    );
    assert!(budget.used(BudgetKind::Work) >= consumed);
}
