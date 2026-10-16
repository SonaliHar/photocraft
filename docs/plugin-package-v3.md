# Plugin package format 3 RFC

**Status:** design proposal, 2026-10-15; not an accepted installation format. Current packages
continue to use [package format 2](plugin-native-packages.md). Panel API 2 is independently
versioned and does not require package format 3.

## Motivation and ownership

Format 2 derives identity and UI metadata from its embedded WASM manifest. A future format 3
will expose verified descriptors without activating WASM during ordinary discovery, and will
describe multiple target payloads, activation and integrity using one authoring source.

Every package remains self-contained: its algorithms, executable modules, native dependencies,
assets, licenses, private settings and release cadence belong to that plugin. The descriptor
must not require another installed plugin, sibling checkout, shared model catalog, service
registration or an install script. Shared storage deduplication, if introduced later, must not
change ownership or removal safety.

## Proposed descriptor

The proposed `package.json` contains:

| Field | Purpose |
|---|---|
| `packageVersion: 3` | Selects this package reader explicitly |
| `descriptor` | Complete panel-2 identity, requested capabilities and contributions |
| `hostApi` | Supported host metadata major/range; independent from editor release version |
| `activation` | Bounded command/view/event triggers; no automatic startup execution |
| `payloads` | Declared target, entry modules and explicitly versioned transport |
| `integrity` | Canonical index digest identifying exactly the protected package bytes |
| `extensions` | Bounded namespaced optional metadata; no implicit authority |

The embedded guest identity/contract digest must agree with the generated installation
descriptor. Identity conflicts, unsupported required features and target mismatches fail before
publishing an installation. Existing format-2 package metadata stays unchanged. API-1/filter-1/
worker-1 adapters stay available; a version number alone never selects a weaker trust mode.

The SDK should generate the installation descriptor, embedded manifest, schema bindings and
digest from one source. PhotoCraft persists its own validated descriptor as installed metadata;
an editable loose sidecar is never trusted as installation authority.

## Validation and installation rules

Before enabling format 3, publish a complete JSON Schema and malformed-input corpus, define
canonical digest bytes and verify the descriptor against the exact archive. Reject unknown
required features, duplicate and cross-owner IDs, undeclared payload members, traversal,
symlinks, case aliases and conflicting paths. Stage and verify before atomically publishing;
retain the previous generation if installation fails. Running generations keep leases until
completion, independently of which generation is current.

Future signatures cover identity, permissions and every payload entry, excluding only their
own envelope. Digests detect altered bytes; they do not identify a publisher. Native execution
profiles and permission expansion require explicit host policy. Ordinary package installation
must not start arbitrary native code or download missing dependencies.

Large archives need streaming extraction with explicit compressed/expanded byte, entry-count,
per-entry, disk and time budgets. ZIP64 needs dedicated implementation and adversarial tests.
The current format-2 limits must not simply be removed or silently changed.

## Compatibility and release gate

Format 3 stays disabled until schema validation, descriptor/digest matching, target selection,
permission review, interrupted install/rollback and generation leases pass on supported hosts.
Its release must publish exact limits and conformance results. Existing format-2 immutable
packages must still install, restart, process, cancel, Apply and repair unchanged. Standalone
acceptance projects must build with the editor and sibling plugin sources absent.

This RFC intentionally does not promise a signature infrastructure, marketplace, worker-2
transport, lazy startup, portable native sandbox or new archive limits in the first slice.
Those features require their own completed implementations and tests under the
[platform plan](plugin-platform-plan.md).
