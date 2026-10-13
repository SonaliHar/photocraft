//! Throughput check: `cargo test -p photocraft-cms --release --test bench -- --ignored --nocapture`.

use photocraft_cms::{Builtin, Intent, Transform};

/// Stream a 24 MP RGB workload through the same cached table, avoiding image allocation in
/// the timing. This measures the old f32 reference against the new f64 interpolation path.
#[test]
#[ignore]
fn bench_wide_rgb_clut_24mp() {
    use photocraft_cms::Clut;
    use std::hint::black_box;
    let clut = Clut::sample(vec![17; 3], 3, |input, output| {
        output.copy_from_slice(input);
    });
    let pixels = 24_000_000;
    let mut f32_out = [0.0; 3];
    let begin = std::time::Instant::now();
    for i in 0..pixels {
        let input = [((i % 997) as f32 + 0.5) / 997.0, 0.375, 0.875];
        clut.eval(black_box(&input), black_box(&mut f32_out));
    }
    let old = begin.elapsed();
    let mut f64_out = [0.0; 3];
    let begin = std::time::Instant::now();
    for i in 0..pixels {
        let input = [((i % 997) as f64 + 0.5) / 997.0, 0.375, 0.875];
        clut.try_eval_f64(black_box(&input), black_box(&mut f64_out)).unwrap();
    }
    let wide = begin.elapsed();
    eprintln!("24 MP RGB CLUT f32={old:?}, checked f64={wide:?}, ratio={:.3}", wide.as_secs_f64() / old.as_secs_f64());
    assert!(f64_out.iter().all(|value| value.is_finite()));
}

#[test]
#[ignore]
fn bench_srgb8_to_cmyk8() {
    let t0 = std::time::Instant::now();
    let t = Transform::new(Builtin::Srgb.profile(), Builtin::CoatedCmyk.profile(), Intent::RelativeColorimetric, true).unwrap();
    eprintln!("build: {:?}", t0.elapsed());
    let n = 6016 * 6016;
    let mut src = vec![0u8; n * 4];
    let mut x = 1u32;
    for v in src.iter_mut() {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *v = x as u8;
    }
    let mut dst = vec![0u8; n * 5];
    let t1 = std::time::Instant::now();
    t.convert_u8(&src, 4, &mut dst, 5, true);
    eprintln!("convert 6016² RGBA8 → CMYKA8 (one buffer): {:?}", t1.elapsed());
    let t2 = std::time::Instant::now();
    for (s, d) in src.chunks(65536 * 4).zip(dst.chunks_mut(65536 * 5)) {
        t.convert_u8(s, 4, d, 5, true);
    }
    eprintln!("convert per 256² tile: {:?}", t2.elapsed());
}
