//! Private exact-font public outline parity probe; no production admission.
use serde_json::{Value, json};
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
};
#[derive(Default)]
struct Pen {
    commands: Vec<Value>,
    start: [i64; 2],
    current: [i64; 2],
}
fn quantize(x: f32, y: f32) -> [i64; 2] {
    [(f64::from(x) * 64.).round() as i64, (f64::from(y) * 64.).round() as i64]
}
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.current = quantize(x, y);
        self.start = self.current;
        self.commands.push(json!({"kind":"move","to":self.current}));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.current = quantize(x, y);
        self.commands.push(json!({"kind":"line","to":self.current}));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.current = quantize(x, y);
        self.commands.push(json!({"kind":"quadratic","control":quantize(cx,cy),"to":self.current}));
    }
    fn curve_to(&mut self, cx1: f32, cy1: f32, cx2: f32, cy2: f32, x: f32, y: f32) {
        self.current = quantize(x, y);
        self.commands.push(json!({"kind":"cubic","control1":quantize(cx1,cy1),"control2":quantize(cx2,cy2),"to":self.current}));
    }
    fn close(&mut self) {
        if self.current != self.start {
            self.commands.push(json!({"kind":"line","to":self.start}));
            self.current = self.start;
        }
    }
}
#[test]
#[ignore = "private public FreeType decomposition vs portable outline quantization contract"]
fn exact_authority_quantized_outline_commands_match_public_decomposition() {
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text");
    let data: Value = serde_json::from_slice(&std::fs::read(base.join("curve-kern-freetype-evidence/outlines.json")).unwrap()).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(photocraft_text::fonts::INTER_REGULAR.as_slice())), data["font_sha256"].as_str().unwrap());
    let face = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let mut results = Vec::new();
    let mut mismatch = 0usize;
    for row in data["rows"].as_array().unwrap() {
        let case = row["case"].as_str().unwrap();
        let size: f32 = case.split('-').nth(1).unwrap().parse().unwrap();
        for glyph in row["glyphs"].as_array().unwrap() {
            let id = glyph["id"].as_u64().unwrap() as u32;
            let mut pen = Pen::default();
            face.outline_glyphs()
                .get(skrifa::GlyphId::new(id))
                .unwrap()
                .draw(DrawSettings::unhinted(Size::new(size), LocationRef::default()), &mut pen)
                .unwrap();
            let expected = glyph["decomposed"].as_array().unwrap();
            let different = pen.commands != *expected;
            mismatch += usize::from(different);
            results.push(json!({"case":case,"glyph":id,"different":different,"portable":pen.commands,"public":expected}));
        }
    }
    std::fs::write(base.join("curve-kern-freetype-basis/portable-outline-parity.json"), serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    assert_eq!(mismatch, 0, "portable point rounding does not reproduce the observed public outline contract");
}

#[test]
#[ignore = "private whole-string positioning against independently rendered exact-font controls"]
fn portable_whole_string_advances_clusters_and_baselines_match_controls() {
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-xcf-text");
    let evidence = std::fs::read(base.join("curve-kern-freetype-evidence/outlines.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&evidence)), "dea7aefa140f4778318c7f21a01692fc9bc0ecc608516e33ea3921737476a453");
    let data: Value = serde_json::from_slice(&evidence).unwrap();
    let font = harfrust::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let shaping = harfrust::ShaperData::new(&font);
    let shaper = shaping.shaper(&font).build();
    let face = skrifa::FontRef::new(photocraft_text::fonts::INTER_REGULAR.as_slice()).unwrap();
    let mut results = Vec::new();
    for row in data["rows"].as_array().unwrap() {
        let case = row["case"].as_str().unwrap();
        let mut parts = case.splitn(3, '-');
        assert_eq!(parts.next(), Some("aa"));
        let size: f32 = parts.next().unwrap().parse().unwrap();
        let content = parts.next().unwrap();
        let mut buffer = harfrust::UnicodeBuffer::new();
        buffer.push_str(content);
        buffer.set_direction(harfrust::Direction::LeftToRight);
        buffer.set_script(harfrust::script::LATIN);
        let shaped = shaper.shape(buffer, harfrust::ShapeOptions::default());
        let baseline = face.metrics(Size::new(size), LocationRef::default()).ascent.ceil();
        let mut x = 0.;
        let mut glyphs = Vec::new();
        for (info, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            assert_eq!([position.x_offset, position.y_offset, position.y_advance], [0; 3]);
            glyphs.push(json!({"id":info.glyph_id,"cluster":info.cluster,"x":x,"baseline":baseline}));
            x += (f64::from(position.x_advance) * f64::from(size) / f64::from(shaper.units_per_em())).round();
        }
        assert_eq!(glyphs.len(), row["glyphs"].as_array().unwrap().len());
        for (actual, expected) in glyphs.iter().zip(row["glyphs"].as_array().unwrap()) {
            for field in ["id", "cluster", "x", "baseline"] {
                assert_eq!(actual[field].as_f64(), expected[field].as_f64(), "{case}: {field}");
            }
        }
        results.push(json!({"case":case,"glyphs":glyphs,"rounded_total_advance":x}));
    }
    assert_eq!(results.len(), 24);
    std::fs::write(base.join("curve-kern-freetype-basis/portable-positioning-parity.json"), serde_json::to_vec_pretty(&results).unwrap()).unwrap();
}
