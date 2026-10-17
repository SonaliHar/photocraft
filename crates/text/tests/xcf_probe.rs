use photocraft_color::{Color, PixelFormat};
use photocraft_doc::{TextLayer, text::AntiAlias};
use photocraft_geom::{Affine, Rect};
use photocraft_text::TextEngine;
use skrifa::{MetadataProvider, instance::Size};
#[test]
#[ignore = "requires own public GIMP exact bundled Inter native source/edit observations"]
fn exact_face_unhinted_stem_layout_matches_independent_stock() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/simple");
    let font = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let metrics = font.metrics(Size::new(32.), skrifa::instance::LocationRef::default());
    println!("exact face metrics {metrics:?}");
    for (phase, text) in [("source", "I"), ("edit", "II")] {
        let layer = TextLayer {
            text: text.into(),
            font_family: "Inter".into(),
            size_pt: 32.,
            color: Color::WHITE,
            transform: Affine::translate(8., 4. + f64::from(metrics.ascent.ceil())),
            antialias: AntiAlias::None,
            ..Default::default()
        };
        let mut layout = TextEngine::new().layout(&layer, 72.);
        let mut previous = 0.;
        let mut current = 0.;
        for g in &mut layout.glyphs {
            let original = g.x;
            current += (original - previous).round();
            g.x = current;
            previous = original;
        }
        let rendered = photocraft_text::render::rasterize(&layout, &layer.transform, PixelFormat::RGBA8, layer.antialias);
        assert!(layout.faces.iter().all(|face| face.font.data.as_ref() == photocraft_text::fonts::INTER_REGULAR.as_slice()));
        println!("{phase}: lines={:?} glyphs={:?} rect={:?}", layout.lines, layout.glyphs, rendered.rect);
        let actual = rendered.surface.try_read_interleaved(Rect::new(0, 0, 128, 64)).unwrap();
        let expected = std::fs::read(base.join(format!("{phase}-reopened.projection.bin"))).unwrap();
        let differences: Vec<_> = actual
            .as_chunks::<4>()
            .0
            .iter()
            .zip(expected.as_chunks::<4>().0.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i % 128, i / 128, a.to_vec(), b.to_vec()))
            .collect();
        println!("{phase} differences{} first{:?}", differences.len(), differences.first());
        assert!(differences.is_empty());
    }
}

#[test]
#[ignore = "private public GIMP rectilinear glyph/spacing diagnostic; no admission"]
fn rectilinear_glyph_and_spacing_controls() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/rectilinear-controls");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let font = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let ascent = font.metrics(Size::new(32.), skrifa::instance::LocationRef::default()).ascent.ceil();
    for (index, row) in observations["rows"].as_array().unwrap().iter().enumerate() {
        let text = row["text"].as_str().unwrap();
        let layer = TextLayer {
            text: text.into(),
            font_family: "Inter".into(),
            size_pt: 32.,
            color: Color::WHITE,
            transform: Affine::translate(8., 4. + f64::from(ascent)),
            antialias: AntiAlias::None,
            ..Default::default()
        };
        let mut layout = TextEngine::new().layout(&layer, 72.);
        let mut previous = 0.;
        let mut current = 0.;
        for g in &mut layout.glyphs {
            let original = g.x;
            current += (original - previous).round();
            g.x = current;
            previous = original;
        }
        assert!(layout.faces.iter().all(|face| face.font.data.as_ref() == photocraft_text::fonts::INTER_REGULAR.as_slice()));
        let rendered = photocraft_text::render::rasterize(&layout, &layer.transform, PixelFormat::RGBA8, layer.antialias);
        let actual = rendered.surface.try_read_interleaved(Rect::new(0, 0, 128, 64)).unwrap();
        let expected = std::fs::read(base.join(format!("control-{index}.projection.bin"))).unwrap();
        let repeat = std::fs::read(base.join(format!("round2/control-{index}.projection.bin"))).unwrap();
        assert_eq!(expected, repeat);
        let differences: Vec<_> = actual
            .as_chunks::<4>()
            .0
            .iter()
            .zip(expected.as_chunks::<4>().0.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i % 128, i / 128, a.to_vec(), b.to_vec()))
            .collect();
        println!("{text:?} differingpixels{} first{:?} glyphs{:?}", differences.len(), differences.first(), layout.glyphs);
    }
}

