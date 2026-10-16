# Immutable plugin compatibility corpus

These tests freeze the compiled guests and packages that existed on 2026-10-15, before the
universal platform work. Never rebuild an existing baseline from current source to repair a
failure. A changed artifact is a new fixture with its own identity; keep the old one.

`catalog.json` pins size, SHA-256 and BLAKE3 for the three shipped Windows packages and the
original independent Image Gain package. It also pins the small compiled guests committed in
`frozen/`. These are copied directly from the original archives, including their license and
notice files under `licenses/`. `panel-golden.json` records requests and responses from those
old compiled panels. The compiled Channel Gain filter is exercised at 8-bit, 16-bit and float
depths. None of those tests invokes a guest compiler.

The ordinary engine test target is self-contained:

```powershell
cargo test -p photocraft-engine --test plugin_compatibility
```

It verifies immutable bytes, panel responses, filter processing, installation, restart after
removing the downloaded copy, persistent enable state, preview, Apply, one-step Undo/Redo,
cancellation and repair of a corrupted installed module. Its portable test package is a small
synthetic wrapper around the exact old panel and filter; it is not presented as the original
native archive.

## Large native artifacts

Full native archives and runtime/model payloads stay outside Git. Restore the exact files from
a controlled artifact store, preserving their names. No network URL is configured until a
maintainer provides a permitted store. The runner checks the pinned hashes before installation;
a missing or mismatched corpus fails the explicitly selected tests. The default Rust test run
reports these cases as **ignored**, never silently passing them.

`PHOTOCRAFT_PLUGIN_COMPAT_ROOT` may point to a flat directory containing all four named archives,
or to the original PhotoCraft workspace containing the root packages and
`upstream/photocraft/examples/extensions/image-plugin-starter/dist/`.

```powershell
./scripts/plugins/test-compatibility.ps1 -ArtifactRoot C:/path/to/immutable-artifact-store
```

The runner first tests the portable fixtures, then all four packages' install/restart/cancel/
repair lifecycle. It then runs the original Image Gain native worker, AI Upscale's conventional
CPU backend, Background Removal's bundled quantized ONNX model, and Rembg's bundled U²-Net Small
model through the old panel/native transport and Apply/Undo/Redo. The native fixtures advertise
Windows x86-64 and deliberately fail on a different platform. Each test uses temporary stores;
it never attaches the user's installed Extensions folder. No model download action is invoked.
The runner redirects native runtime caches into `target/plugin-compatibility-cache` and restores
the calling shell's cache environment afterward. Direct native processing tests require
`PHOTOCRAFT_PLUGIN_COMPAT_CACHE` and an isolated `LOCALAPPDATA` within it; this is enforced before
starting a worker.

Add `-IncludeVulkan` only on a runner with a functioning Vulkan driver. CPU conformance does not
prove Vulkan compatibility, and an unrun Vulkan test is not a pass. These tests check host
cancellation and result rejection; they do not establish a timed native descendant-cleanup
guarantee. The immutable transport examples in `native-v1.json` record the protocol field set;
the real original workers provide the executable transport check.

Archived packages include licenses with their individual runtime and model conditions. In
particular, some optional AI Upscale models have noncommercial conditions. Hashes and catalog
entries do not grant redistribution rights. Maintain the permitted private test store rather
than copying model payloads into this repository.
