use photocraft_doc::{FillRule, Knot, Path, PathOp, PixelFormat, Subpath};
use photocraft_geom::{Point, Rect};
use photocraft_raster::Interrupt;
use photocraft_vector::xcf::{SolidSettings, SolidStroke, VectorError, render_solid, render_solid_with_transform};

#[test]
fn checked_solid_rejects_bad_geometry_color_budget_and_cancel() {
    let mut path = Path { subpaths: vec![Subpath::polygon(&[(0., 0.), (4., 0.), (4., 4.), (0., 4.)])], fill_rule: FillRule::NonZero, inverted: false };
    let settings = SolidSettings { fill: Some([1., 0., 0., 1.]), stroke: None, antialias: true };
    let render =
        |p: &Path, s: &SolidSettings| render_solid(p, s, Rect::new(0, 0, 4, 4), PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(()));
    assert!(render(&path, &settings).is_ok());
    path.subpaths[0].knots[0].anchor.x = f64::NAN;
    assert!(render(&path, &settings).is_err());
    path.subpaths[0].knots[0].anchor.x = 1e20;
    assert!(render(&path, &settings).is_err());
    path.subpaths[0].knots[0].anchor.x = 0.;
    assert_eq!(
        render_solid(&path, &settings, Rect::new(0, 0, 4, 4), PixelFormat::RGBA8, &Interrupt::cancel_only(&|| true), &mut |_| Ok(()), &mut |_| Ok(()))
            .unwrap_err(),
        VectorError::Cancelled
    );
    assert_eq!(
        render_solid(&path, &settings, Rect::new(0, 0, 4, 4), PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Err(VectorError::Budget), &mut |_| Ok(()))
            .unwrap_err(),
        VectorError::Budget
    );
    assert!(render(&path, &SolidSettings { fill: Some([f64::INFINITY, 0., 0., 1.]), ..settings }).is_err());
}
#[test]
#[ignore = "requires reviewed vector source and independently reopened native planes"]
fn originals_regenerate_from_saved_geometry_and_color_not_modified_cache() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
    for (family, suffix) in [("vector-basic", ""), ("vector-basic", ".edit"), ("vector-modified", ""), ("vector-modified", ".edit")] {
        let file = photocraft_xcf::decode(&std::fs::read(base.join(format!("{family}{suffix}.xcf"))).unwrap(), &Default::default()).unwrap();
        let layer = file.layers.iter().find(|layer| layer.vector.is_some()).unwrap();
        let vector = layer.vector.as_ref().unwrap();
        println!("{suffix} vector={vector:?}");
        let source = file.paths.iter().find(|path| path.tattoo == Some(vector.path_tattoo)).unwrap();
        let path = Path {
            subpaths: source
                .strokes
                .iter()
                .map(|stroke| Subpath {
                    closed: stroke.closed,
                    op: PathOp::Combine,
                    knots: stroke
                        .points
                        .as_chunks::<3>().0.iter()
                        .map(|triple| {
                            let p = |index: usize| {
                                Point::new(f64::from(f32::from_bits(triple[index].components[0])), f64::from(f32::from_bits(triple[index].components[1])))
                            };
                            Knot { anchor: p(1), in_ctrl: p(0), out_ctrl: p(2), smooth: false }
                        })
                        .collect(),
                })
                .collect(),
            fill_rule: FillRule::NonZero,
            inverted: false,
        };
        let rgba = |fill: &photocraft_xcf::VectorFill| {
            let color = fill.color.as_ref().unwrap();
            assert!(color.icc.is_empty());
            assert_eq!(color.encoding, "R'G'B'A float");
            let numbers: Vec<f64> = color.samples.as_chunks::<4>().0.iter().map(|chunk| f64::from(f32::from_le_bytes(*chunk))).collect();
            <[f64; 4]>::try_from(numbers).unwrap()
        };
        println!("path={path:?}");
        let settings = SolidSettings {
            fill: vector.filled.then(|| rgba(&vector.fill)),
            stroke: vector.stroked.then(|| SolidStroke {
                color: rgba(&vector.stroke.fill),
                style: photocraft_vector::StrokeStyle {
                    width: f64::from(f32::from_bits(vector.stroke.width_bits)),
                    cap: photocraft_doc::LineCap::Butt,
                    join: photocraft_doc::LineJoin::Miter,
                    miter_limit: f64::from(f32::from_bits(vector.stroke.miter_limit_bits)),
                    dashes: vector
                        .stroke
                        .dashes_bits
                        .iter()
                        .map(|bits| f64::from(f32::from_bits(*bits)) * f64::from(f32::from_bits(vector.stroke.width_bits)))
                        .collect(),
                    dash_offset: 0.,
                },
            }),
            antialias: vector.fill.antialias,
        };
        let state: serde_json::Value = serde_json::from_slice(
            &std::fs::read(base.join(format!("{family}{}.json", if suffix.is_empty() { ".reopened_state" } else { ".edited_state" }))).unwrap(),
        )
        .unwrap();
        let expected = state["layers"].as_array().unwrap().iter().find(|row| row["type"] == "VectorLayer").unwrap();
        let x = expected["offset"][0].as_i64().unwrap() as i32;
        let y = expected["offset"][1].as_i64().unwrap() as i32;
        let w = expected["size"][0].as_i64().unwrap() as i32;
        let h = expected["size"][1].as_i64().unwrap() as i32;
        let surface = render_solid_with_transform(
            &path,
            &settings,
            Rect::new(x, y, x + w, y + h),
            PixelFormat::RGBA8,
            &Interrupt::NONE,
            &mut |_| Ok(()),
            &mut |_| Ok(()),
            &|rgb| Ok(rgb.map(|v| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) })),
            &|rgb| Ok(rgb.map(|v| if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1. / 2.4) - 0.055 })),
        )
        .unwrap();
        let native = std::fs::read(base.join(expected["native"]["file"].as_str().unwrap())).unwrap();
        println!(
            "serialized plane matches reopenednative: {} first stored={:?} firstreopened={:?}",
            layer.plane.as_ref().unwrap().bytes == native,
            layer.plane.as_ref().unwrap().bytes.get(..4),
            native.get(..4)
        );
        let actual = surface.read_region(Rect::new(x, y, x + w, y + h));
        let mut max = 0f32;
        for (index, (a, b)) in actual.as_chunks::<4>().0.iter().zip(native.as_chunks::<4>().0.iter()).enumerate() {
            for c in 0..4 {
                if c == 3 || a[3] > 0. && b[3] > 0 {
                    let delta = (a[c] * 255. - f32::from(b[c])).abs();
                    if delta > max {
                        max = delta;
                        println!("max pixel{index} channel{c}: {a:?} vs {b:?}");
                    }
                }
            }
        }
        println!("{suffix} regenerated max delta={max}/255");
        if family == "vector-modified" {
            assert!(max > 2., "modified raster must not be replaced by geometric regeneration");
            assert_eq!(layer.plane.as_ref().unwrap().bytes, native);
        } else {
            assert!(max <= 2., "native regeneration mismatch {max}/255");
        }
    }
}
#[test]
#[ignore = "local original metadata diagnostic"]
fn original_vector_geometry_audit() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
    for suffix in ["", ".edit"] {
        let f = photocraft_xcf::decode(&std::fs::read(base.join(format!("vector-modified{suffix}.xcf"))).unwrap(), &Default::default()).unwrap();
        for d in &f.layers {
            let raw = d.plane.as_ref().map(|p| &p.bytes);
            let pixel = raw.and_then(|p| p.get((2 * d.width as usize + 2) * 4..(2 * d.width as usize + 2) * 4 + 4));
            println!(
                "{suffix} {} offset{:?} size{}x{} path{:?} visible{} opacity{} mode{} tuple{:?} first{:?} local22{:?}",
                d.name,
                d.offset,
                d.width,
                d.height,
                d.path,
                d.visible,
                d.opacity,
                d.mode,
                (d.composite_mode, d.composite_space, d.blend_space),
                raw.and_then(|p| p.get(..4)),
                pixel
            );
        }
    }
}
#[test]
fn transforms_work_denial_and_mid_render_cancel_are_explicit() {
    use std::cell::Cell;
    let path = Path { subpaths: vec![Subpath::polygon(&[(0., 0.), (4., 0.), (4., 4.), (0., 4.)])], fill_rule: FillRule::NonZero, inverted: false };
    let settings = SolidSettings { fill: Some([0.25, 0.5, 0.75, 1.]), stroke: None, antialias: true };
    let bounds = Rect::new(0, 0, 4, 4);
    assert!(
        render_solid_with_transform(
            &path,
            &settings,
            bounds,
            PixelFormat::RGBA8,
            &Interrupt::NONE,
            &mut |_| Ok(()),
            &mut |_| Ok(()),
            &|_| Ok([f64::NAN, 0., 0.]),
            &|v| Ok(v)
        )
        .is_err()
    );
    assert!(
        render_solid_with_transform(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(()), &|v| Ok(v), &|_| Ok([
            0.,
            f64::INFINITY,
            0.
        ]))
        .is_err()
    );
    assert_eq!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Err(VectorError::Budget)).unwrap_err(),
        VectorError::Budget
    );
    let stop = std::sync::atomic::AtomicBool::new(false);
    let calls = Cell::new(0);
    let cancelled = || stop.load(std::sync::atomic::Ordering::Relaxed);
    let ctl = Interrupt::cancel_only(&cancelled);
    let mut work = |_| {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    };
    assert_eq!(render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &ctl, &mut |_| Ok(()), &mut work).unwrap_err(), VectorError::Cancelled);
}

