//! Read-only import view borrowing the large encoded image payloads.

use crate::compression::decode_planes;
use crate::{ImageData, PsdError, PsdFile, Result, RgbaImage};

pub(crate) type BorrowedChannels<'a> = Vec<Vec<&'a [u8]>>;

/// A PSD/PSB import view that keeps encoded channels and merged pixels in the source slice.
///
/// Metadata and opaque tagged blocks remain owned. The source must live until importing is
/// complete; decoded pixels do not borrow it. Use [`Self::to_owned`] for a serializable file.
#[derive(Debug)]
pub struct PsdView<'a> {
    metadata: PsdFile,
    channels: BorrowedChannels<'a>,
    merged: &'a [u8],
}

impl<'a> PsdView<'a> {
    /// Parses with the same validation and layer-block selection as [`PsdFile::from_bytes`].
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self> {
        let (metadata, channels, merged) = PsdFile::parse_with(bytes, true)?;
        Ok(Self { metadata, channels, merged })
    }

    /// Read-only metadata. Encoded channel and merged buffers are intentionally empty.
    ///
    /// Decode through this view and serialize through [`Self::to_owned`], rather than using
    /// the metadata file's pixel decoding or writing methods.
    pub fn metadata(&self) -> &PsdFile {
        &self.metadata
    }

