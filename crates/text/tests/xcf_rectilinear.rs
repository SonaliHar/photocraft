use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
use photocraft_text::xcf::{ExactFont, StemText, TextSourceError, qualify_rectilinear_geometry, render_rectilinear_with_charge};
#[test]
fn rectilinear_semantics_and_atomic_budget_denial() {
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    let request = StemText { content: "HEFT", origin: [8, 4] };
    qualify_rectilinear_geometry(&font, request, &Interrupt::NONE).unwrap();
    let mut calls = 0;
    assert_eq!(
        render_rectilinear_with_charge(&font, request, 16 << 20, &Interrupt::NONE, &mut |_| {
            calls += 1;
            Err(TextSourceError::Budget)
        })
        .err(),
        Some(TextSourceError::Budget)
    );
    assert_eq!(calls, 1);
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let cancel = || cancelled.load(std::sync::atomic::Ordering::Relaxed);
    let ctl = Interrupt::new(&cancel, &|_| {});
    assert_eq!(
        render_rectilinear_with_charge(&font, request, 16 << 20, &ctl, &mut |_| {
            cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        })
        .err(),
        Some(TextSourceError::Cancelled)
    );
    let mut output_calls = 0;
    assert_eq!(
        render_rectilinear_with_charge(&font, request, 16 << 20, &Interrupt::NONE, &mut |_| {
            output_calls += 1;
            if output_calls == 2 { Err(TextSourceError::Budget) } else { Ok(()) }
        })
        .err(),
        Some(TextSourceError::Budget)
    );
    assert_eq!(output_calls, 2);
    for text in ["", "iiiiiiiii", "O", "A", "é", "        "] {
        assert!(qualify_rectilinear_geometry(&font, StemText { content: text, origin: [8, 4] }, &Interrupt::NONE).is_err());
        assert!(
            render_rectilinear_with_charge(&font, StemText { content: text, origin: [8, 4] }, 16 << 20, &Interrupt::NONE, &mut |_| Ok(())).is_err(),
            "{text:?}"
        );
    }
    assert!(render_rectilinear_with_charge(&font, request, 1, &Interrupt::NONE, &mut |_| Ok(())).is_err());
}
#[test]
#[ignore = "requires own isolated stock GIMP exact-authority public15controls"]
fn fifteen_independent_originals_exact_shaping_geometry_and_repeat() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-rectilinear-evidence/rectilinear-controls");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(base.join("observations.json")).unwrap()).unwrap();
    let font = ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    for (index, row) in observations["rows"].as_array().unwrap().iter().enumerate() {
        qualify_rectilinear_geometry(&font, StemText { content: row["text"].as_str().unwrap(), origin: [8, 4] }, &Interrupt::NONE).unwrap();
        let mut charged = 0;
        let r =
            render_rectilinear_with_charge(&font, StemText { content: row["text"].as_str().unwrap(), origin: [8, 4] }, 16 << 20, &Interrupt::NONE, &mut |n| {
                charged += n;
                Ok(())
            })
            .unwrap();
        assert_eq!(charged, r.resident_bytes);
        let actual = r.surface.try_read_interleaved(Rect::new(0, 0, 128, 64)).unwrap();
        let expected = std::fs::read(base.join(format!("control-{index}.projection.bin"))).unwrap();
        let repeat = std::fs::read(base.join(format!("round2/control-{index}.projection.bin"))).unwrap();
        assert_eq!(expected, repeat);
        assert_eq!(actual, expected, "{}", row["text"]);
    }
}