#[test]
fn analytical_fill_coverage_is_independent_of_clip_and_integer_origin() {
    // The top boundary y = 14 + 5(x-12)/62 stays inside pixel (30,15).
    // Integrating the covered area below it over [30,31] gives 63/124.
    // This checks our mathematical coverage, not Cairo/GIMP's backend AA rule.
    let polygon = [(12., 14.), (74., 19.), (74., 70.), (12., 70.)];
    let make_path =
        |dx: f64, dy: f64| Path { subpaths: vec![Subpath::polygon(&polygon.map(|(x, y)| (x + dx, y + dy)))], fill_rule: FillRule::NonZero, inverted: false };
    let settings = SolidSettings { fill: Some([1., 1., 1., 1.]), stroke: None, antialias: true };
    let draw =
        |path: &Path, bounds: Rect| render_solid(path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    let path = make_path(0., 0.);
    let full = draw(&path, Rect::new(10, 12, 80, 71));
    let pixel = draw(&path, Rect::new(30, 15, 31, 16));
    let alpha = pixel.read_region(Rect::new(30, 15, 31, 16))[3];
    let expected = (63_f64 / 124. * 255.).round() as f32 / 255.;
    assert!((alpha - expected).abs() < 1e-6, "area alpha {alpha} vs {expected}");
    assert_eq!(full.read_region(Rect::new(30, 15, 31, 16)), pixel.read_region(Rect::new(30, 15, 31, 16)));
    let translated = draw(&make_path(-40., -30.), Rect::new(-30, -18, 40, 41));
    assert_eq!(full.read_region(Rect::new(10, 12, 80, 71)), translated.read_region(Rect::new(-30, -18, 40, 41)));
}

#[test]
fn cubic_rectangle_fill_uses_observed_stock_subrow_grid() {
    let path = Path { subpaths: vec![Subpath::polygon(&[(0.25, 0.5), (1.5, 0.5), (1.5, 1.75), (0.25, 1.75)])], fill_rule: FillRule::NonZero, inverted: false };
    let surface = render_solid(
        &path,
        &SolidSettings { fill: Some([0.2, 0.4, 0.6, 1.]), stroke: None, antialias: true },
        Rect::new(0, 0, 2, 2),
        PixelFormat::RGBA8,
        &Interrupt::NONE,
        &mut |_| Ok(()),
        &mut |_| Ok(()),
    )
    .unwrap();
    for (pixel, area) in
        surface.read_region(Rect::new(0, 0, 2, 2)).as_chunks::<4>().0.iter().zip([0.75_f64 * 7. / 15., 0.5 * 7. / 15., 0.75 * 11. / 15., 0.5 * 11. / 15.])
    {
        assert!((pixel[3] - (area * 255.).round() as f32 / 255.).abs() < 1e-6);
    }
}

#[test]
#[ignore = "local disabled-group source field audit"]
fn original_disabled_group_fields_audit() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/group-disabled-evidence");
    for name in ["source.disabled.xcf", "edit.disabled.xcf"] {
        let file = photocraft_xcf::decode(&std::fs::read(base.join(name)).unwrap(), &Default::default()).unwrap();
        println!("FILE {name}: version={} precision={}", file.version, file.precision);
        for layer in &file.layers {
            println!(
                "LAYER {} bounds={:?} {}x{} mode={} tuple=({},{},{}) opacity={} visible={} maskflags=({},{},{})",
                layer.name,
                layer.offset,
                layer.width,
                layer.height,
                layer.mode,
                layer.composite_mode,
                layer.composite_space,
                layer.blend_space,
                layer.opacity,
                layer.visible,
                layer.mask_enabled,
                layer.mask_shown,
                layer.mask_editing
            );
            for effect in &layer.effects {
                println!("EFFECT {} {:?} {:?} args={:?}", effect.operation, effect.version, effect.state, effect.arguments);
                println!("RAW {:?}", effect.properties.iter().filter(|p| p.tag != 45).map(|p| (p.tag, p.payload.as_slice())).collect::<Vec<_>>());
                if let Some(mask) = &effect.mask {
                    println!(
                        "MASK {:?} {}x{} tattoo={:?} visible={} legacy_linked={} flags=({},{},{}) sample={:?} nonzero={}",
                        mask.offset,
                        mask.width,
                        mask.height,
                        mask.tattoo,
                        mask.visible,
                        mask.legacy_linked,
                        mask.mask_enabled,
                        mask.mask_shown,
                        mask.mask_editing,
                        mask.plane.as_ref().map(|p| p.sample),
                        mask.plane.as_ref().is_some_and(|p| p.bytes.iter().any(|b| *b != 0))
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "requires owned public Cairo fractional holdout diagnostic packet"]
fn independent_cubic_rectangle_holdouts_match() {
    let packet = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/rectangle-holdout.json");
    let packet: serde_json::Value = serde_json::from_slice(&std::fs::read(packet).unwrap()).unwrap();
    let rows = packet["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 128);
    for row in rows {
        let b: Vec<f64> = row["bounds"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let path =
            Path { subpaths: vec![Subpath::polygon(&[(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])])], fill_rule: FillRule::NonZero, inverted: false };
        let draw = |path: &Path, bounds| {
            render_solid(
                path,
                &SolidSettings { fill: Some([1.; 4]), stroke: None, antialias: true },
                bounds,
                PixelFormat::RGBA8,
                &Interrupt::NONE,
                &mut |_| Ok(()),
                &mut |_| Ok(()),
            )
            .unwrap()
        };
        let pixels = draw(&path, Rect::new(0, 0, 4, 4)).read_region(Rect::new(0, 0, 4, 4));
        let expected = row["actual"].as_array().unwrap();
        for (pixel, expected) in pixels.as_chunks::<4>().0.iter().zip(expected) {
            assert_eq!((pixel[3] * 255.).round() as u64, expected.as_u64().unwrap(), "{b:?}");
        }
        let translated = Path {
            subpaths: vec![Subpath::polygon(&[(b[0] - 8., b[1] - 8.), (b[2] - 8., b[1] - 8.), (b[2] - 8., b[3] - 8.), (b[0] - 8., b[3] - 8.)])],
            ..path
        };
        assert_eq!(pixels, draw(&translated, Rect::new(-8, -8, -4, -4)).read_region(Rect::new(-8, -8, -4, -4)));
    }
}

#[test]
fn retraced_axis_segments_do_not_become_a_rectangle() {
    let path = Path { subpaths: vec![Subpath::polygon(&[(0., 0.), (1., 0.), (0., 0.), (0., 1.)])], fill_rule: FillRule::NonZero, inverted: false };
    let surface = render_solid(
        &path,
        &SolidSettings { fill: Some([1.; 4]), stroke: None, antialias: true },
        Rect::new(0, 0, 2, 2),
        PixelFormat::RGBA8,
        &Interrupt::NONE,
        &mut |_| Ok(()),
        &mut |_| Ok(()),
    )
    .unwrap();
    assert!(surface.read_region(Rect::new(0, 0, 2, 2)).as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0.));
}

#[test]
#[ignore = "requires owned GIMP rectangle source/edit evidence packet"]
fn original_rectangle_source_and_path_edit_regenerate_exact_native_and_projection() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/vector-rectangle-evidence");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let independent: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("round2/observations.json")).unwrap()).unwrap();
    let mut projections = Vec::new();
    for phase in ["source", "edit"] {
        let bytes = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        use sha2::{Digest, Sha256};
        let hash: String = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(
            hash,
            if phase == "source" {
                "534fc2bf3b95b2384768832e43a60cf52f42286b6e178fc1b8df3989dcfbe9bd"
            } else {
                "e67908d884bead724342fa3ed515a3784c3406a62ea8f75c681170c12bb139f7"
            }
        );
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!((file.version, file.precision), (24, 150));
        println!("{phase} version={} precision={} icc={:?}", file.version, file.precision, file.icc.len());
        let layer = file.layers.iter().find(|l| l.vector.is_some()).unwrap();
        let vector = layer.vector.as_ref().unwrap();
        println!("{phase} offset={:?} vector={vector:?}", layer.offset);
        assert!(!vector.source_modified);
        assert!(vector.filled);
        assert!(!vector.stroked);
        assert_eq!(vector.fill.style, 0);
        assert!(vector.fill.antialias);
        let path = file.paths.iter().find(|p| p.tattoo == Some(vector.path_tattoo)).unwrap();
        assert_eq!(path.strokes.len(), 1);
        let stroke = &path.strokes[0];
        assert!(stroke.closed);
        assert_eq!(stroke.points.len(), 12);
        let knots = stroke
            .points
            .as_chunks::<3>().0.iter()
            .map(|triple| {
                assert_eq!([triple[0].kind, triple[1].kind, triple[2].kind], [1, 0, 1]);
                let point = |p: &photocraft_xcf::PathPoint| Point::new(f64::from(f32::from_bits(p.components[0])), f64::from(f32::from_bits(p.components[1])));
                Knot { in_ctrl: point(&triple[0]), anchor: point(&triple[1]), out_ctrl: point(&triple[2]), smooth: false }
            })
            .collect();
        let path = Path { subpaths: vec![Subpath { knots, closed: true, op: PathOp::Combine }], fill_rule: FillRule::NonZero, inverted: false };
        let color = vector.fill.color.as_ref().unwrap();
        assert_eq!(color.encoding, "R'G'B'A float");
        assert!(color.icc.is_empty());
        let rgba: Vec<f64> = color.samples.as_chunks::<4>().0.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect();
        let settings = SolidSettings { fill: Some(rgba.try_into().unwrap()), stroke: None, antialias: true };
        let render = |bounds| render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        let state = &observations["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        let repeated = &independent["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        assert_eq!(state["layer"]["offset"], repeated["layer"]["offset"]);
        assert_eq!(layer.offset, (state["layer"]["offset"][0].as_i64().unwrap() as i32, state["layer"]["offset"][1].as_i64().unwrap() as i32));
        assert_eq!([layer.width, layer.height], [state["layer"]["size"][0].as_u64().unwrap() as u32, state["layer"]["size"][1].as_u64().unwrap() as u32]);
        let bounds = Rect::new(layer.offset.0, layer.offset.1, layer.offset.0 + layer.width as i32, layer.offset.1 + layer.height as i32);
        let native = std::fs::read(base.join(state["layer"]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(layer.plane.as_ref().unwrap().bytes, native);
        assert_eq!(render(bounds).try_read_interleaved(bounds).unwrap(), native);
        assert_eq!(native, std::fs::read(base.join("round2").join(repeated["layer"]["native"]["file"].as_str().unwrap())).unwrap());
        let canvas = Rect::new(0, 0, 64, 48);
        let projected = render(canvas).try_read_interleaved(canvas).unwrap();
        assert_eq!(projected, std::fs::read(base.join(state["projection"]["native"]["file"].as_str().unwrap())).unwrap());
        assert_eq!(projected, std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap());
        assert_eq!(projected, std::fs::read(base.join("round2").join(repeated["projection"]["native"]["file"].as_str().unwrap())).unwrap());
        projections.push(projected);
    }
    assert_ne!(projections[0], projections[1], "public path translation edit must change canvas projection");
}

#[test]
fn resident_budget_counts_partial_destination_tiles() {
    let path = Path { subpaths: vec![Subpath::polygon(&[(0., 0.), (1., 0.), (1., 4.), (0., 4.)])], fill_rule: FillRule::NonZero, inverted: false };
    let mut requested = 0;
    let result = render_solid(
        &path,
        &SolidSettings { fill: Some([1.; 4]), stroke: None, antialias: true },
        Rect::new(0, 0, 1, 4096),
        PixelFormat::RGBA8,
        &Interrupt::NONE,
        &mut |n| {
            requested += n;
            if requested > 2200000 { Err(VectorError::Budget) } else { Ok(()) }
        },
        &mut |_| Ok(()),
    );
    assert_eq!(result.unwrap_err(), VectorError::Budget);
    assert!(requested >= 16 * 256 * 256 * 4);
}
#[test]
#[ignore = "local Gaussian color saved-byte field audit"]
fn original_gaussian_color_fields_audit() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/gaussian-raster-color-evidence");
    for name in ["implicit-unit-f32", "assigned-unit-f32", "implicit-hdr-f32", "assigned-hdr-f32", "implicit-hdr-f64"]
        .into_iter()
        .flat_map(|case| [format!("{case}.source.xcf"), format!("{case}.edit.xcf")])
    {
        let file = photocraft_xcf::decode(&std::fs::read(base.join(&name)).unwrap(), &Default::default()).unwrap();
        println!("FILE {name}: version={} precision={} ICC={}", file.version, file.precision, file.icc.len());
        for layer in &file.layers {
            println!(
                "LAYER {} bounds={:?} {}x{} mode={} tuple=({},{},{}) opacity={} visible={} maskflags=({},{},{})",
                layer.name,
                layer.offset,
                layer.width,
                layer.height,
                layer.mode,
                layer.composite_mode,
                layer.composite_space,
                layer.blend_space,
                layer.opacity,
                layer.visible,
                layer.mask_enabled,
                layer.mask_shown,
                layer.mask_editing
            );
            for effect in &layer.effects {
                println!("EFFECT {} {:?} {:?} args={:?}", effect.operation, effect.version, effect.state, effect.arguments);
                println!("RAW {:?}", effect.properties.iter().filter(|p| p.tag != 45).map(|p| (p.tag, p.payload.as_slice())).collect::<Vec<_>>());
                if let Some(mask) = &effect.mask {
                    println!(
                        "MASK {:?} {}x{} tattoo={:?} visible={} legacy_linked={} flags=({},{},{}) sample={:?} nonzero={}",
                        mask.offset,
                        mask.width,
                        mask.height,
                        mask.tattoo,
                        mask.visible,
                        mask.legacy_linked,
                        mask.mask_enabled,
                        mask.mask_shown,
                        mask.mask_editing,
                        mask.plane.as_ref().map(|p| p.sample),
                        mask.plane.as_ref().is_some_and(|p| p.bytes.iter().any(|b| *b != 0))
                    );
                }
            }
        }
    }
}
#[test]
#[ignore = "requires failed public context-only no-AA authoring control"]
fn context_noaa_does_not_configure_saved_vector_antialias() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/noaa-evidence");
    for phase in ["source", "edit"] {
        let file = photocraft_xcf::decode(&std::fs::read(base.join(format!("{phase}.xcf"))).unwrap(), &Default::default()).unwrap();
        assert!(file.layers.iter().find_map(|l| l.vector.as_ref()).unwrap().fill.antialias);
    }
}
#[test]
#[ignore = "requires own canonical diagonal no-AA counterexample"]
fn diagonal_noaa_is_not_silently_approximated() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/noaa-canonical");
    for phase in ["source", "edit"] {
        let file = photocraft_xcf::decode(&std::fs::read(base.join(format!("{phase}.xcf"))).unwrap(), &Default::default()).unwrap();
        assert!(!file.layers.iter().find_map(|l| l.vector.as_ref()).unwrap().fill.antialias);
    }
    let path = Path::new(vec![Subpath::polygon(&[(10.25, 11.75), (47.5, 15.25), (24.75, 38.5)])]);
    assert!(photocraft_vector::xcf::polygon_descriptor(&path).is_err());
}
#[test]
#[ignore = "requires own canonical rectilinear no-AA source/edit evidence packet"]
fn original_noaa_orthogonal_source_and_path_edit_regenerate_exact_native_and_projection() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/orthogonal-canonical");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let independent: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("round2/observations.json")).unwrap()).unwrap();
    let mut projections = Vec::new();
    for phase in ["source", "edit"] {
        let bytes = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        use sha2::{Digest, Sha256};
        let hash: String = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(
            hash,
            if phase == "source" {
                "4df42225a017b2c4548e14defabae15f1662c52f003b2e9b59ea693e695042ad"
            } else {
                "270f6079a4d7e0ef2f4300f731f2acb0ec5b2ebce221aca8b2bb51588cde4fd0"
            }
        );
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!((file.version, file.precision), (24, 150));
        println!("{phase} version={} precision={} icc={:?}", file.version, file.precision, file.icc.len());
        let layer = file.layers.iter().find(|l| l.vector.is_some()).unwrap();
        let vector = layer.vector.as_ref().unwrap();
        println!("{phase} offset={:?} vector={vector:?}", layer.offset);
        assert!(!vector.source_modified);
        assert!(vector.filled);
        assert!(!vector.stroked);
        assert_eq!(vector.fill.style, 0);
        assert!(!vector.fill.antialias);
        let path = file.paths.iter().find(|p| p.tattoo == Some(vector.path_tattoo)).unwrap();
        assert_eq!(path.strokes.len(), 1);
        let stroke = &path.strokes[0];
        assert!(stroke.closed);
        assert_eq!(stroke.points.len(), 18);
        let knots = stroke
            .points
            .as_chunks::<3>().0.iter()
            .map(|triple| {
                assert_eq!([triple[0].kind, triple[1].kind, triple[2].kind], [1, 0, 1]);
                let point = |p: &photocraft_xcf::PathPoint| Point::new(f64::from(f32::from_bits(p.components[0])), f64::from(f32::from_bits(p.components[1])));
                Knot { in_ctrl: point(&triple[0]), anchor: point(&triple[1]), out_ctrl: point(&triple[2]), smooth: false }
            })
            .collect();
        let path = Path { subpaths: vec![Subpath { knots, closed: true, op: PathOp::Combine }], fill_rule: FillRule::NonZero, inverted: false };
        let color = vector.fill.color.as_ref().unwrap();
        assert_eq!(color.encoding, "R'G'B'A float");
        assert!(color.icc.is_empty());
        let rgba: Vec<f64> = color.samples.as_chunks::<4>().0.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect();
        let settings = SolidSettings { fill: Some(rgba.try_into().unwrap()), stroke: None, antialias: false };
        let render = |bounds| render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        let state = &observations["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        let repeated = &independent["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        assert_eq!(state["layer"]["offset"], repeated["layer"]["offset"]);
        assert_eq!(layer.offset, (state["layer"]["offset"][0].as_i64().unwrap() as i32, state["layer"]["offset"][1].as_i64().unwrap() as i32));
        assert_eq!([layer.width, layer.height], [state["layer"]["size"][0].as_u64().unwrap() as u32, state["layer"]["size"][1].as_u64().unwrap() as u32]);
        let bounds = Rect::new(layer.offset.0, layer.offset.1, layer.offset.0 + layer.width as i32, layer.offset.1 + layer.height as i32);
        let native = std::fs::read(base.join(state["layer"]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(layer.plane.as_ref().unwrap().bytes, native);
        let actual = render(bounds).try_read_interleaved(bounds).unwrap();
        let mismatches: Vec<_> = actual
            .as_chunks::<4>().0.iter()
            .zip(native.as_chunks::<4>().0.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i % layer.width as usize, i / layer.width as usize, a.to_vec(), b.to_vec()))
            .collect();
        println!("noAA orthogonal mismatch{} first{:?}", mismatches.len(), mismatches.iter().take(12).collect::<Vec<_>>());
        assert!(mismatches.is_empty());
        assert_eq!(native, std::fs::read(base.join("round2").join(repeated["layer"]["native"]["file"].as_str().unwrap())).unwrap());
        let canvas = Rect::new(0, 0, 64, 48);
        let projected = render(canvas).try_read_interleaved(canvas).unwrap();
        assert_eq!(projected, std::fs::read(base.join(state["projection"]["native"]["file"].as_str().unwrap())).unwrap());
        assert_eq!(projected, std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap());
        assert_eq!(projected, std::fs::read(base.join("round2").join(repeated["projection"]["native"]["file"].as_str().unwrap())).unwrap());
        projections.push(projected);
    }
    assert_ne!(projections[0], projections[1], "public path translation edit must change canvas projection");
}

#[test]
#[ignore = "requires independent public Cairo binary rectilinear holdouts"]
fn independent_noaa_rectilinear_holdouts_match() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/noaa-holdouts.json");
    let packet: serde_json::Value = serde_json::from_slice(&std::fs::read(base).unwrap()).unwrap();
    let rows = packet["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 512);
    for (case, row) in rows.iter().enumerate() {
        let points: Vec<_> = row["points"].as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect();
        let path = Path::new(vec![Subpath::polygon(&points)]);
        let surface = render_solid(
            &path,
            &SolidSettings { fill: Some([1.; 4]), stroke: None, antialias: false },
            Rect::new(0, 0, 32, 32),
            PixelFormat::RGBA8,
            &Interrupt::NONE,
            &mut |_| Ok(()),
            &mut |_| Ok(()),
        )
        .unwrap();
        let hex = row["alpha_hex"].as_str().unwrap();
        let expected: Vec<_> = hex.as_bytes().as_chunks::<2>().0.iter().map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap()).collect();
        let actual: Vec<_> = surface.try_read_interleaved(Rect::new(0, 0, 32, 32)).unwrap().as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        let mismatch: Vec<_> = actual.iter().zip(&expected).enumerate().filter(|(_, (a, b))| a != b).map(|(i, (a, b))| (i % 32, i / 32, *a, *b)).collect();
        assert!(mismatch.is_empty(), "case{case} mismatch{:?}", mismatch.iter().take(8).collect::<Vec<_>>());
    }
}
#[test]
fn aliased_domain_rejects_diagonal_curves_intersections_and_hostile_coordinates() {
    let descriptor = photocraft_vector::xcf::polygon_descriptor;
    assert!(descriptor(&Path::new(vec![Subpath::polygon(&[(0., 0.), (4., 0.), (4., 1.), (2., 1.), (2., 3.), (0., 3.)])])).is_ok());
    for points in [
        vec![(0., 0.), (4., 1.), (2., 3.)],
        vec![(0., 0.), (4., 0.), (4., 4.), (2., 4.), (2., -1.), (0., -1.)],
        vec![(0., 0.), (4., 0.), (4., 4.), (0., 4.), (0., 4.)],
        vec![(0., 0.), (f64::INFINITY, 0.), (4., 4.), (0., 4.)],
    ] {
        assert!(descriptor(&Path::new(vec![Subpath::polygon(&points)])).is_err());
    }
    let mut curved = Path::new(vec![Subpath::polygon(&[(0., 0.), (4., 0.), (4., 4.), (0., 4.)])]);
    curved.subpaths[0].knots[0].out_ctrl.y = 1.;
    assert!(descriptor(&curved).is_err());
}

#[test]
fn aliased_compound_nonzero_preserves_holes_and_islands() {
    let outer = Subpath::polygon(&[(0., 0.), (8., 0.), (8., 8.), (0., 8.)]);
    let hole = Subpath::polygon(&[(2., 2.), (2., 6.), (6., 6.), (6., 2.)]);
    let island = Subpath::polygon(&[(10., 0.), (12., 0.), (12., 2.), (10., 2.)]);
    let path = Path::new(vec![outer, hole, island]);
    let bounds = Rect::new(0, 0, 12, 8);
    assert_eq!(photocraft_vector::xcf::polygon_descriptor(&path).unwrap(), [0., 0., 12., 8.]);
    let rendered = render_solid(
        &path,
        &SolidSettings { fill: Some([1.; 4]), stroke: None, antialias: false },
        bounds,
        PixelFormat::RGBA8,
        &Interrupt::NONE,
        &mut |_| Ok(()),
        &mut |_| Ok(()),
    )
    .unwrap();
    let bytes = rendered.try_read_interleaved(bounds).unwrap();
    for y in 0..8 {
        for x in 0..12 {
            let expected = (x < 8 && !((2..6).contains(&x) && (2..6).contains(&y))) || (x >= 10 && y < 2);
            assert_eq!(bytes[(y * 12 + x) * 4 + 3], if expected { 255 } else { 0 });
        }
    }
}

#[test]
fn compound_contours_reject_touch_crossing_and_count_abuse() {
    let rect = |x: f64, y: f64| Subpath::polygon(&[(x, y), (x + 4., y), (x + 4., y + 4.), (x, y + 4.)]);
    for path in [Path::new(vec![rect(0., 0.), rect(4., 0.)]), Path::new(vec![rect(0., 0.), rect(2., 2.)]), Path::new(vec![rect(0., 0.); 17])] {
        assert!(photocraft_vector::xcf::polygon_descriptor(&path).is_err());
    }
    let mut path = Path::new(vec![rect(0., 0.), rect(8., 0.)]);
    path.subpaths[1].op = PathOp::Subtract;
    assert!(photocraft_vector::xcf::polygon_descriptor(&path).is_err());
    path.subpaths[1].op = PathOp::Combine;
    let settings = SolidSettings { fill: Some([1.; 4]), stroke: None, antialias: false };
    assert!(matches!(
        render_solid(&path, &settings, Rect::new(0, 0, 12, 4), PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Err(VectorError::Budget)),
        Err(VectorError::Budget)
    ));
    assert!(
        render_solid(
            &path,
            &SolidSettings { antialias: true, ..settings },
            Rect::new(0, 0, 12, 4),
            PixelFormat::RGBA8,
            &Interrupt::NONE,
            &mut |_| Ok(()),
            &mut |_| Ok(())
        )
        .is_err()
    );
}

#[test]
fn compound_same_winding_nested_contours_stay_unsupported() {
    let outer = Subpath::polygon(&[(0., 0.), (8., 0.), (8., 8.), (0., 8.)]);
    let inner = Subpath::polygon(&[(2., 2.), (6., 2.), (6., 6.), (2., 6.)]);
    assert!(
        photocraft_vector::xcf::polygon_descriptor(&Path::new(vec![outer, inner])).is_err(),
        "stock independent reverse-stroke control leaves the nested region empty; nonzero would fill it"
    );
}

#[test]
fn explicit_parity_fills_nested_contours_without_rewriting_source_fill_rule() {
    use photocraft_vector::xcf::{AliasedFillRule, render_solid_with_rule};
    let path = Path::new(vec![
        Subpath::polygon(&[(0., 0.), (8., 0.), (8., 8.), (0., 8.)]),
        Subpath::polygon(&[(2., 2.), (6., 2.), (6., 6.), (2., 6.)]),
        Subpath::polygon(&[(3., 3.), (5., 3.), (5., 5.), (3., 5.)]),
    ]);
    let before = path.clone();
    let settings = SolidSettings { fill: Some([1., 0., 0., 1.]), stroke: None, antialias: false };
    let bounds = Rect::new(0, 0, 8, 8);
    let surface =
        render_solid_with_rule(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(()), AliasedFillRule::EvenOdd)
            .unwrap();
    let sample = |x, y| surface.try_read_interleaved(Rect::new(x, y, x + 1, y + 1)).unwrap();
    assert_eq!(sample(1, 1)[3], 255);
    assert_eq!(sample(2, 2)[3], 0);
    assert_eq!(sample(3, 3)[3], 255);
    assert_eq!(path, before);
    assert_eq!(path.fill_rule, FillRule::NonZero);
}

#[test]
#[ignore = "requires own independent stock reverse-stroke negative control"]
fn original_same_winding_control_is_not_admitted_as_nonzero() {
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/vector-compound-winding-control");
    let raw = std::fs::read(base.join("source.xcf")).unwrap();
    assert_eq!(Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect::<String>(), "b68dc17b58a855c5114dbae79568ec7c2da5c42786c7d57a3d2c20a49d4e943f");
    let file = photocraft_xcf::decode(&raw, &Default::default()).unwrap();
    assert_eq!((file.version, file.precision), (24, 150));
    let layer = file.layers.iter().find(|l| l.vector.is_some()).unwrap();
    let source = layer.vector.as_ref().unwrap();
    assert!(!source.source_modified);
    assert!(!source.fill.antialias);
    let saved = file.paths.iter().find(|p| p.tattoo == Some(source.path_tattoo)).unwrap();
    let path = Path::new(
        saved
            .strokes
            .iter()
            .map(|s| {
                assert!(s.closed);
                Subpath {
                    closed: true,
                    op: PathOp::Combine,
                    knots: s
                        .points
                        .as_chunks::<3>().0.iter()
                        .map(|v| {
                            let p = |v: &photocraft_xcf::PathPoint| {
                                Point::new(f64::from(f32::from_bits(v.components[0])), f64::from(f32::from_bits(v.components[1])))
                            };
                            Knot { in_ctrl: p(&v[0]), anchor: p(&v[1]), out_ctrl: p(&v[2]), smooth: false }
                        })
                        .collect(),
                }
            })
            .collect(),
    );
    assert!(photocraft_vector::xcf::polygon_descriptor(&path).is_err());
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("winding-observations.json")).unwrap()).unwrap();
    for row in observations["measured"].as_array().unwrap() {
        assert_eq!(row["alpha_at_canvas_20_20"], 0);
        assert!(row["signed_areas"].as_array().unwrap().iter().all(|v| v.as_f64().unwrap() > 0.));
    }
}

#[test]
#[ignore = "requires own canonical rectilinear no-AA source/edit evidence packet"]
fn original_noaa_compound_hole_island_and_path_edit_match_independent_gimp() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/compound-canonical");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let independent: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("round2/observations.json")).unwrap()).unwrap();
    let mut projections = Vec::new();
    for phase in ["source", "edit"] {
        let bytes = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        use sha2::{Digest, Sha256};
        let hash: String = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(
            hash,
            if phase == "source" {
                "de6b936e0873198508953cb293b1256b2627ec4314adc1912f289bb8ab2bb2bd"
            } else {
                "ad479b969a799f7dc3dd5bfd40089f8651b55861ad893b7fbfc5ba15e93ff5bf"
            }
        );
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!((file.version, file.precision), (24, 150));
        println!("{phase} version={} precision={} icc={:?}", file.version, file.precision, file.icc.len());
        let layer = file.layers.iter().find(|l| l.vector.is_some()).unwrap();
        let vector = layer.vector.as_ref().unwrap();
        println!("{phase} offset={:?} vector={vector:?}", layer.offset);
        assert!(!vector.source_modified);
        assert!(vector.filled);
        assert!(!vector.stroked);
        assert_eq!(vector.fill.style, 0);
        assert!(!vector.fill.antialias);
        let path = file.paths.iter().find(|p| p.tattoo == Some(vector.path_tattoo)).unwrap();
        assert_eq!(path.strokes.len(), 3);
        let subpaths = path
            .strokes
            .iter()
            .map(|stroke| {
                assert!(stroke.closed);
                assert_eq!(stroke.points.len(), 12);
                let knots = stroke
                    .points
                    .as_chunks::<3>().0.iter()
                    .map(|triple| {
                        assert_eq!([triple[0].kind, triple[1].kind, triple[2].kind], [1, 0, 1]);
                        let point =
                            |p: &photocraft_xcf::PathPoint| Point::new(f64::from(f32::from_bits(p.components[0])), f64::from(f32::from_bits(p.components[1])));
                        Knot { in_ctrl: point(&triple[0]), anchor: point(&triple[1]), out_ctrl: point(&triple[2]), smooth: false }
                    })
                    .collect();
                Subpath { knots, closed: true, op: PathOp::Combine }
            })
            .collect();
        let path = Path { subpaths, fill_rule: FillRule::NonZero, inverted: false };
        let color = vector.fill.color.as_ref().unwrap();
        assert_eq!(color.encoding, "R'G'B'A float");
        assert!(color.icc.is_empty());
        let rgba: Vec<f64> = color.samples.as_chunks::<4>().0.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect();
        let settings = SolidSettings { fill: Some(rgba.try_into().unwrap()), stroke: None, antialias: false };
        let render = |bounds| render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        let state = &observations["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        let repeated = &independent["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        assert_eq!(state["layer"]["offset"], repeated["layer"]["offset"]);
        assert_eq!(layer.offset, (state["layer"]["offset"][0].as_i64().unwrap() as i32, state["layer"]["offset"][1].as_i64().unwrap() as i32));
        assert_eq!([layer.width, layer.height], [state["layer"]["size"][0].as_u64().unwrap() as u32, state["layer"]["size"][1].as_u64().unwrap() as u32]);
        let bounds = Rect::new(layer.offset.0, layer.offset.1, layer.offset.0 + layer.width as i32, layer.offset.1 + layer.height as i32);
        let native = std::fs::read(base.join(state["layer"]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(layer.plane.as_ref().unwrap().bytes, native);
        let actual = render(bounds).try_read_interleaved(bounds).unwrap();
        let mismatches: Vec<_> = actual
            .as_chunks::<4>().0.iter()
            .zip(native.as_chunks::<4>().0.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i % layer.width as usize, i / layer.width as usize, a.to_vec(), b.to_vec()))
            .collect();
        println!("noAA orthogonal mismatch{} first{:?}", mismatches.len(), mismatches.iter().take(12).collect::<Vec<_>>());
        assert!(mismatches.is_empty());
        assert_eq!(native, std::fs::read(base.join("round2").join(repeated["layer"]["native"]["file"].as_str().unwrap())).unwrap());
        let canvas = Rect::new(0, 0, 64, 48);
        let projected = render(canvas).try_read_interleaved(canvas).unwrap();
        assert_eq!(projected, std::fs::read(base.join(state["projection"]["native"]["file"].as_str().unwrap())).unwrap());
        assert_eq!(projected, std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap());
        assert_eq!(projected, std::fs::read(base.join("round2").join(repeated["projection"]["native"]["file"].as_str().unwrap())).unwrap());
        projections.push(projected);
    }
    assert_ne!(projections[0], projections[1], "public path translation edit must change canvas projection");
}

#[test]
#[ignore = "requires own canonical rectilinear no-AA source/edit evidence packet"]
fn original_same_winding_parity_source_edit_matches_independent_gimp() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/compound-winding-control");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let mut projections = Vec::new();
    for phase in ["source", "edit"] {
        let bytes = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        use sha2::{Digest, Sha256};
        let hash: String = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(
            hash,
            if phase == "source" {
                "b68dc17b58a855c5114dbae79568ec7c2da5c42786c7d57a3d2c20a49d4e943f"
            } else {
                "e3a1039ce15dbad2839326eb7231dea85c92d3f04202aeeaa8714b51ea4dc312"
            }
        );
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!((file.version, file.precision), (24, 150));
        println!("{phase} version={} precision={} icc={:?}", file.version, file.precision, file.icc.len());
        let layer = file.layers.iter().find(|l| l.vector.is_some()).unwrap();
        let vector = layer.vector.as_ref().unwrap();
        println!("{phase} offset={:?} vector={vector:?}", layer.offset);
        assert!(!vector.source_modified);
        assert!(vector.filled);
        assert!(!vector.stroked);
        assert_eq!(vector.fill.style, 0);
        assert!(!vector.fill.antialias);
        let path = file.paths.iter().find(|p| p.tattoo == Some(vector.path_tattoo)).unwrap();
        assert_eq!(path.strokes.len(), 3);
        let subpaths = path
            .strokes
            .iter()
            .map(|stroke| {
                assert!(stroke.closed);
                assert_eq!(stroke.points.len(), 12);
                let knots = stroke
                    .points
                    .as_chunks::<3>().0.iter()
                    .map(|triple| {
                        assert_eq!([triple[0].kind, triple[1].kind, triple[2].kind], [1, 0, 1]);
                        let point =
                            |p: &photocraft_xcf::PathPoint| Point::new(f64::from(f32::from_bits(p.components[0])), f64::from(f32::from_bits(p.components[1])));
                        Knot { in_ctrl: point(&triple[0]), anchor: point(&triple[1]), out_ctrl: point(&triple[2]), smooth: false }
                    })
                    .collect();
                Subpath { knots, closed: true, op: PathOp::Combine }
            })
            .collect();
        let path = Path { subpaths, fill_rule: FillRule::NonZero, inverted: false };
        let color = vector.fill.color.as_ref().unwrap();
        assert_eq!(color.encoding, "R'G'B'A float");
        assert!(color.icc.is_empty());
        let rgba: Vec<f64> = color.samples.as_chunks::<4>().0.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect();
        let settings = SolidSettings { fill: Some(rgba.try_into().unwrap()), stroke: None, antialias: false };
        let render = |bounds| {
            photocraft_vector::xcf::render_solid_with_rule(
                &path,
                &settings,
                bounds,
                PixelFormat::RGBA8,
                &Interrupt::NONE,
                &mut |_| Ok(()),
                &mut |_| Ok(()),
                photocraft_vector::xcf::AliasedFillRule::EvenOdd,
            )
            .unwrap()
        };
        let state = &observations["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        assert_eq!(layer.offset, (state["layer"]["offset"][0].as_i64().unwrap() as i32, state["layer"]["offset"][1].as_i64().unwrap() as i32));
        assert_eq!([layer.width, layer.height], [state["layer"]["size"][0].as_u64().unwrap() as u32, state["layer"]["size"][1].as_u64().unwrap() as u32]);
        let bounds = Rect::new(layer.offset.0, layer.offset.1, layer.offset.0 + layer.width as i32, layer.offset.1 + layer.height as i32);
        let native = std::fs::read(base.join(state["layer"]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(layer.plane.as_ref().unwrap().bytes, native);
        let actual = render(bounds).try_read_interleaved(bounds).unwrap();
        let mismatches: Vec<_> = actual
            .as_chunks::<4>().0.iter()
            .zip(native.as_chunks::<4>().0.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i % layer.width as usize, i / layer.width as usize, a.to_vec(), b.to_vec()))
            .collect();
        println!("noAA orthogonal mismatch{} first{:?}", mismatches.len(), mismatches.iter().take(12).collect::<Vec<_>>());
        assert!(mismatches.is_empty());
        let canvas = Rect::new(0, 0, 64, 48);
        let projected = render(canvas).try_read_interleaved(canvas).unwrap();
        assert_eq!(projected, std::fs::read(base.join(state["projection"]["native"]["file"].as_str().unwrap())).unwrap());
        projections.push(projected);
    }
    assert_ne!(projections[0], projections[1], "public path translation edit must change canvas projection");
}

#[test]
#[ignore = "requires own canonical rectilinear no-AA source/edit evidence packet"]
fn original_noaa_solid_stroke_source_edit_match_independent_gimp() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/stroke-canonical");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let independent: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("round2/observations.json")).unwrap()).unwrap();
    let mut projections = Vec::new();
    for phase in ["source", "edit"] {
        let bytes = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        use sha2::{Digest, Sha256};
        let hash: String = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(
            hash,
            if phase == "source" {
                "afe3e369553c1f714525cb25e8b6e8bc84e8e49427beb75f17fb7f236e213cd7"
            } else {
                "7b1a4a48a8dd93c89d5a876a1aaf5df7b7edaa62983e6cb9b3d8423fcdad9653"
            }
        );
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!((file.version, file.precision), (24, 150));
        println!("{phase} version={} precision={} icc={:?}", file.version, file.precision, file.icc.len());
        let layer = file.layers.iter().find(|l| l.vector.is_some()).unwrap();
        let vector = layer.vector.as_ref().unwrap();
        println!("{phase} offset={:?} vector={vector:?}", layer.offset);
        assert!(!vector.source_modified);
        assert!(!vector.filled);
        assert!(vector.stroked);
        assert_eq!(vector.stroke.fill.style, 0);
        assert!(!vector.stroke.fill.antialias);
        let path = file.paths.iter().find(|p| p.tattoo == Some(vector.path_tattoo)).unwrap();
        assert_eq!(path.strokes.len(), 1);
        let subpaths = path
            .strokes
            .iter()
            .map(|stroke| {
                assert!(stroke.closed);
                assert_eq!(stroke.points.len(), 12);
                let knots = stroke
                    .points
                    .as_chunks::<3>().0.iter()
                    .map(|triple| {
                        assert_eq!([triple[0].kind, triple[1].kind, triple[2].kind], [1, 0, 1]);
                        let point =
                            |p: &photocraft_xcf::PathPoint| Point::new(f64::from(f32::from_bits(p.components[0])), f64::from(f32::from_bits(p.components[1])));
                        Knot { in_ctrl: point(&triple[0]), anchor: point(&triple[1]), out_ctrl: point(&triple[2]), smooth: false }
                    })
                    .collect();
                Subpath { knots, closed: true, op: PathOp::Combine }
            })
            .collect();
        let path = Path { subpaths, fill_rule: FillRule::NonZero, inverted: false };
        let color = vector.stroke.fill.color.as_ref().unwrap();
        assert_eq!(color.encoding, "R'G'B'A float");
        assert!(color.icc.is_empty());
        let rgba: Vec<f64> = color.samples.as_chunks::<4>().0.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect();
        assert_eq!(vector.stroke.cap_style, 0);
        assert_eq!(vector.stroke.join_style, 0);
        assert_eq!(vector.stroke.miter_limit_bits, 10f32.to_bits());
        assert!(vector.stroke.dashes_bits.is_empty());
        assert_eq!(vector.stroke.width_bits, if phase == "source" { 6f32.to_bits() } else { 8f32.to_bits() });
        let settings = SolidSettings {
            fill: None,
            stroke: Some(SolidStroke {
                color: rgba.try_into().unwrap(),
                style: photocraft_vector::StrokeStyle {
                    width: f64::from(f32::from_bits(vector.stroke.width_bits)),
                    cap: photocraft_doc::LineCap::Butt,
                    join: photocraft_doc::LineJoin::Miter,
                    miter_limit: 10.,
                    dashes: Vec::new(),
                    dash_offset: 0.,
                },
            }),
            antialias: false,
        };
        let render = |bounds| render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        let state = &observations["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        let repeated = &independent["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        assert_eq!(state["layer"]["offset"], repeated["layer"]["offset"]);
        assert_eq!(layer.offset, (state["layer"]["offset"][0].as_i64().unwrap() as i32, state["layer"]["offset"][1].as_i64().unwrap() as i32));
        assert_eq!([layer.width, layer.height], [state["layer"]["size"][0].as_u64().unwrap() as u32, state["layer"]["size"][1].as_u64().unwrap() as u32]);
        let bounds = Rect::new(layer.offset.0, layer.offset.1, layer.offset.0 + layer.width as i32, layer.offset.1 + layer.height as i32);
        let native = std::fs::read(base.join(state["layer"]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(layer.plane.as_ref().unwrap().bytes, native);
        let actual = render(bounds).try_read_interleaved(bounds).unwrap();
        let mismatches: Vec<_> = actual
            .as_chunks::<4>().0.iter()
            .zip(native.as_chunks::<4>().0.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i % layer.width as usize, i / layer.width as usize, a.to_vec(), b.to_vec()))
            .collect();
        println!("noAA orthogonal mismatch{} first{:?}", mismatches.len(), mismatches.iter().take(12).collect::<Vec<_>>());
        assert!(mismatches.is_empty());
        assert_eq!(native, std::fs::read(base.join("round2").join(repeated["layer"]["native"]["file"].as_str().unwrap())).unwrap());
        let canvas = Rect::new(0, 0, 64, 48);
        let projected = render(canvas).try_read_interleaved(canvas).unwrap();
        assert_eq!(projected, std::fs::read(base.join(state["projection"]["native"]["file"].as_str().unwrap())).unwrap());
        assert_eq!(projected, std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap());
        assert_eq!(projected, std::fs::read(base.join("round2").join(repeated["projection"]["native"]["file"].as_str().unwrap())).unwrap());
        projections.push(projected);
    }
    assert_ne!(projections[0], projections[1], "public path translation edit must change canvas projection");
}

#[test]
fn aliased_rectangle_stroke_checks_style_and_exact_inner_outer_coverage() {
    use photocraft_vector::xcf::aliased_rectangle_stroke_descriptor;
    let path = Path::new(vec![Subpath::polygon(&[(10., 12.), (46., 12.), (46., 36.), (10., 36.)])]);
    let style = photocraft_vector::StrokeStyle {
        width: 6.,
        cap: photocraft_doc::LineCap::Butt,
        join: photocraft_doc::LineJoin::Miter,
        miter_limit: 10.,
        dashes: Vec::new(),
        dash_offset: 0.,
    };
    assert_eq!(aliased_rectangle_stroke_descriptor(&path, &style).unwrap(), [7., 9., 49., 39.]);
    for width in [f64::NAN, f64::INFINITY, 0., -6., 6.25, 33.] {
        let mut bad = style.clone();
        bad.width = width;
        assert!(aliased_rectangle_stroke_descriptor(&path, &bad).is_err());
    }
    let mut bad = style.clone();
    bad.dashes = vec![2., 2.];
    assert!(aliased_rectangle_stroke_descriptor(&path, &bad).is_err());
    let mut bad = style.clone();
    bad.cap = photocraft_doc::LineCap::Round;
    assert!(aliased_rectangle_stroke_descriptor(&path, &bad).is_err());
    let mut curved = path.clone();
    curved.subpaths[0].knots[0].out_ctrl.x += 1.;
    assert!(aliased_rectangle_stroke_descriptor(&curved, &style).is_err());
    let settings = SolidSettings { fill: None, stroke: Some(SolidStroke { color: [1., 0., 0., 1.], style }), antialias: false };
    let bounds = Rect::new(7, 9, 49, 39);
    let surface = render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    assert_eq!(surface.try_read_interleaved(Rect::new(7, 9, 8, 10)).unwrap()[3], 255);
    assert_eq!(surface.try_read_interleaved(Rect::new(13, 15, 14, 16)).unwrap()[3], 0);
    assert!(render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Err(VectorError::Budget), &mut |_| Ok(())).is_err());
    assert!(render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::cancel_only(&|| true), &mut |_| Ok(()), &mut |_| Ok(())).is_err());
}

#[test]
#[ignore = "requires own natural public AA stroke source/edit and repeat packet"]
fn original_aa_solid_stroke_source_edit_match_independent_gimp() {
    verify_stroke_pair(
        "aa-stroke-control",
        ["86f8cbba43879965a77e1218361d9f98ba48cf82e5282924d8f3ce2796844db0", "42e0650abb63c875bbc720b79f5f577fe7a08d6bb642abba36215059d616ca06"],
        [3., 5.],
        true,
        0,
    );
}
#[test]
#[ignore = "requires own natural public even AA stroke source/edit and repeat packet"]
fn original_aa_even_solid_stroke_source_edit_match_independent_gimp() {
    verify_stroke_pair(
        "aa-even-stroke-control",
        ["559ddb3e3ef94b680936cbd67176e64688ae90bb7aac19d63ce09579be4bf852", "1b020d836fa6e57133f9738730893a882552c80bd9047fb1ecb2ae86a015dec9"],
        [2., 4.],
        true,
        0,
    );
}
#[test]
#[ignore = "requires own public no-AA bevel stroke sources/edit and independent repeats"]
fn original_noaa_bevel_stroke_source_edit_and_reversed_negative_holdout_match_gimp() {
    verify_stroke_pair(
        "bevel-stroke-control",
        ["c4c3504301234f459645034f92f055b5e6da2e906fc413577a14257af756dff7", "7f8e1341afc124baa7c3c50d2b59197a411636878772d16a41c651278fe75fa4"],
        [6., 8.],
        false,
        2,
    );
    verify_stroke_pair(
        "bevel-reversed-negative-control",
        ["2f3ea5ad1bd239305a39a5d458b70734f42291e65d00572bd02d417371572d75", "9733d834d99b51e4b3c4e2454041587b84c9f56acba87d75ca5d4db2cf044178"],
        [6., 8.],
        false,
        2,
    );
}
#[test]
#[ignore = "requires own public round-join source/edit and negative-origin repeat witnesses"]
fn original_noaa_round_stroke_source_edit_and_reversed_negative_holdout_match_gimp() {
    verify_stroke_pair(
        "round-stroke-control",
        ["698b64785218f4f1cf7c3a9c94e0317debaec723b9a0b51256d5074eb156cd47", "72b6bb5ffbe5fd648a925ac75b6f4ee76a26936548ce33117e559255b90b6976"],
        [6., 8.],
        false,
        1,
    );
    verify_stroke_pair(
        "round-reversed-negative-control",
        ["4794075aeb13e228a9583840137506f207b9ba272e449812f12bf071bfbb2a1c", "8a0e47222498a317349de9674d42a28073bbb8768f02afacb93d22bedc702ffa"],
        [6., 8.],
        false,
        1,
    );
}
#[test]
#[ignore = "requires own public odd-width no-AA miter source/edit and repeat witnesses"]
fn original_noaa_odd_miter_stroke_source_edit_match_gimp() {
    verify_stroke_pair(
        "odd-miter-stroke-control",
        ["95a76e08ccedb0ceb06bbfbf6de9bf37a30252ecc66cbd377871cef854e9f7c3", "141a0498494475826f7e6a7f0a8bd91c15de08d283371b10e9de62e6c63d297a"],
        [7., 9.],
        false,
        0,
    );
}
#[test]
#[ignore = "requires own public AA widths1/7 source/edit and repeat witnesses"]
fn original_aa_extended_odd_miter_stroke_source_edit_match_gimp() {
    verify_stroke_pair(
        "aa-extended-odd-stroke-control",
        ["06409365fd48d645b06533d86eea11f649bdc12f4927872e09386a587374c8b2", "3375f8c9c606206c14c9a6f028dbd02b608ee8d4e90bcbb27bc0ca379b45121a"],
        [1., 7.],
        true,
        0,
    );
}
fn verify_stroke_pair(directory: &str, hashes: [&str; 2], widths: [f32; 2], antialias: bool, join: u32) {
    verify_stroke_pair_with_closed(directory, hashes, widths, antialias, join, true);
}
fn verify_stroke_pair_with_closed(directory: &str, hashes: [&str; 2], widths: [f32; 2], antialias: bool, join: u32, closed: bool) {
    verify_stroke_pair_with_cap(directory, hashes, widths, antialias, join, closed, 0);
}
fn verify_stroke_pair_with_cap(directory: &str, hashes: [&str; 2], widths: [f32; 2], antialias: bool, join: u32, closed: bool, cap: u32) {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa").join(directory);
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let independent: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("round2/observations.json")).unwrap()).unwrap();
    let mut projections = Vec::new();
    for phase in ["source", "edit"] {
        let bytes = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        use sha2::{Digest, Sha256};
        let hash: String = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(hash, hashes[usize::from(phase == "edit")]);
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!((file.version, file.precision), (24, 150));
        println!("{phase} version={} precision={} icc={:?}", file.version, file.precision, file.icc.len());
        let layer = file.layers.iter().find(|l| l.vector.is_some()).unwrap();
        let vector = layer.vector.as_ref().unwrap();
        println!("{phase} offset={:?} vector={vector:?}", layer.offset);
        assert!(!vector.source_modified);
        assert!(!vector.filled);
        assert!(vector.stroked);
        assert_eq!(vector.stroke.fill.style, 0);
        assert_eq!(vector.stroke.fill.antialias, antialias);
        let path = file.paths.iter().find(|p| p.tattoo == Some(vector.path_tattoo)).unwrap();
        assert_eq!(path.strokes.len(), 1);
        let subpaths = path
            .strokes
            .iter()
            .map(|stroke| {
                assert_eq!(stroke.closed, closed);
                assert_eq!(
                    stroke.points.len(),
                    if closed || (directory.contains("collinear-") || directory.contains("holdout-")) && directory.ends_with("-vertical") {
                        12
                    } else if (directory.contains("collinear-") || directory.contains("holdout-")) && directory.ends_with("-horizontal") {
                        9
                    } else {
                        6
                    }
                );
                let knots = stroke
                    .points
                    .as_chunks::<3>().0.iter()
                    .map(|triple| {
                        assert_eq!([triple[0].kind, triple[1].kind, triple[2].kind], [1, 0, 1]);
                        let point =
                            |p: &photocraft_xcf::PathPoint| Point::new(f64::from(f32::from_bits(p.components[0])), f64::from(f32::from_bits(p.components[1])));
                        Knot { in_ctrl: point(&triple[0]), anchor: point(&triple[1]), out_ctrl: point(&triple[2]), smooth: false }
                    })
                    .collect();
                Subpath { knots, closed, op: PathOp::Combine }
            })
            .collect();
        let path = Path { subpaths, fill_rule: FillRule::NonZero, inverted: false };
        let color = vector.stroke.fill.color.as_ref().unwrap();
        assert_eq!(color.encoding, "R'G'B'A float");
        assert!(color.icc.is_empty());
        let rgba: Vec<f64> = color.samples.as_chunks::<4>().0.iter().map(|v| f64::from(f32::from_le_bytes(*v))).collect();
        assert_eq!(vector.stroke.cap_style, cap);
        assert_eq!(vector.stroke.join_style, join);
        assert_eq!(vector.stroke.miter_limit_bits, 10f32.to_bits());
        assert!(vector.stroke.dashes_bits.is_empty());
        assert_eq!(vector.stroke.width_bits, widths[usize::from(phase == "edit")].to_bits());
        let settings = SolidSettings {
            fill: None,
            stroke: Some(SolidStroke {
                color: rgba.try_into().unwrap(),
                style: photocraft_vector::StrokeStyle {
                    width: f64::from(f32::from_bits(vector.stroke.width_bits)),
                    cap: match cap {
                        1 => photocraft_doc::LineCap::Round,
                        2 => photocraft_doc::LineCap::Square,
                        _ => photocraft_doc::LineCap::Butt,
                    },
                    join: match join {
                        1 => photocraft_doc::LineJoin::Round,
                        2 => photocraft_doc::LineJoin::Bevel,
                        _ => photocraft_doc::LineJoin::Miter,
                    },
                    miter_limit: 10.,
                    dashes: Vec::new(),
                    dash_offset: 0.,
                },
            }),
            antialias,
        };
        let render = |bounds| render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        let state = &observations["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        let repeated = &independent["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
        assert_eq!(state["layer"]["offset"], repeated["layer"]["offset"]);
        assert_eq!(layer.offset, (state["layer"]["offset"][0].as_i64().unwrap() as i32, state["layer"]["offset"][1].as_i64().unwrap() as i32));
        assert_eq!([layer.width, layer.height], [state["layer"]["size"][0].as_u64().unwrap() as u32, state["layer"]["size"][1].as_u64().unwrap() as u32]);
        let bounds = Rect::new(layer.offset.0, layer.offset.1, layer.offset.0 + layer.width as i32, layer.offset.1 + layer.height as i32);
        let native = std::fs::read(base.join(state["layer"]["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(layer.plane.as_ref().unwrap().bytes, native);
        let actual = render(bounds).try_read_interleaved(bounds).unwrap();
        let mismatches: Vec<_> = actual
            .as_chunks::<4>().0.iter()
            .zip(native.as_chunks::<4>().0.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i % layer.width as usize, i / layer.width as usize, a.to_vec(), b.to_vec()))
            .collect();
        println!("AA stroke mismatch{} first{:?}", mismatches.len(), mismatches.iter().take(12).collect::<Vec<_>>());
        assert!(mismatches.is_empty());
        assert_eq!(native, std::fs::read(base.join("round2").join(repeated["layer"]["native"]["file"].as_str().unwrap())).unwrap());
        let canvas = Rect::new(0, 0, 64, 48);
        let projected = render(bounds).try_read_interleaved(canvas).unwrap();
        assert_eq!(projected, std::fs::read(base.join(state["projection"]["native"]["file"].as_str().unwrap())).unwrap());
        assert_eq!(projected, std::fs::read(base.join(format!("{phase}.ref.rgba-u8.bin"))).unwrap());
        assert_eq!(projected, std::fs::read(base.join("round2").join(repeated["projection"]["native"]["file"].as_str().unwrap())).unwrap());
        projections.push(projected);
    }
    assert_ne!(projections[0], projections[1], "public path translation edit must change canvas projection");
}

#[test]
fn aa_rectangle_ring_checks_observed_boundary_storage_budget_and_cancel() {
    use photocraft_vector::xcf::antialiased_rectangle_stroke_descriptor;
    use std::sync::atomic::{AtomicBool, Ordering};
    let path = Path::new(vec![Subpath::polygon(&[(10., 12.), (46., 12.), (46., 36.), (10., 36.)])]);
    let style = photocraft_vector::StrokeStyle {
        width: 3.,
        cap: photocraft_doc::LineCap::Butt,
        join: photocraft_doc::LineJoin::Miter,
        miter_limit: 10.,
        dashes: vec![],
        dash_offset: 0.,
    };
    let bounds = antialiased_rectangle_stroke_descriptor(&path, &style).unwrap();
    assert_eq!(bounds, Rect::new(8, 10, 47, 37));
    let settings = SolidSettings { fill: None, stroke: Some(SolidStroke { color: [1.; 4], style: style.clone() }), antialias: true };
    let mut resident = 0usize;
    let output = render_solid(
        &path,
        &settings,
        bounds,
        PixelFormat::RGBA8,
        &Interrupt::NONE,
        &mut |n| {
            resident = resident.checked_add(n).ok_or(VectorError::Budget)?;
            if resident > 4 * 1024 * 1024 {
                return Err(VectorError::Budget);
            }
            Ok(())
        },
        &mut |_| Ok(()),
    )
    .unwrap();
    let alpha = |x, y| output.try_read_interleaved(Rect::new(x, y, x + 1, y + 1)).unwrap()[3];
    assert_eq!([alpha(8, 10), alpha(9, 10), alpha(8, 11), alpha(11, 13), alpha(12, 13), alpha(47, 11)], [60, 119, 128, 196, 136, 0]);
    // Integer even widths have no half-pixel boundary coverage.
    for (width, expected) in [(2., Rect::new(9, 11, 47, 37)), (4., Rect::new(8, 10, 48, 38))] {
        let mut even = settings.clone();
        even.stroke.as_mut().unwrap().style.width = width;
        let bounds = antialiased_rectangle_stroke_descriptor(&path, &even.stroke.as_ref().unwrap().style).unwrap();
        assert_eq!(bounds, expected);
        let rendered = render_solid(&path, &even, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        assert_eq!(rendered.try_read_interleaved(Rect::new(bounds.x0, bounds.y0, bounds.x0 + 1, bounds.y0 + 1)).unwrap()[3], 255);
        assert_eq!(rendered.try_read_interleaved(Rect::new(12, 14, 13, 15)).unwrap()[3], 0);
    }
    for width in [f64::NAN, f64::INFINITY, 0., -1., 3.5, 24., 33.] {
        let mut bad = style.clone();
        bad.width = width;
        assert!(antialiased_rectangle_stroke_descriptor(&path, &bad).is_err());
    }
    let mut fractional = path.clone();
    for knot in &mut fractional.subpaths[0].knots {
        knot.anchor.x += 0.25;
        knot.in_ctrl = knot.anchor;
        knot.out_ctrl = knot.anchor;
    }
    assert!(antialiased_rectangle_stroke_descriptor(&fractional, &style).is_err());
    assert!(matches!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Err(VectorError::Budget), &mut |_| Ok(())),
        Err(VectorError::Budget)
    ));
    let cancelled = AtomicBool::new(false);
    let poll = || cancelled.load(Ordering::Relaxed);
    let ctl = Interrupt::cancel_only(&poll);
    assert!(matches!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &ctl, &mut |_| Ok(()), &mut |_| {
            cancelled.store(true, Ordering::Relaxed);
            Ok(())
        }),
        Err(VectorError::Cancelled)
    ));
    let mut translated = path.clone();
    for knot in &mut translated.subpaths[0].knots {
        knot.anchor.x -= 20.;
        knot.anchor.y -= 20.;
        knot.in_ctrl = knot.anchor;
        knot.out_ctrl = knot.anchor;
    }
    let moved = antialiased_rectangle_stroke_descriptor(&translated, &style).unwrap();
    let negative = render_solid(&translated, &settings, moved, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    // Stock storage truncation is not translation-invariant across zero;
    // compare corresponding interior geometry rather than different edge crops.
    assert_eq!(moved, Rect::new(-11, -9, 28, 18));
    assert_eq!(output.try_read_interleaved(Rect::new(10, 12, 40, 32)).unwrap(), negative.try_read_interleaved(Rect::new(-10, -8, 20, 12)).unwrap());
}

#[test]
fn noaa_bevel_diagonal_boundary_limits_winding_origin_budget_and_cancel() {
    let path = Path::new(vec![Subpath::polygon(&[(10., 12.), (46., 12.), (46., 36.), (10., 36.)])]);
    let style = photocraft_vector::StrokeStyle {
        width: 6.,
        cap: photocraft_doc::LineCap::Butt,
        join: photocraft_doc::LineJoin::Bevel,
        miter_limit: 10.,
        dashes: vec![],
        dash_offset: 0.,
    };
    let settings = SolidSettings { fill: None, stroke: Some(SolidStroke { color: [1.; 4], style: style.clone() }), antialias: false };
    let bounds = Rect::new(7, 9, 49, 39);
    let mut held = 0usize;
    let rendered = render_solid(
        &path,
        &settings,
        bounds,
        PixelFormat::RGBA8,
        &Interrupt::NONE,
        &mut |n| {
            held += n;
            if held > 4 * 1024 * 1024 { Err(VectorError::Budget) } else { Ok(()) }
        },
        &mut |_| Ok(()),
    )
    .unwrap();
    let alpha = |x, y| rendered.try_read_interleaved(Rect::new(x, y, x + 1, y + 1)).unwrap()[3];
    assert_eq!([alpha(9, 9), alpha(46, 9), alpha(48, 36), alpha(7, 36), alpha(13, 15)], [0, 0, 255, 255, 0]);
    let mut moved = path.clone();
    moved.subpaths[0].knots.reverse();
    for k in &mut moved.subpaths[0].knots {
        k.anchor.x -= 20.;
        k.anchor.y -= 20.;
        k.in_ctrl = k.anchor;
        k.out_ctrl = k.anchor;
    }
    let relocated = Rect::new(-13, -11, 29, 19);
    let moved_pixels = render_solid(&moved, &settings, relocated, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    assert_eq!(rendered.try_read_interleaved(bounds).unwrap(), moved_pixels.try_read_interleaved(relocated).unwrap());
    assert!(photocraft_vector::xcf::antialiased_rectangle_stroke_descriptor(&path, &style).is_err());
    assert!(matches!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Err(VectorError::Budget), &mut |_| Ok(())),
        Err(VectorError::Budget)
    ));
    assert!(matches!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::cancel_only(&|| true), &mut |_| Ok(()), &mut |_| Ok(())),
        Err(VectorError::Cancelled)
    ));
    let mut fractional = path.clone();
    for k in &mut fractional.subpaths[0].knots {
        k.anchor.x += 0.25;
        k.in_ctrl = k.anchor;
        k.out_ctrl = k.anchor;
    }
    assert!(photocraft_vector::xcf::aliased_rectangle_stroke_descriptor(&fractional, &style).is_err());
}

