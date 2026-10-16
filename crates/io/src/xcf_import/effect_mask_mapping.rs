//! Shared existing filter selection-mask mapping over exact native storage.
//! Storage transfer changes no source eligibility or empty-selection semantics.
use super::*;

/// Sealed mixed source prepass: actual all-node physical tiles are admitted
/// before the existing mapper transfers or scans even the first mask.
pub(super) fn preflight_filtered_root<P:MappingPlane>(drawable:&Drawable<P>,ctl:&Interrupt<'_>)->Result<(),IoError>{
    if drawable.effects.len()>4096 {return Err(unsupported(drawable,"filtered mask node count"));}
    let mut tiles=0usize;
    for effect in &drawable.effects {
        ctl.check().map_err(|_|IoError::Cancelled)?;
        let plane=effect.mask.as_ref().and_then(|m|m.plane.as_ref()).ok_or_else(||unsupported(drawable,"filtered mask physical plane"))?;
        let actual=plane.retained_filter_mask_tiles().ok_or_else(||unsupported(drawable,"filtered mask physical tile count"))?;
        tiles=tiles.checked_add(actual).ok_or_else(||unsupported(drawable,"filtered mask aggregate tiles"))?;
        if tiles>4096{return Err(unsupported(drawable,"filtered mask aggregate tiles"));}
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn map<P: MappingPlane>(
    drawable: &mut Drawable<P>,
    owner: photocraft_doc::item_identity::ItemRef,
    first_filter: u64,
    next_mask: &mut u64,
    state: &mut photocraft_doc::XcfState,
    budget: &mut Budget,
    ctl: &Interrupt<'_>,
    canvas_group_mask: bool,
    finite_raster_mask: bool,
    value_precision_mask: bool,
    color_balance_depth_mask: bool,
    exposure_geometry_mask: bool,
    point_replace_canvas: Option<Rect>,
    observational_box_canvas:Option<Rect>,
) -> Result<Vec<Option<photocraft_doc::item_identity::ItemRef>>, IoError> {
    map_inner(drawable,owner,first_filter,next_mask,state,budget,ctl,canvas_group_mask,finite_raster_mask,value_precision_mask,color_balance_depth_mask,exposure_geometry_mask,point_replace_canvas,observational_box_canvas,None)
}
/// Private sealed-source entry; callers must already own exact nonempty
/// Selected masked singleton source admission. The actual immutable mask is scanned here.
#[allow(clippy::too_many_arguments)]
pub(super) fn map_masked_source<P:MappingPlane>(drawable:&mut Drawable<P>,owner:photocraft_doc::item_identity::ItemRef,first_filter:u64,next_mask:&mut u64,state:&mut photocraft_doc::XcfState,budget:&mut Budget,ctl:&Interrupt<'_>,canvas:Rect,source_budget:&photocraft_xcf::Budget<'_>)->Result<Vec<Option<photocraft_doc::item_identity::ItemRef>>,IoError>{
    if drawable.effects.len()!=1{return Err(unsupported(drawable,"masked source singleton mapper"));}
    map_inner(drawable,owner,first_filter,next_mask,state,budget,ctl,false,false,false,false,false,Some(canvas),None,Some((source_budget,true)))
}
/// New masked-mixed purpose only. Old singleton nonempty and generic empty
/// entries retain their contracts; the complete original plan caps precede it.
#[allow(clippy::too_many_arguments)]
pub(super) fn map_masked_filtered_flat_source<P:MappingPlane>(drawable:&mut Drawable<P>,owner:photocraft_doc::item_identity::ItemRef,first_filter:u64,next_mask:&mut u64,state:&mut photocraft_doc::XcfState,budget:&mut Budget,ctl:&Interrupt<'_>,canvas:Rect,source_budget:&photocraft_xcf::Budget<'_>)->Result<Vec<Option<photocraft_doc::item_identity::ItemRef>>,IoError>{
 if drawable.effects.len()>1024{return Err(unsupported(drawable,"masked mixed perroot nodes"));}
 map_inner(drawable,owner,first_filter,next_mask,state,budget,ctl,false,false,false,false,false,Some(canvas),None,Some((source_budget,false)))
}
#[allow(clippy::too_many_arguments)]
pub(super) fn map_masked_normal_hierarchy_source<P:MappingPlane>(drawable:&mut Drawable<P>,owner:photocraft_doc::item_identity::ItemRef,first_filter:u64,next_mask:&mut u64,state:&mut photocraft_doc::XcfState,budget:&mut Budget,ctl:&Interrupt<'_>,canvas:Rect,source_budget:&photocraft_xcf::Budget<'_>)->Result<Vec<Option<photocraft_doc::item_identity::ItemRef>>,IoError>{
 if drawable.effects.len()>1024{return Err(unsupported(drawable,"masked hierarchy perroot nodes"));}
 map_inner(drawable,owner,first_filter,next_mask,state,budget,ctl,false,false,false,false,false,Some(canvas),None,Some((source_budget,false)))
}
#[allow(clippy::too_many_arguments)]
fn map_inner<P: MappingPlane>(
    drawable: &mut Drawable<P>,
    owner: photocraft_doc::item_identity::ItemRef,
    first_filter: u64,
    next_mask: &mut u64,
    state: &mut photocraft_doc::XcfState,
    budget: &mut Budget,
    ctl: &Interrupt<'_>,
    canvas_group_mask: bool,
    finite_raster_mask: bool,
    value_precision_mask: bool,
    color_balance_depth_mask: bool,
    exposure_geometry_mask: bool,
    point_replace_canvas: Option<Rect>,
    observational_box_canvas:Option<Rect>,
    coverage:Option<(&photocraft_xcf::Budget<'_>,bool)>,
) -> Result<Vec<Option<photocraft_doc::item_identity::ItemRef>>, IoError> {
    use photocraft_doc::{
        item_identity::{FilterId, ItemRef, MaskId},
        xcf_state::{ItemLocks, ItemState, MaskState},
    };
    if point_replace_canvas.is_some() && observational_box_canvas.is_some(){return Err(IoError::Unsupported("conflicting filter mask canvas policies".into()));}
    let canvas_mask=point_replace_canvas.or(observational_box_canvas);
    let owner_sample = drawable.plane.as_ref().map(|p| p.descriptor().sample);
    let mut needs_owner_bounds = false;
    if canvas_mask.is_none() && !exposure_geometry_mask && !canvas_group_mask {
        for effect in &drawable.effects {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            if effect.mask.is_some() {
                needs_owner_bounds = true;
                break;
            }
        }
    }
    let owner_bounds = if needs_owner_bounds { Some(bounds(drawable)?) } else { None };
    let mut mapped = Vec::new();
    mapped.try_reserve_exact(drawable.effects.len()).map_err(|_| unsupported(drawable, "filter mask allocation failed"))?;
    if coverage.is_some_and(|(_,required)|!required)&&mapped.capacity()>drawable.effects.len().checked_mul(2).ok_or_else(||unsupported(drawable,"masked mixed map capacity"))?{return Err(unsupported(drawable,"masked mixed map capacity"));}
    for (ordinal, effect) in drawable.effects.iter_mut().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let Some(mask) = &mut effect.mask else {
            mapped.push(None);
            continue;
        };
        let rect = bounds(mask)?;
        let plane = mask.plane.as_ref().ok_or_else(|| unsupported(mask, "missing filter mask plane"))?.descriptor();
        if !mask.unsupported.is_empty()
            || !(plane.sample == Sample::U8
                || canvas_mask.is_some() && matches!(plane.sample, Sample::U16 | Sample::U32 | Sample::F32 | Sample::F64)
                || color_balance_depth_mask && plane.sample == Sample::U32
                || (value_precision_mask || exposure_geometry_mask) && plane.sample == Sample::U16
                || (effect.operation == "gimp:levels" || canvas_group_mask) && plane.sample == Sample::F32
                || (finite_raster_mask || exposure_geometry_mask) && matches!(plane.sample, Sample::F32 | Sample::F64) && owner_sample == Some(plane.sample))
            || (finite_raster_mask || exposure_geometry_mask || canvas_mask.is_some()) && owner_sample != Some(plane.sample)
            || plane.channels != 1
            || rect
                != if let Some(canvas) = canvas_mask {
                    canvas
                } else if exposure_geometry_mask {
                    Rect::new(0, 0, 295, 160)
                } else if canvas_group_mask {
                    Rect::new(0, 0, 16, 12)
                } else {
                    owner_bounds.ok_or_else(|| IoError::Unsupported("XCF filter mask owner bounds unavailable".into()))?
                }
            || (finite_raster_mask || exposure_geometry_mask || canvas_mask.is_some())
                && (mask.model != Model::Gray
                    || mask.alpha
                    || !mask.visible
                    || !mask.mask_enabled
                    || mask.mask_shown
                    || mask.mask_editing
                    || mask.legacy_linked
                    || mask.mode != 0
                    || (mask.blend_space, mask.composite_space, mask.composite_mode) != (2, 2, 1)
                    || !mask.effects.is_empty())
        {
            return Err(unsupported(mask, "unqualified filter selection mask domain"));
        }
        let surface = match mask.plane.as_mut() {
            Some(plane) => plane.current_filter_mask_surface(rect, budget, ctl)?,
            None => return Err(unsupported(mask, "missing filter mask plane")),
        };
        let id = MaskId(*next_mask);
        *next_mask = next_mask.checked_add(1).ok_or_else(|| unsupported(mask, "filter mask identity overflow"))?;
        let filter =
            FilterId(u64::try_from(ordinal).ok().and_then(|n| first_filter.checked_add(n)).ok_or_else(|| unsupported(mask, "filter identity overflow"))?);
        let current = photocraft_doc::xcf_filter_masks::FilterMask { id, owner: filter, surface, bounds: rect };
        if let Some((source_budget,require_nonempty))=coverage {
            // A local callback preserves this non-Sync original ledger. Polls
            // also debit one bookkeeping unit so its parent observer checks
            // the original deadline/cancellation even after payload prepay.
            let work=|units:u64|source_budget.consume_work(units.max(1),"masked source physical coverage").map_err(|e|if e.kind==ErrorKind::Cancelled{photocraft_raster::backing::BackingError::Cancelled}else{photocraft_raster::backing::BackingError::Work});
            let proof=current.validate_checked_f32_coverage_with_local_work(ctl,&work).map_err(crate::xcf_point_replace_source::masked_exposure_source::coverage_error)?;
            if require_nonempty && !proof.raw_nonempty() || !require_nonempty && !proof.raw_nonempty() && !proof.logical_all_byte_zero(){return Err(unsupported(mask,"masked source mask classification"));}
        }else if !current.is_empty_selection_with(ctl).map_err(|e| if e.code == "cancelled" { IoError::Cancelled } else { unsupported(mask, e) })? {
            return Err(unsupported(mask, "nonempty filter selection mask evaluation is unqualified"));
        }
        budget.charge(1024 + mask.name.len())?;
        state.filter_masks.try_reserve_exact(1).map_err(|_| unsupported(mask, "filter mask state allocation failed"))?;
        state.items.try_reserve_exact(1).map_err(|_| unsupported(mask, "filter mask item allocation failed"))?;
        let item = ItemRef::Mask(id);
        state.items.push(ItemState {
            item,
            name: mask.name.clone(),
            custom_name: false,
            visible: mask.visible,
            locks: ItemLocks { content: mask.lock_content, position: mask.lock_position, alpha: mask.lock_alpha, visibility: mask.lock_visibility },
            parent: None,
            source_tattoo: mask.tattoo,
            source_flags: mask.group_flags,
            raster_bounds: Some(rect),
            composition: None,
            mask: Some(MaskState {
                owner,
                bounds: rect,
                offset: [mask.offset.0, mask.offset.1],
                outside_f64_bits: 0.0_f64.to_bits(),
                enabled: mask.mask_enabled,
                shown: mask.mask_shown,
                editing: mask.mask_editing,
                linked: mask.legacy_linked,
            }),
        });
        state.filter_masks.push(current);
        mapped.push(Some(item));
    }
    Ok(mapped)
}

#[cfg(test)]
#[path = "effect_mask_mapping_tests.rs"]
mod tests;
