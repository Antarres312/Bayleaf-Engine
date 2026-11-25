//! L3: wgpu renderer for scenes: lit meshes, unlit sprites, the editor ground grid, picking and
//! headless images.
//!
//! [`extract`] turns a scene into a GPU-independent [`Frame`]; [`Renderer`] draws a frame into a
//! [`RenderTarget`]. The editor shows the target's texture; [`HeadlessGpu`] reads it back as pixels.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod extract;
mod gpu;
mod headless;
pub mod material;
mod post;
mod timing;
pub use post::PostSettings;
mod mesh;

pub use bayleaf_assets as assets;
pub use extract::{
    AssetSource, CameraView, DrawItem, Effects, EffectsAllowed, FileAssets, Frame, FrameOptions, LightItem, LightKind, MAX_LIGHTS, NoAssets, Ray,
    ShadowSettings, Skin, Sky, WHOLE_IMAGE, bounds, camera_object, extract, extract_with, game_camera, material_preview, nearest_hit, object_bounds, object_camera,
    pick, post_settings, scene_camera, skin_for,
};
pub use gpu::{COLOR_FORMAT, DEFAULT_SAMPLES, RenderStats, RenderTarget, Renderer};
pub use headless::{HeadlessGpu, Image};
pub use material::{CompiledMaterial, MaterialLibrary};
pub use wgpu;

/// Errors from rendering. Messages are shown to the user.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no usable graphics adapter: {0}")]
    NoGpu(String),
    #[error("graphics error: {0}")]
    Gpu(String),
    #[error("{0}")]
    Io(String),
}

#[cfg(test)]
mod tests;
