use photocraft_compose::gimp_vector_items::render_source;
use photocraft_doc::item_identity::{ItemRef, ResourceId};
use photocraft_doc::xcf_filters::{ExactNumber as N, FilterArgument as A, FilterValue as V};
use photocraft_doc::xcf_items::{EditableSource, SourceCache, SourceItem};
use photocraft_doc::xcf_resources::{ResourceKind, ResourceReference};
use photocraft_doc::*;
use photocraft_geom::Size;
use photocraft_raster::Interrupt;
fn arg(name: &str, value: V) -> A {
    A { name: name.into(), value }
}
fn uint(n: u64) -> V {
    V::Number(N::Unsigned { width: 32, value: n })
}
fn fill(hex: &str) -> V {
    V::Config(vec![
        arg("style", uint(0)),
        arg("antialias", V::Bool(true)),
        arg("color", V::Config(vec![arg("encoding", V::String("R'G'B'A float".into())), arg("native-samples-hex", V::String(hex.into()))])),
        arg("pattern", V::Resource(ResourceId(1))),
    ])
}
fn example() -> (Document, SourceItem) {
    let mut doc = Document::new("rectangle", Size::new(64, 48), ColorMode::Rgb, SampleType::U8);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    let active = photocraft_cms::xcf_color::effective_profile(None, photocraft_cms::ColorSpace::Rgb, photocraft_cms::xcf_color::Encoding::NonlinearProfile)
        .unwrap()
        .to_bytes();
    doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&active);
    doc.icc_profile = Some(active);

    doc.source_color_context = Some(SourceColorContext { source_precision: 150, original_icc: None });
    let layer = Layer::raster("vector", PixelFormat::RGBA8);
    let owner = ItemRef::Layer(layer.id);
    doc.layers.push(layer);
    let path = NamedPath::new("rectangle", Path::new(vec![Subpath::polygon(&[(12.25, 14.5), (45.5, 14.5), (45.5, 32.75), (12.25, 32.75)])]));
    let id = path.id;
    doc.paths.push(path);
    let configuration = vec![
        arg("source-path-tattoo", uint(1201)),
        arg("filled", V::Bool(true)),
        arg("stroked", V::Bool(false)),
        arg("fill", fill("0000803e0000303f0000e03e0000803f")),
        arg(
            "stroke",
            V::Config(vec![
                arg("fill", fill("0000000000000000000000000000803f")),
                arg("width", V::Number(N::Float32Bits(6f32.to_bits()))),
                arg("cap-style", uint(0)),
                arg("join-style", uint(0)),
                arg("miter-limit", V::Number(N::Float32Bits(10f32.to_bits()))),
                arg("dashes", V::Config(vec![])),
            ]),
        ),
    ];
    let item = SourceItem {
        owner,
        source: EditableSource::Vector { path: id, source_modified: false, configuration },
        transform: None,
        cache: Some(SourceCache {
            bytes: vec![0; 4].into(),
            blake3: "ec2bd03bf86b935fa34d71ad7ebb049f1f10f87d343e521511d8f9e6625620cd".into(),
            width: 1,
            height: 1,
            format: PixelFormat::RGBA8,
        }),
    };
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 24;
    for (item, name) in [(owner, "vector"), (ItemRef::Path(id), "rectangle")] {
        state.items.push(photocraft_doc::xcf_state::ItemState {
            item,
            name: name.into(),
            custom_name: false,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: None,
            mask: None,
            composition: None,
        });
    }
    state.source_items.push(item.clone());
    state.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::Pattern,
        name: "inert".into(),
        collection: None,
        internal: false,
        locator: Some("https://invalid.example/pattern".into()),
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    });
    doc.xcf_state = Some(state);
    (doc, item)
}
#[test]
fn regenerates_current_geometry_not_cache_and_tracks_path_edit() {
    let (mut doc, item) = example();
    let a = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    assert_eq!(a.bounds, Rect::new(12, 14, 46, 33));
    assert_eq!(a.surface.try_read_interleaved(Rect::new(14, 16, 15, 17)).unwrap(), [64, 175, 112, 255]);
    for k in &mut doc.paths[0].path.subpaths[0].knots {
        k.anchor.x -= 2.;
        k.anchor.y += 3.;
        k.in_ctrl = k.anchor;
        k.out_ctrl = k.anchor;
    }
    let b = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    assert_eq!(b.bounds, Rect::new(10, 17, 44, 36));
    assert_eq!(item.cache.as_ref().unwrap().bytes.as_slice(), [0; 4]);
}
#[test]
fn rejects_unknown_duplicate_modified_profile_cache_and_budget() {
    for variant in 0..6 {
        let (mut doc, mut item) = example();
        match variant {
            0 => {
                if let EditableSource::Vector { configuration, .. } = &mut item.source {
                    configuration.push(arg("unknown", V::Bool(true)));
                }
            }
            1 => {
                if let EditableSource::Vector { source_modified, .. } = &mut item.source {
                    *source_modified = true;
                }
            }
            2 => doc.source_color_context.as_mut().unwrap().source_precision = 650,
            3 => std::sync::Arc::make_mut(&mut item.cache.as_mut().unwrap().bytes)[0] = 1,
            4 => doc.icc_profile = Some(std::sync::Arc::new(vec![0; 64])),
            _ => {
                if let EditableSource::Vector { configuration, .. } = &mut item.source {
                    configuration[1] = configuration[0].clone();
                }
            }
        };
        doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
        assert!(render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).is_err());
    }
    let (cancel_doc, cancel_item) = example();
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let check = || cancelled.load(std::sync::atomic::Ordering::Relaxed);
    let result = render_source(&cancel_doc, &cancel_item, &Interrupt::cancel_only(&check), &mut |_| Ok(()), &mut |_| {
        cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    });
    assert!(matches!(result, Err(photocraft_compose::gimp_composite::CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled))));
    let (doc, item) = example();
    assert!(render_source(&doc, &item, &Interrupt::cancel_only(&|| true), &mut |_| Ok(()), &mut |_| Ok(())).is_err());
    assert!(
        render_source(&doc, &item, &Interrupt::NONE, &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("denied")), &mut |_| Ok(()))
            .is_err()
    );
    assert!(
        render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("denied")))
            .is_err()
    );
}

#[test]
#[ignore = "requires independently authored local vector rectangle packet"]
fn recorded_canonical_rectangle_regenerates_exact_gimp_reference() {
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/vector-rectangle-evidence");
    for phase in ["source", "edit"] {
        let original = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        let hash: String = Sha256::digest(&original).iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(
            hash,
            if phase == "source" {
                "534fc2bf3b95b2384768832e43a60cf52f42286b6e178fc1b8df3989dcfbe9bd"
            } else {
                "e67908d884bead724342fa3ed515a3784c3406a62ea8f75c681170c12bb139f7"
            }
        );
        let (mut doc, mut item) = example();
        let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
        configuration[3] = arg("fill", fill(if phase == "source" { "8780803eb4b0303fe6e0e03e0000803f" } else { "8d80803eb5b0303fece0e03e0000803f" }));
        if phase == "edit" {
            for k in &mut doc.paths[0].path.subpaths[0].knots {
                k.anchor.x -= 2.;
                k.anchor.y += 3.;
                k.in_ctrl = k.anchor;
                k.out_ctrl = k.anchor;
            }
        }
        doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
        let actual = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        let projected = actual.surface.try_read_interleaved(Rect::new(0, 0, 64, 48)).unwrap();
        let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
        assert_eq!(projected, expected);
        let expected_hash = if phase == "source" {
            "e8eeebe272dd5bd5ce317b770fc26e9a6dc3d4e6590084e42d53a642b468672f"
        } else {
            "f336c5a9f4c6b7258db09986bf9c53134099bddf66083fb3d383b2c5a00cc846"
        };
        assert_eq!(Sha256::digest(&expected).iter().map(|v| format!("{v:02x}")).collect::<String>(), expected_hash);
    }
}

