//! Private exact quadratic Green-integral comparison. No production admission.
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
};
#[derive(Default)]
struct Pen {
    segments: Vec<[[f64; 2]; 3]>,
    current: [f64; 2],
    start: [f64; 2],
    moves: usize,
    cubic: bool,
}
impl Pen {
    fn line(&mut self, to: [f64; 2]) {
        let c = [(self.current[0] + to[0]) * 0.5, (self.current[1] + to[1]) * 0.5];
        self.segments.push([self.current, c, to]);
        self.current = to;
    }
}
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.moves += 1;
        self.current = [x.into(), y.into()];
        self.start = self.current;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.line([x.into(), y.into()]);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let to = [x.into(), y.into()];
        self.segments.push([self.current, [cx.into(), cy.into()], to]);
        self.current = to;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.cubic = true;
    }
    fn close(&mut self) {
        if self.current != self.start {
            self.line(self.start);
        }
    }
}
fn roots(a: f64, b: f64, c: f64, out: &mut Vec<f64>) {
    if a == 0. {
        if b != 0. {
            let t = -c / b;
            if t > 0. && t < 1. {
                out.push(t);
            }
        }
        return;
    }
    let d = b * b - 4. * a * c;
    if d < 0. {
        return;
    }
    let q = -0.5 * (b + if b >= 0. { d.sqrt() } else { -d.sqrt() });
    if q == 0. {
        let t = -b / (2. * a);
        if t > 0. && t < 1. {
            out.push(t);
        }
    } else {
        for t in [q / a, c / q] {
            if t > 0. && t < 1. {
                out.push(t);
            }
        }
    }
}
fn integral(segment: [[f64; 2]; 3], x: f64, y: f64) -> f64 {
    let p = segment.map(|p| [p[0] - x, p[1] - y]);
    let a = [p[0][0] - 2. * p[1][0] + p[2][0], p[0][1] - 2. * p[1][1] + p[2][1]];
    let b = [2. * (p[1][0] - p[0][0]), 2. * (p[1][1] - p[0][1])];
    let c = p[0];
    let mut ts = vec![0., 1.];
    for axis in 0..2 {
        for edge in [0., 1.] {
            roots(a[axis], b[axis], c[axis] - edge, &mut ts);
        }
    }
    ts.sort_by(f64::total_cmp);
    let mut sum = 0.;
    for interval in ts.windows(2) {
        let l = interval[0];
        let r = interval[1];
        let mid = (l + r) * 0.5;
        let yy = (a[1] * mid + b[1]) * mid + c[1];
        if !(0.0..1.).contains(&yy) {
            continue;
        }
        let xx = (a[0] * mid + b[0]) * mid + c[0];
        if xx <= 0. {
            continue;
        }
        if xx >= 1. {
            sum += a[1] * (r * r - l * l) + b[1] * (r - l);
        } else {
            let coeff = [c[0] * b[1], b[0] * b[1] + 2. * c[0] * a[1], a[0] * b[1] + 2. * b[0] * a[1], 2. * a[0] * a[1]];
            let f = |t: f64| (((coeff[3] / 4. * t + coeff[2] / 3.) * t + coeff[1] / 2.) * t + coeff[0]) * t;
            sum += f(r) - f(l);
        }
    }
    sum
}
#[test]
fn signed_clipping_quadratic_and_hole_geometry() {
    let line = |a: [f64; 2], b: [f64; 2]| [a, [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5], b];
    let rect = |lo: f64, hi: f64| {
        let points = [[lo, lo], [hi, lo], [hi, hi], [lo, hi]];
        (0..4).map(|i| line(points[i], points[(i + 1) % 4])).collect::<Vec<_>>()
    };
    let outer = rect(0., 1.);
    let hole = rect(0.25, 0.75);
    let full: f64 = outer.iter().map(|p| integral(*p, 0., 0.)).sum();
    let inside: f64 = hole.iter().rev().map(|p| integral([p[2], p[1], p[0]], 0., 0.)).sum();
    assert_eq!((full + inside).abs(), 0.75);
    assert_eq!(outer.iter().map(|p| integral(*p, 1., 0.)).sum::<f64>(), 0.);
    let q = [[0., 0.], [0.5, 1.], [1., 0.]];
    let closed = integral(q, 0., 0.) + integral(line([1., 0.], [0., 0.]), 0., 0.);
    assert!((closed.abs() - 1. / 3.).abs() < 1e-14);
}
#[test]
#[ignore = "private independent GIMP quadratic AA controls; no product qualification"]
fn original_quadratic_diagonal_controls() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/diagonal-aa-evidence");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let face = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let size = Size::new(32.);
    let loc = LocationRef::default();
    let baseline = f64::from(face.metrics(size, loc).ascent.ceil());
    let mut worst = 0;
    for i in 0..8 {
        let text = observations["rows"][i]["text"].as_str().unwrap();
        let mut p = Pen::default();
        let glyph = face.charmap().map(text.chars().next().unwrap()).unwrap();
        face.outline_glyphs().get(glyph).unwrap().draw(DrawSettings::unhinted(size, loc), &mut p).unwrap();
        assert_eq!(p.moves, 1);
        assert!(!p.cubic);
        for q in &mut p.segments {
            for point in q {
                point[0] += 8.;
                point[1] = 4. + baseline - point[1];
            }
        }
        let expected = std::fs::read(base.join(format!("control-{i}.projection.bin"))).unwrap();
        assert_eq!(expected, std::fs::read(base.join(format!("round2/control-{i}.projection.bin"))).unwrap());
        let mut max = 0;
        let mut over = 0;
        let mut alpha_max = 0;
        let mut premult_max = 0_f64;
        for y in 0..64 {
            for x in 0..128 {
                let covered = p.segments.iter().map(|q| integral(*q, x as f64, y as f64)).sum::<f64>().abs();
                assert!(covered <= 1. + 1e-10, "unsupported overlapping contour");
                let alpha = (covered.clamp(0., 1.) * 255.).round() as u8;
                let actual = if alpha == 0 { [0, 0, 0, 0] } else { [255, 255, 255, alpha] };
                let px = &expected[(y * 128 + x) * 4..(y * 128 + x) * 4 + 4];
                let d = actual.iter().zip(px).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                alpha_max = alpha_max.max(alpha.abs_diff(px[3]));
                for (component, expected) in actual.iter().zip(px).take(3) {
                    premult_max = premult_max.max((f64::from(*component) * f64::from(alpha) / 255. - f64::from(*expected) * f64::from(px[3]) / 255.).abs());
                }
                max = max.max(d);
                over += usize::from(d > 2);
            }
        }
        println!("{text}: exact quadratic clipped integral rawmax{max}/255 alpha{alpha_max}/255 premult{premult_max}/255 beyond2={over}");
        worst = worst.max(max);
    }
    assert!(worst <= 2, "quadratic candidate exceeds unchanged bound {worst}");
}

