//! Exact pinned default and PhotoCraft font metadata; byte delivery is injected.
use super::{
    UiAssetError,
    descriptors::{self as assets, FontRole},
};
use egui::{FontData, FontDefinitions, FontFamily};
pub(super) fn definitions(bytes_for: impl Fn(FontRole) -> Result<&'static [u8], UiAssetError>) -> Result<FontDefinitions, UiAssetError> {
    let mut fonts = FontDefinitions::empty();
    for d in &assets::DESCRIPTORS {
        let Some(meta) = &d.font else { continue };
        let name = match meta.role {
            FontRole::InterMedium => "Inter-Medium",
            FontRole::InterSemiBold => "Inter-SemiBold",
            FontRole::JetBrainsMono => "JetBrainsMono",
            FontRole::Hack => "Hack",
            FontRole::UbuntuLight => "Ubuntu-Light",
            FontRole::NotoEmoji => "NotoEmoji-Regular",
            FontRole::EmojiIcon => "emoji-icon-font",
        };
        let bytes = bytes_for(meta.role)?;
        let mut data = FontData::from_static(bytes);
        data.index = meta.index;
        data.tweak.scale = f32::from_bits(meta.scale_bits);
        fonts.font_data.insert(name.to_owned(), std::sync::Arc::new(data));
    }
    fonts.font_data.insert("Inter".into(), std::sync::Arc::new(FontData::from_static(&photocraft_text::fonts::INTER_REGULAR)));
    fonts.families.insert(FontFamily::Monospace, ["JetBrainsMono", "Hack", "Ubuntu-Light", "NotoEmoji-Regular", "emoji-icon-font"].map(str::to_owned).to_vec());
    let fallback = ["Inter", "Ubuntu-Light", "NotoEmoji-Regular", "emoji-icon-font"].map(str::to_owned).to_vec();
    fonts.families.insert(FontFamily::Proportional, fallback.clone());
    for (family, first) in [("medium", "Inter-Medium"), ("semibold", "Inter-SemiBold")] {
        let mut stack = vec![first.to_owned()];
        stack.extend(fallback.iter().cloned());
        fonts.families.insert(FontFamily::Name(family.into()), stack);
    }
    Ok(fonts)
}