#[test]
fn prepared_consumer_regenerates_once_charges_and_cancels() {
    use photocraft_cms::WideWork;
    use photocraft_compose::gimp_composite::CompositionError;
    use photocraft_compose::gimp_render::PreparedRenderer;
    let (doc, _) = example();
    let prepared = PreparedRenderer::new(&doc).unwrap();
    let mut resident = 0u64;
    let mut peak = 0u64;
    let mut work = 0u64;
    let output = prepared
        .render(Rect::new(14, 16, 16, 18), &mut |event| {
            match event {
                WideWork::ReserveBytes(n) => {
                    resident += n;
                    peak = peak.max(resident);
                }
                WideWork::ReleaseBytes(n) => resident -= n,
                WideWork::SampleUnits(n) => work += n,
            };
            Ok(())
        })
        .unwrap();
    assert_eq!(output.px.len(), 4);
    assert!(output.px.iter().all(|p| p[3] == 1.));
    assert!(peak > 256 * 256 * 4);
    assert!(work > 4);
    assert_eq!(resident, 4 * 32, "only returned wide output remains resident");
    let (mut moved, _) = example();
    moved.xcf_state.as_mut().unwrap().items[0].raster_bounds = Some(Rect::new(12, 14, 46, 33));
    for knot in &mut moved.paths[0].path.subpaths[0].knots {
        knot.anchor.x -= 2.;
        knot.anchor.y += 3.;
        knot.in_ctrl = knot.anchor;
        knot.out_ctrl = knot.anchor;
    }
    let regenerated = PreparedRenderer::new(&moved).unwrap().render(Rect::new(11, 34, 12, 35), &mut |_| Ok(())).unwrap();
    assert_eq!(regenerated.px[0][3], 1., "current geometry extends beyond immutable original drawable bounds");
    assert!(
        prepared
            .render(Rect::new(14, 16, 16, 18), &mut |event| if matches!(event,WideWork::ReserveBytes(n) if n>1024) {
                Err(CompositionError::Invalid("test resident denial"))
            } else {
                Ok(())
            })
            .is_err()
    );
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let check = || cancelled.load(std::sync::atomic::Ordering::Relaxed);
    let ctl = Interrupt::cancel_only(&check);
    assert!(
        prepared
            .render_with_interrupt(
                Rect::new(14, 16, 16, 18),
                &mut |event| {
                    if matches!(event,WideWork::SampleUnits(n) if n>0) {
                        cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    Ok(())
                },
                &ctl
            )
            .is_err()
    );
}

#[test]
fn arch_prepared_current_controls_budget_cancel_and_cache_invariants() {
    use photocraft_cms::WideWork;
    use photocraft_compose::{gimp_composite::CompositionError, gimp_render::PreparedRenderer};
    let (mut doc, item) = orthogonal_example();
    let a = photocraft_geom::Point::new(8., 32.);
    let d = photocraft_geom::Point::new(54., 32.);
    doc.paths[0].path = Path::new(vec![Subpath {
        knots: vec![
            Knot { anchor: a, in_ctrl: a, out_ctrl: photocraft_geom::Point::new(18., 8.), smooth: false },
            Knot { anchor: d, in_ctrl: photocraft_geom::Point::new(40., 8.), out_ctrl: d, smooth: false },
        ],
        closed: true,
        op: PathOp::Combine,
    }]);
    let before = doc.clone();
    let bounds = Rect::new(0, 0, 64, 48);
    let prepared = PreparedRenderer::new(&doc).unwrap();
    let original = prepared.render(bounds, &mut |_| Ok(())).unwrap();
    assert_eq!(doc, before);
    assert_eq!(doc.xcf_state.as_ref().unwrap().source_items[0].cache, item.cache);
    assert!(
        prepared
            .render(bounds, &mut |event| if matches!(event,WideWork::ReserveBytes(n) if n>1024) {
                Err(CompositionError::Invalid("arch resident denied"))
            } else {
                Ok(())
            })
            .is_err()
    );
    let stop = std::sync::atomic::AtomicBool::new(false);
    let cancelled = || stop.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        prepared
            .render_with_interrupt(
                bounds,
                &mut |event| {
                    if matches!(event,WideWork::SampleUnits(n) if n>0) {
                        stop.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    Ok(())
                },
                &Interrupt::cancel_only(&cancelled)
            )
            .is_err()
    );
    let mut edited = doc.clone();
    edited.paths[0].path.subpaths[0].knots[0].out_ctrl.y = 20.;
    let current = PreparedRenderer::new(&edited).unwrap().render(bounds, &mut |_| Ok(())).unwrap();
    assert_ne!(current.px, original.px);
    assert_eq!(edited.layers, doc.layers);
    assert_eq!(edited.xcf_state, doc.xcf_state);
    edited.paths[0].path.subpaths[0].knots[0].out_ctrl.x = 18.001;
    assert!(PreparedRenderer::new(&edited).is_err());
}

fn orthogonal_example() -> (Document, SourceItem) {
    let (mut doc, mut item) = example();
    doc.paths[0].path = Path::new(vec![Subpath::polygon(&[(10.25, 11.75), (47.5, 11.75), (47.5, 20.25), (24.75, 20.25), (24.75, 38.5), (10.25, 38.5)])]);
    let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
    let V::Config(fill) = &mut configuration[3].value else { panic!() };
    fill[1].value = V::Bool(false);
    doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
    (doc, item)
}
#[test]
fn canonical_noaa_concave_shape_prepared_edit_and_dependency_errors() {
    let (mut doc, item) = orthogonal_example();
    let direct = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    assert_eq!(direct.bounds, Rect::new(10, 11, 48, 39));
    use photocraft_compose::gimp_render::PreparedRenderer;
    let current = PreparedRenderer::new(&doc).unwrap().render(Rect::new(30, 25, 31, 26), &mut |_| Ok(())).unwrap();
    assert_eq!(current.px[0][3], 0., "concave cutout remains empty");
    for k in &mut doc.paths[0].path.subpaths[0].knots {
        k.anchor.x -= 2.;
        k.anchor.y += 3.;
        k.in_ctrl = k.anchor;
        k.out_ctrl = k.anchor;
    }
    let current = PreparedRenderer::new(&doc).unwrap().render(Rect::new(9, 39, 10, 40), &mut |_| Ok(())).unwrap();
    assert_eq!(current.px[0][3], 1.);
    doc.paths.clear();
    assert!(render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).is_err());
    let (mut doc, item) = orthogonal_example();
    doc.xcf_state.as_mut().unwrap().resources.clear();
    assert!(render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).is_err());
}
#[test]
#[ignore = "requires own canonical stock no-AA source/edit and independent PNG/native pair"]
fn canonical_noaa_matches_independent_stock_source_edit_references() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/orthogonal-canonical");
    for phase in ["source", "edit"] {
        let original = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        let expected_hash = if phase == "source" {
            "4df42225a017b2c4548e14defabae15f1662c52f003b2e9b59ea693e695042ad"
        } else {
            "270f6079a4d7e0ef2f4300f731f2acb0ec5b2ebce221aca8b2bb51588cde4fd0"
        };
        assert_eq!(Sha256::digest(&original).iter().map(|v| format!("{v:02x}")).collect::<String>(), expected_hash);
        let (mut doc, mut item) = orthogonal_example();
        let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
        configuration[3] = arg("fill", fill(if phase == "source" { "8780803eb4b0303fe6e0e03e0000803f" } else { "8d80803eb5b0303fece0e03e0000803f" }));
        let V::Config(fill) = &mut configuration[3].value else { panic!() };
        fill[1].value = V::Bool(false);
        if phase == "edit" {
            for k in &mut doc.paths[0].path.subpaths[0].knots {
                k.anchor.x -= 2.;
                k.anchor.y += 3.;
                k.in_ctrl = k.anchor;
                k.out_ctrl = k.anchor;
            }
        }
        doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
        let actual = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
        assert_eq!(actual.surface.try_read_interleaved(Rect::new(0, 0, 64, 48)).unwrap(), expected);
        let rendered = PreparedRenderer::new(&doc).unwrap().render(Rect::new(0, 0, 64, 48), &mut |_| Ok(())).unwrap().try_display().unwrap();
        let expected: Vec<_> = expected.iter().map(|v| f32::from(*v) / 255.).collect();
        let max = rendered.px.iter().flatten().zip(&expected).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        assert!(max <= 1. / 255., "independent profile-aware display max {max}");
    }
}