// Public format: https://freetype.org/freetype2/docs/glyphs/glyphs-6.html
#[test]
#[ignore = "private documented fixed-grid hypothesis, no product qualification"]
fn documented_26_6_outline_grid_hypothesis() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/diagonal-aa-evidence");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let face = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let size = Size::new(32.);
    let loc = LocationRef::default();
    let baseline = f64::from(face.metrics(size, loc).ascent.ceil());
    let mut worst = 0_f64;
    for i in 0..8 {
        let text = observations["rows"][i]["text"].as_str().unwrap();
        let mut p = Pen::default();
        let glyph = face.charmap().map(text.chars().next().unwrap()).unwrap();
        face.outline_glyphs().get(glyph).unwrap().draw(DrawSettings::unhinted(size, loc), &mut p).unwrap();
        assert_eq!(p.moves, 1);
        assert!(!p.cubic);
        for q in &mut p.segments {
            for point in q {
                // Public FreeType outline format is 26.6. This tests only
                // nearest conversion as a hypothesis, not backend behavior.
                point[0] = (point[0] * 64.).round() / 64.;
                point[1] = (point[1] * 64.).round() / 64.;
                point[0] += 8.;
                point[1] = 4. + baseline - point[1];
            }
        }
        let expected = std::fs::read(base.join(format!("control-{i}.projection.bin"))).unwrap();
        assert_eq!(expected, std::fs::read(base.join(format!("round2/control-{i}.projection.bin"))).unwrap());
        let mut max = 0;
        let mut over = 0;
        let mut alpha_max = 0;
        let mut premult_max = 0_f64;
        for y in 0..64 {
            for x in 0..128 {
                let covered = p.segments.iter().map(|q| integral(*q, x as f64, y as f64)).sum::<f64>().abs();
                assert!(covered <= 1. + 1e-10, "unsupported overlapping contour");
                let alpha = (covered.clamp(0., 1.) * 255.).round() as u8;
                let actual = if alpha == 0 { [0, 0, 0, 0] } else { [255, 255, 255, alpha] };
                let px = &expected[(y * 128 + x) * 4..(y * 128 + x) * 4 + 4];
                let d = actual.iter().zip(px).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                alpha_max = alpha_max.max(alpha.abs_diff(px[3]));
                for (component, expected) in actual.iter().zip(px).take(3) {
                    premult_max = premult_max.max((f64::from(*component) * f64::from(alpha) / 255. - f64::from(*expected) * f64::from(px[3]) / 255.).abs());
                }
                max = max.max(d);
                over += usize::from(d > 2);
            }
        }
        println!("{text}: 26.6 nearest outline + exact quadratic integral rawmax{max}/255 alpha{alpha_max}/255 premult{premult_max}/255 beyond2={over}");
        worst = worst.max(f64::from(alpha_max)).max(premult_max);
    }
    assert!(worst <= 2., "quadratic candidate exceeds unchanged bound {worst}");
}
#[test]
#[ignore = "private public GIMP text-to-path geometry observation; no raster admission"]
fn public_text_path_geometry_vs_exact_font_outline() {
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/text-path-evidence");
    let bytes = std::fs::read(base.join("paths.json")).unwrap();
    assert_eq!(bytes, std::fs::read(base.join("round2/paths.json")).unwrap());
    let observations: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let face = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let size = Size::new(32.);
    let loc = LocationRef::default();
    let baseline = f64::from(face.metrics(size, loc).ascent.ceil());
    let mut worst = 0_f64;
    for row in observations["rows"].as_array().unwrap() {
        let i = row["case"].as_u64().unwrap();
        let source = base.parent().unwrap().join(format!("diagonal-aa-evidence/control-{i}.xcf"));
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(source).unwrap())), row["source_sha256"].as_str().unwrap());
        let text = row["text"].as_str().unwrap();
        let mut p = Pen::default();
        let glyph = face.charmap().map(text.chars().next().unwrap()).unwrap();
        face.outline_glyphs().get(glyph).unwrap().draw(DrawSettings::unhinted(size, loc), &mut p).unwrap();
        assert_eq!(p.moves, 1);
        assert!(!p.cubic);
        let strokes = row["strokes"].as_array().unwrap();
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0]["closed"], true);
        let points = strokes[0]["points"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>();
        assert_eq!(points.len(), p.segments.len() * 6, "segment count {text}");
        let n = p.segments.len();
        let mut max = 0_f64;
        let mut grid = [[0_f64; 3]; 2];
        for (index, q) in p.segments.iter().enumerate() {
            let q = q.map(|v| [v[0] + 8., 4. + baseline - v[1]]);
            let node = &points[index * 6..index * 6 + 6];
            let next = &points[((index + 1) % n) * 6..((index + 1) % n) * 6 + 6];
            for axis in 0..2 {
                max = max.max((node[axis + 2] - q[0][axis]).abs()).max((next[axis + 2] - q[2][axis]).abs());
            }
            let line = q[1] == [(q[0][0] + q[2][0]) * 0.5, (q[0][1] + q[2][1]) * 0.5];
            if !line {
                for axis in 0..2 {
                    for (actual, expected) in [(node[axis + 4], q[0][axis] / 3. + q[1][axis] * 2. / 3.), (next[axis], q[2][axis] / 3. + q[1][axis] * 2. / 3.)] {
                        max = max.max((actual - expected).abs());
                        assert_eq!((actual * 256.).fract(), 0., "public outline coordinate not on observed grid");
                        for (mode, quantized) in
                            [(0, (expected * 256.).floor() / 256.), (1, (expected * 256.).ceil() / 256.), (2, (expected * 256.).round() / 256.)]
                        {
                            grid[axis][mode] = grid[axis][mode].max((actual - quantized).abs());
                        }
                    }
                }
            }
        }
        println!("{text}: public cubic path vs exact font max coordinate delta{max}, grid [floor/ceil/nearest] peraxis {grid:?}");
        worst = worst.max(max);
    }
    assert!(worst < 1e-5, "public path differs from source font outline {worst}");
}
