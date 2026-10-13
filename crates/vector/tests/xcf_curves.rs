use photocraft_doc::{Knot, Path, PathOp, Subpath};
use photocraft_geom::Point;
use photocraft_raster::Interrupt;
use photocraft_vector::xcf::{VectorError, tessellate_curves_bounded};

fn bowed() -> Path {
    let p = |x, y| Point::new(x, y);
    Path::new(vec![Subpath {
        closed: true,
        op: PathOp::Combine,
        knots: vec![
            Knot { anchor: p(10., 12.), in_ctrl: p(10., 12.), out_ctrl: p(20., 4.), smooth: false },
            Knot { anchor: p(46., 12.), in_ctrl: p(36., 4.), out_ctrl: p(46., 12.), smooth: false },
            Knot { anchor: p(46., 36.), in_ctrl: p(46., 36.), out_ctrl: p(46., 36.), smooth: false },
            Knot { anchor: p(10., 36.), in_ctrl: p(10., 36.), out_ctrl: p(10., 36.), smooth: false },
        ],
    }])
}
#[test]
fn fixed_grid_observed_vertices_and_source_immutable() {
    let path = bowed();
    let before = path.clone();
    let mut charged = 0;
    let mut worked = 0;
    let result = tessellate_curves_bounded(
        &path,
        &Interrupt::NONE,
        &mut |n| {
            charged += n;
            Ok(())
        },
        &mut |n| {
            worked += n;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(path, before);
    assert_eq!(result.len(), 1);
    assert!(result[0].closed);
    assert_eq!(result[0].operation, PathOp::Combine);
    assert_eq!(result[0].points.len(), 19);
    assert_eq!(&result[0].points[..3], &[Point::new(10., 12.), Point::new(11.94140625, 10.59375), Point::new(14.0078125, 9.375)]);
    assert!(charged > 0 && charged < 128 * 1024);
    assert!(worked > 0);
}

#[test]
fn negative_grid_translation_and_open_contour_identity() {
    let path = bowed();
    let expected = tessellate_curves_bounded(&path, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    let mut moved = path.clone();
    for k in &mut moved.subpaths[0].knots {
        for p in [&mut k.anchor, &mut k.in_ctrl, &mut k.out_ctrl] {
            p.x -= 20.;
            p.y -= 20.;
        }
    }
    let result = tessellate_curves_bounded(&moved, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    assert_eq!(result[0].points, expected[0].points.iter().map(|p| Point::new(p.x - 20., p.y - 20.)).collect::<Vec<_>>());
    moved.subpaths[0].closed = false;
    moved.subpaths[0].op = PathOp::Subtract;
    let result = tessellate_curves_bounded(&moved, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
    assert!(!result[0].closed);
    assert_eq!(result[0].operation, PathOp::Subtract);
    assert_eq!(result[0].points.last(), Some(&moved.subpaths[0].knots.last().unwrap().anchor));
}

#[test]
fn hostile_structure_coordinates_budgets_and_cancellation_fail_without_mutation() {
    let path = bowed();
    for value in [f64::NAN, f64::INFINITY, 4097., 0.1] {
        let mut invalid = path.clone();
        invalid.subpaths[0].knots[0].out_ctrl.x = value;
        assert!(matches!(tessellate_curves_bounded(&invalid, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())), Err(VectorError::Unsupported(_))));
    }
    let mut huge = path.clone();
    huge.subpaths = vec![huge.subpaths[0].clone(); 17];
    assert!(tessellate_curves_bounded(&huge, &Interrupt::NONE, &mut |_| panic!("invalid structure allocated"), &mut |_| Ok(())).is_err());
    let before = path.clone();
    assert!(matches!(tessellate_curves_bounded(&path, &Interrupt::NONE, &mut |_| Err(VectorError::Budget), &mut |_| Ok(())), Err(VectorError::Budget)));
    assert!(matches!(tessellate_curves_bounded(&path, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Err(VectorError::Budget)), Err(VectorError::Budget)));
    assert!(matches!(tessellate_curves_bounded(&path, &Interrupt::cancel_only(&|| true), &mut |_| Ok(()), &mut |_| Ok(())), Err(VectorError::Cancelled)));
    use std::sync::atomic::{AtomicUsize, Ordering};
    let steps = AtomicUsize::new(0);
    let cancel = || steps.load(Ordering::Relaxed) > 10;
    assert!(matches!(
        tessellate_curves_bounded(&path, &Interrupt::cancel_only(&cancel), &mut |_| Ok(()), &mut |_| {
            steps.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }),
        Err(VectorError::Cancelled)
    ));
    assert_eq!(path, before);
}

#[test]
fn oscillating_curve_vertex_limit_is_explicit_and_work_is_bounded() {
    let k = Knot { anchor: Point::new(0., 0.), in_ctrl: Point::new(-4096., 4096.), out_ctrl: Point::new(4096., 4096.), smooth: false };
    let path = Path::new(vec![Subpath { knots: vec![k; 32], closed: true, op: PathOp::Combine }]);
    let mut work = 0;
    let result = tessellate_curves_bounded(&path, &Interrupt::NONE, &mut |_| Ok(()), &mut |n| {
        work += n;
        Ok(())
    });
    assert!(matches!(result, Err(VectorError::Unsupported(_))));
    assert!(work < 100_000);
}

#[test]
fn flattened_geometry_does_not_silently_enable_unqualified_curve_coverage() {
    use photocraft_color::PixelFormat;
    use photocraft_geom::Rect;
    use photocraft_vector::xcf::{AliasedFillRule, SolidSettings, render_solid_with_rule};
    let path = bowed();
    let original = path.clone();
    let settings = SolidSettings { fill: Some([1., 0., 0., 1.]), stroke: None, antialias: false };
    let mut charged = 0;
    let result = render_solid_with_rule(
        &path,
        &settings,
        Rect::new(0, 0, 64, 48),
        PixelFormat::RGBA8,
        &Interrupt::NONE,
        &mut |bytes| {
            charged += bytes;
            Ok(())
        },
        &mut |_| Ok(()),
        AliasedFillRule::EvenOdd,
    );
    assert!(matches!(result, Err(VectorError::Unsupported(_))));
    // The existing checked renderer admits bounded control scratch first, but
    // rejects before interleaved/tiled output and rasterizer allocation charges.
    assert_eq!(charged, 64 * std::mem::size_of::<[Point; 4]>());
    assert_eq!(path, original);
}

#[test]
#[ignore = "requires own original cubic source/edit plus independent public flattened-path captures"]
fn fourteen_original_curved_paths_match_public_observed_vertices_exactly() {
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa");
    let bytes = std::fs::read(base.join("curved-cairo-public-observations.json")).unwrap();
    let sha = |b: &[u8]| Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect::<String>();
    assert_eq!(sha(&bytes), "8f71715b94d242d1a86e789a1df406e03b84e439c4afacd798c8cdc0b10a191f");
    let capture: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let rows = capture["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 14);
    for row in rows {
        let family = row["family"].as_str().unwrap();
        let phase = row["phase"].as_str().unwrap();
        let dir = if family == "curved-fill-control" { base.join(family) } else { base.join("curved-holdouts").join(family) };
        let original = std::fs::read(dir.join(format!("{phase}.xcf"))).unwrap();
        assert_eq!(sha(&original), row["source_sha256"].as_str().unwrap());
        let file = photocraft_xcf::decode(&original, &Default::default()).unwrap();
        let layer = file.layers.iter().find(|l| l.vector.is_some()).unwrap();
        let source = layer.vector.as_ref().unwrap();
        assert!(!source.source_modified && source.filled && !source.stroked && !source.fill.antialias);
        let saved = file.paths.iter().find(|p| p.tattoo == Some(source.path_tattoo)).unwrap();
        assert_eq!(saved.strokes.len(), 1);
        let stroke = &saved.strokes[0];
        let point = |p: &photocraft_xcf::PathPoint| Point::new(f64::from(f32::from_bits(p.components[0])), f64::from(f32::from_bits(p.components[1])));
        let knots = stroke
            .points
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| {
                assert_eq!([p[0].kind, p[1].kind, p[2].kind], [1, 0, 1]);
                Knot { in_ctrl: point(&p[0]), anchor: point(&p[1]), out_ctrl: point(&p[2]), smooth: false }
            })
            .collect();
        let current = Path::new(vec![Subpath { knots, closed: stroke.closed, op: PathOp::Combine }]);
        let actual = tessellate_curves_bounded(&current, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        let mut expected: Vec<Point> = row["flat_public"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| matches!(r["kind"].as_u64(), Some(0 | 1)))
            .map(|r| Point::new(r["coordinates"][0].as_f64().unwrap(), r["coordinates"][1].as_f64().unwrap()))
            .collect();
        while expected.len() > 1 && expected.last() == expected.first() {
            expected.pop();
        }
        assert_eq!(actual[0].points, expected, "{family}/{phase}");
        assert_eq!(layer.plane.as_ref().unwrap().bytes, std::fs::read(dir.join(format!("{phase}-reopened.layer.native.bin"))).unwrap());
        assert_eq!(
            std::fs::read(dir.join(format!("{phase}-reopened.layer.native.bin"))).unwrap(),
            std::fs::read(dir.join(format!("round2/{phase}-reopened.layer.native.bin"))).unwrap()
        );
    }
}
