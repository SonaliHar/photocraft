use crate::{reader::Reader, *};
fn scalar(r: &Reader<'_, '_>, values: &[ConfigNode]) -> Result<u64, XcfError> {
    let [ConfigNode { value: ConfigValue::Number { bits, .. }, .. }] = values else {
        return Err(r.error(ErrorKind::Unsupported, "unknown required numeric text setting schema"));
    };
    Ok(*bits)
}
fn boolean(r: &Reader<'_, '_>, values: &[ConfigNode]) -> Result<bool, XcfError> {
    let [ConfigNode { value: ConfigValue::Atom(value), .. }] = values else {
        return Err(r.error(ErrorKind::Unsupported, "unknown required boolean text setting schema"));
    };
    match value.as_str() {
        "yes" => Ok(true),
        "no" => Ok(false),
        _ => Err(r.error(ErrorKind::Unsupported, "unknown text boolean value")),
    }
}
fn string(r: &mut Reader<'_, '_>, values: &[ConfigNode]) -> Result<String, XcfError> {
    let mut out = String::new();
    for node in values {
        let value = match &node.value {
            ConfigValue::Atom(s) => s.as_str(),
            ConfigValue::Quoted(b) => std::str::from_utf8(b).map_err(|_| r.error(ErrorKind::Structure, "text unit/name UTF8"))?,
            _ => return Err(r.error(ErrorKind::Unsupported, "unknown required text name schema")),
        };
        if value.contains('\0') {
            return Err(r.error(ErrorKind::Structure, "embedded NUL in text layout name"));
        }
        let added = value.len().checked_add(1).ok_or_else(|| r.error(ErrorKind::Limit, "text name size"))?;
        if out.len().checked_add(added).is_none_or(|n| n > r.limits.string_bytes) {
            return Err(r.error(ErrorKind::Limit, "text layout name byte limit"));
        }
        r.charge(added)?;
        out.try_reserve_exact(added).map_err(|_| r.error(ErrorKind::Allocation, "text layout name"))?;
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(value);
    }
    if out.is_empty() {
        return Err(r.error(ErrorKind::Unsupported, "empty text layout name"));
    }
    Ok(out)
}
pub(crate) fn parse(r: &mut Reader<'_, '_>, configuration: &[ConfigNode]) -> Result<TextLayout, XcfError> {
    let mut l = TextLayout::default();
    for node in configuration {
        let ConfigValue::List(fields) = &node.value else {
            continue;
        };
        let Some(ConfigNode { value: ConfigValue::Atom(key), .. }) = fields.first() else {
            continue;
        };
        let values = fields.get(1..).ok_or_else(|| r.error(ErrorKind::Structure, "text layout fields"))?;
        match key.as_str() {
            "font-size" => l.font_size_bits = Some(scalar(r, values)?),
            "font-size-unit" => l.font_size_unit = Some(string(r, values)?),
            "antialias" => l.antialias = Some(boolean(r, values)?),
            "hinting" => l.hinting = Some(boolean(r, values)?),
            "language" => l.language = Some(string(r, values)?),
            "base-direction" => l.direction = Some(string(r, values)?),
            "justify" => l.justification = Some(string(r, values)?),
            "indent" => l.indent_bits = Some(scalar(r, values)?),
            "line-spacing" => l.line_spacing_bits = Some(scalar(r, values)?),
            "letter-spacing" => l.letter_spacing_bits = Some(scalar(r, values)?),
            "box-mode" => l.box_mode = Some(string(r, values)?),
            "box-unit" => l.box_unit = Some(string(r, values)?),
            "box-width" => l.box_width_bits = Some(scalar(r, values)?),
            "box-height" => l.box_height_bits = Some(scalar(r, values)?),
            "offset-x" => l.offset_x_bits = Some(scalar(r, values)?),
            "offset-y" => l.offset_y_bits = Some(scalar(r, values)?),
            "transformation" => {
                if values.len() != 4 {
                    return Err(r.error(ErrorKind::Unsupported, "text transformation schema must have four coefficients"));
                }
                let mut matrix = [0; 4];
                for (dst, value) in matrix.iter_mut().zip(values) {
                    *dst = scalar(r, std::slice::from_ref(value))?;
                }
                l.transformation_bits = Some(matrix);
            }
            _ => {}
        }
    }
    Ok(l)
}
