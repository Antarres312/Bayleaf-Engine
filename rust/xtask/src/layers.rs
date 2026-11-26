//! Layer map. A crate may depend only on Bayleaf crates in a strictly lower layer.
//! Register every new crate here in the same change that adds it.

pub const LAYERS: &[(&str, u8)] = &[
    ("bayleaf-core", 0),
    // Guest-side SDK for game scripts; the engine never depends on it.
    ("bayleaf-script-sdk", 0),
    ("bayleaf-ecs", 1),
    ("bayleaf-scene", 2),
    ("bayleaf-assets", 2),
    ("bayleaf-graph", 2),
    ("bayleaf-interface", 2),
    ("bayleaf-render", 3),
    ("bayleaf-animation", 3),
    ("bayleaf-script", 4),
    ("bayleaf-gameplay", 4),
    ("bayleaf-streaming", 4),
    ("bayleaf-particles", 4),
    ("bayleaf-physics", 4),
    ("bayleaf-audio", 4),
    ("bayleaf-engine", 5),
    ("bayleaf-ui-egui", 6),
    ("bayleaf-automation", 6),
    ("bayleaf", 7),
    ("bayleaf-cli", 7),
    ("bayleaf-player", 7),
    // Internal performance benchmark (bench/README.md).
    ("bayleaf-bench", 7),
    ("bayleaf-bench-physics", 7),
];

/// UI crates that nothing below layer 6 may use.
pub const UI_ONLY: &[&str] = &["egui", "egui-wgpu", "egui_dock", "eframe", "winit", "rfd"];

pub fn layer_of(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}
