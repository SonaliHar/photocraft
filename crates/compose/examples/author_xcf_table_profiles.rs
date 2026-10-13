//! Own sampled table-TRC RGB/Gray profiles for independent public GIMP evidence.
//! MIT OR Apache-2.0; no copied profile assets or implementation.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_cms::{Builtin, curve::Curve};
    let directory = std::path::PathBuf::from(std::env::args_os().nth(1).ok_or("private output directory required")?);
    std::fs::create_dir_all(&directory)?;
    let table = |gamma: f64| Curve::Table64((0..=4096).map(|index| (f64::from(index) / 4096.0).powf(gamma)).collect());
    let mut rgb = Builtin::Srgb.profile().clone();
    rgb.description = "Own Normal RGB sampled unequal TRCs 4097 nodes".into();
    rgb.copyright = "Original PhotoCraft corpus contributors; MIT OR Apache-2.0".into();
    rgb.trc = Some([table(1.6), table(2.3), table(2.8)]);
    let mut gray = Builtin::GrayGamma22.profile().clone();
    gray.description = "Own Normal Gray sampled gamma1.8 4097 nodes".into();
    gray.copyright = rgb.copyright.clone();
    gray.trc = None;
    gray.gray_trc = Some(table(1.8));
    for (name, profile) in [("own-rgb-table-trc.icc", rgb), ("own-gray-table-trc.icc", gray)] {
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