fn compound_example() -> (Document, SourceItem) {
    let (mut doc, item) = orthogonal_example();
    doc.paths[0].path = Path::new(vec![
        Subpath::polygon(&[(4.25, 4.75), (44.5, 4.75), (44.5, 42.5), (4.25, 42.5)]),
        Subpath::polygon(&[(14.25, 14.75), (14.25, 32.5), (34.5, 32.5), (34.5, 14.75)]),
        Subpath::polygon(&[(50.25, 8.75), (60.5, 8.75), (60.5, 24.5), (50.25, 24.5)]),
    ]);
    (doc, item)
}
#[test]
fn compound_prepared_uses_current_hole_and_retains_cache() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    let (mut doc, item) = compound_example();
    let cache = item.cache.clone();
    let sample = |d: &Document, x, y| PreparedRenderer::new(d).unwrap().render(Rect::new(x, y, x + 1, y + 1), &mut |_| Ok(())).unwrap().px[0][3];
    assert_eq!(sample(&doc, 20, 20), 0.);
    assert_eq!(sample(&doc, 55, 12), 1.);
    assert_eq!(sample(&doc, 15, 20), 0.);
    for knot in &mut doc.paths[0].path.subpaths[1].knots {
        knot.anchor.x += 2.;
        knot.in_ctrl = knot.anchor;
        knot.out_ctrl = knot.anchor;
    }
    assert_eq!(sample(&doc, 15, 20), 1.);
    assert_eq!(doc.xcf_state.as_ref().unwrap().source_items[0].cache, cache);
}
#[test]
#[ignore = "requires own canonical stock no-AA source/edit and independent PNG/native pair"]
fn canonical_compound_hole_and_island_match_independent_stock_pair() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/compound-canonical");
    for phase in ["source", "edit"] {
        let original = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        let expected_hash = if phase == "source" {
            "de6b936e0873198508953cb293b1256b2627ec4314adc1912f289bb8ab2bb2bd"
        } else {
            "ad479b969a799f7dc3dd5bfd40089f8651b55861ad893b7fbfc5ba15e93ff5bf"
        };
        assert_eq!(Sha256::digest(&original).iter().map(|v| format!("{v:02x}")).collect::<String>(), expected_hash);
        let (mut doc, mut item) = compound_example();
        let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
        configuration[3] = arg("fill", fill(if phase == "source" { "8d80803eb5b0303fece0e03e0000803f" } else { "9180803eb7b0303ff3e0e03e0000803f" }));
        let V::Config(fill) = &mut configuration[3].value else { panic!() };
        fill[1].value = V::Bool(false);
        if phase == "edit" {
            for k in &mut doc.paths[0].path.subpaths[1].knots {
                k.anchor.x += 2.;
                k.anchor.y -= 1.;
                k.in_ctrl = k.anchor;
                k.out_ctrl = k.anchor;
            }
        }
        doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
        let actual = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
        assert_eq!(actual.surface.try_read_interleaved(Rect::new(0, 0, 64, 48)).unwrap(), expected);
        let rendered = PreparedRenderer::new(&doc).unwrap().render(Rect::new(0, 0, 64, 48), &mut |_| Ok(())).unwrap().try_display().unwrap();
        let expected: Vec<_> = expected.iter().map(|v| f32::from(*v) / 255.).collect();
        let max = rendered.px.iter().flatten().zip(&expected).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        assert!(max <= 1. / 255., "independent profile-aware display max {max}");
    }
}

#[test]
#[ignore = "requires own canonical stock no-AA source/edit and independent PNG/native pair"]
fn canonical_same_winding_parity_matches_independent_stock_pair() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/compound-winding-control");
    for phase in ["source", "edit"] {
        let original = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        let expected_hash = if phase == "source" {
            "b68dc17b58a855c5114dbae79568ec7c2da5c42786c7d57a3d2c20a49d4e943f"
        } else {
            "e3a1039ce15dbad2839326eb7231dea85c92d3f04202aeeaa8714b51ea4dc312"
        };
        assert_eq!(Sha256::digest(&original).iter().map(|v| format!("{v:02x}")).collect::<String>(), expected_hash);
        let (mut doc, mut item) = compound_example();
        doc.paths[0].path.subpaths[1].knots.reverse();
        assert_eq!(doc.paths[0].path.fill_rule, photocraft_doc::FillRule::NonZero);
        let cache = item.cache.clone();
        let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
        configuration[3] = arg("fill", fill(if phase == "source" { "9180803eb7b0303ff3e0e03e0000803f" } else { "9880803eb9b0303ff8e0e03e0000803f" }));
        let V::Config(fill) = &mut configuration[3].value else { panic!() };
        fill[1].value = V::Bool(false);
        if phase == "edit" {
            for k in &mut doc.paths[0].path.subpaths[1].knots {
                k.anchor.x += 2.;
                k.anchor.y -= 1.;
                k.in_ctrl = k.anchor;
                k.out_ctrl = k.anchor;
            }
        }
        doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
        let actual = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        let expected = std::fs::read(base.join(format!("{phase}-reopened.projection.native.bin"))).unwrap();
        assert_eq!(item.cache, cache);
        assert_eq!(doc.paths[0].path.fill_rule, photocraft_doc::FillRule::NonZero);
        assert_eq!(actual.surface.try_read_interleaved(Rect::new(0, 0, 64, 48)).unwrap(), expected);
        let rendered = PreparedRenderer::new(&doc).unwrap().render(Rect::new(0, 0, 64, 48), &mut |_| Ok(())).unwrap().try_display().unwrap();
        let expected: Vec<_> = expected.iter().map(|v| f32::from(*v) / 255.).collect();
        let max = rendered.px.iter().flatten().zip(&expected).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        assert!(max <= 1. / 255., "independent profile-aware display max {max}");
    }
}

