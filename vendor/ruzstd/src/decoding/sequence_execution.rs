use super::scratch::DecoderScratch;
use crate::decoding::errors::ExecuteSequencesError;

/// Take the provided decoder and execute the sequences stored within
pub fn execute_sequences(scratch: &mut DecoderScratch) -> Result<(), ExecuteSequencesError> {
    // RFC 8878 block maximum also bounds decoded bytes. Validate the complete
    // sequence plan before any history push/repeat allocation or mutation.
    let limit = scratch.buffer.window_size.min(128 * 1024);
    let oversized = || ExecuteSequencesError::RegeneratedBlockTooLarge { limit };
    let mut regenerated = 0_usize;
    let mut used_literals = 0_usize;
    for sequence in &scratch.sequences {
        used_literals = used_literals.checked_add(sequence.ll as usize).ok_or_else(oversized)?;
        if used_literals > scratch.literals_buffer.len() {
            return Err(ExecuteSequencesError::NotEnoughBytesForSequence { wanted: used_literals, have: scratch.literals_buffer.len() });
        }
        regenerated = regenerated.checked_add(sequence.ll as usize).and_then(|n| n.checked_add(sequence.ml as usize)).ok_or_else(oversized)?;
        if regenerated > limit {
            return Err(oversized());
        }
    }
    regenerated = regenerated.checked_add(scratch.literals_buffer.len() - used_literals).ok_or_else(oversized)?;
    if regenerated > limit {
        return Err(oversized());
    }
    let mut literals_copy_counter = 0;
    let mut next_offset_history = scratch.offset_hist;
    let old_buffer_size = scratch.buffer.len();
    let mut seq_sum = 0;

    for idx in 0..scratch.sequences.len() {
        let seq = scratch.sequences[idx];

        if seq.ll > 0 {
            let high = literals_copy_counter + seq.ll as usize;
            if high > scratch.literals_buffer.len() {
                return Err(ExecuteSequencesError::NotEnoughBytesForSequence { wanted: high, have: scratch.literals_buffer.len() });
            }
            let literals = &scratch.literals_buffer[literals_copy_counter..high];
            literals_copy_counter += seq.ll as usize;

            scratch.buffer.push(literals)?;
        }

        let actual_offset = do_offset_history(seq.of, seq.ll, &mut next_offset_history)?;
        if actual_offset == 0 {
            return Err(ExecuteSequencesError::ZeroOffset);
        }
        if seq.ml > 0 {
            scratch.buffer.repeat(actual_offset as usize, seq.ml as usize)?;
        }

        seq_sum += seq.ml;
        seq_sum += seq.ll;
    }
    if literals_copy_counter < scratch.literals_buffer.len() {
        let rest_literals = &scratch.literals_buffer[literals_copy_counter..];
        scratch.buffer.push(rest_literals)?;
        seq_sum += rest_literals.len() as u32;
    }

    let diff = scratch.buffer.len().checked_sub(old_buffer_size).ok_or(ExecuteSequencesError::RegeneratedSizeMismatch)?;
    if seq_sum as usize != diff {
        return Err(ExecuteSequencesError::RegeneratedSizeMismatch);
    }
    scratch.offset_hist = next_offset_history;
    Ok(())
}

