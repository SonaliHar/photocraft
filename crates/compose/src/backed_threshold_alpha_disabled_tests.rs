//! UNRUN disabled CURRENT recipe/native identity and display-policy drafts.
use super::*;

#[test]
fn disabled_recipe_keeps_raw_native_words_and_existing_normal_display_policy() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
            for stored in [SampleEncoding::Linear, SampleEncoding::NonlinearProfile] {
                let (mut doc, q, guard, _, profile, raw) = fixture(mode, sample, stored);
                doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
                let owner = adopt(doc, guard, q);
                let prepared = PreparedBackedThresholdAlphaContext::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
                let native = prepared.render_band(owner.document().bounds(), &ctl()).unwrap();
                assert_eq!(native.bytes(), raw);
                let display =
                    crate::backed_threshold_alpha_document::PreparedBackedThresholdAlphaDocument::new(&owner, profile, NormalBandLimits::default(), &ctl())
                        .unwrap()
                        .render_band(owner.document().bounds(), &ctl())
                        .unwrap();
                for (pixel, words) in display.pixels().iter().zip(raw.chunks_exact(native.format().bytes_per_pixel())) {
                    let alpha = photocraft_color::try_read_sample_f64(words, sample, mode.color_channels()).unwrap();
                    if alpha == 0.0 {
                        assert!(pixel.iter().all(|v| v.to_bits() == 0));
                    } else {
                        for (channel, value) in pixel.iter().enumerate().take(3) {
                            let source = if mode == ColorMode::Grayscale { 0 } else { channel };
                            assert_eq!(value.to_bits(), photocraft_color::try_read_sample_f64(words, sample, source).unwrap().to_bits());
                        }
                        assert_eq!(pixel[3].to_bits(), alpha.to_bits());
                    }
                }
            }
        }
    }
}
