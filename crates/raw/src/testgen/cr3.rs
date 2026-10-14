//! ISO BMFF CR3 fixtures, with metadata independent of the JPEG.
use super::{TiffBuilder, Val};

pub fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    [((body.len() + 8) as u32).to_be_bytes().as_slice(), kind, body].concat()
}
fn full(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    bx(kind, &[&[0; 4][..], body].concat())
}
pub fn preview(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    with_metadata(jpeg, orientation, 1, "<x:xmpmeta xmlns:x='adobe:ns:meta/'/>")
}
pub fn with_metadata(jpeg: &[u8], orientation: u16, color: u16, xmp: &str) -> Vec<u8> {
    let tiff = |entries| {
        let mut t = TiffBuilder::default();
        let ifd = t.ifd(entries);
        t.chain = vec![ifd];
        t.build()
    };
    let cmt1 = tiff(vec![
        (271, Val::Ascii("Canon".into())),
        (272, Val::Ascii("Canon Synthetic CR3".into())),
        (274, Val::Short(vec![orientation])),
        (65000, Val::Undefined(vec![7, 9, 11, 13, 15])),
    ]);
    let cmt2 =
        tiff(vec![(34855, Val::Short(vec![800])), (33434, Val::Rational(vec![(1, 125)])), (40961, Val::Short(vec![if color == 1 { 1 } else { 65535 }]))]);
    let cmt3 = tiff(vec![(0xb4, Val::Short(vec![color])), (0x4001, Val::Short(vec![1, 2, 3, 4, 5]))]);
    let cmt4 = tiff(vec![(0, Val::Byte(vec![2, 3, 0, 0]))]);
    let mut metadata = vec![0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48];
    for (kind, body) in [(b"CMT1", cmt1), (b"CMT2", cmt2), (b"CMT3", cmt3), (b"CMT4", cmt4)] {
        metadata.extend(bx(kind, &body));
    }
    let mut xmp_body = vec![0xbe, 0x7a, 0xcf, 0xcb, 0x97, 0xa9, 0x42, 0xe8, 0x9c, 0x71, 0x99, 0x94, 0x91, 0xe3, 0xaf, 0xac];
    xmp_body.extend_from_slice(xmp.as_bytes());
    let ftyp = bx(b"ftyp", b"crx \0\0\0\x01crx isom");
    let movie = |offset: u64| {
        let mut entry = vec![0; 82];
        entry.extend(bx(b"JPEG", &[0; 4]));
        let stsd = full(b"stsd", &[1u32.to_be_bytes().as_slice(), &bx(b"CRAW", &entry)].concat());
        let stsc = full(b"stsc", &[1u32; 4].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>());
        let stsz = full(b"stsz", &[(jpeg.len() as u32).to_be_bytes().as_slice(), 1u32.to_be_bytes().as_slice()].concat());
        let co64 = full(b"co64", &[1u32.to_be_bytes().as_slice(), offset.to_be_bytes().as_slice()].concat());
        let track = bx(b"trak", &bx(b"mdia", &bx(b"minf", &bx(b"stbl", &[stsd, stsc, stsz, co64].concat()))));
        bx(b"moov", &[bx(b"uuid", &metadata), track].concat())
    };
    let m = movie(0);
    let offset = (ftyp.len() + m.len() + bx(b"uuid", &xmp_body).len() + 8) as u64;
    [ftyp, movie(offset), bx(b"uuid", &xmp_body), bx(b"mdat", jpeg)].concat()
}

/// An 8×4 lossless Bayer sensor with manually coded flat colour planes.
pub fn lossless(orientation: u16) -> Vec<u8> {
    let mut sample = Vec::new();
    let marker = |kind: u16, n: u32, flags: u32| {
        [kind.to_be_bytes().as_slice(), 8u16.to_be_bytes().as_slice(), n.to_be_bytes().as_slice(), flags.to_be_bytes().as_slice()].concat()
    };
    sample.extend(marker(0xff01, 64, 0));
    for p in 0..4 {
        sample.extend(marker(0xff02, 16, (p << 28) | 0x08000000));
        sample.extend(marker(0xff03, 16, 0x00200006));
    }
    sample.resize(112, 0);
    for p in 0..4u32 {
        // Flag 0, 41-zero escape + terminating 1 + 21-bit signed symbol;
        // three zero residuals (100/10/1), then a four-sample run (1111).
        let value = 200 * (p + 1);
        sample.extend([0, 0, 0, 0, 0, 0x20, (value >> 8) as u8, value as u8, 0x97, 0xc0, 0, 0, 0, 0, 0, 0]);
    }
    sensor(orientation, 0x100, 8, 4, 0, 112, &sample)
}

