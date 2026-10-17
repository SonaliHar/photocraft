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
#[ignore = "private fresh curve/multicontour AA integral diagnostic; no admission"]
fn fresh_curve_kern_quadratic_integral() {
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/curve-kern-evidence");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let bytes = photocraft_text::fonts::INTER_REGULAR.as_slice();
    assert_eq!(format!("{:x}", Sha256::digest(bytes)), "40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82");
    let face = skrifa::FontRef::new(bytes).unwrap();
    let hfont = harfrust::FontRef::new(bytes).unwrap();
    let sd = harfrust::ShaperData::new(&hfont);
    let shaper = sd.shaper(&hfont).build();
    let mut report = Vec::new();
    let mut worst = 0u8;
    for row in data["rows"].as_array().unwrap().iter().filter(|r| r["antialias"] == true) {
        let text = row["text"].as_str().unwrap();
        let case = row["case"].as_str().unwrap();
        let size_px = row["size_px"].as_f64().unwrap() as f32;
        let size = Size::new(size_px);
        let location = LocationRef::default();
        let baseline = f64::from(face.metrics(size, location).ascent.ceil());
        let mut buf = harfrust::UnicodeBuffer::new();
        buf.push_str(text);
        buf.set_direction(harfrust::Direction::LeftToRight);
        buf.set_script(harfrust::script::LATIN);
        let shaped = shaper.shape(buf, harfrust::ShapeOptions::default());
        let mut pen = Pen::default();
        let mut x = 0.;
        let mut glyphs = Vec::new();
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            let start = pen.segments.len();
            face.outline_glyphs().get(skrifa::GlyphId::new(info.glyph_id)).unwrap().draw(DrawSettings::unhinted(size, location), &mut pen).unwrap();
            assert!(!pen.cubic);
            assert_eq!(pos.y_advance, 0);
            let offset = f64::from(pos.x_offset) * f64::from(size_px) / f64::from(shaper.units_per_em());
            let yo = f64::from(pos.y_offset) * f64::from(size_px) / f64::from(shaper.units_per_em());
            for q in &mut pen.segments[start..] {
                for p in q {
                    p[0] += 8. + x + offset;
                    p[1] = 4. + baseline - p[1] - yo;
                }
            }
            glyphs.push(serde_json::json!({"id":info.glyph_id,"cluster":info.cluster,"x":x,"x_offset":offset,"y_offset":yo,"x_advance_raw":pos.x_advance}));
            x += (f64::from(pos.x_advance) * f64::from(size_px) / f64::from(shaper.units_per_em())).round();
        }
        let reference = std::fs::read(base.join(format!("{case}.projection.bin"))).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&reference)), row["state"]["projection"]["sha256"].as_str().unwrap());
        for quantized in [false, true] {
            let mut spans = pen.segments.clone();
            if quantized {
                for q in &mut spans {
                    for p in q {
                        p[0] = (p[0] * 64.).round() / 64.;
                        p[1] = (p[1] * 64.).round() / 64.;
                    }
                }
            }
            let mut actual = vec![0u8; 256 * 128];
            let bounds = spans.iter().flatten().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |r, p| {
                [r[0].min(p[0]), r[1].min(p[1]), r[2].max(p[0]), r[3].max(p[1])]
            });
            for yy in (bounds[1].floor() as i32).max(0)..(bounds[3].ceil() as i32).min(128) {
                for xx in (bounds[0].floor() as i32).max(0)..(bounds[2].ceil() as i32).min(256) {
                    let area = spans.iter().map(|q| integral(*q, f64::from(xx), f64::from(yy))).sum::<f64>().abs();
                    assert!(area <= 1. + 1e-9, "overlapping contour unresolved {case} {area}");
                    actual[yy as usize * 256 + xx as usize] = (area.clamp(0., 1.) * 255.).round() as u8;
                }
            }
            let delta = actual.iter().zip(reference.as_chunks::<4>().0.iter()).map(|(a, b)| a.abs_diff(b[3])).max().unwrap();
            let over = actual.iter().zip(reference.as_chunks::<4>().0.iter()).filter(|(a, b)| a.abs_diff(b[3]) > 2).count();
            println!("{case} nearest26.6={quantized}: alpha{delta}/255 beyond2={over} contours{} glyphs{}", pen.moves, glyphs.len());
            report.push(
                serde_json::json!({"case":case,"quantized_26_6_nearest":quantized,"glyphs":glyphs,"contours":pen.moves,"alpha_max":delta,"beyond2":over}),
            );
            worst = worst.max(delta);
        }
    }
    std::fs::write(base.join("quadratic-integral-comparison.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(worst <= 2, "quadratic candidate exceeds unchanged bound {worst}");
}
