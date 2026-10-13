//! Release 24 MP point-kernel throughput; excludes storage and ICC transforms.
use photocraft_algo::gimp::{
    curves::StoredCurvesConfiguration,
    levels::{LevelsChannel, LevelsConfiguration},
};
use std::{hint::black_box, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tables: [[f64; 256]; 5] = std::array::from_fn(|channel| std::array::from_fn(|i| (i as f64 / 255.0).powf(0.65 + 0.16 * channel as f64) * 0.95 + 0.025));
    let curves = StoredCurvesConfiguration { tables: std::array::from_fn(|i| tables.get(i).map(|v| v.as_slice()).unwrap_or(&[])) }.prepare()?;
    let levels = LevelsConfiguration {
        channels: [LevelsChannel { low_input: 0.03, high_input: 0.94, gamma: 0.75, low_output: 0.02, high_output: 0.98 }; 5],
        clamp_input: false,
        clamp_output: false,
    };
    for curve in [true, false] {
        let start = Instant::now();
        let mut out = [0.0; 4];
        for i in 0..24_000_000_u32 {
            let x = f64::from(i % 1024) / 1023.0;
            let input = black_box([x, x * 0.7, x * 0.4, 0.5]);
            if curve {
                curves.try_eval_pixel(&input, 3, &mut out)?;
            } else {
                levels.try_eval_pixel(&input, 3, &mut out)?;
            }
            black_box(out);
        }
        println!(
            "24 MP RGBA f64 {} kernel: {:.3} ms (storage/CMS excluded)",
            if curve { "stored-curves256" } else { "repeated-levels" },
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
    Ok(())
}
