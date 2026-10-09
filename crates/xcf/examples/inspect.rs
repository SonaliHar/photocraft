//! Development-only corpus inspection; the library itself is byte-only.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args().nth(1).ok_or("expected directory")?;
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().is_some_and(|s| s.eq_ignore_ascii_case("xcf")) {
            let bytes = std::fs::read(&path)?;
            match photocraft_xcf::decode(&bytes, &Default::default()) {
                Ok(f) => println!(
                    "{}: v{} {:?}/{:?}/{:?} layers{} {:?}",
                    path.display(),
                    f.version,
                    f.model,
                    f.sample,
                    f.compression,
                    f.layers.len(),
                    f.layers.iter().map(|l| (&l.name, l.mode, l.composite_space, l.blend_space, &l.unsupported)).collect::<Vec<_>>()
                ),
                Err(e) => println!("{}: {e}", path.display()),
            }
        }
    }
    Ok(())
}
