//! Compare our independent lossless CRX reconstruction with released-binary PGM output.
use photocraft_raw::{Limits, decode};
fn hash(values: impl Iterator<Item = u16>) -> u64 {
    values.flat_map(u16::to_le_bytes).fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    for filename in std::env::args_os().skip(1) {
        let path = std::path::PathBuf::from(filename);
        let bytes = std::fs::read(&path)?;
        let now = std::time::Instant::now();
        let samples = decode(&bytes, &Limits::default())?.data;
        let elapsed = now.elapsed();
        let digest = hash(samples.iter().copied());
        let id = path.file_stem().and_then(|s| s.to_str()).ok_or("file id")?;
        let reference = std::path::Path::new("plan/cr3-audit/oracle").join(format!("{id}.sensor.bin"));
        if reference.is_file() {
            let pgm = std::fs::read(reference)?;
            let start = pgm.iter().take(256).position(|v| *v == 10).ok_or("PGM header")? + 1;
            let raw = pgm.get(start..).ok_or("PGM pixels")?;
            if raw.len() != samples.len().checked_mul(2).ok_or("sample count")? {
                return Err("oracle dimensions disagree".into());
            }
            let mut differences = 0usize;
            for (ours, reference) in samples.iter().zip(raw.as_chunks::<2>().0) {
                if *ours != u16::from_be_bytes(*reference) {
                    differences += 1;
                }
            }
            println!("{id}: {} samples, FNV {digest:016x}, {elapsed:?}, {differences} oracle differences", samples.len());
            if differences != 0 {
                return Err("sensor oracle mismatch".into());
            }
        } else {
            println!("{id}: {} samples, FNV {digest:016x}, {elapsed:?}; no local oracle", samples.len());
        }
    }
    Ok(())
}