#[derive(Default)]
struct PolygonPen {
    points: Vec<[f64; 2]>,
    curved: bool,
}
impl skrifa::outline::OutlinePen for PolygonPen {
    fn move_to(&mut self, x: f32, y: f32) {
        assert!(self.points.is_empty());
        self.points.push([x.into(), y.into()]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.points.push([x.into(), y.into()]);
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.curved = true;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.curved = true;
    }
    fn close(&mut self) {}
}
fn pixel_area(points: &[[f64; 2]], x: f64, y: f64) -> f64 {
    let mut p = points.to_vec();
    for (axis, value, lower) in [(0, x, true), (0, x + 1., false), (1, y, true), (1, y + 1., false)] {
        let old = std::mem::take(&mut p);
        if old.is_empty() {
            return 0.;
        }
        for (i, b) in old.iter().enumerate() {
            let a = old[(i + old.len() - 1) % old.len()];
            let ai = if lower { a[axis] >= value } else { a[axis] <= value };
            let bi = if lower { b[axis] >= value } else { b[axis] <= value };
            if ai != bi {
                let t = (value - a[axis]) / (b[axis] - a[axis]);
                p.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
            }
            if bi {
                p.push(*b);
            }
        }
    }
    p.iter()
        .enumerate()
        .map(|(i, a)| {
            let b = p[(i + 1) % p.len()];
            a[0] * b[1] - a[1] * b[0]
        })
        .sum::<f64>()
        .abs()
        * 0.5
}
#[test]
#[ignore = "private exact area vs stock public rectilinear controls diagnostic"]
fn rectilinear_exact_area_is_not_stock_noaa_rule() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/rectilinear-controls");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let font = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let size = Size::new(32.);
    let loc = skrifa::instance::LocationRef::default();
    let mut total = 0;
    for (i, row) in data["rows"].as_array().unwrap().iter().enumerate() {
        let text = row["text"].as_str().unwrap();
        let mut polys = Vec::new();
        let mut x = 8.;
        for c in text.chars() {
            let glyph = font.charmap().map(c).unwrap();
            if c != ' ' {
                let mut pen = PolygonPen::default();
                font.outline_glyphs().get(glyph).unwrap().draw(skrifa::outline::DrawSettings::unhinted(size, loc), &mut pen).unwrap();
                assert!(!pen.curved);
                polys.push(pen.points.into_iter().map(|p| [x + p[0], 35. - p[1]]).collect::<Vec<_>>());
            }
            x += f64::from(font.glyph_metrics(size, loc).advance_width(glyph).unwrap().round());
        }
        let exp = std::fs::read(base.join(format!("control-{i}.projection.bin"))).unwrap();
        let mut diff = Vec::new();
        for y in 0..64 {
            for x in 0..128 {
                let area = polys.iter().map(|p| pixel_area(p, x as f64, y as f64)).sum::<f64>();
                let a = if area >= 0.5 { 255 } else { 0 };
                let e = exp[(y * 128 + x) * 4 + 3];
                if a != e {
                    diff.push((x, y, a, e, area));
                }
            }
        }
        println!("{text:?} exactarea differences{} first{:?}", diff.len(), diff.first());
        total += diff.len();
    }
    assert!(total > 0, "area threshold hypothesis must stay disproven");
}

fn point_inside(p: &[[f64; 2]], x: f64, y: f64) -> bool {
    let mut winding = 0;
    for (i, a) in p.iter().enumerate() {
        let b = p[(i + 1) % p.len()];
        if a[1] <= y && b[1] > y && a[0] > x {
            winding += 1;
        }
        if b[1] <= y && a[1] > y && a[0] > x {
            winding -= 1;
        }
    }
    winding != 0
}
#[test]
#[ignore = "private center winding vs stock public rectilinear controls diagnostic"]
fn rectilinear_center_controls() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/rectilinear-controls");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let font = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let size = Size::new(32.);
    let loc = skrifa::instance::LocationRef::default();
    let mut total = 0;
    for (i, row) in data["rows"].as_array().unwrap().iter().enumerate() {
        let text = row["text"].as_str().unwrap();
        let mut polys = Vec::new();
        let layer =
            TextLayer { text: text.into(), font_family: "Inter".into(), size_pt: 32., color: Color::WHITE, antialias: AntiAlias::None, ..Default::default() };
        let layout = TextEngine::new().layout(&layer, 72.);
        let mut previous = 0.;
        let mut current = 0.;
        for g in layout.glyphs {
            current += (g.x - previous).round();
            previous = g.x;
            let glyph = skrifa::GlyphId::new(g.id);
            let Some(outline) = font.outline_glyphs().get(glyph) else { continue };
            let mut pen = PolygonPen::default();
            outline.draw(skrifa::outline::DrawSettings::unhinted(size, loc), &mut pen).unwrap();
            assert!(!pen.curved);
            if !pen.points.is_empty() {
                polys.push(pen.points.into_iter().map(|p| [8. + f64::from(current) + p[0], 35. - p[1]]).collect::<Vec<_>>());
            }
        }
        let exp = std::fs::read(base.join(format!("control-{i}.projection.bin"))).unwrap();
        let mut diff = Vec::new();
        for y in 0..64 {
            for x in 0..128 {
                let area = polys.iter().any(|p| point_inside(p, x as f64 + 0.5, y as f64 + 0.5)) as u8 as f64;
                let a = if area >= 0.5 { 255 } else { 0 };
                let e = exp[(y * 128 + x) * 4 + 3];
                if a != e {
                    diff.push((x, y, a, e, area));
                }
            }
        }
        println!("{text:?} center differences{} first{:?}", diff.len(), diff.first());
        total += diff.len();
    }
    assert_eq!(total, 0);
}

