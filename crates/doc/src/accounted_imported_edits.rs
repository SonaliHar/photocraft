//! Closed current metadata/native edits. Original imported evidence remains inert
//! and shared; raw native writes do not grant source or renderer qualification.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EditDomain {
    Resident,
    Plain,
    AssignedGeometry,
    IsolatedGroup,
    GaussianGroup,
    FlatComposite,
    PointMetadata,
    FilteredFlatMetadata,
}

pub(super) fn validate_owner(owner: &AccountedDocument, edit: AccountedEdit<'_>, c: &ReadControl<'_>) -> Result<EditDomain, BackingError> {
    // Actual Point classes admit only the structural compound metadata plan.
    // This runs before generic no-op/history admission and grants no recipe.
    if owner.is_readonly_filtered_flat_point_storage() || owner.is_readonly_masked_filtered_flat_point_storage() {
        filtered_flat_readonly::metadata::validate(owner, edit, c)?;
        return Ok(EditDomain::FilteredFlatMetadata);
    }
    if owner.is_readonly_point_storage() || owner.is_readonly_assigned_point_storage() {
        point_readonly::metadata::validate(owner, edit, c)?;
        return Ok(EditDomain::PointMetadata);
    }
    // The separate compound-only plan validates actual class/current metadata
    // before generic no-op/history admission, including Normal-surrogate modes.
    if owner.is_readonly_flat_composite_storage() {
        flat_composite_readonly::metadata::validate(owner, edit, c)?;
        return Ok(EditDomain::FlatComposite);
    }
    // A genuine Gaussian class must never fall through to the unrelated
    // assigned group hint or acquire plain/native-pixel edit authority.
    if owner.is_readonly_gaussian_group_storage() {
        gaussian_group_readonly::metadata::validate(owner, edit, c)?;
        return Ok(EditDomain::GaussianGroup);
    }
    let imported = imported_selection::applies(owner.document());
    if imported {
        // Only this exact bounded group lane can copy recursive containers;
        // selecting it is authoritative and never falls back to the flat lane.
        if matches!(edit, AccountedEdit::LayerMetadata { .. } | AccountedEdit::MaskEnabled { .. })
            && owner.document().layers.len() == 2
            && owner.document().layers.get(1).is_some_and(|layer| matches!(&layer.content, LayerContent::Group(_)))
        {
            if matches!(edit, AccountedEdit::MaskEnabled { .. }) {
                c.check(1)?;
            } else {
                validate_parameters(edit, c)?;
            }
            owner.validate_assigned_isolated_normal_group_storage(c)?;
            validate_group_items(owner.document(), c)?;
            return Ok(EditDomain::IsolatedGroup);
        }
        // This hint selects only a validator. It cannot grant source, pixel or
        // generic imported edit authority, and a selected refusal is final.
        if matches!(edit, AccountedEdit::LayerMetadata { .. } | AccountedEdit::MaskEnabled { .. })
            && owner.document().layers.len() == 2
            && owner.document().layers.get(1).is_some_and(|layer| layer.mask.is_some())
        {
            if matches!(edit, AccountedEdit::MaskEnabled { .. }) {
                c.check(1)?;
            } else {
                validate_parameters(edit, c)?;
            }
            owner.validate_assigned_analytic_geometry_storage(c)?;
            // A malformed native current with divergent canonical names is
            // refused; omitted fields do not silently repair unrelated state.
            validate_current_items(owner.document(), true, c)?;
            return Ok(EditDomain::AssignedGeometry);
        }
        validate(owner.document(), edit, owner.quota(), c)?;
        selection_census(owner.document(), false, owner.owner._selection.as_ref(), owner.quota(), c)?;
    } else if matches!(edit, AccountedEdit::LayerMetadata { .. }) {
        return Err(BackingError::Unsupported);
    }
    Ok(if imported { EditDomain::Plain } else { EditDomain::Resident })
}

pub(super) fn validate(doc: &Document, edit: AccountedEdit<'_>, quota: &BackingQuota, c: &ReadControl<'_>) -> Result<(), BackingError> {
    if let AccountedEdit::NativePixels { layer, rect, bytes } = edit {
        let target = accounted_layer_access(doc, layer, c)?.ok_or(BackingError::Bounds)?;
        let LayerContent::BackedRaster(backed) = &target.layer.content else { return Err(BackingError::Unsupported) };
        validate_native(backed, rect, bytes, c)?;
    }
    validate_parameters(edit, c)?;
    imported_selection::validate(doc, quota, c)?;
    validate_current_items(doc, false, c)
}

