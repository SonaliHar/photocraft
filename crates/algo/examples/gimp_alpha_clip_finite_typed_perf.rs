//! Isolated scalar kernel: no image IO/CMS/quantization/compositing/allocations.
use photocraft_algo::gimp::registry::{WorkingEncoding, alpha_clip_finite_member};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (_, op) = alpha_clip_finite_member(Some("0:0"), true, 0.875, true, 0.125, WorkingEncoding::Linear)?;
    let prepared = op.prepare()?;
    let begin = std::time::Instant::now();
    let mut sum = 0.;
    for i in 0..24_000_000u64 {
        let c = (i % 1024) as f64 / 1023.;
        let input = std::hint::black_box([c, 0.375 + 2_f64.powi(-40), 0.625, (i % 257) as f64 / 256.]);
        let mut out = [0.; 4];
        prepared.try_eval_pixel(&input, 3, &mut out)?;
        sum += std::hint::black_box(out)[3];
    }
    println!(
        "24MP typed prepared AlphaClipFinite crossed RGBA kernel seconds={:.6} checksum={sum:.9}; excludes source/CMS/native encoding/output/scene/IO",
        begin.elapsed().as_secs_f64()
    );
    Ok(())
}
