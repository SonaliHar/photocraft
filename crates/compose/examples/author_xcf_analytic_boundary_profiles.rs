//! Original class-boundary ICC holdouts, MIT OR Apache-2.0.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_cms::{Builtin, curve::Curve, xcf_color::qualify_analytic_profile};
    let directory = std::path::PathBuf::from(std::env::args_os().nth(1).ok_or("private output directory required")?);
    let piecewise = |kind, gamma: f64, d: f64| {
        let gamma = (gamma * 65536.0).round() / 65536.0;
        let c = ((d.powf(gamma) / d) * 65536.0).floor() / 65536.0;
        Curve::Parametric { kind, p: [gamma, 1.0, 0.0, c, d, 0.0, 0.0] }
    };
    for variant in ["edge-gamma", "edge-piecewise"] {
        let mut rgb = Builtin::Srgb.profile().clone();
        rgb.description = format!("Own analytic boundary {variant} RGB");
        rgb.copyright = "Original PhotoCraft corpus contributors; MIT OR Apache-2.0".into();
        rgb.matrix = Some([[0.56, 0.27, 0.1342], [0.24, 0.68, 0.08], [0.01, 0.14, 0.6749]]);
        rgb.trc = Some(if variant == "edge-gamma" {
            [Curve::Gamma(0.75), Curve::Gamma(3.5), Curve::Identity]
        } else {
            [piecewise(3, 0.75, 0.25), piecewise(4, 3.5, 0.25), piecewise(4, 1.25, 0.125)]
        });
        let mut gray = Builtin::GrayGamma22.profile().clone();
        gray.description = format!("Own analytic boundary {variant} Gray");
        gray.copyright = rgb.copyright.clone();
        gray.trc = None;
        gray.gray_trc = Some(if variant == "edge-gamma" { Curve::Gamma(3.5) } else { piecewise(3, 0.75, 0.25) });
        let folder = directory.join(variant);
        std::fs::create_dir_all(&folder)?;
        for (model, definition) in [("rgb", rgb), ("gray", gray)] {
            let bytes = definition.with_encoded_bytes().to_bytes();
            let profile = photocraft_cms::Profile::parse(&bytes)?;
            qualify_analytic_profile(&profile)?;
            std::fs::write(folder.join(format!("own-{model}-parametric-trc.icc")), bytes.as_slice())?;
        }
    }
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
