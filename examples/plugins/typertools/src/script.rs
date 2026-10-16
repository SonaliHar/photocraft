//! Parsing a lettering script: one line of dialogue per row, tagged by a style prefix.
//!
//! A letterer pastes a chapter into the panel and writes it the way they already write it:
//!
//! ```text
//! d: I'll be home by six.
//! ## (a note to myself, not dialogue)
//! s: You said that yesterday.
//! ```
//!
//! The first token that matches a style's prefix picks that style for the line; a line whose prefix
//! is in the ignore list, or that is blank once its prefixes are off, is not dialogue and is skipped
//! by the line navigation. What follows ports TyperTools 1.4.9's rules exactly, including the ones
//! that surprise people:
//!
//! * matching is **case sensitive** and against the **raw, untrimmed** line, so ` d:` on an indented
//!   line does not match the prefix `d:` — it matches nothing;
//! * the **first** prefix in library order wins, not the longest: a `#` prefix on an earlier style
//!   shadows `##` on a later one;
//! * exactly **one** style prefix is taken off a line.
//!
//! One rule is deliberately *not* ported: TyperTools splits on `\n` alone, so a script pasted from
//! Windows keeps a `\r` at the end of every line. That is invisible in the text (it is trimmed) but
//! it can hide a line's last character from anything that inspects the raw line, and there is no
//! reason to reproduce it. Carriage returns are stripped here, and a test says so.

/// One row of the script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// The line as it was typed, without its line ending.
    pub raw: String,
    /// The dialogue: the raw line with its ignore prefix and its style prefix taken off, trimmed.
    pub text: String,
    /// The style the line's prefix names, if any prefix matched.
    pub style_id: Option<String>,
    /// The ignore prefix that matched, if one did.
    pub ignore_prefix: Option<String>,
    /// Not dialogue: it carries an ignore prefix, or nothing is left of it.
    pub ignored: bool,
    /// The line's number among the dialogue lines, from 1. Zero for an ignored line — which is why
    /// it is not a simple count of the rows above.
    pub number: usize,
}

/// A prefix that picks a style, in library order: `(style id, prefix)`.
pub type StylePrefix = (String, String);

/// Splits `text` into lines. Only `\n` separates lines, as in TyperTools; a lone `\r` (an old Mac
/// file, or a stray one) is treated as one too rather than left inside the line.
pub fn split(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l))
}

/// Parses a whole script.
///
/// `style_prefixes` must already be in the library's order — unassigned styles first, then each
/// folder's, and each style's prefixes in the order they were typed. The order is the matching
/// order, so it is the caller's ([`crate::library::Library::prefix_order`]).
pub fn parse(text: &str, ignore_prefixes: &[String], style_prefixes: &[StylePrefix]) -> Vec<Line> {
    let mut number = 0;
    split(text)
        .map(|raw| {
            let ignore_prefix = ignore_prefixes.iter().find(|p| !p.is_empty() && raw.starts_with(p.as_str())).cloned();
            let matched = style_prefixes.iter().find(|(_, p)| !p.is_empty() && raw.starts_with(p.as_str()));
            let mut rest = raw;
            if let Some(p) = &ignore_prefix {
                rest = rest.strip_prefix(p.as_str()).unwrap_or(rest);
            }
            if let Some((_, p)) = matched {
                rest = rest.strip_prefix(p.as_str()).unwrap_or(rest);
            }
            let text = rest.trim().to_string();
            let ignored = ignore_prefix.is_some() || text.is_empty();
            if !ignored {
                number += 1;
            }
            Line { raw: raw.to_string(), text, style_id: matched.map(|(id, _)| id.clone()), ignore_prefix, ignored, number: if ignored { 0 } else { number } }
        })
        .collect()
}

/// The number of dialogue lines: what the line field counts up to.
pub fn count(lines: &[Line]) -> usize {
    lines.iter().filter(|l| !l.ignored).count()
}

/// The raw index of the `n`th dialogue line, counting from 0 — and its neighbours, for the ← and →
/// buttons. Both skip ignored lines, and neither runs off either end.
pub fn nth(lines: &[Line], n: usize) -> Option<usize> {
    lines.iter().enumerate().filter(|(_, l)| !l.ignored).map(|(i, _)| i).nth(n)
}

/// The next dialogue line after `raw`, or `None` at the end.
pub fn next(lines: &[Line], raw: usize) -> Option<usize> {
    ((raw + 1)..lines.len()).find(|&i| !lines[i].ignored)
}

/// The previous dialogue line before `raw`, or `None` at the start.
pub fn previous(lines: &[Line], raw: usize) -> Option<usize> {
    (0..raw).rev().find(|&i| !lines[i].ignored)
}