fn validate_parameters(edit: AccountedEdit<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
    match edit {
        AccountedEdit::LayerMetadata { name, opacity, .. } => {
            if let Some(name) = name {
                validate_name(name, c)?;
            }
            if opacity.is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v)) {
                return Err(BackingError::Bounds);
            }
        }
        AccountedEdit::LayerName { name, .. } => {
            validate_name(name, c)?;
        }
        AccountedEdit::LayerVisibility { .. } => {}
        AccountedEdit::NativePixels { .. } => {}
        AccountedEdit::LayerOpacity { opacity, .. } => {
            if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
                return Err(BackingError::Bounds);
            }
        }
        _ => return Err(BackingError::Unsupported),
    }
    Ok(())
}

fn validate_current_items(doc: &Document, allow_mask: bool, c: &ReadControl<'_>) -> Result<(), BackingError> {
    // Preserve one unambiguous current name/visibility representation. No
    // source payload is rewritten, and arbitrary composition remains unchanged.
    if let Some(state) = &doc.xcf_state {
        for item in &state.items {
            c.check(1)?;
            let crate::item_identity::ItemRef::Layer(id) = item.item else {
                if allow_mask && matches!(item.item, crate::item_identity::ItemRef::Mask(_)) {
                    continue;
                }
                return Err(BackingError::Unsupported);
            };
            let mut found = None;
            for layer in &doc.layers {
                c.check(1)?;
                if layer.id == id {
                    found = Some(layer);
                }
            }
            let layer = found.ok_or(BackingError::Integrity)?;
            if !names_equal(&item.name, &layer.name, c)? || item.visible != layer.visible {
                return Err(BackingError::Integrity);
            }
        }
    }
    Ok(())
}

fn validate_group_items(doc: &Document, c: &ReadControl<'_>) -> Result<(), BackingError> {
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Unsupported)?;
    for item in &state.items {
        c.check(1)?;
        let crate::item_identity::ItemRef::Layer(id) = item.item else {
            if matches!(item.item, crate::item_identity::ItemRef::Mask(_)) {
                continue;
            }
            return Err(BackingError::Unsupported);
        };
        let layer = accounted_layer_access(doc, id, c)?.ok_or(BackingError::Integrity)?.layer;
        // As for the flat imported domain, malformed canonical prestate refuses
        // before no-op; omitted properties never repair unrelated native state.
        if !names_equal(&item.name, &layer.name, c)? || item.visible != layer.visible {
            return Err(BackingError::Integrity);
        }
    }
    Ok(())
}

pub(super) fn synchronize(doc: &mut Document, edit: AccountedEdit<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
    if let AccountedEdit::LayerMetadata { layer, name, visible, .. } = edit {
        if let Some(name) = name {
            synchronize(doc, AccountedEdit::LayerName { layer, name }, c)?;
        }
        if let Some(visible) = visible {
            synchronize(doc, AccountedEdit::LayerVisibility { layer, visible }, c)?;
        }
        return Ok(());
    }
    let Some(state) = &mut doc.xcf_state else {
        return Ok(());
    };
    if let AccountedEdit::MaskEnabled { layer, enabled } = edit {
        // Only the sealed assigned edit domain reaches this branch. Complete
        // the controlled unique relation scan before touching canonical state.
        let mut found = None;
        for (index, item) in state.items.iter().enumerate() {
            c.check(1)?;
            if item.mask.as_ref().is_some_and(|mask| mask.owner == crate::item_identity::ItemRef::Layer(layer)) {
                if found.is_some() || !matches!(item.item, crate::item_identity::ItemRef::Mask(_)) {
                    return Err(BackingError::Integrity);
                }
                found = Some(index);
            }
        }
        c.check(1)?;
        let mask = state.items.get_mut(found.ok_or(BackingError::Integrity)?).and_then(|item| item.mask.as_mut()).ok_or(BackingError::Integrity)?;
        mask.enabled = enabled;
        return Ok(());
    }
    let id = crate::item_identity::ItemRef::Layer(edit.layer_id());
    let mut found = None;
    for (index, item) in state.items.iter().enumerate() {
        c.check(1)?;
        if item.item == id {
            if found.is_some() {
                return Err(BackingError::Integrity);
            }
            found = Some(index);
        }
    }
    let item = state.items.get_mut(found.ok_or(BackingError::Integrity)?).ok_or(BackingError::Integrity)?;
    match edit {
        AccountedEdit::LayerName { name, .. } => {
            if !names_equal(&item.name, name, c)? {
                item.name = copy_name(name, c)?;
                item.custom_name = true;
            }
        }
        AccountedEdit::LayerVisibility { visible, .. } => item.visible = visible,
        AccountedEdit::LayerOpacity { .. } | AccountedEdit::NativePixels { .. } => {} // Original metadata/composition is unchanged.
        _ => return Err(BackingError::Unsupported),
    }
    Ok(())
}

