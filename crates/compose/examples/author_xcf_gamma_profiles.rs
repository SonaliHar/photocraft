//! Generate original gamma 2.2 corpus profiles from PhotoCraft's public-spec CMS definitions.
//! No third-party ICC assets. Usage: cargo run -p photocraft-compose --example author_xcf_gamma_profiles -- <private packet directory>
#[cfg(not(target_arch = "wasm32"))]
use photocraft_cms::{Builtin, curve::Curve};
#[cfg(not(target_arch = "wasm32"))]
use std::{error::Error, path::PathBuf};
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn Error>> {
    let directory = PathBuf::from(std::env::args_os().nth(1).ok_or("private output directory required")?);
    std::fs::create_dir_all(&directory)?;
    let mut rgb = Builtin::Srgb.profile().clone();
    rgb.description = "Own Normal mask RGB gamma 2.2".into();
    rgb.copyright = "Original PhotoCraft corpus contributors; MIT OR Apache-2.0".into();
    rgb.trc = Some([Curve::Gamma(2.2), Curve::Gamma(2.2), Curve::Gamma(2.2)]);
    let mut gray = Builtin::GrayGamma22.profile().clone();
    gray.description = "Own Normal mask Gray gamma 2.2".into();
    gray.copyright = rgb.copyright.clone();
    for (name, profile) in [("own-rgb-gamma22.icc", rgb), ("own-gray-gamma22.icc", gray)] {
        let bytes = profile.with_encoded_bytes().to_bytes();
        photocraft_cms::Profile::parse(&bytes)?;
        std::fs::write(directory.join(name), bytes.as_slice())?;
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
