//! Pango markup syntax only; styles remain inert declarations for the layout engine.
use crate::{reader::Reader, *};
use quick_xml::{Reader as XmlReader, events::Event};
use std::io::{self, BufRead, Read};
struct Input<'a, 'b> {
    data: &'a [u8],
    pos: usize,
    budget: Option<&'b Budget<'b>>,
    cancelled: Option<&'b dyn Fn() -> bool>,
}
impl BufRead for Input<'_, '_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if let Some(b) = self.budget {
            b.poll("markup input").map_err(|e| io::Error::other(e.to_string()))?;
        }
        if self.cancelled.is_some_and(|f| f()) {
            return Err(io::Error::other("markup cancelled"));
        }
        let end = self.pos.saturating_add(4096).min(self.data.len());
        self.data.get(self.pos..end).ok_or_else(|| io::Error::other("markup input extent"))
    }
    fn consume(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n).min(self.data.len());
    }
}
impl Read for Input<'_, '_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let input = self.fill_buf()?;
        let n = input.len().min(out.len());
        let dst = out.get_mut(..n).ok_or_else(|| io::Error::other("markup output extent"))?;
        let src = input.get(..n).ok_or_else(|| io::Error::other("markup input extent"))?;
        dst.copy_from_slice(src);
        self.consume(n);
        Ok(n)
    }
}
fn copy(r: &mut Reader<'_, '_>, s: &str) -> Result<String, XcfError> {
    r.charge(s.len())?;
    let mut out = String::new();
    out.try_reserve_exact(s.len()).map_err(|_| r.error(ErrorKind::Allocation, "markup string"))?;
    out.push_str(s);
    Ok(out)
}
fn append(r: &mut Reader<'_, '_>, content: &mut String, runs: &mut Vec<TextRun>, stack: &[usize], s: &str) -> Result<(), XcfError> {
    if s.is_empty() {
        return Ok(());
    }
    r.poll()?;
    let start = content.len();
    r.charge(s.len() + std::mem::size_of::<TextRun>())?;
    content.try_reserve_exact(s.len()).map_err(|_| r.error(ErrorKind::Allocation, "markup content"))?;
    for chunk in s.as_bytes().chunks(4096) {
        r.poll()?;
        if let Some(b) = r.budget {
            b.consume_work(chunk.len() as u64, "markup content")?;
        }
    }
    content.push_str(s);
    let end = content.len();
    let mut style_ids = Vec::new();
    r.charge(std::mem::size_of_val(stack))?;
    style_ids.try_reserve_exact(stack.len()).map_err(|_| r.error(ErrorKind::Allocation, "markup run styles"))?;
    style_ids.extend_from_slice(stack);
    runs.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "markup runs"))?;
    runs.push(TextRun { start: start as u64, end: end as u64, style_ids });
    Ok(())
}
fn allowed_attribute(name: &str) -> bool {
    matches!(
        name,
        "font"
            | "font_desc"
            | "font_family"
            | "face"
            | "font_size"
            | "size"
            | "font_style"
            | "style"
            | "font_weight"
            | "weight"
            | "font_variant"
            | "variant"
            | "font_stretch"
            | "stretch"
            | "font_width"
            | "width"
            | "font_features"
            | "foreground"
            | "fgcolor"
            | "color"
            | "background"
            | "bgcolor"
            | "alpha"
            | "fgalpha"
            | "background_alpha"
            | "bgalpha"
            | "underline"
            | "underline_color"
            | "overline"
            | "overline_color"
            | "rise"
            | "baseline_shift"
            | "font_scale"
            | "strikethrough"
            | "strikethrough_color"
            | "fallback"
            | "lang"
            | "letter_spacing"
            | "gravity"
            | "gravity_hint"
            | "show"
            | "insert_hyphens"
            | "allow_breaks"
            | "line_height"
            | "text_transform"
            | "segment"
    )
}
pub(crate) fn parse(r: &mut Reader<'_, '_>, markup: &str) -> Result<(String, Vec<TextRun>, Vec<MarkupStyle>), XcfError> {
    // Precharge the bounded backend event/name scratch before its infallible internal
    // allocations. read_event_into never grows the pre-reserved input-sized buffer.
    r.charge(markup.len().checked_mul(4).ok_or_else(|| r.error(ErrorKind::Limit, "markup scratch overflow"))?)?;
    let mut buffer = r.vec(markup.len())?;
    buffer.clear();
    let input = Input { data: markup.as_bytes(), pos: 0, budget: r.budget, cancelled: r.options.cancelled };
    let mut xml = XmlReader::from_reader(input);
    xml.config_mut().check_end_names = true;
    xml.config_mut().allow_unmatched_ends = false;
    let mut content = String::new();
    let mut runs = Vec::new();
    let mut styles = Vec::new();
    let mut stack = Vec::new();
    loop {
        r.poll()?;
        r.config_entries = r.config_entries.checked_add(1).ok_or_else(|| r.error(ErrorKind::Limit, "markup event count"))?;
        if let Some(b) = r.budget {
            b.check(BudgetKind::ConfigEntries, r.config_entries, "markup events")?;
        } else if r.config_entries > 100000 {
            return Err(r.error(ErrorKind::Limit, "markup events"));
        }
        buffer.clear();
        let event = xml.read_event_into(&mut buffer).map_err(|_| r.error(ErrorKind::Structure, "malformed markup XML"))?;
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.name();
                let tag = std::str::from_utf8(name.as_ref()).map_err(|_| r.error(ErrorKind::Structure, "markup tag UTF8"))?;
                if !matches!(tag, "markup" | "span" | "b" | "big" | "i" | "s" | "sub" | "sup" | "small" | "tt" | "u") {
                    return Err(r.error(ErrorKind::Unsupported, format!("unknown required markup tag {tag}")));
                }
                let depth = stack.len() as u64 + 1;
                let cap = r.budget.map(|b| b.limits().get(BudgetKind::ConfigDepth)).unwrap_or(32);
                if depth > cap {
                    return Err(r.error(ErrorKind::Limit, "markup depth"));
                }
                r.charge(std::mem::size_of::<MarkupStyle>() + std::mem::size_of::<usize>())?;
                let mut attributes = Vec::new();
                for attr in e.attributes() {
                    r.poll()?;
                    let attr = attr.map_err(|_| r.error(ErrorKind::Structure, "markup attribute"))?;
                    let key = std::str::from_utf8(attr.key.as_ref()).map_err(|_| r.error(ErrorKind::Structure, "markup attribute UTF8"))?;
                    if tag != "span" || !allowed_attribute(key) {
                        return Err(r.error(ErrorKind::Unsupported, format!("unknown required markup attribute {key}")));
                    }
                    r.charge(attr.value.len().saturating_mul(2) + std::mem::size_of::<(String, String)>())?;
                    let value =
                        attr.normalized_value(quick_xml::XmlVersion::Implicit1_0).map_err(|_| r.error(ErrorKind::Structure, "markup attribute entity"))?;
                    let key = copy(r, key)?;
                    let value = copy(r, &value)?;
                    attributes.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "markup attributes"))?;
                    attributes.push((key, value));
                }
                let tag = copy(r, tag)?;
                let id = styles.len();
                styles.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "markup styles"))?;
                styles.push(MarkupStyle { tag, attributes });
                if !empty {
                    stack.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "markup stack"))?;
                    stack.push(id);
                }
            }
            Event::End(_) => {
                if stack.pop().is_none() {
                    return Err(r.error(ErrorKind::Structure, "unmatched markup end"));
                }
            }
            Event::Text(e) => {
                let text = e.xml10_content().map_err(|_| r.error(ErrorKind::Structure, "markup text UTF8"))?;
                append(r, &mut content, &mut runs, &stack, &text)?;
            }
            Event::GeneralRef(e) => {
                let mut temp = [0u8; 4];
                let resolved: &str;
                if let Some(c) = e.resolve_char_ref().map_err(|_| r.error(ErrorKind::Structure, "markup character reference"))? {
                    if c < ' ' && !matches!(c, '\t' | '\r' | '\n') {
                        return Err(r.error(ErrorKind::Structure, "forbidden markup control character"));
                    }
                    resolved = c.encode_utf8(&mut temp);
                } else {
                    let name = e.decode().map_err(|_| r.error(ErrorKind::Structure, "markup entity UTF8"))?;
                    resolved = match name.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => return Err(r.error(ErrorKind::Unsupported, "external/unknown markup entity")),
                    };
                }
                append(r, &mut content, &mut runs, &stack, resolved)?;
            }
            Event::Eof => break,
            Event::DocType(_) => return Err(r.error(ErrorKind::Unsupported, "markup DTD forbidden")),
            Event::CData(_) | Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {
                return Err(r.error(ErrorKind::Unsupported, "unqualified markup XML construct"));
            }
        }
    }
    if !stack.is_empty() {
        return Err(r.error(ErrorKind::Structure, "unclosed markup style"));
    }
    for run in &runs {
        let start = usize::try_from(run.start).map_err(|_| r.error(ErrorKind::Structure, "markup run width"))?;
        let end = usize::try_from(run.end).map_err(|_| r.error(ErrorKind::Structure, "markup run width"))?;
        if start > end || !content.is_char_boundary(start) || !content.is_char_boundary(end) {
            return Err(r.error(ErrorKind::Structure, "markup run UTF8 boundaries"));
        }
    }
    Ok((content, runs, styles))
}
