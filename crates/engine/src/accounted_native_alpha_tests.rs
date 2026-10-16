//! UNRUN verified alpha-lock exception; ordinary NativePixels stays strict.
use super::*;
use crate::Session;
use photocraft_doc::SampleType;
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaLimits, QuotaUsage},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Clone, Copy)]
enum Lock {
    Alpha,
    Content,
    Pixels,
    All,
}
fn fixture(sample: SampleType, lock: Lock) -> Session {
    let (precision, color, alpha) = match sample {
        SampleType::U8 => (150, vec![127], vec![191]),
        SampleType::U16 => (250, 0x8001u16.to_be_bytes().to_vec(), 0xc001u16.to_be_bytes().to_vec()),
        SampleType::U32 => (350, 0x80000001u32.to_be_bytes().to_vec(), 0xc0000001u32.to_be_bytes().to_vec()),
        SampleType::F32 => (650, 0.5f32.to_be_bytes().to_vec(), (-0.0f32).to_be_bytes().to_vec()),
        SampleType::F64 => (750, (0.5 + 2f64.powi(-44)).to_be_bytes().to_vec(), (-0.0f64).to_be_bytes().to_vec()),
        _ => panic!(),
    };
    let pixel = [color.as_slice(), color.as_slice(), color.as_slice(), alpha.as_slice()].concat();
    let raw = pixel.repeat(257 * 2);
    let mut options = xcf::FixtureOptions::default();
    options.layer_properties.push((10, 1u32.to_be_bytes().to_vec()));
    if matches!(lock, Lock::Content) {
        options.layer_properties.push((28, 1u32.to_be_bytes().to_vec()));
    }
    let q =
        BackingQuota::new_with_aggregate_initial(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 }, 256 << 20, QuotaUsage::default()).unwrap().0;
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 16 << 20), ("scratchBytes", 0)])
        .unwrap();
    let imported = photocraft_io::import_owned_staged_plain_bytes_in_quota(
        "alpha-locked.xcf",
        Arc::new(xcf::image_custom(13, precision, 0, 257, 2, &raw, &options)),
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let mut s = Session::new();
    s.add_owned_plain_xcf_import(&imported, "alpha-locked.xcf", Some("authorized/alpha-locked.xcf"), &ctl()).unwrap();
    if matches!(lock, Lock::Pixels | Lock::All) {
        // Test-only native lock fixture: reserve the known bounded cloned model
        // before making it, while the actual imported backing/source stay held.
        let old = s.active().unwrap().doc.accounted().unwrap().clone();
        let fee = q.reserve(QuotaKind::Resident, 2 << 20).unwrap();
        let mut model = old.document().clone();
        model.layers[0].locks.pixels = matches!(lock, Lock::Pixels);
        model.layers[0].locks.all = matches!(lock, Lock::All);
        let owner = AccountedDocument::from_owned(model, Arc::new((old, fee)), q, &ctl()).unwrap();
        s = Session::new();
        s.add_accounted_document(owner, None, &ctl()).unwrap();
    }
    s
}
fn native(s: &Session, rect: Rect) -> Vec<u8> {
    let LayerContent::BackedRaster(layer) = &s.active().unwrap().doc.layers[0].content else { panic!() };
    let bpp = layer.format().bytes_per_pixel();
    let mut bytes = vec![0; rect.width() as usize * rect.height() as usize * bpp];
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let begin = ((y - rect.y0) as usize * rect.width() as usize + (x - rect.x0) as usize) * bpp;
            layer.checked_view().read_logical_pixel_native_into(x, y, &mut bytes[begin..begin + bpp], &ctl()).unwrap();
        }
    }
    bytes
}
fn changed_colors(s: &Session, rect: Rect) -> Vec<u8> {
    let mut raw = native(s, rect);
    let format = s.active().unwrap().doc.pixel_format();
    for pixel in raw.chunks_exact_mut(format.bytes_per_pixel()) {
        pixel[..format.sample.bytes()].fill(0);
    }
    raw
}
#[test]
fn verified_fill_changes_only_colors_one_history_step_and_generic_edit_stays_locked() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let mut s = fixture(sample, Lock::Alpha);
        let original = s.active().unwrap().doc.clone();
        let source = s.active().unwrap().source.clone();
        let full = original.bounds();
        let original_words = native(&s, full);
        let rect = Rect::new(254, 0, 257, 2); // Crosses a genuine tile boundary.
        let output = changed_colors(&s, rect);
        let id = original.layers[0].id;
        let past = s.active().unwrap().history.past_len();
        assert!(s.edit_accounted("Fill", AccountedEdit::NativePixels { layer: id, rect, bytes: &output }, &ctl()).is_err());
        s.edit_accounted_preserving_native_alpha("Fill", id, rect, &output, &ctl()).unwrap();
        let state = s.active().unwrap();
        assert_eq!(state.history.past_len(), past + 1);
        assert_eq!(state.doc.xcf_state, original.xcf_state);
        assert_eq!(state.doc.source_color_context, original.source_color_context);
        assert_eq!(state.source, source);
        let actual = native(&s, full);
        let format = state.doc.pixel_format();
        let bpp = format.bytes_per_pixel();
        let alpha = bpp - sample.bytes();
        for (i, (before, after)) in original_words.chunks_exact(bpp).zip(actual.chunks_exact(bpp)).enumerate() {
            assert_eq!(&before[alpha..], &after[alpha..]);
            if i % 257 < 254 {
                assert_eq!(before, after);
            }
        }
        assert!(s.undo_with_control(&ctl()).unwrap());
        assert!(original.shares_state(&s.active().unwrap().doc));
        let current = native(&s, rect);
        s.edit_accounted_preserving_native_alpha("Fill", id, rect, &current, &ctl()).unwrap();
        assert!(original.shares_state(&s.active().unwrap().doc));
        assert!(s.redo_with_control(&ctl()).unwrap());
        assert_eq!(native(&s, full), actual);
    }
}
#[test]
fn raw_signed_zero_or_single_bit_alpha_change_refuses_without_publication() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let mut s = fixture(sample, Lock::Alpha);
        let before = s.active().unwrap().doc.clone();
        let id = before.layers[0].id;
        let rect = Rect::new(0, 0, 1, 1);
        let mut output = changed_colors(&s, rect);
        // -0/+0 compare numerically equal at Float depths but must not bypass
        // alpha locks. Integer depths similarly differ by one native bit.
        let index = if matches!(sample, SampleType::F32 | SampleType::F64) {
            let alpha = output.len() - sample.bytes();
            let native_minus_zero = if sample == SampleType::F32 { (-0.0f32).to_ne_bytes().to_vec() } else { (-0.0f64).to_ne_bytes().to_vec() };
            alpha + native_minus_zero.iter().position(|v| *v != 0).unwrap()
        } else {
            output.len() - 1
        };
        output[index] ^= if matches!(sample, SampleType::F32 | SampleType::F64) { 0x80 } else { 1 };
        assert!(s.edit_accounted_preserving_native_alpha("Fill", id, rect, &output, &ctl()).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}
