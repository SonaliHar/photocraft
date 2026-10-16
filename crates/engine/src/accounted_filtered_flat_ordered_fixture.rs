//! Compact literal storage evidence only. No numerical/operative-order oracle.
//! Canonical/public order is exactly raw XCF/GIMP list order; document roots
//! below are BOTTOM-first. Per-stage processing/internal formats remain HOLD.
#![allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct Binding {
    pub path: &'static str,
    pub bytes: usize,
    pub sha256: &'static str,
}
#[derive(Clone, Copy, Debug)]
pub struct Case {
    pub source: Binding,
    pub capture: Binding,
    pub precision: u32,
}
pub const ROOT_NAMES: [&str; 4] = ["Own root 0", "Own root 1", "Own root 2", "Own root 3"];
pub const OPERATIONS: [&[&str]; 4] = [
    &[],
    &["gegl:vibrance", "gegl:hue-chroma", "gimp:semi-flatten"],
    &["gimp:semi-flatten", "gegl:vibrance", "gegl:hue-chroma"],
    &["gegl:hue-chroma", "gimp:semi-flatten", "gegl:vibrance"],
];
pub const CASES: [Case; 2] = [
    Case {
        source: Binding {
            path: "temp/root-filtered-flat-stacks-gimp-20261016/author-v2/outputs/f32-ordered-three-filter-stacks.xcf",
            bytes: 8088,
            sha256: "ba7f1eca6867c25687681dd7950fb034488cad1676a6dfd7d6095198ea5a24b0",
        },
        capture: Binding {
            path: "temp/root-filtered-flat-stacks-gimp-20261016/author-v2/outputs/f32-ordered-three-filter-stacks.case.json",
            bytes: 85738,
            sha256: "688abf502db7da7ba8241d7eb69d3b36af0ed08d71d0648d340f36d182df63b4",
        },
        precision: 650,
    },
    Case {
        source: Binding {
            path: "temp/root-filtered-flat-stacks-gimp-20261016/author-v2/outputs/f64-ordered-three-filter-stacks.xcf",
            bytes: 8134,
            sha256: "90d660cce44ed1469ef7a097c80adbd3742e86ae34e4ea6e1150b96e733885e3",
        },
        capture: Binding {
            path: "temp/root-filtered-flat-stacks-gimp-20261016/author-v2/outputs/f64-ordered-three-filter-stacks.case.json",
            bytes: 85826,
            sha256: "65d684c500031e0a4389a3b305d1f906326a6ec6ba51456b3af43e6bc9cf2c9a",
        },
        precision: 750,
    },
];
