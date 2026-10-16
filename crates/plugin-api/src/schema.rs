//! Generated JSON Schema contracts. Cross-field/authority checks also require native validation.

use schemars::{Schema, schema_for};

pub fn descriptor() -> Schema {
    let mut schema = schema_for!(crate::Descriptor);
    // `flatten` combines the common field properties with a tagged parameter's oneOf.
    // Draft 2020-12 unevaluatedProperties closes that combined shape correctly.
    if let Some(field) = schema.get_mut("$defs").and_then(|defs| defs.get_mut("Field")).and_then(serde_json::Value::as_object_mut) {
        field.insert("unevaluatedProperties".into(), serde_json::Value::Bool(false));
    }
    schema
}

pub fn panel_response() -> Schema {
    schema_for!(crate::PanelResponse)
}

pub fn host_api() -> Schema {
    schema_for!(crate::HostApi)
}
