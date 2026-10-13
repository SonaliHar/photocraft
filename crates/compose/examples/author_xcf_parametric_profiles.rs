//! Original piecewise ICC type-4 curves for public GIMP observation.
//! MIT OR Apache-2.0; no copied profile assets or implementation.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_cms::{Builtin, curve::Curve};
    let directory = std::path::PathBuf::from(std::env::args_os().nth(1).ok_or("private output directory required")?);
    std::fs::create_dir_all(&directory)?;
    let piecewise = |gamma: f64, breakpoint: f64| {
        // Author directly in ICC s15Fixed16 representable parameters. A rounded-down
        // linear slope makes the join monotone; the small upward jump is retained.
        let g = (gamma * 65536.0).round() / 65536.0;
        let d = (breakpoint * 65536.0).round() / 65536.0;
        let c = (d.powf(g - 1.0) * 65536.0).floor() / 65536.0;
        Curve::Parametric { kind: 4, p: [g, 1.0, 0.0, c, d, 0.0, 0.0] }
    };
    let mut rgb = Builtin::Srgb.profile().clone();
    rgb.description = "Own Normal RGB unequal piecewise ICC type4 TRCs".into();
    rgb.copyright = "Original PhotoCraft corpus contributors; MIT OR Apache-2.0".into();
    rgb.trc = Some([piecewise(1.6, 0.015), piecewise(2.3, 0.04), piecewise(2.8, 0.07)]);
    let mut gray = Builtin::GrayGamma22.profile().clone();
    gray.description = "Own Normal Gray piecewise ICC type4 kTRC".into();
    gray.copyright = rgb.copyright.clone();
    gray.trc = None;
    gray.gray_trc = Some(piecewise(1.8, 0.025));
    for (name, profile) in [("own-rgb-parametric-trc.icc", rgb), ("own-gray-parametric-trc.icc", gray)] {
        let channels = profile.channels();
        let bytes = profile.with_encoded_bytes().to_bytes();
        let parsed = photocraft_cms::Profile::parse(&bytes)?;
        if parsed.channels() != channels {
            return Err("authored profile model changed".into());
        }
        std::fs::write(directory.join(name), bytes.as_slice())?;
    }
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