#[test]
fn noaa_round_rectangle_geometry_budget_cancel_and_rejected_aa() {
    let path = Path::new(vec![Subpath::polygon(&[(10., 12.), (46., 12.), (46., 36.), (10., 36.)])]);
    let style = photocraft_vector::StrokeStyle {
        width: 6.,
        cap: photocraft_doc::LineCap::Butt,
        join: photocraft_doc::LineJoin::Round,
        miter_limit: 10.,
        dashes: vec![],
        dash_offset: 0.,
    };
    let settings = SolidSettings { fill: None, stroke: Some(SolidStroke { color: [1.; 4], style: style.clone() }), antialias: false };
    let bounds = Rect::new(7, 9, 49, 39);
    let surface = render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            let px = f64::from(x) + 0.5;
            let py = f64::from(y) + 0.5;
            let dx = (10. - px).max(0.).max(px - 46.);
            let dy = (12. - py).max(0.).max(py - 36.);
            let outer = dx * dx + dy * dy < 9.;
            let inner = (13. ..43.).contains(&px) && (15. ..33.).contains(&py);
            assert_eq!(surface.try_read_interleaved(Rect::new(x, y, x + 1, y + 1)).unwrap()[3], if outer && !inner { 255 } else { 0 });
        }
    }
    assert!(photocraft_vector::xcf::antialiased_rectangle_stroke_descriptor(&path, &style).is_err());
    assert!(matches!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Err(VectorError::Budget), &mut |_| Ok(())),
        Err(VectorError::Budget)
    ));
    assert!(matches!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::cancel_only(&|| true), &mut |_| Ok(()), &mut |_| Ok(())),
        Err(VectorError::Cancelled)
    ));
}

