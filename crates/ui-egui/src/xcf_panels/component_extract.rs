use photocraft_doc::xcf_filters::{FilterArgument, FilterValue};

pub(super) const DOMAIN: &str = "gegl:component-extract/component";
pub(super) const EPOCH: &str = "xcf-22-stock-component-extract-four-precision-v1";
pub(super) const CMYK_EPOCH: &str = "xcf-22-stock-component-extract-cmyk-precision-v1";
pub(super) const YCBCR_EPOCH: &str = "xcf-22-stock-component-extract-ycbcr-precision-v1";
pub(super) const GEOMETRY_EPOCH: &str = "xcf-22-stock-component-extract-geometry-v1";

pub(super) fn allowed_component(epoch: &str, domain: &str, value: i32) -> bool {
    domain == DOMAIN
        && match epoch {
            EPOCH => matches!(value, 0 | 1 | 2 | 20),
            CMYK_EPOCH => (8..=11).contains(&value),
            YCBCR_EPOCH => (12..=14).contains(&value),
            GEOMETRY_EPOCH => matches!(value, 0 | 1 | 2 | 8..=14 | 20),
            _ => false,
        }
}
pub(super) fn valid_arguments_for_epoch(epoch: &str, arguments: &[FilterArgument]) -> bool {
    arguments.len() == 3
        && arguments.iter().any(|argument| {
            argument.name == "component" && matches!(&argument.value, FilterValue::Enum { domain, value } if allowed_component(epoch, domain, *value))
        })
        && ["invert", "linear"].iter().all(|name| arguments.iter().any(|argument| argument.name == *name && matches!(argument.value, FilterValue::Bool(_))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn component_schema_rejects_unqualified_domains_and_shapes() {
        let valid = vec![
            FilterArgument { name: "component".into(), value: FilterValue::Enum { domain: DOMAIN.into(), value: 0 } },
            FilterArgument { name: "invert".into(), value: FilterValue::Bool(false) },
            FilterArgument { name: "linear".into(), value: FilterValue::Bool(false) },
        ];
        for (epoch, values) in [(EPOCH, vec![0, 1, 2, 20]), (CMYK_EPOCH, vec![8, 9, 10, 11]), (YCBCR_EPOCH, vec![12, 13, 14])] {
            for value in values {
                let mut arguments = valid.clone();
                arguments[0].value = FilterValue::Enum { domain: DOMAIN.into(), value };
                assert!(valid_arguments_for_epoch(epoch, &arguments));
                arguments.reverse();
                assert!(valid_arguments_for_epoch(epoch, &arguments));
                for other in [EPOCH, CMYK_EPOCH, YCBCR_EPOCH, "unknown"] {
                    if other != epoch {
                        assert!(!valid_arguments_for_epoch(other, &arguments));
                    }
                }
            }
            for value in [-1, 3, 4, 5, 6, 7, 15, 16, 17, 18, 19, 21, i32::MAX] {
                let mut arguments = valid.clone();
                arguments[0].value = FilterValue::Enum { domain: DOMAIN.into(), value };
                assert!(!valid_arguments_for_epoch(epoch, &arguments));
            }
        }
        assert!(!valid_arguments_for_epoch(EPOCH, &valid[..2]));
        for index in 0..3 {
            let mut unknown = valid.clone();
            unknown[index].name = "unknown".into();
            assert!(!valid_arguments_for_epoch(EPOCH, &unknown));
            let mut duplicate = valid.clone();
            duplicate.push(valid[index].clone());
            assert!(!valid_arguments_for_epoch(EPOCH, &duplicate));
            let mut wrong = valid.clone();
            wrong[index].value = FilterValue::String("0".into());
            assert!(!valid_arguments_for_epoch(EPOCH, &wrong));
        }
        let mut wrong = valid;
        wrong[0].value = FilterValue::Enum { domain: "unqualified".into(), value: 0 };
        assert!(!valid_arguments_for_epoch(EPOCH, &wrong));
    }
}
