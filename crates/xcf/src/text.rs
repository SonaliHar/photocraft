//! Bounded stock GimpText configuration syntax, implemented from owned observations.
use crate::{reader::Reader, *};
const TEXT_FIELDS: &[&str] = &[
    "text",
    "markup",
    "font",
    "font-size",
    "font-size-unit",
    "antialias",
    "hinting",
    "hint-style",
    "kerning",
    "language",
    "base-direction",
    "color",
    "justify",
    "indent",
    "line-spacing",
    "letter-spacing",
    "box-mode",
    "box-width",
    "box-height",
    "box-unit",
    "transformation",
    "offset-x",
    "offset-y",
    "outline",
    "outline-direction",
    "outline-width",
    "outline-unit",
    "outline-custom-style",
    "outline-foreground",
    "outline-pattern",
    "outline-cap-style",
    "outline-join-style",
    "outline-miter-limit",
    "outline-antialias",
    "outline-dash-offset",
    "outline-dash-info",
];
fn unique_field(r: &Reader<'_, '_>, seen: &mut u64, key: &str, keys: &[&str], source_offset: u64) -> Result<(), XcfError> {
    if let Some(index) = keys.iter().position(|name| *name == key) {
        let index = u32::try_from(index).map_err(|_| r.error(ErrorKind::Limit, "text field identity"))?;
        let bit = 1u64.checked_shl(index).ok_or_else(|| r.error(ErrorKind::Limit, "text field identity width"))?;
        if *seen & bit != 0 {
            return Err(r.error(ErrorKind::Structure, format!("duplicate text/font field {key} at configuration byte {source_offset}")));
        }
        *seen |= bit;
    }
    Ok(())
}
struct Parser<'r, 'b, 'o, 'd> {
    r: &'r mut Reader<'b, 'o>,
    bytes: &'d [u8],
    pos: usize,
}
impl Parser<'_, '_, '_, '_> {
    fn poll(&mut self) -> Result<(), XcfError> {
        self.r.poll()
    }
    fn byte(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn ws(&mut self) -> Result<(), XcfError> {
        while self.byte().is_some_and(|b| b.is_ascii_whitespace()) {
            if self.pos.is_multiple_of(4096) {
                self.poll()?;
            }
            self.pos += 1;
        }
        Ok(())
    }
    fn text(&mut self, data: &[u8]) -> Result<String, XcfError> {
        let s = std::str::from_utf8(data).map_err(|_| self.r.error(ErrorKind::Structure, "text/config UTF8"))?;
        self.r.charge(data.len())?;
        let mut out = String::new();
        out.try_reserve_exact(data.len()).map_err(|_| self.r.error(ErrorKind::Allocation, "text/config string"))?;
        out.push_str(s);
        Ok(out)
    }
    fn quoted(&mut self) -> Result<Vec<u8>, XcfError> {
        self.pos += 1;
        let start = self.pos;
        let mut scan = start;
        let mut escape = false;
        loop {
            if scan.is_multiple_of(4096) {
                self.poll()?;
            }
            let b = self.bytes.get(scan).copied().ok_or_else(|| self.r.error(ErrorKind::Truncated, "unterminated config quote"))?;
            if b == 0 {
                return Err(self.r.error(ErrorKind::Structure, "unescaped NUL in config quote"));
            }
            if b == b'"' && !escape {
                break;
            }
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            }
            scan += 1;
        }
        let capacity = scan.checked_sub(start).ok_or_else(|| self.r.error(ErrorKind::Structure, "quote extent"))?;
        // Quoted config values include binary sample/profile blobs; string caps
        // apply only after a semantic text/font/markup field is identified.
        if capacity > self.r.limits.property_bytes {
            return Err(self.r.error(ErrorKind::Limit, "config quoted blob byte limit"));
        }
        self.r.charge(capacity)?;
        let mut out = Vec::new();
        out.try_reserve_exact(capacity).map_err(|_| self.r.error(ErrorKind::Allocation, "config quote"))?;
        while self.pos < scan {
            if self.pos.is_multiple_of(4096) {
                self.poll()?;
            }
            let mut b = self.byte().ok_or_else(|| self.r.error(ErrorKind::Truncated, "config quote"))?;
            self.pos += 1;
            if b == b'\\' {
                b = self.byte().ok_or_else(|| self.r.error(ErrorKind::Truncated, "config escape"))?;
                self.pos += 1;
                b = match b {
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'b' => 8,
                    b'f' => 12,
                    b'v' => 11,
                    b'a' => 7,
                    b'\\' => b'\\',
                    b'"' => b'"',
                    b'0'..=b'7' => {
                        let mut v = u16::from(b - b'0');
                        for _ in 0..2 {
                            if self.pos >= scan {
                                break;
                            }
                            if let Some(next @ b'0'..=b'7') = self.byte() {
                                v = v * 8 + u16::from(next - b'0');
                                self.pos += 1;
                            } else {
                                break;
                            }
                        }
                        u8::try_from(v).map_err(|_| self.r.error(ErrorKind::Structure, "config octal exceeds byte"))?
                    }
                    _ => return Err(self.r.error(ErrorKind::Unsupported, "unknown config string escape")),
                };
            }
            out.push(b);
        }
        self.pos = scan + 1;
        Ok(out)
    }
    fn node(&mut self, depth: u64) -> Result<ConfigNode, XcfError> {
        self.poll()?;
        let cap = self.r.budget.map(|b| b.limits().get(BudgetKind::ConfigDepth)).unwrap_or(32);
        if depth > cap {
            return Err(self.r.error(ErrorKind::Limit, "text config depth"));
        }
        self.r.config_entries = self.r.config_entries.checked_add(1).ok_or_else(|| self.r.error(ErrorKind::Limit, "text config count overflow"))?;
        if let Some(b) = self.r.budget {
            b.check(BudgetKind::ConfigEntries, self.r.config_entries, "text config nodes")?;
        } else if self.r.config_entries > 100000 {
            return Err(self.r.error(ErrorKind::Limit, "text config count"));
        }
        self.r.charge(std::mem::size_of::<ConfigNode>())?;
        let start = self.pos;
        let value = match self.byte() {
            Some(b'(') => {
                self.pos += 1;
                let mut nodes = Vec::new();
                loop {
                    self.ws()?;
                    if self.byte() == Some(b')') {
                        self.pos += 1;
                        break;
                    }
                    if self.byte().is_none() {
                        return Err(self.r.error(ErrorKind::Truncated, "unclosed text config list"));
                    }
                    let node = self.node(depth + 1)?;
                    nodes.try_reserve_exact(1).map_err(|_| self.r.error(ErrorKind::Allocation, "text config list"))?;
                    nodes.push(node);
                }
                ConfigValue::List(nodes)
            }
            Some(b'"') => ConfigValue::Quoted(self.quoted()?),
            Some(b')' | 0) | None => return Err(self.r.error(ErrorKind::Structure, "unexpected text config delimiter")),
            Some(_) => {
                while self.byte().is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'(' | b')' | b'"' | 0)) {
                    if self.pos.is_multiple_of(4096) {
                        self.poll()?;
                    }
                    self.pos += 1;
                }
                let data = self.bytes.get(start..self.pos).ok_or_else(|| self.r.error(ErrorKind::Structure, "config atom extent"))?;
                if data.len() > self.r.limits.string_bytes {
                    return Err(self.r.error(ErrorKind::Limit, "config atom bytes"));
                }
                let literal = self.text(data)?;
                if let Ok(number) = literal.parse::<f64>() {
                    if !number.is_finite() {
                        return Err(self.r.error(ErrorKind::Unsupported, "nonfinite config number"));
                    }
                    ConfigValue::Number { literal, bits: number.to_bits() }
                } else {
                    ConfigValue::Atom(literal)
                }
            }
        };
        Ok(ConfigNode { start: start as u64, end: self.pos as u64, value })
    }
    fn quoted_text(&mut self, node: &ConfigNode) -> Result<String, XcfError> {
        let ConfigValue::Quoted(bytes) = &node.value else {
            return Err(self.r.error(ErrorKind::Structure, "text value must be quoted"));
        };
        if bytes.contains(&0) {
            return Err(self.r.error(ErrorKind::Structure, "embedded NUL in text/font string"));
        }
        if bytes.len() > self.r.limits.string_bytes {
            return Err(self.r.error(ErrorKind::Limit, "text/font string byte limit"));
        }
        self.text(bytes)
    }
    fn font(&mut self, args: &[ConfigNode]) -> Result<FontReference, XcfError> {
        let mut f = FontReference::default();
        let mut seen = 0u64;
        if let Some(first) = args.first() {
            f.name = Some(self.quoted_text(first)?);
        }
        for child in args.iter().skip(1) {
            let ConfigValue::List(fields) = &child.value else {
                return Err(self.r.error(ErrorKind::Structure, "font fields must be lists"));
            };
            let Some(ConfigNode { value: ConfigValue::Atom(key), .. }) = fields.first() else {
                return Err(self.r.error(ErrorKind::Structure, "font field name"));
            };
            unique_field(
                self.r,
                &mut seen,
                key,
                &["family", "style", "psname", "fonthash", "fullname", "index", "weight", "slant", "width", "fontversion"],
                child.start,
            )?;
            let target = match key.as_str() {
                "family" => Some(&mut f.family),
                "style" => Some(&mut f.style),
                "psname" => Some(&mut f.postscript_name),
                "fonthash" => Some(&mut f.hash),
                "fullname" => Some(&mut f.name),
                _ => None,
            };
            if let Some(target) = target {
                if fields.len() != 2 {
                    return Err(self.r.error(ErrorKind::Structure, "font field arity"));
                }
                let value = fields.get(1).ok_or_else(|| self.r.error(ErrorKind::Structure, "font value"))?;
                *target = Some(self.quoted_text(value)?);
            } else if matches!(key.as_str(), "index" | "weight" | "slant" | "width" | "fontversion") {
                let Some(ConfigNode { value: ConfigValue::Number { literal, .. }, .. }) = fields.get(1) else {
                    return Err(self.r.error(ErrorKind::Unsupported, "font metadata integer schema"));
                };
                if fields.len() != 2 {
                    return Err(self.r.error(ErrorKind::Structure, "font metadata arity"));
                }
                match key.as_str() {
                    "index" => f.index = Some(literal.parse().map_err(|_| self.r.error(ErrorKind::Structure, "font index"))?),
                    "weight" => f.weight = Some(literal.parse().map_err(|_| self.r.error(ErrorKind::Structure, "font weight"))?),
                    "slant" => f.slant = Some(literal.parse().map_err(|_| self.r.error(ErrorKind::Structure, "font slant"))?),
                    "width" => f.width = Some(literal.parse().map_err(|_| self.r.error(ErrorKind::Structure, "font width"))?),
                    "fontversion" => f.version = Some(literal.parse().map_err(|_| self.r.error(ErrorKind::Structure, "font version"))?),
                    _ => {}
                }
            } else {
                f.unknown_fields.try_reserve_exact(1).map_err(|_| self.r.error(ErrorKind::Allocation, "unknown font fields"))?;
                f.unknown_fields.push(self.text(key.as_bytes())?);
            }
        }
        Ok(f)
    }
}
pub(crate) fn parse<'b, 'o>(r: &mut Reader<'b, 'o>, data: &[u8], parasite_flags: u32) -> Result<TextLayer, XcfError> {
    r.text_bytes = r.text_bytes.checked_add(data.len()).ok_or_else(|| r.error(ErrorKind::Limit, "text bytes overflow"))?;
    if let Some(b) = r.budget {
        b.check(BudgetKind::Text, r.text_bytes as u64, "text parasites")?;
    } else if r.text_bytes > 64 << 20 {
        return Err(r.error(ErrorKind::Limit, "aggregate text bytes"));
    }
    let mut raw = r.vec(data.len())?;
    for (out, input) in raw.chunks_mut(65536).zip(data.chunks(65536)) {
        r.poll()?;
        out.copy_from_slice(input);
    }
    let bytes =
        if data.last() == Some(&0) { data.get(..data.len().saturating_sub(1)).ok_or_else(|| r.error(ErrorKind::Structure, "text extent"))? } else { data };
    let mut p = Parser { r, bytes, pos: 0 };
    let mut configuration = Vec::new();
    let mut text = None;
    let mut markup = None;
    let mut font = None;
    let mut unknown_fields = Vec::new();
    let mut seen = 0u64;
    loop {
        p.ws()?;
        if p.pos == bytes.len() {
            break;
        }
        let node = p.node(0)?;
        let ConfigValue::List(fields) = &node.value else {
            return Err(p.r.error(ErrorKind::Structure, "top-level text settings must be lists"));
        };
        let Some(ConfigNode { value: ConfigValue::Atom(key), .. }) = fields.first() else {
            return Err(p.r.error(ErrorKind::Structure, "text setting name"));
        };
        unique_field(p.r, &mut seen, key, TEXT_FIELDS, node.start)?;
        match key.as_str() {
            "text" | "markup" => {
                let target = if key == "text" { &mut text } else { &mut markup };
                if target.is_some() || fields.len() < 2 || key == "text" && fields.len() != 2 {
                    return Err(p.r.error(ErrorKind::Structure, "duplicate/invalid text or markup"));
                }
                let value = fields.get(1).ok_or_else(|| p.r.error(ErrorKind::Structure, "text value"))?;
                *target = Some(p.quoted_text(value)?);
            }
            "font" => {
                if font.is_some() {
                    return Err(p.r.error(ErrorKind::Structure, "duplicate font"));
                }
                font = Some(p.font(fields.get(1..).ok_or_else(|| p.r.error(ErrorKind::Structure, "font arguments"))?)?);
            }
            "font-size"
            | "font-size-unit"
            | "antialias"
            | "hinting"
            | "hint-style"
            | "kerning"
            | "language"
            | "base-direction"
            | "color"
            | "justify"
            | "indent"
            | "line-spacing"
            | "letter-spacing"
            | "box-mode"
            | "box-width"
            | "box-height"
            | "box-unit"
            | "transformation"
            | "offset-x"
            | "offset-y"
            | "outline"
            | "outline-direction"
            | "outline-width"
            | "outline-unit"
            | "outline-custom-style"
            | "outline-foreground"
            | "outline-pattern"
            | "outline-cap-style"
            | "outline-join-style"
            | "outline-miter-limit"
            | "outline-antialias"
            | "outline-dash-offset"
            | "outline-dash-info" => {}
            _ => {
                unknown_fields.try_reserve_exact(1).map_err(|_| p.r.error(ErrorKind::Allocation, "unknown text fields"))?;
                unknown_fields.push(p.text(key.as_bytes())?);
            }
        }
        configuration.try_reserve_exact(1).map_err(|_| p.r.error(ErrorKind::Allocation, "text configuration"))?;
        configuration.push(node);
    }
    let (content, runs, styles) = if let Some(markup) = &markup {
        crate::markup::parse(p.r, markup)?
    } else {
        let content = p.text(text.as_deref().unwrap_or("").as_bytes())?;
        let mut runs = Vec::new();
        if !content.is_empty() {
            p.r.charge(std::mem::size_of::<TextRun>())?;
            runs.try_reserve_exact(1).map_err(|_| p.r.error(ErrorKind::Allocation, "plain text run"))?;
            runs.push(TextRun { start: 0, end: content.len() as u64, style_ids: Vec::new() });
        }
        (content, runs, Vec::new())
    };
    let layout = crate::text_layout::parse(p.r, &configuration)?;
    if let Some(font) = &font {
        for field in &font.unknown_fields {
            unknown_fields.try_reserve_exact(1).map_err(|_| p.r.error(ErrorKind::Allocation, "unknown required font configuration"))?;
            unknown_fields.push(p.text(field.as_bytes())?);
        }
    }
    Ok(TextLayer { parasite_flags, raw, text, markup, font, configuration, unknown_fields, content, runs, styles, layout })
}

/// Shared GimpConfig syntax: ordered source spans, with no operation inference.
pub(crate) fn tree(r: &mut Reader<'_, '_>, data: &[u8]) -> Result<Vec<ConfigNode>, XcfError> {
    let mut p = Parser { r, bytes: data, pos: 0 };
    let mut nodes = Vec::new();
    loop {
        p.ws()?;
        if p.pos == data.len() {
            break;
        }
        let node = p.node(0)?;
        if !matches!(node.value, ConfigValue::List(_)) {
            return Err(p.r.error(ErrorKind::Structure, "top-level configuration must be lists"));
        }
        nodes.try_reserve_exact(1).map_err(|_| p.r.error(ErrorKind::Allocation, "configuration tree"))?;
        nodes.push(node);
    }
    Ok(nodes)
}
