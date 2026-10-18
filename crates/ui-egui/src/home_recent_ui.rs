//! Home screen Recent Files Thumbnail Card Grid and List views.
//!
//! Provides thumbnail preview loading and caching, interactive file cards with
//! thumbnail image, document icon, filename, and file path/location, along with
//! Grid/List view switching matching Photoshop 2026 Home screen.

use std::collections::HashMap;
use std::path::Path;

use egui::{Color32, ColorImage, Rect, Response, Sense, Stroke, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2};

use crate::theme::Tokens;
use crate::{PhotocraftApp, icons};

/// Cache key identifying the origin state of a loaded thumbnail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CacheKey {
    /// Rendered from an active in-memory document with a specific revision.
    OpenDoc { revision: u64 },
    /// Read from a file on disk with modification time and file size.
    #[cfg(not(target_arch = "wasm32"))]
    DiskFile {
        modified: Option<std::time::SystemTime>,
        len: u64,
    },
}

pub(crate) struct ThumbnailEntry {
    pub(crate) key: CacheKey,
    pub(crate) texture: Option<TextureHandle>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) last_checked: std::time::Instant,
}

/// Cache for loaded recent file preview textures.
#[derive(Default)]
pub(crate) struct ThumbnailCache {
    pub(crate) entries: HashMap<String, ThumbnailEntry>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) was_focused: bool,
}

impl ThumbnailCache {
    /// Invalidate the cached thumbnail for a single path (e.g. after save, open, or remove).
    pub(crate) fn invalidate(&mut self, path: &str) {
        self.entries.remove(path);
    }

