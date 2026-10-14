/// Hard ceilings still apply when a caller supplies looser budgets.
#[derive(Clone, Debug)]
pub struct Limits {
    pub input_bytes: usize,
    pub resident_bytes: usize,
    pub side: u32,
    pub pixels: u64,
    pub drawables: usize,
    pub properties: usize,
    pub string_bytes: usize,
    pub profile_bytes: usize,
    pub property_bytes: usize,
    pub metadata_bytes: usize,
    pub visits: usize,
    pub group_depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            input_bytes: if cfg!(target_arch = "wasm32") { 256 << 20 } else { 1 << 30 },
            resident_bytes: if cfg!(target_arch = "wasm32") { 512 << 20 } else { 2 << 30 },
            side: 262144,
            pixels: 1 << 30,
            drawables: 10000,
            properties: 100000,
            string_bytes: 1 << 20,
            profile_bytes: 16 << 20,
            property_bytes: 64 << 20,
            metadata_bytes: 256 << 20,
            visits: 20000000,
            group_depth: 100,
        }
    }
}
/// Callbacks must not mutate input. Progress is monotonic in 0..=1.
#[derive(Default)]
pub struct DecodeOptions<'a> {
    pub limits: Limits,
    /// Preserve finite signed/over-one drawable alpha for a caller that performs
    /// its own scoped rendering-policy validation after parsing. This is not
    /// rendering admission. Default decoding, channels, and masks remain unit coverage.
    pub allow_finite_drawable_alpha: bool,
    pub cancelled: Option<&'a dyn Fn() -> bool>,
    pub progress: Option<&'a dyn Fn(f32)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Rgb,
    Gray,
    Indexed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    U8,
    U16,
    U32,
    F16,
    F32,
    F64,
}
impl Sample {
    pub fn source_bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 | Self::F16 => 2,
            Self::U32 | Self::F32 => 4,
            Self::F64 => 8,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Linear,
    NonlinearProfile,
    Perceptual,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    Rle,
    Zlib,
}
#[derive(Clone, Debug)]
pub struct Plane {
    pub width: u32,
    pub height: u32,
    pub channels: usize,
    /// F16 is widened exactly to F32. Other sample types retain native-endian bytes.
    pub sample: Sample,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct RawProperty {
    pub tag: u32,
    /// Offset of payload in logical unwrapped XCF, not encoded wrapper bytes.
    pub offset: u64,
    pub declared_bytes: u32,
    pub payload: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct Effect<P = Plane> {
    pub name: String,
    pub icon: String,
    pub operation: String,
    pub version: Option<String>,
    pub properties: Vec<RawProperty>,
    pub arguments: Vec<Argument>,
    pub mask: Option<Box<Drawable<P>>>,
    pub state: EffectState,
}
/// Stored presence and original numeric values; absent fields are not resolved defaults.
#[derive(Clone, Debug, Default)]
pub struct EffectState {
    pub enabled: Option<bool>,
    pub opacity_u8: Option<u8>,
    pub opacity_bits: Option<u32>,
    pub mode: Option<i32>,
    pub composite_mode: Option<i32>,
    pub composite_space: Option<i32>,
    pub blend_space: Option<i32>,
    pub clipping: Option<u32>,
    pub region: Option<u32>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Argument {
    pub name: String,
    /// Index in the containing effect's raw properties, retaining the envelope.
    pub property_index: usize,
    pub value: ArgumentValue,
    pub configuration: Option<EffectConfiguration>,
    pub graph: Option<StoredGraph>,
}
/// Ordered GEGL chain syntax only; identities/resources are not executed/resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredGraph {
    pub entries: Vec<GraphEntry>,
    pub nodes: u64,
    pub edges: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub enum GraphEntry {
    Operation { identity: String, start: u64, end: u64 },
    Property { name: String, value: GraphValue, start: u64, end: u64 },
}
#[derive(Clone, Debug, PartialEq)]
pub enum GraphValue {
    Literal { source: String, numeric_bits: Option<u64> },
    Chain(Vec<GraphEntry>),
}
#[derive(Clone, Debug, PartialEq)]
pub struct EffectConfiguration {
    pub nodes: Vec<ConfigNode>,
    pub typed: Option<TypedConfiguration>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum TypedConfiguration {
    Levels(LevelsConfiguration),
    Curves(CurvesConfiguration),
    ColorBalance(ColorBalanceConfiguration),
    BrightnessContrast(BrightnessContrastConfiguration),
    HueSaturation(HueSaturationConfiguration),
}
/// Ordered ALL, RED, YELLOW, GREEN, CYAN, BLUE, MAGENTA native configuration words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HueSaturationConfiguration {
    pub time_bits: Option<u64>,
    /// Each selector retains hue, saturation and lightness, in that order.
    pub ranges: [[u64; 3]; 7],
    pub overlap_bits: u64,
}
/// Exact stock GIMP wrapper settings, separate from GEGL affine parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrightnessContrastConfiguration {
    pub time_bits: Option<u64>,
    pub brightness_bits: u64,
    pub contrast_bits: u64,
}
/// Ordered, exact scalar settings from a stock Color Balance configuration.
/// This is source data; it does not qualify an evaluator or invent defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorBalanceConfiguration {
    pub time_bits: Option<u64>,
    pub ranges: Vec<ColorBalanceRange>,
    pub preserve_luminosity: Option<bool>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ColorBalanceRange {
    pub range: String,
    pub cyan_red_bits: u64,
    pub magenta_green_bits: u64,
    pub yellow_blue_bits: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct LevelsConfiguration {
    pub time_bits: Option<u64>,
    pub trc: Option<String>,
    pub clamp_input: Option<bool>,
    pub clamp_output: Option<bool>,
    pub channels: Vec<LevelsChannel>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct LevelsChannel {
    pub channel: String,
    pub low_input_bits: u64,
    pub high_input_bits: u64,
    pub gamma_bits: u64,
    pub low_output_bits: u64,
    pub high_output_bits: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CurvesConfiguration {
    pub time_bits: Option<u64>,
    pub trc: Option<String>,
    pub channels: Vec<CurveChannel>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CurveChannel {
    pub channel: String,
    pub curve: StoredCurve,
}
#[derive(Clone, Debug, PartialEq)]
pub struct StoredCurve {
    pub curve_type: String,
    pub points_bits: Vec<u64>,
    pub point_types: Vec<String>,
    pub n_samples: u32,
    pub samples_bits: Vec<u64>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ArgumentValue {
    Integer(i32),
    Boolean(bool),
    Unsigned(u32),
    Float32Bits(u32),
    Float64Bits(u64),
    String(String),
    Enum(i32),
    /// Exact config text only; parsing its complete grammar remains separate.
    Config(String),
    /// Explicit nullable GeglColor framing, never a transparent-color substitute.
    /// The exact original argument bytes remain in the enclosing RawProperty.
    NullColor,
    Color {
        encoding: String,
        samples: Vec<u8>,
        icc: Vec<u8>,
    },
    /// Unknown types retain their bytes in the raw property, never default values.
    Unknown(u32),
}
#[derive(Clone, Debug)]
pub struct Drawable<P = Plane> {
    pub record_offset: u64,
    pub tattoo: Option<u32>,
    pub active: bool,
    pub legacy_linked: bool,
    pub group_flags: u32,
    pub color_tag: Option<u32>,
    pub mask_editing: bool,
    pub mask_shown: bool,
    pub show_masked: bool,
    pub channel_color: Option<[f32; 3]>,
    pub floating_attachment: Option<u64>,
    pub item_sets: Vec<u32>,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub model: Model,
    pub alpha: bool,
    pub offset: (i32, i32),
    pub opacity: f32,
    pub visible: bool,
    pub lock_alpha: bool,
    pub lock_content: bool,
    pub lock_position: bool,
    pub lock_visibility: bool,
    pub group: bool,
    pub path: Vec<u32>,
    pub mode: i32,
    pub composite_mode: i32,
    pub composite_space: i32,
    pub blend_space: i32,
    pub mask_enabled: bool,
    pub mask: Option<Box<Drawable<P>>>,
    pub plane: Option<P>,
    pub selection: bool,
    pub unsupported: Vec<String>,
    pub diagnostics: Vec<String>,
    pub raw_properties: Vec<RawProperty>,
    pub effects: Vec<Effect<P>>,
    pub link: Option<LinkedLayer>,
    pub transform: Option<LayerTransform>,
    pub text: Option<TextLayer>,
    pub text_flags: u32,
    pub vector: Option<VectorLayer>,
}
impl<P> Default for Drawable<P> {
    fn default() -> Self {
        Self {
            record_offset: 0,
            tattoo: None,
            active: false,
            legacy_linked: false,
            group_flags: 0,
            color_tag: None,
            mask_editing: false,
            mask_shown: false,
            show_masked: false,
            channel_color: None,
            floating_attachment: None,
            item_sets: Vec::new(),
            name: String::new(),
            width: 0,
            height: 0,
            model: Model::Rgb,
            alpha: false,
            offset: (0, 0),
            opacity: 1.0,
            visible: true,
            lock_alpha: false,
            lock_content: false,
            lock_position: false,
            lock_visibility: false,
            group: false,
            path: Vec::new(),
            mode: 0,
            composite_mode: 1,
            composite_space: 2,
            blend_space: 2,
            mask_enabled: true,
            mask: None,
            plane: None,
            selection: false,
            unsupported: Vec::new(),
            diagnostics: Vec::new(),
            raw_properties: Vec::new(),
            effects: Vec::new(),
            link: None,
            transform: None,
            text: None,
            text_flags: 0,
            vector: None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct StoredColor {
    pub encoding: String,
    pub samples: Vec<u8>,
    pub icc: Vec<u8>,
}
/// Inert resource identity; no filesystem or runtime lookup is authorized.
#[derive(Clone, Debug)]
pub struct StoredResource {
    pub name: String,
    pub collection: String,
    pub internal: bool,
}
#[derive(Clone, Debug)]
pub struct VectorLayer {
    pub source_modified: bool,
    pub path_tattoo: u32,
    pub filled: bool,
    pub stroked: bool,
    pub fill: VectorFill,
    pub stroke: VectorStroke,
}
#[derive(Clone, Debug)]
pub struct VectorFill {
    pub style: u32,
    pub antialias: bool,
    pub color: Option<StoredColor>,
    pub pattern: Option<StoredResource>,
}
#[derive(Clone, Debug)]
pub struct VectorStroke {
    pub fill: VectorFill,
    pub width_bits: u32,
    pub cap_style: u32,
    pub join_style: u32,
    pub miter_limit_bits: u32,
    pub dashes_bits: Vec<u32>,
}
impl<P> Drawable<P> {
    pub fn text_source_modified(&self) -> bool {
        self.text_flags & 2 != 0
    }
}
#[derive(Clone, Debug)]
pub struct TextLayer {
    pub parasite_flags: u32,
    pub raw: Vec<u8>,
    pub text: Option<String>,
    pub markup: Option<String>,
    pub font: Option<FontReference>,
    pub configuration: Vec<ConfigNode>,
    pub unknown_fields: Vec<String>,
    /// Plain UTF8 content after bounded markup/entity decoding.
    pub content: String,
    pub runs: Vec<TextRun>,
    pub styles: Vec<MarkupStyle>,
    pub layout: TextLayout,
}
#[derive(Clone, Debug, Default)]
pub struct TextLayout {
    pub font_size_bits: Option<u64>,
    pub font_size_unit: Option<String>,
    pub antialias: Option<bool>,
    pub hinting: Option<bool>,
    pub language: Option<String>,
    pub direction: Option<String>,
    pub justification: Option<String>,
    pub indent_bits: Option<u64>,
    pub line_spacing_bits: Option<u64>,
    pub letter_spacing_bits: Option<u64>,
    pub box_mode: Option<String>,
    pub box_unit: Option<String>,
    pub box_width_bits: Option<u64>,
    pub box_height_bits: Option<u64>,
    pub offset_x_bits: Option<u64>,
    pub offset_y_bits: Option<u64>,
    pub transformation_bits: Option<[u64; 4]>,
}
#[derive(Clone, Debug)]
pub struct TextRun {
    /// UTF8 byte boundaries in TextLayer.content, never character indices.
    pub start: u64,
    pub end: u64,
    /// Outer-to-inner style declaration IDs; their semantics are not normalized.
    pub style_ids: Vec<usize>,
}
#[derive(Clone, Debug)]
pub struct MarkupStyle {
    pub tag: String,
    pub attributes: Vec<(String, String)>,
}
/// Inert requested identity only. No font files are opened by this reader.
#[derive(Clone, Debug, Default)]
pub struct FontReference {
    pub name: Option<String>,
    pub family: Option<String>,
    pub style: Option<String>,
    pub postscript_name: Option<String>,
    pub hash: Option<String>,
    pub index: Option<u32>,
    pub weight: Option<i32>,
    pub slant: Option<i32>,
    pub width: Option<i32>,
    pub version: Option<u32>,
    pub unknown_fields: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigNode {
    /// UTF8/binary config byte offsets into the original parasite, not characters.
    pub start: u64,
    pub end: u64,
    pub value: ConfigValue,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValue {
    Atom(String),
    /// Original decimal spelling plus finite parsed f64, without losing spelling.
    Number {
        literal: String,
        bits: u64,
    },
    /// Decoded C-style bytes; text fields must separately validate UTF8.
    Quoted(Vec<u8>),
    List(Vec<ConfigNode>),
}
/// Stored reference only. Decoding does not grant access to this path.
#[derive(Clone, Debug)]
pub struct LinkedLayer {
    pub flags: u32,
    pub path: String,
    pub width: u32,
    pub height: u32,
}
impl LinkedLayer {
    /// Source name is independent of later link-path edits.
    pub fn custom_name(&self) -> bool {
        self.flags & 1 != 0
    }
    /// Stored GIMP monitoring state, not permission to start a watcher or read.
    pub fn monitoring_disabled(&self) -> bool {
        self.flags & 2 != 0
    }
    pub fn unknown_flags(&self) -> u32 {
        self.flags & !3
    }
}
#[derive(Clone, Debug)]
pub struct LayerTransform {
    pub offset: (i32, i32),
    pub interpolation: u32,
    /// Row-major original finite f32 coefficients, with no normalization.
    pub matrix: [f32; 9],
}
#[derive(Clone, Debug)]
pub struct Guide {
    pub position: i32,
    pub horizontal: bool,
}
#[derive(Clone, Debug)]
pub struct XcfFile<P = Plane> {
    pub paths: Vec<VectorPath>,
    pub tattoo: Option<u32>,
    pub item_sets: Vec<ItemSet>,
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub model: Model,
    pub precision: u32,
    pub sample: Sample,
    pub encoding: Encoding,
    pub compression: Compression,
    pub icc: Vec<u8>,
    pub dpi: (f32, f32),
    pub unit: Unit,
    pub palette: Vec<[u8; 3]>,
    pub layers: Vec<Drawable<P>>,
    pub channels: Vec<Drawable<P>>,
    pub guides: Vec<Guide>,
    pub diagnostics: Vec<String>,
    pub charged_bytes: usize,
    pub raw_properties: Vec<RawProperty>,
}
#[derive(Clone, Debug, Default)]
pub struct VectorPath {
    /// Whole standalone record; legacy paths are retained in image raw properties.
    pub raw_record: Vec<u8>,
    pub legacy_format: Option<u32>,
    pub legacy_state: Option<u8>,
    pub record_offset: Option<u64>,
    pub name: String,
    pub tattoo: Option<u32>,
    pub selected: bool,
    pub visible: bool,
    pub legacy_linked: bool,
    pub lock_content: bool,
    pub lock_position: bool,
    pub lock_visibility: bool,
    pub color_tag: Option<u32>,
    pub item_sets: Vec<u32>,
    pub properties: Vec<RawProperty>,
    pub parasites: Vec<PathParasite>,
    pub strokes: Vec<PathStroke>,
    pub unsupported: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct PathParasite {
    pub name: String,
    pub flags: u32,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct PathStroke {
    pub closed: bool,
    pub points: Vec<PathPoint>,
}
#[derive(Clone, Debug)]
pub struct PathPoint {
    pub kind: u32,
    /// Present only for integer coordinates in legacy PROP_PATHS format 1.
    pub legacy_integer_coordinates: Option<[i32; 2]>,
    /// Stored finite f32 component bits in x/y/pressure/tilt/tilt/wheel order.
    pub components: Vec<u32>,
}
/// Organisational declaration only: search/regex/glob text is never executed.
#[derive(Clone, Debug)]
pub struct ItemSet {
    pub item_type: u32,
    pub method: u32,
    pub name: String,
}
/// Physical display unit. Resolution is always pixels per inch independently.
#[derive(Clone, Debug, PartialEq)]
pub enum Unit {
    Inch,
    Millimeter,
    Point,
    Pica,
    Custom(CustomUnit),
}
#[derive(Clone, Debug, PartialEq)]
pub struct CustomUnit {
    /// Number of these units in one inch, preserving the original f32 bits.
    pub factor: f32,
    pub digits: u32,
    /// Original identifier before XCF21, display name in XCF21 and later.
    pub name: String,
    pub symbol: String,
    pub abbreviation: String,
    pub singular: Option<String>,
    pub plural: Option<String>,
}
