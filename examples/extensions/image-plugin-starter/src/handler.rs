//! Panel events propose a command; pixels remain in the host and selected backend.
use serde_json::{Value, json};

fn reply(state: &Value, message: &str) -> Value {
    json!({"state": state, "message": message, "commands": []})
}

pub(crate) fn handle(request: &Value) -> Value {
    let state = request.get("state").unwrap_or(&Value::Null);
    let Some(object) = request.as_object() else { return reply(state, "Invalid event request.") };
    if object.keys().any(|key| !matches!(key.as_str(), "event" | "values" | "state" | "document")) {
        return reply(state, "Unknown event request field.");
    }
    if request.get("event").and_then(Value::as_str) != Some("process") {
        return reply(state, "Unknown action. Use Generate preview.");
    }
    let Some(values) = request.get("values").and_then(Value::as_object) else { return reply(state, "Panel values must be an object.") };
    if values.keys().any(|key| !matches!(key.as_str(), "gain" | "backend")) {
        return reply(state, "Unknown panel field.");
    }
    let gain = match values.get("gain") {
        None => 1.25,
        Some(value) => match value.as_f64().filter(|gain| gain.is_finite() && (0.0..=2.0).contains(gain)) {
            Some(gain) => gain,
            None => return reply(state, "Gain must be a number from 0 to 2."),
        },
    };
    let processor = match values.get("backend").and_then(Value::as_str) {
        None if !values.contains_key("backend") => "native",
        Some("Native worker") => "native",
        Some("Sandboxed filter") => "org.example.channel-gain",
        _ => return reply(state, "Choose a declared processing backend."),
    };
    let Some(document) = request.get("document").and_then(Value::as_object) else {
        return reply(state, "Open an RGB or grayscale image before generating a preview.");
    };
    if !["id", "width", "height", "revision"].iter().all(|key| document.get(*key).and_then(Value::as_u64).is_some()) {
        return reply(state, "The document summary is incomplete.");
    }
    json!({
        "state": state,
        "message": "Processing image. Compare the preview, then choose Apply or Discard.",
        "commands": [{"command": "plugin.processImage", "params": {
            "processor": processor, "options": {"gain": gain}, "preview": true
        }}]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Value {
        json!({"event": "process", "values": {}, "state": {"custom": 1},
            "document": {"id": 1, "width": 8, "height": 6, "revision": 0}})
    }

    #[test]
    fn native_defaults_request_only_the_own_installation_processor() {
        let input = request();
        let result = handle(&input);
        assert_eq!(
            result["commands"],
            json!([{"command": "plugin.processImage", "params": {
                "processor": "native", "options": {"gain": 1.25}, "preview": true
            }}])
        );
        assert_eq!(result["state"], input["state"]);
        assert!(result["commands"][0]["params"].get("id").is_none());
    }

    #[test]
    fn packaged_filter_is_selected_by_its_embedded_manifest_id() {
        let mut input = request();
        input["values"] = json!({"backend": "Sandboxed filter", "gain": 0.5});
        let result = handle(&input);
        assert_eq!(result["commands"][0]["params"]["processor"], "org.example.channel-gain");
        assert_eq!(result["commands"][0]["params"]["options"], json!({"gain": 0.5}));
    }

    #[test]
    fn malformed_values_and_requests_never_start_processing() {
        for values in [
            json!([]),
            json!(null),
            json!({"gain": -1}),
            json!({"gain": 2.1}),
            json!({"gain": "1"}),
            json!({"backend": false}),
            json!({"backend": "another-plugin"}),
            json!({"path": "worker.exe"}),
        ] {
            let mut input = request();
            input["values"] = values;
            assert_eq!(handle(&input)["commands"], json!([]));
        }
        for input in [json!([]), json!(null), json!({"event": "delete"})] {
            assert_eq!(handle(&input)["commands"], json!([]));
        }
    }

    #[test]
    fn no_or_malformed_document_preserves_state_without_commands() {
        for document in [json!(null), json!({}), json!({"id": "1"})] {
            let mut input = request();
            input["document"] = document;
            let result = handle(&input);
            assert_eq!(result["commands"], json!([]));
            assert_eq!(result["state"], input["state"]);
        }
    }
}
