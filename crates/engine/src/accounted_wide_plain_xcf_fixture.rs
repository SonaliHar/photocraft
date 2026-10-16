//! DATA ONLY: generated uncompressed public XCF, not a GIMP/runtime oracle.
//! Raw/encoded/expected buffers are external harness allocations, not model fees.
#![allow(dead_code)]
pub const WIDTH: u32 = 257;
pub const HEIGHT: u32 = 259;
pub fn precision(depth: usize) -> u32 {
    match depth {
        0 => 350,
        1 => 750,
        _ => panic!("closed wide fixture depth"),
    }
}
pub fn channels(gray: bool) -> usize {
    if gray { 2 } else { 4 }
}
/// Fixed finiteunit words include unequal RGB, integer lowbits, binary64 lowbits
/// and signed-zero color/alpha. No f32 conversion constructs these samples.
pub fn words(depth: usize, gray: bool, x: u32, y: u32) -> ([u64; 4], usize) {
    assert!(x < WIDTH && y < HEIGHT);
    let n = x + y * WIDTH;
    let (r, g, b, a) = match depth {
        0 => (
            0x80000001u64 + 2 * u64::from(n % 31),
            0x40000003 + 2 * u64::from(n % 17),
            0xc0000005 - 2 * u64::from(n % 19),
            match n % 8 {
                0 => 0,
                1 => u32::MAX as u64,
                2 => 0x80000001,
                _ => 0xc0000001,
            },
        ),
        1 => (
            if n % 13 == 0 { (-0f64).to_bits() } else { 0x3fd0000000000001 + u64::from(n % 11) },
            0x3fe0000000002001 + u64::from(n % 7),
            0x3fe8000000000001 + u64::from(n % 5),
            match n % 8 {
                0 => (-0f64).to_bits(),
                1 => 0,
                2 => 1f64.to_bits(),
                3 => 0x3fe8000000000001,
                _ => 0x3fe0000000000001,
            },
        ),
        _ => panic!("closed wide fixture depth"),
    };
    if gray { ([r, a, 0, 0], 2) } else { ([r, g, b, a], 4) }
}
pub fn native(gray: bool, depth: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let (w, n) = words(depth, gray, x, y);
            for word in w.into_iter().take(n) {
                if depth == 0 { v.extend_from_slice(&(word as u32).to_ne_bytes()) } else { v.extend_from_slice(&word.to_ne_bytes()) }
            }
        }
    }
    v
}
fn u32be(v: &mut Vec<u8>, n: u32) {
    v.extend_from_slice(&n.to_be_bytes());
}
fn ptr(v: &mut Vec<u8>, n: usize) {
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
pub fn image(gray: bool, depth: usize, alpha_lock: bool) -> Vec<u8> {
    let precision = precision(depth);
    let channels = channels(gray);
    let word = if depth == 0 { 4usize } else { 8 };
    let mut v = b"gimp xcf v013\0".to_vec();
    u32be(&mut v, WIDTH);
    u32be(&mut v, HEIGHT);
    u32be(&mut v, u32::from(gray));
    u32be(&mut v, precision);
    prop(&mut v, 17, &[0]);
    prop(&mut v, 0, &[]);
    let lp = v.len();
    ptr(&mut v, 0);
    ptr(&mut v, 0);
    ptr(&mut v, 0);
    let at = v.len();
    patch(&mut v, lp, at);
    u32be(&mut v, WIDTH);
    u32be(&mut v, HEIGHT);
    u32be(&mut v, if gray { 3 } else { 1 });
    let name = b"LegacyWide\0";
    u32be(&mut v, name.len() as u32);
    v.extend_from_slice(name);
    if alpha_lock {
        prop(&mut v, 10, &1u32.to_be_bytes());
    }
    prop(&mut v, 0, &[]);
    let hp = v.len();
    ptr(&mut v, 0);
    ptr(&mut v, 0);
    let at = v.len();
    patch(&mut v, hp, at);
    u32be(&mut v, WIDTH);
    u32be(&mut v, HEIGHT);
    u32be(&mut v, (channels * word) as u32);
    let levelp = v.len();
    ptr(&mut v, 0);
    ptr(&mut v, 0);
    let at = v.len();
    patch(&mut v, levelp, at);
    u32be(&mut v, WIDTH);
    u32be(&mut v, HEIGHT);
    let tx = WIDTH.div_ceil(64);
    let ty = HEIGHT.div_ceil(64);
    let pointers = v.len();
    for _ in 0..=tx * ty {
        ptr(&mut v, 0);
    }
    for y in 0..ty {
        for x in 0..tx {
            let at = v.len();
            patch(&mut v, pointers + ((y * tx + x) as usize) * 8, at);
            let tw = (WIDTH - x * 64).min(64);
            let th = (HEIGHT - y * 64).min(64);
            for row in 0..th {
                for column in 0..tw {
                    let (w, n) = words(depth, gray, x * 64 + column, y * 64 + row);
                    for bits in w.into_iter().take(n) {
                        if depth == 0 { v.extend_from_slice(&(bits as u32).to_be_bytes()) } else { v.extend_from_slice(&bits.to_be_bytes()) }
                    }
                }
            }
        }
    }
    v
}
