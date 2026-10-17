//! UNRUN display and actual runtime-class dispatch regression drafts.
use super::*;
use crate::backed_threshold_context::tests::{adopt, ctl, fixture};
use photocraft_cms::Builtin;
use photocraft_doc::{SampleEncoding, WorkingProfileInterpretation, item_identity::ItemRef};
use photocraft_raster::{Interrupt, backing::BackingError};
use std::sync::atomic::Ordering;
#[test]
fn native_and_display_outputs_are_separate_and_dispatch_matches() {
    for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
        let (doc, q, guard, _, p, _) = fixture(ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
        let owner = adopt(doc, guard, q);
        let display = PreparedBackedThresholdDocument::new(&owner, p, NormalBandLimits::default(), &ctl()).unwrap();
        let dispatched = crate::backed_document::PreparedBackedDocument::new_accounted(&owner, p, NormalBandLimits::default(), &ctl()).unwrap();
        let bounds = owner.document().bounds();
        let band = display.render_band(bounds, &ctl()).unwrap();
        let routed = dispatched.render_band(bounds, &ctl()).unwrap();
        assert_eq!(band.pixels(), routed.pixels());
        for (i, pixel) in band.pixels().iter().enumerate() {
            let rgb = if i % 3 == 0 || i % 4 == 0 { 0. } else { 1. };
            assert_eq!(&pixel[..3], &[rgb; 3]);
            assert!(
                (pixel[3]
                    - if i % 4 == 0 {
                        0.0
                    } else if i % 2 == 0 {
                        0.25
                    } else {
                        0.75
                    })
                .abs()
                    < 0.00002
            );
        }
        assert!(display.render_band(Rect::new(-1, 0, 8, 6), &ctl()).is_err());
    }
}
#[test]
fn explicit_current_profile_and_actual_storage_class_remain_mandatory() {
    let (mut doc, q, guard, _, p, _) = fixture(ColorMode::Rgb, SampleType::F32, SampleEncoding::NonlinearProfile);
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    let owner = adopt(doc, guard, q);
    assert!(PreparedBackedThresholdDocument::new(&owner, p, NormalBandLimits::default(), &ctl()).is_err());
    assert!(crate::backed_document::PreparedBackedDocument::new_accounted(&owner, Builtin::SGray.profile(), NormalBandLimits::default(), &ctl()).is_err());
    assert!(crate::backed_document::PreparedBackedDocument::new(owner.document(), p, NormalBandLimits::default(), &ctl()).is_err());
}

#[test]
fn current_layer_coverage_preserves_native_words_and_uses_original_zero_backdrop_math() {
    for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
        for (visible, opacity) in [(true, 0.0f32), (true, -0.0), (true, 0.5), (true, 1.0), (false, 0.5)] {
            let (mut doc, q, guard, _reads, profile, raw) = fixture(ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
            doc.layers[0].visible = visible;
            doc.layers[0].opacity = opacity;
            let id = doc.layers[0].id;
            doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(id)).unwrap().visible = visible;
            doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
            if !visible || opacity.to_bits() != 1f32.to_bits() {
                let state = doc.xcf_state.as_ref().unwrap();
                let photocraft_doc::LayerContent::BackedRaster(backed) = &doc.layers[0].content else { panic!() };
                assert!(
                    crate::gimp_render::qualify_generic_point_storage_metadata(
                        &doc,
                        &doc.layers[0],
                        state,
                        &state.filter_stacks[0],
                        doc.bounds(),
                        SampleEncoding::NonlinearProfile,
                        backed.format(),
                        backed.default_bytes(),
                    )
                    .is_err(),
                    "original source recipe must not gain dynamic CURRENT coverage"
                );
            }
            let owner = adopt(doc, guard, q.clone());
            let context = crate::backed_threshold_context::PreparedBackedThresholdContext::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
            assert_eq!(context.layer_visible(), visible);
            assert_eq!(context.layer_opacity().to_bits(), f64::from(opacity).to_bits());
            let native = context.render_band(owner.document().bounds(), &ctl()).unwrap();
            assert_eq!(native.bytes(), raw);
            drop(native);
            let display = PreparedBackedThresholdDocument::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
            let work_units = std::sync::atomic::AtomicU64::new(0);
            let charge = |units| {
                work_units.fetch_add(units, Ordering::Relaxed);
                Ok(())
            };
            let render_control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
            let before = q.usage().unwrap();
            let band = display.render_band(owner.document().bounds(), &render_control).unwrap();
            assert!(work_units.load(Ordering::Relaxed) > 0, "cached hidden bands still pay operative validation work");
            for (pixel, raw) in band.pixels().iter().zip(raw.chunks_exact(4 * sample.bytes())) {
                let alpha = photocraft_color::try_read_sample_f64(raw, sample, 3).unwrap() * if visible { f64::from(opacity) } else { 0.0 };
                if alpha == 0.0 {
                    assert!(pixel.iter().all(|v| v.to_bits() == 0));
                } else {
                    assert_eq!(pixel[3].to_bits(), alpha.to_bits());
                    for (channel, value) in pixel.iter().enumerate().take(3) {
                        assert_eq!(value.to_bits(), photocraft_color::try_read_sample_f64(raw, sample, channel).unwrap().to_bits());
                    }
                }
            }
            drop(band);
            assert_eq!(q.usage().unwrap(), before);
            let native = context.render_band(owner.document().bounds(), &ctl()).unwrap();
            assert_eq!(native.bytes(), raw);
        }
    }
}
#[test]
fn current_coverage_rejects_invalid_or_incoherent_metadata_and_hidden_limits_remain_paid() {
    for opacity in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        let (mut doc, q, guard, _, profile, _) = fixture(ColorMode::Rgb, SampleType::F32, SampleEncoding::NonlinearProfile);
        doc.layers[0].opacity = opacity;
        let owner = adopt(doc, guard, q);
        assert!(PreparedBackedThresholdDocument::new(&owner, profile, NormalBandLimits::default(), &ctl()).is_err());
    }
    let (mut doc, q, guard, reads, profile, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    doc.layers[0].visible = false;
    let inconsistent = adopt(doc.clone(), guard.clone(), q.clone());
    assert!(PreparedBackedThresholdDocument::new(&inconsistent, profile, NormalBandLimits::default(), &ctl()).is_err());
    let id = doc.layers[0].id;
    doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(id)).unwrap().visible = false;
    let owner = adopt(doc, guard, q.clone());
    let display = PreparedBackedThresholdDocument::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
    let before = q.usage().unwrap();
    let blocked = q.reserve(photocraft_raster::backing::quota::QuotaKind::Resident, (64u64 << 20) - before.resident).unwrap();
    assert!(display.render_band(owner.document().bounds(), &ctl()).is_err());
    assert_eq!(reads.load(Ordering::Relaxed), 0, "hidden output is reserved before source access");
    drop(blocked);
    assert!(display.render_band(Rect::new(-1, 0, 8, 6), &ctl()).is_err());
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(display.render_band(owner.document().bounds(), &denied).is_err());
    let stopped = || true;
    let interrupt = Interrupt::cancel_only(&stopped);
    assert!(display.render_band(owner.document().bounds(), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert_eq!(q.usage().unwrap(), before);
}