/// Update the most recently used offsets to reflect the provided offset value, and return the
/// "actual" offset needed because offsets are not stored in a raw way, some transformations are needed
/// before you get a functional number.
fn do_offset_history(offset_value: u32, lit_len: u32, scratch: &mut [u32; 3]) -> Result<u32, ExecuteSequencesError> {
    let actual_offset = if lit_len > 0 {
        match offset_value {
            1..=3 => scratch[offset_value as usize - 1],
            _ => {
                //new offset
                offset_value.checked_sub(3).ok_or(ExecuteSequencesError::ZeroOffset)?
            }
        }
    } else {
        match offset_value {
            1..=2 => scratch[offset_value as usize],
            3 => scratch[0].checked_sub(1).ok_or(ExecuteSequencesError::ZeroOffset)?,
            _ => {
                //new offset
                offset_value.checked_sub(3).ok_or(ExecuteSequencesError::ZeroOffset)?
            }
        }
    };

    //update history
    if lit_len > 0 {
        match offset_value {
            1 => {
                //nothing
            }
            2 => {
                scratch[1] = scratch[0];
                scratch[0] = actual_offset;
            }
            _ => {
                scratch[2] = scratch[1];
                scratch[1] = scratch[0];
                scratch[0] = actual_offset;
            }
        }
    } else {
        match offset_value {
            1 => {
                scratch[1] = scratch[0];
                scratch[0] = actual_offset;
            }
            2 => {
                scratch[2] = scratch[1];
                scratch[1] = scratch[0];
                scratch[0] = actual_offset;
            }
            _ => {
                scratch[2] = scratch[1];
                scratch[1] = scratch[0];
                scratch[0] = actual_offset;
            }
        }
    }

    Ok(actual_offset)
}

