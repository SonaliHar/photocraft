#![allow(dead_code)]
use std::io::Write;
pub fn u32be(v: &mut Vec<u8>, n: u32) {
    v.extend(n.to_be_bytes());
}
pub fn pointer(v: &mut Vec<u8>, n: usize, version: u32) {
    if version >= 11 {
        v.extend((n as u64).to_be_bytes());
    } else {
        u32be(v, n as u32);
    }
}
pub fn patch(v: &mut [u8], at: usize, n: usize, version: u32) {
    if version >= 11 {
        v[at..at + 8].copy_from_slice(&(n as u64).to_be_bytes());
    } else {
        v[at..at + 4].copy_from_slice(&(n as u32).to_be_bytes());
    }
}
pub fn prop(v: &mut Vec<u8>, tag: u32, payload: &[u8]) {
    u32be(v, tag);
    u32be(v, payload.len() as u32);
    v.extend(payload);
}
pub fn image(version: u32, precision: u32, compression: u8, width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    image_custom(version, precision, compression, width, height, pixels, &FixtureOptions::default())
}
#[derive(Default)]
pub struct FixtureOptions {
    pub model: u32,
    pub layer_type: Option<u32>,
    pub name: Option<String>,
    pub image_properties: Vec<(u32, Vec<u8>)>,
    pub layer_properties: Vec<(u32, Vec<u8>)>,
    pub effect_records: Vec<Vec<u8>>,
    pub path_records: Vec<Vec<u8>>,
}
#[allow(clippy::too_many_arguments)]
pub fn image_custom(version: u32, precision: u32, compression: u8, width: u32, height: u32, pixels: &[u8], options: &FixtureOptions) -> Vec<u8> {
    let mut v = if version == 0 { b"gimp xcf file\0".to_vec() } else { format!("gimp xcf v{version:03}\0").into_bytes() };
    u32be(&mut v, width);
    u32be(&mut v, height);
    u32be(&mut v, options.model);
    if version >= 4 {
        u32be(&mut v, precision);
    }
    prop(&mut v, 17, &[compression]);
    for (tag, data) in &options.image_properties {
        prop(&mut v, *tag, data);
    }
    prop(&mut v, 0, &[]);
    let layer_ptr = v.len();
    pointer(&mut v, 0, version);
    pointer(&mut v, 0, version);
    pointer(&mut v, 0, version);
    let mut path_pointers = Vec::new();
    if version >= 18 {
        for _ in &options.path_records {
            path_pointers.push(v.len());
            pointer(&mut v, 0, version);
        }
        pointer(&mut v, 0, version);
    }
    let layer = v.len();
    patch(&mut v, layer_ptr, layer, version);
    u32be(&mut v, width);
    u32be(&mut v, height);
    let layer_type = options.layer_type.unwrap_or(options.model * 2 + 1);
    u32be(&mut v, layer_type);
    let name = options.name.as_deref().unwrap_or("Layer");
    u32be(&mut v, (name.len() + 1) as u32);
    v.extend(name.as_bytes());
    v.push(0);
    for (tag, data) in &options.layer_properties {
        prop(&mut v, *tag, data);
    }
    prop(&mut v, 0, &[]);
    let hp = v.len();
    pointer(&mut v, 0, version);
    pointer(&mut v, 0, version);
    let mut effect_pointers = Vec::new();
    if version >= 20 {
        for _ in &options.effect_records {
            effect_pointers.push(v.len());
            pointer(&mut v, 0, version);
        }
        pointer(&mut v, 0, version);
    }
    let hierarchy = v.len();
    patch(&mut v, hp, hierarchy, version);
    let sample = match precision {
        200 | 250 | 275 | 500 | 550 | 575 => 2,
        300 | 350 | 375 | 600 | 650 | 675 => 4,
        700 | 750 | 775 => 8,
        _ => 1,
    };
    let channels = match layer_type {
        0 => 3,
        1 => 4,
        2 | 4 => 1,
        3 | 5 => 2,
        _ => panic!("synthetic layer type"),
    };
    let bpp = if options.model == 2 { channels } else { channels * sample };
    u32be(&mut v, width);
    u32be(&mut v, height);
    u32be(&mut v, bpp);
    let lp = v.len();
    pointer(&mut v, 0, version);
    pointer(&mut v, 0, version);
    let level = v.len();
    patch(&mut v, lp, level, version);
    u32be(&mut v, width);
    u32be(&mut v, height);
    let count = width.div_ceil(64) * height.div_ceil(64);
    let pointers = v.len();
    for _ in 0..=count {
        pointer(&mut v, 0, version);
    }
    let stride = if version >= 11 { 8 } else { 4 };
    for y in 0..height.div_ceil(64) {
        for x in 0..width.div_ceil(64) {
            let at = v.len();
            patch(&mut v, pointers + ((y * width.div_ceil(64) + x) as usize) * stride, at, version);
            let tw = (width - x * 64).min(64);
            let th = (height - y * 64).min(64);
            let mut tile = Vec::new();
            for row in 0..th {
                let start = (((y * 64 + row) * width + x * 64) * bpp) as usize;
                tile.extend(&pixels[start..start + (tw * bpp) as usize]);
            }
            match compression {
                0 => v.extend(tile),
                1 => {
                    for c in 0..bpp as usize {
                        let plane: Vec<_> = tile.iter().skip(c).step_by(bpp as usize).copied().collect();
                        for p in plane.chunks(127) {
                            v.push((256 - p.len()) as u8);
                            v.extend(p);
                        }
                    }
                }
                2 => {
                    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                    z.write_all(&tile).unwrap();
                    v.extend(z.finish().unwrap());
                }
                _ => v.extend(tile),
            }
        }
    }
    for (ptr, record) in effect_pointers.into_iter().zip(&options.effect_records) {
        let at = v.len();
        patch(&mut v, ptr, at, version);
        v.extend(record);
    }
    for (ptr, record) in path_pointers.into_iter().zip(&options.path_records) {
        let at = v.len();
        patch(&mut v, ptr, at, version);
        v.extend(record);
    }
    v
}
pub fn empty(version: u32) -> Vec<u8> {
    let mut v = format!("gimp xcf v{version:03}\0").into_bytes();
    u32be(&mut v, 1);
    u32be(&mut v, 1);
    u32be(&mut v, 0);
    if version >= 4 {
        u32be(&mut v, 150);
    }
    prop(&mut v, 17, &[0]);
    prop(&mut v, 0, &[]);
    pointer(&mut v, 0, version);
    pointer(&mut v, 0, version);
    if version >= 18 {
        pointer(&mut v, 0, version);
    }
    v
}
