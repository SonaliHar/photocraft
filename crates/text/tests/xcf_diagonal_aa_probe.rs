//! Private clean-room straight-outline coverage comparison; not an admission path.
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
};
#[derive(Default)]
struct Pen {
    points: Vec<[f64; 2]>,
    closed: bool,
    complex: bool,
    moves: usize,
    quads: usize,
    cubics: usize,
}
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.moves += 1;
        if !self.points.is_empty() {
            self.complex = true;
        }
        self.points.push([x.into(), y.into()]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.points.push([x.into(), y.into()]);
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.quads += 1;
        self.complex = true;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.cubics += 1;
        self.complex = true;
    }
    fn close(&mut self) {
        self.closed = true;
    }
}
fn area(points: &[[f64; 2]], x: f64, y: f64) -> f64 {
    let mut p = points.to_vec();
    for (axis, edge, less) in [(0, x, false), (0, x + 1., true), (1, y, false), (1, y + 1., true)] {
        let mut out = Vec::new();
        for i in 0..p.len() {
            let a = p[i];
            let b = p[(i + 1) % p.len()];
            let ai = if less { a[axis] <= edge } else { a[axis] >= edge };
            let bi = if less { b[axis] <= edge } else { b[axis] >= edge };
            if ai {
                out.push(a);
            }
            if ai != bi {
                let t = (edge - a[axis]) / (b[axis] - a[axis]);
                let mut q = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                q[axis] = edge;
                out.push(q);
            }
        }
        p = out;
    }
    let mut sum = 0.;
    for i in 0..p.len() {
        let a = p[i];
        let b = p[(i + 1) % p.len()];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    (sum.abs() * 0.5).clamp(0., 1.)
}
#[test]
#[ignore = "private independent straight-outline GIMP AA holdout; no product qualification"]
fn independent_diagonal_aa_single_glyphs() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/diagonal-aa-evidence");
    let a: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let b: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("round2/observations.json")).unwrap()).unwrap();
    let face = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let size = Size::new(32.);
    let loc = LocationRef::default();
    let baseline = f64::from(face.metrics(size, loc).ascent.ceil());
    let mut worst = 0;
    let mut tested = 0;
    for i in 0..8 {
        let row = &a["rows"][i];
        assert_eq!(row["source_sha256"], b["rows"][i]["source_sha256"]);
        assert_eq!(row["state"]["native"]["sha256"], b["rows"][i]["state"]["native"]["sha256"]);
        let text = row["text"].as_str().unwrap();
        let glyph = face.charmap().map(text.chars().next().unwrap()).unwrap();
        let mut pen = Pen::default();
        face.outline_glyphs().get(glyph).unwrap().draw(DrawSettings::unhinted(size, loc), &mut pen).unwrap();
        if pen.complex || !pen.closed {
            println!("{text}: moves{} quads{} cubics{}, remains unsupported", pen.moves, pen.quads, pen.cubics);
            continue;
        }
        tested += 1;
        for p in &mut pen.points {
            p[0] += 8.;
            p[1] = 4. + baseline - p[1];
        }
        let expected = std::fs::read(base.join(format!("control-{i}.projection.bin"))).unwrap();
        assert_eq!(expected, std::fs::read(base.join(format!("round2/control-{i}.projection.bin"))).unwrap());
        let mut max = 0;
        let mut over = 0;
        for y in 0..64 {
            for x in 0..128 {
                let alpha = (area(&pen.points, x as f64, y as f64) * 255.).round() as u8;
                let px = &expected[(y * 128 + x) * 4..(y * 128 + x) * 4 + 4];
                let actual = if alpha == 0 { [0, 0, 0, 0] } else { [255, 255, 255, alpha] };
                let diff = actual.iter().zip(px).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                max = max.max(diff);
                over += usize::from(diff > 2);
            }
        }
        println!("{text}: analytic straight polygon max{max}/255, beyond2={over}");
        worst = worst.max(max);
    }
    assert_eq!(tested, 0, "new eligible geometry needs separate original qualification");
    assert!(worst <= 2, "independent diagonal controls exceed unchanged display bound: {worst}");
}