#[cfg(test)]
mod bounded_tests {
    use super::*;
    use crate::blocks::sequence_section::Sequence;
    #[test]
    fn rejects_overblock_before_ring_mutation() {
        let mut scratch = DecoderScratch::new(1 << 20);
        scratch.buffer.push(&[7]).expect("seed");
        scratch.sequences.push(Sequence { ll: 0, ml: 131073, of: 4 });
        let before = scratch.buffer.len();
        let history = scratch.offset_hist;
        assert!(execute_sequences(&mut scratch).is_err());
        assert_eq!(scratch.buffer.len(), before);
        assert_eq!(scratch.offset_hist, history);
    }
    #[test]
    fn malformed_zero_dictionary_history_and_failed_ring_growth() {
        let mut history = [0, 0, 0];
        assert!(do_offset_history(3, 0, &mut history).is_err());
        assert_eq!(history, [0, 0, 0]);
        assert!(do_offset_history(0, 1, &mut history).is_err());
        let mut ring = crate::decoding::ringbuffer::RingBuffer::new();
        ring.extend(&[1, 2, 3]).expect("seed");
        assert!(ring.reserve(usize::MAX).is_err());
        let (first, second) = ring.as_slices();
        assert_eq!([first, second].concat(), [1, 2, 3]);
    }
    #[test]
    fn rejects_cumulative_overblock_and_literal_tail_before_mutation() {
        let mut scratch = DecoderScratch::new(1 << 20);
        scratch.buffer.push(&[7]).expect("seed");
        scratch.sequences.extend([Sequence { ll: 0, ml: 65536, of: 4 }, Sequence { ll: 0, ml: 65536, of: 4 }]);
        scratch.literals_buffer.push(8);
        assert!(execute_sequences(&mut scratch).is_err());
        assert_eq!(scratch.buffer.len(), 1);
        scratch.literals_buffer.clear();
        execute_sequences(&mut scratch).expect("exact maximum regenerated block");
        assert_eq!(scratch.buffer.len(), 131073);
    }
    #[test]
    fn public_compressed_multiblock_and_window_limit() {
        use std::io::Read;
        let original: alloc::vec::Vec<u8> = (0..500_000).map(|i| ((i / 17 + i % 251) & 255) as u8).collect();
        let encoded = zstd::stream::encode_all(original.as_slice(), 3).expect("independent standard encoder");
        let mut decoder = crate::decoding::StreamingDecoder::new_with_max_window_size(encoded.as_slice(), 4 << 20).expect("bounded frame");
        let mut actual = alloc::vec::Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let count = decoder.read(&mut chunk).expect("valid compressed blocks");
            if count == 0 {
                break;
            }
            actual.extend_from_slice(&chunk[..count]);
        }
        assert_eq!(actual, original);
        assert!(crate::decoding::StreamingDecoder::new_with_max_window_size(encoded.as_slice(), 1024).is_err());
    }
    #[test]
    fn public_overblock_literals_and_unknown_dictionary_fail_without_panic() {
        use std::io::Read;
        // Single-segment FCS=1, compressed last block with RLE literals
        // regenerated=131073, one repeated byte, zero sequences. All transport
        // lengths are complete: the invalidity is regenerated-block size.
        let frame = [0x28, 0xb5, 0x2f, 0xfd, 0xa0, 1, 0, 0, 0, 45, 0, 0, 0x1d, 0, 0x20, 7, 0];
        let mut decoder = crate::decoding::StreamingDecoder::new_with_max_window_size(frame.as_slice(), 4 << 20).expect("frame header");
        assert!(decoder.read(&mut [0_u8; 4096]).is_err());
        let dictionary = [0x28, 0xb5, 0x2f, 0xfd, 0x21, 1, 1, 9, 0, 0, 7];
        assert!(crate::decoding::StreamingDecoder::new_with_max_window_size(dictionary.as_slice(), 4 << 20).is_err());
    }
    #[test]
    fn bounded_complete_malformed_block_matrix_never_panics() {
        use std::io::Read;
        let mut state = 0x29c0_6417_u32;
        for length in 1..=32 {
            for _ in 0..64 {
                let mut frame = alloc::vec![0x28, 0xb5, 0x2f, 0xfd, 0, 0x50];
                let header = ((length as u32) << 3) | 5;
                frame.extend_from_slice(&header.to_le_bytes()[..3]);
                for _ in 0..length {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    frame.push((state >> 24) as u8);
                }
                let outcome = std::panic::catch_unwind(|| {
                    if let Ok(mut decoder) = crate::decoding::StreamingDecoder::new_with_max_window_size(frame.as_slice(), 1 << 20) {
                        let _ = decoder.read(&mut [0_u8; 4096]);
                    }
                });
                assert!(outcome.is_ok(), "complete compressed body length {}, seed {:x}", length, state);
            }
        }
    }
    #[test]
    fn maximum_block_and_sequence_release_measurement() {
        let mut scratch = DecoderScratch::new(16 << 20);
        scratch.buffer.push(&[7]).expect("seed");
        scratch.sequences.resize(131072 / 3, Sequence { ll: 0, ml: 3, of: 4 });
        scratch.literals_buffer.extend_from_slice(&[9, 9]);
        let start = std::time::Instant::now();
        execute_sequences(&mut scratch).expect("valid maximum sequence count and block");
        std::println!("max_sequence_count={} decoded=131072 elapsed_us={}", scratch.sequences.len(), start.elapsed().as_micros());
        assert_eq!(scratch.buffer.len(), 131073);
    }
    #[test]
    fn declared_sixteen_mib_window_exact_lower_oneover() {
        use std::io::Read;
        let frame = [0x28, 0xb5, 0x2f, 0xfd, 0, 0x70, 9, 0, 0, 7];
        assert!(crate::decoding::StreamingDecoder::new_with_max_window_size(frame.as_slice(), (16 << 20) - 1).is_err());
        let mut decoder = crate::decoding::StreamingDecoder::new_with_max_window_size(frame.as_slice(), 16 << 20).expect("exact declared ceiling");
        let mut output = [0_u8; 4096];
        assert_eq!(decoder.read(&mut output).expect("tiny valid raw block"), 1);
        assert_eq!(output[0], 7);
        assert!(crate::decoding::StreamingDecoder::new_with_max_window_size(frame.as_slice(), 0).is_err());
    }
    #[test]
    fn history_fill_control_fails_before_output_publication() {
        let original = alloc::vec![7_u8; 1 << 20];
        let encoded = zstd::stream::encode_all(original.as_slice(), 3).expect("independent compressed frame");
        let mut decoder = crate::decoding::StreamingDecoder::new_with_max_window_size(encoded.as_slice(), 4 << 20).expect("header");
        let mut polls = 0;
        let mut output = [0xa5_u8; 4096];
        let result = decoder.read_with_poll(&mut output, &mut || {
            polls += 1;
            if polls == 5 {
                Err(std::io::Error::other("late block control"))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert_eq!(polls, 5);
        assert_eq!(output, [0xa5; 4096]);
    }
}
