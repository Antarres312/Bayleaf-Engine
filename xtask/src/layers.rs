//! Layer map. A crate may depend only on Bayleaf crates in a strictly lower layer.
//! Register every new crate here in the same change that adds it.

pub const LAYERS: &[(&str, u8)] = &[
    ("bayleaf-core", 0),
    ("bayleaf-ecs", 1),
    ("bayleaf-scene", 2),
    ("bayleaf-assets", 2),
    ("bayleaf-graph", 2),
    ("bayleaf-render", 3),
    ("bayleaf-script", 4),
    ("bayleaf-gameplay", 4),
    ("bayleaf-streaming", 4),
    ("bayleaf-engine", 5),
    ("bayleaf-ui-egui", 6),
    ("bayleaf-automation", 6),
    ("bayleaf", 7),
    ("bayleaf-cli", 7),
];

/// UI crates that nothing below layer 6 may use.
pub const UI_ONLY: &[&str] = &["egui", "eframe", "winit", "rfd"];

pub fn layer_of(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}