#[test]
#[ignore = "requires own independently authored cubic-fill controls and fresh saved native planes"]
fn genuine_curved_source_controls_preserved_and_unqualified_fill_rejected() {
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/curved-fill-control");
    for (phase, hash, tx, ty) in [
        ("source", "81465c057e8afd184e06eb8493dd03904fd5d11238b599cb251c06ce4e62f1b4", 0., 0.),
        ("edit", "739bede6c52fa8314fedd06d8b845cba7e5eed840b66389acd9a89bfa7a10647", 2., -1.),
    ] {
        let bytes = std::fs::read(base.join(format!("{phase}.xcf"))).unwrap();
        assert_eq!(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), hash);
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!((file.version, file.precision), (24, 150));
        let layer = file.layers.iter().find(|l| l.vector.is_some()).unwrap();
        let vector = layer.vector.as_ref().unwrap();
        assert!(!vector.source_modified && vector.filled && !vector.stroked && !vector.fill.antialias);
        let path = file.paths.iter().find(|p| p.tattoo == Some(vector.path_tattoo)).unwrap();
        assert_eq!(path.strokes.len(), 1);
        assert!(path.strokes[0].closed);
        let points = &path.strokes[0].points;
        assert_eq!(points.len(), 12);
        let point = |i: usize| Point::new(f64::from(f32::from_bits(points[i].components[0])), f64::from(f32::from_bits(points[i].components[1])));
        assert_eq!(point(1), Point::new(10. + tx, 12. + ty));
        assert_eq!(point(2), Point::new(20. + tx, 4. + ty));
        assert_eq!(point(3), Point::new(36. + tx, 4. + ty));
        assert_eq!(point(4), Point::new(46. + tx, 12. + ty));
        let knots = points
            .as_chunks::<3>().0.iter()
            .map(|p| {
                assert_eq!([p[0].kind, p[1].kind, p[2].kind], [1, 0, 1]);
                let convert =
                    |p: &photocraft_xcf::PathPoint| Point::new(f64::from(f32::from_bits(p.components[0])), f64::from(f32::from_bits(p.components[1])));
                Knot { in_ctrl: convert(&p[0]), anchor: convert(&p[1]), out_ctrl: convert(&p[2]), smooth: false }
            })
            .collect();
        let canonical = Path::new(vec![Subpath { knots, closed: true, op: PathOp::Combine }]);
        let fresh = std::fs::read(base.join(format!("{phase}-reopened.layer.native.bin"))).unwrap();
        assert_eq!(fresh, std::fs::read(base.join(format!("round2/{phase}-reopened.layer.native.bin"))).unwrap());
        assert_eq!(layer.plane.as_ref().unwrap().bytes, fresh);
        assert_eq!(layer.offset, (10 + tx as i32, 6 + ty as i32));
        assert_eq!((layer.width, layer.height), (36, 30));
        let bounds = Rect::from_xywh(layer.offset.0, layer.offset.1, layer.width, layer.height);
        let settings = SolidSettings { fill: Some([1.; 4]), stroke: None, antialias: false };
        assert!(
            matches!(
                render_solid(&canonical, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())),
                Err(VectorError::Unsupported(_))
            ),
            "curves must not be approximated as a rectangle or ideal fill before independent closure"
        );
    }
}
#[test]
#[ignore = "requires own reversed negative-origin odd Butt/Miter witnesses"]
fn original_noaa_odd_negative_stroke_source_edit_match_gimp() {
    verify_stroke_pair(
        "odd-miter-negative-control",
        ["08dd7dc3d941ab569f7f43b7602a89130da9de5d69fea15d34ee7d6b14b72994", "07c87a4da83e94b7a205e9d23c4907bb4396a164e99bc29c001a22e010ce39e7"],
        [7., 9.],
        false,
        0,
    );
}
#[test]
fn odd_storage_negative_origin_uses_observed_truncation() {
    let path = Path::new(vec![Subpath::polygon(&[(-10., -8.), (26., -8.), (26., 16.), (-10., 16.)])]);
    let style = photocraft_vector::StrokeStyle {
        width: 7.,
        cap: photocraft_doc::LineCap::Butt,
        join: photocraft_doc::LineJoin::Miter,
        miter_limit: 10.,
        dashes: vec![],
        dash_offset: 0.,
    };
    assert_eq!(photocraft_vector::xcf::aliased_rectangle_stroke_storage_descriptor(&path, &style).unwrap(), Rect::new(-13, -11, 30, 20));
}

