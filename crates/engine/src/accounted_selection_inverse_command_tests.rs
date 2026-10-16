//! Deferred registered native inverse integration, not producer selection math.
//! Generated source/expected buffers are external harness allocations; imports,
//! guards and native models use the real prospective caller quota.
use super::*;
#[path = "accounted_programme_selection_native_test_support.rs"]
mod f;
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::accounted_selection::{AccountedSelection, CoverageDomain};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use serde_json::{Value, json};
fn legacy(sample: SampleType) -> Session {
    let (precision, word) = match sample {
        SampleType::U8 => (150, vec![64]),
        SampleType::U16 => (250, 0x4001u16.to_be_bytes().to_vec()),
        SampleType::U32 => (350, 0x40000001u32.to_be_bytes().to_vec()),
        SampleType::F32 => (650, 0.25f32.to_be_bytes().to_vec()),
        SampleType::F64 => (750, (0.25 + 2f64.powi(-48)).to_be_bytes().to_vec()),
        _ => panic!("five native samples"),
    };
    let mut pixel = word.repeat(3);
    pixel.extend_from_slice(&word);
    let raw = pixel.repeat(6);
    let q = BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 32 << 20, scratch: 16 << 20 });
    let input = photocraft_io::import_owned_staged_plain_bytes_in_quota(
        "inverse.xcf",
        std::sync::Arc::new(xcf::image_custom(13, precision, 0, 3, 2, &raw, &Default::default())),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        q,
        &f::ctl(),
    )
    .unwrap();
    let mut s = Session::new();
    s.add_owned_plain_xcf_import(&input, "inverse.xcf", Some("protected/inverse.xcf"), &f::ctl()).unwrap();
    s
}
fn programme(case: f::Case, depth: usize) -> Session {
    let input = f::load(case, depth);
    let mut s = Session::new();
    f::adopt(&mut s, case, &input);
    s
}
fn words(sample: SampleType) -> Vec<Vec<u8>> {
    match sample {
        SampleType::U8 => [0, 1, 127, 128, 254, 255].into_iter().map(|v| vec![v]).collect(),
        SampleType::U16 => [0, 1, 0x7fff, 0x8001, 0xfffe, 0xffff].into_iter().map(|v: u16| v.to_ne_bytes().to_vec()).collect(),
        SampleType::U32 => [0, 1, 0x7fffffff, 0x80000001, u32::MAX - 1, u32::MAX].into_iter().map(|v| v.to_ne_bytes().to_vec()).collect(),
        SampleType::F32 => [-0.0f32, 0.0, 0.25, 0.50000006, 0.75, 1.0].into_iter().map(|v| v.to_ne_bytes().to_vec()).collect(),
        SampleType::F64 => [-0.0f64, 0.0, 0.25 + 2f64.powi(-48), 0.5 + 2f64.powi(-45), 0.75, 1.0].into_iter().map(|v| v.to_ne_bytes().to_vec()).collect(),
        _ => panic!("sample"),
    }
}
fn inverse(sample: SampleType, w: &[u8]) -> Vec<u8> {
    match sample {
        SampleType::U8 => vec![255 - w[0]],
        SampleType::U16 => (u16::MAX - u16::from_ne_bytes(w.try_into().unwrap())).to_ne_bytes().to_vec(),
        SampleType::U32 => (u32::MAX - u32::from_ne_bytes(w.try_into().unwrap())).to_ne_bytes().to_vec(),
        SampleType::F32 => (1.0f32 - f32::from_ne_bytes(w.try_into().unwrap())).to_ne_bytes().to_vec(),
        SampleType::F64 => (1.0f64 - f64::from_ne_bytes(w.try_into().unwrap())).to_ne_bytes().to_vec(),
        _ => panic!("sample"),
    }
}
fn install(s: &mut Session, sample: SampleType) -> Vec<Vec<u8>> {
    let d = &s.active().unwrap().doc;
    let values = words(sample);
    let count = d.size.width as usize * d.size.height as usize;
    let raw = (0..count).flat_map(|i| values[i % values.len()].iter().copied()).collect::<Vec<_>>();
    let field = AccountedSelection::try_from_native(
        PixelFormat::new(ColorMode::Grayscale, sample, false),
        d.bounds(),
        &vec![0; sample.bytes()],
        &raw,
        CoverageDomain::FiniteUnit,
        d.quota().unwrap(),
        &f::ctl(),
    )
    .unwrap();
    let target = AccountedSelectionTarget::capture_with_control(s, &f::ctl()).unwrap();
    s.replace_accounted_selection(target, "native inverse input", Some(&field), &f::ctl()).unwrap();
    values
}
fn check(s: &Session, sample: SampleType, expected: &[Vec<u8>]) {
    let d = &s.active().unwrap().doc;
    let selection = d.selection.as_ref().unwrap();
    assert_eq!(selection.format(), PixelFormat::new(ColorMode::Grayscale, sample, false));
    assert!(selection.default_bytes().iter().all(|v| *v == 0));
    let mut word = vec![0; sample.bytes()];
    for y in 0..d.size.height {
        for x in 0..d.size.width {
            selection.try_read_pixel_bytes(x as i32, y as i32, &mut word).unwrap();
            assert_eq!(word, expected[(y as usize * d.size.width as usize + x as usize) % expected.len()]);
        }
    }
    selection.try_read_pixel_bytes(d.bounds().x1, d.bounds().y0, &mut word).unwrap();
    assert!(word.iter().all(|v| *v == 0));
}
#[test]
#[ignore = "UNRUN/uncompiled: registered Legacy native five-sample inversion and genuine four-programme floating Source owners, exact U32/F64 lowbits and signedzero arithmetic, guard/native/source/default/history custody and exact-half owner/redo no-op; no Source selection grant"]
fn inverse_registered_route_keeps_native_words_and_real_owner() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let mut s = legacy(sample);
        let id = f::base::target(&s.active().unwrap().doc);
        let raw = f::raw(&s.active().unwrap().doc, id);
        let source = s.active().unwrap().source.clone();
        let values = install(&mut s, sample);
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let past = s.active().unwrap().history.past_len();
        assert_eq!(s.execute("select.inverse", Value::Null).unwrap(), Value::Null);
        let expected = values.iter().map(|w| inverse(sample, w)).collect::<Vec<_>>();
        check(&s, sample, &expected);
        assert_eq!(s.active().unwrap().revision, revision + 1);
        assert_eq!(s.active().unwrap().history.past_len(), past + 1);
        assert_eq!(f::raw(&s.active().unwrap().doc, id), raw);
        assert_eq!(s.active().unwrap().source, source);
        assert!(s.active().unwrap().source_read_only);
        assert!(s.undo_with_control(&f::ctl()).unwrap());
        f::same_selection(&before, &s.active().unwrap().doc);
        assert!(s.redo_with_control(&f::ctl()).unwrap());
        check(&s, sample, &expected);
    }
    for case in f::CASES {
        for depth in 0..2 {
            let mut s = programme(case, depth);
            let id = f::base::target(&s.active().unwrap().doc);
            let witness = f::Witness::capture(s.active().unwrap().doc.accounted().unwrap(), &f::ctl());
            let native = f::raw(&s.active().unwrap().doc, id);
            let sample = s.active().unwrap().doc.depth;
            let values = install(&mut s, sample);
            let st = s.active().unwrap();
            let p = json!({"documentId":st.doc.id.0.to_string(),"expectedRevision":st.revision.to_string()});
            s.execute("select.inverse", p).unwrap();
            check(&s, sample, &values.iter().map(|w| inverse(sample, w)).collect::<Vec<_>>());
            witness.preserved(&s.active().unwrap().doc, id, &f::ctl());
            assert_eq!(f::raw(&s.active().unwrap().doc, id), native);
        }
    }
    // The inverse-specific native equality proof must reuse the original guard
    // for exact stored .5, preserving the owner/history/redo instead of copying.
    for programme_case in [false, true] {
        let mut s = if programme_case { programme(f::Case::MaskedFilteredFlat, 1) } else { legacy(SampleType::F64) };
        let d = &s.active().unwrap().doc;
        let raw = 0.5f64.to_ne_bytes().repeat(d.size.width as usize * d.size.height as usize);
        let half = AccountedSelection::try_from_native(
            PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false),
            d.bounds(),
            &[0; 8],
            &raw,
            CoverageDomain::FiniteUnit,
            d.quota().unwrap(),
            &f::ctl(),
        )
        .unwrap();
        let target = AccountedSelectionTarget::capture_with_control(&s, &f::ctl()).unwrap();
        s.replace_accounted_selection(target, "exact half", Some(&half), &f::ctl()).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        assert!(s.undo_with_control(&f::ctl()).unwrap());
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.clone();
        let revision = s.active().unwrap().revision;
        s.execute("select.inverse", json!({})).unwrap();
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(history.shares_accounted_history(&s.active().unwrap().history));
        assert!(s.active().unwrap().history.can_redo());
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: captured Legacy and programme source/view/history/revision/wire fences, None-selection refusal, original early Work/cancel and no publication; no late-stage cancellation assertion"]
fn inverse_captured_context_and_original_controls_refuse_atomically() {
    for programme_case in [false, true] {
        for variant in 0..7 {
            let mut s = if programme_case { programme(f::Case::FilteredFlat, 1) } else { legacy(SampleType::F64) };
            install(&mut s, SampleType::F64);
            let before = s.active().unwrap().doc.clone();
            let journal = s.journal.clone();
            let p = json!({});
            let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "select.inverse", &p, &f::ctl()).unwrap();
            match variant {
                0 => s.active_mut().unwrap().source.as_mut().unwrap().label.push_str(" drift"),
                1 => s.active_mut().unwrap().path = Some("changed.pcraft".into()),
                2 => {
                    let v = s.active().unwrap().source_read_only;
                    s.active_mut().unwrap().source_read_only = !v
                }
                3 => s.active_mut().unwrap().view_revision += 1,
                4 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
                5 => s.active_mut().unwrap().history.set_current_label("changed", &f::ctl()).unwrap(),
                _ => s.prefs.edit(|p| p.performance.history_states = 1),
            }
            assert!(prepared.execute(&mut s, &f::ctl()).is_err());
            assert!(before.shares_state(&s.active().unwrap().doc));
            assert_eq!(s.journal, journal);
        }
    }
    let mut s = programme(f::Case::MaskedFilteredFlat, 0);
    let original = s.active().unwrap().doc.clone();
    assert!(s.execute("select.inverse", json!({})).is_err());
    assert!(original.shares_state(&s.active().unwrap().doc));
    install(&mut s, SampleType::F32);
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    for p in [
        json!({"unexpected":true}),
        json!({"documentId":"01","expectedRevision":"1"}),
        json!({"documentId":1,"expectedRevision":1}),
        json!({"expectedRevision":"1"}),
    ] {
        assert!(s.execute("select.inverse", p).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert_eq!(s.journal, journal);
    }
    let p = json!({});
    let deny = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
    assert!(crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "select.inverse", &p, &c).is_err());
    let yes = || true;
    let interrupt = photocraft_raster::Interrupt::cancel_only(&yes);
    assert!(
        crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "select.inverse", &p, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) })
            .is_err()
    );
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert_eq!(s.journal, journal);
}
