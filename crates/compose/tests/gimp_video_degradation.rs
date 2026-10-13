use photocraft_algo::gimp::point_video_degradation::VideoDegradation;
use photocraft_cms::Builtin;
use photocraft_color::PixelFormat;
use photocraft_compose::gimp_video_degradation::VideoDegradationCandidate;
use photocraft_doc::SampleEncoding;
use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
fn op() -> VideoDegradation {
    VideoDegradation { pattern: 5, additive: false, rotated: false }
}
#[test]
fn native_route_scope_and_atomic_coordinates() {
    let owner = Rect::new(-5, -3, 11, 13);
    let c = VideoDegradationCandidate::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, PixelFormat::RGBA32F, owner, op()).unwrap();
    let mut out = [17.; 4];
    c.try_eval_at((-5, -3), &[0.25, 0.5, 0.75, -0.0], &mut out, true, &Interrupt::NONE).unwrap();
    assert_eq!(out[..3], [0.25, 0., 0.]);
    assert_eq!(out[3].to_bits(), (-0.0f64).to_bits());
    for p in [(-6, -3), (-5, -4), (11, 12)] {
        out = [17.; 4];
        assert!(c.try_eval_at(p, &[0.25, 0.5, 0.75, 1.], &mut out, true, &Interrupt::NONE).is_err());
        assert_eq!(out, [17.; 4]);
    }
    for encoding in [SampleEncoding::Linear, SampleEncoding::Perceptual] {
        assert!(VideoDegradationCandidate::new(Builtin::Srgb.profile(), encoding, PixelFormat::RGBA32F, owner, op()).is_err());
    }
    for format in [PixelFormat::RGBA8, PixelFormat::RGBA64F] {
        assert!(VideoDegradationCandidate::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, format, owner, op()).is_err());
    }
    assert!(VideoDegradationCandidate::new(Builtin::DisplayP3.profile(), SampleEncoding::NonlinearProfile, PixelFormat::RGBA32F, owner, op()).is_err());
    assert!(
        VideoDegradationCandidate::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, PixelFormat::RGBA32F, Rect::new(0, 0, 33, 24), op()).is_err()
    );
    out = [17.; 4];
    assert!(c.try_eval_at((-5, -3), &[0.1, 0., 0., 1.], &mut out, true, &Interrupt::NONE).is_err());
    assert_eq!(out, [17.; 4]);
    out = [17.; 4];
    assert!(c.try_eval_at((-5, -3), &[0.25, 0., 0., 1.], &mut out, true, &Interrupt::cancel_only(&|| true)).is_err());
    assert_eq!(out, [17.; 4]);
    let original = [-0.0, 0.5, 0.75, -0.0];
    c.try_eval_at((-5, -3), &original, &mut out, false, &Interrupt::NONE).unwrap();
    assert_eq!(out.map(f64::to_bits), original.map(f64::to_bits));
}
#[test]
fn unqualified_owner_geometry_is_refused() {
    let admitted =
        |owner, operation| VideoDegradationCandidate::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, PixelFormat::RGBA32F, owner, operation);
    for owner in [
        Rect::new(0, 0, 1, 1),
        Rect::new(0, 0, 17, 16),
        Rect::new(1, 0, 17, 16),
        Rect::new(i32::MIN, 0, i32::MIN + 16, 16),
        Rect::new(0, i32::MAX - 16, 16, i32::MAX),
    ] {
        assert!(admitted(owner, op()).is_err(), "unobserved owner {owner:?}");
    }
    for operation in [VideoDegradation { pattern: 0, ..op() }, VideoDegradation { additive: true, ..op() }, VideoDegradation { rotated: true, ..op() }] {
        assert!(admitted(Rect::new(-5, -3, 11, 13), operation).is_err());
    }
    let candidate = admitted(Rect::new(0, 0, 16, 16), op()).unwrap();
    for enabled in [true, false] {
        let mut output = [17.; 4];
        assert!(candidate.try_eval_at((0, 0), &[2., 0.5, 0.75, 1.], &mut output, enabled, &Interrupt::NONE).is_err());
        assert_eq!(output, [17.; 4]);
    }
}
#[test]
fn cancellation_after_evaluation_and_bad_slices_are_atomic() {
    let candidate =
        VideoDegradationCandidate::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, PixelFormat::RGBA32F, Rect::new(0, 0, 16, 16), op()).unwrap();
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 1;
    let mut output = [17.; 4];
    assert!(candidate.try_eval_at((0, 0), &[0.25, 0.5, 0.75, 1.], &mut output, true, &Interrupt::cancel_only(&cancel)).is_err());
    assert_eq!(polls.load(std::sync::atomic::Ordering::Relaxed), 2);
    assert_eq!(output, [17.; 4]);
    assert!(candidate.try_eval_at((0, 0), &[0.25, 0.5, 0.75], &mut output, true, &Interrupt::NONE).is_err());
    assert_eq!(output, [17.; 4]);
    let mut short = [17.; 3];
    assert!(candidate.try_eval_at((0, 0), &[0.25, 0.5, 0.75, 1.], &mut short, true, &Interrupt::NONE).is_err());
    assert_eq!(short, [17.; 3]);
}
#[test]
#[ignore = "independent public local-phase/native controls"]
fn native_candidate_saved_offset_controls() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/video-degradation-origin-evidence");
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    assert_eq!(j["records"].as_array().unwrap().len(), 3);
    for r in j["records"].as_array().unwrap() {
        let x = r["offset"][0].as_i64().unwrap() as i32;
        let y = r["offset"][1].as_i64().unwrap() as i32;
        let c = VideoDegradationCandidate::new(
            Builtin::Srgb.profile(),
            SampleEncoding::NonlinearProfile,
            PixelFormat::RGBA32F,
            Rect::new(x, y, x + 16, y + 16),
            op(),
        )
        .unwrap();
        let read = |d: &serde_json::Value| {
            std::fs::read(root.join(d["file"].as_str().unwrap()))
                .unwrap()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f64::from(f32::from_le_bytes(*b)))
                .collect::<Vec<_>>()
        };
        let input = read(&r["current"]["layers"][0]["native"]);
        let expected = read(&r["baked"]["layers"][0]["native"]);
        assert_eq!(input.len(), 16 * 16 * 4);
        assert_eq!(expected.len(), input.len());
        for (i, (src, dst)) in input.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0.iter()).enumerate() {
            let mut out = [17.; 4];
            c.try_eval_at((x + (i % 16) as i32, y + (i / 16) as i32), src, &mut out, true, &Interrupt::NONE).unwrap();
            assert_eq!(out.map(f64::to_bits), dst.map(f64::to_bits));
        }
        assert_eq!(r["roi_rect"], serde_json::json!([5, 7, 3, 4]));
        let roi = read(&r["roi_native"]);
        assert_eq!(roi.len(), 3 * 4 * 4);
        for (index, expected) in roi.as_chunks::<4>().0.iter().enumerate() {
            let local_x = 5 + (index % 3) as i32;
            let local_y = 7 + (index / 3) as i32;
            let start = ((local_y * 16 + local_x) * 4) as usize;
            let mut output = [17.; 4];
            c.try_eval_at((x + local_x, y + local_y), &input[start..start + 4], &mut output, true, &Interrupt::NONE).unwrap();
            assert_eq!(output.map(f64::to_bits), expected.map(f64::to_bits));
        }
    }
}