#[test]
#[ignore = "requires own public horizontal open Butt stroke source/edit native witnesses"]
fn original_open_horizontal_stroke_source_edit_match_gimp() {
    verify_stroke_pair_with_closed(
        "open-axis-horizontal",
        ["476bdd82d260b5db84b4a81e14f79ffa05b9f46b4dce119eb1ddad0678bdd2a5", "5b452db619638bad90de04da7773322ab81713ea8a1ef95e2a557d568871b256"],
        [6., 8.],
        false,
        0,
        false,
    );
}
#[test]
#[ignore = "requires own public vertical open Butt stroke source/edit native witnesses"]
fn original_open_vertical_stroke_source_edit_match_gimp() {
    verify_stroke_pair_with_closed(
        "open-axis-vertical",
        ["c5fb3f6e217b7c1bd79e5b3f04ec60f066c6a30230f9fe4c3c52aba26f809ac0", "be2823fc132218511e98f51bd78e3d6468429f7d95ff4654cdbac98bea2129f1"],
        [6., 8.],
        false,
        0,
        false,
    );
}
#[test]
fn open_axis_butt_padding_direction_bounds_budget_and_cancel() {
    use photocraft_vector::xcf::open_axis_stroke_descriptor;
    let mut sub = Subpath::polygon(&[(10., 24.), (46., 24.)]);
    sub.closed = false;
    let path = Path::new(vec![sub]);
    let style = photocraft_vector::StrokeStyle {
        width: 6.,
        cap: photocraft_doc::LineCap::Butt,
        join: photocraft_doc::LineJoin::Miter,
        miter_limit: 10.,
        dashes: vec![],
        dash_offset: 0.,
    };
    let bounds = open_axis_stroke_descriptor(&path, &style).unwrap();
    assert_eq!(bounds, Rect::new(7, 21, 49, 27));
    let settings = SolidSettings { fill: None, stroke: Some(SolidStroke { color: [1.; 4], style: style.clone() }), antialias: false };
    let output = render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    let alpha = |x| output.try_read_interleaved(Rect::new(x, 24, x + 1, 25)).unwrap()[3];
    assert_eq!([alpha(7), alpha(9), alpha(10), alpha(45), alpha(46), alpha(48)], [0, 0, 255, 255, 0, 0]);
    let mut reversed = path.clone();
    reversed.subpaths[0].knots.reverse();
    assert_eq!(open_axis_stroke_descriptor(&reversed, &style).unwrap(), bounds);
    assert_eq!(
        render_solid(&reversed, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(()))
            .unwrap()
            .try_read_interleaved(bounds)
            .unwrap(),
        output.try_read_interleaved(bounds).unwrap()
    );
    for kind in 0..5 {
        let mut bad = path.clone();
        match kind {
            0 => bad.subpaths[0].knots[1].anchor.y += 1.,
            1 => bad.subpaths[0].knots[0].in_ctrl.x += 1.,
            2 => bad.subpaths[0].knots[0].anchor.x = f64::NAN,
            3 => bad.subpaths[0].knots[0].anchor.x = 4097.,
            _ => bad.subpaths[0].closed = true,
        };
        assert!(open_axis_stroke_descriptor(&bad, &style).is_err());
    }
    assert!(matches!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Err(VectorError::Budget), &mut |_| Ok(())),
        Err(VectorError::Budget)
    ));
    let flag = std::sync::atomic::AtomicBool::new(false);
    let poll = || flag.load(std::sync::atomic::Ordering::Relaxed);
    let ctl = Interrupt::cancel_only(&poll);
    assert!(matches!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &ctl, &mut |_| Ok(()), &mut |_| {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }),
        Err(VectorError::Cancelled)
    ));
}
#[test]
#[ignore = "requires own reversed negative-origin open Butt source/edit native witnesses"]
fn original_open_negative_stroke_source_edit_match_gimp() {
    verify_stroke_pair_with_closed(
        "open-axis-horizontal-negative",
        ["eba4f544b85a6cff8c5bcca7bf5301385fd5f8982633e035686ffd5b32cef779", "805cac9eeb452475576899d42510d281a4e0dd4ac0e27c106967130eec066717"],
        [6., 8.],
        false,
        0,
        false,
    );
}

