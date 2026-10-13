//! Release 24 MP checked tonal-kernel throughput; storage and ICC work excluded.
use photocraft_algo::gimp::color_balance::ColorBalance;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for preserve_luminosity in [false, true] {
        let config = ColorBalance { ranges: [[0.125, -0.25, 0.375], [0.18, -0.12, 0.23], [-0.0625, 0.3125, -0.1875]], preserve_luminosity };
        config.validate()?;
        let started = std::time::Instant::now();
        let mut checksum = 0.0;
        for i in 0..24_000_000_u32 {
            let value = f64::from(i % 1024) / 1023.0;
            let input = std::hint::black_box([value, 1.0 - value, value * 0.5, 0.75]);
            let mut output = [0.0; 4];
            config.try_eval_pixel(&input, &mut output)?;
            checksum += std::hint::black_box(output).into_iter().sum::<f64>();
        }
        println!(
            "24MP RGBA F64 Color Balance preserve={preserve_luminosity}: {:.3}ms; checksum {checksum:.3} (storage/CMS excluded)",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    Ok(())
}