#[test]
#[ignore = "private public AA controls area-integration diagnostic, no admission"]
fn rectilinear_analytic_aa_controls() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/aa-controls");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let font = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let size = Size::new(32.);
    let loc = skrifa::instance::LocationRef::default();
    for (index, row) in data["rows"].as_array().unwrap().iter().enumerate() {
        let text = row["state"]["text"].as_str().unwrap();
        let layer =
            TextLayer { text: text.into(), font_family: "Inter".into(), size_pt: 32., color: Color::WHITE, antialias: AntiAlias::None, ..Default::default() };
        let layout = TextEngine::new().layout(&layer, 72.);
        assert!(layout.faces.iter().all(|f| f.font.data.as_ref() == photocraft_text::fonts::INTER_REGULAR.as_slice()));
        let mut polys = Vec::new();
        let mut previous = 0.;
        let mut current = 0.;
        for g in layout.glyphs {
            current += (g.x - previous).round();
            previous = g.x;
            let Some(outline) = font.outline_glyphs().get(skrifa::GlyphId::new(g.id)) else { continue };
            let mut pen = PolygonPen::default();
            outline.draw(skrifa::outline::DrawSettings::unhinted(size, loc), &mut pen).unwrap();
            assert!(!pen.curved);
            if !pen.points.is_empty() {
                polys.push(pen.points.into_iter().map(|p| [8. + f64::from(current) + p[0], 35. - p[1]]).collect::<Vec<_>>());
            }
        }
        let name = if index < 11 { format!("control-{index}") } else { format!("{}-{}", row["case"].as_str().unwrap(), row["phase"].as_str().unwrap()) };
        let expected = std::fs::read(base.join(format!("{name}.projection.bin"))).unwrap();
        assert_eq!(expected, std::fs::read(base.join(format!("round2/{name}.projection.bin"))).unwrap());
        let mut differences = Vec::new();
        for y in 0..64 {
            for x in 0..128 {
                let area = polys.iter().map(|p| pixel_area(p, x as f64, y as f64)).sum::<f64>().clamp(0., 1.);
                let alpha = (area * 255.).round() as u8;
                let e = expected[(y * 128 + x) * 4 + 3];
                if alpha != e {
                    differences.push((x, y, alpha, e, alpha.abs_diff(e)));
                }
            }
        }
        println!("AA {text:?} pixels{} max{} first{:?}", differences.len(), differences.iter().map(|v| v.4).max().unwrap_or(0), differences.first());
    }
}

