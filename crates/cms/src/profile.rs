//! ICC profiles: the parsed model, the reader (ICC.1:2001-04 v2 and ICC.1:2010 v4) and the
//! connection of a profile to the PCS as evaluation stages.

use std::sync::Arc;

use crate::clut::Clut;
use crate::curve::Curve;
use crate::math::{self, Mat3};
use crate::pipeline::Stage;
use crate::{CmsError, Intent};

mod controlled_parse;
pub use controlled_parse::ParseRequirements;
pub use crate::write::EncodeRequirements;

/// Profile/device class (header bytes 12..16).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProfileClass {
    Input,
    Display,
    Output,
    DeviceLink,
    ColorSpace,
    Abstract,
    NamedColor,
    Unknown(u32),
}

impl ProfileClass {
    pub fn from_sig(s: u32) -> Self {
        match &s.to_be_bytes() {
            b"scnr" => Self::Input,
            b"mntr" => Self::Display,
            b"prtr" => Self::Output,
            b"link" => Self::DeviceLink,
            b"spac" => Self::ColorSpace,
            b"abst" => Self::Abstract,
            b"nmcl" => Self::NamedColor,
            _ => Self::Unknown(s),
        }
    }
    pub fn sig(self) -> u32 {
        u32::from_be_bytes(*match self {
            Self::Input => b"scnr",
            Self::Display => b"mntr",
            Self::Output => b"prtr",
            Self::DeviceLink => b"link",
            Self::ColorSpace => b"spac",
            Self::Abstract => b"abst",
            Self::NamedColor => b"nmcl",
            Self::Unknown(s) => return s,
        })
    }
}

/// Data colour space (header bytes 16..20).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    Xyz,
    Lab,
    Luv,
    YCbCr,
    Yxy,
    Rgb,
    Gray,
    Hsv,
    Hls,
    Cmyk,
    Cmy,
    /// `nCLR` spaces with n = 2..=15 colorants.
    Color(u8),
    Unknown(u32),
}

impl ColorSpace {
    pub fn from_sig(s: u32) -> Self {
        let b = s.to_be_bytes();
        match &b {
            b"XYZ " => Self::Xyz,
            b"Lab " => Self::Lab,
            b"Luv " => Self::Luv,
            b"YCbr" => Self::YCbCr,
            b"Yxy " => Self::Yxy,
            b"RGB " => Self::Rgb,
            b"GRAY" => Self::Gray,
            b"HSV " => Self::Hsv,
            b"HLS " => Self::Hls,
            b"CMYK" => Self::Cmyk,
            b"CMY " => Self::Cmy,
            [d, b'C', b'L', b'R'] => match *d {
                b'2'..=b'9' => Self::Color(d - b'0'),
                b'A'..=b'F' => Self::Color(d - b'A' + 10),
                _ => Self::Unknown(s),
            },
            _ => Self::Unknown(s),
        }
    }
    pub fn sig(self) -> u32 {
        u32::from_be_bytes(match self {
            Self::Xyz => *b"XYZ ",
            Self::Lab => *b"Lab ",
            Self::Luv => *b"Luv ",
            Self::YCbCr => *b"YCbr",
            Self::Yxy => *b"Yxy ",
            Self::Rgb => *b"RGB ",
            Self::Gray => *b"GRAY",
            Self::Hsv => *b"HSV ",
            Self::Hls => *b"HLS ",
            Self::Cmyk => *b"CMYK",
            Self::Cmy => *b"CMY ",
            Self::Color(n) => [if n < 10 { b'0' + n } else { b'A' + n - 10 }, b'C', b'L', b'R'],
            Self::Unknown(s) => return s,
        })
    }
    /// Number of channels of the space.
    pub fn channels(self) -> usize {
        match self {
            Self::Gray => 1,
            Self::Cmyk => 4,
            Self::Color(n) => n as usize,
            Self::Unknown(_) => 0,
            _ => 3,
        }
    }
}

/// Which PCS a stage list ends in (float values: XYZ with Y = 1 for the white, or Lab L 0–100).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pcs {
    Xyz,
    Lab,
}

/// Encoding family of a LUT tag, which fixes how its PCS side is encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutKind {
    /// `mft1` (8-bit), v4-style Lab encoding.
    Lut8,
    /// `mft2` (16-bit), legacy v2 Lab encoding.
    Lut16,
    /// `mAB ` / `mBA ` (v4), v4 Lab encoding.
    Ab,
}

/// A parsed `AToB`/`BToA` tag: device-side and PCS-side values are normalised to `[0, 1]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    pub kind: LutKind,
    pub inputs: usize,
    pub outputs: usize,
    pub stages: Vec<Stage>,
}

/// Largest colour difference (CIE ΔE*ab) for which [`Profile::same_colors`] treats two profiles
/// as one colour space: invisible, yet 20× what re-encoding a profile costs (sRGB as v2 tables
/// against v4 parametric curves differs by 0.025). Moving sRGB's red primary 2% costs 1.0.
pub const SAME_COLORS_MAX_DELTA_E: f64 = 0.5;

/// A parsed ICC profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Major version and minor.bugfix byte (e.g. `(4, 0x30)` for 4.3, `(2, 0x10)` for 2.1).
    pub version: (u8, u8),
    pub class: ProfileClass,
    pub color_space: ColorSpace,
    pub pcs: Pcs,
    pub rendering_intent: Intent,
    pub description: String,
    pub copyright: String,
    /// `wtpt` as stored (XYZ; D50 when absent).
    pub white_point: [f64; 3],
    /// `bkpt` when present.
    pub black_point: Option<[f64; 3]>,
    /// `chad` chromatic adaptation matrix when present.
    pub chad: Option<Mat3>,
    /// Colorant matrix (columns rXYZ, gXYZ, bXYZ), D50-adapted.
    pub matrix: Option<Mat3>,
    pub trc: Option<[Curve; 3]>,
    pub gray_trc: Option<Curve>,
    /// A2B0/A2B1/A2B2 (perceptual, colorimetric, saturation).
    pub a2b: [Option<Lut>; 3],
    pub b2a: [Option<Lut>; 3],
    /// The bytes this profile was parsed from (or encoded to), kept for byte-exact embedding.
    pub(crate) bytes: Option<Arc<Vec<u8>>>,
    pub(crate) hash: std::sync::OnceLock<u64>,
}

impl Profile {
    /// Parses ICC profile bytes.
    pub fn parse(bytes: &[u8]) -> Result<Profile, CmsError> {
        let mut p = parse_profile(bytes)?;
        p.bytes = Some(Arc::new(bytes.to_vec()));
        Ok(p)
    }