fn stroked_rectangle_example(edit: bool) -> (Document, SourceItem) {
    let (mut doc, mut item) = example();
    doc.paths[0].path = Path::new(vec![Subpath::polygon(&[(10., 12.), (46., 12.), (46., 36.), (10., 36.)])]);
    if edit {
        for k in &mut doc.paths[0].path.subpaths[0].knots {
            k.anchor.x += 2.;
            k.anchor.y -= 1.;
            k.in_ctrl = k.anchor;
            k.out_ctrl = k.anchor;
        }
    }
    let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
    configuration[1] = arg("filled", V::Bool(false));
    configuration[2] = arg("stroked", V::Bool(true));
    configuration[3] = arg("fill", fill("0000803f0000803f0000803f0000803f"));
    let V::Config(stroke) = &mut configuration[4].value else { panic!() };
    stroke[0] = arg("fill", fill(if edit { "9180803eb7b0303ff3e0e03e0000803f" } else { "8d80803eb5b0303fece0e03e0000803f" }));
    let V::Config(f) = &mut stroke[0].value else { panic!() };
    f[1].value = V::Bool(false);
    stroke[1].value = V::Number(N::Float32Bits(if edit { 8f32.to_bits() } else { 6f32.to_bits() }));
    doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
    (doc, item)
}
#[test]
fn current_solid_stroke_geometry_style_cache_budget_and_cancel() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    let (mut doc, item) = stroked_rectangle_example(false);
    let cache = item.cache.clone();
    let rendered = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    assert_eq!(rendered.bounds, Rect::new(7, 9, 49, 39));
    assert_eq!(rendered.surface.try_read_interleaved(Rect::new(13, 20, 14, 21)).unwrap()[3], 0);
    let EditableSource::Vector { configuration, .. } = &mut doc.xcf_state.as_mut().unwrap().source_items[0].source else { panic!() };
    let V::Config(stroke) = &mut configuration[4].value else { panic!() };
    stroke[1].value = V::Number(N::Float32Bits(8f32.to_bits()));
    let updated = doc.xcf_state.as_ref().unwrap().source_items[0].clone();
    let rendered = render_source(&doc, &updated, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    assert_eq!(rendered.bounds, Rect::new(6, 8, 50, 40));
    assert_eq!(rendered.surface.try_read_interleaved(Rect::new(13, 20, 14, 21)).unwrap()[3], 255);
    assert_eq!(updated.cache, cache);
    assert!(
        render_source(
            &doc,
            &updated,
            &Interrupt::NONE,
            &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("test denial")),
            &mut |_| Ok(())
        )
        .is_err()
    );
    let cancelled = || true;
    let ctl = Interrupt::cancel_only(&cancelled);
    assert!(render_source(&doc, &updated, &ctl, &mut |_| Ok(()), &mut |_| Ok(())).is_err());
    assert_eq!(PreparedRenderer::new(&doc).unwrap().render(Rect::new(13, 20, 14, 21), &mut |_| Ok(())).unwrap().px[0][3], 1.);
}
#[test]
#[ignore = "requires own canonical solid stroke source/edit and fresh native/PNG witnesses"]
fn canonical_solid_stroke_matches_independent_stock_source_edit() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/stroke-canonical");
    for (phase, edit, hash) in [
        ("source", false, "afe3e369553c1f714525cb25e8b6e8bc84e8e49427beb75f17fb7f236e213cd7"),
        ("edit", true, "7b1a4a48a8dd93c89d5a876a1aaf5df7b7edaa62983e6cb9b3d8423fcdad9653"),
    ] {
        let raw = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        assert_eq!(Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect::<String>(), hash);
        let (doc, item) = stroked_rectangle_example(edit);
        let cache = item.cache.clone();
        let surface = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap().surface;
        let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
        assert_eq!(surface.try_read_interleaved(Rect::new(0, 0, 64, 48)).unwrap(), expected);
        assert_eq!(item.cache, cache);
        let output = PreparedRenderer::new(&doc).unwrap().render(Rect::new(0, 0, 64, 48), &mut |_| Ok(())).unwrap().try_display().unwrap();
        let max = output.px.iter().flatten().zip(&expected).map(|(a, b)| (a - f32::from(*b) / 255.).abs()).fold(0f32, f32::max);
        assert!(max <= 1. / 255., "independent stroke projected display max{max}");
    }
}

#[test]
#[ignore = "requires own odd-width miter original source/edit and independent repeat"]
fn canonical_odd_noaa_miter_source_edit_storage_cache_and_prepared_match_gimp() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/odd-miter-stroke-control");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let repeat: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("round2/observations.json")).unwrap()).unwrap();
    for (phase, edit, width, color, hash) in [
        ("source", false, 7f32, "9880803eb9b0303ff8e0e03e0000803f", "95a76e08ccedb0ceb06bbfbf6de9bf37a30252ecc66cbd377871cef854e9f7c3"),
        ("edit", true, 9f32, "9e80803ebdb0303ffee0e03e0000803f", "141a0498494475826f7e6a7f0a8bd91c15de08d283371b10e9de62e6c63d297a"),
    ] {
        let raw = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        assert_eq!(Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect::<String>(), hash);
        let (mut doc, mut item) = stroked_rectangle_example(edit);
        let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
        let V::Config(stroke) = &mut configuration.iter_mut().find(|a| a.name == "stroke").unwrap().value else { panic!() };
        stroke.iter_mut().find(|a| a.name == "width").unwrap().value = V::Number(N::Float32Bits(width.to_bits()));
        let mut f = fill(color);
        let V::Config(config) = &mut f else { panic!() };
        config.iter_mut().find(|a| a.name == "antialias").unwrap().value = V::Bool(false);
        stroke.iter_mut().find(|a| a.name == "fill").unwrap().value = f;
        doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
        let before = doc.clone();
        let state = &observations["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        let second = &repeat["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        let expected_bounds = if edit { Rect::new(7, 6, 52, 39) } else { Rect::new(6, 8, 49, 39) };
        let rendered = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        assert_eq!(rendered.bounds, expected_bounds);
        let native = std::fs::read(base.join(state["layer"]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(native, std::fs::read(base.join("round2").join(second["layer"]["native"]["file"].as_str().unwrap())).unwrap());
        assert_eq!(rendered.surface.try_read_interleaved(expected_bounds).unwrap(), native);
        let canvas = Rect::new(0, 0, 64, 48);
        let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
        assert_eq!(rendered.surface.try_read_interleaved(canvas).unwrap(), expected);
        let output = PreparedRenderer::new(&doc).unwrap().render(canvas, &mut |_| Ok(())).unwrap().try_display().unwrap();
        let max = output.px.iter().flatten().zip(&expected).map(|(a, b)| (a - f32::from(*b) / 255.).abs()).fold(0f32, f32::max);
        assert!(max <= 1. / 255., "odd stroke projected display max{max}");
        assert_eq!(doc, before);
        assert!(render_source(&doc, &item, &Interrupt::cancel_only(&|| true), &mut |_| Ok(()), &mut |_| Ok(())).is_err());
        assert!(
            render_source(&doc, &item, &Interrupt::NONE, &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("deny")), &mut |_| Ok(()))
                .is_err()
        );
    }
}

fn antialiased_stroked_rectangle_example(edit: bool) -> (Document, SourceItem) {
    let (mut doc, mut item) = stroked_rectangle_example(edit);
    let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
    let V::Config(stroke) = &mut configuration.iter_mut().find(|a| a.name == "stroke").unwrap().value else { panic!() };
    stroke.iter_mut().find(|a| a.name == "width").unwrap().value = V::Number(N::Float32Bits(if edit { 5f32.to_bits() } else { 3f32.to_bits() }));
    stroke[0] = arg("fill", fill(if edit { "8d80803eb5b0303fece0e03e0000803f" } else { "8780803eb4b0303fe6e0e03e0000803f" }));
    doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
    (doc, item)
}
#[test]
fn current_aa_stroke_storage_extent_coverage_geometry_and_budget() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use photocraft_compose::gimp_vector_items::qualify_source;
    let (doc, item) = antialiased_stroked_rectangle_example(false);
    let cache = item.cache.clone();
    assert_eq!(qualify_source(&doc, &item, &Interrupt::NONE).unwrap(), Rect::new(8, 10, 47, 37));
    let output = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    let alpha = |x, y| output.surface.try_read_interleaved(Rect::new(x, y, x + 1, y + 1)).unwrap()[3];
    // Captured independent stock native values, including half-unit corner subtraction.
    assert_eq!([alpha(8, 10), alpha(9, 10), alpha(8, 11), alpha(47, 11)], [60, 119, 128, 0]);
    assert_eq!(item.cache, cache);
    let prepared = PreparedRenderer::new(&doc).unwrap();
    assert!(prepared.render(Rect::new(0, 0, 64, 48), &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("test denial"))).is_err());
    assert!(render_source(&doc, &item, &Interrupt::cancel_only(&|| true), &mut |_| Ok(()), &mut |_| Ok(())).is_err());
    let (edit, changed) = antialiased_stroked_rectangle_example(true);
    assert_eq!(qualify_source(&edit, &changed, &Interrupt::NONE).unwrap(), Rect::new(9, 8, 50, 37));
    assert_ne!(doc.paths, edit.paths);
    assert_eq!(changed.cache, cache);
}

