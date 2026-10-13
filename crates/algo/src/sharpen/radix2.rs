//! A small in-place radix-2 FFT for the browser build.
//!
//! `rustfft` brings its whole planner (every radix, Bluestein, Rader) into the binary, and the
//! web build has a hard size limit, so the wasm build transforms with this instead and pads to
//! powers of two. Native builds use `rustfft` (SIMD, any smooth length); this module is also
//! compiled there so tests can check it against `rustfft` and run the whole deconvolution on it.

use rustfft::num_complex::Complex;

type C = Complex<f32>;

pub(super) struct Radix2 {
    n: usize,
    /// Bit-reversal permutation of `0..n`.
    rev: Vec<u32>,
    /// `exp(-2πi k / n)` for `k < n / 2`.
    twiddle: Vec<C>,
}

impl Radix2 {
    /// `n` must be a power of two (a length of 0 or 1 is the identity).
    pub(super) fn new(n: usize) -> Self {
        let n = n.max(1);
        debug_assert!(n.is_power_of_two());
        let bits = n.trailing_zeros();
        let rev = (0..n as u32).map(|i| if bits == 0 { 0 } else { i.reverse_bits() >> (32 - bits) }).collect();
        let twiddle = (0..n / 2)
            .map(|k| {
                let a = -2.0 * std::f64::consts::PI * k as f64 / n as f64;
                C::new(a.cos() as f32, a.sin() as f32)
            })
            .collect();
        Radix2 { n, rev, twiddle }
    }

    /// Transforms `row` (length `n`) in place, unnormalised. `inverse` uses the conjugate kernel.
    pub(super) fn process(&self, row: &mut [C], inverse: bool) {
        let n = self.n;
        if row.len() != n || n < 2 {
            return;
        }
        for (i, &j) in self.rev.iter().enumerate() {
            let j = j as usize;
            if i < j {
                row.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let (half, step) = (len / 2, n / len);
            for block in row.chunks_exact_mut(len) {
                let (lo, hi) = block.split_at_mut(half);
                for (j, (a, b)) in lo.iter_mut().zip(hi.iter_mut()).enumerate() {
                    let mut w = self.twiddle.get(j * step).copied().unwrap_or_default();
                    if inverse {
                        w = w.conj();
                    }
                    let t = *b * w;
                    *b = *a - t;
                    *a += t;
                }
            }
            len *= 2;
        }
    }
}