    /// Borrowed pre-allocation census for the additive controlled ICC parser.
    /// Peak includes borrowed bytes, their retained original copy, decoded model,
    /// and conservative tag/text/CLUT-shadow scratch. No profile policy inferred.
    pub fn parse_requirements_with_control(bytes: &[u8], limits: crate::control::Limits, cancelled: &dyn Fn() -> bool) -> crate::control::Result<ParseRequirements> {
        controlled_parse::requirements(bytes, limits, cancelled)
    }

    /// Fallible, cooperative equivalent of parse for the existing ICC families.
    /// Keeps the complete original input, including tolerated padding/raw tail.
    pub fn parse_with_control(bytes: &[u8], limits: crate::control::Limits, cancelled: &dyn Fn() -> bool) -> crate::control::Result<Self> {
        controlled_parse::parse(bytes, limits, cancelled)
    }

    /// Returns only the retained original/cached encoding, without generating or
    /// allocating a new ICC profile. `None` means the caller must explicitly
    /// account for an encoding operation before using [`Self::to_bytes`].
    pub fn encoded_bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref().map(Vec::as_slice)
    }

    /// The ICC bytes of this profile, encoding it when no encoding is retained.
    pub fn to_bytes(&self) -> Arc<Vec<u8>> {
        match &self.bytes {
            Some(b) => b.clone(),
            None => Arc::new(crate::write::encode(self)),
        }
    }

    /// Re-encodes the model as ICC v4 bytes and caches them (used for built-in profiles).
    pub fn with_encoded_bytes(mut self) -> Profile {
        self.bytes = None;
        self.hash = Default::default();
        let b = crate::write::encode(&self);
        self.bytes = Some(Arc::new(b));
        self
    }

    pub fn channels(&self) -> usize {
        self.color_space.channels()
    }

    /// Hash of the profile's ICC bytes (stable within a process; used as a cache key).
    pub fn content_hash(&self) -> u64 {
        *self.hash.get_or_init(|| {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            self.to_bytes().hash(&mut h);
            h.finish()
        })
    }

    /// Whether `self` and `other` describe the same colours: same colour space, and every sample
    /// of a device-value grid has the same colour through both (relative colorimetric, within
    /// [`SAME_COLORS_MAX_DELTA_E`]). Different encodings of one space are the same colours: the
    /// sRGB IEC61966-2.1 Photoshop embeds (v2, 1024-entry tables) and the built-in sRGB (v4,
    /// parametric curves) differ in every byte. Colours are compared rather than round-tripped
    /// because CMYK → PCS → CMYK is not an identity even through a single profile.
    pub fn same_colors(&self, other: &Profile) -> bool {
        if self.content_hash() == other.content_hash() {
            return true;
        }
        let n = self.channels();
        if self.color_space != other.color_space || n == 0 || n > 4 {
            return false;
        }
        let lab = crate::builtin::Builtin::LabD50.profile();
        let to_lab = |p: &Profile| crate::transform::Transform::new(p, lab, Intent::RelativeColorimetric, false);
        let (Ok(a), Ok(b)) = (to_lab(self), to_lab(other)) else {
            return false;
        };
        // 17 steps per channel catch a TRC that differs only in its toe; 9 keep CMYK at 6561 samples.
        let steps: usize = if n <= 3 { 17 } else { 9 };
        // Lab D50 is the PCS, normalised as in ICC v4.
        let decode = |o: [f32; 3]| [f64::from(o[0]) * 100.0, f64::from(o[1]) * 255.0 - 128.0, f64::from(o[2]) * 255.0 - 128.0];
        let mut input = [0f32; 4];
        (0..steps.pow(n as u32)).all(|i| {
            let mut k = i;
            for v in input.iter_mut().take(n) {
                *v = (k % steps) as f32 / (steps - 1) as f32;
                k /= steps;
            }
            let (mut la, mut lb) = ([0f32; 3], [0f32; 3]);
            a.eval(&input[..n], &mut la);
            b.eval(&input[..n], &mut lb);
            math::delta_e76(decode(la), decode(lb)) <= SAME_COLORS_MAX_DELTA_E
        })
    }

    /// Media white point used for ICC-absolute colorimetry: `wtpt`, except for v2 display
    /// profiles whose `wtpt` records the (unadapted) display white; those are treated as D50.
    pub fn media_white(&self) -> [f64; 3] {
        if self.version.0 < 4 && self.class == ProfileClass::Display {
            return math::D50;
        }
        self.white_point
    }

    /// An RGB view of a gray TRC profile: each of R, G, B carries the gray curve and maps to a
    /// third of the white, so `(v, v, v)` has exactly the colour of gray `v`. Used for
    /// composites of gray documents, which are stored as replicated RGB.
    pub fn gray_as_rgb(&self) -> Option<Profile> {
        let c = self.gray_trc.clone()?;
        if self.pcs != Pcs::Xyz {
            return None;
        }
        let w = math::D50;
        let m = [[w[0] / 3.0; 3], [w[1] / 3.0; 3], [w[2] / 3.0; 3]];
        let mut p = self.clone();
        p.color_space = ColorSpace::Rgb;
        p.description = format!("{} (as RGB)", self.description);
        p.matrix = Some(m);
        p.trc = Some([c.clone(), c.clone(), c]);
        p.gray_trc = None;
        // The gray LUTs take one input channel; this profile is the matrix/TRC model above.
        p.a2b = Default::default();
        p.b2a = Default::default();
        p.bytes = None;
        p.hash = Default::default();
        Some(p)
    }

    /// `true` for matrix/TRC RGB or gray TRC profiles without LUTs.
    pub fn is_matrix_shaper(&self) -> bool {
        self.a2b.iter().all(Option::is_none) && (self.matrix.is_some() && self.trc.is_some() || self.gray_trc.is_some())
    }

    /// Whether colours can be converted into this profile for every intent. Input profiles with
    /// only AToB tables (Apple's Wide Color Sharing Profile on iPhone photos) can't: they
    /// describe a source but have no PCS → device path.
    pub fn can_be_destination(&self) -> bool {
        [Intent::Perceptual, Intent::RelativeColorimetric, Intent::Saturation].into_iter().all(|i| self.pcs_to_device(i).is_ok())
    }

    fn lut_for(luts: &[Option<Lut>; 3], intent: Intent) -> Option<&Lut> {
        let i = match intent {
            Intent::Perceptual => 0,
            Intent::RelativeColorimetric | Intent::AbsoluteColorimetric => 1,
            Intent::Saturation => 2,
        };
        luts[i].as_ref().or(luts[0].as_ref())
    }

    /// Whether the profile has a dedicated table for `intent` (matrix profiles support every
    /// intent colorimetrically).
    pub fn supports_intent(&self, intent: Intent, input: bool) -> bool {
        let luts = if input { &self.a2b } else { &self.b2a };
        let i = match intent {
            Intent::Perceptual => 0,
            Intent::RelativeColorimetric | Intent::AbsoluteColorimetric => 1,
            Intent::Saturation => 2,
        };
        luts[i].is_some() || luts.iter().all(Option::is_none)
    }

    /// Stages mapping normalised device values to the float PCS.
    pub fn device_to_pcs(&self, intent: Intent) -> Result<(Vec<Stage>, Pcs), CmsError> {
        if let Some(l) = Self::lut_for(&self.a2b, intent) {
            let mut st = l.stages.clone();
            st.push(decode_pcs(self.pcs, l.kind));
            return Ok((st, self.pcs));
        }
        match self.color_space {
            ColorSpace::Rgb => {
                let (Some(m), Some(trc)) = (&self.matrix, &self.trc) else {
                    return Err(CmsError::Unsupported("RGB profile without matrix/TRC or AToB tags".into()));
                };
                Ok((vec![Stage::Curves(trc.to_vec()), Stage::matrix3(m)], Pcs::Xyz))
            }
            ColorSpace::Gray => {
                let c = self.gray_trc.clone().ok_or_else(|| CmsError::Unsupported("gray profile without kTRC".into()))?;
                if self.pcs == Pcs::Lab {
                    // The curve yields L*/100.
                    let m = Stage::Matrix { rows: 3, cols: 1, m: vec![100.0, 0.0, 0.0], offset: vec![0.0; 3] };
                    Ok((vec![Stage::Curves(vec![c]), m], Pcs::Lab))
                } else {
                    let w = math::D50;
                    let m = Stage::Matrix { rows: 3, cols: 1, m: w.to_vec(), offset: vec![0.0; 3] };
                    Ok((vec![Stage::Curves(vec![c]), m], Pcs::Xyz))
                }
            }
            ColorSpace::Lab => Ok((vec![Stage::DecodeLabV4], Pcs::Lab)),
            ColorSpace::Xyz => Ok((vec![Stage::DecodeXyz], Pcs::Xyz)),
            cs => Err(CmsError::Unsupported(format!("{cs:?} profile without AToB tags"))),
        }
    }

    /// Stages mapping the float PCS (of the returned kind) to normalised device values.
    pub fn pcs_to_device(&self, intent: Intent) -> Result<(Vec<Stage>, Pcs), CmsError> {
        if let Some(l) = Self::lut_for(&self.b2a, intent) {
            let mut st = vec![encode_pcs(self.pcs, l.kind)];
            st.extend(l.stages.iter().cloned());
            return Ok((st, self.pcs));
        }
        match self.color_space {
            ColorSpace::Rgb => {
                let (Some(m), Some(trc)) = (&self.matrix, &self.trc) else {
                    return Err(CmsError::Unsupported("RGB profile without matrix/TRC or BToA tags".into()));
                };
                let inv = math::invert(m).ok_or_else(|| CmsError::Invalid("singular colorant matrix".into()))?;
                Ok((vec![Stage::matrix3(&inv), Stage::InvCurves(trc.to_vec())], Pcs::Xyz))
            }
            ColorSpace::Gray => {
                let c = self.gray_trc.clone().ok_or_else(|| CmsError::Unsupported("gray profile without kTRC".into()))?;
                if self.pcs == Pcs::Lab {
                    let m = Stage::Matrix { rows: 1, cols: 3, m: vec![0.01, 0.0, 0.0], offset: vec![0.0] };
                    Ok((vec![m, Stage::InvCurves(vec![c])], Pcs::Lab))
                } else {
                    let m = Stage::Matrix { rows: 1, cols: 3, m: vec![0.0, 1.0, 0.0], offset: vec![0.0] };
                    Ok((vec![m, Stage::InvCurves(vec![c])], Pcs::Xyz))
                }
            }
            ColorSpace::Lab => Ok((vec![Stage::EncodeLabV4], Pcs::Lab)),
            ColorSpace::Xyz => Ok((vec![Stage::EncodeXyz], Pcs::Xyz)),
            cs => Err(CmsError::Unsupported(format!("{cs:?} profile without BToA tags"))),
        }
    }
}