/// Where the line field should point when the script changed: the same row, as long as the script
/// still has one, and the first dialogue line when it does not.
///
/// A row is kept even when it is not dialogue. The letterer is looking at it, and typing above it
/// must not move it; an ignored row is also a real answer — "this line is a stage direction" — which
/// is worth saying rather than stepping over. The arrows and the line field are what skip them.
pub fn settle(lines: &[Line], wanted: usize) -> Option<usize> {
    match lines.get(wanted) {
        Some(_) => Some(wanted),
        None => nth(lines, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefixes() -> Vec<StylePrefix> {
        vec![("dialogue".into(), "d:".into()), ("shout".into(), "#".into()), ("whisper".into(), "##".into()), ("thought".into(), "t:".into())]
    }

    fn ignored() -> Vec<String> {
        vec!["##".into()]
    }

    fn parsed(text: &str) -> Vec<Line> {
        parse(text, &ignored(), &prefixes())
    }

    #[test]
    fn numbers_count_dialogue_lines_only() {
        let lines = parsed("d: One\n\n## a note\nd: Two\nt: Three");
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0].number, 1);
        assert!(lines[1].ignored, "a blank line is not dialogue");
        assert_eq!(lines[1].number, 0);
        assert!(lines[2].ignored);
        assert_eq!(lines[3].number, 2, "the numbering ignores the rows above it");
        assert_eq!(lines[4].number, 3);
        assert_eq!(count(&lines), 3);
    }

    #[test]
    fn a_prefix_is_taken_off_the_line_and_names_the_style() {
        let lines = parsed("d: I'll be home by six.\n t: indented");
        assert_eq!(lines[0].text, "I'll be home by six.");
        assert_eq!(lines[0].style_id.as_deref(), Some("dialogue"));
        // Leading space: the raw line does not start with `t:`, so no prefix matches and the whole
        // line is the dialogue. This is TyperTools' behaviour, and it is *not* a bug to fix: a
        // letterer indenting a line means to indent it.
        assert_eq!(lines[1].text, "t: indented");
        assert_eq!(lines[1].style_id, None);
        assert!(!lines[1].ignored);
    }

    #[test]
    fn the_first_prefix_in_library_order_wins_not_the_longest() {
        // `#` (shout) is listed before `##` (whisper), so `##` never wins as a *style*: the line is a
        // shout whose dialogue begins with `#`. TyperTools has the same shadowing, and a library that
        // wants `##` must list it first.
        //
        // The ignore prefix is taken off first, though, so on this line — which `##` ignores — the
        // `#` goes with it, and nothing of the shadowing is left to see. That is the second case.
        let lines = parsed("## Careful!");
        assert_eq!(lines[0].style_id.as_deref(), Some("shout"));
        assert_eq!(lines[0].text, "Careful!");
        assert!(lines[0].ignored, "and the ignore prefix `##` takes precedence over the style");

        // With the ignore list out of the way the shadowing is plain to see.
        let lines = parse("## Careful!", &[], &prefixes());
        assert_eq!(lines[0].style_id.as_deref(), Some("shout"));
        assert_eq!(lines[0].text, "# Careful!");
        assert!(!lines[0].ignored);
    }

    #[test]
    fn an_ignored_line_keeps_its_text_for_the_pane_but_is_not_dialogue() {
        let lines = parsed("## (later: the reveal goes here)");
        assert!(lines[0].ignored);
        assert_eq!(lines[0].ignore_prefix.as_deref(), Some("##"));
        assert_eq!(lines[0].text, "(later: the reveal goes here)");
        assert_eq!(nth(&lines, 0), None);
    }

    #[test]
    fn a_line_of_nothing_but_a_prefix_is_blank() {
        let lines = parsed("d:\n\n   \n##\n");
        assert!(lines.iter().all(|l| l.ignored), "{lines:?}");
        assert_eq!(count(&lines), 0);
        // The row is still there to point at — an empty script is one blank row, not no rows — and
        // pointing at it is what makes "this line is empty" the answer rather than "no line here".
        assert_eq!(settle(&lines, 3), Some(3));
        assert_eq!(settle(&parsed(""), 0), Some(0), "an empty script is one blank row");
    }

    #[test]
    fn navigation_skips_the_lines_that_are_not_dialogue() {
        let lines = parsed("d: One\n## note\nd: Two\n\nd: Three");
        assert_eq!(nth(&lines, 0), Some(0));
        assert_eq!(nth(&lines, 1), Some(2));
        assert_eq!(nth(&lines, 2), Some(4));
        assert_eq!(nth(&lines, 3), None);
        assert_eq!(next(&lines, 0), Some(2));
        assert_eq!(next(&lines, 2), Some(4));
        assert_eq!(next(&lines, 4), None);
        assert_eq!(previous(&lines, 4), Some(2));
        assert_eq!(previous(&lines, 0), None);
    }

    #[test]
    fn a_position_that_is_gone_falls_back_to_the_first_line() {
        let lines = parsed("d: One\nd: Two");
        assert_eq!(settle(&lines, 1), Some(1), "a position that still holds is kept");
        assert_eq!(settle(&lines, 7), Some(0), "one that does not is the first line");
        // A row that is not dialogue is still a row the letterer is looking at: typing above it must
        // not move it, and Create has something worth saying about it.
        let with_blank = parsed("d: One\n\nd: Two");
        assert_eq!(settle(&with_blank, 1), Some(1), "an ignored row is kept where it is");
        assert_eq!(settle(&parsed("\n\nd: Two"), 0), Some(0), "even at the top of the script");
    }

    #[test]
    fn a_script_pasted_from_windows_has_no_carriage_returns() {
        // TyperTools splits on `\n` and leaves the `\r` in the line; a `\r` is not dialogue here.
        let lines = parsed("d: One\r\nd: Two\r\n");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].raw, "d: One");
        assert_eq!(lines[1].text, "Two");
        assert!(lines[2].ignored, "the trailing newline leaves one empty row");
    }

    #[test]
    fn a_prefix_longer_than_the_line_does_not_match() {
        let lines = parsed("d");
        assert_eq!(lines[0].text, "d");
        assert_eq!(lines[0].style_id, None);
        let lines = parsed("d:");
        assert!(lines[0].ignored, "prefix, nothing left");
        assert_eq!(lines[0].style_id.as_deref(), Some("dialogue"));
    }
}
