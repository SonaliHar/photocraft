//! Exact retained text raster switches, separate from a resolved hinting engine.
//! A source boolean does not identify an interpreter/autohinter, target, or grid.
use super::{TextSourceError, poll};
use photocraft_doc::{
    xcf_filters::{FilterArgument, FilterValue},
    xcf_items::{EditableSource, SourceItem},
};
use photocraft_raster::Interrupt;
type Result<T> = std::result::Result<T, TextSourceError>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceRasterPolicy {
    pub hinting: bool,
    pub antialias: bool,
}
/// Opaque capability for this candidate's existing unhinted area-coverage policy.
/// No source-render qualification, font authority or resource grant is implied.
#[derive(Clone, Copy, Debug)]
pub struct UnhintedAreaPolicy {
    _private: (),
}
/// Closed candidate coverage algorithm selected from complete validated source policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoveragePolicy {
    Area,
    PixelCenterNonZero,
}
#[derive(Clone, Copy, Debug)]
pub struct UnhintedRasterPolicy {
    coverage: CoveragePolicy,
}
impl UnhintedRasterPolicy {
    pub fn coverage(self) -> CoveragePolicy {
        self.coverage
    }
}
impl SourceRasterPolicy {
    pub fn require_unhinted_area(self) -> Result<UnhintedAreaPolicy> {
        if self.hinting {
            return Err(TextSourceError::Unsupported("hinted source requires explicit engine, target, metrics and two-axis grid policy"));
        }
        if !self.antialias {
            return Err(TextSourceError::Unsupported("non-antialiased source requires a separately implemented binary coverage policy"));
        }
        Ok(UnhintedAreaPolicy { _private: () })
    }
}
fn malformed() -> TextSourceError {
    TextSourceError::Unsupported("missing, duplicate or wrongly typed retained text raster policy")
}
/// Borrowed bounded canonical fields only; no inference from opaque AST, source
/// cache pixels or font family. Unknown unrelated fields stay inert for their
/// own schema validator; these two switches must each occur exactly once.
pub fn read_layout(fields: &[FilterArgument], control: &Interrupt<'_>) -> Result<SourceRasterPolicy> {
    poll(control)?;
    if fields.len() > 64 {
        return Err(TextSourceError::Budget);
    }
    let mut hinting = None;
    let mut antialias = None;
    for f in fields {
        poll(control)?;
        if f.name.len() > 128 {
            return Err(TextSourceError::Budget);
        }
        let target = match f.name.as_str() {
            "hinting" => &mut hinting,
            "antialias" => &mut antialias,
            _ => continue,
        };
        if target.is_some() {
            return Err(malformed());
        }
        let FilterValue::Bool(value) = &f.value else {
            return Err(malformed());
        };
        *target = Some(*value);
    }
    Ok(SourceRasterPolicy { hinting: hinting.ok_or_else(malformed)?, antialias: antialias.ok_or_else(malformed)? })
}
/// Exact outer AST policy fields; absence is retained as absence, never `none`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceRasterPolicyDetails<'a> {
    pub switches: SourceRasterPolicy,
    pub hint_style: Option<&'a str>,
    pub raw_hinting: Option<&'a str>,
    pub raw_antialias: Option<&'a str>,
}
impl SourceRasterPolicyDetails<'_> {
    pub fn require_unhinted(self) -> Result<UnhintedRasterPolicy> {
        if self.hint_style != Some("none") {
            return Err(TextSourceError::Unsupported("missing or unsupported retained hint-style policy"));
        }
        for (raw, expected) in [(self.raw_hinting, self.switches.hinting), (self.raw_antialias, self.switches.antialias)] {
            if raw != Some(if expected { "yes" } else { "no" }) {
                return Err(TextSourceError::Unsupported("incomplete or contradictory retained raster switches"));
            }
        }
        if self.switches.hinting {
            return Err(TextSourceError::Unsupported("hinted source requires explicit engine, target, metrics and two-axis grid policy"));
        }
        Ok(UnhintedRasterPolicy { coverage: if self.switches.antialias { CoveragePolicy::Area } else { CoveragePolicy::PixelCenterNonZero } })
    }
    pub fn require_unhinted_area(self) -> Result<UnhintedAreaPolicy> {
        let policy = self.require_unhinted()?;
        if policy.coverage() != CoveragePolicy::Area {
            return Err(TextSourceError::Unsupported("source selects binary coverage rather than area"));
        }
        Ok(UnhintedAreaPolicy { _private: () })
    }
}
fn raw_policy<'a>(raw: &'a [FilterArgument], control: &Interrupt<'_>) -> Result<(Option<&'a str>, Option<&'a str>, Option<&'a str>)> {
    if raw.len() > 256 {
        return Err(TextSourceError::Budget);
    }
    let mut style = None;
    let mut hinting = None;
    let mut antialias = None;
    for (index, argument) in raw.iter().enumerate() {
        poll(control)?;
        if argument.name.len() > 128 {
            return Err(TextSourceError::Budget);
        }
        let Some((kind, suffix)) = argument.name.rsplit_once('-') else {
            return Err(malformed());
        };
        if suffix.is_empty()
            || !suffix.bytes().all(|b| b.is_ascii_digit())
            || suffix.parse::<usize>().ok() != Some(index)
            || suffix.len() > 1 && suffix.starts_with('0')
        {
            return Err(malformed());
        }
        if kind != "list" {
            continue;
        }
        let FilterValue::Config(list) = &argument.value else {
            return Err(malformed());
        };
        let Some(first) = list.first() else {
            continue;
        };
        if first.name != "atom-0" {
            continue;
        }
        let FilterValue::String(key) = &first.value else {
            continue;
        };
        let target = match key.as_str() {
            "hint-style" => &mut style,
            "hinting" => &mut hinting,
            "antialias" => &mut antialias,
            _ => continue,
        };
        if target.is_some() || list.len() != 2 {
            return Err(malformed());
        }
        let value = list.get(1).ok_or_else(malformed)?;
        if value.name != "atom-1" {
            return Err(malformed());
        }
        let FilterValue::String(token) = &value.value else {
            return Err(malformed());
        };
        if token.is_empty() || token.len() > 128 || token.contains('\0') {
            return Err(malformed());
        }
        *target = Some(token.as_str());
    }
    Ok((style, hinting, antialias))
}
/// Read canonical switches AND exact outer AST hint-style/legacy switches.
/// Unknown style strings remain inspectable; candidate policy selection refuses
/// them explicitly without guessing an Engine, Target or deprecated-flag priority.
pub fn read_source<'a>(item: &'a SourceItem, control: &Interrupt<'_>) -> Result<SourceRasterPolicyDetails<'a>> {
    poll(control)?;
    let EditableSource::Text { configuration, .. } = &item.source else {
        return Err(malformed());
    };
    if configuration.len() > 64 {
        return Err(TextSourceError::Budget);
    }
    let mut layout = None;
    let mut raw = None;
    for f in configuration {
        poll(control)?;
        if f.name.len() > 128 {
            return Err(TextSourceError::Budget);
        }
        let target = match f.name.as_str() {
            "layout" => &mut layout,
            "gimp-text-configuration" => &mut raw,
            _ => continue,
        };
        if target.is_some() {
            return Err(malformed());
        }
        let FilterValue::Config(fields) = &f.value else {
            return Err(malformed());
        };
        *target = Some(fields.as_slice());
    }
    let switches = read_layout(layout.ok_or_else(malformed)?, control)?;
    let (hint_style, raw_hinting, raw_antialias) = raw_policy(raw.ok_or_else(malformed)?, control)?;
    Ok(SourceRasterPolicyDetails { switches, hint_style, raw_hinting, raw_antialias })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_unhinted_fixture_requires_explicit_none_style() {
        let raw = [
            FilterArgument {
                name: "list-0".into(),
                value: FilterValue::Config(vec![
                    FilterArgument { name: "atom-0".into(), value: FilterValue::String("hint-style".into()) },
                    FilterArgument { name: "atom-1".into(), value: FilterValue::String("none".into()) },
                ]),
            },
            FilterArgument {
                name: "list-1".into(),
                value: FilterValue::Config(vec![
                    FilterArgument { name: "atom-0".into(), value: FilterValue::String("hinting".into()) },
                    FilterArgument { name: "atom-1".into(), value: FilterValue::String("no".into()) },
                ]),
            },
            FilterArgument {
                name: "list-2".into(),
                value: FilterValue::Config(vec![
                    FilterArgument { name: "atom-0".into(), value: FilterValue::String("antialias".into()) },
                    FilterArgument { name: "atom-1".into(), value: FilterValue::String("yes".into()) },
                ]),
            },
        ];
        let (hint_style, raw_hinting, raw_antialias) = raw_policy(&raw, &Interrupt::NONE).unwrap();
        let details = SourceRasterPolicyDetails { switches: SourceRasterPolicy { hinting: false, antialias: true }, hint_style, raw_hinting, raw_antialias };
        assert!(details.require_unhinted_area().is_ok());
        assert!(SourceRasterPolicyDetails { hint_style: None, ..details }.require_unhinted_area().is_err());
        assert!(SourceRasterPolicyDetails { hint_style: Some("future-style"), ..details }.require_unhinted_area().is_err());
    }
    #[test]
    fn exact_switches_remain_readable_when_hinted_candidate_is_pending() {
        let fields = vec![
            FilterArgument { name: "hinting".into(), value: FilterValue::Bool(true) },
            FilterArgument { name: "antialias".into(), value: FilterValue::Bool(true) },
        ];
        let policy = read_layout(&fields, &Interrupt::NONE).unwrap();
        assert_eq!(policy, SourceRasterPolicy { hinting: true, antialias: true });
        assert!(policy.require_unhinted_area().is_err());
    }
    #[test]
    fn missing_wrong_type_and_binary_are_not_area_defaults() {
        assert!(read_layout(&[], &Interrupt::NONE).is_err());
        let fields = vec![
            FilterArgument { name: "hinting".into(), value: FilterValue::Bool(false) },
            FilterArgument { name: "antialias".into(), value: FilterValue::String("yes".into()) },
        ];
        assert!(read_layout(&fields, &Interrupt::NONE).is_err());
        assert!(SourceRasterPolicy { hinting: false, antialias: false }.require_unhinted_area().is_err());
    }
}
