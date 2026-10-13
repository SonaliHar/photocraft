//! Isolated scalar kernel: no image IO/CMS/quantization/compositing/allocations.
use photocraft_algo::gimp::alpha_clip_finite::AlphaClipFinite;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let op = AlphaClipFinite { clip_low: true, low_limit: 0.875, clip_high: true, high_limit: 0.125 };
    let begin = std::time::Instant::now();
    let mut sum = 0.;
    for i in 0..24_000_000u64 {
        let c = (i % 1024) as f64 / 1023.;
        let input = std::hint::black_box([c, 0.375 + 2_f64.powi(-40), 0.625, (i % 257) as f64 / 256.]);
        let mut out = [0.; 4];
        op.try_eval_pixel(&input, &mut out)?;
        sum += std::hint::black_box(out)[3];
    }
    println!(
        "24MP standalone validated AlphaClipFinite crossed RGBA kernel seconds={:.6} checksum={sum:.9}; excludes source/CMS/native encoding/output/scene/IO",
        begin.elapsed().as_secs_f64()
    );
    Ok(())
}
