//! Checked alpha-kernel throughput; storage and profile work are excluded.
use photocraft_algo::gimp::threshold_alpha::ThresholdAlpha;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    for channels in [1, 3] {
        let operation = ThresholdAlpha { value: 0.5 };
        operation.validate()?;
        let started = std::time::Instant::now();
        let mut checksum = 0.0;
        for i in 0..24_000_000_u32 {
            let value = f64::from(i % 1024) / 1023.0;
            let input = if channels == 1 { [value, 1.0 - value, 0.0, 0.0] } else { [value, 1.0 - value, value * 0.5, 1.0 - value] };
            let input = std::hint::black_box(input);
            let mut output = [0.0; 4];
            operation.try_eval_pixel(input.get(..channels + 1).ok_or("input channels")?, channels, output.get_mut(..channels + 1).ok_or("output channels")?)?;
            checksum += std::hint::black_box(output).into_iter().sum::<f64>();
        }
        println!(
            "24MP F64 Alpha Threshold colors={channels}: {:.3}ms; checksum {checksum:.3} (storage/CMS excluded)",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    Ok(())
}
