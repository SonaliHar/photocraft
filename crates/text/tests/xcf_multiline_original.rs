use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
use photocraft_text::xcf::{ExactFont, RectilinearAntialias, ScaledText, prepare_multiline_rectilinear_with_charge};
#[test]
fn multiline_limits_leases_cancellation_and_singleline_boundary_are_explicit() {
    use photocraft_text::xcf::{TextSourceError, prepare_spaced_rectilinear_with_charge};
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    let request = ScaledText { content: "HE\nFT", origin: [8, 4], size_px: 32. };
    assert!(prepare_spaced_rectilinear_with_charge(&font, request, 0., 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).is_err());
    for content in ["HEFT", "\nI", "I\n", "I\nI\nI", "H\r\nE", "Hé\nE", "HEFTL\nHEFTL"] {
        let mut charged = 0;
        assert!(
            prepare_multiline_rectilinear_with_charge(&font, ScaledText { content, ..request }, 0., 8 << 20, &Interrupt::NONE, &mut |_| {
                charged += 1;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(charged, 0);
    }
    for spacing in [f32::NAN, f32::INFINITY, -2., 1., 1000.] {
        assert!(
            prepare_multiline_rectilinear_with_charge(&font, request, spacing, 8 << 20, &Interrupt::NONE, &mut |_| panic!("invalid spacing acquired lease"))
                .is_err()
        );
    }
    assert!(matches!(
        prepare_multiline_rectilinear_with_charge(&font, request, 0., (4 << 20) - 1, &Interrupt::NONE, &mut |_| panic!("aggregate denial acquired lease")),
        Err(TextSourceError::Budget)
    ));
    for denied in 1..=2 {
        let mut calls = 0;
        assert!(matches!(
            prepare_multiline_rectilinear_with_charge(&font, request, 4., 8 << 20, &Interrupt::NONE, &mut |_| {
                calls += 1;
                if calls == denied { Err(TextSourceError::Budget) } else { Ok(()) }
            }),
            Err(TextSourceError::Budget)
        ));
        assert_eq!(calls, denied);
    }
    for at in 1..=2 {
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let check = || cancelled.load(std::sync::atomic::Ordering::Relaxed);
        let ctl = Interrupt::cancel_only(&check);
        let mut calls = 0;
        assert!(matches!(
            prepare_multiline_rectilinear_with_charge(&font, request, 2., 8 << 20, &ctl, &mut |_| {
                calls += 1;
                if calls == at {
                    cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                Ok(())
            }),
            Err(TextSourceError::Cancelled)
        ));
        assert_eq!(calls, at);
    }
    assert!(
        prepare_multiline_rectilinear_with_charge(&font, ScaledText { origin: [8, (1 << 29) - 40], ..request }, 4., 8 << 20, &Interrupt::NONE, &mut |_| Ok(()))
            .is_err()
    );
    let prepared = prepare_multiline_rectilinear_with_charge(&font, request, 0., 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
    assert!(matches!(
        prepared.render_mode_with_charge(RectilinearAntialias::AreaCoverage, 8 << 20, &Interrupt::NONE, &mut |_| Err(TextSourceError::Budget)),
        Err(TextSourceError::Budget)
    ));
}
#[test]
#[ignore = "requires frozen independently authored multiline originals"]
fn original_multiline_geometry_layout_and_projection() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-multiline-evidence");
    let raw = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), "b399e6e64e4dbb2390ff52a037a0b6eb60071721b32bbfd3ef922552317dfb50");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&raw).unwrap();
    for (name, hash) in files {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(root.join(name)).unwrap())), hash);
    }
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    for row in observations["rows"].as_array().unwrap() {
        let state = &row["state"];
        let name = row["case"].as_str().unwrap();
        let request = ScaledText { content: state["text"].as_str().unwrap(), origin: [8, 4], size_px: state["font_size"][0].as_f64().unwrap() as f32 };
        let prepared =
            prepare_multiline_rectilinear_with_charge(&font, request, state["line_spacing"].as_f64().unwrap() as f32, 8 << 20, &Interrupt::NONE, &mut |_| {
                Ok(())
            })
            .unwrap();
        assert_eq!(u64::from(prepared.layout_rect().width()), state["size"][0].as_u64().unwrap());
        assert_eq!(u64::from(prepared.layout_rect().height()), state["size"][1].as_u64().unwrap());
        let mode = if state["antialias"].as_bool().unwrap() { RectilinearAntialias::AreaCoverage } else { RectilinearAntialias::None };
        let rendered = prepared.render_mode_with_charge(mode, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let expected = std::fs::read(root.join(state["projection"]["file"].as_str().unwrap())).unwrap();
        let actual = rendered.surface.try_read_interleaved(Rect::new(0, 0, 128, 128)).unwrap();
        let max = actual.iter().zip(&expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        println!("{name}: max {max}/255");
        assert!(max <= 2, "{name}: {max}/255");
    }
}
