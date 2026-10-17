# PhotoCraft patch to wgpu-hal 30.0.1

Source: https://crates.io/crates/wgpu-hal/30.0.1 (MIT OR Apache-2.0; licenses are alongside this file).

PhotoCraft changes one line in `src/dx12/mod.rs`: the `DXGI_SWAP_CHAIN_DESC1`
created for a surface uses `DXGI_SCALING_NONE` instead of
`DXGI_SCALING_STRETCH`. No other upstream source is changed. The Cargo
`[patch.crates-io]` entry in the workspace manifest applies this to the
existing wgpu dependency graph.

On the affected Windows system, `DXGI_SCALING_STRETCH` resamples the restored
window's UI despite a sharp framebuffer. `DXGI_SCALING_NONE` prevents that
extra resampling. The same root cause was measured in FigCraft issue #778.

Remove this vendor copy when a released wgpu-hal includes the fix and
PhotoCraft upgrades to it.
