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
    F16,
    F32,
}
impl Sample {
    pub fn source_bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 | Self::F16 => 2,
            Self::F32 => 4,
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
pub struct Drawable {
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
    pub mask: Option<Box<Drawable>>,
    pub plane: Option<Plane>,
    pub selection: bool,
    pub unsupported: Vec<String>,
    pub diagnostics: Vec<String>,
}
impl Default for Drawable {
    fn default() -> Self {
        Self {
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
        }
    }
}
#[derive(Clone, Debug)]
pub struct Guide {
    pub position: i32,
    pub horizontal: bool,
}
#[derive(Clone, Debug)]
pub struct XcfFile {
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
    pub palette: Vec<[u8; 3]>,
    pub layers: Vec<Drawable>,
    pub channels: Vec<Drawable>,
    pub guides: Vec<Guide>,
    pub diagnostics: Vec<String>,
    pub charged_bytes: usize,
}
