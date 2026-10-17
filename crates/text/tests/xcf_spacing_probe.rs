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
#[ignore = "private independent GIMP font-size controls; no product qualification"]
fn original_letter_spacing_orthogonal_controls() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/spacing-evidence");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let face = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let sf = harfrust::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let data = harfrust::ShaperData::new(&sf);
    let shaper = data.shaper(&sf).build();
    let mut worst = 0;
    for row in observations["rows"].as_array().unwrap() {
        let name = row["case"].as_str().unwrap();
        let text = row["state"]["text"].as_str().unwrap();
        let pixels = row["state"]["font_size"][0].as_f64().unwrap();
        let size = Size::new(pixels as f32);
        let loc = LocationRef::default();
        let baseline = f64::from(face.metrics(size, loc).ascent.ceil());
        let mut pens = Vec::new();
        let mut advance = 0.;
        let mut buffer = harfrust::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.set_direction(harfrust::Direction::LeftToRight);
        buffer.set_script(harfrust::script::LATIN);
        let shaped = shaper.shape(buffer, harfrust::ShapeOptions::default());
        assert_eq!(shaped.glyph_infos().len(), text.len());
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            let mut p = Pen::default();
            if let Some(outline) = face.outline_glyphs().get(skrifa::GlyphId::new(info.glyph_id)) {
                outline.draw(DrawSettings::unhinted(size, loc), &mut p).unwrap();
            }
            assert!(!p.cubic);
            for q in &mut p.segments {
                assert!(q[0][0] == q[2][0] || q[0][1] == q[2][1]);
                for point in q {
                    point[0] += 8. + advance;
                    point[1] = 4. + baseline - point[1];
                }
            }
            pens.push(p);
            advance += (f64::from(pos.x_advance) * pixels / f64::from(shaper.units_per_em())).round() + row["state"]["letter_spacing"].as_f64().unwrap();
        }
        advance -= row["state"]["letter_spacing"].as_f64().unwrap();
        assert_eq!(advance, row["state"]["size"][0].as_f64().unwrap(), "independent layout width {name}");
        let expected = std::fs::read(base.join(format!("{name}.projection.bin"))).unwrap();
        assert_eq!(expected, std::fs::read(base.join(format!("round2/{name}.projection.bin"))).unwrap());
        let mut max = 0;
        let mut alpha_max = 0;
        let aa = row["state"]["antialias"].as_bool().unwrap();
        for y in 0..64 {
            for x in 0..128 {
                let alpha = if aa {
                    let covered = pens.iter().map(|p| p.segments.iter().map(|q| integral(*q, x as f64, y as f64)).sum::<f64>().abs()).sum::<f64>();
                    assert!(covered <= 1. + 1e-10);
                    (covered.clamp(0., 1.) * 255.).round() as u8
                } else {
                    let cx = x as f64 + 0.5;
                    let cy = y as f64 + 0.5;
                    let any = pens.iter().any(|p| {
                        let mut winding = 0;
                        for q in &p.segments {
                            let a = q[0];
                            let b = q[2];
                            if a[1] <= cy && b[1] > cy && a[0] > cx {
                                winding += 1;
                            }
                            if b[1] <= cy && a[1] > cy && a[0] > cx {
                                winding -= 1;
                            }
                        }
                        winding != 0
                    });
                    if any { 255 } else { 0 }
                };
                let actual = if alpha == 0 { [0, 0, 0, 0] } else { [255, 255, 255, alpha] };
                let px = &expected[(y * 128 + x) * 4..(y * 128 + x) * 4 + 4];
                let d = actual.iter().zip(px).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                max = max.max(d);
                alpha_max = alpha_max.max(alpha.abs_diff(px[3]));
            }
        }
        println!("{name}: size{pixels} aa{aa} rawmax{max}/255 alpha{alpha_max}/255");
        worst = worst.max(max);
    }
    assert!(worst <= 2, "scaled candidate fails unchanged bound {worst}");
}
