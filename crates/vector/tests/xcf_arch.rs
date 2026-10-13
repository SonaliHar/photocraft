use photocraft_color::PixelFormat;
use photocraft_doc::{FillRule, Knot, Path, PathOp, Subpath};
use photocraft_geom::{Point, Rect};
use photocraft_raster::Interrupt;
use photocraft_vector::xcf::{SolidSettings, render_solid};
use sha2::{Digest, Sha256};
fn arch() -> Path {
    let a = Point::new(8., 32.);
    let d = Point::new(54., 32.);
    Path {
        subpaths: vec![Subpath {
            knots: vec![
                Knot { anchor: a, in_ctrl: a, out_ctrl: Point::new(18., 8.), smooth: false },
                Knot { anchor: d, in_ctrl: Point::new(40., 8.), out_ctrl: d, smooth: false },
            ],
            closed: true,
            op: PathOp::Combine,
        }],
        fill_rule: FillRule::NonZero,
        inverted: false,
    }
}
#[test]
fn arch_scope_and_simultaneous_storage_are_checked() {
    use photocraft_vector::xcf::{VectorError, monotone_arch_descriptor};
    let path = arch();
    let settings = SolidSettings { fill: Some([1., 0., 0., 1.]), stroke: None, antialias: false };
    let bounds = Rect::new(0, 0, 64, 48);
    let render = |p: &Path, s: &SolidSettings| render_solid(p, s, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(()));
    assert!(render(&path, &settings).is_ok());
    let mut bad = path.clone();
    for coordinate in [f64::NAN, f64::INFINITY, 1e20, 18.001] {
        bad.subpaths[0].knots[0].out_ctrl.x = coordinate;
        assert!(monotone_arch_descriptor(&bad).is_err());
    }
    bad = path.clone();
    bad.subpaths[0].knots[0].out_ctrl.x = 45.;
    assert!(render(&bad, &settings).is_err());
    bad = path.clone();
    bad.subpaths[0].knots[0].in_ctrl.y = 31.;
    assert!(render(&bad, &settings).is_err());
    assert!(render(&path, &SolidSettings { fill: Some([1., 0., 0., 0.5]), ..settings.clone() }).is_err());
    // A complete destination tile and staging bytes coexist, not merely 64x48 pixels.
    let mut charged = 0usize;
    let result = render_solid(
        &path,
        &settings,
        bounds,
        PixelFormat::RGBA8,
        &Interrupt::NONE,
        &mut |n| {
            charged = charged.checked_add(n).ok_or(VectorError::Budget)?;
            if charged > 256 * 256 * 4 { Err(VectorError::Budget) } else { Ok(()) }
        },
        &mut |_| Ok(()),
    );
    assert_eq!(result.unwrap_err(), VectorError::Budget);
    assert!(charged > 256 * 256 * 4);
    assert_eq!(
        render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Err(VectorError::Budget)).unwrap_err(),
        VectorError::Budget
    );
}
#[test]
fn arch_mid_work_cancellation_returns_no_surface() {
    use photocraft_vector::xcf::VectorError;
    use std::sync::atomic::{AtomicBool, Ordering};
    let stop = AtomicBool::new(false);
    let cancelled = || stop.load(Ordering::Relaxed);
    let ctl = Interrupt::cancel_only(&cancelled);
    let result = render_solid(
        &arch(),
        &SolidSettings { fill: Some([1., 0., 0., 1.]), stroke: None, antialias: false },
        Rect::new(0, 0, 64, 48),
        PixelFormat::RGBA8,
        &ctl,
        &mut |_| Ok(()),
        &mut |_| {
            stop.store(true, Ordering::Relaxed);
            Ok(())
        },
    );
    assert_eq!(result.unwrap_err(), VectorError::Cancelled);
}
#[test]
#[ignore = "requires own independent public GIMP cubic arch source/edit files"]
fn original_monotone_arch_current_geometry_matches_stock() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-vector-aa/monotone-arch-stock");
    let hashes = [
        (
            "asymmetric-fractional",
            ["08df3e3592ed2348fd7ed8a1fe1b6115d8a6da95887b54d8af9b51385a7d198b", "0b50a7895e90935f7bd46136821ecbce01f649437ad0b02f97289840f0a9827d"],
        ),
        ("basic", ["87a5c88293ebb4fb4ff22bd4917b2025ee97fe47f6748b8ed39ff5675ae5d8ae", "af47dc5bca9e1ce5187049747d33b443c6b36c42ae8a5b0178fda4ea17ba60e5"]),
        ("reversed", ["c92168afbd592b39332ed839752a3f9a6c51de94610ab8e91157d8ff39eb0fc8", "a7e7d0a7d5cf9721bd9d7837b95f4553134a903212dadaf9eda09320d43828d8"]),
        ("wide", ["2dd9e57a259469c195d61de372fd59934635746a50364c9a180cb201b7cbc1ba", "0dda408e632def57f6636a0060d027f5a74476abf7c01ea017851038140dd3b5"]),
    ];
    for (name, hashes) in hashes {
        let d = base.join(name);
        let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(d.join("observations.json")).unwrap()).unwrap();
        let independent: serde_json::Value = serde_json::from_slice(&std::fs::read(d.join("round2/observations.json")).unwrap()).unwrap();
        for (i, phase) in ["source", "edit"].into_iter().enumerate() {
            let bytes = std::fs::read(d.join(format!("{phase}.xcf"))).unwrap();
            let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hash, hashes[i]);
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert_eq!((file.version, file.precision), (24, 150));
            assert!(file.icc.is_empty());
            let layer = file.layers.iter().find(|l| l.vector.is_some()).unwrap();
            let v = layer.vector.as_ref().unwrap();
            assert!(v.filled);
            assert!(!v.stroked);
            assert!(!v.fill.antialias);
            assert!(!v.source_modified);
            let source = file.paths.iter().find(|p| p.tattoo == Some(v.path_tattoo)).unwrap();
            assert_eq!(source.strokes.len(), 1);
            let s = &source.strokes[0];
            assert!(s.closed);
            assert_eq!(s.points.len(), 6);
            let point = |p: &photocraft_xcf::PathPoint| Point::new(f64::from(f32::from_bits(p.components[0])), f64::from(f32::from_bits(p.components[1])));
            let knots = s
                .points
                .as_chunks::<3>()
                .0
                .iter()
                .map(|p| {
                    assert_eq!([p[0].kind, p[1].kind, p[2].kind], [1, 0, 1]);
                    Knot { in_ctrl: point(&p[0]), anchor: point(&p[1]), out_ctrl: point(&p[2]), smooth: false }
                })
                .collect();
            let path = Path { subpaths: vec![Subpath { knots, closed: true, op: PathOp::Combine }], fill_rule: FillRule::NonZero, inverted: false };
            let before = path.clone();
            let color = v.fill.color.as_ref().unwrap();
            assert_eq!(color.encoding, "R'G'B'A float");
            assert!(color.icc.is_empty());
            let rgba: Vec<f64> = color.samples.as_chunks::<4>().0.iter().map(|b| f64::from(f32::from_le_bytes(*b))).collect();
            let settings = SolidSettings { fill: Some(rgba.try_into().unwrap()), stroke: None, antialias: false };
            let state = &observations["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
            let repeated = &independent["records"].as_array().unwrap().iter().find(|r| r["state"] == format!("{phase}-reopened")).unwrap()["snapshot"];
            let bounds = Rect::from_xywh(layer.offset.0, layer.offset.1, layer.width, layer.height);
            let native = std::fs::read(d.join(state["layer"]["native"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(layer.plane.as_ref().unwrap().bytes, native);
            assert_eq!(native, std::fs::read(d.join("round2").join(repeated["layer"]["native"]["file"].as_str().unwrap())).unwrap());
            let render = |bounds| render_solid(&path, &settings, bounds, PixelFormat::RGBA8, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
            assert_eq!(render(bounds).try_read_interleaved(bounds).unwrap(), native, "{name}/{phase} exact native");
            let canvas = Rect::new(0, 0, 64, 48);
            assert_eq!(
                render(canvas).try_read_interleaved(canvas).unwrap(),
                std::fs::read(d.join(state["projection"]["native"]["file"].as_str().unwrap())).unwrap()
            );
            assert_eq!(path, before);
        }
    }
}
