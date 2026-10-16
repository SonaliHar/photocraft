# Plugin contract schemas

These Draft 2020-12 JSON Schemas are generated from the independent `photocraft-plugin-api`
crate. Use the `export_schemas` example with its optional `schema` feature; the crate's
all-features contract test checks exact generation consistency.

- `panel-v2.schema.json`: complete API-2 descriptor.
- `panel-v2-response.schema.json`: bounded API-2 response structure.
- `host-api-v1.schema.json`: capability-discovery metadata, independently versioned.

Schemas validate structural shape, not authority. Native validators also enforce cross-field
constraints, byte budgets and owner namespaces. The host checks grants and scoped handles.
See [panel API 2](../../docs/plugin-api-v2.md). Existing panel-1/package-2 readers remain
separate. Package format 3 is an RFC and has no supported schema yet.