fn decode_pcs(pcs: Pcs, kind: LutKind) -> Stage {
    match (pcs, kind) {
        (Pcs::Xyz, _) => Stage::DecodeXyz,
        (Pcs::Lab, LutKind::Lut16) => Stage::DecodeLabV2,
        (Pcs::Lab, _) => Stage::DecodeLabV4,
    }
}

fn encode_pcs(pcs: Pcs, kind: LutKind) -> Stage {
    match (pcs, kind) {
        (Pcs::Xyz, _) => Stage::EncodeXyz,
        (Pcs::Lab, LutKind::Lut16) => Stage::EncodeLabV2,
        (Pcs::Lab, _) => Stage::EncodeLabV4,
    }
}

// ---------------------------------------------------------------- reader

struct Reader<'a> {
    b: &'a [u8],
}

impl<'a> Reader<'a> {
    fn slice(&self, off: usize, len: usize) -> Result<&'a [u8], CmsError> {
        off.checked_add(len).and_then(|end| self.b.get(off..end)).ok_or(CmsError::Truncated)
    }
    fn u8(&self, off: usize) -> Result<u8, CmsError> {
        self.b.get(off).copied().ok_or(CmsError::Truncated)
    }
    fn u16(&self, off: usize) -> Result<u16, CmsError> {
        let s = self.slice(off, 2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    fn u32(&self, off: usize) -> Result<u32, CmsError> {
        let s = self.slice(off, 4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn s15f16(&self, off: usize) -> Result<f64, CmsError> {
        Ok(self.u32(off)? as i32 as f64 / 65536.0)
    }
    fn xyz(&self, off: usize) -> Result<[f64; 3], CmsError> {
        Ok([self.s15f16(off)?, self.s15f16(off + 4)?, self.s15f16(off + 8)?])
    }
}

fn sig(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

/// Upper bound on CLUT nodes × outputs we accept, to reject hostile files.
const MAX_CLUT_VALUES: usize = 16 * 1024 * 1024;

fn parse_profile(bytes: &[u8]) -> Result<Profile, CmsError> {
    let r = Reader { b: bytes };
    if bytes.len() < 132 {
        return Err(CmsError::Truncated);
    }
    if r.slice(36, 4)? != b"acsp" {
        return Err(CmsError::BadSignature);
    }
    let declared = r.u32(0)? as usize;
    // Some writers pad or truncate; trust the smaller of the two.
    let end = if declared >= 132 && declared <= bytes.len() { declared } else { bytes.len() };
    let r = Reader { b: &bytes[..end] };
    let version = (r.u8(8)?, r.u8(9)?);
    let class = ProfileClass::from_sig(r.u32(12)?);
    let color_space = ColorSpace::from_sig(r.u32(16)?);
    let pcs_sig = r.u32(20)?;
    let pcs = match &pcs_sig.to_be_bytes() {
        b"Lab " => Pcs::Lab,
        _ => Pcs::Xyz,
    };
    let rendering_intent = Intent::from_u32(r.u32(64)? & 0xFFFF).unwrap_or(Intent::Perceptual);
    let count = r.u32(128)? as usize;
    if count > 1024 {
        return Err(CmsError::Invalid(format!("{count} tags")));
    }
    let mut tags: Vec<(u32, usize, usize)> = Vec::with_capacity(count);
    for i in 0..count {
        let o = 132 + i * 12;
        let (s, off, len) = (r.u32(o)?, r.u32(o + 4)? as usize, r.u32(o + 8)? as usize);
        tags.push((s, off, len));
    }
    let find = |s: &[u8; 4]| tags.iter().find(|t| t.0 == sig(s)).map(|t| (t.1, t.2));
    let tag_data = |s: &[u8; 4]| -> Result<Option<&[u8]>, CmsError> {
        match find(s) {
            Some((off, len)) => Ok(Some(r.slice(off, len)?)),
            None => Ok(None),
        }
    };

    let xyz_tag = |s: &[u8; 4]| -> Result<Option<[f64; 3]>, CmsError> {
        match tag_data(s)? {
            Some(d) if d.len() >= 20 && &d[..4] == b"XYZ " => Ok(Some(Reader { b: d }.xyz(8)?)),
            Some(_) => Err(CmsError::Invalid(format!("tag {} is not XYZType", String::from_utf8_lossy(s)))),
            None => Ok(None),
        }
    };
    let curve_tag = |s: &[u8; 4]| -> Result<Option<Curve>, CmsError> {
        match tag_data(s)? {
            Some(d) => Ok(Some(parse_curve(d)?.0)),
            None => Ok(None),
        }
    };

    let white_point = xyz_tag(b"wtpt")?.unwrap_or(math::D50);
    let black_point = xyz_tag(b"bkpt").ok().flatten();
    let chad = match tag_data(b"chad")? {
        Some(d) if d.len() >= 44 && &d[..4] == b"sf32" => {
            let rd = Reader { b: d };
            let mut m = [[0.0; 3]; 3];
            for (i, row) in m.iter_mut().enumerate() {
                for (j, v) in row.iter_mut().enumerate() {
                    *v = rd.s15f16(8 + (i * 3 + j) * 4)?;
                }
            }
            Some(m)
        }
        _ => None,
    };
    let matrix = match (xyz_tag(b"rXYZ")?, xyz_tag(b"gXYZ")?, xyz_tag(b"bXYZ")?) {
        (Some(rr), Some(g), Some(b)) => Some([[rr[0], g[0], b[0]], [rr[1], g[1], b[1]], [rr[2], g[2], b[2]]]),
        _ => None,
    };
    let trc = match (curve_tag(b"rTRC")?, curve_tag(b"gTRC")?, curve_tag(b"bTRC")?) {
        (Some(a), Some(b), Some(c)) => Some([a, b, c]),
        _ => None,
    };
    let gray_trc = curve_tag(b"kTRC")?;
    let lut = |s: &[u8; 4], a2b: bool| -> Result<Option<Lut>, CmsError> {
        match tag_data(s)? {
            Some(d) => parse_lut(d, a2b).map(Some),
            None => Ok(None),
        }
    };
    let a2b = [lut(b"A2B0", true)?, lut(b"A2B1", true)?, lut(b"A2B2", true)?];
    let b2a = [lut(b"B2A0", false)?, lut(b"B2A1", false)?, lut(b"B2A2", false)?];
    let description = tag_data(b"desc")?.map(parse_text).unwrap_or_default();
    let copyright = tag_data(b"cprt")?.map(parse_text).unwrap_or_default();

    let p = Profile {
        version,
        class,
        color_space,
        pcs,
        rendering_intent,
        description,
        copyright,
        white_point,
        black_point,
        chad,
        matrix,
        trc,
        gray_trc,
        a2b,
        b2a,
        bytes: None,
        hash: Default::default(),
    };
    // Validate channel counts of LUTs against the header. A DeviceLink connects two device
    // spaces: its header PCS field holds the output colour space (ICC.1:2010 §7.2.7), so a
    // CMYK→CMYK link has a 4→4 AToB0.
    let dev = p.color_space.channels();
    let conn = if p.class == ProfileClass::DeviceLink { ColorSpace::from_sig(pcs_sig).channels() } else { 3 };
    for l in p.a2b.iter().flatten() {
        if dev != 0 && l.inputs != dev || conn != 0 && l.outputs != conn {
            return Err(CmsError::Invalid(format!("AToB tag has {}→{} channels for a {:?} profile", l.inputs, l.outputs, p.color_space)));
        }
    }
    for l in p.b2a.iter().flatten() {
        if conn != 0 && l.inputs != conn || dev != 0 && l.outputs != dev {
            return Err(CmsError::Invalid(format!("BToA tag has {}→{} channels for a {:?} profile", l.inputs, l.outputs, p.color_space)));
        }
    }
    Ok(p)
}

/// Parses a `curv` or `para` element; returns the curve and the element's byte length
/// (unpadded).
fn parse_curve(d: &[u8]) -> Result<(Curve, usize), CmsError> {
    let r = Reader { b: d };
    match r.slice(0, 4)? {
        b"curv" => {
            let n = r.u32(8)? as usize;
            let len = 12 + n * 2;
            match n {
                0 => Ok((Curve::Identity, len)),
                1 => Ok((Curve::Gamma(r.u16(12)? as f64 / 256.0), len)),
                _ => {
                    if n > 1 << 20 {
                        return Err(CmsError::Invalid("curve too long".into()));
                    }
                    let s = r.slice(12, n * 2)?;
                    let t = s.as_chunks::<2>().0.iter().map(|c| f64::from(u16::from_be_bytes([c[0], c[1]])) / 65535.0).collect();
                    Ok((Curve::Table64(t), len))
                }
            }
        }
        b"para" => {
            let kind = r.u16(8)?;
            if kind > 4 {
                return Err(CmsError::Unsupported(format!("parametric curve type {kind}")));
            }
            let n = Curve::param_count(kind);
            let mut p = [0.0f64; 7];
            for (i, v) in p.iter_mut().enumerate().take(n) {
                *v = r.s15f16(12 + i * 4)?;
            }
            let len = 12 + n * 4;
            let c = if kind == 0 { Curve::Gamma(p[0]) } else { Curve::Parametric { kind, p } };
            Ok((c, len))
        }
        other => Err(CmsError::Unsupported(format!("curve type {:?}", String::from_utf8_lossy(other)))),
    }
}

fn parse_curves_at(d: &[u8], off: usize, n: usize) -> Result<Vec<Curve>, CmsError> {
    let mut out = Vec::with_capacity(n);
    let mut o = off;
    for _ in 0..n {
        let sub = d.get(o..).ok_or(CmsError::Truncated)?;
        let (c, len) = parse_curve(sub)?;
        out.push(c);
        o += (len + 3) & !3;
    }
    Ok(out)
}

fn parse_lut(d: &[u8], a2b: bool) -> Result<Lut, CmsError> {
    let r = Reader { b: d };
    let ty = r.slice(0, 4)?;
    match ty {
        b"mft1" | b"mft2" => {
            let wide = ty == b"mft2";
            let (i, o, g) = (r.u8(8)? as usize, r.u8(9)? as usize, r.u8(10)? as usize);
            if !(1..=15).contains(&i) || !(1..=15).contains(&o) || g == 0 {
                return Err(CmsError::Invalid("bad lut dimensions".into()));
            }
            let mut m = [[0.0; 3]; 3];
            for (a, row) in m.iter_mut().enumerate() {
                for (b, v) in row.iter_mut().enumerate() {
                    *v = r.s15f16(12 + (a * 3 + b) * 4)?;
                }
            }
            let (n_in, n_out, mut off) = if wide { (r.u16(48)? as usize, r.u16(50)? as usize, 52) } else { (256, 256, 48) };
            if wide && (!(2..=4096).contains(&n_in) || !(2..=4096).contains(&n_out)) {
                return Err(CmsError::Invalid("bad lut16 table size".into()));
            }
            let bps = if wide { 2 } else { 1 };
            let read_tables = |off: &mut usize, count: usize, len: usize| -> Result<Vec<Curve>, CmsError> {
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    let s = r.slice(*off, len * bps)?;
                    let t: Vec<f64> = if wide {
                        s.as_chunks::<2>().0.iter().map(|c| f64::from(u16::from_be_bytes([c[0], c[1]])) / 65535.0).collect()
                    } else {
                        s.iter().map(|b| f64::from(*b) / 255.0).collect()
                    };
                    *off += len * bps;
                    v.push(Curve::Table64(t));
                }
                Ok(v)
            };
            let in_curves = read_tables(&mut off, i, n_in)?;
            let nodes = (g as u64).checked_pow(i as u32).unwrap_or(u64::MAX);
            if nodes as u128 * o as u128 > MAX_CLUT_VALUES as u128 {
                return Err(CmsError::Invalid("CLUT too large".into()));
            }
            let total = nodes as usize * o;
            let s = r.slice(off, total * bps)?;
            let data: Vec<f64> = if wide {
                s.as_chunks::<2>().0.iter().map(|c| f64::from(u16::from_be_bytes([c[0], c[1]])) / 65535.0).collect()
            } else {
                s.iter().map(|b| f64::from(*b) / 255.0).collect()
            };
            off += total * bps;
            let out_curves = read_tables(&mut off, o, n_out)?;
            let mut stages = Vec::new();
            // The matrix only applies when the input is XYZ (BToA with an XYZ PCS).
            let ident = m.iter().enumerate().all(|(a, row)| row.iter().enumerate().all(|(b, v)| (v - if a == b { 1.0 } else { 0.0 }).abs() < 1e-6));
            if !a2b && i == 3 && !ident {
                // Only meaningful for an XYZ PCS; the caller decides (we cannot know the PCS here),
                // so keep it: for Lab PCS writers are required to store the identity.
                stages.push(Stage::matrix3(&m));
            }
            stages.push(Stage::Curves(in_curves));
            stages.push(Stage::Clut(Clut::try_new_f64(vec![g; i], o, data)?));
            stages.push(Stage::Curves(out_curves));
            Ok(Lut { kind: if wide { LutKind::Lut16 } else { LutKind::Lut8 }, inputs: i, outputs: o, stages })
        }
        b"mAB " | b"mBA " => {
            let (i, o) = (r.u8(8)? as usize, r.u8(9)? as usize);
            if !(1..=15).contains(&i) || !(1..=15).contains(&o) {
                return Err(CmsError::Invalid("bad lutAB dimensions".into()));
            }
            let (ob, om, omc, oc, oa) = (r.u32(12)? as usize, r.u32(16)? as usize, r.u32(20)? as usize, r.u32(24)? as usize, r.u32(28)? as usize);
            let matrix = |off: usize| -> Result<Stage, CmsError> {
                let mut m = vec![0.0; 9];
                for (k, v) in m.iter_mut().enumerate() {
                    *v = r.s15f16(off + k * 4)?;
                }
                let offset = vec![r.s15f16(off + 36)?, r.s15f16(off + 40)?, r.s15f16(off + 44)?];
                Ok(Stage::Matrix { rows: 3, cols: 3, m, offset })
            };
            let clut = |off: usize, cin: usize, cout: usize| -> Result<Stage, CmsError> {
                let mut grid = Vec::with_capacity(cin);
                for k in 0..cin {
                    let g = r.u8(off + k)? as usize;
                    if g < 1 {
                        return Err(CmsError::Invalid("zero grid points".into()));
                    }
                    grid.push(g);
                }
                let prec = r.u8(off + 16)? as usize;
                let nodes: u128 = grid.iter().map(|g| *g as u128).product();
                if nodes * cout as u128 > MAX_CLUT_VALUES as u128 {
                    return Err(CmsError::Invalid("CLUT too large".into()));
                }
                let total = nodes as usize * cout;
                let data: Vec<f64> = match prec {
                    1 => r.slice(off + 20, total)?.iter().map(|b| f64::from(*b) / 255.0).collect(),
                    2 => r.slice(off + 20, total * 2)?.as_chunks::<2>().0.iter().map(|c| f64::from(u16::from_be_bytes([c[0], c[1]])) / 65535.0).collect(),
                    _ => return Err(CmsError::Invalid(format!("CLUT precision {prec}"))),
                };
                Ok(Stage::Clut(Clut::try_new_f64(grid, cout, data)?))
            };
            let mut stages = Vec::new();
            if ty == b"mAB " {
                // A curves → CLUT → M curves → matrix → B curves.
                if oc != 0 {
                    if oa == 0 {
                        return Err(CmsError::Invalid("lutAtoB with CLUT but no A curves".into()));
                    }
                    stages.push(Stage::Curves(parse_curves_at(d, oa, i)?));
                    stages.push(clut(oc, i, o)?);
                } else if i != o {
                    return Err(CmsError::Invalid("lutAtoB without CLUT changes channel count".into()));
                }
                if om != 0 {
                    stages.push(Stage::Curves(parse_curves_at(d, omc, o)?));
                    stages.push(matrix(om)?);
                }
                if ob == 0 {
                    return Err(CmsError::Invalid("lutAtoB without B curves".into()));
                }
                stages.push(Stage::Curves(parse_curves_at(d, ob, o)?));
            } else {
                // B curves → matrix → M curves → CLUT → A curves.
                if ob == 0 {
                    return Err(CmsError::Invalid("lutBtoA without B curves".into()));
                }
                stages.push(Stage::Curves(parse_curves_at(d, ob, i)?));
                if om != 0 {
                    stages.push(matrix(om)?);
                    stages.push(Stage::Curves(parse_curves_at(d, omc, i)?));
                }
                if oc != 0 {
                    if oa == 0 {
                        return Err(CmsError::Invalid("lutBtoA with CLUT but no A curves".into()));
                    }
                    stages.push(clut(oc, i, o)?);
                    stages.push(Stage::Curves(parse_curves_at(d, oa, o)?));
                } else if i != o {
                    return Err(CmsError::Invalid("lutBtoA without CLUT changes channel count".into()));
                }
            }
            Ok(Lut { kind: LutKind::Ab, inputs: i, outputs: o, stages })
        }
        other => Err(CmsError::Unsupported(format!("LUT type {:?}", String::from_utf8_lossy(other)))),
    }
}

/// Text from `desc` (textDescriptionType), `mluc` (English or first record) or `text`.
fn parse_text(d: &[u8]) -> String {
    let r = Reader { b: d };
    let Ok(ty) = r.slice(0, 4) else { return String::new() };
    let ascii = |s: &[u8]| String::from_utf8_lossy(s.split(|b| *b == 0).next().unwrap_or(&[])).trim().to_string();
    match ty {
        b"desc" => {
            let n = r.u32(8).unwrap_or(0) as usize;
            r.slice(12, n).map(ascii).unwrap_or_default()
        }
        b"text" => ascii(d.get(8..).unwrap_or(&[])),
        b"mluc" => {
            let (Ok(n), Ok(size)) = (r.u32(8), r.u32(12)) else { return String::new() };
            let mut best: Option<(usize, usize)> = None;
            for k in 0..(n as usize).min(256) {
                let o = 16 + k * size as usize;
                let (Ok(lang), Ok(len), Ok(off)) = (r.u16(o), r.u32(o + 4), r.u32(o + 8)) else { break };
                if best.is_none() || lang == u16::from_be_bytes(*b"en") {
                    best = Some((off as usize, len as usize));
                    if lang == u16::from_be_bytes(*b"en") {
                        break;
                    }
                }
            }
            let Some((off, len)) = best else { return String::new() };
            let Ok(s) = r.slice(off, len) else { return String::new() };
            let units: Vec<u16> = s.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units).trim_end_matches('\0').trim().to_string()
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod retained_encoding_tests {
    use super::*;
    #[test]
    fn retained_encoding_borrows_exact_original_bytes_and_never_encodes_an_uncached_model() {
        let bytes = crate::Builtin::Srgb.profile().to_bytes();
        let parsed = Profile::parse(&bytes).unwrap();
        let retained = parsed.encoded_bytes().unwrap();
        assert_eq!(retained, bytes.as_slice());
        assert_eq!(retained.as_ptr(), parsed.bytes.as_ref().unwrap().as_ptr());
        let mut uncached = parsed.clone();
        uncached.bytes = None;
        assert!(uncached.encoded_bytes().is_none());
        assert!(uncached.bytes.is_none());
        assert_eq!(parsed.encoded_bytes(), Some(bytes.as_slice()));
    }
}

impl Profile {
 pub(crate) fn controlled_retained_bytes(&self,b:&crate::control::Budget<'_>)->crate::control::Result<usize>{
  use crate::control::Error;b.poll()?;let mut total=self.description.capacity().checked_add(self.copyright.capacity()).and_then(|n|n.checked_add(self.bytes.as_ref().map_or(0,|v|v.capacity()))).ok_or(Error::Limit("profile retention"))?;
  for lut in self.a2b.iter().chain(&self.b2a).flatten(){b.poll()?;if lut.stages.len()>4096{return Err(Error::Limit("profile stage count"));}total=total.checked_add(lut.stages.capacity().checked_mul(std::mem::size_of::<Stage>()).ok_or(Error::Limit("profile stage capacity"))?).ok_or(Error::Limit("profile retention"))?;for stage in &lut.stages{b.poll()?;total=total.checked_add(stage.controlled_bytes()?).ok_or(Error::Limit("profile retention"))?;}}
  for c in self.trc.iter().flatten().chain(self.gray_trc.iter()){b.poll()?;total=total.checked_add(c.controlled_bytes()?).ok_or(Error::Limit("profile curve retention"))?;}Ok(total)
 }
 pub(crate) fn controlled_side_cost(&self,input:bool,b:&crate::control::Budget<'_>)->crate::control::Result<(usize,u64,usize)>{
  use crate::control::Error;b.poll()?;let luts=if input{&self.a2b}else{&self.b2a};
  if let Some(lut)=Self::lut_for(luts,Intent::RelativeColorimetric){if lut.stages.len()>4096{return Err(Error::Limit("profile stage count"));}let count=lut.stages.len().checked_add(1).ok_or(Error::Limit("profile stages"))?;let mut bytes=count.checked_mul(std::mem::size_of::<Stage>()).ok_or(Error::Limit("profile stage storage"))?;let mut work=128u64;for stage in &lut.stages{b.poll()?;bytes=bytes.checked_add(stage.controlled_bytes()?).ok_or(Error::Limit("profile copied bytes"))?;work=work.checked_add(stage.controlled_work()?).ok_or(Error::Limit("profile evaluation work"))?;}return Ok((bytes,work,count));}
  let mut bytes=2*std::mem::size_of::<Stage>()+128;let mut work=256u64;
  match self.color_space{ColorSpace::Rgb=>{let curves=self.trc.as_ref().ok_or_else(||crate::control::invalid("RGB profile without TRC or LUT"))?;if self.matrix.is_none(){return Err(crate::control::invalid("RGB profile without matrix or LUT"));}bytes+=3*std::mem::size_of::<Curve>();for c in curves{b.poll()?;bytes=bytes.checked_add(c.controlled_bytes()?).ok_or(Error::Limit("profile copied curves"))?;work=work.checked_add(if input{128}else{c.controlled_work()}).ok_or(Error::Limit("profile curve work"))?;}},ColorSpace::Gray=>{let c=self.gray_trc.as_ref().ok_or_else(||crate::control::invalid("gray profile without kTRC"))?;bytes=bytes.checked_add(c.controlled_bytes()?).and_then(|n|n.checked_add(std::mem::size_of::<Curve>())).ok_or(Error::Limit("profile copied curve"))?;work=work.checked_add(if input{128}else{c.controlled_work()}).ok_or(Error::Limit("profile curve work"))?;},ColorSpace::Lab|ColorSpace::Xyz=>{},_=>return Err(crate::CmsError::Unsupported("profile without relative-colorimetric LUT".into()).into())}
  Ok((bytes,work,2))
 }
 pub(crate) fn relative_side_controlled(&self,input:bool,b:&mut crate::control::Budget<'_>)->crate::control::Result<(Vec<Stage>,Pcs)>{
  use crate::control::invalid;b.poll()?;let (_,_,count)=self.controlled_side_cost(input,b)?;let mut out=b.reserve(count)?;
  let luts=if input{&self.a2b}else{&self.b2a};if let Some(lut)=Self::lut_for(luts,Intent::RelativeColorimetric){
   if input{for stage in &lut.stages{out.push(stage.copy_controlled(b)?);}out.push(decode_pcs(self.pcs,lut.kind));}
   else{out.push(encode_pcs(self.pcs,lut.kind));for stage in &lut.stages{out.push(stage.copy_controlled(b)?);}}return Ok((out,self.pcs));
  }
  let matrix=|rows,cols,m:&[f64],offset:&[f64],b:&mut crate::control::Budget<'_>|->crate::control::Result<Stage>{Ok(Stage::Matrix{rows,cols,m:b.copy(m)?,offset:b.copy(offset)?})};
  match self.color_space{
   ColorSpace::Rgb=>{let m=self.matrix.as_ref().ok_or_else(||invalid("RGB profile without matrix"))?;let curves=self.trc.as_ref().ok_or_else(||invalid("RGB profile without TRC"))?;
    let mut copied=b.reserve(3)?;for curve in curves{copied.push(curve.copy_controlled(b)?);}
    let m=if input{*m}else{math::invert(m).ok_or_else(||invalid("singular colorant matrix"))?};let mut flat=[0f64;9];for (dst,value) in flat.iter_mut().zip(m.iter().flatten()){*dst=*value;}
    if input{out.push(Stage::Curves(copied));out.push(matrix(3,3,&flat,&[0.0;3],b)?);}else{out.push(matrix(3,3,&flat,&[0.0;3],b)?);out.push(Stage::InvCurves(copied));}Ok((out,Pcs::Xyz))
   },ColorSpace::Gray=>{let curve=self.gray_trc.as_ref().ok_or_else(||invalid("gray profile without kTRC"))?;let mut curves=b.reserve(1)?;curves.push(curve.copy_controlled(b)?);
    if input{out.push(Stage::Curves(curves));let values=if self.pcs==Pcs::Lab{[100.0,0.0,0.0]}else{math::D50};out.push(matrix(3,1,&values,&[0.0;3],b)?);}else{let values=if self.pcs==Pcs::Lab{[0.01,0.0,0.0]}else{[0.0,1.0,0.0]};out.push(matrix(1,3,&values,&[0.0],b)?);out.push(Stage::InvCurves(curves));}Ok((out,self.pcs))
   },ColorSpace::Lab=>{out.push(if input{Stage::DecodeLabV4}else{Stage::EncodeLabV4});Ok((out,Pcs::Lab))},ColorSpace::Xyz=>{out.push(if input{Stage::DecodeXyz}else{Stage::EncodeXyz});Ok((out,Pcs::Xyz))},_=>Err(crate::CmsError::Unsupported("profile without relative-colorimetric LUT".into()).into())
  }
 }
}

/// Public model accounting; cached encoded ICC bytes remain shared, not copied.
#[derive(Clone,Copy,Debug)]
pub struct ProfileCloneRequirements {pub retained_bytes:usize,pub cloned_model_bytes:usize,pub work_units:u64}
impl Profile {
 pub fn clone_requirements_with_control(&self,limits:crate::control::Limits,cancelled:&dyn Fn()->bool)->crate::control::Result<ProfileCloneRequirements>{
  use crate::control::{Budget,Error};let b=Budget::new(limits,cancelled)?;let retained=self.controlled_retained_bytes(&b)?;
  let cached=self.bytes.as_ref().map_or(0,|v|v.capacity());let cloned=retained.checked_sub(cached).ok_or(Error::Limit("profile clone accounting"))?;
  b.admit(retained.checked_add(cloned).ok_or(Error::Limit("profile clone peak"))?,cloned as u64)?;
  Ok(ProfileCloneRequirements{retained_bytes:retained,cloned_model_bytes:cloned,work_units:cloned as u64})
 }
 /// Fallible cooperative public-model clone. No parsing/encoding/CMS policy is
 /// introduced; retained original ICC Arc and cached hash are preserved.
 pub fn try_clone_with_control(&self,limits:crate::control::Limits,cancelled:&dyn Fn()->bool)->crate::control::Result<Self>{
  use crate::control::Budget;let r=self.clone_requirements_with_control(limits,cancelled)?;let mut b=Budget::new(limits,cancelled)?;b.retained(r.retained_bytes)?;
  let string=|v:&str,b:&mut Budget<'_>|->crate::control::Result<String>{String::from_utf8(b.copy(v.as_bytes())?).map_err(|_|crate::control::invalid("profile string encoding"))};
  let mut trc=None;if let Some(curves)=&self.trc{trc=Some([curves[0].copy_controlled(&mut b)?,curves[1].copy_controlled(&mut b)?,curves[2].copy_controlled(&mut b)?]);}
  let gray_trc=self.gray_trc.as_ref().map(|c|c.copy_controlled(&mut b)).transpose()?;
  let lut=|value:&Option<Lut>,b:&mut Budget<'_>|->crate::control::Result<Option<Lut>>{match value{None=>Ok(None),Some(l)=>{let mut stages=b.reserve(l.stages.len())?;for stage in &l.stages{b.poll()?;stages.push(stage.copy_controlled(b)?);}Ok(Some(Lut{kind:l.kind,inputs:l.inputs,outputs:l.outputs,stages}))}}};
  let a2b=[lut(&self.a2b[0],&mut b)?,lut(&self.a2b[1],&mut b)?,lut(&self.a2b[2],&mut b)?];let b2a=[lut(&self.b2a[0],&mut b)?,lut(&self.b2a[1],&mut b)?,lut(&self.b2a[2],&mut b)?];
  let result=Self{version:self.version,class:self.class,color_space:self.color_space,pcs:self.pcs,rendering_intent:self.rendering_intent,description:string(&self.description,&mut b)?,copyright:string(&self.copyright,&mut b)?,white_point:self.white_point,black_point:self.black_point,chad:self.chad,matrix:self.matrix,trc,gray_trc,a2b,b2a,bytes:self.bytes.as_ref().map(std::sync::Arc::clone),hash:self.hash.clone()};
  b.poll()?;Ok(result)
 }
}

/// Source public-model retention and exact final Gray-as-RGB allocations.
/// Discarded LUTs/cached ICC bytes remain source-owned and are never copied.
#[derive(Clone,Copy,Debug)]
pub struct GrayAsRgbRequirements {pub retained_bytes:usize,pub staged_bytes:usize,pub work_units:u64}
impl Profile {
 pub fn gray_as_rgb_requirements_with_control(&self,limits:crate::control::Limits,cancelled:&dyn Fn()->bool)->crate::control::Result<Option<GrayAsRgbRequirements>>{
  use crate::control::{Budget,Error};let b=Budget::new(limits,cancelled)?;
  let Some(curve)=&self.gray_trc else{return Ok(None);};if self.pcs!=Pcs::Xyz{return Ok(None);}
  let retained=self.controlled_retained_bytes(&b)?;
  let text=self.description.len().checked_add(" (as RGB)".len()).and_then(|n|n.checked_add(self.copyright.len())).ok_or(Error::Limit("gray-as-RGB description length"))?;
  let staged=text.checked_add(curve.controlled_bytes()?.checked_mul(3).ok_or(Error::Limit("gray-as-RGB curves"))?).ok_or(Error::Limit("gray-as-RGB model"))?;
  let work=staged as u64;b.admit(retained.checked_add(staged).ok_or(Error::Limit("gray-as-RGB peak"))?,work)?;
  Ok(Some(GrayAsRgbRequirements{retained_bytes:retained,staged_bytes:staged,work_units:work}))
 }
 /// Controlled equivalent of gray_as_rgb for an already-resolved profile.
 /// Preserves its legacy interpretation exactly; performs no parse/encoding.
 pub fn gray_as_rgb_with_control(&self,limits:crate::control::Limits,cancelled:&dyn Fn()->bool)->crate::control::Result<Option<Self>>{
  use crate::control::{Budget,Error};let Some(r)=self.gray_as_rgb_requirements_with_control(limits,cancelled)? else{return Ok(None);};
  let mut b=Budget::new(limits,cancelled)?;b.retained(r.retained_bytes)?;
  let curve=self.gray_trc.as_ref().ok_or_else(||crate::control::invalid("gray-as-RGB curve disappeared"))?;
  let suffix=" (as RGB)";let count=self.description.len().checked_add(suffix.len()).ok_or(Error::Limit("gray-as-RGB description length"))?;
  let mut text=b.reserve(count)?;for part in self.description.as_bytes().chunks(1024){b.work(part.len() as u64)?;text.extend_from_slice(part);}b.work(suffix.len() as u64)?;text.extend_from_slice(suffix.as_bytes());
  let description=String::from_utf8(text).map_err(|_|crate::control::invalid("gray-as-RGB description encoding"))?;
  let copyright=String::from_utf8(b.copy(self.copyright.as_bytes())?).map_err(|_|crate::control::invalid("gray-as-RGB copyright encoding"))?;
  let trc=Some([curve.copy_controlled(&mut b)?,curve.copy_controlled(&mut b)?,curve.copy_controlled(&mut b)?]);
  let w=math::D50;let matrix=Some([[w[0]/3.0;3],[w[1]/3.0;3],[w[2]/3.0;3]]);
  let result=Self{version:self.version,class:self.class,color_space:ColorSpace::Rgb,pcs:self.pcs,rendering_intent:self.rendering_intent,description,copyright,white_point:self.white_point,black_point:self.black_point,chad:self.chad,matrix,trc,gray_trc:None,a2b:Default::default(),b2a:Default::default(),bytes:None,hash:Default::default()};
  b.poll()?;Ok(Some(result))
 }
}
#[cfg(test)]
mod controlled_gray_rgb_tests {
 use super::*;
 #[test]
 fn resolved_gray_rgb_matches_legacy_model_and_uncached_encoding_state(){
  for builtin in [crate::Builtin::SGray,crate::Builtin::GrayGamma22,crate::Builtin::DotGain20Compat]{
   let source=builtin.profile();let before=source.clone();let expected=source.gray_as_rgb().unwrap();
   let controlled=source.gray_as_rgb_with_control(crate::control::Limits::default(),&||false).unwrap().unwrap();
   assert_eq!(controlled,expected);assert!(controlled.encoded_bytes().is_none());assert!(controlled.hash.get().is_none());assert_eq!(source,&before);
  }
 }
 #[test]
 fn exact_table_curve_and_cached_source_identity_remain_source_owned(){
  let mut source=crate::Builtin::SGray.profile().clone();source.description="Gray λ".into();source.gray_trc=Some(Curve::Table64(vec![0.0,0.5+2f64.powi(-45),1.0]));let _=source.content_hash();let before=source.clone();
  let expected=source.gray_as_rgb().unwrap();let controlled=source.gray_as_rgb_with_control(crate::control::Limits::default(),&||false).unwrap().unwrap();
  assert_eq!(controlled,expected);assert_eq!(controlled.description,"Gray λ (as RGB)");assert!(controlled.bytes.is_none()&&controlled.hash.get().is_none());assert_eq!(source,before);
 }
 #[test]
 fn unsupported_cancel_and_tiny_budget_are_explicit_without_source_mutation(){
  let source=crate::Builtin::SGray.profile();let before=source.clone();
  assert!(matches!(source.gray_as_rgb_with_control(crate::control::Limits::default(),&||true),Err(crate::control::Error::Cancelled)));
  assert!(matches!(source.gray_as_rgb_with_control(crate::control::Limits{resident_bytes:1,work_units:1},&||false),Err(crate::control::Error::Limit(_))));
  assert!(crate::Builtin::LabD50.profile().gray_as_rgb_with_control(crate::control::Limits::default(),&||false).unwrap().is_none());
  let mut lab=source.clone();lab.pcs=Pcs::Lab;assert_eq!(lab.gray_as_rgb(),None);assert_eq!(lab.gray_as_rgb_with_control(crate::control::Limits::default(),&||false).unwrap(),None);assert_eq!(source,&before);
 }
}

impl Profile {
    /// Borrowed encoding admission, including source model and generated buffers.
    pub fn encode_requirements_with_control(&self,limits:crate::control::Limits,cancelled:&dyn Fn()->bool)->crate::control::Result<EncodeRequirements> {
        crate::write::requirements_with_control(self,limits,cancelled)
    }
    /// Encode exactly the legacy ICC model recipe with controlled admission,
    /// replacing cached bytes and resetting the hash only after success.
    pub fn with_encoded_bytes_with_control(mut self,limits:crate::control::Limits,cancelled:&dyn Fn()->bool)->crate::control::Result<Self> {
        let bytes=crate::write::encode_with_control(&self,limits,cancelled)?;
        if cancelled(){return Err(crate::control::Error::Cancelled);}
        self.bytes=Some(std::sync::Arc::new(bytes));
        self.hash=Default::default();
        Ok(self)
    }
    /// Cached bytes are shared exactly; uncached models use the controlled writer.
    /// Fixed-size Arc wrapper allocation remains standard-library managed.
    pub fn to_bytes_with_control(&self,limits:crate::control::Limits,cancelled:&dyn Fn()->bool)->crate::control::Result<std::sync::Arc<Vec<u8>>> {
        let budget=crate::control::Budget::new(limits,cancelled)?;
        if let Some(bytes)=&self.bytes {budget.admit(bytes.capacity(),0)?;budget.poll()?;return Ok(std::sync::Arc::clone(bytes));}
        let bytes=crate::write::encode_with_control(self,limits,cancelled)?;
        budget.poll()?;Ok(std::sync::Arc::new(bytes))
    }
}