#[test]
#[ignore = "requires independently authored square-cap horizontal and vertical originals"]
fn original_square_axis_source_edit_match_stock() {
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
        verify_stroke_pair_with_cap(&format!("square-axis-{axis}"), hashes, [6., 8.], false, 0, false, 2);
    }
}

#[test]
fn open_square_cap_extension_reversal_and_unknown_forms_are_checked() {
    let mut sub = Subpath::polygon(&[(10., 24.), (46., 24.)]);
    sub.closed = false;
    let path = Path::new(vec![sub]);
    let mut style = photocraft_vector::StrokeStyle {
        width: 6.,
        cap: photocraft_doc::LineCap::Square,
        join: photocraft_doc::LineJoin::Miter,
        miter_limit: 10.,
        dashes: vec![],
        dash_offset: 0.,
    };
    let bounds = photocraft_vector::xcf::open_axis_stroke_descriptor(&path, &style).unwrap();
    assert_eq!(bounds, Rect::new(7, 21, 49, 27));
    let settings = SolidSettings { fill: None, stroke: Some(SolidStroke { color: [1.; 4], style: style.clone() }), antialias: false };
    let bytes = render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(()))
        .unwrap()
        .try_read_interleaved(bounds)
        .unwrap();
    assert!(bytes.as_chunks::<4>().0.iter().all(|p| *p == [255; 4]));
    let mut reverse = path.clone();
    reverse.subpaths[0].knots.reverse();
    assert_eq!(
        render_solid(&reverse, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(()))
            .unwrap()
            .try_read_interleaved(bounds)
            .unwrap(),
        bytes
    );
    style.cap = photocraft_doc::LineCap::Round;
    style.join = photocraft_doc::LineJoin::Bevel;
    assert!(photocraft_vector::xcf::open_axis_stroke_descriptor(&path, &style).is_err());
    style.cap = photocraft_doc::LineCap::Square;
    style.join = photocraft_doc::LineJoin::Miter;
    style.width = 7.;
    assert!(photocraft_vector::xcf::open_axis_stroke_descriptor(&path, &style).is_err());
    assert!(matches!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Err(VectorError::Budget), &mut |_| Ok(())),
        Err(VectorError::Budget)
    ));
    let flag = std::sync::atomic::AtomicBool::new(false);
    let poll = || flag.load(std::sync::atomic::Ordering::Relaxed);
    let ctl = Interrupt::cancel_only(&poll);
    assert!(matches!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &ctl, &mut |_| Ok(()), &mut |_| {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }),
        Err(VectorError::Cancelled)
    ));
}