#[test]
#[ignore = "requires own canonical solid stroke source/edit and fresh native/PNG witnesses"]
fn canonical_aa_solid_stroke_matches_independent_stock_source_edit() {
    verify_aa_stroke_pair(
        "aa-stroke-control",
        ["86f8cbba43879965a77e1218361d9f98ba48cf82e5282924d8f3ce2796844db0", "42e0650abb63c875bbc720b79f5f577fe7a08d6bb642abba36215059d616ca06"],
        [3., 5.],
    );
}
#[test]
#[ignore = "requires own even AA solid stroke source/edit and fresh native/PNG witnesses"]
fn canonical_aa_even_solid_stroke_matches_independent_stock_source_edit() {
    verify_aa_stroke_pair(
        "aa-even-stroke-control",
        ["559ddb3e3ef94b680936cbd67176e64688ae90bb7aac19d63ce09579be4bf852", "1b020d836fa6e57133f9738730893a882552c80bd9047fb1ecb2ae86a015dec9"],
        [2., 4.],
    );
}
fn verify_aa_stroke_pair(directory: &str, hashes: [&str; 2], widths: [f32; 2]) {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa").join(directory);
    for (phase, edit, hash) in [("source", false, hashes[0]), ("edit", true, hashes[1])] {
        let raw = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        assert_eq!(Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect::<String>(), hash);
        let (mut doc, mut item) = antialiased_stroked_rectangle_example(edit);
        let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
        let V::Config(stroke) = &mut configuration.iter_mut().find(|a| a.name == "stroke").unwrap().value else { panic!() };
        stroke.iter_mut().find(|a| a.name == "width").unwrap().value = V::Number(N::Float32Bits(widths[usize::from(edit)].to_bits()));
        doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
        let cache = item.cache.clone();
        let surface = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap().surface;
        let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
        assert_eq!(surface.try_read_interleaved(Rect::new(0, 0, 64, 48)).unwrap(), expected);
        assert_eq!(item.cache, cache);
        let output = PreparedRenderer::new(&doc).unwrap().render(Rect::new(0, 0, 64, 48), &mut |_| Ok(())).unwrap().try_display().unwrap();
        let max = output.px.iter().flatten().zip(&expected).map(|(a, b)| (a - f32::from(*b) / 255.).abs()).fold(0f32, f32::max);
        assert!(max <= 1. / 255., "independent stroke projected display max{max}");
    }
}

#[test]
#[ignore = "requires original bevel source/edit and reversed negative-origin holdout"]
fn canonical_noaa_bevel_matches_stock_source_edit_and_negative_holdout() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use sha2::{Digest, Sha256};
    for (directory, hashes, negative) in [
        (
            "bevel-stroke-control",
            ["c4c3504301234f459645034f92f055b5e6da2e906fc413577a14257af756dff7", "7f8e1341afc124baa7c3c50d2b59197a411636878772d16a41c651278fe75fa4"],
            false,
        ),
        (
            "bevel-reversed-negative-control",
            ["2f3ea5ad1bd239305a39a5d458b70734f42291e65d00572bd02d417371572d75", "9733d834d99b51e4b3c4e2454041587b84c9f56acba87d75ca5d4db2cf044178"],
            true,
        ),
    ] {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa").join(directory);
        for (phase, edit, hash) in [("source", false, hashes[0]), ("edit", true, hashes[1])] {
            let raw = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
            assert_eq!(Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect::<String>(), hash);
            let (mut doc, mut item) = stroked_rectangle_example(edit);
            if negative {
                for k in &mut doc.paths[0].path.subpaths[0].knots {
                    k.anchor.x -= 20.;
                    k.anchor.y -= 20.;
                    k.in_ctrl = k.anchor;
                    k.out_ctrl = k.anchor;
                }
                doc.paths[0].path.subpaths[0].knots.reverse();
            }
            let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
            let V::Config(stroke) = &mut configuration.iter_mut().find(|a| a.name == "stroke").unwrap().value else { panic!() };
            stroke.iter_mut().find(|a| a.name == "join-style").unwrap().value = uint(2);
            doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
            let cache = item.cache.clone();
            let generated = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
            let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
            assert_eq!(generated.surface.try_read_interleaved(Rect::new(0, 0, 64, 48)).unwrap(), expected);
            let prepared = PreparedRenderer::new(&doc).unwrap().render(Rect::new(0, 0, 64, 48), &mut |_| Ok(())).unwrap().try_display().unwrap();
            let max = prepared.px.iter().flatten().zip(&expected).map(|(a, b)| (a - f32::from(*b) / 255.).abs()).fold(0f32, f32::max);
            assert!(max <= 1. / 255., "bevel independent projected display max{max}");
            assert_eq!(item.cache, cache);
        }
    }
}
#[test]
#[ignore = "requires original round source/edit and reversed negative-origin holdout"]
fn canonical_noaa_round_matches_stock_source_edit_and_negative_holdout() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use sha2::{Digest, Sha256};
    for (directory, hashes, negative) in [
        (
            "round-stroke-control",
            ["698b64785218f4f1cf7c3a9c94e0317debaec723b9a0b51256d5074eb156cd47", "72b6bb5ffbe5fd648a925ac75b6f4ee76a26936548ce33117e559255b90b6976"],
            false,
        ),
        (
            "round-reversed-negative-control",
            ["4794075aeb13e228a9583840137506f207b9ba272e449812f12bf071bfbb2a1c", "8a0e47222498a317349de9674d42a28073bbb8768f02afacb93d22bedc702ffa"],
            true,
        ),
    ] {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa").join(directory);
        for (phase, edit, hash) in [("source", false, hashes[0]), ("edit", true, hashes[1])] {
            let raw = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
            assert_eq!(Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect::<String>(), hash);
            let (mut doc, mut item) = stroked_rectangle_example(edit);
            if negative {
                for k in &mut doc.paths[0].path.subpaths[0].knots {
                    k.anchor.x -= 20.;
                    k.anchor.y -= 20.;
                    k.in_ctrl = k.anchor;
                    k.out_ctrl = k.anchor;
                }
                doc.paths[0].path.subpaths[0].knots.reverse();
            }
            let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
            let V::Config(stroke) = &mut configuration.iter_mut().find(|a| a.name == "stroke").unwrap().value else { panic!() };
            stroke.iter_mut().find(|a| a.name == "join-style").unwrap().value = uint(1);
            doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
            let cache = item.cache.clone();
            let generated = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
            let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
            assert_eq!(generated.surface.try_read_interleaved(Rect::new(0, 0, 64, 48)).unwrap(), expected);
            let prepared = PreparedRenderer::new(&doc).unwrap().render(Rect::new(0, 0, 64, 48), &mut |_| Ok(())).unwrap().try_display().unwrap();
            let max = prepared.px.iter().flatten().zip(&expected).map(|(a, b)| (a - f32::from(*b) / 255.).abs()).fold(0f32, f32::max);
            assert!(max <= 1. / 255., "round independent projected display max{max}");
            assert_eq!(item.cache, cache);
        }
    }
}

