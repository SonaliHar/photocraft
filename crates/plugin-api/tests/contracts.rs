use photocraft_plugin_api::*;
use serde_json::{Value, json};

fn descriptor() -> Value {
    json!({"apiVersion":2,"id":"org.example.layers","name":"Layer Organizer","version":"0.1.0",
        "permissions":{"documentRead":true,"commands":[]},
        "panel":{"fields":[{"id":"visible","label":"Visible","type":"bool","default":true}],"buttons":[{"id":"apply","label":"Apply"}]},
        "capabilities":{"required":[{"id":"host.document.edit","minVersion":1,"maxVersion":1}],"optional":[{"id":"future.optional","minVersion":1}]},
        "contributes":{"commands":[{"id":"org.example.layers.organize","label":"Organize Layers","event":"organize","params":{},"needsDocument":true}]}})
}

#[test]
fn complete_descriptor_round_trip_is_independent_of_editor_types() {
    let bytes = serde_json::to_vec(&descriptor()).unwrap();
    let first = Descriptor::parse(&bytes).unwrap();
    let second = Descriptor::parse(&serde_json::to_vec(&first).unwrap()).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.contributes.commands[0].event, "organize");
}

#[test]
fn missing_required_and_wrong_version_have_structured_errors_optional_falls_back() {
    let parsed = Descriptor::parse(&serde_json::to_vec(&descriptor()).unwrap()).unwrap();
    let absent = negotiate(&parsed.capabilities, &[]).unwrap_err();
    assert_eq!(absent.code, ErrorCode::MissingCapability);
    assert_eq!(absent.details["capability"], "host.document.edit");
    let host = [CapabilitySupport { id: KnownCapability::DocumentEdit.into(), version: 1 }];
    let accepted = negotiate(&parsed.capabilities, &host).unwrap();
    assert_eq!(accepted.available, host);
    assert_eq!(accepted.unavailable_optional[0].as_str(), "future.optional");
    let wrong = [CapabilitySupport { id: KnownCapability::DocumentEdit.into(), version: 2 }];
    assert_eq!(negotiate(&parsed.capabilities, &wrong).unwrap_err().code, ErrorCode::UnsupportedVersion);
}

