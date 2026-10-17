//! Ignored UNRUN/uncompiled private alpha proof contracts. These raw patches
//! exercise no brush numerical policy and grant no generic alpha-lock bypass.
use super::*;
#[test]
#[ignore = "UNRUN/uncompiled: U32/F64 proof binds exact borrowed raw bytes/model/rect and refunds actual late proof Work/cancel"]
fn wide_proof_identity_is_not_a_reusable_decoded_alpha_flag() {
    for sample in [SampleType::U32, SampleType::F64] {
        let s = fixture(sample, Lock::Alpha);
        let owner = s.active().unwrap().doc.accounted().unwrap();
        let q = owner.quota().clone();
        let _harness = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
        let _index = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
        let id = owner.document().layers[0].id;
        let rect = Rect::new(254, 0, 257, 2);
        let mut bytes = native(&s, rect);
        let bpp = owner.document().pixel_format().bytes_per_pixel();
        for p in bytes.chunks_exact_mut(bpp) {
            p[0] ^= 1;
        }
        let edit = AccountedEdit::NativePixels { layer: id, rect, bytes: &bytes };
        let baseline = q.usage().unwrap();
        let total = AtomicU64::new(0);
        let count = |n| {
            total.fetch_add(n, Ordering::Relaxed);
            Ok(())
        };
        let proof = VerifiedNativeAlpha::verify(owner, edit, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
        assert!(proof.authorizes(&owner.clone(), edit));
        let samebytes = bytes.clone();
        assert!(!proof.authorizes(owner, AccountedEdit::NativePixels { layer: id, rect, bytes: &samebytes }));
        assert!(!proof.authorizes(owner, AccountedEdit::NativePixels { layer: id, rect: Rect::new(0, 0, 3, 2), bytes: &bytes }));
        assert!(!proof.authorizes(owner, AccountedEdit::NativePixels { layer: LayerId(u64::MAX), rect, bytes: &bytes }));
        let renamed = owner.try_edit(AccountedEdit::LayerName { layer: id, name: "other model" }, &ctl()).unwrap();
        assert!(!proof.authorizes(&renamed, edit));
        drop(renamed);
        drop(proof);
        assert_eq!(q.usage().unwrap(), baseline);
        let left = AtomicU64::new(total.load(Ordering::Relaxed) - 1);
        let work = |n| left.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
        assert!(VerifiedNativeAlpha::verify(owner, edit, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
        let work = |_| if q.usage().unwrap().resident > baseline.resident { Err(BackingError::Cancelled) } else { Ok(()) };
        assert!(matches!(VerifiedNativeAlpha::verify(owner, edit, &ReadControl { interrupt: &Interrupt::NONE, work: &work }), Err(BackingError::Cancelled)));
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: real wide lowbit native change preserves raw locked alpha and untouched colors; numeric-equal alpha changes/other locks refuse"]
fn wide_alpha_lock_transaction_preserves_lowbits_signedzero_and_history() {
    for sample in [SampleType::U32, SampleType::F64] {
        let mut s = fixture(sample, Lock::Alpha);
        let before = s.active().unwrap().doc.clone();
        let q = before.quota().unwrap().clone();
        let _harness = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
        let _index = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
        let id = before.layers[0].id;
        let full = before.bounds();
        let rect = Rect::new(254, 0, 257, 2);
        let original = native(&s, full);
        let bpp = before.pixel_format().bytes_per_pixel();
        let sample_bytes = sample.bytes();
        let alpha = bpp - sample_bytes;
        let mut output = native(&s, rect);
        for p in output.chunks_exact_mut(bpp) {
            p[0] ^= 1;
        }
        assert!(s.edit_accounted("raw", AccountedEdit::NativePixels { layer: id, rect, bytes: &output }, &ctl()).is_err());
        s.edit_accounted_preserving_native_alpha("wide raw contract", id, rect, &output, &ctl()).unwrap();
        let after = native(&s, full);
        for (i, (old, new)) in original.chunks_exact(bpp).zip(after.chunks_exact(bpp)).enumerate() {
            assert_eq!(&old[sample_bytes..], &new[sample_bytes..]);
            if i % 257 < 254 {
                assert_eq!(old, new);
            } else {
                assert_eq!(old[0] ^ 1, new[0]);
            }
        }
        assert_eq!(s.active().unwrap().doc.xcf_state, before.xcf_state);
        assert_eq!(s.active().unwrap().doc.source_color_context, before.source_color_context);
        let changed = s.active().unwrap().doc.clone();
        assert!(s.undo_with_control(&ctl()).unwrap());
        let noop = native(&s, rect);
        s.edit_accounted_preserving_native_alpha("noop", id, rect, &noop, &ctl()).unwrap();
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert!(s.redo_with_control(&ctl()).unwrap());
        assert!(changed.shares_state(&s.active().unwrap().doc));
        let mut wrong = native(&s, rect);
        let original_alpha = photocraft_color::try_read_sample_f64(&wrong[..bpp], sample, 3).unwrap();
        if sample == SampleType::F64 {
            wrong[alpha..bpp].copy_from_slice(&0f64.to_ne_bytes());
        } else {
            wrong[alpha..bpp].copy_from_slice(&0xc0000000u32.to_ne_bytes());
        }
        let wrong_alpha = photocraft_color::try_read_sample_f64(&wrong[..bpp], sample, 3).unwrap();
        assert_eq!(original_alpha as f32, wrong_alpha as f32);
        assert!(s.edit_accounted_preserving_native_alpha("wrong alpha", id, rect, &wrong, &ctl()).is_err());
        assert!(changed.shares_state(&s.active().unwrap().doc));
        for lock in [Lock::Content, Lock::Pixels, Lock::All] {
            let mut locked = fixture(sample, lock);
            let rect = Rect::new(0, 0, 1, 1);
            let output = changed_colors(&locked, rect);
            let old = locked.active().unwrap().doc.clone();
            assert!(locked.edit_accounted_preserving_native_alpha("blocked", old.layers[0].id, rect, &output, &ctl()).is_err());
            assert!(old.shares_state(&locked.active().unwrap().doc));
        }
    }
}