#[test]
#[ignore = "requires original AA widths one/seven source/edit and fresh native/PNG witnesses"]
fn canonical_aa_extended_odd_stroke_matches_independent_stock_source_edit() {
    verify_aa_stroke_pair(
        "aa-extended-odd-stroke-control",
        ["06409365fd48d645b06533d86eea11f649bdc12f4927872e09386a587374c8b2", "3375f8c9c606206c14c9a6f028dbd02b608ee8d4e90bcbb27bc0ca379b45121a"],
        [1., 7.],
    );
}

#[test]
#[ignore = "requires own odd-width miter original source/edit and independent repeat"]
fn canonical_odd_noaa_negative_source_edit_storage_cache_and_prepared_match_gimp() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/odd-miter-negative-control");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let repeat: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("round2/observations.json")).unwrap()).unwrap();
    for (phase, edit, width, color, hash) in [
        ("source", false, 7f32, "9e80803ebdb0303ffee0e03e0000803f", "08dd7dc3d941ab569f7f43b7602a89130da9de5d69fea15d34ee7d6b14b72994"),
        ("edit", true, 9f32, "a280803ebeb0303f05e1e03e0000803f", "07c87a4da83e94b7a205e9d23c4907bb4396a164e99bc29c001a22e010ce39e7"),
    ] {
        let raw = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        assert_eq!(Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect::<String>(), hash);
        let (mut doc, mut item) = stroked_rectangle_example(edit);
        for knot in &mut doc.paths[0].path.subpaths[0].knots {
            knot.anchor.x -= 20.;
            knot.anchor.y -= 20.;
            knot.in_ctrl = knot.anchor;
            knot.out_ctrl = knot.anchor;
        }
        doc.paths[0].path.subpaths[0].knots.reverse();
        let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
        let V::Config(stroke) = &mut configuration.iter_mut().find(|a| a.name == "stroke").unwrap().value else { panic!() };
        stroke.iter_mut().find(|a| a.name == "width").unwrap().value = V::Number(N::Float32Bits(width.to_bits()));
        let mut f = fill(color);
        let V::Config(config) = &mut f else { panic!() };
        config.iter_mut().find(|a| a.name == "antialias").unwrap().value = V::Bool(false);
        stroke.iter_mut().find(|a| a.name == "fill").unwrap().value = f;
        doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
        let before = doc.clone();
        let state = &observations["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        let second = &repeat["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        let expected_bounds = if edit { Rect::new(-12, -13, 33, 20) } else { Rect::new(-13, -11, 30, 20) };
        let rendered = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        assert_eq!(rendered.bounds, expected_bounds);
        let native = std::fs::read(base.join(state["layer"]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(native, std::fs::read(base.join("round2").join(second["layer"]["native"]["file"].as_str().unwrap())).unwrap());
        assert_eq!(rendered.surface.try_read_interleaved(expected_bounds).unwrap(), native);
        let canvas = Rect::new(0, 0, 64, 48);
        let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
        assert_eq!(rendered.surface.try_read_interleaved(canvas).unwrap(), expected);
        let output = PreparedRenderer::new(&doc).unwrap().render(canvas, &mut |_| Ok(())).unwrap().try_display().unwrap();
        let max = output.px.iter().flatten().zip(&expected).map(|(a, b)| (a - f32::from(*b) / 255.).abs()).fold(0f32, f32::max);
        assert!(max <= 1. / 255., "odd stroke projected display max{max}");
        assert_eq!(doc, before);
        assert!(render_source(&doc, &item, &Interrupt::cancel_only(&|| true), &mut |_| Ok(()), &mut |_| Ok(())).is_err());
        assert!(
            render_source(&doc, &item, &Interrupt::NONE, &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("deny")), &mut |_| Ok(()))
                .is_err()
        );
    }
}
#[test]
fn odd_width_domains_reject_unmeasured_style_and_preserve_state_on_denial() {
    for (aa, width, join) in [(false, 7f32, 1), (false, 9f32, 2), (false, 11f32, 0), (true, 9f32, 0)] {
        let (mut doc, mut item) = if aa { antialiased_stroked_rectangle_example(false) } else { stroked_rectangle_example(false) };
        let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
        let V::Config(stroke) = &mut configuration.iter_mut().find(|a| a.name == "stroke").unwrap().value else { panic!() };
        stroke.iter_mut().find(|a| a.name == "width").unwrap().value = V::Number(N::Float32Bits(width.to_bits()));
        stroke.iter_mut().find(|a| a.name == "join-style").unwrap().value = uint(join);
        doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
        let before = doc.clone();
        assert!(render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).is_err());
        assert_eq!(doc, before);
    }
    for width in [1f32, 7f32] {
        let (mut doc, mut item) = antialiased_stroked_rectangle_example(false);
        let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
        let V::Config(stroke) = &mut configuration.iter_mut().find(|a| a.name == "stroke").unwrap().value else { panic!() };
        stroke.iter_mut().find(|a| a.name == "width").unwrap().value = V::Number(N::Float32Bits(width.to_bits()));
        doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
        let before = doc.clone();
        assert!(render_source(&doc, &item, &Interrupt::cancel_only(&|| true), &mut |_| Ok(()), &mut |_| Ok(())).is_err());
        assert!(
            render_source(&doc, &item, &Interrupt::NONE, &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("deny")), &mut |_| Ok(()))
                .is_err()
        );
        assert_eq!(doc, before);
    }
}
#[test]
#[ignore = "requires independently authored horizontal/vertical open Butt source/edit native evidence"]
fn canonical_open_axis_source_edit_current_geometry_and_prepared_match_stock() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use sha2::{Digest, Sha256};
    for (axis, hashes) in [
        (
            "horizontal-negative",
            ["eba4f544b85a6cff8c5bcca7bf5301385fd5f8982633e035686ffd5b32cef779", "805cac9eeb452475576899d42510d281a4e0dd4ac0e27c106967130eec066717"],
        ),
        (
            "horizontal",
            ["476bdd82d260b5db84b4a81e14f79ffa05b9f46b4dce119eb1ddad0678bdd2a5", "5b452db619638bad90de04da7773322ab81713ea8a1ef95e2a557d568871b256"],
        ),
        ("vertical", ["c5fb3f6e217b7c1bd79e5b3f04ec60f066c6a30230f9fe4c3c52aba26f809ac0", "be2823fc132218511e98f51bd78e3d6468429f7d95ff4654cdbac98bea2129f1"]),
    ] {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa").join(format!("open-axis-{axis}"));
        for edit in [false, true] {
            let phase = if edit { "edit" } else { "source" };
            let raw = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
            assert_eq!(Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect::<String>(), hashes[usize::from(edit)]);
            let (mut doc, item) = stroked_rectangle_example(edit);
            let coordinates = if axis == "horizontal-negative" {
                [(26., 4.), (-10., 4.)]
            } else if axis == "horizontal" {
                [(10., 24.), (46., 24.)]
            } else {
                [(28., 8.), (28., 40.)]
            };
            let mut sub = photocraft_doc::Subpath::polygon(&coordinates);
            sub.closed = false;
            if edit {
                for k in &mut sub.knots {
                    k.anchor.x += 2.;
                    k.anchor.y -= 1.;
                    k.in_ctrl = k.anchor;
                    k.out_ctrl = k.anchor;
                }
            }
            doc.paths[0].path.subpaths = vec![sub];
            let before = doc.clone();
            let generated = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
            let expected_bounds = match (axis, edit) {
                ("horizontal-negative", false) => Rect::new(-13, 1, 29, 7),
                ("horizontal-negative", true) => Rect::new(-12, -1, 32, 7),
                ("horizontal", false) => Rect::new(7, 21, 49, 27),
                ("horizontal", true) => Rect::new(8, 19, 52, 27),
                ("vertical", false) => Rect::new(25, 5, 31, 43),
                _ => Rect::new(26, 3, 34, 43),
            };
            assert_eq!(generated.bounds, expected_bounds);
            let native = std::fs::read(base.join(format!("{phase}-reopened.layer.native.bin"))).unwrap();
            assert_eq!(native, std::fs::read(base.join(format!("round2/{phase}-reopened.layer.native.bin"))).unwrap());
            assert_eq!(generated.surface.try_read_interleaved(expected_bounds).unwrap(), native);
            let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
            assert_eq!(generated.surface.try_read_interleaved(Rect::new(0, 0, 64, 48)).unwrap(), expected);
            let output = PreparedRenderer::new(&doc).unwrap().render(doc.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
            let max = output.px.iter().flatten().zip(&expected).map(|(a, b)| (a - f32::from(*b) / 255.).abs()).fold(0f32, f32::max);
            assert!(max <= 1. / 255.);
            assert_eq!(doc, before);
        }
    }
}

