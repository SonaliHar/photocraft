//! Implements filter used by both the 7z and XZ file format.

pub mod bcj;

pub mod delta;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum FilterType {
    /// Delta filter
    Delta,
    /// BCJ x86 filter
    BcjX86,
    /// BCJ PowerPC filter
    BcjPpc,
    /// BCJ IA64 filter
    BcjIa64,
    /// BCJ ARM filter
    BcjArm,
    /// BCJ ARM Thumb
    BcjArmThumb,
    /// BCJ SPARC filter
    BcjSparc,
    /// BCJ ARM64 filter
    BcjArm64,
    /// BCJ RISC-V filter
    BcjRiscv,
    /// LZMA2 filter
    Lzma2,
}

impl TryFrom<u64> for FilterType {
    type Error = ();

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0x03 => Ok(FilterType::Delta),
            0x04 => Ok(FilterType::BcjX86),
            0x05 => Ok(FilterType::BcjPpc),
            0x06 => Ok(FilterType::BcjIa64),
            0x07 => Ok(FilterType::BcjArm),
            0x08 => Ok(FilterType::BcjArmThumb),
            0x09 => Ok(FilterType::BcjSparc),
            0x0A => Ok(FilterType::BcjArm64),
            0x0B => Ok(FilterType::BcjRiscv),
            0x21 => Ok(FilterType::Lzma2),
            _ => Err(()),
        }
    }
}
