//! Original analytic ICC holdouts, MIT OR Apache-2.0. No copied profile assets.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_cms::{Builtin, curve::Curve, xcf_color::qualify_analytic_profile};
    let directory = std::path::PathBuf::from(std::env::args_os().nth(1).ok_or("private output directory required")?);
    std::fs::create_dir_all(&directory)?;
    let fixed = |value: f64| (value * 65536.0).round() / 65536.0;
    let piecewise = |kind, gamma: f64, a: f64, d: f64| {
        let (gamma, a, d) = (fixed(gamma), fixed(a), fixed(d));
        let b = 1.0 - a;
        let c = (((a * d + b).powf(gamma) / d) * 65536.0).floor() / 65536.0;
        Curve::Parametric { kind, p: [gamma, a, b, c, d, 0.0, 0.0] }
    };
    for variant in ["gamma", "type3", "type4"] {
        let mut rgb = Builtin::Srgb.profile().clone();
        rgb.description = format!("Own analytic class {variant} RGB holdout");
        rgb.copyright = "Original PhotoCraft corpus contributors; MIT OR Apache-2.0".into();
        rgb.matrix = Some(match variant {
            "gamma" => [[0.52, 0.32, 0.1242], [0.27, 0.65, 0.08], [0.03, 0.10, 0.6949]],
            "type3" => [[0.60, 0.22, 0.1442], [0.30, 0.60, 0.10], [0.02, 0.12, 0.6849]],
            _ => [[0.57, 0.29, 0.1042], [0.25, 0.67, 0.08], [0.04, 0.09, 0.6949]],
        });
        rgb.trc = Some(match variant {
            "gamma" => [Curve::Gamma(1.1), Curve::Gamma(1.7), Curve::Gamma(2.6)],
            "type3" => [piecewise(3, 1.5, 0.95, 0.035), piecewise(3, 2.2, 0.95, 0.045), piecewise(3, 3.0, 0.95, 0.055)],
            _ => [piecewise(4, 1.9, 1.01, 0.08), piecewise(4, 2.7, 1.01, 0.10), piecewise(4, 3.2, 1.01, 0.12)],
        });
        let mut gray = Builtin::GrayGamma22.profile().clone();
        gray.description = format!("Own analytic class {variant} Gray holdout");
        gray.copyright = rgb.copyright.clone();
        gray.trc = None;
        gray.gray_trc = Some(match variant {
            "gamma" => Curve::Gamma(1.35),
            "type3" => piecewise(3, 2.15, 0.94, 0.05),
            _ => piecewise(4, 2.5, 1.02, 0.07),
        });
        for (model, definition) in [("rgb", rgb), ("gray", gray)] {
            let bytes = definition.with_encoded_bytes().to_bytes();
            let parsed = photocraft_cms::Profile::parse(&bytes)?;
            qualify_analytic_profile(&parsed)?;
            let folder = directory.join(variant);
            std::fs::create_dir_all(&folder)?;
            std::fs::write(folder.join(format!("own-{model}-parametric-trc.icc")), bytes.as_slice())?;
        }
    }
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