fn validate_native(backed: &crate::BackedLayer, rect: Rect, bytes: &[u8], c: &ReadControl<'_>) -> Result<usize, BackingError> {
    c.check(1)?;
    if rect.is_empty() || !backed.operative_bounds().contains_rect(&rect) {
        return Err(BackingError::Bounds);
    }
    let bpp = backed.format().bytes_per_pixel();
    if bpp == 0 || bpp > 40 {
        return Err(BackingError::Unsupported);
    }
    let required = u64::from(rect.width()).checked_mul(u64::from(rect.height())).and_then(|n| n.checked_mul(bpp as u64)).ok_or(BackingError::Length)?;
    if required > photocraft_raster::MAX_CHECKED_RASTER_BYTES || required != bytes.len() as u64 {
        return Err(BackingError::Length);
    }
    Ok(bpp)
}
fn native_view_error(error: photocraft_raster::checked_view::RasterViewError) -> BackingError {
    use photocraft_raster::checked_view::RasterViewError as E;
    match error {
        E::Backed(e) | E::Control(e) | E::Descriptor(e) => e,
        E::Length => BackingError::Length,
        E::Bounds => BackingError::Bounds,
        E::Resident(_) | E::Sample(_) => BackingError::Integrity,
    }
}
/// Raw equality in the already validated imported plain class. Tile/cache IO
/// remains genuinely charged to its original owner; no source plane or Surface
/// copy is created. Stack scratch is fixed, and every read/comparison polls.
pub(super) fn native_is_noop(layer: &crate::Layer, rect: Rect, bytes: &[u8], c: &ReadControl<'_>) -> Result<bool, BackingError> {
    let LayerContent::BackedRaster(backed) = &layer.content else { return Err(BackingError::Unsupported) };
    let bpp = validate_native(backed, rect, bytes, c)?;
    let width = usize::try_from(rect.width()).map_err(|_| BackingError::Length)?;
    let view = backed.checked_view();
    let mut scratch = [0u8; 40];
    for coord in rect.tiles() {
        c.check(1)?;
        let x = coord.tx.checked_mul(photocraft_geom::TILE_SIZE).ok_or(BackingError::Bounds)?;
        let y = coord.ty.checked_mul(photocraft_geom::TILE_SIZE).ok_or(BackingError::Bounds)?;
        let tile = Rect::new(
            x,
            y,
            x.checked_add(photocraft_geom::TILE_SIZE).ok_or(BackingError::Bounds)?,
            y.checked_add(photocraft_geom::TILE_SIZE).ok_or(BackingError::Bounds)?,
        );
        let overlap = tile.intersect(&rect);
        let pin = view.pin_tile(coord, c).map_err(native_view_error)?;
        for y in overlap.y0..overlap.y1 {
            for x in overlap.x0..overlap.x1 {
                c.check((bpp as u64).checked_mul(2).ok_or(BackingError::Length)?)?;
                let row = usize::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| BackingError::Bounds)?;
                let column = usize::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| BackingError::Bounds)?;
                let offset = row.checked_mul(width).and_then(|n| n.checked_add(column)).and_then(|n| n.checked_mul(bpp)).ok_or(BackingError::Length)?;
                let end = offset.checked_add(bpp).ok_or(BackingError::Length)?;
                let expected = bytes.get(offset..end).ok_or(BackingError::Length)?;
                let actual = scratch.get_mut(..bpp).ok_or(BackingError::Length)?;
                pin.read_pixel_native_into(x, y, actual, c).map_err(native_view_error)?;
                if &*actual != expected {
                    return Ok(false);
                }
            }
        }
    }
    c.check(1)?;
    Ok(true)
}

pub(super) fn validate_name(name: &str, c: &ReadControl<'_>) -> Result<(), BackingError> {
    if name.len() > 1 << 20 {
        return Err(BackingError::Length);
    }
    for chunk in name.as_bytes().chunks(32768) {
        c.check(chunk.len() as u64)?;
        if chunk.contains(&0) {
            return Err(BackingError::Length);
        }
    }
    Ok(())
}

pub(super) fn names_equal(a: &str, b: &str, c: &ReadControl<'_>) -> Result<bool, BackingError> {
    if a.len() != b.len() {
        c.check(1)?;
        return Ok(false);
    }
    for (a, b) in a.as_bytes().chunks(32768).zip(b.as_bytes().chunks(32768)) {
        c.check((a.len() as u64).checked_mul(2).ok_or(BackingError::Length)?)?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn copy_name(name: &str, c: &ReadControl<'_>) -> Result<String, BackingError> {
    let mut owned = String::new();
    owned.try_reserve_exact(name.len()).map_err(|_| BackingError::Allocation)?;
    if owned.capacity() > name.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation);
    }
    for ch in name.chars() {
        c.check(ch.len_utf8() as u64)?;
        owned.push(ch);
    }
    Ok(owned)
}

#[cfg(test)]
#[path = "accounted_imported_edits_tests.rs"]
mod tests;
