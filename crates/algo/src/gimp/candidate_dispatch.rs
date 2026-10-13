//! Typed dispatch for standalone candidate kernels, never serialized admission.
//! Callers decode a separately bound schema and perform checked CMS conversion
//! before supplying these values. No profile/default/opaque GEGL value is inferred.
use super::{edge_sobel, generator_checkerboard, global_stretch_contrast};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::SampleEncoding;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface, checked_region_byte_len};

/// Named arguments remain typed; unknown names and duplicate values refuse.
pub enum Configuration<'a> {
    Checkerboard { format: PixelFormat, encoding: SampleEncoding, arguments: &'a [generator_checkerboard::Argument<'a>] },
    Sobel { arguments: &'a [global_stretch_contrast::BoolArgument<'a>] },
    StretchContrast { arguments: &'a [global_stretch_contrast::BoolArgument<'a>] },
}

/// A prepared candidate; creation validates exact identity/version/configuration.
/// It conveys kernel availability only, not source/current rendering fidelity.
pub struct Candidate {
    kernel: Kernel,
    checker_width: Option<i32>,
}

enum Kernel {
    Checkerboard(generator_checkerboard::Checkerboard),
    Sobel(edge_sobel::SobelParams),
    StretchContrast(global_stretch_contrast::StretchContrast),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Checkerboard,
    Sobel,
    StretchContrast,
}

