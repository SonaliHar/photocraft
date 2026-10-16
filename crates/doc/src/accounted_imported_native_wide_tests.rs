//! Ignored UNRUN/uncompiled raw-model contracts, not brush arithmetic or source
//! qualification. Existing true-q fixture providers and explicit harness fees.
use super::*;
fn wide_words(sample: SampleType) -> Vec<u8> {
    match sample {
        SampleType::U32 => [0x80000001u32, 0x12345679, 0xfffffffd, 0xc0000001].into_iter().flat_map(u32::to_ne_bytes).collect(),
        SampleType::F64 => [(0.5 + 2f64.powi(-44)).to_bits(), (-0f64).to_bits(), 1, (-0f64).to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect(),
        _ => unreachable!(),
    }
}
fn read(owner: &AccountedDocument, x: i32) -> Vec<u8> {
    let LayerContent::BackedRaster(b) = &owner.document().layers[0].content else { panic!() };
    let mut bytes = vec![0; b.format().bytes_per_pixel()];
    b.checked_view().read_logical_pixel_native_into(x, 0, &mut bytes, &control()).unwrap();
    bytes
}
#[test]
#[ignore = "UNRUN/uncompiled: raw U32/F64 lowbits and signedzero COW, exact alpha/untouched pixels, history no-op preserves redo and original source"]
fn exact_wide_native_words_and_redo_are_not_display_equality() {
    for sample in [SampleType::U32, SampleType::F64] {
        let q = quota();
        let _index = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
        let _resident = q.reserve(QuotaKind::Resident, 2 << 20).unwrap();
        let original = native_model(&q, sample);
        let id = original.document().layers[0].id;
        let rect = Rect::new(0, 0, 1, 1);
        let bytes = wide_words(sample);
        original.validate_imported_native_edit(AccountedEdit::NativePixels { layer: id, rect, bytes: &bytes }, &control()).unwrap();
        let mut history = AccountedHistory::new(original.clone(), 4, &control()).unwrap();
        history.try_commit(AccountedEdit::NativePixels { layer: id, rect, bytes: &bytes }, &control()).unwrap();
        let first = history.current().unwrap().clone();
        assert_eq!(read(&first, 0), bytes);
        assert!(read(&first, 1).iter().all(|b| *b == 0));
        assert!(read(&original, 0).iter().all(|b| *b == 0));
        let LayerContent::BackedRaster(b) = &first.document().layers[0].content else { panic!() };
        assert!(b.default_bytes().iter().all(|v| *v == 0));
        assert_eq!(first.document().xcf_state, original.document().xcf_state);
        assert_eq!(first.document().source_color_context, original.document().source_color_context);
        assert!(Arc::ptr_eq(first.document().icc_profile.as_ref().unwrap(), original.document().icc_profile.as_ref().unwrap()));
        history.undo(&control()).unwrap();
        let zeros = vec![0; bytes.len()];
        history.try_commit(AccountedEdit::NativePixels { layer: id, rect, bytes: &zeros }, &control()).unwrap();
        assert!(history.current().unwrap().shares_model(&original));
        assert!(history.can_redo());
        history.redo(&control()).unwrap();
        assert!(history.current().unwrap().shares_model(&first));
        let mut changed = bytes.clone();
        changed[0] ^= 1;
        let sample_bytes = sample.bytes();
        let alpha = bytes.len() - sample_bytes;
        let second = first.try_edit(AccountedEdit::NativePixels { layer: id, rect, bytes: &changed }, &control()).unwrap();
        assert!(!second.shares_model(&first));
        assert_eq!(read(&second, 0), changed);
        assert_eq!(&read(&second, 0)[sample_bytes..], &bytes[sample_bytes..]);
        assert_eq!(&read(&second, 0)[alpha..], &bytes[alpha..]);
        assert!(second.try_edit(AccountedEdit::NativePixels { layer: id, rect, bytes: &changed }, &control()).unwrap().shares_model(&second));
        assert_eq!(read(&first, 0), bytes);
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: wide raw COW final-unit Work/cancel/Index failure refunds and raw signedzero alpha is a real model edit"]
fn wide_raw_patch_atomic_fences_and_signedzero() {
    for sample in [SampleType::U32, SampleType::F64] {
        let q = quota();
        let _index = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
        let _resident = q.reserve(QuotaKind::Resident, 2 << 20).unwrap();
        let original = native_model(&q, sample);
        let id = original.document().layers[0].id;
        let rect = Rect::new(0, 0, 1, 1);
        let bytes = wide_words(sample);
        let first = original.try_edit(AccountedEdit::NativePixels { layer: id, rect, bytes: &bytes }, &control()).unwrap();
        let mut output = bytes.clone();
        let alpha = output.len() - sample.bytes();
        if sample == SampleType::F64 {
            output[alpha..].copy_from_slice(&0f64.to_ne_bytes());
        } else {
            let last = output.last_mut().unwrap();
            *last ^= 1;
        }
        let total = AtomicU64::new(0);
        let count = |n| {
            total.fetch_add(n, Ordering::Relaxed);
            Ok(())
        };
        let baseline = q.usage().unwrap();
        let edited = first
            .try_edit(AccountedEdit::NativePixels { layer: id, rect, bytes: &output }, &ReadControl { interrupt: &Interrupt::NONE, work: &count })
            .unwrap();
        assert!(!edited.shares_model(&first));
        assert_eq!(read(&edited, 0), output);
        drop(edited);
        assert_eq!(q.usage().unwrap(), baseline);
        let left = AtomicU64::new(total.load(Ordering::Relaxed) - 1);
        let work = |n| left.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
        assert!(
            first.try_edit(AccountedEdit::NativePixels { layer: id, rect, bytes: &output }, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err()
        );
        assert_eq!(q.usage().unwrap(), baseline);
        assert_eq!(read(&first, 0), bytes);
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        assert!(
            first.try_edit(AccountedEdit::NativePixels { layer: id, rect, bytes: &output }, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err()
        );
        assert_eq!(q.usage().unwrap(), baseline);
        let hold = q.reserve(QuotaKind::Index, (8u64 << 20) - baseline.index).unwrap();
        assert!(first.try_edit(AccountedEdit::NativePixels { layer: id, rect, bytes: &output }, &control()).is_err());
        drop(hold);
        assert_eq!(q.usage().unwrap(), baseline);
        assert_eq!(read(&first, 0), bytes);
    }
}
