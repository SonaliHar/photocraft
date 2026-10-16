//! Manifest migrations. Each step upgrades the raw JSON from version `n` to
//! `n + 1`, so old bundles keep loading as the schema evolves.

use serde_json::Value;

use crate::manifest::FORMAT_VERSION;
use crate::{FormatError, Result};

pub(crate) type Step = fn(&mut Value) -> Result<()>;

/// `STEPS[i]` upgrades version `i + 1` to `i + 2`.
pub(crate) const STEPS: &[Step] = &[
    resolution_axes,
    source_cache_blobs,
    current_text_origin,
    finite_raster_content,
    current_resource_resolution,
    current_linked_geometry,
    brown_layer_label,
    stored_color,
    current_storage_hint,
    point_f32_coverage,
    masked_filtered_flat,
];
// 11 -> 12: new explicit structural handoff only; invent no intent.
fn masked_filtered_flat(value:&mut Value)->Result<()> {
    if value.get("document").and_then(Value::as_object).is_none(){return Err(FormatError::corrupt("manifest has no document object"))}Ok(())
}
// 10 -> 11: a new closed hint value; absence and old values remain exact.
fn point_f32_coverage(value: &mut Value) -> Result<()> {
    if value.get("document").and_then(Value::as_object).is_none() {
        return Err(FormatError::corrupt("manifest has no document object"));
    }
    Ok(())
}
fn stored_color(value: &mut Value) -> Result<()> {
    if value.get("document").and_then(Value::as_object).is_none() {
        return Err(FormatError::corrupt("manifest has no document object"));
    }
    // StoredColor is already explicit data; invent no literal or encoding.
    Ok(())
}
fn current_storage_hint(value: &mut Value) -> Result<()> {
    if value.get("document").and_then(Value::as_object).is_none() {
        return Err(FormatError::corrupt("manifest has no document object"));
    }
    // Absence remains absence. Neither topology nor source provenance selects a class.
    Ok(())
}
fn brown_layer_label(value: &mut Value) -> Result<()> {
    if value.get("document").and_then(Value::as_object).is_none() {
        return Err(FormatError::corrupt("manifest has no document object"));
    }
    // Existing enum names and layer records stay exact; migration invents no label.
    Ok(())
}
fn current_linked_geometry(value: &mut Value) -> Result<()> {
    if value.get("document").and_then(Value::as_object).is_none() {
        return Err(FormatError::corrupt("manifest has no document object"));
    }
    // The optional current geometry defaults absent. Keep original source
    // transforms/cache blobs unchanged; never fabricate an edit during migration.
    Ok(())
}
fn current_resource_resolution(value: &mut Value) -> Result<()> {
    if value.get("document").and_then(Value::as_object).is_none() {
        return Err(FormatError::corrupt("manifest has no document object"));
    }
    // Optional current resolutions default absent; original resource identity and
    // source-state blobs are unchanged. No filesystem authority is serialized.
    Ok(())
}
fn finite_raster_content(value: &mut Value) -> Result<()> {
    if value.get("document").and_then(Value::as_object).is_none() {
        return Err(FormatError::corrupt("manifest has no document object"));
    }
    // Preserve all old resident/native/source bytes; no finite extent invented.
    Ok(())
}
fn current_text_origin(v: &mut Value) -> Result<()> {
    if v.get("document").and_then(Value::as_object).is_none() {
        return Err(FormatError::corrupt("manifest has no document object"));
    }
    // Source-state blobs retain their original bytes. The new optional placement
    // defaults to None on decode; migration never invents an edit or new cache.
    Ok(())
}
fn source_cache_blobs(v: &mut Value) -> Result<()> {
    if v.get("document").and_then(Value::as_object).is_none() {
        return Err(FormatError::corrupt("manifest has no document object"));
    }
    // Inline caches remain valid and are decoded by the existing source state model.
    // The new optional blob records default empty; migration never fabricates bytes.
    Ok(())
}
fn resolution_axes(v: &mut Value) -> Result<()> {
    let document = v.get_mut("document").and_then(Value::as_object_mut).ok_or_else(|| FormatError::corrupt("manifest has no document object"))?;
    if !document.contains_key("physical_resolution") {
        let dpi = document.get("resolution_dpi").and_then(Value::as_f64).ok_or_else(|| FormatError::corrupt("legacy resolution missing"))?;
        // Version 1 stored this field as f32. Recover that value before widening.
        let dpi = dpi as f32;
        if !dpi.is_finite() || dpi <= 0.0 {
            return Err(FormatError::corrupt("legacy resolution must be positive finite"));
        }
        let dpi = f64::from(dpi);
        document.insert("physical_resolution".into(), serde_json::json!({"x_dpi": dpi,"y_dpi": dpi,"unit":"Inch"}));
    }
    let object = v.as_object_mut().ok_or_else(|| FormatError::corrupt("manifest object missing"))?;
    object.insert("minimum_reader_version".into(), Value::from(2));
    Ok(())
}

/// Upgrade `v` in place to `target` using `steps`.
pub(crate) fn migrate_with(v: &mut Value, steps: &[Step], target: u32) -> Result<u32> {
    let found = v.get("format_version").and_then(Value::as_u64).ok_or_else(|| FormatError::corrupt("manifest has no format_version"))?;
    let found = u32::try_from(found).map_err(|_| FormatError::corrupt("format_version out of range"))?;
    if found == 0 {
        return Err(FormatError::corrupt("format_version 0 is invalid"));
    }
    if found > target {
        return Err(FormatError::TooNew { found, supported: target });
    }
    for ver in found..target {
        let step = steps.get(ver as usize - 1).ok_or_else(|| FormatError::Unsupported(format!("no migration from format {ver}")))?;
        step(v)?;
        v["format_version"] = Value::from(ver + 1);
    }
    Ok(found)
}

pub(crate) fn migrate(v: &mut Value) -> Result<u32> {
    migrate_with(v, STEPS, FORMAT_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rename_title(v: &mut Value) -> Result<()> {
        let t = v["document"]["title"].take();
        v["document"]["name"] = t;
        Ok(())
    }
    fn add_field(v: &mut Value) -> Result<()> {
        v["document"]["added"] = json!(true);
        Ok(())
    }

    #[test]
    fn chained_steps_run_in_order() {
        let mut v = json!({"format_version": 1, "document": {"title": "old"}});
        let found = migrate_with(&mut v, &[rename_title, add_field], 3).unwrap();
        assert_eq!(found, 1);
        assert_eq!(v["format_version"], 3);
        assert_eq!(v["document"]["name"], "old");
        assert_eq!(v["document"]["added"], true);
    }

    #[test]
    fn partial_chain_from_middle() {
        let mut v = json!({"format_version": 2, "document": {}});
        migrate_with(&mut v, &[rename_title, add_field], 3).unwrap();
        assert_eq!(v["document"]["added"], true);
        assert!(v["document"].get("name").is_none());
    }

    #[test]
    fn too_new_and_invalid_versions() {
        let mut v = json!({"format_version": 99});
        assert!(matches!(migrate(&mut v), Err(FormatError::TooNew { found: 99, .. })));
        assert!(migrate(&mut json!({"format_version": 0})).is_err());
        assert!(migrate(&mut json!({})).is_err());
        assert!(migrate_with(&mut json!({"format_version": 1}), &[], 2).is_err());
    }

    #[test]
    fn current_version_is_noop() {
        let mut v = json!({"format_version": FORMAT_VERSION});
        assert_eq!(migrate(&mut v).unwrap(), FORMAT_VERSION);
    }
}
