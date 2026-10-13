//! Source schema observed through public GIMP 3.2.6 setters and XCF saves.
//! Evaluation and parameter defaults are deliberately separate from decoding.

use super::{copy, fail, field, flag, named, scalar};
use crate::{ColorBalanceConfiguration, ColorBalanceRange, ErrorKind, TypedConfiguration, XcfError, reader::Reader};

fn push_range(r: &mut Reader<'_, '_>, output: &mut Vec<ColorBalanceRange>, range: String, values: &[Option<u64>; 3]) -> Result<(), XcfError> {
    let [Some(cyan_red_bits), Some(magenta_green_bits), Some(yellow_blue_bits)] = *values else {
        return Err(fail(r));
    };
    if output.iter().any(|item| item.range == range) || output.len() >= 3 {
        return Err(fail(r));
    }
    r.charge(std::mem::size_of::<ColorBalanceRange>())?;
    output.try_reserve_exact(1).map_err(|_| r.error(ErrorKind::Allocation, "Color Balance ranges"))?;
    output.push(ColorBalanceRange { range, cyan_red_bits, magenta_green_bits, yellow_blue_bits });
    Ok(())
}

pub(super) fn typed(r: &mut Reader<'_, '_>, nodes: &[crate::ConfigNode]) -> Result<TypedConfiguration, XcfError> {
    let mut time = None;
    let mut preserve_luminosity = None;
    let mut current = None;
    let mut values = [None; 3];
    let mut ranges = Vec::new();
    for node in nodes {
        r.poll()?;
        let (name, arguments) = field(node).ok_or_else(|| fail(r))?;
        match name {
            "time" => {
                if time.is_some() {
                    return Err(fail(r));
                }
                time = Some(scalar(r, arguments)?);
            }
            "preserve-luminosity" => {
                if preserve_luminosity.is_some() {
                    return Err(fail(r));
                }
                preserve_luminosity = Some(flag(r, arguments)?);
            }
            "range" => {
                if let Some(range) = current.take() {
                    push_range(r, &mut ranges, range, &values)?;
                    values = [None; 3];
                }
                let range = named(r, arguments)?;
                if !matches!(range, "shadows" | "midtones" | "highlights") {
                    return Err(fail(r));
                }
                current = Some(copy(r, range)?);
            }
            "cyan-red" | "magenta-green" | "yellow-blue" => {
                if current.is_none() {
                    return Err(fail(r));
                }
                let index = match name {
                    "cyan-red" => 0,
                    "magenta-green" => 1,
                    _ => 2,
                };
                let value = values.get_mut(index).ok_or_else(|| fail(r))?;
                if value.is_some() {
                    return Err(fail(r));
                }
                *value = Some(scalar(r, arguments)?);
            }
            _ => return Err(fail(r)),
        }
    }
    if let Some(range) = current {
        push_range(r, &mut ranges, range, &values)?;
    }
    if ranges.len() != 3 || time.is_none() || preserve_luminosity.is_none() {
        return Err(fail(r));
    }
    Ok(TypedConfiguration::ColorBalance(ColorBalanceConfiguration { time_bits: time, ranges, preserve_luminosity }))
}