    /// Clear all cached thumbnails (e.g. on Clear Recent Files).
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Check if two path strings match, tolerating slash direction and Windows case differences.
fn paths_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    #[cfg(windows)]
    {
        let a_norm = a.replace('\\', "/");
        let b_norm = b.replace('\\', "/");
        a_norm.eq_ignore_ascii_case(&b_norm)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Downscale an RGBA8 buffer to fit within `max_dim` on its longest side.
#[cfg(not(target_arch = "wasm32"))]
fn downsample_rgba(w: u32, h: u32, src_data: &[u8], max_dim: u32) -> Option<ColorImage> {
    if w == 0 || h == 0 || src_data.len() != (w as usize) * (h as usize) * 4 {
        return None;
    }
    let (tw, th) = if w > max_dim || h > max_dim {
        let scale = (max_dim as f32) / (w.max(h) as f32);
        (((w as f32) * scale).round().max(1.0) as u32, ((h as f32) * scale).round().max(1.0) as u32)
    } else {
        (w, h)
    };
    if tw == w && th == h {
        Some(ColorImage::from_rgba_unmultiplied([w as usize, h as usize], src_data))
    } else {
        let mut thumb_px = vec![0u8; (tw as usize) * (th as usize) * 4];
        for y in 0..th {
            let sy = ((y as f32 / th as f32) * h as f32) as usize;
            for x in 0..tw {
                let sx = ((x as f32 / tw as f32) * w as f32) as usize;
                let src_idx = (sy * w as usize + sx) * 4;
                let dst_idx = (y as usize * tw as usize + x as usize) * 4;
                if src_idx + 4 <= src_data.len() && dst_idx + 4 <= thumb_px.len() {
                    thumb_px[dst_idx..dst_idx + 4].copy_from_slice(&src_data[src_idx..src_idx + 4]);
                }
            }
        }
        Some(ColorImage::from_rgba_unmultiplied([tw as usize, th as usize], &thumb_px))
    }
}

/// Try to load a thumbnail image for `path` as an egui `ColorImage`.
#[cfg(not(target_arch = "wasm32"))]
fn load_thumbnail_image(path: &str) -> Option<ColorImage> {
    {
        let std_path = Path::new(path);
        let bytes = std::fs::read(std_path).ok()?;
        let ext = std_path.extension().and_then(|e| e.to_str()).unwrap_or("");

        // 1. If .pcraft, read the embedded thumbnail PNG
        if ext.eq_ignore_ascii_case("pcraft")
            && let Ok(Some(png_bytes)) = photocraft_format::read_thumbnail(&bytes)
            && let Ok(img) = photocraft_codecs::decode(&png_bytes)
        {
            let rgba = img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
            if rgba.data().len() == rgba.width() as usize * rgba.height() as usize * 4 {
                return Some(ColorImage::from_rgba_unmultiplied(
                    [rgba.width() as usize, rgba.height() as usize],
                    rgba.data(),
                ));
            }
        }

        // 2. If PSD/PSB, inspect resource 1036 (JFIF JPEG) or decode composite
        if (ext.eq_ignore_ascii_case("psd") || ext.eq_ignore_ascii_case("psb"))
            && let Ok(file) = photocraft_psd::PsdFile::from_bytes(&bytes)
        {
            for r in &file.resources {
                if (r.id == photocraft_psd::resources::ids::THUMBNAIL || r.id == photocraft_psd::resources::ids::THUMBNAIL_PS4)
                    && let Some(jfif) = r.data.get(28..)
                    && let Ok(img) = photocraft_codecs::decode(jfif)
                {
                    let rgba = img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
                    if rgba.data().len() == rgba.width() as usize * rgba.height() as usize * 4 {
                        return Some(ColorImage::from_rgba_unmultiplied(
                            [rgba.width() as usize, rgba.height() as usize],
                            rgba.data(),
                        ));
                    }
                }
            }
            // Fallback for PSDs without embedded JFIF previews: decode merged composite
            // only if within safe dimensions (<= 2048x2048) to avoid memory spikes on massive files.
            if file.header.width <= 2048 && file.header.height <= 2048
                && let Ok(comp) = photocraft_io::merged_composite(&file)
            {
                let w = file.header.width;
                let h = file.header.height;
                if w > 0 && h > 0 && comp.len() == (w as usize) * (h as usize) {
                    let mut raw_bytes = vec![0u8; comp.len() * 4];
                    for (i, &[r, g, b, a]) in comp.iter().enumerate() {
                        let idx = i * 4;
                        raw_bytes[idx] = (r.clamp(0.0, 1.0) * 255.0).round() as u8;
                        raw_bytes[idx + 1] = (g.clamp(0.0, 1.0) * 255.0).round() as u8;
                        raw_bytes[idx + 2] = (b.clamp(0.0, 1.0) * 255.0).round() as u8;
                        raw_bytes[idx + 3] = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                    return downsample_rgba(w, h, &raw_bytes, 256);
                }
            }
        }

        // 3. Flat raster formats (PNG, JPG, WebP, TIFF, GIF, BMP, etc.)
        if let Ok(img) = photocraft_codecs::decode(&bytes) {
            let (w, h) = img.dimensions();
            if w > 0 && h > 0 {
                let rgba = img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
                return downsample_rgba(w, h, rgba.data(), 256);
            }
        }

        None
    }
}

/// Retrieve, lazily load, or regenerate the preview texture for `path`.
///
/// If `path` is currently open in `app.session`, the thumbnail is rendered from
/// the live document composite and re-rendered whenever its revision changes.
/// If `path` is closed, the thumbnail is decoded from the file on disk, automatically
/// re-loading if the file's modification timestamp or length on disk changes.
pub(crate) fn get_or_load_thumbnail(app: &PhotocraftApp, ctx: &egui::Context, path: &str) -> Option<TextureHandle> {
    let opts = TextureOptions {
        magnification: egui::TextureFilter::Linear,
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: Some(egui::TextureFilter::Linear),
    };

    // 1. Is this document currently open in the session?
    if let Some(st) = app.session.documents().iter().find(|d| d.path.as_deref().is_some_and(|p| paths_match(p, path))) {
        let current_key = CacheKey::OpenDoc { revision: st.revision };
        let mut cache = app.thumbnail_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = cache.entries.get(path)
            && entry.key == current_key
        {
            return entry.texture.clone();
        }

        let rgba = photocraft_compose::thumbnail(&st.doc, 256);
        let color_image = ColorImage::from_rgba_unmultiplied(
            [rgba.width as usize, rgba.height as usize],
            &rgba.pixels,
        );

        let tex = if let Some(existing) = cache.entries.get_mut(path).and_then(|e| e.texture.as_mut()) {
            existing.set(color_image, opts);
            existing.clone()
        } else {
            ctx.load_texture(format!("recent_thumb_{path}"), color_image, opts)
        };

        cache.entries.insert(path.to_string(), ThumbnailEntry {
            key: current_key,
            texture: Some(tex.clone()),
            #[cfg(not(target_arch = "wasm32"))]
            last_checked: std::time::Instant::now(),
        });
        return Some(tex);
    }

    // 2. Otherwise inspect the file on disk
    #[cfg(not(target_arch = "wasm32"))]
    {
        #[cfg(test)]
        const RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);
        #[cfg(not(test))]
        const RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

        let mut cache = app.thumbnail_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

        // Detect window regaining focus from an external application
        let focused = ctx.input(|i| i.focused);
        let focus_gained = focused && !cache.was_focused;
        if focus_gained {
            cache.was_focused = true;
        } else if !focused {
            cache.was_focused = false;
        }

        // Fast path: if already cached and neither focus gained nor recheck interval elapsed,
        // return the texture immediately from memory without touching the filesystem.
        if !focus_gained
            && let Some(entry) = cache.entries.get(path)
            && entry.last_checked.elapsed() < RECHECK_INTERVAL
        {
            return entry.texture.clone();
        }

        let std_path = Path::new(path);
        let Ok(meta) = std::fs::metadata(std_path) else {
            cache.entries.remove(path);
            return None;
        };

        let current_key = CacheKey::DiskFile {
            modified: meta.modified().ok(),
            len: meta.len(),
        };

        if let Some(entry) = cache.entries.get_mut(path)
            && entry.key == current_key
        {
            entry.last_checked = std::time::Instant::now();
            return entry.texture.clone();
        }

        let opt_img = load_thumbnail_image(path);
        let tex = opt_img.map(|ci| {
            if let Some(existing) = cache.entries.get_mut(path).and_then(|e| e.texture.as_mut()) {
                existing.set(ci, opts);
                existing.clone()
            } else {
                ctx.load_texture(format!("recent_thumb_{path}"), ci, opts)
            }
        });

        cache.entries.insert(path.to_string(), ThumbnailEntry {
            key: current_key,
            texture: tex.clone(),
            last_checked: std::time::Instant::now(),
        });
        tex
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ctx;
        let _ = path;
        None
    }
}

/// Action output from the recent files view.
pub(crate) enum RecentAction {
    Open(String),
    Remove(String),
    Clear,
    OpenDialog,
}

/// Card width in the grid.
pub const GRID_CARD_WIDTH: f32 = 184.0;
/// Card height in the grid.
pub const GRID_CARD_HEIGHT: f32 = 244.0;
/// Preview area height within a grid card.
pub const GRID_PREVIEW_HEIGHT: f32 = 168.0;
/// Row height in list view.
pub const LIST_ROW_HEIGHT: f32 = 34.0;

/// Render the header for Recent files with title and Grid/List toggle buttons.
pub(crate) fn recent_header(ui: &mut Ui, is_grid: &mut bool, total_width: f32) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Recent")).font(crate::theme::semibold(17.0)).color(t.text));
        let toggle_w = 64.0;
        let space = (total_width - 90.0 - toggle_w).max(0.0);
        ui.add_space(space);

        // Container box for the toggles
        let (box_rect, _) = ui.allocate_exact_size(vec2(60.0, 28.0), Sense::hover());
        ui.painter().rect_filled(box_rect, 4.0, Color32::from_black_alpha(70));
        ui.painter().rect_stroke(box_rect, 4.0, Stroke::new(1.0, Color32::from_white_alpha(25)), egui::StrokeKind::Inside);

        let grid_rect = Rect::from_min_size(box_rect.min + vec2(2.0, 2.0), vec2(28.0, 24.0));
        let list_rect = Rect::from_min_size(pos2(box_rect.min.x + 30.0, box_rect.min.y + 2.0), vec2(28.0, 24.0));

        let grid_resp = ui.interact(grid_rect, ui.id().with("toggle_grid"), Sense::click());
        let list_resp = ui.interact(list_rect, ui.id().with("toggle_list"), Sense::click());

        grid_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Grid view")));
        list_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("List view")));

        let grid_resp = grid_resp.on_hover_text(tl!("Grid view")).on_hover_cursor(egui::CursorIcon::PointingHand);
        let list_resp = list_resp.on_hover_text(tl!("List view")).on_hover_cursor(egui::CursorIcon::PointingHand);

        if *is_grid {
            ui.painter().rect_filled(grid_rect, 3.0, t.hover);
            ui.painter().rect_stroke(grid_rect, 3.0, Stroke::new(1.0, Color32::from_rgb(26, 115, 232)), egui::StrokeKind::Inside);
        }
        if !*is_grid {
            ui.painter().rect_filled(list_rect, 3.0, t.hover);
            ui.painter().rect_stroke(list_rect, 3.0, Stroke::new(1.0, Color32::from_rgb(26, 115, 232)), egui::StrokeKind::Inside);
        }

        let grid_tint = if *is_grid { Color32::WHITE } else { t.text_dim };
        let list_tint = if !*is_grid { Color32::WHITE } else { t.text_dim };

        icons::paint(ui, grid_rect, "grid-2x2", 15.0, grid_tint);
        icons::paint(ui, list_rect, "marquee-row", 15.0, list_tint);

        if grid_resp.clicked() {
            *is_grid = true;
        }
        if list_resp.clicked() {
            *is_grid = false;
        }
    });
}