#[test]
#[ignore = "requires independent public Round cap originals and native repeat"]
fn original_round_axis_source_edit_match_stock() {
    verify_stroke_pair_with_cap(
        "round-axis-horizontal",
        ["82c0f1a985d771bed6dbb10f37ec612e59e266c9fdeb7787eb212229ba7f259d", "d8eacc9d7d9b5bcb245bbcaca24d7479b3729fbd7600dfdddd0f8c62dc85c680"],
        [6., 8.],
        false,
        0,
        false,
        1,
    );
    verify_stroke_pair_with_cap(
        "round-axis-vertical",
        ["2d9e632fa527fada80b3bc142d28b6ba204499a016088fb54eff81745491b832", "849bd180f564843f8c2872388dbd6f5e4d6cfb134b3ea2a06a9dee874c0de126"],
        [6., 8.],
        false,
        0,
        false,
        1,
    );
    verify_stroke_pair_with_cap(
        "round-axis-horizontal-negative",
        ["210f4b96c53aa6a8500d797a7a3d86c78147811019b647a36a0f9c478b29b994", "5cc5bd70200bcd35acd833552aa9ebeb70bcaef588600ad21a2b9dce3e12e768"],
        [6., 8.],
        false,
        0,
        false,
        1,
    );
}

#[test]
fn open_round_capsule_is_bounded_and_preserves_empty_corners() {
    let mut sub = Subpath::polygon(&[(10., 24.), (46., 24.)]);
    sub.closed = false;
    let path = Path::new(vec![sub]);
    let style = photocraft_vector::StrokeStyle {
        width: 6.,
        cap: photocraft_doc::LineCap::Round,
        join: photocraft_doc::LineJoin::Miter,
        miter_limit: 10.,
        dashes: vec![],
        dash_offset: 0.,
    };
    let bounds = photocraft_vector::xcf::open_axis_stroke_descriptor(&path, &style).unwrap();
    let settings = SolidSettings { fill: None, stroke: Some(SolidStroke { color: [1.; 4], style }), antialias: false };
    let surface = render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    assert_eq!(surface.try_read_interleaved(Rect::new(7, 21, 8, 22)).unwrap(), [0; 4]);
    assert_eq!(surface.try_read_interleaved(Rect::new(7, 23, 8, 24)).unwrap(), [255; 4]);
    let mut reverse = path.clone();
    reverse.subpaths[0].knots.reverse();
    assert_eq!(
        render_solid(&reverse, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(()))
            .unwrap()
            .try_read_interleaved(bounds)
            .unwrap(),
        surface.try_read_interleaved(bounds).unwrap()
    );
    assert!(matches!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Err(VectorError::Budget), &mut |_| Ok(())),
        Err(VectorError::Budget)
    ));
    let flag = std::sync::atomic::AtomicBool::new(false);
    let poll = || flag.load(std::sync::atomic::Ordering::Relaxed);
    let ctl = Interrupt::cancel_only(&poll);
    assert!(matches!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &ctl, &mut |_| Ok(()), &mut |_| {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }),
        Err(VectorError::Cancelled)
    ));
    let mut bad = settings.clone();
    bad.antialias = true;
    assert!(render_solid(&path, &bad, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).is_err());
    bad.antialias = false;
    bad.stroke.as_mut().unwrap().color[3] = 0.5;
    assert!(render_solid(&path, &bad, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).is_err());
    assert!(
        photocraft_vector::xcf::render_solid_with_transform(
            &path,
            &settings,
            bounds,
            PixelFormat::RGBA8,
            &Interrupt::NONE,
            &mut |_| Ok(()),
            &mut |_| Ok(()),
            &|_| Ok([f64::NAN; 3]),
            &|v| Ok(v)
        )
        .is_err()
    );
}

