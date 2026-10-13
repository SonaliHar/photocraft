//! Original unequal-channel RGB and gamma1.8 Gray ICC evidence profiles.
//! MIT OR Apache-2.0; public ICC/CMS definitions, no copied profile assets.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_cms::{Builtin, curve::Curve};
    let directory = std::path::PathBuf::from(std::env::args_os().nth(1).ok_or("private output directory required")?);
    std::fs::create_dir_all(&directory)?;
    let mut rgb = Builtin::Srgb.profile().clone();
    rgb.description = "Own Normal RGB unequal gamma1.6 2.3 2.8".into();
    rgb.copyright = "Original PhotoCraft corpus contributors; MIT OR Apache-2.0".into();
    rgb.trc = Some([Curve::Gamma(1.6), Curve::Gamma(2.3), Curve::Gamma(2.8)]);
    let mut gray = Builtin::GrayGamma22.profile().clone();
    gray.description = "Own Normal Gray gamma1.8".into();
    gray.copyright = rgb.copyright.clone();
    gray.trc = None;
    gray.gray_trc = Some(Curve::Gamma(1.8));
    for (name, profile) in [("own-rgb-channel-trc.icc", rgb), ("own-gray-channel-trc.icc", gray)] {
        let bytes = profile.with_encoded_bytes().to_bytes();
        photocraft_cms::Profile::parse(&bytes)?;
        std::fs::write(directory.join(name), bytes.as_slice())?;
    }
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