    /// Encoded bytes for the first channel with `id` in the specified layer record.
    pub fn channel_bytes(&self, index: usize, id: i16) -> Option<&'a [u8]> {
        let rec = self.metadata.layers().get(index)?;
        let channel = rec.channels.iter().position(|c| c.id == id)?;
        self.channels.get(index)?.get(channel).copied()
    }

    /// Encoded merged-image bytes, excluding its compression field.
    pub fn merged_bytes(&self) -> &'a [u8] {
        self.merged
    }

    /// Decodes one layer channel using the record's color or mask bounds.
    pub fn decode_channel(&self, index: usize, id: i16) -> Result<Vec<u8>> {
        let rec = self.metadata.layers().get(index).ok_or_else(|| PsdError::invalid(format!("no layer record {index}")))?;
        let channel = rec.channel(id).ok_or_else(|| PsdError::invalid(format!("layer has no channel {id}")))?;
        let bytes = self.channel_bytes(index, id).ok_or_else(|| PsdError::invalid("missing borrowed channel data"))?;
        let (w, h) = rec.channel_rect(id).size()?;
        channel.decode_from(bytes, w, h, self.metadata.header.depth, self.metadata.header.version)
    }

    /// Decodes all merged channels into planar big-endian samples.
    pub fn decode_merged(&self) -> Result<Vec<u8>> {
        decode_planes(self.metadata.image_data.compression, self.merged, &ImageData::layout(&self.metadata.header))
    }

    /// Converts the merged image to RGBA8, validating its output budget before decoding.
    pub fn composite_rgba8(&self) -> Result<RgbaImage> {
        self.metadata.composite_rgba8_with(|| self.decode_merged())
    }

    /// Materializes a complete owned model, including every encoded payload for roundtrip.
    pub fn to_owned(&self) -> PsdFile {
        let mut file = self.metadata.clone();
        if let Some(info) = &mut file.layer_info {
            for (rec, row) in info.layers.iter_mut().zip(&self.channels) {
                for (channel, bytes) in rec.channels.iter_mut().zip(row) {
                    channel.data = bytes.to_vec();
                }
            }
        }
        file.image_data.data = self.merged.to_vec();
        file
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::{ColorMode, Compression, LayerInfoPlacement, TaggedBlock, Version, testgen};

    fn assert_equivalent(bytes: &[u8]) {
        let owned = PsdFile::from_bytes(bytes).unwrap();
        let view = PsdView::from_bytes(bytes).unwrap();
        assert_eq!(view.to_owned(), owned);
        assert_eq!(view.to_owned().to_bytes().unwrap(), owned.to_bytes().unwrap());
        assert_eq!(view.decode_merged(), owned.decode_merged());
        let expected = owned.composite_rgba8();
        let actual = view.composite_rgba8();
        assert_eq!(actual, expected);
        if let Ok(planes) = view.decode_merged() {
            assert_eq!(view.metadata().composite_rgba8_from_planes(&planes), expected);
        }
        for (index, rec) in owned.layers().iter().enumerate() {
            for channel in &rec.channels {
                assert_eq!(view.decode_channel(index, channel.id), rec.decode_channel(channel.id, owned.header.depth, owned.header.version));
            }
        }
    }

    #[test]
    fn borrowed_generated_files_match_owned_at_every_depth_mode_and_compression() {
        for case in testgen::all_cases() {
            assert_equivalent(&case.file.to_bytes().unwrap());
        }
    }

    #[test]
    fn borrowed_payloads_point_into_input_and_materialize_original_bytes() {
        for version in [Version::Psd, Version::Psb] {
            for depth in [8, 16, 32] {
                let file = testgen::layered(version, ColorMode::Rgb, depth, Compression::Rle);
                let bytes = file.to_bytes().unwrap();
                let view = PsdView::from_bytes(&bytes).unwrap();
                let start = bytes.as_ptr() as usize;
                let end = start + bytes.len();
                let within = |slice: &[u8]| {
                    if !slice.is_empty() {
                        assert!(slice.as_ptr() as usize >= start);
                        assert!(slice.as_ptr() as usize + slice.len() <= end);
                    }
                };
                within(view.merged_bytes());
                assert!(view.metadata().image_data.data.is_empty());
                for (index, rec) in view.metadata().layers().iter().enumerate() {
                    for channel in &rec.channels {
                        assert!(channel.data.is_empty());
                        within(view.channel_bytes(index, channel.id).unwrap());
                    }
                }
                assert_eq!(view.to_owned().to_bytes().unwrap(), bytes);
            }
        }
    }

    #[test]
    fn borrowed_global_placement_preserves_padding_order_and_invalid_first_candidate() {
        for version in [Version::Psd, Version::Psb] {
            for depth in [16, 32] {
                let mut file = testgen::layered(version, ColorMode::Rgb, depth, Compression::Raw);
                let key = if depth == 16 { *b"Lr16" } else { *b"Lr32" };
                file.global_blocks = vec![TaggedBlock::new(*b"BEFR", vec![1]), TaggedBlock::new(*b"AFTR", vec![2, 3])];
                file.layer_info_placement = LayerInfoPlacement::GlobalBlock { index: 1, signature: *b"8B64", key, padding: Some(vec![7, 8]) };
                file.layer_mask_trailing = vec![9; 4];
                let bytes = file.to_bytes().unwrap();
                assert_equivalent(&bytes);
                let mut duplicate = Vec::new();
                file.layer_info.as_ref().unwrap().write_body(&mut duplicate, version).unwrap();
                file.global_blocks.push(TaggedBlock::new(key, duplicate));
                assert_equivalent(&file.to_bytes().unwrap());
                file.global_blocks.insert(0, TaggedBlock::new(key, vec![255]));
                let bytes = file.to_bytes().unwrap();
                let view = PsdView::from_bytes(&bytes).unwrap();
                assert!(view.metadata().layer_info.is_none());
                assert!(view.channels.is_empty());
                assert!(view.metadata().unreadable_layer_block().is_some());
                assert_equivalent(&bytes);
            }
        }
    }

    #[test]
    fn borrowed_and_owned_report_identical_truncation_errors() {
        let bytes = testgen::small(Version::Psb, Compression::Rle).to_bytes().unwrap();
        for end in 0..bytes.len() {
            assert_eq!(PsdView::from_bytes(&bytes[..end]).err(), PsdFile::from_bytes(&bytes[..end]).err());
        }
    }
}
