//! SHA-256 (FIPS 180-4), for verifying downloaded test corpora without
//! depending on `sha256sum`/`shasum` being installed.

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74,
    0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d,
    0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e,
    0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Lower-case hex SHA-256 of `data`.
pub fn hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.as_chunks::<64>().0 {
        compress(&mut h, block);
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

/// Hash a reader without retaining its contents (large corpus/native evidence).
/// The cap is enforced even if the file grows after metadata inspection.
pub fn reader_hex(mut reader: impl std::io::Read, max_bytes: u64) -> Result<String, String> {
    let mut state: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut block = [0u8; 64];
    let mut used = 0usize;
    let mut total = 0u64;
    let mut input = [0u8; 64 * 1024];
    loop {
        let n = match reader.read(&mut input) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|e| format!("SHA256 read: {e}"))?,
        };
        if n == 0 {
            break;
        }
        total = total.checked_add(n as u64).ok_or("SHA256 length overflow")?;
        if total > max_bytes {
            return Err(format!("evidence exceeds {max_bytes} byte limit"));
        }
        let mut chunk = input.get(..n).ok_or("invalid SHA256 reader length")?;
        while !chunk.is_empty() {
            let take = chunk.len().min(64usize.saturating_sub(used));
            let end = used.checked_add(take).ok_or("SHA256 block length overflow")?;
            block.get_mut(used..end).ok_or("invalid SHA256 block position")?.copy_from_slice(chunk.get(..take).ok_or("invalid SHA256 chunk")?);
            chunk = chunk.get(take..).ok_or("invalid SHA256 remaining chunk")?;
            used = end;
            if used == 64 {
                compress(&mut state, &block);
                used = 0;
            }
        }
    }
    let bits = total.checked_mul(8).ok_or("SHA256 bit length overflow")?;
    *block.get_mut(used).ok_or("invalid SHA256 final position")? = 0x80;
    used += 1;
    block.get_mut(used..).ok_or("invalid SHA256 padding")?.fill(0);
    if used > 56 {
        compress(&mut state, &block);
        block.fill(0);
    }
    block.get_mut(56..).ok_or("invalid SHA256 length field")?.copy_from_slice(&bits.to_be_bytes());
    compress(&mut state, &block);
    Ok(state.iter().map(|v| format!("{v:08x}")).collect())
}

fn compress(h: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (slot, word) in w.iter_mut().zip(block.as_chunks::<4>().0.iter()) {
        *slot = u32::from_be_bytes(*word);
    }
    // All indices are bounded by the constant schedule/round lengths, never input data.
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
    for (k, word) in K.iter().zip(w.iter()) {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ (!e & g);
        let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(*k).wrapping_add(*word);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let t2 = s0.wrapping_add((a & b) ^ (a & c) ^ (b & c));
        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (x, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
        *x = x.wrapping_add(value);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn streaming_vectors_boundaries_and_reader_failures() {
        use std::io::{self, Read};
        struct Short<'a>(&'a [u8]);
        impl Read for Short<'_> {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                let n = out.len().min(self.0.len()).min(3);
                out[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }
        for length in [0, 1, 55, 56, 63, 64, 65, 127, 128, 1000, 131_072] {
            let bytes = vec![b'a'; length];
            assert_eq!(super::reader_hex(Short(&bytes), length as u64).unwrap(), super::hex(&bytes));
        }
        assert!(super::reader_hex(&b"abc"[..], 2).is_err());
        assert_eq!(super::reader_hex(&b""[..], 0).unwrap(), super::hex(b""));
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("failed"))
            }
        }
        assert!(super::reader_hex(Broken, 64).is_err());
    }
    #[test]
    fn known_vectors() {
        assert_eq!(super::hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(super::hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(super::hex(long), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
        assert_eq!(super::hex(&[b'a'; 1000]), "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3");
    }
}
