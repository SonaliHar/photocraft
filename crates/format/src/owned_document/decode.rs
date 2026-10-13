//! STAGED ordinary native object decoder; both compressed and raw Zstd frames.
//! Window bound is applied before decoder initialization. This is a conservative
//! logical decoder reservation, not a measurement of third-party allocator RSS.
use super::accounting::ChargedVec;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind},
};

pub struct ObjectDecodeLimits {
    pub expanded: usize,
    pub window: u64,
}
pub fn decode_owned_object(
    encoded: &[u8],
    expected: Option<usize>,
    hash: [u8; 32],
    limits: ObjectDecodeLimits,
    quota: &BackingQuota,
    control: &ReadControl<'_>,
) -> Result<ChargedVec<u8>, BackingError> {
    if limits.expanded > 16 << 20 || limits.window == 0 || limits.window > 16 << 20 || expected.is_some_and(|n| n > limits.expanded) {
        return Err(BackingError::Length);
    }
    control.check(0)?;
    // Decoder retains history plus current block/output and bounded tables.
    // This scope is intentionally capped; declarations above it fail decoder
    // initialization rather than retrying an uncontrolled legacy decode.
    let bytes = limits
        .window
        .checked_mul(4)
        .and_then(|n| n.checked_add(8 << 20))
        .and_then(|n| n.checked_add(std::mem::size_of::<blake3::Hasher>() as u64))
        .ok_or(BackingError::Length)?;
    let temporary = quota.reserve(QuotaKind::Resident, bytes).map_err(|_| BackingError::Quota)?;
    let mut decoder = ruzstd::decoding::StreamingDecoder::new_with_max_window_size(encoded, limits.window).map_err(|_| BackingError::Integrity)?;
    let check_capacity = |decoder: &ruzstd::decoding::FrameDecoder| -> Result<(), BackingError> {
        let actual = decoder
            .logical_capacity_bytes()
            .ok_or(BackingError::Length)?
            .checked_add(std::mem::size_of::<blake3::Hasher>())
            .and_then(|n| n.checked_add(4096))
            .ok_or(BackingError::Length)?;
        if u64::try_from(actual).map_err(|_| BackingError::Length)? > temporary.bytes() {
            return Err(BackingError::Allocation);
        }
        Ok(())
    };
    check_capacity(&decoder.decoder)?;
    let declared = usize::try_from(decoder.decoder.content_size()).map_err(|_| BackingError::Length)?;
    if declared > limits.expanded || (declared != 0 && expected.is_some_and(|n| n != declared)) {
        return Err(BackingError::Length);
    }
    let capacity = expected.unwrap_or(if declared == 0 { limits.expanded } else { declared });
    let mut result = ChargedVec::with_capacity(capacity, quota, control)?;
    let mut hasher = blake3::Hasher::new();
    let mut chunk = [0_u8; 4096];
    loop {
        control.check(0)?;
        let mut failure = None;
        let count = decoder
            .read_with_poll(&mut chunk, &mut || {
                control.check(1).map_err(|error| {
                    failure = Some(error);
                    std::io::Error::other("owned decode block control")
                })
            })
            .map_err(|_| failure.unwrap_or(BackingError::Integrity))?;
        check_capacity(&decoder.decoder)?;
        if count == 0 {
            break;
        }
        let part = chunk.get(..count).ok_or(BackingError::Length)?;
        result.extend(part, control)?;
        hasher.update(part);
    }
    if expected.is_some_and(|n| n != result.as_slice().len()) || hasher.finalize().as_bytes() != &hash {
        return Err(BackingError::Integrity);
    }
    control.check(0)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_raster::{Interrupt, backing::quota::QuotaLimits};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn history_fill_control_errors_preserve_type_and_refund_before_return() {
        let original = vec![7_u8; 1 << 20];
        // Independent public raw Zstd framing, eight maximum-sized blocks.
        let mut encoded = vec![0x28, 0xb5, 0x2f, 0xfd, 0, 0x60];
        for block in 0..8 {
            let header = (131072_u32 << 3) | u32::from(block == 7);
            encoded.extend_from_slice(&header.to_le_bytes()[..3]);
            encoded.extend_from_slice(&original[..131072]);
        }
        for cancelled in [false, true] {
            let quota = BackingQuota::new(QuotaLimits { resident: 32 << 20, index: 1 << 20, scratch: 0 });
            let polls = AtomicUsize::new(0);
            let flag = AtomicBool::new(false);
            let cancel = || flag.load(Ordering::SeqCst);
            let interrupt = Interrupt::cancel_only(&cancel);
            let work = |units| {
                if units == 1 && polls.fetch_add(1, Ordering::SeqCst) == 4 {
                    if cancelled {
                        flag.store(true, Ordering::SeqCst);
                    } else {
                        return Err(BackingError::Work);
                    }
                }
                Ok(())
            };
            let control = ReadControl { interrupt: &interrupt, work: &work };
            let result = decode_owned_object(
                &encoded,
                Some(original.len()),
                *blake3::hash(&original).as_bytes(),
                ObjectDecodeLimits { expanded: original.len(), window: 4 << 20 },
                &quota,
                &control,
            );
            assert_eq!(result.err(), Some(if cancelled { BackingError::Cancelled } else { BackingError::Work }));
            assert_eq!(polls.load(Ordering::SeqCst), 5);
            assert_eq!(quota.usage().unwrap().resident, 0);
        }
    }
}