/// Independently encoded zero wavelet coefficients in a 64×48 C-RAW frame.
pub fn craw(orientation: u16) -> Vec<u8> {
    let marker = |kind: u16, size: usize, flags: u32| {
        [kind.to_be_bytes().as_slice(), 8u16.to_be_bytes().as_slice(), (size as u32).to_be_bytes().as_slice(), flags.to_be_bytes().as_slice()].concat()
    };
    let mut bands = Vec::new();
    for b in 0..10 {
        let (w, h) = if b < 4 {
            (4, 3)
        } else if b < 7 {
            (8, 6)
        } else {
            (16, 12)
        };
        let j_table = [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let mut state = 0;
        let mut bits = Vec::new();
        // Each one bit represents a separate MEL state transition.
        #[allow(clippy::same_item_push)]
        for _ in 0..h {
            bits.push(true);
            let mut n = 1;
            loop {
                bits.push(true);
                n += 1usize << j_table[state];
                if n > w {
                    break;
                }
                state = (state + 1).min(31);
                if n == w {
                    break;
                }
            }
        }
        let mut bytes = vec![0; bits.len().div_ceil(8)];
        for (i, value) in bits.into_iter().enumerate() {
            if value {
                bytes[i / 8] |= 1 << (7 - i % 8);
            }
        }
        bands.push(bytes);
    }
    let size: usize = bands.iter().map(Vec::len).sum();
    let mut sample = marker(0xff01, size * 4, 0);
    for p in 0..4 {
        sample.extend(marker(0xff02, size, (p << 28) | 0x08000000));
        for (b, data) in bands.iter().enumerate() {
            sample.extend(marker(0xff03, data.len(), ((b as u32) << 28) | (4 << 19)));
        }
    }
    let header_size = sample.len().div_ceil(8) * 8;
    sample.resize(header_size, 0);
    for _ in 0..4 {
        for data in &bands {
            sample.extend(data);
        }
    }
    sensor(orientation, 0x100, 64, 48, 3, header_size, &sample)
}

struct Writer {
    bits: Vec<bool>,
}
impl Writer {
    fn put(&mut self, v: u32, n: u32) {
        for shift in (0..n).rev() {
            self.bits.push((v >> shift) & 1 != 0);
        }
    }
    fn rice(&mut self, v: u32, k: u32, escape: u32, precision: u32) {
        let zeros = v >> k;
        if zeros >= escape {
            self.put(0, escape);
            self.put(1, 1);
            self.put(v, precision);
        } else {
            self.put(0, zeros);
            self.put(1, 1);
            self.put(v & ((1 << k) - 1), k);
        }
    }
    fn finish(self) -> Vec<u8> {
        let mut bytes = vec![0; self.bits.len().div_ceil(8)];
        for (i, bit) in self.bits.into_iter().enumerate() {
            if bit {
                bytes[i / 8] |= 1 << (7 - i % 8);
            }
        }
        bytes
    }
}
fn rice_next(k: u32, v: u32) -> u32 {
    let scaled = v >> k;
    (k as i32 + i32::from(scaled > 2) + i32::from(scaled > 5) - i32::from(v < (1 << k.saturating_sub(1)))).clamp(0, 20) as u32
}
fn zero_band(width: usize, height: usize) -> Vec<u8> {
    let j = [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    let mut state = 0;
    let mut writer = Writer { bits: Vec::new() };
    for _ in 0..height {
        writer.put(1, 1);
        let mut count = 1;
        loop {
            writer.put(1, 1);
            count += 1usize << j[state];
            if count > width {
                break;
            }
            state = (state + 1).min(31);
            if count == width {
                break;
            }
        }
    }
    writer.finish()
}
/// Adaptive 64×48 C-RAW: alternating QP blocks and a nonzero HL1 band.
pub fn adaptive(orientation: u16) -> Vec<u8> {
    let mut qp_writer = Writer { bits: Vec::new() };
    let mut prev = [0i32; 4];
    let mut row = prev;
    let mut k = 0;
    for y in 0..12 {
        for (x, v) in row.iter_mut().enumerate() {
            *v = if (x / 2 + y) % 2 == 0 { 22 } else { 26 };
        }
        for x in 0..4 {
            let a = if x == 0 { prev[0] } else { row[x - 1] };
            let b = prev[x];
            let c = if x == 0 { b } else { prev[x - 1] };
            let d = prev[(x + 1).min(3)];
            let prediction = if y == 0 {
                a
            } else if c >= a.max(b) {
                a.min(b)
            } else if c <= a.min(b) {
                a.max(b)
            } else {
                a + b - c
            };
            let diff = row[x] - prediction;
            let mapped = if diff < 0 { (-diff as u32) * 2 - 1 } else { diff as u32 * 2 };
            qp_writer.rice(mapped, k, 23, 8);
            k = rice_next(k, if y == 0 || x == 3 { mapped } else { mapped / 2 + (d - b).unsigned_abs() });
        }
        prev = row;
    }
    let qp = qp_writer.finish();
    let mut bands = Vec::new();
    for b in 0..10 {
        let (w, h) = if b < 4 {
            (4, 3)
        } else if b < 7 {
            (8, 6)
        } else {
            (16, 12)
        };
        if b == 7 {
            let mut writer = Writer { bits: Vec::new() };
            let mut k = 0;
            for y in 0..h {
                for x in 0..w {
                    let first = x == 0 && y == 0;
                    if first {
                        writer.put(0, 1);
                    }
                    let symbol = if first { 3 } else { 4 };
                    writer.rice(symbol, k, 41, 21);
                    k = rice_next(k, symbol);
                }
            }
            bands.push(writer.finish());
        } else {
            bands.push(zero_band(w, h));
        }
    }
    let size: usize = bands.iter().map(Vec::len).sum();
    let marker = |kind: u16, size: usize, fields: &[u8]| {
        [kind.to_be_bytes().as_slice(), ((fields.len() + 4) as u16).to_be_bytes().as_slice(), (size as u32).to_be_bytes().as_slice(), fields].concat()
    };
    let mut sample = marker(0xff11, qp.len() + size * 4, &[0x4000u32.to_be_bytes().as_slice(), (qp.len() as u32).to_be_bytes().as_slice(), &[0; 4]].concat());
    for p in 0..4 {
        sample.extend(marker(0xff12, size, &((p as u32) << 28 | 0x08000000).to_be_bytes()));
        for (b, data) in bands.iter().enumerate() {
            let delta = if b < 4 {
                0
            } else if b < 6 {
                4
            } else if b < 9 {
                8
            } else {
                16
            };
            sample.extend(marker(
                0xff13,
                data.len(),
                &[((b as u32) << 28 | delta).to_be_bytes().as_slice(), u32::from(b < 4).to_be_bytes().as_slice(), &[0; 4]].concat(),
            ));
        }
    }
    let header_size = sample.len().div_ceil(8) * 8;
    sample.resize(header_size, 0);
    sample.extend(qp);
    for _ in 0..4 {
        for data in &bands {
            sample.extend(data);
        }
    }
    sensor(orientation, 0x200, 64, 48, 3, header_size, &sample)
}

#[allow(clippy::too_many_arguments)] // Fixture container fields mirror CMP1.
fn sensor(orientation: u16, version: u16, width: u16, height: u16, levels: u8, header_size: usize, sample: &[u8]) -> Vec<u8> {
    let base = preview(&[], orientation);
    let ftyp = bx(b"ftyp", b"crx \0\0\0\x01crx isom");
    let meta_start = ftyp.len() + 8;
    let size = u32::from_be_bytes(base[meta_start..meta_start + 4].try_into().unwrap_or([0; 4])) as usize;
    let meta = &base[meta_start..meta_start + size];
    let movie = |offset: u64| {
        let mut cmp = vec![0; 32];
        cmp[4..6].copy_from_slice(&version.to_be_bytes());
        for (at, v) in [(8, u32::from(width)), (12, u32::from(height)), (16, u32::from(width)), (20, u32::from(height)), (28, header_size as u32)] {
            cmp[at..at + 4].copy_from_slice(&v.to_be_bytes());
        }
        cmp[24] = 14;
        cmp[25] = 0x40;
        cmp[26] = levels;
        let mut dims = vec![0; 48];
        for (at, v) in [(4, width), (6, height), (8, 1), (10, 2), (12, 1), (20, width - 1), (22, height - 1), (44, width - 1), (46, height - 1)] {
            dims[at..at + 2].copy_from_slice(&v.to_be_bytes());
        }
        let mut entry = vec![0; 82];
        entry.extend(bx(b"CMP1", &cmp));
        entry.extend(full(b"CDI1", &bx(b"IAD1", &dims)));
        let stsd = full(b"stsd", &[1u32.to_be_bytes().as_slice(), &bx(b"CRAW", &entry)].concat());
        let stsc = full(b"stsc", &[1u32; 4].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>());
        let stsz = full(b"stsz", &[(sample.len() as u32).to_be_bytes().as_slice(), 1u32.to_be_bytes().as_slice()].concat());
        let co64 = full(b"co64", &[1u32.to_be_bytes().as_slice(), offset.to_be_bytes().as_slice()].concat());
        let mut tkhd = vec![0; 16];
        tkhd[12..16].copy_from_slice(&3u32.to_be_bytes());
        let track = bx(b"trak", &[bx(b"tkhd", &tkhd), bx(b"mdia", &bx(b"minf", &bx(b"stbl", &[stsd, stsc, stsz, co64].concat())))].concat());
        let ptr = bx(b"CCDT", &[0u64.to_be_bytes().as_slice(), 0u32.to_be_bytes().as_slice(), 3u32.to_be_bytes().as_slice()].concat());
        let cctp = full(b"CCTP", &[1u32.to_be_bytes().as_slice(), 1u32.to_be_bytes().as_slice(), &ptr].concat());
        let mut values = vec![0; 798];
        values[0] = 16;
        values[71..75].fill(1024);
        values[329..333].fill(64);
        values[797] = 16383;
        let mut t = TiffBuilder::default();
        let id = t.ifd(vec![(0x4001, Val::Short(values))]);
        t.chain = vec![id];
        let cmt3 = bx(b"CMT3", &t.build());
        let uuid = bx(b"uuid", &[&[0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48][..], &cctp, &cmt3].concat());
        bx(b"moov", &[meta.to_vec(), uuid, track].concat())
    };
    let offset = (ftyp.len() + movie(0).len() + 8) as u64;
    [ftyp, movie(offset), bx(b"mdat", sample)].concat()
}