#[derive(Default)]
struct CurvedPen {
    contours: Vec<Vec<[f64; 2]>>,
    tol: f64,
    fixed_grid: bool,
    fixed_subdivision: bool,
}
impl CurvedPen {
    fn coordinate(&self, x: f32, y: f32) -> [f64; 2] {
        let p = [f64::from(x), f64::from(y)];
        if self.fixed_grid { p.map(|v| (v * 64.).round() / 64.) } else { p }
    }
    fn append(&mut self, p: [f64; 2]) {
        self.contours.last_mut().unwrap().push(p);
    }
    fn quad(&mut self, a: [f64; 2], b: [f64; 2], c: [f64; 2], depth: u32) {
        let deviation = ((a[0] - 2. * b[0] + c[0]).powi(2) + (a[1] - 2. * b[1] + c[1]).powi(2)).sqrt() * 0.25;
        if deviation <= self.tol || depth >= 16 {
            self.append(c);
            return;
        }
        let midpoint = |p: [f64; 2], q: [f64; 2]| {
            let v = [(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5];
            if self.fixed_subdivision { v.map(|c| (c * 64.).floor() / 64.) } else { v }
        };
        let ab = midpoint(a, b);
        let bc = midpoint(b, c);
        let m = midpoint(ab, bc);
        self.quad(a, ab, m, depth + 1);
        self.quad(m, bc, c, depth + 1);
    }
}
impl skrifa::outline::OutlinePen for CurvedPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.contours.push(vec![self.coordinate(x, y)]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.append(self.coordinate(x, y));
    }
    fn quad_to(&mut self, x: f32, y: f32, ex: f32, ey: f32) {
        let a = *self.contours.last().unwrap().last().unwrap();
        self.quad(a, self.coordinate(x, y), self.coordinate(ex, ey), 0);
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        panic!("known quadratic face unexpectedly cubic");
    }
    fn close(&mut self) {}
}
fn general_winding(p: &[[f64; 2]], x: f64, y: f64) -> i32 {
    let mut winding = 0;
    for (i, a) in p.iter().enumerate() {
        let b = p[(i + 1) % p.len()];
        if a[1] <= y && b[1] > y || b[1] <= y && a[1] > y {
            let at = a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if at > x {
                winding += if b[1] > a[1] { 1 } else { -1 };
            }
        }
    }
    winding
}
fn contour_distance(p: &[[f64; 2]], x: f64, y: f64) -> f64 {
    p.iter()
        .enumerate()
        .map(|(i, a)| {
            let b = p[(i + 1) % p.len()];
            let delta = [b[0] - a[0], b[1] - a[1]];
            let squared = delta[0] * delta[0] + delta[1] * delta[1];
            let t = if squared == 0. { 0. } else { (((x - a[0]) * delta[0] + (y - a[1]) * delta[1]) / squared).clamp(0., 1.) };
            ((x - a[0] - t * delta[0]).powi(2) + (y - a[1] - t * delta[1]).powi(2)).sqrt()
        })
        .fold(f64::INFINITY, f64::min)
}
#[test]
#[ignore = "private public36controls exactfont quadratic center-sampling diagnostic, no admission"]
fn general_quadratic_glyph_controls() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text/general-controls");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let font = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let size = Size::new(32.);
    let loc = skrifa::instance::LocationRef::default();
    // Public FT_Vector uses 26.6 pixel coordinates for scaled outlines. This
    // is an isolated hypothesis about the observed backend, not an admission.
    for (tolerance, fixed_grid, fixed_subdivision) in
        [(0.1, false, false), (0.03, false, false), (1. / 4096., false, false), (1. / 4096., true, false), (1. / 64., true, true)]
    {
        let mut total = 0;
        let mut boundary_total = 0;
        for (index, row) in data["rows"].as_array().unwrap().iter().enumerate() {
            let text = row["text"].as_str().unwrap();
            let layer = TextLayer {
                text: text.into(),
                font_family: "Inter".into(),
                size_pt: 32.,
                color: Color::WHITE,
                antialias: AntiAlias::None,
                ..Default::default()
            };
            let layout = TextEngine::new().layout(&layer, 72.);
            assert!(layout.faces.iter().all(|f| f.font.data.as_ref() == photocraft_text::fonts::INTER_REGULAR.as_slice()));
            let mut polys = Vec::new();
            let mut previous = 0.;
            let mut current = 0.;
            for g in layout.glyphs {
                current += (g.x - previous).round();
                previous = g.x;
                let Some(outline) = font.outline_glyphs().get(skrifa::GlyphId::new(g.id)) else { continue };
                let mut pen = CurvedPen { tol: tolerance, fixed_grid, fixed_subdivision, ..Default::default() };
                outline.draw(skrifa::outline::DrawSettings::unhinted(size, loc), &mut pen).unwrap();
                for contour in pen.contours {
                    polys.push(contour.into_iter().map(|p| [8. + f64::from(current) + p[0], 35. - p[1]]).collect::<Vec<_>>());
                }
            }
            let expected = std::fs::read(base.join(format!("control-{index}.projection.bin"))).unwrap();
            let repeat = std::fs::read(base.join(format!("round2/control-{index}.projection.bin"))).unwrap();
            assert_eq!(expected, repeat);
            let mut differences = Vec::new();
            let mut boundary_differences = 0;
            for y in 0..64 {
                for x in 0..128 {
                    let inside = polys.iter().map(|p| general_winding(p, x as f64 + 0.5, y as f64 + 0.5)).sum::<i32>() != 0;
                    let e = expected[(y * 128 + x) * 4 + 3];
                    // OpenType scan conversion Rule 2 makes exact on-contour
                    // centers black. This numerical-zero diagnostic does not
                    // turn near-contour pixels into a fitted tolerance rule.
                    let distance = if inside && e != 0 {
                        f64::INFINITY
                    } else {
                        polys.iter().map(|p| contour_distance(p, x as f64 + 0.5, y as f64 + 0.5)).fold(f64::INFINITY, f64::min)
                    };
                    let boundary_inside = inside || distance <= 1e-9;
                    boundary_differences += usize::from(boundary_inside != (e != 0));
                    if inside != (e != 0) {
                        differences.push((x, y, inside, e, distance));
                    }
                }
            }
            println!(
                "tol{tolerance} fixed26.6={fixed_grid} fixed-midpoints={fixed_subdivision} {text:?} difference{} boundary-rule2={boundary_differences} first{:?}",
                differences.len(),
                differences.first()
            );
            total += differences.len();
            boundary_total += boundary_differences;
        }
        println!("tol{tolerance} fixed26.6={fixed_grid} fixed-midpoints={fixed_subdivision} TOTAL{total} boundary-rule2={boundary_total}");
    }
}
