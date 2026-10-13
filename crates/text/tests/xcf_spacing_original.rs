use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
use photocraft_text::xcf::{ExactFont, RectilinearAntialias, ScaledText, TextSourceError, prepare_spaced_rectilinear_with_charge};
#[test]
fn spacing_domain_charge_and_cancel_are_explicit() {
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    let request = ScaledText { content: "HEFT", origin: [8, 4], size_px: 32. };
    for spacing in [f32::NAN, f32::INFINITY, -2., 1.5, 1000.] {
        let mut charged = 0;
        assert!(
            prepare_spaced_rectilinear_with_charge(&font, request, spacing, 8 << 20, &Interrupt::NONE, &mut |_| {
                charged += 1;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(charged, 0);
    }
    let mut charged = 0;
    assert!(
        prepare_spaced_rectilinear_with_charge(&font, ScaledText { size_px: 24., ..request }, 2., 8 << 20, &Interrupt::NONE, &mut |_| {
            charged += 1;
            Ok(())
        })
        .is_err()
    );
    assert_eq!(charged, 0, "nonzero-spacing size24 remains unobserved");
    assert!(matches!(
        prepare_spaced_rectilinear_with_charge(&font, request, 2., 8 << 20, &Interrupt::NONE, &mut |_| Err(TextSourceError::Budget)),
        Err(TextSourceError::Budget)
    ));
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let check = || cancelled.load(std::sync::atomic::Ordering::Relaxed);
    let ctl = Interrupt::cancel_only(&check);
    assert!(matches!(
        prepare_spaced_rectilinear_with_charge(&font, request, 4., 8 << 20, &ctl, &mut |_| {
            cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }),
        Err(TextSourceError::Cancelled)
    ));
}
#[test]
#[ignore = "requires frozen independently authored letter-spacing originals"]
fn original_spacing_geometry_layout_and_projection() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-spacing-evidence");
    let raw = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), "2c987cfa774b38d9004717f68a657c6f7c2b3ff00004b43fcf03c8f61a934687");
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
        let prepared = prepare_spaced_rectilinear_with_charge(
            &font,
            request,
            state["letter_spacing"].as_f64().unwrap() as f32,
            8 << 20,
            &Interrupt::NONE,
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_eq!(u64::from(prepared.layout_rect().width()), state["size"][0].as_u64().unwrap());
        assert_eq!(u64::from(prepared.layout_rect().height()), state["size"][1].as_u64().unwrap());
        let mode = if state["antialias"].as_bool().unwrap() { RectilinearAntialias::AreaCoverage } else { RectilinearAntialias::None };
        let rendered = prepared.render_mode_with_charge(mode, 8 << 20, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
        let expected = std::fs::read(root.join(state["projection"]["file"].as_str().unwrap())).unwrap();
        let actual = rendered.surface.try_read_interleaved(Rect::new(0, 0, 128, 64)).unwrap();
        let max = actual.iter().zip(&expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        println!("{name}: max {max}/255");
        assert!(max <= 2, "{name}: {max}/255");
    }
}