#[test]
fn monotone_collinear_butt_anchors_preserve_ink_and_reject_cusps() {
    use photocraft_vector::xcf::open_axis_stroke_descriptor;
    let style = photocraft_vector::StrokeStyle {
        width: 6.,
        cap: photocraft_doc::LineCap::Butt,
        join: photocraft_doc::LineJoin::Miter,
        miter_limit: 10.,
        dashes: vec![],
        dash_offset: 0.,
    };
    for coords in [vec![(10., 24.), (28., 24.), (46., 24.)], vec![(28., 8.), (28., 16.), (28., 32.), (28., 40.)]] {
        let mut sub = Subpath::polygon(&coords);
        sub.closed = false;
        let path = Path::new(vec![sub]);
        let bounds = open_axis_stroke_descriptor(&path, &style).unwrap();
        let mut two = path.clone();
        two.subpaths[0].knots = vec![path.subpaths[0].knots[0], *path.subpaths[0].knots.last().unwrap()];
        let settings = SolidSettings { fill: None, stroke: Some(SolidStroke { color: [1.; 4], style: style.clone() }), antialias: false };
        let render = |p: &Path| {
            render_solid(p, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(()))
                .unwrap()
                .try_read_interleaved(bounds)
                .unwrap()
        };
        assert_eq!(render(&path), render(&two));
        let mut reversed = path.clone();
        reversed.subpaths[0].knots.reverse();
        assert_eq!(render(&reversed), render(&two));
        for kind in 0..4 {
            let mut bad = path.clone();
            match kind {
                0 => bad.subpaths[0].knots[1] = bad.subpaths[0].knots[0],
                1 => bad.subpaths[0].knots.swap(0, 1),
                2 => bad.subpaths[0].knots[1].out_ctrl.x += 1.,
                _ => {
                    if coords[0].1 == coords[1].1 {
                        bad.subpaths[0].knots[1].anchor.y += 1.
                    } else {
                        bad.subpaths[0].knots[1].anchor.x += 1.
                    }
                }
            };
            assert!(open_axis_stroke_descriptor(&bad, &style).is_err());
        }
        for cap in [photocraft_doc::LineCap::Square, photocraft_doc::LineCap::Round] {
            let mut bad = style.clone();
            bad.cap = cap;
            assert_eq!(open_axis_stroke_descriptor(&path, &bad).unwrap(), bounds);
            let settings = SolidSettings { fill: None, stroke: Some(SolidStroke { color: [1.; 4], style: bad }), antialias: false };
            let render_cap = |p: &Path| {
                render_solid(p, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(()))
                    .unwrap()
                    .try_read_interleaved(bounds)
                    .unwrap()
            };
            assert_eq!(render_cap(&path), render_cap(&two));
        }
    }
}

#[test]
#[ignore = "requires own public collinear three/four-anchor Butt source/edit witnesses"]
fn original_monotone_collinear_strokes_match_stock() {
    verify_stroke_pair_with_closed(
        "collinear-axis-horizontal",
        ["15f8cecddf4dc5224d61291d6464968273c1d8f97f1ad173442b78efcf61fc20", "a907613b56e9d3d20a9d41ccc8e26460074d6a4885c35ac731c0b8e9b789dd36"],
        [6., 8.],
        false,
        0,
        false,
    );
    verify_stroke_pair_with_closed(
        "collinear-axis-vertical",
        ["805f1015fae0e9d2ab7138e7cbbc55162aced99341777ef1029a9726d554f71e", "629f4bdea5e0f4ea01b09626e43496c338ad0ed730670d954e33d195e2ed263d"],
        [6., 8.],
        false,
        0,
        false,
    );
}

#[test]
#[ignore = "requires own public collinear Round/Square cap originals"]
fn original_monotone_collinear_caps_match_stock() {
    verify_stroke_pair_with_cap(
        "collinear-square-axis-horizontal",
        ["01cc1f3a20f5aece86e3869f4d4ec508d29af3d8587f0ca4748832477a697043", "f38de78440f4c4c978a30756a6229844709115e30a5a380ca058b238198ea26e"],
        [6., 8.],
        false,
        0,
        false,
        2,
    );
    verify_stroke_pair_with_cap(
        "collinear-square-axis-vertical",
        ["58c991617c5a0668cf3f6551d64b1aaa87d7afdc3a9b223b92a5955b2d39e8cc", "fad02843ba1faf8c407f013f21bc21cdc99ba3d4f522a01a8bfef98a19df942b"],
        [6., 8.],
        false,
        0,
        false,
        2,
    );
    verify_stroke_pair_with_cap(
        "collinear-round-axis-horizontal",
        ["6755c57de57797727230cd6d0929b7a83280d6e0f58377d33b6e03e9b8affd9a", "6e4145b88b344e507eeaebe3eddef6556894714de0e7e46fa3c4f94f5c9c8ab5"],
        [6., 8.],
        false,
        0,
        false,
        1,
    );
    verify_stroke_pair_with_cap(
        "collinear-round-axis-vertical",
        ["8bcd3908fd0934355f6660a68ab0d26eca9cb61e6658fb12df593bc6c408db06", "a4340734285eec74329e76ecc85528a6fa0fb1e4a7160aaecca06ea5e82a7086"],
        [6., 8.],
        false,
        0,
        false,
        1,
    );
}

#[test]
#[ignore = "requires own public noAA odd-width Square caps source/edit witnesses"]
fn original_odd_collinear_square_caps_match_stock() {
    verify_stroke_pair_with_cap(
        "odd-collinear-square-axis-horizontal",
        ["c15fc236c4f3f29785d676c186be1c84f4e2b1e2d3dfac7b5b651a01a628bdee", "47367b671df00266f84e3ba2857711c6c159ef504c6da78ba5df2a9d22498bae"],
        [3., 5.],
        false,
        0,
        false,
        2,
    );
    verify_stroke_pair_with_cap(
        "odd-collinear-square-axis-vertical",
        ["7d0dffa4039734f7792de2a6191ea34fedbb30de0db5d0cb1496ced8b9fcb5da", "40b70d3d5c5cfbec2f972b2b8a3d70409a2176f6662ccdf306bc7f613ad57d8d"],
        [3., 5.],
        false,
        0,
        false,
        2,
    );
}

#[test]
fn odd_caps_widths_are_exactly_scoped_and_unqualified_forms_stay_closed() {
    let mut sub = Subpath::polygon(&[(10., 24.), (28., 24.), (46., 24.)]);
    sub.closed = false;
    let path = Path::new(vec![sub]);
    for width in [3., 5.] {
        let mut style = photocraft_vector::StrokeStyle {
            width,
            cap: photocraft_doc::LineCap::Square,
            join: photocraft_doc::LineJoin::Miter,
            miter_limit: 10.,
            dashes: vec![],
            dash_offset: 0.,
        };
        let b = photocraft_vector::xcf::open_axis_stroke_descriptor(&path, &style).unwrap();
        assert_eq!(b.height(), width as u32);
        assert_eq!(b.width(), 36 + width as u32);
        style.cap = photocraft_doc::LineCap::Round;
        assert_eq!(photocraft_vector::xcf::open_axis_stroke_descriptor(&path, &style).unwrap(), b);
        style.cap = photocraft_doc::LineCap::Butt;
        assert_eq!(photocraft_vector::xcf::open_axis_stroke_descriptor(&path, &style).unwrap(), b);
    }
    for width in [1., 2., 4., 7., 9., f64::NAN, f64::INFINITY] {
        let style = photocraft_vector::StrokeStyle {
            width,
            cap: photocraft_doc::LineCap::Square,
            join: photocraft_doc::LineJoin::Miter,
            miter_limit: 10.,
            dashes: vec![],
            dash_offset: 0.,
        };
        assert!(photocraft_vector::xcf::open_axis_stroke_descriptor(&path, &style).is_err());
    }
}

#[test]
#[ignore = "requires independent odd Round body/caps boundary and reversed negative holdouts"]
fn original_odd_round_body_caps_union_matches_stock() {
    verify_stroke_pair_with_cap(
        "odd-collinear-round-axis-horizontal",
        ["b43986c04d6728581beeff1c2927ab04ad82dad2602e2d963b77a7006efc7a8d", "8a4ecb857aa0a66dd89e4316b1dbdac3c9cd50427a71b9568444da19e7ec3a88"],
        [3., 5.],
        false,
        0,
        false,
        1,
    );
    verify_stroke_pair_with_cap(
        "odd-collinear-round-axis-vertical",
        ["d91ba628a74180e311c8da83394336c4cd7b958845c5f86d5d94fa5dbd7132c9", "8c87eea154373aa64e81a891689ca0dc52d45b14c3eb5e11449fb0a4b0586e9d"],
        [3., 5.],
        false,
        0,
        false,
        1,
    );
    verify_stroke_pair_with_cap(
        "odd-round-holdout-horizontal",
        ["5294e6574dd619b49866a6f63c6a2d9d74c6809086f40308b57c912ce156eec8", "6d0066b9900c9a7d5b8266f7504987f0bc5fbd2b5bd3a4d57423c66356efcbf8"],
        [3., 5.],
        false,
        0,
        false,
        1,
    );
    verify_stroke_pair_with_cap(
        "odd-round-holdout-vertical",
        ["8361ce78c147e640eb060f43bb5be0fb768d6d12697665d11d96103858461150", "e3ad108c81dca7128853bb99130f1176dae26bfea8715d2b3832bc5f724dd3ca"],
        [3., 5.],
        false,
        0,
        false,
        1,
    );
}

#[test]
#[ignore = "requires own independent odd-width Butt source/edit witnesses"]
fn original_odd_collinear_butt_caps_match_stock() {
    verify_stroke_pair_with_cap(
        "odd-collinear-butt-axis-horizontal",
        ["1026f953c993c0eeeb4caf3a9f5f3b67f4a7ec5c498212fb547335fe3cf686d7", "341690961b694ae2f20ac5425807ba03ee50e8b2a13e1a1ee2a922b3df3ac7c0"],
        [3., 5.],
        false,
        0,
        false,
        0,
    );
    verify_stroke_pair_with_cap(
        "odd-collinear-butt-axis-vertical",
        ["6f4f95ec8a1a87ab8b8b548faa6d5380f2c3bd16aa112e79dfc6106132ad06e1", "d1239fd06d7d737608259beb4ca25dc51e5bba514f5eb2822a06931f0d7632d2"],
        [3., 5.],
        false,
        0,
        false,
        0,
    );
}

#[test]
#[ignore = "requires independent reversed-negative odd-width Butt holdouts"]
fn original_odd_collinear_butt_negative_holdouts_match_stock() {
    verify_stroke_pair_with_cap(
        "odd-butt-holdout-horizontal",
        ["ea4ec230ebf26062362ba0566906490bfd21b93197697ecddb199e90d6d33a63", "175e932c5f37ce16fdaaa20a423a2398bf27a08662d6b73c7b822d5583a23921"],
        [3., 5.],
        false,
        0,
        false,
        0,
    );
    verify_stroke_pair_with_cap(
        "odd-butt-holdout-vertical",
        ["298a36b866fdd215e3678bb3ff343b3854162877770bf58865ea984cb08a5d3e", "8a6eb2182e1cba768a90b7e7452c32cee3fe573352fee3412d49765426f2fc22"],
        [3., 5.],
        false,
        0,
        false,
        0,
    );
}
