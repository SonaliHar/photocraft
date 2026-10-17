//! Colour reduction for Image › Mode › Indexed Color and Bitmap: palette construction (exact,
//! system, web, uniform, perceptual, selective, adaptive), forced colours, and dithering
//! (none, Floyd–Steinberg diffusion, ordered pattern, noise), plus 1-bit conversion methods.
//!
//! Buffers are straight RGBA in 0..=1, row-major. Median cut follows Heckbert (1982); k-means
//! refinement is Lloyd's algorithm; error diffusion is Floyd & Steinberg (1976).

use photocraft_color::convert::srgb_to_lab;
use serde::{Deserialize, Serialize};

mod nearest;
use nearest::Nearest;

pub type Rgb8 = [u8; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaletteKind {
    /// Every colour of the image (fails above 256).
    Exact,
    SystemMac,
    SystemWindows,
    Web,
    Uniform,
    /// Lab median cut + k-means (perceptually weighted).
    Perceptual,
    /// Perceptual, with entries near web-safe colours snapped to them.
    Selective,
    /// RGB median cut weighted by frequency.
    Adaptive,
}

impl PaletteKind {
    pub fn from_id(s: &str) -> Option<Self> {
        Some(match s {
            "exact" => PaletteKind::Exact,
            "system" | "systemMac" | "systemMacOs" => PaletteKind::SystemMac,
            "systemWindows" => PaletteKind::SystemWindows,
            "web" => PaletteKind::Web,
            "uniform" => PaletteKind::Uniform,
            "perceptual" | "localPerceptual" => PaletteKind::Perceptual,
            "selective" | "localSelective" => PaletteKind::Selective,
            "adaptive" | "localAdaptive" => PaletteKind::Adaptive,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Forced {
    None,
    BlackWhite,
    Primaries,
    Web,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dither {
    None,
    Diffusion,
    Pattern,
    Noise,
}

fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn rgb8(p: &[f32; 4]) -> Rgb8 {
    [to8(p[0]), to8(p[1]), to8(p[2])]
}

/// The 216-colour web-safe cube (0, 51, …, 255 per channel).
pub fn web_palette() -> Vec<Rgb8> {
    let mut v = Vec::with_capacity(216);
    for r in 0..6u8 {
        for g in 0..6u8 {
            for b in 0..6u8 {
                v.push([r * 51, g * 51, b * 51]);
            }
        }
    }
    v
}

/// Classic Mac OS system palette: the web cube plus 10-step red, green, blue and gray ramps.
pub fn mac_palette() -> Vec<Rgb8> {
    let mut v = web_palette();
    // Ramps use the values between cube steps (17-unit spacing, skipping multiples of 51).
    let steps: Vec<u8> = (1..=15u8).map(|k| k * 17).filter(|x| x % 51 != 0).collect();
    for &s in steps.iter().rev() {
        v.push([s, 0, 0]);
    }
    for &s in steps.iter().rev() {
        v.push([0, s, 0]);
    }
    for &s in steps.iter().rev() {
        v.push([0, 0, s]);
    }
    for &s in steps.iter().rev() {
        v.push([s, s, s]);
    }
    v.truncate(256);
    v
}

/// Windows default palette: the 20 static system colours plus the web cube and a gray ramp.
pub fn windows_palette() -> Vec<Rgb8> {
    let mut v: Vec<Rgb8> = vec![
        [0, 0, 0],
        [128, 0, 0],
        [0, 128, 0],
        [128, 128, 0],
        [0, 0, 128],
        [128, 0, 128],
        [0, 128, 128],
        [192, 192, 192],
        [192, 220, 192],
        [166, 202, 240],
        [255, 251, 240],
        [160, 160, 164],
        [128, 128, 128],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [0, 0, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    for c in web_palette() {
        if !v.contains(&c) {
            v.push(c);
        }
    }
    let mut g = 8u8;
    while v.len() < 256 {
        if !v.contains(&[g, g, g]) {
            v.push([g, g, g]);
        }
        g = g.wrapping_add(12);
    }
    v
}

/// Up to `n` colours on an equal-step RGB grid.
pub fn uniform_palette(n: usize) -> Vec<Rgb8> {
    let k = ((n as f32).cbrt().floor() as usize).max(2);
    let lv = |i: usize| ((i as f32 / (k - 1) as f32) * 255.0).round() as u8;
    let mut v = Vec::with_capacity(k * k * k);
    for r in 0..k {
        for g in 0..k {
            for b in 0..k {
                v.push([lv(r), lv(g), lv(b)]);
            }
        }
    }
    v
}

fn forced_colors(f: Forced) -> Vec<Rgb8> {
    match f {
        Forced::None => Vec::new(),
        Forced::BlackWhite => vec![[0, 0, 0], [255, 255, 255]],
        Forced::Primaries => vec![[0, 0, 0], [255, 255, 255], [255, 0, 0], [0, 255, 0], [0, 0, 255], [0, 255, 255], [255, 0, 255], [255, 255, 0]],
        Forced::Web => web_palette(),
    }
}

/// Unique 8-bit colours with counts (opaque pixels only).
fn histogram(px: &[[f32; 4]]) -> Vec<(Rgb8, u32)> {
    let mut m: std::collections::HashMap<Rgb8, u32> = std::collections::HashMap::new();
    for p in px.iter().filter(|p| p[3] >= 0.5) {
        *m.entry(rgb8(p)).or_default() += 1;
    }
    let mut v: Vec<(Rgb8, u32)> = m.into_iter().collect();
    v.sort_unstable();
    v
}

/// Median cut over `pts` (feature vectors + weights) into at most `n` boxes; returns box means.
fn median_cut(pts: &[([f32; 3], u32)], n: usize) -> Vec<[f32; 3]> {
    if pts.is_empty() || n == 0 {
        return Vec::new();
    }
    let mut boxes: Vec<Vec<([f32; 3], u32)>> = vec![pts.to_vec()];
    while boxes.len() < n {
        // Split the box with the largest weighted extent.
        let (bi, axis, score) = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .map(|(i, b)| {
                let mut best = (0usize, -1.0f32);
                for a in 0..3 {
                    let (lo, hi) = b.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.0[a]), hi.max(p.0[a])));
                    if hi - lo > best.1 {
                        best = (a, hi - lo);
                    }
                }
                let w: f32 = b.iter().map(|p| p.1 as f32).sum();
                (i, best.0, best.1 * w.sqrt())
            })
            .fold((usize::MAX, 0, -1.0f32), |acc, x| if x.2 > acc.2 { x } else { acc });
        if bi == usize::MAX || score <= 0.0 {
            break;
        }
        let mut b = boxes.swap_remove(bi);
        b.sort_by(|p, q| p.0[axis].total_cmp(&q.0[axis]));
        let total: u64 = b.iter().map(|p| u64::from(p.1)).sum();
        let mut acc = 0u64;
        let mut cut = 1;
        for (i, p) in b.iter().enumerate() {
            acc += u64::from(p.1);
            if acc * 2 >= total {
                cut = (i + 1).clamp(1, b.len() - 1);
                break;
            }
        }
        let rest = b.split_off(cut);
        boxes.push(b);
        boxes.push(rest);
    }
    boxes
        .iter()
        .map(|b| {
            let w: f64 = b.iter().map(|p| f64::from(p.1)).sum();
            std::array::from_fn(|a| (b.iter().map(|p| f64::from(p.0[a]) * f64::from(p.1)).sum::<f64>() / w) as f32)
        })
        .collect()
}

/// Lloyd refinement of `centers` over weighted points.
fn kmeans(pts: &[([f32; 3], u32)], centers: &mut [[f32; 3]], iters: usize) {
    if centers.is_empty() {
        // No room left (the forced colours fill the palette): nothing to refine.
        return;
    }
    for _ in 0..iters {
        let lookup = Nearest::new(centers.iter().copied());
        let mut sum = vec![[0.0f64; 3]; centers.len()];
        let mut cnt = vec![0.0f64; centers.len()];
        for (p, w) in pts {
            let i = lookup.find(*p);
            for a in 0..3 {
                sum[i][a] += f64::from(p[a]) * f64::from(*w);
            }
            cnt[i] += f64::from(*w);
        }
        for (i, c) in centers.iter_mut().enumerate() {
            if cnt[i] > 0.0 {
                *c = std::array::from_fn(|a| (sum[i][a] / cnt[i]) as f32);
            }
        }
    }
}

fn lab_to_rgb8(lab: [f32; 3]) -> Rgb8 {
    photocraft_color::convert::lab_to_srgb(lab).map(to8)
}

/// Builds the palette for `px`. `colors` is the requested count (2..=256) for the computed
/// kinds; forced colours are included first and count toward it.
pub fn build_palette(px: &[[f32; 4]], kind: PaletteKind, colors: usize, forced: Forced) -> Result<Vec<Rgb8>, String> {
    let colors = colors.clamp(2, 256);
    let mut pal = forced_colors(forced);
    let room = colors.saturating_sub(pal.len());
    // Fixed palettes, or computed palettes already filled by forced colours, need no scan.
    // Exact still counts every colour to enforce its 256-colour limit.
    let hist = if kind == PaletteKind::Exact || (room > 0 && matches!(kind, PaletteKind::Adaptive | PaletteKind::Perceptual | PaletteKind::Selective)) {
        histogram(px)
    } else {
        Vec::new()
    };
    let computed: Vec<Rgb8> = match kind {
        PaletteKind::Exact => {
            if hist.len() > 256 {
                return Err(format!("the image has {} colours; Exact needs 256 or fewer", hist.len()));
            }
            hist.iter().map(|h| h.0).collect()
        }
        PaletteKind::SystemMac => mac_palette(),
        PaletteKind::SystemWindows => windows_palette(),
        PaletteKind::Web => web_palette(),
        PaletteKind::Uniform => uniform_palette(room.max(8)),
        PaletteKind::Adaptive => {
            if hist.len() <= room {
                hist.iter().map(|h| h.0).collect()
            } else {
                let pts: Vec<([f32; 3], u32)> = hist.iter().map(|(c, n)| (c.map(f32::from), *n)).collect();
                let mut centers = median_cut(&pts, room);
                kmeans(&pts, &mut centers, 2);
                centers.iter().map(|c| c.map(|v| v.round().clamp(0.0, 255.0) as u8)).collect()
            }
        }
        PaletteKind::Perceptual | PaletteKind::Selective => {
            if hist.len() <= room {
                hist.iter().map(|h| h.0).collect()
            } else {
                let pts: Vec<([f32; 3], u32)> = hist.iter().map(|(c, n)| (srgb_to_lab(c.map(|v| f32::from(v) / 255.0)), *n)).collect();
                let mut centers = median_cut(&pts, room);
                kmeans(&pts, &mut centers, 4);
                let mut v: Vec<Rgb8> = centers.iter().map(|c| lab_to_rgb8(*c)).collect();
                if kind == PaletteKind::Selective {
                    // Favour web-safe colours: snap entries within ~10 levels of one.
                    for c in &mut v {
                        let snap = c.map(|x| ((f32::from(x) / 51.0).round() * 51.0) as u8);
                        if c.iter().zip(snap).all(|(a, b)| a.abs_diff(b) <= 10) {
                            *c = snap;
                        }
                    }
                }
                v
            }
        }
    };
    for c in computed {
        if pal.len() >= 256 {
            break;
        }
        if !pal.contains(&c) {
            pal.push(c);
        }
    }
    if matches!(kind, PaletteKind::Adaptive | PaletteKind::Perceptual | PaletteKind::Selective | PaletteKind::Uniform) {
        pal.truncate(colors.max(forced_colors(forced).len()));
    }
    if pal.is_empty() {
        pal.push([0, 0, 0]);
    }
    Ok(pal)
}

/// 8×8 Bayer threshold in -0.5..0.5.
pub fn bayer8(x: usize, y: usize) -> f32 {
    const M: [u8; 64] = [
        0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46, 6, 38, 60, 28, 52, 20, 62, 30, 54, 22, 3, 35, 11, 43, 1, 33, 9, 41,
        51, 19, 59, 27, 49, 17, 57, 25, 15, 47, 7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21,
    ];
    (f32::from(M[(y % 8) * 8 + x % 8]) + 0.5) / 64.0 - 0.5
}

fn hash01(x: usize, y: usize, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0
}

/// Maps every opaque pixel to its palette entry (in place) with the chosen dither; returns the
/// index per pixel. Pixels with alpha < ½ become index `transparent` (and alpha 0) when given;
/// otherwise alpha is kept. `amount` (0..=1) scales diffusion / pattern / noise strength.
pub fn quantize(px: &mut [[f32; 4]], w: usize, pal: &[Rgb8], dither: Dither, amount: f32, transparent: Option<usize>) -> Vec<u8> {
    let lookup = Nearest::new(pal.iter().map(|c| c.map(f32::from)));
    quantize_with(px, w, pal, dither, amount, transparent, |c| lookup.find(c.map(|v| v * 255.0)))
}

fn quantize_with(
    px: &mut [[f32; 4]],
    w: usize,
    pal: &[Rgb8],
    dither: Dither,
    amount: f32,
    transparent: Option<usize>,
    nearest: impl Fn([f32; 3]) -> usize + Sync,
) -> Vec<u8> {
    let h = px.len().checked_div(w).unwrap_or(0);
    let mut idx = vec![0u8; px.len()];
    // Typical palette spacing sets the ordered/noise dither amplitude.
    let spread = (1.0 / (pal.len() as f32).cbrt().max(1.0)).min(0.5) * amount.clamp(0.0, 1.0);
    // None / Pattern / Noise dither have no cross-pixel dependency: map every pixel in parallel.
    // (Diffusion propagates error between pixels, so it stays serial below.)
    if dither != Dither::Diffusion {
        crate::photo_util::par_rows2(&mut idx, px, w, |y, idxrow, pxrow| {
            for (x, (ip, p)) in idxrow.iter_mut().zip(pxrow.iter_mut()).enumerate() {
                let pv = *p;
                if pv[3] < 0.5 {
                    if let Some(t) = transparent {
                        *ip = t as u8;
                        *p = [0.0, 0.0, 0.0, 0.0];
                        continue;
                    }
                    if pv[3] <= 0.0 {
                        continue;
                    }
                }
                let mut c = [pv[0], pv[1], pv[2]];
                match dither {
                    Dither::Pattern => c = c.map(|v| v + bayer8(x, y) * spread),
                    Dither::Noise => c = c.map(|v| v + (hash01(x, y, 7) - 0.5) * spread),
                    _ => {}
                }
                let c = c.map(|v| v.clamp(0.0, 1.0));
                let k = nearest(c);
                *ip = k as u8;
                let e = pal[k].map(|v| f32::from(v) / 255.0);
                *p = [e[0], e[1], e[2], if transparent.is_some() { 1.0 } else { pv[3] }];
            }
        });
        return idx;
    }
    let mut err = vec![[0.0f32; 3]; 2 * (w + 2)];
    for y in 0..h {
        if dither == Dither::Diffusion {
            // Rows ping-pong: `cur` is row y, `next` row y + 1 (offset by one for x = -1).
            let (cur, next) = err.split_at_mut(w + 2);
            cur.copy_from_slice(next);
            next.iter_mut().for_each(|e| *e = [0.0; 3]);
        }
        for x in 0..w {
            let i = y * w + x;
            let p = px[i];
            if p[3] < 0.5 {
                if let Some(t) = transparent {
                    idx[i] = t as u8;
                    px[i] = [0.0, 0.0, 0.0, 0.0];
                    continue;
                }
                if p[3] <= 0.0 {
                    continue;
                }
            }
            let mut c = [p[0], p[1], p[2]];
            match dither {
                Dither::None => {}
                Dither::Diffusion => {
                    for k in 0..3 {
                        c[k] += err[x + 1][k];
                    }
                }
                Dither::Pattern => {
                    let t = bayer8(x, y) * spread;
                    c = c.map(|v| v + t);
                }
                Dither::Noise => {
                    let t = (hash01(x, y, 7) - 0.5) * spread;
                    c = c.map(|v| v + t);
                }
            }
            let c = c.map(|v| v.clamp(0.0, 1.0));
            let k = nearest(c);
            idx[i] = k as u8;
            let e = pal[k].map(|v| f32::from(v) / 255.0);
            if dither == Dither::Diffusion {
                let a = amount.clamp(0.0, 1.0);
                let d: [f32; 3] = std::array::from_fn(|q| (c[q] - e[q]) * a);
                let (cur, next) = err.split_at_mut(w + 2);
                for q in 0..3 {
                    cur[x + 2][q] += d[q] * 7.0 / 16.0;
                    next[x][q] += d[q] * 3.0 / 16.0;
                    next[x + 1][q] += d[q] * 5.0 / 16.0;
                    next[x + 2][q] += d[q] / 16.0;
                }
            }
            px[i] = [e[0], e[1], e[2], if transparent.is_some() { 1.0 } else { p[3] }];
        }
    }
    idx
}

/// Image › Mode › Bitmap methods.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum BitmapMethod {
    Threshold,
    Pattern,
    Diffusion,
    /// Halftone screen: lines per pixel-cell size, angle in degrees, dot shape.
    Halftone {
        cell: f32,
        angle: f32,
        shape: HalftoneShape,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HalftoneShape {
    Round,
    Ellipse,
    Line,
    Square,
    Diamond,
    Cross,
}

impl HalftoneShape {
    pub fn from_id(s: &str) -> Option<Self> {
        Some(match s {
            "round" => HalftoneShape::Round,
            "ellipse" => HalftoneShape::Ellipse,
            "line" => HalftoneShape::Line,
            "square" => HalftoneShape::Square,
            "diamond" => HalftoneShape::Diamond,
            "cross" => HalftoneShape::Cross,
            _ => return None,
        })
    }
}

/// Spot function in 0..=1 for a position (u, v) in -1..1 within a halftone cell.
fn spot(shape: HalftoneShape, u: f32, v: f32) -> f32 {
    let s = match shape {
        HalftoneShape::Round => 1.0 - (u * u + v * v) / 2.0,
        HalftoneShape::Ellipse => 1.0 - (u * u + 0.6 * v * v) / 1.6,
        HalftoneShape::Line => 1.0 - v.abs(),
        HalftoneShape::Square => 1.0 - u.abs().max(v.abs()),
        HalftoneShape::Diamond => 1.0 - (u.abs() + v.abs()) / 2.0,
        HalftoneShape::Cross => 1.0 - u.abs().min(v.abs()),
    };
    s.clamp(0.0, 1.0)
}

/// Converts gray values (0..=1, row-major `w` wide) to 0/1 in place.
pub fn to_bitmap(gray: &mut [f32], w: usize, method: BitmapMethod) {
    let h = gray.len().checked_div(w).unwrap_or(0);
    match method {
        BitmapMethod::Threshold => gray.iter_mut().for_each(|v| *v = if *v >= 0.5 { 1.0 } else { 0.0 }),
        BitmapMethod::Pattern => {
            for (i, v) in gray.iter_mut().enumerate() {
                *v = if *v > bayer8(i % w, i / w) + 0.5 { 1.0 } else { 0.0 };
            }
        }
        BitmapMethod::Diffusion => {
            let mut buf: Vec<f32> = gray.to_vec();
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    let old = buf[i];
                    let new = if old >= 0.5 { 1.0 } else { 0.0 };
                    let e = old - new;
                    gray[i] = new;
                    if x + 1 < w {
                        buf[i + 1] += e * 7.0 / 16.0;
                    }
                    if y + 1 < h {
                        if x > 0 {
                            buf[i + w - 1] += e * 3.0 / 16.0;
                        }
                        buf[i + w] += e * 5.0 / 16.0;
                        if x + 1 < w {
                            buf[i + w + 1] += e / 16.0;
                        }
                    }
                }
            }
        }
        BitmapMethod::Halftone { cell, angle, shape } => {
            let cell = cell.max(2.0);
            let (s, c) = angle.to_radians().sin_cos();
            // Rank the spot function over the cell so coverage is proportional to the tone.
            let mut table: Vec<f32> = (0..32 * 32).map(|k| spot(shape, ((k % 32) as f32 + 0.5) / 16.0 - 1.0, ((k / 32) as f32 + 0.5) / 16.0 - 1.0)).collect();
            table.sort_by(f32::total_cmp);
            let cdf = |x: f32| table.partition_point(|t| *t < x) as f32 / table.len() as f32;
            for (i, v) in gray.iter_mut().enumerate() {
                let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                let (rx, ry) = (x * c + y * s, -x * s + y * c);
                let u = (rx / cell).rem_euclid(1.0) * 2.0 - 1.0;
                let vv = (ry / cell).rem_euclid(1.0) * 2.0 - 1.0;
                // Ink (black) covers where the spot function exceeds the gray level.
                *v = if cdf(spot(shape, u, vv)) >= *v { 0.0 } else { 1.0 };
            }
        }
    }
}

/// Lower-only resource limits for the ordinary finite F32 quantizer. They do
/// not qualify wide/native storage, profiles, or a source operation schema.
#[derive(Clone, Copy, Debug)]
pub struct ControlledLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for ControlledLimits {
    fn default() -> Self {
        Self { resident_bytes: photocraft_raster::MAX_CHECKED_RASTER_BYTES, work_units: 8_000_000_000 }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ControlledRequirements {
    pub retained_bytes: u64,
    pub scratch_bytes: u64,
    pub output_bytes: u64,
    pub work_units: u64,
}
type ControlledResult<T> = Result<T, photocraft_raster::RasterInputError>;
fn controlled_poll(control: &photocraft_raster::Interrupt<'_>) -> ControlledResult<()> {
    control.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)
}
fn controlled_limit() -> photocraft_raster::RasterInputError {
    photocraft_raster::RasterInputError::LimitExceeded
}
fn controlled_invalid() -> photocraft_raster::RasterInputError {
    photocraft_raster::RasterInputError::InvalidBounds
}
fn controlled_add(a: u64, b: u64) -> ControlledResult<u64> {
    a.checked_add(b).ok_or_else(controlled_limit)
}
fn controlled_mul(a: u64, b: u64) -> ControlledResult<u64> {
    a.checked_mul(b).ok_or_else(controlled_limit)
}
fn controlled_admit(requirements: ControlledRequirements, limits: ControlledLimits) -> ControlledResult<()> {
    let standard = ControlledLimits::default();
    if limits.resident_bytes == 0 || limits.work_units == 0 || limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units {
        return Err(controlled_limit());
    }
    let peak = controlled_add(controlled_add(requirements.retained_bytes, requirements.scratch_bytes)?, requirements.output_bytes)?;
    if peak > limits.resident_bytes || requirements.work_units > limits.work_units {
        return Err(controlled_limit());
    }
    Ok(())
}
fn controlled_vec<T>(count: usize, ceiling: u64) -> ControlledResult<Vec<T>> {
    let bytes = count.checked_mul(std::mem::size_of::<T>()).ok_or_else(controlled_limit)?;
    if bytes as u64 > ceiling {
        return Err(controlled_limit());
    }
    let mut result = Vec::new();
    result.try_reserve_exact(count).map_err(|_| controlled_limit())?;
    if (result.capacity() as u64).checked_mul(std::mem::size_of::<T>() as u64).is_none_or(|n| n > ceiling) {
        return Err(controlled_limit());
    }
    Ok(result)
}
fn controlled_finite(px: &[[f32; 4]], control: &photocraft_raster::Interrupt<'_>) -> ControlledResult<()> {
    for chunk in px.chunks(4096) {
        controlled_poll(control)?;
        if chunk.iter().flatten().any(|value| !value.is_finite()) {
            return Err(controlled_invalid());
        }
    }
    controlled_poll(control)
}
/// Allocation-free maximum envelope, including the caller's input and the
/// ordinal result. Finite HDR RGB/alpha retain the legacy clamping decisions.
pub fn quantize_requirements(pixels: usize, width: usize, palette: usize, dither: Dither) -> ControlledResult<ControlledRequirements> {
    if width == 0 || !pixels.is_multiple_of(width) || palette == 0 || palette > 256 {
        return Err(controlled_invalid());
    }
    let count = u64::try_from(pixels).map_err(|_| controlled_limit())?;
    let scratch = if dither == Dither::Diffusion {
        controlled_mul(u64::try_from(width.checked_add(2).ok_or_else(controlled_limit)?).map_err(|_| controlled_limit())?, 24)?
    } else {
        0
    };
    let lookup = Nearest::checked_bytes(palette).ok_or_else(controlled_limit)? as u64;
    let scratch = controlled_add(scratch, lookup)?;
    Ok(ControlledRequirements {
        retained_bytes: controlled_add(controlled_mul(count, 16)?, controlled_mul(palette as u64, 3)?)?,
        output_bytes: count,
        scratch_bytes: scratch,
        work_units: controlled_add(
            controlled_mul(count, controlled_add(controlled_mul(palette as u64, 8)?, 64)?)?,
            controlled_mul(controlled_mul(palette as u64, palette as u64)?, 4)?,
        )?,
    })
}
/// The input belongs to a caller-private stage: cancellation may leave a mapped
/// prefix in `px`, but returns no ordinal result. No public document is mutated.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn quantize_with_interrupt(
    px: &mut [[f32; 4]],
    width: usize,
    palette: &[Rgb8],
    dither: Dither,
    amount: f32,
    transparent: Option<usize>,
    limits: ControlledLimits,
    control: &photocraft_raster::Interrupt<'_>,
) -> ControlledResult<Vec<u8>> {
    controlled_poll(control)?;
    let required = quantize_requirements(px.len(), width, palette.len(), dither)?;
    controlled_admit(required, limits)?;
    if !amount.is_finite() || transparent.is_some_and(|index| index >= palette.len()) {
        return Err(controlled_invalid());
    }
    controlled_finite(px, control)?;
    let lookup = Nearest::try_new(palette.iter().map(|color| color.map(f32::from)), control)?;
    let lookup_bytes = Nearest::checked_bytes(palette.len()).ok_or_else(controlled_limit)? as u64;
    let mut indices = controlled_vec(px.len(), required.output_bytes)?;
    while indices.len() < px.len() {
        controlled_poll(control)?;
        indices.resize(indices.len().saturating_add(64 * 1024).min(px.len()), 0u8);
    }
    let spread = (1.0 / (palette.len() as f32).cbrt().max(1.0)).min(0.5) * amount.clamp(0.0, 1.0);
    if dither != Dither::Diffusion {
        // Independent rows preserve the exact former per-row arithmetic and
        // coordinate hashing; scheduling order never affects their results.
        for (y, (index_row, pixel_row)) in indices.chunks_exact_mut(width).zip(px.chunks_exact_mut(width)).enumerate() {
            controlled_poll(control)?;
            for (x, (index, pixel)) in index_row.iter_mut().zip(pixel_row.iter_mut()).enumerate() {
                if x.is_multiple_of(256) {
                    controlled_poll(control)?;
                }
                let original = *pixel;
                if original[3] < 0.5 {
                    if let Some(value) = transparent {
                        *index = value as u8;
                        *pixel = [0.0; 4];
                        continue;
                    }
                    if original[3] <= 0.0 {
                        continue;
                    }
                }
                let mut color = [original[0], original[1], original[2]];
                match dither {
                    Dither::Pattern => color = color.map(|value| value + bayer8(x, y) * spread),
                    Dither::Noise => color = color.map(|value| value + (hash01(x, y, 7) - 0.5) * spread),
                    _ => {}
                }
                let selected = lookup.find(color.map(|value| value.clamp(0.0, 1.0) * 255.0));
                let entry = palette.get(selected).ok_or_else(controlled_invalid)?.map(|value| f32::from(value) / 255.0);
                *index = selected as u8;
                *pixel = [entry[0], entry[1], entry[2], if transparent.is_some() { 1.0 } else { original[3] }];
            }
        }
    } else {
        let stride = width.checked_add(2).ok_or_else(controlled_limit)?;
        let count = stride.checked_mul(2).ok_or_else(controlled_limit)?;
        let mut errors = controlled_vec::<[f32; 3]>(count, required.scratch_bytes.checked_sub(lookup_bytes).ok_or_else(controlled_limit)?)?;
        while errors.len() < count {
            controlled_poll(control)?;
            errors.resize(errors.len().saturating_add(4096).min(count), [0.0; 3]);
        }
        for y in 0..px.len() / width {
            controlled_poll(control)?;
            let (current, next) = errors.split_at_mut_checked(stride).ok_or_else(controlled_invalid)?;
            for (i, (target, source)) in current.iter_mut().zip(next.iter_mut()).enumerate() {
                if i.is_multiple_of(256) {
                    controlled_poll(control)?;
                }
                *target = *source;
                *source = [0.0; 3];
            }
            for x in 0..width {
                if x.is_multiple_of(256) {
                    controlled_poll(control)?;
                }
                let position = y.checked_mul(width).and_then(|n| n.checked_add(x)).ok_or_else(controlled_limit)?;
                let original = *px.get(position).ok_or_else(controlled_invalid)?;
                if original[3] < 0.5 {
                    if let Some(value) = transparent {
                        *indices.get_mut(position).ok_or_else(controlled_invalid)? = value as u8;
                        *px.get_mut(position).ok_or_else(controlled_invalid)? = [0.0; 4];
                        continue;
                    }
                    if original[3] <= 0.0 {
                        continue;
                    }
                }
                let offset = x.checked_add(1).ok_or_else(controlled_limit)?;
                let accumulated = *errors.get(offset).ok_or_else(controlled_invalid)?;
                let mut color = [original[0], original[1], original[2]];
                for channel in 0..3 {
                    color[channel] += accumulated[channel];
                }
                let color = color.map(|value| value.clamp(0.0, 1.0));
                let selected = lookup.find(color.map(|value| value * 255.0));
                let entry = palette.get(selected).ok_or_else(controlled_invalid)?.map(|value| f32::from(value) / 255.0);
                *indices.get_mut(position).ok_or_else(controlled_invalid)? = selected as u8;
                let delta: [f32; 3] = std::array::from_fn(|channel| (color[channel] - entry[channel]) * amount.clamp(0.0, 1.0));
                let right = x.checked_add(2).ok_or_else(controlled_limit)?;
                let (current, next) = errors.split_at_mut_checked(stride).ok_or_else(controlled_invalid)?;
                for (channel, value) in delta.iter().enumerate() {
                    current.get_mut(right).ok_or_else(controlled_invalid)?[channel] += *value * 7.0 / 16.0;
                    next.get_mut(x).ok_or_else(controlled_invalid)?[channel] += *value * 3.0 / 16.0;
                    next.get_mut(offset).ok_or_else(controlled_invalid)?[channel] += *value * 5.0 / 16.0;
                    next.get_mut(right).ok_or_else(controlled_invalid)?[channel] += *value / 16.0;
                }
                *px.get_mut(position).ok_or_else(controlled_invalid)? = [entry[0], entry[1], entry[2], if transparent.is_some() { 1.0 } else { original[3] }];
            }
        }
    }
    controlled_poll(control)?;
    Ok(indices)
}

/// Maximum palette-construction envelope; fixed palettes need no histogram.
/// This conservative bound may refuse a large input even if its eventual
/// number of unique colors would be small. No input-dependent allocation is
/// attempted before the complete envelope is admitted.
pub fn build_palette_requirements(pixels: usize, kind: PaletteKind, colors: usize, forced: Forced) -> ControlledResult<ControlledRequirements> {
    let input = u64::try_from(pixels).map_err(|_| controlled_limit())?;
    let room = colors.clamp(2, 256).saturating_sub(match forced {
        Forced::None => 0,
        Forced::BlackWhite => 2,
        Forced::Primaries => 8,
        Forced::Web => 216,
    });
    let unique = match kind {
        PaletteKind::Exact => input.min(257),
        PaletteKind::Adaptive | PaletteKind::Perceptual | PaletteKind::Selective => input.min(1 << 24),
        _ => 0,
    };
    let iterations = if matches!(kind, PaletteKind::Perceptual | PaletteKind::Selective) { 4 } else { 2 };
    let partition = if matches!(kind, PaletteKind::Adaptive | PaletteKind::Perceptual | PaletteKind::Selective) {
        controlled_mul(unique, controlled_add(controlled_mul(room as u64, 512)?, controlled_mul(iterations, controlled_mul(room as u64, 16)?)?)?)?
    } else {
        0
    };
    Ok(ControlledRequirements {
        retained_bytes: controlled_mul(input, 16)?,
        scratch_bytes: controlled_add(controlled_mul(unique, 192)?, 64 * 1024)?,
        output_bytes: 768,
        work_units: controlled_add(controlled_mul(input, 32)?, controlled_add(controlled_mul(unique, 512)?, partition)?)?,
    })
}
fn controlled_web(palette: &mut Vec<Rgb8>) {
    for r in 0..6u8 {
        for g in 0..6u8 {
            for b in 0..6u8 {
                palette.push([r * 51, g * 51, b * 51]);
            }
        }
    }
}
fn controlled_forced(forced: Forced) -> ControlledResult<Vec<Rgb8>> {
    let mut palette = controlled_vec(256, 768)?;
    match forced {
        Forced::None => {}
        Forced::BlackWhite => palette.extend_from_slice(&[[0, 0, 0], [255, 255, 255]]),
        Forced::Primaries => {
            palette.extend_from_slice(&[[0, 0, 0], [255, 255, 255], [255, 0, 0], [0, 255, 0], [0, 0, 255], [0, 255, 255], [255, 0, 255], [255, 255, 0]])
        }
        Forced::Web => controlled_web(&mut palette),
    }
    Ok(palette)
}
fn controlled_fixed(kind: PaletteKind, count: usize) -> ControlledResult<Vec<Rgb8>> {
    let mut palette = controlled_vec(256, 768)?;
    match kind {
        PaletteKind::Web => controlled_web(&mut palette),
        PaletteKind::SystemMac => {
            controlled_web(&mut palette);
            for channel in 0..4 {
                for k in (1..=15u8).rev() {
                    let value = k * 17;
                    if value % 51 != 0 {
                        palette.push(match channel {
                            0 => [value, 0, 0],
                            1 => [0, value, 0],
                            2 => [0, 0, value],
                            _ => [value; 3],
                        });
                    }
                }
            }
            palette.truncate(256);
        }
        PaletteKind::SystemWindows => {
            palette.extend_from_slice(&[
                [0, 0, 0],
                [128, 0, 0],
                [0, 128, 0],
                [128, 128, 0],
                [0, 0, 128],
                [128, 0, 128],
                [0, 128, 128],
                [192, 192, 192],
                [192, 220, 192],
                [166, 202, 240],
                [255, 251, 240],
                [160, 160, 164],
                [128, 128, 128],
                [255, 0, 0],
                [0, 255, 0],
                [255, 255, 0],
                [0, 0, 255],
                [255, 0, 255],
                [0, 255, 255],
                [255, 255, 255],
            ]);
            for r in 0..6u8 {
                for g in 0..6u8 {
                    for b in 0..6u8 {
                        let color = [r * 51, g * 51, b * 51];
                        if !palette.contains(&color) {
                            palette.push(color);
                        }
                    }
                }
            }
            let mut gray = 8u8;
            while palette.len() < 256 {
                if !palette.contains(&[gray; 3]) {
                    palette.push([gray; 3]);
                }
                gray = gray.wrapping_add(12);
            }
        }
        PaletteKind::Uniform => {
            let side = ((count as f32).cbrt().floor() as usize).max(2);
            let level = |index: usize| ((index as f32 / (side - 1) as f32) * 255.0).round() as u8;
            for r in 0..side {
                for g in 0..side {
                    for b in 0..side {
                        palette.push([level(r), level(g), level(b)]);
                    }
                }
            }
        }
        _ => return Err(controlled_invalid()),
    }
    Ok(palette)
}
type ControlledPoint = ([f32; 3], u32);
fn controlled_histogram_sort(values: &mut [(Rgb8, u32)], control: &photocraft_raster::Interrupt<'_>) -> ControlledResult<()> {
    fn sift(values: &mut [(Rgb8, u32)], mut root: usize, end: usize, control: &photocraft_raster::Interrupt<'_>) -> ControlledResult<()> {
        loop {
            controlled_poll(control)?;
            let Some(left) = root.checked_mul(2).and_then(|n| n.checked_add(1)).filter(|n| *n < end) else { return Ok(()) };
            let right = left.checked_add(1).ok_or_else(controlled_limit)?;
            let selected =
                if right < end && values.get(left).ok_or_else(controlled_invalid)? < values.get(right).ok_or_else(controlled_invalid)? { right } else { left };
            if values.get(root).ok_or_else(controlled_invalid)? >= values.get(selected).ok_or_else(controlled_invalid)? {
                return Ok(());
            }
            values.swap(root, selected);
            root = selected;
        }
    }
    for root in (0..values.len() / 2).rev() {
        sift(values, root, values.len(), control)?;
    }
    for end in (1..values.len()).rev() {
        controlled_poll(control)?;
        values.swap(0, end);
        sift(values, 0, end, control)?;
    }
    controlled_poll(control)
}
fn controlled_copy<T: Copy>(source: &[T], ceiling: u64, control: &photocraft_raster::Interrupt<'_>) -> ControlledResult<Vec<T>> {
    let mut output = controlled_vec(source.len(), ceiling)?;
    for chunk in source.chunks(4096) {
        controlled_poll(control)?;
        output.extend_from_slice(chunk);
    }
    Ok(output)
}
fn controlled_sort(points: &mut [ControlledPoint], axis: usize, ceiling: u64, control: &photocraft_raster::Interrupt<'_>) -> ControlledResult<()> {
    // Fallible stable merge sorting preserves equal-feature order exactly as
    // the legacy stable sort, including ordering inherited from earlier splits.
    let mut scratch = controlled_copy(points, ceiling, control)?;
    let mut run = 1usize;
    while run < points.len() {
        let step = run.checked_mul(2).ok_or_else(controlled_limit)?;
        let mut start = 0usize;
        while start < points.len() {
            controlled_poll(control)?;
            let middle = start.saturating_add(run).min(points.len());
            let end = start.saturating_add(step).min(points.len());
            let (mut left, mut right) = (start, middle);
            for target in start..end {
                if target.is_multiple_of(256) {
                    controlled_poll(control)?;
                }
                let choose_left = left < middle
                    && (right >= end
                        || points
                            .get(left)
                            .and_then(|p| p.0.get(axis))
                            .ok_or_else(controlled_invalid)?
                            .total_cmp(points.get(right).and_then(|p| p.0.get(axis)).ok_or_else(controlled_invalid)?)
                            != std::cmp::Ordering::Greater);
                let next = if choose_left {
                    let i = left;
                    left += 1;
                    i
                } else {
                    let i = right;
                    right += 1;
                    i
                };
                *scratch.get_mut(target).ok_or_else(controlled_invalid)? = *points.get(next).ok_or_else(controlled_invalid)?;
            }
            start = end;
        }
        for (target, source) in points.chunks_mut(4096).zip(scratch.chunks(4096)) {
            controlled_poll(control)?;
            target.copy_from_slice(source);
        }
        run = step;
    }
    Ok(())
}
fn controlled_median(points: &[ControlledPoint], count: usize, ceiling: u64, control: &photocraft_raster::Interrupt<'_>) -> ControlledResult<Vec<[f32; 3]>> {
    let mut centers = controlled_vec::<[f32; 3]>(count, ceiling)?;
    if points.is_empty() || count == 0 {
        return Ok(centers);
    }
    let mut boxes = controlled_vec::<Vec<ControlledPoint>>(count, ceiling)?;
    boxes.push(controlled_copy(points, ceiling, control)?);
    while boxes.len() < count {
        controlled_poll(control)?;
        let mut choice = (usize::MAX, 0usize, -1.0f32);
        for (index, part) in boxes.iter().enumerate().filter(|(_, part)| part.len() > 1) {
            let mut best = (0usize, -1.0f32);
            for axis in 0..3 {
                let (mut low, mut high) = (f32::MAX, f32::MIN);
                for (i, point) in part.iter().enumerate() {
                    if i.is_multiple_of(256) {
                        controlled_poll(control)?;
                    }
                    let value = *point.0.get(axis).ok_or_else(controlled_invalid)?;
                    low = low.min(value);
                    high = high.max(value);
                }
                if high - low > best.1 {
                    best = (axis, high - low);
                }
            }
            let mut weight = 0.0f32;
            for (i, point) in part.iter().enumerate() {
                if i.is_multiple_of(256) {
                    controlled_poll(control)?;
                }
                weight += point.1 as f32;
            }
            let score = best.1 * weight.sqrt();
            if score > choice.2 {
                choice = (index, best.0, score);
            }
        }
        if choice.0 == usize::MAX || choice.2 <= 0.0 {
            break;
        }
        let mut part = boxes.swap_remove(choice.0);
        controlled_sort(&mut part, choice.1, ceiling, control)?;
        let mut total = 0u64;
        for (i, point) in part.iter().enumerate() {
            if i.is_multiple_of(256) {
                controlled_poll(control)?;
            }
            total = total.checked_add(u64::from(point.1)).ok_or_else(controlled_limit)?;
        }
        let mut accumulated = 0u64;
        let mut cut = 1usize;
        for (i, point) in part.iter().enumerate() {
            if i.is_multiple_of(256) {
                controlled_poll(control)?;
            }
            accumulated = accumulated.checked_add(u64::from(point.1)).ok_or_else(controlled_limit)?;
            if accumulated.checked_mul(2).ok_or_else(controlled_limit)? >= total {
                cut = (i + 1).clamp(1, part.len() - 1);
                break;
            }
        }
        let left = controlled_copy(part.get(..cut).ok_or_else(controlled_invalid)?, ceiling, control)?;
        let right = controlled_copy(part.get(cut..).ok_or_else(controlled_invalid)?, ceiling, control)?;
        drop(part);
        boxes.push(left);
        boxes.push(right);
    }
    for part in &boxes {
        let mut weight = 0.0f64;
        for (i, point) in part.iter().enumerate() {
            if i.is_multiple_of(256) {
                controlled_poll(control)?;
            }
            weight += f64::from(point.1);
        }
        if weight <= 0.0 {
            return Err(controlled_invalid());
        }
        let mut center = [0.0; 3];
        for (axis, target) in center.iter_mut().enumerate() {
            let mut sum = 0.0f64;
            for (i, point) in part.iter().enumerate() {
                if i.is_multiple_of(256) {
                    controlled_poll(control)?;
                }
                sum += f64::from(*point.0.get(axis).ok_or_else(controlled_invalid)?) * f64::from(point.1);
            }
            *target = (sum / weight) as f32;
        }
        centers.push(center);
    }
    Ok(centers)
}
fn controlled_kmeans(
    points: &[ControlledPoint],
    centers: &mut [[f32; 3]],
    iterations: usize,
    ceiling: u64,
    control: &photocraft_raster::Interrupt<'_>,
) -> ControlledResult<()> {
    if centers.is_empty() {
        return Ok(());
    }
    let mut sums = controlled_vec::<[f64; 3]>(centers.len(), ceiling)?;
    sums.resize(centers.len(), [0.0; 3]);
    let mut counts = controlled_vec::<f64>(centers.len(), ceiling)?;
    counts.resize(centers.len(), 0.0);
    for _ in 0..iterations {
        controlled_poll(control)?;
        sums.fill([0.0; 3]);
        counts.fill(0.0);
        let lookup = Nearest::try_new(centers.iter().copied(), control)?;
        for (index, (point, weight)) in points.iter().enumerate() {
            if index.is_multiple_of(256) {
                controlled_poll(control)?;
            }
            let selected = lookup.find(*point);
            let sum = sums.get_mut(selected).ok_or_else(controlled_invalid)?;
            for axis in 0..3 {
                sum[axis] += f64::from(point[axis]) * f64::from(*weight);
            }
            *counts.get_mut(selected).ok_or_else(controlled_invalid)? += f64::from(*weight);
        }
        for (index, center) in centers.iter_mut().enumerate() {
            let weight = *counts.get(index).ok_or_else(controlled_invalid)?;
            if weight > 0.0 {
                let sum = sums.get(index).ok_or_else(controlled_invalid)?;
                *center = std::array::from_fn(|axis| (sum[axis] / weight) as f32);
            }
        }
    }
    Ok(())
}
pub fn build_palette_with_interrupt(
    px: &[[f32; 4]],
    kind: PaletteKind,
    colors: usize,
    forced: Forced,
    limits: ControlledLimits,
    control: &photocraft_raster::Interrupt<'_>,
) -> ControlledResult<Vec<Rgb8>> {
    controlled_poll(control)?;
    let required = build_palette_requirements(px.len(), kind, colors, forced)?;
    controlled_admit(required, limits)?;
    controlled_finite(px, control)?;
    let colors = colors.clamp(2, 256);
    let mut palette = controlled_forced(forced)?;
    let room = colors.saturating_sub(palette.len());
    let mut computed = controlled_vec::<Rgb8>(256, 768)?;
    if matches!(kind, PaletteKind::SystemMac | PaletteKind::SystemWindows | PaletteKind::Web | PaletteKind::Uniform) {
        computed = controlled_fixed(kind, room.max(8))?;
    } else {
        let upper = if kind == PaletteKind::Exact { px.len().min(257) } else { px.len().min(1 << 24) };
        let mut histogram = std::collections::HashMap::<Rgb8, u32>::new();
        histogram.try_reserve(upper).map_err(|_| controlled_limit())?;
        if controlled_mul(histogram.capacity() as u64, 32)? > required.scratch_bytes {
            return Err(controlled_limit());
        }
        for (index, pixel) in px.iter().enumerate() {
            if index.is_multiple_of(256) {
                controlled_poll(control)?;
            }
            if pixel[3] < 0.5 {
                continue;
            }
            let color = rgb8(pixel);
            if !histogram.contains_key(&color) && histogram.len() >= upper {
                return Err(controlled_limit());
            }
            let count = histogram.entry(color).or_default();
            *count = count.checked_add(1).ok_or_else(controlled_limit)?;
            if kind == PaletteKind::Exact && histogram.len() > 256 {
                return Err(controlled_limit());
            }
        }
        let mut ordered = controlled_vec(histogram.len(), required.scratch_bytes)?;
        for (index, value) in histogram.into_iter().enumerate() {
            if index.is_multiple_of(256) {
                controlled_poll(control)?;
            }
            ordered.push(value);
        }
        // Unique integer tuple ordering yields the exact legacy histogram order;
        // the heap traversal additionally polls through every bounded sort step.
        controlled_histogram_sort(&mut ordered, control)?;
        if kind == PaletteKind::Exact || ordered.len() <= room {
            for value in &ordered {
                computed.push(value.0);
            }
        } else {
            let mut points = controlled_vec(ordered.len(), required.scratch_bytes)?;
            for (index, (color, count)) in ordered.iter().enumerate() {
                if index.is_multiple_of(256) {
                    controlled_poll(control)?;
                }
                let feature = if kind == PaletteKind::Adaptive { color.map(f32::from) } else { srgb_to_lab(color.map(|v| f32::from(v) / 255.0)) };
                points.push((feature, *count));
            }
            let mut centers = controlled_median(&points, room, required.scratch_bytes, control)?;
            controlled_kmeans(&points, &mut centers, if kind == PaletteKind::Adaptive { 2 } else { 4 }, required.scratch_bytes, control)?;
            for center in centers {
                computed.push(if kind == PaletteKind::Adaptive { center.map(|value| value.round().clamp(0.0, 255.0) as u8) } else { lab_to_rgb8(center) });
            }
            if kind == PaletteKind::Selective {
                for color in &mut computed {
                    let snap = color.map(|v| ((f32::from(v) / 51.0).round() * 51.0) as u8);
                    if color.iter().zip(snap).all(|(a, b)| a.abs_diff(b) <= 10) {
                        *color = snap;
                    }
                }
            }
        }
    }
    for color in computed {
        controlled_poll(control)?;
        if palette.len() >= 256 {
            break;
        }
        if !palette.contains(&color) {
            palette.push(color);
        }
    }
    if matches!(kind, PaletteKind::Adaptive | PaletteKind::Perceptual | PaletteKind::Selective | PaletteKind::Uniform) {
        let forced_count = match forced {
            Forced::None => 0,
            Forced::BlackWhite => 2,
            Forced::Primaries => 8,
            Forced::Web => 216,
        };
        palette.truncate(colors.max(forced_count));
    }
    if palette.is_empty() {
        palette.push([0, 0, 0]);
    }
    controlled_poll(control)?;
    Ok(palette)
}

pub fn bitmap_requirements(pixels: usize, width: usize, method: BitmapMethod) -> ControlledResult<ControlledRequirements> {
    if width == 0 || !pixels.is_multiple_of(width) {
        return Err(controlled_invalid());
    }
    if let BitmapMethod::Halftone { cell, angle, .. } = method
        && (!cell.is_finite() || !angle.is_finite())
    {
        return Err(controlled_invalid());
    }
    let pixels = u64::try_from(pixels).map_err(|_| controlled_limit())?;
    let retained = controlled_mul(pixels, 4)?;
    let (scratch, units) = match method {
        BitmapMethod::Threshold => (0, 4),
        BitmapMethod::Pattern => (0, 32),
        BitmapMethod::Diffusion => (retained, 64),
        BitmapMethod::Halftone { .. } => (4096, 128),
    };
    Ok(ControlledRequirements {
        retained_bytes: retained,
        scratch_bytes: scratch,
        output_bytes: 0,
        work_units: controlled_add(controlled_mul(pixels, units)?, if matches!(method, BitmapMethod::Halftone { .. }) { 32 * 1024 } else { 0 })?,
    })
}
/// Ordinary finite F32 bitmap mathematics on a caller-private buffer. Original
/// APIs remain unchanged. Cancellation can leave private partial output only.
pub fn to_bitmap_with_interrupt(
    gray: &mut [f32],
    width: usize,
    method: BitmapMethod,
    limits: ControlledLimits,
    control: &photocraft_raster::Interrupt<'_>,
) -> ControlledResult<()> {
    controlled_poll(control)?;
    let required = bitmap_requirements(gray.len(), width, method)?;
    controlled_admit(required, limits)?;
    for chunk in gray.chunks(4096) {
        controlled_poll(control)?;
        if chunk.iter().any(|value| !value.is_finite()) {
            return Err(controlled_invalid());
        }
    }
    match method {
        BitmapMethod::Threshold => {
            for chunk in gray.chunks_mut(4096) {
                controlled_poll(control)?;
                for value in chunk {
                    *value = if *value >= 0.5 { 1.0 } else { 0.0 };
                }
            }
        }
        BitmapMethod::Pattern => {
            for (index, value) in gray.iter_mut().enumerate() {
                if index.is_multiple_of(256) {
                    controlled_poll(control)?;
                }
                *value = if *value > bayer8(index % width, index / width) + 0.5 { 1.0 } else { 0.0 };
            }
        }
        BitmapMethod::Diffusion => {
            let mut buffer = controlled_copy(gray, required.scratch_bytes, control)?;
            let height = gray.len() / width;
            for y in 0..height {
                controlled_poll(control)?;
                for x in 0..width {
                    if x.is_multiple_of(256) {
                        controlled_poll(control)?;
                    }
                    let index = y.checked_mul(width).and_then(|n| n.checked_add(x)).ok_or_else(controlled_limit)?;
                    let old = *buffer.get(index).ok_or_else(controlled_invalid)?;
                    let new = if old >= 0.5 { 1.0 } else { 0.0 };
                    let error = old - new;
                    *gray.get_mut(index).ok_or_else(controlled_invalid)? = new;
                    if x + 1 < width {
                        *buffer.get_mut(index.checked_add(1).ok_or_else(controlled_limit)?).ok_or_else(controlled_invalid)? += error * 7.0 / 16.0;
                    }
                    if y + 1 < height {
                        if x > 0 {
                            *buffer
                                .get_mut(index.checked_add(width).and_then(|n| n.checked_sub(1)).ok_or_else(controlled_limit)?)
                                .ok_or_else(controlled_invalid)? += error * 3.0 / 16.0;
                        }
                        *buffer.get_mut(index.checked_add(width).ok_or_else(controlled_limit)?).ok_or_else(controlled_invalid)? += error * 5.0 / 16.0;
                        if x + 1 < width {
                            *buffer
                                .get_mut(index.checked_add(width).and_then(|n| n.checked_add(1)).ok_or_else(controlled_limit)?)
                                .ok_or_else(controlled_invalid)? += error / 16.0;
                        }
                    }
                }
            }
        }
        BitmapMethod::Halftone { cell, angle, shape } => {
            let cell = cell.max(2.0);
            let (sin, cos) = angle.to_radians().sin_cos();
            let mut table = controlled_vec::<f32>(1024, required.scratch_bytes)?;
            for index in 0..1024usize {
                if index.is_multiple_of(256) {
                    controlled_poll(control)?;
                }
                table.push(spot(shape, ((index % 32) as f32 + 0.5) / 16.0 - 1.0, ((index / 32) as f32 + 0.5) / 16.0 - 1.0));
            }
            // Exactly 1024 entries: this sort is a bounded cooperative region.
            table.sort_unstable_by(f32::total_cmp);
            controlled_poll(control)?;
            let cdf = |value: f32| table.partition_point(|threshold| *threshold < value) as f32 / table.len() as f32;
            for (index, value) in gray.iter_mut().enumerate() {
                if index.is_multiple_of(256) {
                    controlled_poll(control)?;
                }
                let (x, y) = ((index % width) as f32 + 0.5, (index / width) as f32 + 0.5);
                let (rx, ry) = (x * cos + y * sin, -x * sin + y * cos);
                let u = (rx / cell).rem_euclid(1.0) * 2.0 - 1.0;
                let v = (ry / cell).rem_euclid(1.0) * 2.0 - 1.0;
                *value = if cdf(spot(shape, u, v)) >= *value { 0.0 } else { 1.0 };
            }
        }
    }
    controlled_poll(control)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Implementation-wave drafts, not run as part of this change.
    #[test]
    fn controlled_palette_matches_legacy_selection_forced_order_and_partition_ties() {
        let pixels: Vec<[f32; 4]> = (0..64)
            .map(|i| [(i % 8) as f32 / 7.0, (i / 8) as f32 / 7.0, if i % 3 == 0 { 1.25 } else { -0.25 }, if i % 5 == 0 { 0.499 } else { 1.0 }])
            .collect();
        for kind in [
            PaletteKind::Exact,
            PaletteKind::SystemMac,
            PaletteKind::SystemWindows,
            PaletteKind::Web,
            PaletteKind::Uniform,
            PaletteKind::Adaptive,
            PaletteKind::Perceptual,
            PaletteKind::Selective,
        ] {
            for forced in [Forced::None, Forced::BlackWhite, Forced::Primaries, Forced::Web] {
                for colors in [2, 6, 32] {
                    assert_eq!(
                        build_palette_with_interrupt(&pixels, kind, colors, forced, ControlledLimits::default(), &photocraft_raster::Interrupt::NONE).unwrap(),
                        build_palette(&pixels, kind, colors, forced).unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn controlled_quantize_matches_legacy_ordinals_and_every_rgba_word() {
        let input: Vec<[f32; 4]> = (0..64)
            .map(|i| {
                [
                    (i % 8) as f32 / 7.0,
                    (i / 8) as f32 / 7.0,
                    if i % 3 == 0 { 1.5 } else { -0.25 },
                    match i % 5 {
                        0 => -0.0,
                        1 => 0.499,
                        2 => 0.5,
                        _ => 1.25,
                    },
                ]
            })
            .collect();
        let palette = [[0, 0, 0], [255, 255, 255], [0, 0, 0], [255, 0, 0]];
        for dither in [Dither::None, Dither::Pattern, Dither::Noise, Dither::Diffusion] {
            for transparent in [None, Some(1)] {
                let mut expected = input.clone();
                let mut actual = input.clone();
                let indices = quantize(&mut expected, 8, &palette, dither, 0.73, transparent);
                assert_eq!(
                    quantize_with_interrupt(
                        &mut actual,
                        8,
                        &palette,
                        dither,
                        0.73,
                        transparent,
                        ControlledLimits::default(),
                        &photocraft_raster::Interrupt::NONE
                    )
                    .unwrap(),
                    indices
                );
                assert!(actual.iter().flatten().zip(expected.iter().flatten()).all(|(a, b)| a.to_bits() == b.to_bits()));
            }
        }
    }

    #[test]
    fn controlled_bitmap_matches_all_legacy_methods_and_early_refusals_keep_input() {
        let input: Vec<f32> = (0..64).map(|i| i as f32 / 63.0).collect();
        let mut methods = vec![BitmapMethod::Threshold, BitmapMethod::Pattern, BitmapMethod::Diffusion];
        for shape in [HalftoneShape::Round, HalftoneShape::Ellipse, HalftoneShape::Line, HalftoneShape::Square, HalftoneShape::Diamond, HalftoneShape::Cross] {
            methods.push(BitmapMethod::Halftone { cell: 2.3, angle: 31.2, shape });
        }
        for method in methods {
            let mut expected = input.clone();
            let mut actual = input.clone();
            to_bitmap(&mut expected, 8, method);
            to_bitmap_with_interrupt(&mut actual, 8, method, ControlledLimits::default(), &photocraft_raster::Interrupt::NONE).unwrap();
            assert_eq!(actual, expected);
        }
        let mut pixels = vec![[0.25, 0.5, 0.75, 1.0]; 64];
        let before = pixels.clone();
        let cancelled = photocraft_raster::Interrupt::cancel_only(&|| true);
        assert!(quantize_with_interrupt(&mut pixels, 8, &[[0, 0, 0]], Dither::Diffusion, 1.0, None, ControlledLimits::default(), &cancelled).is_err());
        assert_eq!(pixels, before);
        assert!(
            quantize_with_interrupt(&mut pixels, 0, &[[0, 0, 0]], Dither::None, 1.0, None, ControlledLimits::default(), &photocraft_raster::Interrupt::NONE)
                .is_err()
        );
        assert_eq!(pixels, before);
        assert!(
            quantize_with_interrupt(
                &mut pixels,
                8,
                &[[0, 0, 0]],
                Dither::None,
                1.0,
                None,
                ControlledLimits { resident_bytes: 1, work_units: 1 },
                &photocraft_raster::Interrupt::NONE
            )
            .is_err()
        );
        assert_eq!(pixels, before);
    }

    fn gradient(w: usize, h: usize) -> Vec<[f32; 4]> {
        (0..w * h).map(|i| [(i % w) as f32 / (w - 1) as f32, (i / w) as f32 / (h - 1) as f32, 0.3, 1.0]).collect()
    }

    #[test]
    fn fixed_palettes_have_expected_sizes() {
        assert_eq!(web_palette().len(), 216);
        assert_eq!(mac_palette().len(), 256);
        assert_eq!(windows_palette().len(), 256);
        assert_eq!(uniform_palette(27).len(), 27);
        let mut m = mac_palette();
        m.sort_unstable();
        m.dedup();
        assert_eq!(m.len(), 256, "mac palette entries are unique");
    }

    #[test]
    fn exact_palette_and_failure() {
        let px = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 0.0, 0.0, 1.0]];
        assert_eq!(build_palette(&px, PaletteKind::Exact, 256, Forced::None).unwrap().len(), 2);
        assert!(build_palette(&gradient(64, 64), PaletteKind::Exact, 256, Forced::None).is_err());
    }

    #[test]
    fn adaptive_and_perceptual_respect_count_and_forced() {
        let px = gradient(40, 40);
        for kind in [PaletteKind::Adaptive, PaletteKind::Perceptual, PaletteKind::Selective] {
            let pal = build_palette(&px, kind, 16, Forced::BlackWhite).unwrap();
            assert!(pal.len() <= 16 && pal.len() >= 8, "{kind:?} {}", pal.len());
            assert_eq!(&pal[..2], &[[0, 0, 0], [255, 255, 255]]);
        }
    }

    #[test]
    fn forced_colours_filling_the_count_do_not_panic() {
        // 2 colours + Black and White (or 8 + Primaries) leaves no room for
        // computed entries; the median cut used to return no centres and
        // k-means indexed into the empty list.
        let px = vec![[1.0, 1.0, 1.0, 1.0]; 16];
        for kind in [PaletteKind::Adaptive, PaletteKind::Perceptual, PaletteKind::Selective] {
            let pal = build_palette(&px, kind, 2, Forced::BlackWhite).unwrap();
            assert_eq!(pal, vec![[0, 0, 0], [255, 255, 255]], "{kind:?}");
            let pal = build_palette(&gradient(8, 8), kind, 8, Forced::Primaries).unwrap();
            assert_eq!(pal.len(), 8, "{kind:?}");
        }
    }

    #[test]
    #[ignore = "24 MP release timing comparison; run with --release --ignored --nocapture"]
    fn indexed_diffusion_24mp_release_comparison() {
        let source = gradient(6000, 4000);
        let pal = web_palette();
        for run in 0..4 {
            let mut linear = source.clone();
            let mut accelerated = source.clone();
            let mut before = None;
            let mut after = None;
            // Reverse the order on alternating rounds; exclude the first round as warmup.
            for old in if run % 2 == 0 { [true, false] } else { [false, true] } {
                let start = std::time::Instant::now();
                if old {
                    let indices = quantize_with(&mut linear, 6000, &pal, Dither::Diffusion, 0.75, None, |c| linear_rgb_nearest(&pal, c));
                    before = Some((start.elapsed().as_secs_f64() * 1000.0, indices));
                } else {
                    let indices = quantize(&mut accelerated, 6000, &pal, Dither::Diffusion, 0.75, None);
                    after = Some((start.elapsed().as_secs_f64() * 1000.0, indices));
                }
            }
            let (before_ms, before_indices) = before.unwrap();
            let (after_ms, after_indices) = after.unwrap();
            assert_eq!(before_indices.iter().zip(&after_indices).position(|(a, b)| a != b), None, "first differing index");
            assert_eq!(linear.iter().zip(&accelerated).position(|(a, b)| a != b), None, "first differing pixel");
            println!(
                "{}",
                serde_json::json!({"run":run,"warmup":run==0,"pixels":24_000_000,"palette_entries":216,"linear_ms":before_ms,"accelerated_ms":after_ms})
            );
        }
    }

    fn linear_rgb_nearest(pal: &[Rgb8], c: [f32; 3]) -> usize {
        let mut best = (f32::MAX, 0);
        for (i, e) in pal.iter().enumerate() {
            let d: f32 = (0..3).map(|k| (c[k] * 255.0 - f32::from(e[k])).powi(2)).sum();
            if d < best.0 {
                best = (d, i);
            }
        }
        best.1
    }

    #[test]
    fn all_dithers_match_the_original_linear_palette_search_exactly() {
        let mut source = gradient(93, 61);
        for (i, p) in source.iter_mut().enumerate() {
            p[2] = (i % 71) as f32 / 70.0;
            p[3] = [0.0, 0.2, 0.7, 1.0][i % 4];
        }
        for count in [8, 16, 32, 64, 128, 256] {
            let pal = build_palette(&source, PaletteKind::Selective, count, Forced::BlackWhite).unwrap();
            for dither in [Dither::None, Dither::Diffusion, Dither::Pattern, Dither::Noise] {
                for transparent in [None, Some(1)] {
                    let mut actual = source.clone();
                    let mut expected = source.clone();
                    let indices = quantize(&mut actual, 93, &pal, dither, 0.75, transparent);
                    let old_indices = quantize_with(&mut expected, 93, &pal, dither, 0.75, transparent, |c| linear_rgb_nearest(&pal, c));
                    assert_eq!(indices, old_indices, "{count} {dither:?} {transparent:?}");
                    assert_eq!(actual, expected, "{count} {dither:?} {transparent:?}");
                }
            }
        }
    }

    #[test]
    fn quantize_maps_to_palette_with_each_dither() {
        let pal = vec![[0, 0, 0], [255, 255, 255]];
        for d in [Dither::None, Dither::Diffusion, Dither::Pattern, Dither::Noise] {
            let mut px: Vec<[f32; 4]> = (0..64)
                .map(|i| [i as f32 / 63.0; 4])
                .map(|mut p| {
                    p[3] = 1.0;
                    p
                })
                .collect();
            let idx = quantize(&mut px, 8, &pal, d, 1.0, None);
            assert!(px.iter().all(|p| p[0] == 0.0 || p[0] == 1.0), "{d:?}");
            assert_eq!(idx.len(), 64);
            // Mean brightness is preserved by dithering (not by plain thresholding).
            if d == Dither::Diffusion {
                let mean = px.iter().map(|p| p[0]).sum::<f32>() / 64.0;
                assert!((mean - 0.5).abs() < 0.08, "{mean}");
            }
        }
        let mut px = vec![[1.0, 1.0, 1.0, 0.2]];
        let idx = quantize(&mut px, 1, &pal, Dither::None, 1.0, Some(1));
        assert_eq!((idx[0], px[0][3]), (1, 0.0));
    }

    #[test]
    fn bitmap_methods_are_binary_and_track_tone() {
        for m in [
            BitmapMethod::Threshold,
            BitmapMethod::Pattern,
            BitmapMethod::Diffusion,
            BitmapMethod::Halftone { cell: 6.0, angle: 45.0, shape: HalftoneShape::Round },
        ] {
            let mut g = vec![0.25f32; 32 * 32];
            to_bitmap(&mut g, 32, m);
            assert!(g.iter().all(|v| *v == 0.0 || *v == 1.0));
            let mean = g.iter().sum::<f32>() / g.len() as f32;
            if m != BitmapMethod::Threshold {
                assert!((mean - 0.25).abs() < 0.15, "{m:?} {mean}");
            } else {
                assert_eq!(mean, 0.0);
            }
        }
    }
}