#[test]
fn proof_is_exact_owner_region_and_borrow_not_a_reusable_boolean() {
    let s = fixture(SampleType::F32, Lock::Alpha);
    let owner = s.active().unwrap().doc.accounted().unwrap();
    let id = owner.document().layers[0].id;
    let rect = Rect::new(0, 0, 1, 1);
    let bytes = changed_colors(&s, rect);
    let edit = AccountedEdit::NativePixels { layer: id, rect, bytes: &bytes };
    let proof = VerifiedNativeAlpha::verify(owner, edit, &ctl()).unwrap();
    assert!(proof.authorizes(&owner.clone(), edit));
    let second = bytes.clone();
    assert!(!proof.authorizes(owner, AccountedEdit::NativePixels { layer: id, rect, bytes: &second }));
    assert!(!proof.authorizes(owner, AccountedEdit::NativePixels { layer: id, rect: Rect::new(1, 0, 2, 1), bytes: &bytes }));
    let renamed = owner.try_edit(AccountedEdit::LayerName { layer: id, name: "different model" }, &ctl()).unwrap();
    assert!(!proof.authorizes(&renamed, edit));
}
#[test]
fn opaque_float_alpha_payload_is_compared_without_numeric_interpretation() {
    for sample in [SampleType::F32, SampleType::F64] {
        let source = fixture(sample, Lock::Alpha);
        let original = source.active().unwrap().doc.accounted().unwrap();
        let id = original.document().layers[0].id;
        let rect = Rect::new(0, 0, 1, 1);
        let mut input = native(&source, rect);
        let alpha = input.len() - sample.bytes();
        let opaque = if sample == SampleType::F32 { 0x7fc12345u32.to_ne_bytes().to_vec() } else { 0x7ff8123456789abcu64.to_ne_bytes().to_vec() };
        input[alpha..].copy_from_slice(&opaque);
        // The raw model API constructs this current-state fixture; it does not
        // assert the sample can be produced by a source-qualified Fill kernel.
        let owner = original.try_edit(AccountedEdit::NativePixels { layer: id, rect, bytes: &input }, &ctl()).unwrap();
        let mut s = Session::new();
        s.add_accounted_document(owner, None, &ctl()).unwrap();
        let mut output = changed_colors(&s, rect);
        s.edit_accounted_preserving_native_alpha("Fill", id, rect, &output, &ctl()).unwrap();
        assert_eq!(&native(&s, rect)[alpha..], opaque.as_slice());
        output[alpha] ^= 1;
        let before = s.active().unwrap().doc.clone();
        assert!(s.edit_accounted_preserving_native_alpha("Fill", id, rect, &output, &ctl()).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
    }
}
#[test]
fn all_content_and_pixel_locks_malformed_region_and_stale_history_still_refuse() {
    for lock in [Lock::Content, Lock::Pixels, Lock::All] {
        let mut s = fixture(SampleType::F32, lock);
        let before = s.active().unwrap().doc.clone();
        let rect = Rect::new(0, 0, 1, 1);
        let output = changed_colors(&s, rect);
        assert!(s.edit_accounted_preserving_native_alpha("Fill", before.layers[0].id, rect, &output, &ctl()).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
    }
    let mut s = fixture(SampleType::F32, Lock::Alpha);
    let id = s.active().unwrap().doc.layers[0].id;
    let rect = Rect::new(0, 0, 1, 1);
    let output = changed_colors(&s, rect);
    for (area, bytes) in [(Rect::EMPTY, output.as_slice()), (Rect::new(-1, 0, 0, 1), output.as_slice()), (rect, &output[..output.len() - 1])] {
        assert!(s.edit_accounted_preserving_native_alpha("Fill", id, area, bytes, &ctl()).is_err());
    }
    let old = s.active().unwrap().history.clone();
    s.edit_accounted_preserving_native_alpha("Fill", id, rect, &output, &ctl()).unwrap();
    s.active_mut().unwrap().history = old;
    let before = s.active().unwrap().doc.clone();
    assert!(s.edit_accounted_preserving_native_alpha("Fill", id, rect, &output, &ctl()).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
}
#[test]
fn real_control_and_quota_failures_keep_owner_history_and_private_fees_atomic() {
    let mut s = fixture(SampleType::F32, Lock::Alpha);
    let id = s.active().unwrap().doc.layers[0].id;
    let rect = Rect::new(0, 0, 257, 2);
    let output = changed_colors(&s, rect); // Warm source cache before fee baseline.
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let q = before.quota().unwrap().clone();
    let baseline = q.usage().unwrap();
    for error in [BackingError::Cancelled, BackingError::Work] {
        let calls = AtomicU64::new(0);
        let work = |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Err(error)
        };
        assert!(s.edit_accounted_preserving_native_alpha("Fill", id, rect, &output, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
        assert!(calls.load(Ordering::Relaxed) > 0);
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert!(history.shares_accounted_history(&s.active().unwrap().history));
        assert_eq!(q.usage().unwrap(), baseline);
    }
    let hold = q.reserve(QuotaKind::Resident, (256 << 20) - baseline.resident).unwrap();
    assert!(s.edit_accounted_preserving_native_alpha("Fill", id, rect, &output, &ctl()).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    drop(hold);
    assert_eq!(q.usage().unwrap(), baseline);
    // A failure after the proof's actual fee exists must also release it.
    let work = |_| if q.usage().map_err(|_| BackingError::Quota)?.resident > baseline.resident { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(s.edit_accounted_preserving_native_alpha("Fill", id, rect, &output, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert_eq!(q.usage().unwrap(), baseline);
}

#[path="accounted_native_alpha_wide_tests.rs"]
mod wide_contracts;