#[test]
#[ignore = "requires independently authored horizontal/vertical open Butt source/edit native evidence"]
fn canonical_square_axis_current_geometry_and_prepared_match_stock() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    use photocraft_doc::xcf_items::EditableSource;
    use sha2::{Digest, Sha256};
    for (axis, hashes) in [
        (
            "horizontal",
            ["e4b8808929c71653523910104ca5194d37046b4ebc52178a20fdb72c613f3e53", "08ae5b7a6fb4c754f3fbe8093776da6509de8faf9fa8ba1f80d2de65a8fa8f2a"],
        ),
        ("vertical", ["fc26c49a8a93d192b1c030e66849bce97d16a1ac55cb3cddd2e31777ac95a027", "add286d2bf2dc249453a0e67e240ee7415339424912acec2b967f64f3c96f221"]),
        (
            "horizontal-negative",
            ["58101cb5d06a8764cffd12d39ef7bc1af9929bdd559f11f649214093b596ee38", "352aac814b438b648eee819fd783f47fee941c076ec42039fab138293865f909"],
        ),
    ] {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa").join(format!("square-axis-{axis}"));
        for edit in [false, true] {
            let phase = if edit { "edit" } else { "source" };
            let raw = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
            assert_eq!(Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect::<String>(), hashes[usize::from(edit)]);
            let (mut doc, item) = stroked_rectangle_example(edit);
            let coordinates = if axis == "horizontal-negative" {
                [(26., 4.), (-10., 4.)]
            } else if axis == "horizontal" {
                [(10., 24.), (46., 24.)]
            } else {
                [(28., 8.), (28., 40.)]
            };
            let mut sub = photocraft_doc::Subpath::polygon(&coordinates);
            sub.closed = false;
            if edit {
                for k in &mut sub.knots {
                    k.anchor.x += 2.;
                    k.anchor.y -= 1.;
                    k.in_ctrl = k.anchor;
                    k.out_ctrl = k.anchor;
                }
            }
            doc.paths[0].path.subpaths = vec![sub];
            let mut item = item;
            let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!("vector") };
            let FilterValue::Config(stroke) = &mut configuration.iter_mut().find(|a| a.name == "stroke").unwrap().value else { panic!("stroke") };
            stroke.iter_mut().find(|a| a.name == "cap-style").unwrap().value = FilterValue::Number(ExactNumber::Unsigned { width: 32, value: 2 });
            doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
            let before = doc.clone();
            let generated = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
            let expected_bounds = match (axis, edit) {
                ("horizontal-negative", false) => Rect::new(-13, 1, 29, 7),
                ("horizontal-negative", true) => Rect::new(-12, -1, 32, 7),
                ("horizontal", false) => Rect::new(7, 21, 49, 27),
                ("horizontal", true) => Rect::new(8, 19, 52, 27),
                ("vertical", false) => Rect::new(25, 5, 31, 43),
                _ => Rect::new(26, 3, 34, 43),
            };
            assert_eq!(generated.bounds, expected_bounds);
            let native = std::fs::read(base.join(format!("{phase}-reopened.layer.native.bin"))).unwrap();
            assert_eq!(native, std::fs::read(base.join(format!("round2/{phase}-reopened.layer.native.bin"))).unwrap());
            assert_eq!(generated.surface.try_read_interleaved(expected_bounds).unwrap(), native);
            let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
            assert_eq!(generated.surface.try_read_interleaved(Rect::new(0, 0, 64, 48)).unwrap(), expected);
            let output = PreparedRenderer::new(&doc).unwrap().render(doc.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
            let max = output.px.iter().flatten().zip(&expected).map(|(a, b)| (a - f32::from(*b) / 255.).abs()).fold(0f32, f32::max);
            assert!(max <= 1. / 255.);
            assert_eq!(doc, before);
        }
    }
}

