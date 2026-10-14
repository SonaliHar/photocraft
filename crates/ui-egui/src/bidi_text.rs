//! Right-to-left text in labels that egui draws (#5159).
//!
//! egui shapes text in runs of one font face and places the runs left to right. An Arabic or
//! Hebrew word is one run, which the shaper draws right to left, but the space between two words
//! comes from another font, so a phrase came out with every word correct and the word order
//! reversed ("THIS IS A TEXT" read as "TEXT A IS THIS"). [`visual`] puts a label into display
//! order with the Unicode Bidirectional Algorithm before egui lays it out: words of a
//! right-to-left script stay in logical order (the shaper reverses them itself), and everything
//! else in a right-to-left run is reversed around them, with brackets mirrored.
//!
//! For display only: never store the result or put it in an editable field.

use std::borrow::Cow;

use unicode_bidi::{BidiClass, BidiInfo, bidi_class};

/// `text` in the order egui should lay it out from left to right. Text without right-to-left
/// characters comes back unchanged, without allocating.
pub fn visual(text: &str) -> Cow<'_, str> {
    if !text.chars().any(is_rtl) {
        return Cow::Borrowed(text);
    }
    let info = BidiInfo::new(text, None);
    let mut out = String::with_capacity(text.len());
    for para in &info.paragraphs {
        let (levels, runs) = info.visual_runs(para, para.range.clone());
        for run in runs {
            let Some(slice) = text.get(run.clone()) else { continue };
            if levels.get(run.start).is_some_and(|level| level.is_rtl()) {
                push_reversed(slice, &mut out);
            } else {
                out.push_str(slice);
            }
        }
    }
    Cow::Owned(out)
}

/// A strong right-to-left letter, or any character of a right-to-left script block (Arabic
/// punctuation such as "،" belongs with the word it follows: the same font shapes both).
fn is_rtl(c: char) -> bool {
    matches!(bidi_class(c), BidiClass::R | BidiClass::AL) || matches!(c, '\u{0590}'..='\u{08FF}' | '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFC}')
}

/// Characters that stay inside a right-to-left word: its letters, combining marks, and joiners.
fn keeps_with_word(c: char) -> bool {
    is_rtl(c) || matches!(bidi_class(c), BidiClass::NSM | BidiClass::BN)
}

/// Append a right-to-left run in left-to-right display order: right-to-left words stay whole and
/// in logical order, every other character is reversed around them, and brackets are mirrored.
fn push_reversed(run: &str, out: &mut String) {
    let mut units: Vec<&str> = Vec::new();
    let mut word: Option<usize> = None;
    for (i, c) in run.char_indices() {
        if keeps_with_word(c) {
            word.get_or_insert(i);
            continue;
        }
        if let Some(start) = word.take()
            && let Some(w) = run.get(start..i)
        {
            units.push(w);
        }
        if let Some(single) = run.get(i..i + c.len_utf8()) {
            units.push(single);
        }
    }
    if let Some(start) = word
        && let Some(w) = run.get(start..)
    {
        units.push(w);
    }
    for unit in units.into_iter().rev() {
        let mut chars = unit.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if !keeps_with_word(c) => out.push(mirror(c)),
            _ => out.push_str(unit),
        }
    }
}

/// The mirrored form of a bracket shown in right-to-left text.
fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_to_right_text_is_untouched_and_not_copied() {
        for text in ["PhotoCraft", "Layer 1", "", "Background copy (2)", "日本語のレイヤー"] {
            assert!(matches!(visual(text), Cow::Borrowed(t) if t == text), "{text}");
        }
    }

    /// #5159: the reported layer name. Each word stays as typed; the word order flips so that
    /// egui, placing words left to right, shows the first word at the right.
    #[test]
    fn arabic_words_are_placed_right_to_left() {
        assert_eq!(visual("لاشي مكتوب هنا"), "هنا مكتوب لاشي");
        assert_eq!(visual("שלום עולם"), "עולם שלום");
    }

    #[test]
    fn numbers_and_latin_keep_their_own_order() {
        // A number inside right-to-left text reads left to right, at the start of the line.
        assert_eq!(visual("طبقة 2"), "2 طبقة");
        assert_eq!(visual("طبقة 12"), "12 طبقة");
        // A left-to-right name with an Arabic word at the end is left as it is.
        assert_eq!(visual("Layer 1 نص"), "Layer 1 نص");
        // Latin inside an Arabic name stays readable.
        assert_eq!(visual("نص PhotoCraft هنا"), "هنا PhotoCraft نص");
    }

    #[test]
    fn brackets_mirror_and_arabic_punctuation_stays_with_its_word() {
        assert_eq!(visual("نص (1)"), "(1) نص");
        assert_eq!(visual("مرحبا، عالم"), "عالم مرحبا،");
    }
}
