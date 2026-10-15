//! Sealed routing for the existing configured Point source domains.
//! Schema/typed configuration and literal composition remain qualified_stack's
//! authority. These hints cannot promote arbitrary Config/Curve storage.
use super::*;
use crate::xcf_filters::NeighborhoodDomain;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::xcf_import) enum ConfiguredPointFamily {
    ColorBalance,
    ColorBalanceDepth,
    Levels,
    Curves,
}
impl ConfiguredPointFamily {
    pub(super) fn hint<P: ExactParsedPlane>(file: &XcfFile<P>) -> Option<Self> {
        let effect = file.layers.first()?.effects.first()?;
        if effect.arguments.len() > 10 {
            return None;
        }
        Some(match effect.operation.as_str() {
            "gimp:color-balance" if matches!(file.precision, 100 | 150 | 175 | 275 | 300 | 350 | 375 | 500 | 550 | 575 | 675 | 775) => Self::ColorBalanceDepth,
            "gimp:color-balance" => Self::ColorBalance,
            "gimp:levels" => Self::Levels,
            "gimp:curves" => Self::Curves,
            _ => return None,
        })
    }
    pub(super) fn domain(self) -> NeighborhoodDomain {
        match self {
            Self::ColorBalance => NeighborhoodDomain::ColorBalancePrecision,
            Self::ColorBalanceDepth => NeighborhoodDomain::ColorBalanceDepth,
            // These are exactly the old non-neighborhood configured mapper
            // domains. Their own schema/composition epochs are not replaced.
            Self::Levels | Self::Curves => NeighborhoodDomain::Unqualified,
        }
    }
    pub(super) fn set_late(self, domains: &mut admission::LateAdmissionDomains) {
        match self {
            Self::ColorBalance => domains.color_balance_domain = true,
            Self::ColorBalanceDepth => {
                domains.color_balance_domain = true;
                domains.color_balance_depth_domain = true;
            }
            Self::Levels => domains.levels_context = true,
            Self::Curves => domains.point_context = true,
        }
    }
    pub(super) fn qualify<P: ExactParsedPlane>(
        self,
        file: &XcfFile<P>,
        ctl: &Interrupt<'_>,
        budget: Option<&photocraft_xcf::Budget<'_>>,
    ) -> Result<bool, IoError> {
        if Self::hint(file) != Some(self) {
            return Ok(false);
        }
        match self {
            Self::ColorBalance | Self::ColorBalanceDepth => {
                precision_point_source::qualify(file, "gimp:color-balance", self == Self::ColorBalanceDepth, false, ctl, budget)
            }
            Self::Levels | Self::Curves => configured_raster(file, self, ctl, budget),
        }
    }
}
fn poll(ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>, units: u64) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if let Some(budget) = budget {
        budget.consume_work(units, "configured Point original source").map_err(accounting_error)?;
    }
    ctl.check().map_err(|_| IoError::Cancelled)
}
// Exact existing point_context/levels_context + legacy_point_candidate, narrowed
// to their current renderer's singleton12x9 intersection. The old eager function
// remains untouched. Unlike precision-point, structured Levels keeps finite HDR
// colors; coverage is unit, and the real prepared pipeline validates color math.
fn configured_raster<P: ExactParsedPlane>(
    file: &XcfFile<P>,
    family: ConfiguredPointFamily,
    ctl: &Interrupt<'_>,
    budget: Option<&photocraft_xcf::Budget<'_>>,
) -> Result<bool, IoError> {
    poll(ctl, budget, 32)?;
    let levels = family == ConfiguredPointFamily::Levels;
    if !(file.version == 22
        && file.model == Model::Rgb
        && file.icc.is_empty()
        && (file.width, file.height) == (12, 9)
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 1
        && if levels {
            file.sample == Sample::F32 && matches!(file.encoding, Encoding::Linear | Encoding::NonlinearProfile)
        } else {
            file.sample == Sample::U8 && file.encoding == Encoding::NonlinearProfile
        })
    {
        return Ok(false);
    }
    let Some(d) = file.layers.first() else {
        return Ok(false);
    };
    if d.model != Model::Rgb
        || d.group
        || d.mask.is_some()
        || !d.alpha
        || d.effects.len() != 1
        || (d.width, d.height) != (12, 9)
        || d.offset != (0, 0)
        || !d.effects.first().is_some_and(|effect| effect.operation == if levels { "gimp:levels" } else { "gimp:curves" })
    {
        return Ok(false);
    }
    let Some(plane) = d.plane.as_ref() else {
        return Ok(false);
    };
    let descriptor = plane.descriptor();
    let width = if levels { 4usize } else { 1usize };
    let stride = 4 * width;
    let expected = 12 * 9 * stride;
    if descriptor.sample != file.sample
        || descriptor.channels != 4
        || descriptor.width != 12
        || descriptor.height != 9
        || plane.native_byte_len() != Some(expected as u64)
    {
        return Ok(false);
    }
    let mut seen = 0usize;
    let mut valid = true;
    plane.visit_native(ctl, &mut |bytes| {
        if bytes.is_empty() || bytes.len() > stride * 64 || bytes.len() % stride != 0 {
            return Err(IoError::Unsupported("configured Point native block".into()));
        }
        seen = seen.checked_add(bytes.len()).filter(|n| *n <= expected).ok_or_else(|| IoError::Unsupported("configured Point extra bytes".into()))?;
        poll(ctl, budget, (bytes.len() + bytes.len() / width) as u64)?;
        if levels {
            for pixel in bytes.chunks_exact(stride) {
                for (channel, raw) in pixel.chunks_exact(4).enumerate() {
                    let value = f32::from_ne_bytes(raw.try_into().map_err(|_| IoError::Unsupported("configured Point native sample".into()))?);
                    valid &= value.is_finite() && (channel != 3 || (0.0..=1.0).contains(&value));
                }
            }
        }
        Ok(())
    })?;
    if seen != expected {
        return Err(IoError::Unsupported("configured Point incomplete native bytes".into()));
    }
    poll(ctl, budget, 0)?;
    Ok(valid)
}

#[cfg(test)]
#[path = "staged_configured_point_tests.rs"]
mod tests;