/// Input interpretation is explicit and must already be CMS-prepared by caller.
pub enum ExecutionDomain<'a> {
    /// Generator colors and output share exactly the prepared native format and
    /// interpretation. The finite ROI is caller-owned, anchored at document origin.
    Checkerboard { format: PixelFormat, encoding: SampleEncoding },
    /// The extent includes all authoritative pixels and the abyss is explicit.
    Sobel { input: edge_sobel::SobelWorkingInput<'a>, abyss: edge_sobel::SobelAbyss },
    /// Opaque linear RGB Float32; full_extent must include default pixels, not
    /// merely the bounds of occupied/nonzero tiles. No source transfer is inferred.
    StretchContrast { source: &'a Surface, full_extent: Rect, encoding: SampleEncoding },
}

/// Kernel-specific invocation limits; aggregate graph ownership is caller-owned.
pub enum Limits {
    Checkerboard(generator_checkerboard::Limits),
    Sobel(edge_sobel::SobelLimits),
    StretchContrast(global_stretch_contrast::GlobalLimits),
}

impl Candidate {
    /// Scalar source binding; named colors still require the caller's checked CMS.
    pub fn matches_source_integer(&self, name: &str, value: i64) -> bool {
        match (self.kind(), name) {
            (Kind::Checkerboard, "x") => self.checker_width.is_some_and(|width| i64::from(width) == value),
            (Kind::Checkerboard, "y") => value == 16,
            (Kind::Checkerboard, "x-offset" | "y-offset") => value == 0,
            _ => false,
        }
    }

    pub fn matches_source_boolean(&self, name: &str, value: bool) -> bool {
        match (&self.kernel, name) {
            (Kernel::Sobel(params), "horizontal") => params.horizontal == value,
            (Kernel::Sobel(params), "vertical") => params.vertical == value,
            (Kernel::Sobel(params), "keep-sign") => params.keep_sign == value,
            (Kernel::StretchContrast(_), "keep-colors" | "perceptual") => !value,
            _ => false,
        }
    }

    pub fn kind(&self) -> Kind {
        match self.kernel {
            Kernel::Checkerboard(_) => Kind::Checkerboard,
            Kernel::Sobel(_) => Kind::Sobel,
            Kernel::StretchContrast(_) => Kind::StretchContrast,
        }
    }

    pub fn identity(&self) -> (&'static str, &'static str) {
        (
            match self.kind() {
                Kind::Checkerboard => "gegl:checkerboard",
                Kind::Sobel => "gegl:edge-sobel",
                Kind::StretchContrast => "gegl:stretch-contrast",
            },
            "0:0",
        )
    }

    /// Allocation-free domain/ROI preflight for enclosing budget schedulers.
    pub fn validate_domain(&self, domain: &ExecutionDomain<'_>, roi: Rect) -> Result<(), RasterInputError> {
        match (&self.kernel, domain) {
            (Kernel::Checkerboard(configuration), ExecutionDomain::Checkerboard { format, encoding }) => {
                if *format != configuration.format() || *encoding != configuration.encoding() {
                    return Err(RasterInputError::UnsupportedConversion);
                }
                if roi.is_empty() || roi.x0 < 0 || roi.y0 < 0 {
                    return Err(RasterInputError::InvalidBounds);
                }
                checked_region_byte_len(roi, *format)?;
            }
            (Kernel::Sobel(_), ExecutionDomain::Sobel { input, .. }) => {
                let format = input.source.format();
                if format.alpha || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(format.sample, SampleType::F32 | SampleType::F64)
                {
                    return Err(RasterInputError::UnsupportedConversion);
                }
                checked_region_byte_len(input.extent, format)?;
                checked_region_byte_len(roi, format)?;
                if input.extent.is_empty() || !input.extent.contains_rect(&roi) {
                    return Err(RasterInputError::InvalidBounds);
                }
                edge_sobel::sobel_required_input(roi)?;
            }
            (Kernel::StretchContrast(_), ExecutionDomain::StretchContrast { source, full_extent, encoding }) => {
                if *encoding != SampleEncoding::Linear || source.format() != PixelFormat::new(ColorMode::Rgb, SampleType::F32, true) {
                    return Err(RasterInputError::UnsupportedConversion);
                }
                checked_region_byte_len(*full_extent, source.format())?;
                checked_region_byte_len(roi, source.format())?;
                if full_extent.is_empty() || roi.is_empty() || !full_extent.contains_rect(&roi) {
                    return Err(RasterInputError::InvalidBounds);
                }
            }
            _ => return Err(RasterInputError::UnsupportedConversion),
        }
        Ok(())
    }

    pub fn from_named(operation: &str, version: &str, configuration: Configuration<'_>) -> Result<Self, RasterInputError> {
        let mut checker_width = None;
        let kernel = match (operation, configuration) {
            ("gegl:checkerboard", Configuration::Checkerboard { format, encoding, arguments }) => {
                checker_width = arguments.iter().find_map(|argument| match argument {
                    generator_checkerboard::Argument::Integer { name: "x", value } => Some(*value),
                    _ => None,
                });
                Kernel::Checkerboard(generator_checkerboard::Checkerboard::from_named(operation, version, format, encoding, arguments)?)
            }
            ("gegl:edge-sobel", Configuration::Sobel { arguments }) => {
                if arguments.len() != 3 {
                    return Err(RasterInputError::UnsupportedConversion);
                }
                let (mut horizontal, mut vertical, mut keep_sign) = (None, None, None);
                for argument in arguments {
                    let destination = match argument.name {
                        "horizontal" => &mut horizontal,
                        "vertical" => &mut vertical,
                        "keep-sign" => &mut keep_sign,
                        _ => return Err(RasterInputError::UnsupportedConversion),
                    };
                    if destination.replace(argument.value).is_some() {
                        return Err(RasterInputError::UnsupportedConversion);
                    }
                }
                Kernel::Sobel(edge_sobel::SobelParams::for_version(
                    version,
                    horizontal.ok_or(RasterInputError::UnsupportedConversion)?,
                    vertical.ok_or(RasterInputError::UnsupportedConversion)?,
                    keep_sign.ok_or(RasterInputError::UnsupportedConversion)?,
                )?)
            }
            ("gegl:stretch-contrast", Configuration::StretchContrast { arguments }) => {
                Kernel::StretchContrast(global_stretch_contrast::StretchContrast::from_named_booleans(operation, version, arguments)?)
            }
            _ => return Err(RasterInputError::UnsupportedConversion),
        };
        Ok(Self { kernel, checker_width })
    }

    /// No allocation or source clone occurs before exact domain/configuration
    /// matching and native ROI preflight. Each kernel enforces its own stronger
    /// sample/extent/work/resident limits and abandons private output on failure.
    pub fn execute(&self, domain: ExecutionDomain<'_>, roi: Rect, limits: Limits, interrupt: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        self.validate_domain(&domain, roi)?;
        match (&self.kernel, domain, limits) {
            (Kernel::Checkerboard(configuration), ExecutionDomain::Checkerboard { format, encoding }, Limits::Checkerboard(limits)) => {
                if format != configuration.format() || encoding != configuration.encoding() {
                    return Err(RasterInputError::UnsupportedConversion);
                }
                checked_region_byte_len(roi, format)?;
                generator_checkerboard::try_generate(configuration, roi, limits, interrupt)
            }
            (Kernel::Sobel(configuration), ExecutionDomain::Sobel { input, abyss }, Limits::Sobel(limits)) => {
                let format = input.source.format();
                if format.alpha || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(format.sample, SampleType::F32 | SampleType::F64)
                {
                    return Err(RasterInputError::UnsupportedConversion);
                }
                checked_region_byte_len(input.extent, format)?;
                checked_region_byte_len(roi, format)?;
                if input.extent.is_empty() || !input.extent.contains_rect(&roi) {
                    return Err(RasterInputError::InvalidBounds);
                }
                edge_sobel::try_sobel_standard(input, roi, *configuration, abyss, limits, interrupt)
            }
            (Kernel::StretchContrast(configuration), ExecutionDomain::StretchContrast { source, full_extent, encoding }, Limits::StretchContrast(limits)) => {
                if encoding != SampleEncoding::Linear || source.format() != PixelFormat::new(ColorMode::Rgb, SampleType::F32, true) {
                    return Err(RasterInputError::UnsupportedConversion);
                }
                checked_region_byte_len(full_extent, source.format())?;
                checked_region_byte_len(roi, source.format())?;
                if full_extent.is_empty() || roi.is_empty() || !full_extent.contains_rect(&roi) {
                    return Err(RasterInputError::InvalidBounds);
                }
                global_stretch_contrast::try_stretch_contrast(source, roi, full_extent, *configuration, limits, interrupt)
            }
            _ => Err(RasterInputError::UnsupportedConversion),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use global_stretch_contrast::BoolArgument;

    fn stretch() -> Candidate {
        Candidate::from_named(
            "gegl:stretch-contrast",
            "0:0",
            Configuration::StretchContrast {
                arguments: &[BoolArgument { name: "keep-colors", value: false }, BoolArgument { name: "perceptual", value: false }],
            },
        )
        .unwrap()
    }

    #[test]
    fn exact_identity_version_names_and_domain_are_required() {
        let arguments = [
            BoolArgument { name: "horizontal", value: true },
            BoolArgument { name: "vertical", value: false },
            BoolArgument { name: "keep-sign", value: false },
        ];
        assert!(Candidate::from_named("gegl:edge-sobel", "0:0", Configuration::Sobel { arguments: &arguments }).is_ok());
        for (operation, version) in [("gegl:edge-sobel", "0:1"), ("edge-sobel", "0:0"), ("gegl:checkerboard", "0:0"), ("unknown", "0:0")] {
            assert!(Candidate::from_named(operation, version, Configuration::Sobel { arguments: &arguments }).is_err());
        }
        let duplicate = [
            BoolArgument { name: "horizontal", value: true },
            BoolArgument { name: "horizontal", value: false },
            BoolArgument { name: "keep-sign", value: false },
        ];
        assert!(Candidate::from_named("gegl:edge-sobel", "0:0", Configuration::Sobel { arguments: &duplicate }).is_err());
        let surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        assert!(matches!(
            stretch().execute(
                ExecutionDomain::StretchContrast { source: &surface, full_extent: Rect::new(0, 0, 1, 1), encoding: SampleEncoding::NonlinearProfile },
                Rect::new(0, 0, 1, 1),
                Limits::StretchContrast(global_stretch_contrast::GlobalLimits::default()),
                &Interrupt::NONE
            ),
            Err(RasterInputError::UnsupportedConversion)
        ));
        assert!(matches!(
            stretch().execute(
                ExecutionDomain::Checkerboard { format: surface.format(), encoding: SampleEncoding::Linear },
                Rect::new(0, 0, 1, 1),
                Limits::StretchContrast(global_stretch_contrast::GlobalLimits::default()),
                &Interrupt::NONE
            ),
            Err(RasterInputError::UnsupportedConversion)
        ));
    }
}
