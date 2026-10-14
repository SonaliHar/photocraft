//! Inspect CR3 track headers/metadata and reframe HEVC previews, without an external RAW decoder.
use photocraft_raw::{Limits, cr3};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for filename in std::env::args_os().skip(1) {
        let path = std::path::PathBuf::from(filename);
        let bytes = std::fs::read(&path)?;
        if let Some(body) = cr3::metadata(&bytes).timed_maker {
            std::fs::write(path.with_extension("timed.tiff"), body)?;
        }
        if let Some(body) = cr3::metadata(&bytes).timed {
            std::fs::write(path.with_extension("ctmd.bin"), body)?;
        }
        for (i, body) in cr3::metadata(&bytes).blocks.iter().enumerate() {
            if let Some(body) = body {
                std::fs::write(path.with_extension(format!("cmt{}.tiff", i + 1)), body)?;
            }
        }
        if let Ok(track) = cr3::sensor_track(&bytes, &Limits::default()) {
            println!(
                "  primary track {} {}x{}, tile {}x{}, version {:x}, {} bit, levels {}, header {}",
                track.id, track.width, track.height, track.tile_width, track.tile_height, track.version, track.bits, track.wavelet_levels, track.header_size
            );
            std::fs::write(path.with_extension("sensor.crx"), track.sample)?;
            println!("  sensor header {:02x?}", track.sample.iter().take(track.header_size as usize).copied().collect::<Vec<_>>());
            println!("  sensor entropy prefix {:02x?}", track.sample.iter().skip(track.header_size as usize).take(32).copied().collect::<Vec<_>>());
            println!("  dimensions {:02x?}", track.dimensions);
        }
        let previews = cr3::hevc_previews(&bytes);
        println!("{}: {} HEVC previews", path.display(), previews.len());
        for (i, p) in previews.iter().enumerate() {
            println!("  {}x{}, {}x{} tiles {}x{}, {} bit, colour {:?}", p.width, p.height, p.tile_width, p.tile_height, p.columns, p.rows, p.bits, p.color);
            println!("  sample prefix {:02x?}", p.sample.iter().take(32).copied().collect::<Vec<_>>());
            let heif = p.to_heif(&Limits::default())?;
            std::fs::write(path.with_extension(format!("preview-{i}.heic")), heif)?;
        }
        if let Some(exif) = cr3::metadata(&bytes).exif(&Limits::default())? {
            std::fs::write(path.with_extension("exif.tiff"), exif)?;
        }
    }
    Ok(())
}
