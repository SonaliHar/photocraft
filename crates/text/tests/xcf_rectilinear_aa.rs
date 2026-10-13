use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
use photocraft_text::xcf::{ExactFont, RectilinearAntialias, StemText, TextSourceError, render_rectilinear_mode_with_charge};
#[test]
fn area_output_is_precharged_and_cancelled_atomically() {
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    let request = StemText { content: "HEFT", origin: [8, 4] };
    let mut charges = 0;
    assert!(matches!(
        render_rectilinear_mode_with_charge(&font, request, RectilinearAntialias::AreaCoverage, 8 << 20, &Interrupt::NONE, &mut |_| {
            charges += 1;
            if charges == 2 { Err(TextSourceError::Budget) } else { Ok(()) }
        }),
        Err(TextSourceError::Budget)
    ));
    assert_eq!(charges, 2);
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let cancel = || cancelled.load(std::sync::atomic::Ordering::Relaxed);
    let ctl = Interrupt::cancel_only(&cancel);
    let mut charges = 0;
    assert!(matches!(
        render_rectilinear_mode_with_charge(&font, request, RectilinearAntialias::AreaCoverage, 8 << 20, &ctl, &mut |_| {
            charges += 1;
            if charges == 2 {
                cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            Ok(())
        }),
        Err(TextSourceError::Cancelled)
    ));
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let cancel_during_coverage = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 200;
    let ctl = Interrupt::cancel_only(&cancel_during_coverage);
    let mut charges = 0;
    assert!(matches!(
        render_rectilinear_mode_with_charge(&font, request, RectilinearAntialias::AreaCoverage, 8 << 20, &ctl, &mut |_| {
            charges += 1;
            Ok(())
        }),
        Err(TextSourceError::Cancelled)
    ));
    assert_eq!(charges, 2, "cancellation reached coverage after output admission");
    for unsupported in ["O", "A", "é", "        "] {
        assert!(
            render_rectilinear_mode_with_charge(
                &font,
                StemText { content: unsupported, origin: [8, 4] },
                RectilinearAntialias::AreaCoverage,
                8 << 20,
                &Interrupt::NONE,
                &mut |_| Ok(())
            )
            .is_err()
        );
    }
}
#[test]
#[ignore = "requires frozen independently authored antialiased exact-font GIMP controls"]
fn independent_aa_controls_and_public_edits() {
    use sha2::{Digest, Sha256};
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-rectilinear-aa-evidence");
    let manifest = std::fs::read(base.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "5b76f7e300ceae7f7051338dfd796701047928d1d2b8b3d00b18863e1533501e");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (name, hash) in files {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(base.join(&name)).unwrap())), hash, "{name}");
    }
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    for (i, row) in data["rows"].as_array().unwrap().iter().enumerate() {
        let text = row["state"]["text"].as_str().unwrap();
        let name = if i < 11 { format!("control-{i}") } else { format!("{}-{}", row["case"].as_str().unwrap(), row["phase"].as_str().unwrap()) };
        let parsed = photocraft_xcf::decode(&std::fs::read(base.join(format!("{name}.xcf"))).unwrap(), &Default::default()).unwrap();
        assert_eq!(parsed.version, 19);
        assert_eq!(parsed.precision, 150);
        let source = parsed.layers[0].text.as_ref().unwrap();
        assert_eq!(source.layout.antialias, Some(true));
        assert_eq!(source.font.as_ref().unwrap().hash.as_deref(), Some("40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82"));
        let expected = std::fs::read(base.join(format!("{name}.projection.bin"))).unwrap();
        assert_eq!(expected, std::fs::read(base.join(format!("round2/{name}.projection.bin"))).unwrap());
        let rendered = render_rectilinear_mode_with_charge(
            &font,
            StemText { content: text, origin: [8, 4] },
            RectilinearAntialias::AreaCoverage,
            8 << 20,
            &Interrupt::NONE,
            &mut |_| Ok(()),
        )
        .unwrap();
        let actual = rendered.surface.try_read_interleaved(Rect::new(0, 0, 128, 64)).unwrap();
        let max = actual.iter().zip(&expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        println!("{text:?} AA max {max}/255");
        assert!(max <= 2, "{text:?}: {max}/255 exceeds existing matched-display bound");
    }
}
