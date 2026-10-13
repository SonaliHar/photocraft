//! Character and paragraph styles ⇄ the PSD text engine's style sheets (see
//! `photocraft_text::psd_styles`). Import merges the named sheets of every type layer into the
//! document's [`TextStyles`] by name and sets the runs' style references; export rewrites each
//! type layer's sheets from the document's styles. Files without named styles are left as they
//! are (byte-exact).

use std::collections::HashMap;

use photocraft_doc::text_styles::{CharacterStyleDef, ParagraphStyleDef, diff_attrs};
use photocraft_doc::{Document, LayerContent, LayerId, TextLayer, TextStyles};

fn checked_style_resolution(doc: &Document) -> Result<photocraft_doc::PhysicalResolution, String> {
    // Borrow canonical custom-unit metadata; validation precedes any owned copy.
    let fallback;
    let resolution = if let Some(resolution) = &doc.physical_resolution {
        resolution
    } else {
        fallback = photocraft_doc::PhysicalResolution::square(f64::from(doc.resolution_dpi));
        &fallback
    };
    resolution.validate().map_err(|error| error.to_string())?;
    Ok(resolution.clone())
}

fn named_style_targets(doc: &Document) -> Result<Vec<LayerId>, String> {
    use std::collections::HashSet;
    let allocation = || "named-style traversal allocation failed".to_string();
    let mut pending = Vec::new();
    pending.try_reserve(1).map_err(|_| allocation())?;
    pending.push((doc.layers.as_slice(), 0usize));
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    while let Some((layers, depth)) = pending.pop() {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err("layer groups exceed the named-style traversal depth limit".into());
        }
        for layer in layers {
            if seen.len() >= 100000 {
                return Err("layers exceed the named-style traversal count limit".into());
            }
            seen.try_reserve(1).map_err(|_| allocation())?;
            if layer.id.0 == 0 || !seen.insert(layer.id) {
                return Err("invalid or duplicate canonical layer identity in named-style import".into());
            }
            match &layer.content {
                LayerContent::Text(text) if text.psd_raw.as_ref().is_some_and(|raw| photocraft_text::psd_styles::has_named_sheets(raw)) => {
                    ids.try_reserve(1).map_err(|_| allocation())?;
                    ids.push(layer.id);
                }
                LayerContent::Group(group) => {
                    pending.try_reserve(1).map_err(|_| allocation())?;
                    pending.push((group.children.as_slice(), depth + 1));
                }
                _ => {}
            }
        }
    }
    Ok(ids)
}

fn retained_warning(error: String) -> Vec<String> {
    vec![format!("Named PSD styles were not imported: {error}; original type data and styles were retained")]
}