/// Render a single visual thumbnail card in grid view.
pub(crate) fn render_grid_card(
    ui: &mut Ui,
    path: &str,
    name: &str,
    folder: &str,
    thumb: Option<&TextureHandle>,
) -> (Response, Option<RecentAction>) {
    let t = Tokens::get(ui.ctx());
    let (card_rect, resp) = ui.allocate_exact_size(vec2(GRID_CARD_WIDTH, GRID_CARD_HEIGHT), Sense::click());
    let hovered = ui.rect_contains_pointer(card_rect);

    // Card background & border
    let bg_fill = Color32::from_rgb(38, 38, 38);
    ui.painter().rect_filled(card_rect, 6.0, bg_fill);
    let border_stroke = if hovered {
        Stroke::new(1.0, Color32::from_rgb(26, 115, 232))
    } else {
        Stroke::new(1.0, Color32::from_white_alpha(20))
    };
    ui.painter().rect_stroke(card_rect, 6.0, border_stroke, egui::StrokeKind::Inside);

    // Preview area
    let preview_rect = Rect::from_min_size(
        card_rect.min + vec2(1.0, 1.0),
        vec2(GRID_CARD_WIDTH - 2.0, GRID_PREVIEW_HEIGHT),
    );
    // Dark inner well for preview area with rounded top corners
    ui.painter().rect_filled(
        preview_rect,
        egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 },
        Color32::from_rgb(20, 20, 20),
    );

    if let Some(tex) = thumb {
        let tex_size = tex.size_vec2();
        let avail_size = preview_rect.size() - vec2(8.0, 8.0);
        if tex_size.x > 0.0 && tex_size.y > 0.0 {
            let aspect = tex_size.x / tex_size.y;
            let target_w = (avail_size.y * aspect).min(avail_size.x);
            let target_h = target_w / aspect;
            let img_rect = Rect::from_center_size(preview_rect.center(), vec2(target_w, target_h));
            ui.painter().image(tex.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    } else {
        // Fallback document vector icon
        let icon_box = Rect::from_center_size(preview_rect.center(), Vec2::splat(40.0));
        icons::paint(ui, icon_box, "file", 34.0, t.text_faint);
    }

    // Small photo icon badge at bottom-left of preview
    let badge_rect = Rect::from_center_size(pos2(preview_rect.left() + 16.0, preview_rect.bottom() - 14.0), vec2(16.0, 16.0));
    icons::paint(ui, badge_rect, "image", 13.0, Color32::from_white_alpha(190));

    // Bottom info area
    let info_top = preview_rect.bottom() + 8.0;
    let text_x = card_rect.left() + 14.0;
    let text_max_w = (card_rect.right() - text_x - 12.0).max(10.0);

    // Filename
    let name_galley = crate::tab_strip::elided(ui, name, crate::theme::semibold(13.0), Color32::WHITE, text_max_w);
    ui.painter().galley(pos2(text_x, info_top), name_galley, Color32::WHITE);

    // Monitor icon + folder / "On your computer"
    let monitor_rect = Rect::from_center_size(pos2(text_x + 6.0, info_top + 28.0), vec2(14.0, 14.0));
    icons::paint(ui, monitor_rect, "monitor", 12.0, t.text_faint);

    let folder_display = if folder.is_empty() { tl!("On your computer").to_owned() } else { folder.to_string() };
    let folder_galley = crate::tab_strip::elided(ui, &folder_display, egui::FontId::proportional(11.0), t.text_faint, text_max_w - 18.0);
    ui.painter().galley(pos2(text_x + 18.0, info_top + 22.0), folder_galley, t.text_faint);

    // Hover 'x' close button in top right of card
    let mut action = None;
    if hovered {
        let close_rect = Rect::from_center_size(pos2(card_rect.right() - 14.0, card_rect.top() + 14.0), vec2(20.0, 20.0));
        let x_resp = ui.interact(close_rect, resp.id.with("remove"), Sense::click());
        let label = crate::i18n::fmt(tl!("Remove {name} from Recent"), &[("name", name)]);
        x_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        ui.painter().rect_filled(close_rect, 10.0, Color32::from_black_alpha(190));
        let x_color = if x_resp.hovered() { Color32::WHITE } else { t.text_dim };
        icons::paint(ui, close_rect, "x", 11.0, x_color);
        if x_resp.on_hover_text(tl!("Remove from Recent")).clicked() {
            action = Some(RecentAction::Remove(path.to_string()));
        }
    }

    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    let resp = resp.on_hover_text(path).on_hover_cursor(egui::CursorIcon::PointingHand);

    if resp.clicked() && action.is_none() {
        action = Some(RecentAction::Open(path.to_string()));
    }

    (resp, action)
}

/// Render a dashed/hairline drop target card inside the recent grid.
pub(crate) fn render_inline_drop_card(ui: &mut Ui) -> bool {
    let t = Tokens::get(ui.ctx());
    let (card_rect, resp) = ui.allocate_exact_size(vec2(GRID_CARD_WIDTH, GRID_CARD_HEIGHT), Sense::click());
    let hovered = ui.rect_contains_pointer(card_rect);

    let border_color = if hovered {
        Color32::from_white_alpha(80)
    } else {
        Color32::from_white_alpha(28)
    };
    let bg_color = if hovered {
        Color32::from_white_alpha(10)
    } else {
        Color32::from_white_alpha(3)
    };

    ui.painter().rect_filled(card_rect, 6.0, bg_color);
    ui.painter().rect_stroke(card_rect, 6.0, Stroke::new(1.0, border_color), egui::StrokeKind::Inside);

    // Centered plus / image icon
    let icon_box = Rect::from_center_size(pos2(card_rect.center().x, card_rect.center().y - 20.0), vec2(36.0, 36.0));
    let icon_color = if hovered { Color32::WHITE } else { t.text_dim };
    icons::paint(ui, icon_box, "image", 28.0, icon_color);

    // Message
    let drop_text = tl!("Drop an image or PSD anywhere to open it.");
    let max_w = card_rect.width() - 24.0;
    let job = egui::text::LayoutJob::simple(
        drop_text.to_string(),
        egui::FontId::proportional(11.5),
        if hovered { t.text } else { t.text_faint },
        max_w,
    );
    let galley = ui.painter().layout_job(job);
    let text_pos = pos2(card_rect.center().x - galley.size().x / 2.0, card_rect.center().y + 8.0);
    ui.painter().galley(text_pos, galley, t.text_faint);

    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Open…")));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tl!("Open…")).clicked()
}

/// Render a single row in list view with thumbnail icon.
pub(crate) fn render_list_row(
    ui: &mut Ui,
    path: &str,
    name: &str,
    folder: &str,
    thumb: Option<&TextureHandle>,
    row_width: f32,
) -> (Response, Option<RecentAction>) {
    let t = Tokens::get(ui.ctx());
    let (row, resp) = ui.allocate_exact_size(vec2(row_width, LIST_ROW_HEIGHT), Sense::click());
    let hovered = ui.rect_contains_pointer(row);
    if hovered {
        ui.painter().rect_filled(row, t.radius_sm, t.hover);
    }

    let close_rect = Rect::from_center_size(pos2(row.right() - 16.0, row.center().y), vec2(22.0, 22.0));
    let right_limit = if hovered { close_rect.left() - 4.0 } else { row.right() - 10.0 };

    // Thumbnail icon on the left (24x24)
    let thumb_box = Rect::from_center_size(pos2(row.left() + 18.0, row.center().y), vec2(24.0, 24.0));
    ui.painter().rect_filled(thumb_box, t.radius_sm, t.field);
    if let Some(tex) = thumb {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        ui.painter().image(tex.id(), thumb_box, uv, Color32::WHITE);
    } else {
        icons::paint(ui, thumb_box, "file", 14.0, t.text_dim);
    }

    let text_x = row.left() + 38.0;
    let name_g = crate::tab_strip::elided(ui, name, crate::theme::semibold(12.5), t.text, 220.0);
    let name_w = name_g.size().x;
    ui.painter().galley(pos2(text_x, row.center().y - name_g.size().y / 2.0), name_g, t.text);

    let folder_max = (right_limit - (text_x + name_w + 14.0)).max(0.0);
    if folder_max > 20.0 && !folder.is_empty() {
        let fg = crate::tab_strip::elided(ui, folder, egui::FontId::proportional(11.5), t.text_faint, folder_max);
        ui.painter().galley(pos2(right_limit - fg.size().x, row.center().y - fg.size().y / 2.0), fg, t.text_faint);
    }

    let mut action = None;
    if hovered {
        let x_resp = ui.interact(close_rect, resp.id.with("remove"), Sense::click());
        let label = crate::i18n::fmt(tl!("Remove {name} from Recent"), &[("name", name)]);
        x_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        let color = icons::button_chrome(ui, close_rect, false, x_resp.hovered());
        icons::paint(ui, close_rect, "x", 12.0, color);
        if x_resp.on_hover_text(tl!("Remove from Recent")).clicked() {
            action = Some(RecentAction::Remove(path.to_string()));
        }
    }

    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    let resp = resp.on_hover_text(path).on_hover_cursor(egui::CursorIcon::PointingHand);

    if resp.clicked() && action.is_none() {
        action = Some(RecentAction::Open(path.to_string()));
    }

    (resp, action)
}

/// The main entry point to render the Home Recent Files visual grid or list.
pub(crate) fn show_home_recent(app: &mut PhotocraftApp, ui: &mut Ui, recent: &[String], avail_width: f32) {
    let is_grid = app.ui.chrome.home_recent_grid;
    let content_width = avail_width;

    let mut toggle_grid = is_grid;
    recent_header(ui, &mut toggle_grid, content_width);
    if toggle_grid != is_grid {
        app.ui.chrome.home_recent_grid = toggle_grid;
    }

    ui.add_space(14.0);

    if recent.is_empty() {
        let t = Tokens::get(ui.ctx());
        ui.label(egui::RichText::new(tl!("No recent files")).color(t.text_dim).size(13.0));
        return;
    }

    let mut action_to_take = None;

    if is_grid {
        let spacing = 16.0;
        let card_w = GRID_CARD_WIDTH;
        let cols = ((content_width + spacing) / (card_w + spacing)).floor().max(1.0) as usize;

        egui::Grid::new("home_recent_grid_layout")
            .spacing(vec2(spacing, spacing))
            .show(ui, |ui| {
                for (idx, path) in recent.iter().enumerate() {
                    let name = crate::file_open::display_name(path);
                    let folder = Path::new(path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                    let thumb = get_or_load_thumbnail(app, ui.ctx(), path);

                    let (resp, action) = render_grid_card(ui, path, &name, &folder, thumb.as_ref());
                    if let Some(act) = action {
                        action_to_take = Some(act);
                    }

                    let path_clone = path.clone();
                    resp.context_menu(|ui| {
                        if ui.button(tl!("Remove from Recent")).clicked() {
                            action_to_take = Some(RecentAction::Remove(path_clone));
                            ui.close();
                        }
                        if ui.button(tl!("Clear Recent Files")).clicked() {
                            action_to_take = Some(RecentAction::Clear);
                            ui.close();
                        }
                    });

                    if (idx + 1) % cols == 0 {
                        ui.end_row();
                    }
                }

                // Inline drop card as the last card in the grid
                let drop_idx = recent.len();
                if render_inline_drop_card(ui) {
                    action_to_take = Some(RecentAction::OpenDialog);
                }
                if (drop_idx + 1).is_multiple_of(cols) {
                    ui.end_row();
                }
            });
    } else {
        for path in recent {
            let name = crate::file_open::display_name(path);
            let folder = Path::new(path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
            let thumb = get_or_load_thumbnail(app, ui.ctx(), path);

            let (resp, action) = render_list_row(ui, path, &name, &folder, thumb.as_ref(), content_width);
            if let Some(act) = action {
                action_to_take = Some(act);
            }

            let path_clone = path.clone();
            resp.context_menu(|ui| {
                if ui.button(tl!("Remove from Recent")).clicked() {
                    action_to_take = Some(RecentAction::Remove(path_clone));
                    ui.close();
                }
                if ui.button(tl!("Clear Recent Files")).clicked() {
                    action_to_take = Some(RecentAction::Clear);
                    ui.close();
                }
            });
        }
    }

    if let Some(action) = action_to_take {
        match action {
            RecentAction::Open(path) => {
                if let Err(e) = app.open_path(&path) {
                    app.open_failed(&crate::file_open::display_name(&path), &e);
                }
            }
            RecentAction::Remove(path) => {
                app.remove_recent(&path);
            }
            RecentAction::Clear => {
                app.clear_recent();
            }
            RecentAction::OpenDialog => {
                let _ = app.open_dialog_file();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn recent_grid_and_list_toggle_switches_view_mode() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent("/tmp/photocraft-recent-grid/a.png");
        app.push_recent("/tmp/photocraft-recent-grid/b.psd");

        let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 760.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    let recent: Vec<String> = app.ui.recent_files.clone();
                    show_home_recent(app, ui, &recent, 600.0);
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.state_mut().1 = true;
        h.run_steps(2);

        // Initially in Grid mode
        assert!(h.state().0.ui.chrome.home_recent_grid);
        assert!(h.query_by_label("b.psd").is_some());
        assert!(h.query_by_label("a.png").is_some());

        // Toggle to List view
        h.get_by_label("List view").click();
        h.run_steps(2);
        assert!(!h.state().0.ui.chrome.home_recent_grid);

        // Toggle back to Grid view
        h.get_by_label("Grid view").click();
        h.run_steps(2);
        assert!(h.state().0.ui.chrome.home_recent_grid);
    }

    #[test]
    fn recent_grid_card_shows_filename_and_folder() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent("/tmp/photocraft-work/document.psd");

        let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 760.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    let recent: Vec<String> = app.ui.recent_files.clone();
                    show_home_recent(app, ui, &recent, 600.0);
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.state_mut().1 = true;
        h.run_steps(2);

        assert!(h.query_by_label("document.psd").is_some());
    }

    #[test]
    fn recent_thumbnail_cleared_on_remove_and_clear_recent() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let path1 = "/tmp/photocraft-test/thumb1.psd";
        let path2 = "/tmp/photocraft-test/thumb2.psd";

        app.push_recent(path1);
        app.push_recent(path2);

        // Pre-populate dummy entries in thumbnail_cache
        {
            let mut cache = app.thumbnail_cache.lock().unwrap();
            cache.entries.insert(path1.to_string(), ThumbnailEntry {
                key: CacheKey::DiskFile { modified: None, len: 100 },
                texture: None,
                #[cfg(not(target_arch = "wasm32"))]
                last_checked: std::time::Instant::now(),
            });
            cache.entries.insert(path2.to_string(), ThumbnailEntry {
                key: CacheKey::DiskFile { modified: None, len: 200 },
                texture: None,
                #[cfg(not(target_arch = "wasm32"))]
                last_checked: std::time::Instant::now(),
            });
        }

        // Verify entries exist
        assert!(app.thumbnail_cache.lock().unwrap().entries.contains_key(path1));
        assert!(app.thumbnail_cache.lock().unwrap().entries.contains_key(path2));

        // remove_recent removes from thumbnail_cache
        app.remove_recent(path1);
        assert!(!app.thumbnail_cache.lock().unwrap().entries.contains_key(path1));
        assert!(app.thumbnail_cache.lock().unwrap().entries.contains_key(path2));

        // clear_recent clears all entries from thumbnail_cache
        app.clear_recent();
        assert!(app.thumbnail_cache.lock().unwrap().entries.is_empty());
    }

    #[test]
    fn recent_thumbnail_updates_when_open_document_revision_changes() {
        use photocraft_doc::{ColorMode, Document, SampleType, Size};

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let path = "/tmp/photocraft-test/open_doc.psd";
        app.push_recent(path);

        let doc = Document::new("open_doc.psd", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
        app.session.open_document(doc, Some(path.to_string()));
        if let Some(st) = app.session.active_mut() {
            st.path = Some(path.to_string());
            st.revision = 0;
        }

        let ctx = egui::Context::default();
        let tex1 = get_or_load_thumbnail(&app, &ctx, path);
        assert!(tex1.is_some());

        // Verify cache key matches revision 0
        {
            let cache = app.thumbnail_cache.lock().unwrap();
            let entry = cache.entries.get(path).unwrap();
            assert_eq!(entry.key, CacheKey::OpenDoc { revision: 0 });
        }

        // Increment revision (simulate an edit to the open document)
        if let Some(st) = app.session.active_mut() {
            st.revision = 1;
        }

        let tex2 = get_or_load_thumbnail(&app, &ctx, path);
        assert!(tex2.is_some());

        // Verify cache key updated to revision 1
        {
            let cache = app.thumbnail_cache.lock().unwrap();
            let entry = cache.entries.get(path).unwrap();
            assert_eq!(entry.key, CacheKey::OpenDoc { revision: 1 });
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_regenerates_when_disk_file_changes() {
        let temp_dir = std::env::temp_dir().join(format!("photocraft_thumb_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("test_file.png");
        let path_str = file_path.to_string_lossy().into_owned();

        // Write an initial 4x4 red PNG
        let red_png = {
            let img = photocraft_codecs::Image::from_raw(
                4,
                4,
                photocraft_codecs::ChannelLayout::Rgba,
                photocraft_codecs::SampleType::U8,
                [255, 0, 0, 255].repeat(16),
            ).unwrap();
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap()
        };
        std::fs::write(&file_path, &red_png).unwrap();

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent(&path_str);

        let ctx = egui::Context::default();
        let tex1 = get_or_load_thumbnail(&app, &ctx, &path_str);
        assert!(tex1.is_some());

        let initial_len = {
            let cache = app.thumbnail_cache.lock().unwrap();
            let entry = cache.entries.get(&path_str).unwrap();
            match entry.key {
                CacheKey::DiskFile { len, .. } => len,
                _ => panic!("expected DiskFile cache key"),
            }
        };

        // Write a larger 32x32 multi-color PNG to change file size and content
        let blue_png = {
            let mut px = Vec::with_capacity(32 * 32 * 4);
            for i in 0..(32 * 32) {
                px.extend_from_slice(&[(i % 256) as u8, ((i * 7) % 256) as u8, ((i * 13) % 256) as u8, 255]);
            }
            let img = photocraft_codecs::Image::from_raw(
                32,
                32,
                photocraft_codecs::ChannelLayout::Rgba,
                photocraft_codecs::SampleType::U8,
                px,
            ).unwrap();
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap()
        };
        std::fs::write(&file_path, &blue_png).unwrap();

        // Let the recheck interval elapse (10ms in tests)
        std::thread::sleep(std::time::Duration::from_millis(20));

        // get_or_load_thumbnail detects the disk metadata change and regenerates
        let tex2 = get_or_load_thumbnail(&app, &ctx, &path_str);
        assert!(tex2.is_some());

        let new_len = {
            let cache = app.thumbnail_cache.lock().unwrap();
            let entry = cache.entries.get(&path_str).unwrap();
            match entry.key {
                CacheKey::DiskFile { len, .. } => len,
                _ => panic!("expected DiskFile cache key"),
            }
        };

        assert_ne!(initial_len, new_len, "thumbnail cache must reflect the updated file size on disk");
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_recheck_interval_avoids_per_frame_disk_io() {
        let temp_dir = std::env::temp_dir().join(format!("photocraft_fast_thumb_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("test_file.png");
        let path_str = file_path.to_string_lossy().into_owned();

        let img = photocraft_codecs::Image::from_raw(
            4,
            4,
            photocraft_codecs::ChannelLayout::Rgba,
            photocraft_codecs::SampleType::U8,
            [0, 255, 0, 255].repeat(16),
        ).unwrap();
        let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap();
        std::fs::write(&file_path, &png).unwrap();

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent(&path_str);

        let ctx = egui::Context::default();
        let tex1 = get_or_load_thumbnail(&app, &ctx, &path_str);
        assert!(tex1.is_some());

        // Immediately remove the file from disk
        std::fs::remove_file(&file_path).unwrap();

        // Calling get_or_load_thumbnail immediately within the recheck window serves from
        // memory without touching disk or erroring out
        let tex2 = get_or_load_thumbnail(&app, &ctx, &path_str);
        assert!(tex2.is_some(), "subsequent frame within recheck interval must serve from cache");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_skips_composite_for_oversized_psd_without_preview() {
        let temp_dir = std::env::temp_dir().join(format!("photocraft_psd_bound_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("oversized.psd");
        let path_str = file_path.to_string_lossy().into_owned();

        // Create an oversized PSD (3000 x 3000) with no thumbnail resource
        let header = photocraft_psd::Header::new(
            photocraft_psd::Version::Psd,
            3000,
            3000,
            3,
            8,
            photocraft_psd::ColorMode::Rgb,
        );
        let file = photocraft_psd::PsdFile {
            header,
            color_mode_data: Vec::new(),
            resources: Vec::new(),
            layer_info: None,
            layer_info_placement: photocraft_psd::LayerInfoPlacement::Section,
            global_layer_mask: None,
            global_blocks: Vec::new(),
            layer_mask_trailing: Vec::new(),
            image_data: photocraft_psd::ImageData {
                compression: photocraft_psd::Compression::Raw,
                data: Vec::new(),
            },
        };
        let bytes = file.to_bytes().unwrap();
        std::fs::write(&file_path, &bytes).unwrap();

        // load_thumbnail_image should return None due to <= 2048 bound, safely falling back to document icon
        let thumb = load_thumbnail_image(&path_str);
        assert!(thumb.is_none(), "oversized PSD without embedded preview must skip composite fallback");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
