# Plugin contract schemas

These Draft 2020-12 JSON Schemas are generated from the independent `photocraft-plugin-api`
crate. Use the `export_schemas` example with its optional `schema` feature; the crate's
all-features contract test checks exact generation consistency.

- `panel-v2.schema.json`: complete API-2 descriptor.
- `panel-v2-response.schema.json`: bounded API-2 response structure.
- `host-api-v1.schema.json`: capability-discovery metadata, independently versioned.
- `panel-v2-event.schema.json`: typed host event and correlated broker replies.
- `authoring-v1.schema.json`: single source descriptor and package authoring input.
- `package-v2.schema.json`: current package metadata; native target compatibility is checked by the host.
- `layers-read-v1.schema.json`, `layers-edit-v1.schema.json`: current document broker requests.
- `layers-snapshot-v1.schema.json`, `layers-edited-v1.schema.json`: document broker success results.
- `worker-v2.schema.json`: explicit private persistent-worker declaration and resource estimates.
- `worker-v2-host-frame.schema.json`, `worker-v2-guest-frame.schema.json`: bounded control transport frames.

Schemas validate structural shape, not authority. Native validators also enforce cross-field
constraints, byte budgets and owner namespaces. The host checks grants and scoped handles.
See [panel API 2](../../docs/plugin-api-v2.md) and the [standalone SDK](../../docs/plugin-sdk.md).
Package format 2 is current; the panel-1 reader remains a temporary compatibility path for
our own existing binaries. The [future package proposal](../../docs/plugin-package-v3.md)
has no accepted runtime format/schema. These labels version distinct wire formats, not API milestones.
