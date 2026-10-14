//! UAX #9 word-order reordering for UI chrome strings (layer names, titles, status text).
//!
//! egui/epaint place font runs left-to-right in stream order and have no bidi reordering, so RTL
//! word order comes out visually reversed. [`visual`] reorders a display string into visual
//! order; canvas rendering and `TextEdit` content stay untouched.

use std::borrow::Cow;

use unicode_bidi::{bidi_class, BidiClass, BidiInfo, LTR_LEVEL};

/// Reorder `text` into visual order for chrome display.
///
/// LTR-only input returns the borrowed input unchanged. The paragraph level is forced LTR — the
/// shell is laid out left-to-right, and this keeps a filename's extension and trailing markers
/// (zoom, depth, the dirty `*`) to the right of the Arabic name, as Photoshop does. RTL runs have
/// their whitespace-separated units emitted in reverse order, each unit's characters kept in
/// logical order (HarfBuzz already reverses glyphs within a word). The result is always a
/// permutation of the input.
pub fn visual(text: &str) -> Cow<'_, str> {
    let has_rtl = text.chars().any(|c| {
        matches!(
            bidi_class(c),
            BidiClass::R | BidiClass::AL | BidiClass::AN | BidiClass::RLE | BidiClass::RLO | BidiClass::RLI
        )
    });
    if !has_rtl {
        return Cow::Borrowed(text);
    }

    let bidi = BidiInfo::new(text, Some(LTR_LEVEL));
    let mut out = String::with_capacity(text.len());
    for para in &bidi.paragraphs {
        let (levels, runs) = bidi.visual_runs(para, para.range.clone());
        for run in runs {
            let level = levels.get(run.start).copied().unwrap_or(LTR_LEVEL);
            let piece = match text.get(run.start..run.end) {
                Some(piece) => piece,
                None => continue,
            };
            if level.is_rtl() {
                push_rtl_reversed(piece, &mut out);
            } else {
                out.push_str(piece);
            }
        }
    }
    Cow::Owned(out)
}

/// Append an RTL run in visual order: alternating whitespace and non-whitespace tokens are
/// reversed as a whole, so a token's characters keep their logical order inside the token.
fn push_rtl_reversed(piece: &str, out: &mut String) {
    let mut tokens: Vec<&str> = Vec::new();
    let mut run_start = 0;
    let mut in_ws: Option<bool> = None;
    for (i, c) in piece.char_indices() {
        let ws = c.is_whitespace();
        if in_ws != Some(ws) {
            if in_ws.is_some()
                && let Some(token) = piece.get(run_start..i)
            {
                tokens.push(token);
            }
            run_start = i;
            in_ws = Some(ws);
        }
    }
    if let Some(token) = piece.get(run_start..) {
        tokens.push(token);
    }
    for token in tokens.iter().rev() {
        out.push_str(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Every output char appears the same number of times as in the input.
    fn assert_permutation(input: &str, output: &str) {
        let count = |s: &str| {
            let mut counts = BTreeMap::new();
            for c in s.chars() {
                *counts.entry(c).or_insert(0u32) += 1;
            }
            counts
        };
        assert_eq!(count(input), count(output));
    }

    /// Arabic words separated by one space: visual order reverses the words, not their letters.
    #[test]
    fn arabic_words_reverse_in_visual_order() {
        let input = "لا شيء";
        let output = visual(input);
        assert_eq!(output, "شيء لا");
        assert_permutation(input, &output);
    }

    #[test]
    fn arabic_multiple_spaces_reverses_words_keeps_count() {
        let input = "واحد   اثنان  ثلاثة";
        let output = visual(input);
        assert_permutation(input, &output);
        let words: Vec<_> = output.split_whitespace().collect();
        assert_eq!(words, ["ثلاثة", "اثنان", "واحد"]);
        let space_count = output.chars().filter(|c| *c == ' ').count();
        assert_eq!(space_count, 5, "whitespace runs survive the reversal");
    }

    #[test]
    fn mixed_latin_arabic_reorders_arabic_run() {
        let input = "foo.psd — لا شيء";
        let output = visual(input);
        assert_permutation(input, &output);
        assert_eq!(output, "foo.psd — شيء لا");
    }

    #[test]
    fn arabic_with_digits_permutes() {
        let input = "الطبقة 5 نسخة";
        let output = visual(input);
        assert_permutation(input, &output);
        assert!(output.contains("نسخة"));
        assert!(output.contains('5'));
    }

    #[test]
    fn ltr_input_is_borrowed_passthrough() {
        let input = "Layer 1 — foo.psd";
        assert!(matches!(visual(input), Cow::Borrowed(s) if s == input));
    }

    #[test]
    fn empty_input_is_borrowed() {
        assert!(matches!(visual(""), Cow::Borrowed("")));
    }

    #[test]
    fn controls_only_permutes() {
        let input = "\u{200f}\u{200e}\t\u{202c}";
        let output = visual(input);
        assert_permutation(input, &output);
    }

    #[test]
    fn multi_byte_boundaries_are_safe() {
        let input = "🦀 لا شيء 🦀";
        let output = visual(input);
        assert_permutation(input, &output);
        assert!(output.starts_with("🦀"));
        assert!(output.ends_with("🦀"));
    }

    /// A mixed filename keeps its extension and trailing markers to the right of the Arabic
    /// name; only the Arabic words reorder.
    #[test]
    fn mixed_filename_keeps_extension_and_tail_right() {
        let input = "صورة لا شيء.jpg @ 86.2% (RGB/8)*";
        let output = visual(input);
        assert_permutation(input, &output);
        assert_eq!(output, "شيء لا صورة.jpg @ 86.2% (RGB/8)*");
    }

    #[test]
    fn every_output_is_a_permutation() {
        for input in [
            "",
            " ",
            "a",
            "لا",
            "لا شيء",
            "foo.psd — لا شيء",
            "5 + 5 = 10",
            "  شيء لا  ",
            "نص\nسطر آخر",
            "مِلْف ٣، صورة",
            "خريطة\tطبقة\nقناع",
        ] {
            let output = visual(input);
            assert_permutation(input, &output);
        }
    }
}