/// Builds `doc.text_styles` from the type layers' engine data and links the runs.
pub fn import(doc: &mut Document) -> Vec<String> {
    let resolution = checked_style_resolution(doc);
    let ids = match named_style_targets(doc) {
        Ok(ids) => ids,
        Err(error) => return retained_warning(error),
    };
    // No scalar interpretation is needed for raster-only or unnamed/raw type.
    if ids.is_empty() {
        return Vec::new();
    }
    let resolution = match resolution {
        Ok(resolution) => resolution,
        Err(error) => return retained_warning(error),
    };
    // Parse every paired point basis before publishing any named style or run mutation.
    let mut parsed = Vec::new();
    if parsed.try_reserve(ids.len()).is_err() {
        return retained_warning("named-style staging allocation failed".into());
    }
    for id in ids {
        let Some(LayerContent::Text(text)) = doc.layer(id).map(|layer| &layer.content) else { continue };
        let Some(raw) = &text.psd_raw else { continue };
        match photocraft_text::psd_styles::read_style_sheets_with_resolution(raw, &resolution) {
            Ok(Some(sheets)) => parsed.push((id, sheets)),
            Ok(None) => {}
            Err(error) => return retained_warning(error.to_string()),
        }
    }
    let mut warnings = Vec::new();
    let mut styles = std::mem::take(&mut doc.text_styles);
    let mut char_ids: HashMap<String, u32> = styles.character.iter().map(|d| (d.name.clone(), d.id)).collect();
    let mut para_ids: HashMap<String, u32> = styles.paragraph.iter().map(|d| (d.name.clone(), d.id)).collect();
    for (id, sh) in parsed {
        let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) else { continue };
        let mut cmap: HashMap<usize, u32> = HashMap::new();
        for (ix, name, st) in &sh.char_sheets {
            let id = if let Some(id) = char_ids.get(name).copied() {
                id
            } else {
                let Some(id) = styles.next_char_id() else {
                    warnings.push(format!("character style {:?} was not imported: style id space exhausted", name));
                    continue;
                };
                let mut attrs = diff_attrs(st, &sh.normal_char);
                attrs.remove("postscript_name");
                styles.character.push(CharacterStyleDef { id, name: name.clone(), attrs });
                char_ids.insert(name.clone(), id);
                id
            };
            cmap.insert(*ix, id);
        }
        let mut pmap: HashMap<usize, u32> = HashMap::new();
        for (ix, name, p, cs) in &sh.para_sheets {
            let id = if let Some(id) = para_ids.get(name).copied() {
                id
            } else {
                let Some(id) = styles.next_para_id() else {
                    warnings.push(format!("paragraph style {:?} was not imported: style id space exhausted", name));
                    continue;
                };
                let mut char_attrs = cs.as_ref().map(|c| diff_attrs(c, &sh.normal_char)).unwrap_or_default();
                char_attrs.remove("postscript_name");
                styles.paragraph.push(ParagraphStyleDef { id, name: name.clone(), para_attrs: diff_attrs(p, &sh.normal_para), char_attrs });
                para_ids.insert(name.clone(), id);
                id
            };
            pmap.insert(*ix, id);
        }
        link(t, &sh.run_char, &sh.run_char_lens, &cmap, &sh.run_para, &pmap);
    }
    doc.text_styles = styles;
    warnings
}

fn link(
    t: &mut TextLayer,
    run_char: &[Option<usize>],
    run_char_lens: &[usize],
    cmap: &HashMap<usize, u32>,
    run_para: &[Option<usize>],
    pmap: &HashMap<usize, u32>,
) {
    if run_char_lens.len() == run_char.len() && !run_char.is_empty() {
        // Each model run takes the sheet of the `StyleRun` entry its first character is in.
        let (mut off, mut i, mut end) = (0usize, 0usize, run_char_lens.first().copied().unwrap_or(0));
        for r in t.runs.iter_mut() {
            while off >= end && i + 1 < run_char_lens.len() {
                i += 1;
                end = end.saturating_add(run_char_lens.get(i).copied().unwrap_or(0));
            }
            r.style.style_sheet = run_char.get(i).copied().flatten().and_then(|i| cmap.get(&i).copied());
            off = off.saturating_add(r.len);
        }
    } else if t.runs.len() == run_char.len() {
        for (r, ix) in t.runs.iter_mut().zip(run_char) {
            r.style.style_sheet = ix.and_then(|i| cmap.get(&i).copied());
        }
    }
    if t.paragraphs.len() == run_para.len() {
        for (r, ix) in t.paragraphs.iter_mut().zip(run_para) {
            r.style.style_sheet = ix.and_then(|i| pmap.get(&i).copied());
        }
    }
}

/// The `TySh` data to write for a type layer, with its style sheets regenerated from
/// `styles`; None when neither the document nor the data has named styles (write as is).
pub fn export_tysh(data: &[u8], t: &TextLayer, styles: &TextStyles, dpi: f32) -> Option<Vec<u8>> {
    let named = !styles.character.is_empty() || !styles.paragraph.is_empty();
    if !named && !photocraft_text::psd_styles::has_named_sheets(data) {
        return None;
    }
    photocraft_text::psd_styles::write_style_sheets(data, t, styles, dpi)
}

/// Data already carries the paired TySh transform; named sheets use its Y-DPI
/// point basis and must not apply the anisotropic transform a second time.
pub fn export_tysh_with_resolution(
    data: &[u8],
    t: &TextLayer,
    styles: &TextStyles,
    resolution: &photocraft_doc::PhysicalResolution,
) -> Result<Option<Vec<u8>>, photocraft_text::PhysicalLayoutError> {
    let named = !styles.character.is_empty() || !styles.paragraph.is_empty();
    if !named && !photocraft_text::psd_styles::has_named_sheets(data) {
        return Ok(None);
    }
    photocraft_text::psd_styles::write_style_sheets_with_resolution(data, t, styles, resolution)
}
