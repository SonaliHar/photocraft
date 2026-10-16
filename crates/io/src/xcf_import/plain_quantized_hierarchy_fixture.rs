//! DATA-ONLY synthetic public-XCF test input. No producer-authored or numerical
//! oracle claim, model ownership receipt, runtime helper or dependency on IO.
#![allow(dead_code)]
fn u32be(v: &mut Vec<u8>, n: u32) {
    v.extend_from_slice(&n.to_be_bytes());
}
fn pointer(v: &mut Vec<u8>, n: usize) {
    v.extend_from_slice(&(n as u64).to_be_bytes());
}
fn patch(v: &mut [u8], at: usize, n: usize) {
    v[at..at + 8].copy_from_slice(&(n as u64).to_be_bytes());
}
fn prop(v: &mut Vec<u8>, tag: u32, payload: &[u8]) {
    u32be(v, tag);
    u32be(v, payload.len() as u32);
    v.extend_from_slice(payload);
}
pub fn dimensions(layout: usize) -> (u32, u32) {
    assert!(layout < 3);
    match layout {
        0 => (8, 6),
        1 => (130, 65),
        _ => (257, 259),
    }
}
/// Native endian RGBA codes, unequalRGB and alpha0/fraction/max. External input.
pub fn native(depth: usize, layout: usize, leaf: usize) -> Vec<u8> {
    if depth >= 3 {
        return native_float(depth, layout, leaf);
    }
    if depth == 2 {
        return native_u32(layout, leaf);
    }
    assert!(depth < 2 && layout < 3 && leaf < 3);
    let (w, h) = dimensions(layout);
    let mut v = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let n = x + y * w + leaf as u32 * 13;
            let words = [
                (n * 997 + 17) & 65535,
                (n * 331 + 32769) & 65535,
                (n * 67 + 65535) & 65535,
                match n % 4 {
                    0 => 0,
                    1 => 16384,
                    2 => 32768,
                    _ => 65535,
                },
            ];
            for code in words {
                if depth == 0 {
                    v.push((code >> 8) as u8)
                } else {
                    v.extend_from_slice(&(code as u16).to_ne_bytes());
                }
            }
        }
    }
    v
}
/// U32-only additive seed. Adjacent first red words0x80000000/0x80000001
/// intentionally differ below normalizedF32 precision; old0/1 seeds unchanged.
pub fn words_u32(layout: usize, leaf: usize, x: u32, y: u32) -> [u32; 4] {
    assert!(layout < 3 && leaf < 3);
    let (w, h) = dimensions(layout);
    assert!(x < w && y < h);
    let n = x + y * w + leaf as u32 * 13;
    [
        0x80000000 | (n % 256),
        0x40000003 + 2 * (n % 31),
        0xc0000005 - 2 * (n % 19),
        match n % 4 {
            0 => 0,
            1 => u32::MAX,
            2 => 0x80000001,
            _ => 0xc0000001,
        },
    ]
}
fn native_u32(layout: usize, leaf: usize) -> Vec<u8> {
    let (w, h) = dimensions(layout);
    let mut v = Vec::new();
    for y in 0..h {
        for x in 0..w {
            for word in words_u32(layout, leaf, x, y) {
                v.extend_from_slice(&word.to_ne_bytes());
            }
        }
    }
    v
}
/// Additive finite-unit floating seeds. Raw words retain signed zero and bits
/// below F32 precision for F64. No producer/render oracle.
pub fn words_float(depth: usize, layout: usize, leaf: usize, x: u32, y: u32) -> [u64; 4] {
    assert!(matches!(depth, 3 | 4) && layout < 3 && leaf < 3);
    let (w, h) = dimensions(layout);
    assert!(x < w && y < h);
    let n = x + y * w + leaf as u32 * 13;
    if depth == 3 {
        [
            if n % 17 == 0 { 0x80000000 } else { 0x3e800001 + u64::from(n % 31) },
            0x3f000001 + u64::from(n % 23),
            0x3f400001 + u64::from(n % 19),
            match n % 5 {
                0 => 0x80000000,
                1 => 0,
                2 => 0x3e800001,
                3 => 0x3f000001,
                _ => 0x3f800000,
            },
        ]
    } else {
        [
            if n % 17 == 0 { 0x8000000000000000 } else { 0x3fd0000000000001 + u64::from(n % 31) },
            0x3fe0000000002001 + u64::from(n % 23),
            0x3fe8000000000001 + u64::from(n % 19),
            match n % 5 {
                0 => 0x8000000000000000,
                1 => 0,
                2 => 0x3fd0000000000001,
                3 => 0x3fe0000000000001,
                _ => 0x3ff0000000000000,
            },
        ]
    }
}
fn native_float(depth: usize, layout: usize, leaf: usize) -> Vec<u8> {
    let (w, h) = dimensions(layout);
    let mut v = Vec::new();
    for y in 0..h {
        for x in 0..w {
            for word in words_float(depth, layout, leaf, x, y) {
                if depth == 3 {
                    v.extend_from_slice(&(word as u32).to_ne_bytes());
                } else {
                    v.extend_from_slice(&word.to_ne_bytes());
                }
            }
        }
    }
    v
}
fn hierarchy(v: &mut Vec<u8>, depth: usize, layout: usize, leaf: usize) {
    let (w, h) = dimensions(layout);
    let word = if depth == 4 {
        8usize
    } else if matches!(depth, 2 | 3) {
        4usize
    } else if depth == 0 {
        1usize
    } else {
        2
    };
    let bpp = 4 * word;
    u32be(v, w);
    u32be(v, h);
    u32be(v, bpp as u32);
    let lp = v.len();
    pointer(v, 0);
    pointer(v, 0);
    let level = v.len();
    patch(v, lp, level);
    u32be(v, w);
    u32be(v, h);
    let tx = w.div_ceil(64);
    let ty = h.div_ceil(64);
    let p = v.len();
    for _ in 0..=tx * ty {
        pointer(v, 0);
    }
    let raw = native(depth, layout, leaf);
    for y in 0..ty {
        for x in 0..tx {
            let at = v.len();
            patch(v, p + ((y * tx + x) as usize) * 8, at);
            let tw = (w - x * 64).min(64);
            let th = (h - y * 64).min(64);
            for row in 0..th {
                let start = (((y * 64 + row) * w + x * 64) as usize) * bpp;
                let bytes = &raw[start..start + tw as usize * bpp];
                if word == 1 {
                    v.extend_from_slice(bytes);
                } else if word == 8 {
                    for c in bytes.chunks_exact(8) {
                        v.extend_from_slice(&u64::from_ne_bytes(c.try_into().unwrap()).to_be_bytes());
                    }
                } else if word == 4 {
                    for c in bytes.chunks_exact(4) {
                        v.extend_from_slice(&u32::from_ne_bytes(c.try_into().unwrap()).to_be_bytes());
                    }
                } else {
                    for c in bytes.chunks_exact(2) {
                        v.extend_from_slice(&u16::from_ne_bytes(c.try_into().unwrap()).to_be_bytes());
                    }
                }
            }
        }
    }
}
/// layout0 is exactly4records/firstgroup without nesting, exercising the paid
/// modern implicit priority partition. layout1 has nested+emptygroups.
pub fn image_version(depth: usize, layout: usize, version: u32) -> Vec<u8> {
    assert!(depth < 5 && layout < 3 && matches!(version, 12 | 13 | 22));
    let (w, h) = dimensions(layout);
    let rows: Vec<(&str, bool, Vec<u32>, Option<usize>)> = if layout == 0 {
        vec![
            ("Group", true, vec![], None),
            ("LeafA", false, vec![0, 0], Some(0)),
            ("LeafB", false, vec![0, 1], Some(1)),
            ("Background", false, vec![], Some(2)),
        ]
    } else {
        vec![
            ("Group", true, vec![], None),
            ("Inner", true, vec![0, 0], None),
            ("LeafA", false, vec![0, 0, 0], Some(0)),
            ("LeafB", false, vec![0, 0, 1], Some(1)),
            ("Empty", true, vec![0, 1], None),
            ("Background", false, vec![], Some(2)),
        ]
    };
    let mut v = format!("gimp xcf v{version:03}\0").into_bytes();
    u32be(&mut v, w);
    u32be(&mut v, h);
    u32be(&mut v, 0);
    u32be(
        &mut v,
        if depth == 4 {
            750
        } else if depth == 3 {
            650
        } else if depth == 2 {
            350
        } else if depth == 0 {
            150
        } else {
            250
        },
    );
    prop(&mut v, 17, &[0]);
    prop(&mut v, 0, &[]);
    let table = v.len();
    for _ in 0..=rows.len() {
        pointer(&mut v, 0);
    }
    pointer(&mut v, 0);
    if version >= 18 {
        pointer(&mut v, 0);
    }
    for (index, (name, group, path, leaf)) in rows.iter().enumerate() {
        let at = v.len();
        patch(&mut v, table + index * 8, at);
        let empty = *name == "Empty";
        u32be(&mut v, if empty { 1 } else { w });
        u32be(&mut v, if empty { 1 } else { h });
        u32be(&mut v, 1);
        u32be(&mut v, (name.len() + 1) as u32);
        v.extend_from_slice(name.as_bytes());
        v.push(0);
        prop(&mut v, 7, &28i32.to_be_bytes());
        prop(&mut v, 37, &0i32.to_be_bytes());
        prop(&mut v, 36, &(-1i32).to_be_bytes());
        prop(&mut v, 35, &(-1i32).to_be_bytes());
        let opacity = if *group {
            0.625f32
        } else if *name == "LeafA" {
            0.375
        } else {
            1.
        };
        prop(&mut v, 33, &opacity.to_bits().to_be_bytes());
        prop(&mut v, 8, &u32::from(*name != "LeafB").to_be_bytes());
        if *group {
            prop(&mut v, 29, &[]);
            prop(&mut v, 31, &1u32.to_be_bytes());
        }
        if !path.is_empty() {
            let bytes: Vec<u8> = path.iter().flat_map(|n| n.to_be_bytes()).collect();
            prop(&mut v, 30, &bytes);
        }
        prop(&mut v, 0, &[]);
        let hp = v.len();
        pointer(&mut v, 0);
        pointer(&mut v, 0);
        if version >= 20 {
            pointer(&mut v, 0);
        }
        if let Some(leaf) = leaf {
            let pos = v.len();
            patch(&mut v, hp, pos);
            hierarchy(&mut v, depth, layout, *leaf);
        }
    }
    v
}
pub fn image_layout(depth: usize, layout: usize) -> Vec<u8> {
    image_version(depth, layout, 12)
}
pub fn image(depth: usize) -> Vec<u8> {
    image_layout(depth, 1)
}

/// New painting layout; old image(depth) stays layout1.
pub fn image_paint(depth: usize) -> Vec<u8> {
    image_layout(depth, 2)
}