#[test]
#[ignore = "requires independently authored horizontal/vertical open Butt source/edit native evidence"]
fn canonical_round_axis_current_geometry_and_prepared_match_stock() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    use photocraft_doc::xcf_items::EditableSource;
    use sha2::{Digest, Sha256};
    for (axis, hashes) in [
        (
            "horizontal",
            ["82c0f1a985d771bed6dbb10f37ec612e59e266c9fdeb7787eb212229ba7f259d", "d8eacc9d7d9b5bcb245bbcaca24d7479b3729fbd7600dfdddd0f8c62dc85c680"],
        ),
        ("vertical", ["2d9e632fa527fada80b3bc142d28b6ba204499a016088fb54eff81745491b832", "849bd180f564843f8c2872388dbd6f5e4d6cfb134b3ea2a06a9dee874c0de126"]),
        (
            "horizontal-negative",
            ["210f4b96c53aa6a8500d797a7a3d86c78147811019b647a36a0f9c478b29b994", "5cc5bd70200bcd35acd833552aa9ebeb70bcaef588600ad21a2b9dce3e12e768"],
        ),
    ] {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa").join(format!("round-axis-{axis}"));
        for edit in [false, true] {
            let phase = if edit { "edit" } else { "source" };
            let raw = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
            assert_eq!(Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect::<String>(), hashes[usize::from(edit)]);
            let (mut doc, item) = stroked_rectangle_example(edit);
            let coordinates = if axis == "horizontal-negative" {
                [(26., 4.), (-10., 4.)]
            } else if axis == "horizontal" {
                [(10., 24.), (46., 24.)]
            } else {
                [(28., 8.), (28., 40.)]
            };
            let mut sub = photocraft_doc::Subpath::polygon(&coordinates);
            sub.closed = false;
            if edit {
                for k in &mut sub.knots {
                    k.anchor.x += 2.;
                    k.anchor.y -= 1.;
                    k.in_ctrl = k.anchor;
                    k.out_ctrl = k.anchor;
                }
            }
            doc.paths[0].path.subpaths = vec![sub];
            let mut item = item;
            let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!("vector") };
            let FilterValue::Config(stroke) = &mut configuration.iter_mut().find(|a| a.name == "stroke").unwrap().value else { panic!("stroke") };
            stroke.iter_mut().find(|a| a.name == "cap-style").unwrap().value = FilterValue::Number(ExactNumber::Unsigned { width: 32, value: 1 });
            doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
            let before = doc.clone();
            let generated = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
            let expected_bounds = match (axis, edit) {
                ("horizontal-negative", false) => Rect::new(-13, 1, 29, 7),
                ("horizontal-negative", true) => Rect::new(-12, -1, 32, 7),
                ("horizontal", false) => Rect::new(7, 21, 49, 27),
                ("horizontal", true) => Rect::new(8, 19, 52, 27),
                ("vertical", false) => Rect::new(25, 5, 31, 43),
                _ => Rect::new(26, 3, 34, 43),
            };
            assert_eq!(generated.bounds, expected_bounds);
            let native = std::fs::read(base.join(format!("{phase}-reopened.layer.native.bin"))).unwrap();
            assert_eq!(native, std::fs::read(base.join(format!("round2/{phase}-reopened.layer.native.bin"))).unwrap());
            assert_eq!(generated.surface.try_read_interleaved(expected_bounds).unwrap(), native);
            let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
            assert_eq!(generated.surface.try_read_interleaved(Rect::new(0, 0, 64, 48)).unwrap(), expected);
            let output = PreparedRenderer::new(&doc).unwrap().render(doc.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
            let max = output.px.iter().flatten().zip(&expected).map(|(a, b)| (a - f32::from(*b) / 255.).abs()).fold(0f32, f32::max);
            assert!(max <= 1. / 255.);
            assert_eq!(doc, before);
        }
    }
}

#[test]
#[ignore = "requires independent odd Round body/cap boundary and reversed negative native witnesses"]
fn canonical_odd_round_body_union_ignores_cache_and_matches_independent_refs() {
    use photocraft_compose::gimp_render::PreparedRenderer;
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    use sha2::{Digest, Sha256};
    for (directory, coordinates, hashes) in [
        (
            "odd-collinear-round-axis-horizontal",
            vec![(10.0, 24.0), (28.0, 24.0), (46.0, 24.0)],
            ["b43986c04d6728581beeff1c2927ab04ad82dad2602e2d963b77a7006efc7a8d", "8a4ecb857aa0a66dd89e4316b1dbdac3c9cd50427a71b9568444da19e7ec3a88"],
        ),
        (
            "odd-collinear-round-axis-vertical",
            vec![(28.0, 8.0), (28.0, 16.0), (28.0, 32.0), (28.0, 40.0)],
            ["d91ba628a74180e311c8da83394336c4cd7b958845c5f86d5d94fa5dbd7132c9", "8c87eea154373aa64e81a891689ca0dc52d45b14c3eb5e11449fb0a4b0586e9d"],
        ),
        (
            "odd-round-holdout-horizontal",
            vec![(26.0, 4.0), (8.0, 4.0), (-10.0, 4.0)],
            ["5294e6574dd619b49866a6f63c6a2d9d74c6809086f40308b57c912ce156eec8", "6d0066b9900c9a7d5b8266f7504987f0bc5fbd2b5bd3a4d57423c66356efcbf8"],
        ),
        (
            "odd-round-holdout-vertical",
            vec![(8.0, 20.0), (8.0, 12.0), (8.0, 0.0), (8.0, -12.0)],
            ["8361ce78c147e640eb060f43bb5be0fb768d6d12697665d11d96103858461150", "e3ad108c81dca7128853bb99130f1176dae26bfea8715d2b3832bc5f724dd3ca"],
        ),
    ] {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa").join(directory);
        for edit in [false, true] {
            let phase = if edit { "edit" } else { "source" };
            let raw = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
            assert_eq!(Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect::<String>(), hashes[usize::from(edit)]);
            let (mut doc, mut item) = stroked_rectangle_example(edit);
            let mut sub = photocraft_doc::Subpath::polygon(&coordinates);
            sub.closed = false;
            if edit {
                for k in &mut sub.knots {
                    k.anchor.x += 2.;
                    k.anchor.y -= 1.;
                    k.in_ctrl = k.anchor;
                    k.out_ctrl = k.anchor;
                }
            }
            doc.paths[0].path.subpaths = vec![sub];
            let EditableSource::Vector { configuration, .. } = &mut item.source else { panic!() };
            let FilterValue::Config(stroke) = &mut configuration.iter_mut().find(|a| a.name == "stroke").unwrap().value else { panic!() };
            stroke.iter_mut().find(|a| a.name == "cap-style").unwrap().value = FilterValue::Number(ExactNumber::Unsigned { width: 32, value: 1 });
            stroke.iter_mut().find(|a| a.name == "width").unwrap().value =
                FilterValue::Number(ExactNumber::Float32Bits(if edit { 5f32 } else { 3f32 }.to_bits()));
            doc.xcf_state.as_mut().unwrap().source_items[0] = item.clone();
            let before = doc.clone();
            let surface = render_source(&doc, &item, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
            let expected = std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap();
            assert_eq!(surface.surface.try_read_interleaved(doc.bounds()).unwrap(), expected);
            let output = PreparedRenderer::new(&doc).unwrap().render(doc.bounds(), &mut |_| Ok(())).unwrap().try_display().unwrap();
            let max = output.px.iter().flatten().zip(&expected).map(|(a, b)| (a - f32::from(*b) / 255.).abs()).fold(0f32, f32::max);
            assert!(max <= 1. / 255.);
            assert_eq!(doc, before);
        }
    }
}