#[test]
#[ignore = "independent public saved native response/edit/disabled controls"]
fn native_candidate_all_saved_controls() {
    let full = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
    let names = ["staggered", "large-staggered", "striped", "wide-striped", "long-staggered", "3x3", "large-3x3", "hex", "dots"];
    let mut count = 0;
    for folder in ["video-degradation-response-evidence", "video-degradation-period-holdout-evidence", "video-degradation-edit-evidence"] {
        let root = full.join(folder);
        let j: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        for r in j["records"].as_array().unwrap() {
            let cfg = &r["current"]["layers"][0]["filters"][0]["config"];
            let pattern = names.iter().position(|n| Some(*n) == cfg["pattern"].as_str()).unwrap() as u8;
            let op = VideoDegradation { pattern, additive: cfg["additive"].as_bool().unwrap(), rotated: cfg["rotated"].as_bool().unwrap() };
            let enabled = r["current"]["layers"][0]["filters"][0]["visible"].as_bool().unwrap();
            let w = r["current"]["size"][0].as_i64().unwrap() as i32;
            let h = r["current"]["size"][1].as_i64().unwrap() as i32;
            let c = VideoDegradationCandidate::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, PixelFormat::RGBA32F, Rect::new(0, 0, w, h), op)
                .unwrap();
            let read = |d: &serde_json::Value| {
                std::fs::read(root.join(d["file"].as_str().unwrap()))
                    .unwrap()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f64::from(f32::from_le_bytes(*b)))
                    .collect::<Vec<_>>()
            };
            let input = read(&r["current"]["layers"][0]["native"]);
            let expected = read(&r["baked"]["layers"][0]["native"]);
            assert_eq!(input.len(), (w * h * 4) as usize);
            assert_eq!(expected.len(), input.len());
            for (i, (src, dst)) in input.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0.iter()).enumerate() {
                let mut out = [17.; 4];
                c.try_eval_at(((i % (w as usize)) as i32, (i / (w as usize)) as i32), src, &mut out, enabled, &Interrupt::NONE).unwrap();
                assert_eq!(out.map(f64::to_bits), dst.map(f64::to_bits), "{} pixel{}", r["id"], i);
            }
            count += 1;
        }
    }
    assert_eq!(count, 75);
}
