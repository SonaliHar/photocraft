//! Source text canonicalization for pinned build inputs and native test reconstruction.
//! Downloaded packs are already canonical and must be authenticated without rewriting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextCanonicalError {
    Length,
    Utf8,
    Allocation,
}

/// Collapse CRLF pairs only. Standalone CR, BOM, all Unicode, and other bytes survive.
/// `cap` bounds both the raw input and the allocation; callers choose a fixed cap.
pub fn canonical_text(bytes: &[u8], cap: usize) -> Result<Vec<u8>, TextCanonicalError> {
    if bytes.len() > cap {
        return Err(TextCanonicalError::Length);
    }
    std::str::from_utf8(bytes).map_err(|_| TextCanonicalError::Utf8)?;
    let mut output = Vec::new();
    output.try_reserve_exact(bytes.len()).map_err(|_| TextCanonicalError::Allocation)?;
    let mut previous_cr = false;
    for byte in bytes {
        if previous_cr {
            if *byte == b'\n' {
                output.push(b'\n');
                previous_cr = false;
                continue;
            }
            output.push(b'\r');
        }
        previous_cr = *byte == b'\r';
        if !previous_cr {
            output.push(*byte);
        }
    }
    if previous_cr {
        output.push(b'\r');
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lf_and_crlf_match_without_changing_lone_cr_unicode_or_bom() {
        let lf = "\u{feff}é\t日本語\nα\rβ\n\r".as_bytes();
        let crlf = "\u{feff}é\t日本語\r\nα\rβ\r\n\r".as_bytes();
        assert_eq!(canonical_text(lf, 128).unwrap(), lf);
        assert_eq!(canonical_text(crlf, 128).unwrap(), lf);
        assert_eq!(canonical_text(b"\r\r\n\n", 4).unwrap(), b"\r\n\n");
        assert_eq!(canonical_text(b"", 0).unwrap(), b"");
    }

    #[test]
    fn malformed_or_oversized_text_is_refused_before_allocation() {
        assert_eq!(canonical_text(&[255], 1), Err(TextCanonicalError::Utf8));
        assert_eq!(canonical_text(b"\r\n", 1), Err(TextCanonicalError::Length));
        assert_eq!(canonical_text(b"X\n", 2).unwrap(), b"X\n");
        assert_ne!(canonical_text(b"X\n", 2).unwrap(), canonical_text(b"Y\n", 2).unwrap());
    }
}
