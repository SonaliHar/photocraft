//! Inert current editing metadata. No payload reads, rule evaluation or grants.
use crate::{DocState, EngineError, Result};
use photocraft_doc::{
    item_identity::ItemRef,
    xcf_editing::{ItemFamily, ItemSetRule},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
use std::cell::Cell;

use crate::xcf_inspect_cmds::{reference, short};

const ROWS: usize = 64;
const MASK_SCAN: usize = 10_000;

fn invalid() -> EngineError {
    EngineError::Other("XCF editing inspection contains an invalid identity".into())
}

fn checked(item: ItemRef) -> Result<()> {
    if item.raw() == 0 { Err(invalid()) } else { Ok(()) }
}

/// Independent of collection lengths: only emitted rows and the explicitly
/// capped mask scan consume work. A cancelled view returns no partial state.
struct ViewControl<'a, 'b> {
    interrupt: &'a Interrupt<'b>,
    remaining: Cell<usize>,
}
impl ViewControl<'_, '_> {
    fn check(&self) -> Result<()> {
        self.interrupt.check().map_err(|_| EngineError::Cancelled)?;
        let next = self.remaining.get().checked_sub(1).ok_or_else(|| EngineError::Other("XCF editing inspection work limit".into()))?;
        self.remaining.set(next);
        Ok(())
    }
}

fn bounded_page<T>(values: &[T], control: &ViewControl<'_, '_>, map: impl Fn(&T) -> Result<Value>) -> Result<Value> {
    control.check()?;
    let mut entries = Vec::new();
    entries.try_reserve_exact(values.len().min(ROWS)).map_err(|_| EngineError::Other("XCF editing inspection allocation".into()))?;
    for value in values.iter().take(ROWS) {
        control.check()?;
        entries.push(map(value)?);
    }
    Ok(json!({"total":values.len(),"offset":0,"limit":ROWS,"entries":entries,"truncated":values.len()>ROWS}))
}

fn references(values: &[ItemRef], control: &ViewControl<'_, '_>) -> Result<Value> {
    bounded_page(values, control, |item| {
        checked(*item)?;
        Ok(reference(*item))
    })
}

pub(super) fn editing(d: &DocState) -> Result<Option<Value>> {
    editing_with_interrupt(d, &Interrupt::NONE)
}

pub(super) fn editing_with_interrupt(d: &DocState, interrupt: &Interrupt<'_>) -> Result<Option<Value>> {
    let control = ViewControl { interrupt, remaining: Cell::new(MASK_SCAN + ROWS * ROWS + ROWS * 12) };
    control.check()?;
    let Some(state) = d.doc.xcf_state.as_ref() else { return Ok(None) };
    if let Some(active) = state.selection.active {
        checked(active)?;
    }
    if let Some(target) = state.selection.mask_target {
        checked(target)?;
    }
    if let Some(layer) = d.active_layer {
        checked(ItemRef::Layer(layer))?;
    }
    let runtime_selected = d.selection_ids();
    let selection = json!({
        "selected":references(&state.selection.selected, &control)?,
        "paths":references(&state.selection.selected_paths, &control)?,
        "active":state.selection.active.map(reference),
        "maskTarget":state.selection.mask_target.map(reference),
        "runtimeActiveLayer":d.active_layer.map(|layer| reference(ItemRef::Layer(layer))),
        "runtimeSelectedLayers":bounded_page(runtime_selected, &control, |layer| { checked(ItemRef::Layer(*layer))?; Ok(reference(ItemRef::Layer(*layer))) })?,
    });
    let sets = bounded_page(&state.item_sets, &control, |set| {
        if set.id.0 == 0 {
            return Err(invalid());
        }
        let family = match set.family {
            ItemFamily::Layer => "layer",
            ItemFamily::Channel => "channel",
            ItemFamily::Path => "path",
        };
        let (kind, expression) = match &set.rule {
            ItemSetRule::Named => ("named", None),
            ItemSetRule::Text(text) => ("text", Some(text)),
            ItemSetRule::Regex(text) => ("regex", Some(text)),
            ItemSetRule::Glob(text) => ("glob", Some(text)),
        };
        Ok(json!({"id":set.id.0.to_string(),"name":short(&set.name),"nameTruncated":set.name.chars().nth(256).is_some(),
            "family":family,"memberCount":set.members.len(),"members":references(&set.members, &control)?,
            "ruleKind":kind,"ruleExpression":expression.map(|text|short(text)),
            "ruleTruncated":expression.is_some_and(|text|text.chars().nth(256).is_some()),"ruleEvaluated":false}))
    })?;
    let floating = state
        .floating
        .as_ref()
        .map(|attachment| -> Result<Value> {
            checked(ItemRef::Layer(attachment.layer))?;
            checked(attachment.target)?;
            Ok(json!({"layerId":attachment.layer.0.to_string(),"target":reference(attachment.target)}))
        })
        .transpose()?;
    let mut mask_items = Vec::new();
    mask_items.try_reserve_exact(ROWS).map_err(|_| EngineError::Other("XCF editing inspection allocation".into()))?;
    let mut matching = 0usize;
    for item in state.items.iter().take(MASK_SCAN) {
        control.check()?;
        if item.mask.is_some() {
            matching = matching.saturating_add(1);
            if mask_items.len() < ROWS {
                checked(item.item)?;
                if let Some(mask) = &item.mask {
                    checked(mask.owner)?;
                }
                mask_items.push(item);
            }
        }
    }
    let scan_truncated = state.items.len() > MASK_SCAN;
    let mut masks = bounded_page(&mask_items, &control, |item| {
        Ok(json!({"item":reference(item.item),"rasterBounds":item.raster_bounds,
            "mask":item.mask.as_ref().map(|mask|json!({"owner":reference(mask.owner),"bounds":mask.bounds,"offset":mask.offset,
                "outsideFloat64Bits":format!("0x{:016x}",mask.outside_f64_bits),"enabled":mask.enabled,"shown":mask.shown,
                "editing":mask.editing,"linked":mask.linked}))}))
    })?;
    if let Value::Object(object) = &mut masks {
        object.insert("total".into(), if scan_truncated { Value::Null } else { json!(matching) });
        object.insert("matchingInScan".into(), json!(matching));
        object.insert("scannedItems".into(), json!(state.items.len().min(MASK_SCAN)));
        object.insert("totalItems".into(), json!(state.items.len()));
        object.insert("scanTruncated".into(), json!(scan_truncated));
        object.insert("truncated".into(), json!(scan_truncated || matching > ROWS));
    }
    let filter_masks = bounded_page(&state.filter_masks, &control, |mask| {
        if mask.id.0 == 0 || mask.owner.0 == 0 {
            return Err(invalid());
        }
        Ok(json!({"id":mask.id.0.to_string(),"filterId":mask.owner.0.to_string(),"bounds":mask.bounds}))
    })?;
    control.check()?;
    Ok(Some(json!({"selection":selection,"itemSets":sets,"floating":floating,"maskItems":masks,"filterMasks":filter_masks,
        "rowLimit":ROWS,"fieldCharacterLimit":256,"metadataOnly":true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::LayerId;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn bounded_page_preserves_total_without_visiting_or_inventing_hidden_rows() {
        let values: Vec<_> = (1..=1000).collect();
        let calls = Cell::new(0usize);
        let control = ViewControl { interrupt: &Interrupt::NONE, remaining: Cell::new(ROWS + 1) };
        let result = bounded_page(&values, &control, |value| {
            calls.set(calls.get() + 1);
            Ok(json!(value))
        })
        .unwrap();
        assert_eq!(calls.get(), ROWS);
        assert_eq!(result["total"], 1000);
        assert_eq!(result["entries"].as_array().unwrap().len(), ROWS);
        assert_eq!(result["truncated"], true);
        assert_eq!(control.remaining.get(), 0);
    }

    #[test]
    fn work_exhaustion_and_mid_page_cancellation_return_no_partial_view() {
        let control = ViewControl { interrupt: &Interrupt::NONE, remaining: Cell::new(2) };
        assert!(bounded_page(&[1, 2, 3], &control, |value| Ok(json!(value))).is_err());
        let calls = AtomicUsize::new(0);
        let stop = || calls.fetch_add(1, Ordering::Relaxed) >= 3;
        let interrupt = Interrupt::cancel_only(&stop);
        let control = ViewControl { interrupt: &interrupt, remaining: Cell::new(100) };
        assert!(matches!(bounded_page(&[1, 2, 3, 4], &control, |value| Ok(json!(value))), Err(EngineError::Cancelled)));
    }

    #[test]
    fn emitted_zero_reference_refuses_instead_of_creating_a_current_identity() {
        let control = ViewControl { interrupt: &Interrupt::NONE, remaining: Cell::new(100) };
        assert!(references(&[ItemRef::Layer(LayerId(0))], &control).is_err());
    }
}