#[test]
fn malformed_duplicate_and_unbounded_descriptors_fail_without_panicking() {
    for (pointer, invalid) in [
        ("/apiVersion", json!(1)),
        ("/id", json!("../escape")),
        ("/name", json!("\u{202e}Layers")),
        ("/capabilities/required/0/minVersion", json!(0)),
        ("/capabilities/required/0/maxVersion", json!(0)),
        ("/capabilities/required/0/id", json!("malformed id")),
        ("/contributes/commands/0/id", json!("org.another.plugin.command")),
        ("/contributes/commands/0/event", json!("_hidden")),
        ("/contributes/commands/0/params", json!([])),
    ] {
        let mut value = descriptor();
        *value.pointer_mut(pointer).unwrap() = invalid;
        assert!(Descriptor::parse(&serde_json::to_vec(&value).unwrap()).is_err(), "{pointer}");
    }
    let mut duplicate = descriptor();
    duplicate["capabilities"]["optional"] = duplicate["capabilities"]["required"].clone();
    assert!(Descriptor::parse(&serde_json::to_vec(&duplicate).unwrap()).is_err());
    let mut value = descriptor();
    value["unexpected"] = json!(true);
    assert!(Descriptor::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    assert!(Descriptor::parse(&vec![b' '; MAX_DESCRIPTOR_BYTES + 1]).is_err());
    assert!(Descriptor::parse(&[0xff]).is_err());
    assert!(Descriptor::parse(br#"{"apiVersion":2,"apiVersion":2}"#).is_err());
    let nested = format!("{}0{}", "[".repeat(1024), "]".repeat(1024));
    assert!(Descriptor::parse(nested.as_bytes()).is_err());
}

#[test]
fn contributions_are_unique_across_categories_and_inspector_needs_no_button() {
    let mut value = descriptor();
    value["panel"]["buttons"] = json!([]);
    assert!(Descriptor::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    value["contributes"]["inspectors"] = json!([{"id":"org.example.layers.inspector","label":"Document Inspector","event":"inspect"}]);
    value["contributes"]["subscriptions"] = json!([{"id":"org.example.layers.events","event":"document.changed","handler":"inspect"}]);
    assert!(Descriptor::parse(&serde_json::to_vec(&value).unwrap()).is_ok());
    value["contributes"]["inspectors"][0]["id"] = json!("org.example.layers.organize");
    assert!(Descriptor::parse(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn response_controls_are_bounded_and_do_not_grant_authority() {
    let response = json!({"requests":[{"id":"read","method":"host.document.read","params":{"document":"fabricated"}}],
        "view":{"title":"Layers","rows":[{"label":"Count","value":"3"}]}});
    let parsed = PanelResponse::parse(&serde_json::to_vec(&response).unwrap()).unwrap();
    // Parsing an opaque handle deliberately says nothing about whether the host accepts it.
    assert_eq!(parsed.requests[0].params["document"], "fabricated");
    let mut mixed = response.clone();
    mixed["commands"] = json!([{"command":"edit.fill","params":{}}]);
    assert!(PanelResponse::parse(&serde_json::to_vec(&mixed).unwrap()).is_err());
    let mut duplicate = response;
    let repeated = duplicate["requests"][0].clone();
    duplicate["requests"].as_array_mut().unwrap().push(repeated);
    assert!(PanelResponse::parse(&serde_json::to_vec(&duplicate).unwrap()).is_err());
    assert!(PanelResponse::parse(&serde_json::to_vec(&json!({"state":"x".repeat(MAX_STATE_BYTES)})).unwrap()).is_err());
}

#[test]
fn metadata_versions_and_error_codes_round_trip_independently() {
    let metadata = HostApi { version: HOST_API_VERSION, capabilities: vec![CapabilitySupport { id: KnownCapability::DocumentRead.into(), version: 1 }] };
    assert_eq!(serde_json::from_value::<HostApi>(serde_json::to_value(&metadata).unwrap()).unwrap(), metadata);
    let error = ApiError::at(ErrorCode::InvalidHandle, "document", "handle expired");
    assert_eq!(serde_json::from_value::<ApiError>(serde_json::to_value(&error).unwrap()).unwrap(), error);
    assert!(CapabilityId::new("future.optional").unwrap().known().is_none());
    assert_eq!(CapabilityId::from(KnownCapability::Events).known(), Some(KnownCapability::Events));
    assert!(metadata.validate().is_ok());
    assert_eq!(HostApi { version: 0, ..HostApi::default() }.validate().unwrap_err().code, ErrorCode::IncompatibleApi);
    let repeated = HostApi { version: HOST_API_VERSION, capabilities: vec![metadata.capabilities[0].clone(), metadata.capabilities[0].clone()] };
    assert!(repeated.validate().is_err());
}

#[cfg(feature = "schema")]
#[test]
fn checked_in_json_schemas_match_rust_contract_generation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas");
    for (name, generated) in [
        ("panel-v2.schema.json", schema::descriptor()),
        ("panel-v2-response.schema.json", schema::panel_response()),
        ("host-api-v1.schema.json", schema::host_api()),
    ] {
        let saved: Value = serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap();
        assert_eq!(saved, serde_json::to_value(generated).unwrap(), "regenerate {name} using export_schemas");
        assert_eq!(saved["$schema"], "https://json-schema.org/draft/2020-12/schema");
        let public = root.join("../../../schemas/plugins").join(name);
        if public.exists() {
            let exported: Value = serde_json::from_slice(&std::fs::read(public).unwrap()).unwrap();
            assert_eq!(saved, exported, "public schema {name} differs from independent crate snapshot");
        }
    }
}
