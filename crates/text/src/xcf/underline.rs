//! Bounded Pango single-underline syntax, independent of cache pixels or fonts.
use super::{TextSourceError, poll};
use photocraft_raster::Interrupt;
use quick_xml::{Reader, events::Event};
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) enum Tag {
    #[default]
    Markup,
    Underline,
    Span,
}
impl Tag {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Markup => "markup",
            Self::Underline => "u",
            Self::Span => "span",
        }
    }
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct Style {
    pub tag: Tag,
    pub underline: Option<bool>,
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct Run {
    pub start: usize,
    pub end: usize,
    pub ids: [usize; 8],
    pub depth: usize,
}
/// Validated byte ranges in a bounded single-line ASCII source, not glyph indices.
#[derive(Clone, Copy, Debug)]
pub struct UnderlineRuns {
    pub(super) bytes: [bool; 8],
    pub(super) styles: [Style; 16],
    pub(super) style_len: usize,
    pub(super) runs: [Run; 8],
    pub(super) run_len: usize,
}
impl UnderlineRuns {
    pub fn underlined_bytes(&self) -> [bool; 8] {
        self.bytes
    }
}
fn unsupported() -> TextSourceError {
    TextSourceError::Unsupported("unqualified underline markup/run semantics")
}
/// No heap allocation, DTD/entity expansion or resource resolution. Each bounded
/// XML event is polled, and input never exceeds1024bytes/eight visible bytes.
pub fn parse_underline_markup(content: &str, markup: &str, ctl: &Interrupt<'_>) -> Result<UnderlineRuns, TextSourceError> {
    poll(ctl)?;
    if !(1..=8).contains(&content.len()) || content.bytes().any(|b| !(32..=126).contains(&b)) || markup.len() > 1024 {
        return Err(unsupported());
    }
    let mut result = UnderlineRuns { bytes: [false; 8], styles: [Style::default(); 16], style_len: 0, runs: [Run::default(); 8], run_len: 0 };
    let mut stack = [0usize; 8];
    let mut depth = 0;
    let mut cursor = 0usize;
    let mut events = 0;
    let mut reader = Reader::from_str(markup);
    reader.config_mut().check_end_names = false; // Fixed stack below checks every closing name.
    loop {
        poll(ctl)?;
        events += 1;
        if events > 64 {
            return Err(unsupported());
        }
        match reader.read_event().map_err(|_| unsupported())? {
            Event::Start(e) => {
                let tag = match e.name().as_ref() {
                    b"markup" => Tag::Markup,
                    b"u" => Tag::Underline,
                    b"span" => Tag::Span,
                    _ => return Err(unsupported()),
                };
                if depth == 0 && (result.style_len != 0 || tag != Tag::Markup) || depth != 0 && tag == Tag::Markup {
                    return Err(unsupported());
                }
                let mut underline = if tag == Tag::Underline { Some(true) } else { None };
                let mut n = 0;
                for attr in e.attributes().with_checks(false) {
                    poll(ctl)?;
                    n += 1;
                    let attr = attr.map_err(|_| unsupported())?;
                    if tag != Tag::Span || n != 1 || attr.key.as_ref() != b"underline" {
                        return Err(unsupported());
                    }
                    underline = Some(match attr.value.as_ref() {
                        b"single" => true,
                        b"none" => false,
                        _ => return Err(unsupported()),
                    });
                }
                if tag == Tag::Span && n != 1 {
                    return Err(unsupported());
                }
                *result.styles.get_mut(result.style_len).ok_or_else(unsupported)? = Style { tag, underline };
                *stack.get_mut(depth).ok_or_else(unsupported)? = result.style_len;
                depth += 1;
                result.style_len += 1;
            }
            Event::End(e) => {
                depth = depth.checked_sub(1).ok_or_else(unsupported)?;
                let style = result.styles.get(*stack.get(depth).ok_or_else(unsupported)?).ok_or_else(unsupported)?;
                if e.name().as_ref() != style.tag.name().as_bytes() {
                    return Err(unsupported());
                }
            }
            Event::Text(e) => {
                if depth == 0 {
                    return Err(unsupported());
                }
                let bytes = e.as_ref();
                if bytes.is_empty() {
                    continue;
                }
                let end = cursor.checked_add(bytes.len()).ok_or_else(unsupported)?;
                if content.as_bytes().get(cursor..end) != Some(bytes) {
                    return Err(unsupported());
                }
                let mut enabled = false;
                for id in stack.get(..depth).ok_or_else(unsupported)? {
                    if let Some(v) = result.styles.get(*id).ok_or_else(unsupported)?.underline {
                        enabled = v;
                    }
                }
                result.bytes.get_mut(cursor..end).ok_or_else(unsupported)?.fill(enabled);
                *result.runs.get_mut(result.run_len).ok_or_else(unsupported)? = Run { start: cursor, end, ids: stack, depth };
                result.run_len += 1;
                cursor = end;
            }
            Event::Eof => break,
            _ => return Err(unsupported()),
        }
    }
    poll(ctl)?;
    if depth != 0 || cursor != content.len() || result.run_len == 0 {
        return Err(unsupported());
    }
    Ok(result)
}
/// Regenerates current glyphs and single underlines from exact-authorized font
/// metrics/shaping. AA16/32 and mono32 are separately observed; mono16 remains
/// unsupported. Caller retains all successful leases until result destruction.
pub fn prepare_underlined_with_charge(
    font: &super::ExactFont<'_>,
    request: super::QualifiedUnderlinedText<'_>,
    max_resident: u64,
    ctl: &Interrupt<'_>,
    resident: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
    work: &mut dyn FnMut(u64) -> Result<(), TextSourceError>,
) -> Result<super::PreparedRectilinear, TextSourceError> {
    super::rectilinear::prepare_underlined(font, request, max_resident, ctl, resident, work)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_runs_overrides_and_neighboring_style_denial() {
        assert_eq!(
            parse_underline_markup("HEFT", "<markup>H<u>EF</u>T</markup>", &Interrupt::NONE).unwrap().bytes,
            [false, true, true, false, false, false, false, false]
        );
        assert_eq!(
            parse_underline_markup("HEFT", "<markup><u>H<span underline=\"none\">EF</span>T</u></markup>", &Interrupt::NONE).unwrap().bytes,
            [true, false, false, true, false, false, false, false]
        );
        for bad in [
            "<markup><u>HE</u><b>FT</b></markup>",
            "<markup><span underline=\"double\">HEFT</span></markup>",
            "<markup><u color=\"red\">HEFT</u></markup>",
            "<!DOCTYPE markup><markup>HEFT</markup>",
            "<markup>HE&amp;FT</markup>",
            "<markup><u>HEFT</markup></u>",
            "<markup><span underline=\"single\" underline=\"none\">HEFT</span></markup>",
        ] {
            assert!(parse_underline_markup("HEFT", bad, &Interrupt::NONE).is_err(), "{bad}");
        }
        assert!(parse_underline_markup("ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â©", "<markup><u>ÃƒÆ’Ã†â€™Ãƒâ€šÃ‚Â©</u></markup>", &Interrupt::NONE).is_err());
        assert_eq!(parse_underline_markup("HEFT", "<markup>HEFT</markup>", &Interrupt::cancel_only(&|| true)).unwrap_err(), TextSourceError::Cancelled);
    }
}
#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn preparation_denies_work_resident_and_cancel_before_output() {
        let font = super::super::ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
        let q = super::super::QualifiedUnderlinedText {
            request: super::super::ScaledText { content: "HEFT", origin: [8, 4], size_px: 16. },
            antialias: super::super::RectilinearAntialias::AreaCoverage,
            runs: parse_underline_markup("HEFT", "<markup><u>HEFT</u></markup>", &Interrupt::NONE).unwrap(),
        };
        let mut resident = 0;
        assert!(matches!(
            prepare_underlined_with_charge(
                &font,
                q,
                8 * 1024 * 1024,
                &Interrupt::NONE,
                &mut |n| {
                    resident += n;
                    Ok(())
                },
                &mut |_| Err(TextSourceError::Budget)
            ),
            Err(TextSourceError::Budget)
        ));
        assert_eq!(resident, 0);
        let mut work = 0;
        assert!(matches!(
            prepare_underlined_with_charge(&font, q, 8 * 1024 * 1024, &Interrupt::NONE, &mut |_| Err(TextSourceError::Budget), &mut |n| {
                work += n;
                Ok(())
            }),
            Err(TextSourceError::Budget)
        ));
        assert_eq!(work, 4096);
        assert!(matches!(
            prepare_underlined_with_charge(&font, q, 8 * 1024 * 1024, &Interrupt::cancel_only(&|| true), &mut |_| Ok(()), &mut |_| Ok(())),
            Err(TextSourceError::Cancelled)
        ));
        let prepared = prepare_underlined_with_charge(&font, q, 8 * 1024 * 1024, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
        assert!(matches!(
            prepared.render_mode_with_charge(q.antialias, 8 * 1024 * 1024, &Interrupt::NONE, &mut |_| Err(TextSourceError::Budget)),
            Err(TextSourceError::Budget)
        ));
    }
}
