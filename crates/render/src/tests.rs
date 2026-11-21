use bayleaf_core::StableId;
use bayleaf_core::math::{Vec2, Vec3};
use bayleaf_scene::Scene;
use serde_json::json;

use crate::{CameraView, FrameOptions, HeadlessGpu, RenderError, extract, pick, scene_camera};

fn object(scene: &mut Scene, component: &str, value: serde_json::Value, position: [f32; 3], rotation: [f32; 3]) -> StableId {
    let id = scene.create_object(None, component).unwrap();
    scene.add_component(id, component).unwrap();
    scene.set_component(id, component, value).unwrap();
    scene.set_component(id, "transform", json!({"position": position, "rotation": rotation, "scale": [1, 1, 1]})).unwrap();
    id
}

fn mesh(scene: &mut Scene, shape: &str, position: [f32; 3]) -> StableId {
    object(scene, "mesh", json!({"shape": shape, "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.5}), position, [0.0; 3])
}

const OPTIONS: FrameOptions = FrameOptions { show_grid: false, selected: None };

#[test]
fn pick_returns_the_nearest_object_under_the_point() {
    let mut scene = Scene::new();
    let far = mesh(&mut scene, "cube", [0.0, 0.0, -5.0]);
    let near = mesh(&mut scene, "sphere", [0.0, 0.0, -2.0]);
    let aside = mesh(&mut scene, "cube", [4.0, 0.0, -2.0]);
    let camera = CameraView::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::new(0.0, 0.0, -1.0), 60.0, 1.0);
    let frame = extract(&scene, camera, OPTIONS);
    assert_eq!(pick(&frame, Vec2::ZERO), Some(near));
    scene.delete_object(near).unwrap();
    let frame = extract(&scene, camera, OPTIONS);
    assert_eq!(pick(&frame, Vec2::ZERO), Some(far));
    assert_eq!(pick(&frame, Vec2::new(0.0, 0.95)), None);
    assert!(scene.contains(aside));
}

#[test]
fn light_direction_follows_the_object_rotation() {
    let mut scene = Scene::new();
    // Rotated −90° about X, the light's −Z axis points straight down.
    object(&mut scene, "directionalLight", json!({"color": [1, 1, 1], "intensity": 2}), [0.0; 3], [-90.0, 0.0, 0.0]);
    let frame = extract(&scene, CameraView::framing(Vec3::ZERO, 1.0, 1.0), OPTIONS);
    assert_eq!(frame.lights.len(), 1);
    assert!(frame.lights[0].direction.dot(Vec3::NEG_Y) > 0.99);
}

#[test]
fn sprites_are_unlit_transparent_and_sized() {
    let mut scene = Scene::new();
    object(&mut scene, "sprite", json!({"color": [1, 0, 0, 1], "size": [3, 1]}), [0.0; 3], [0.0; 3]);
    let frame = extract(&scene, CameraView::framing(Vec3::ZERO, 1.0, 1.0), OPTIONS);
    let sprite = frame.items[0];
    assert!(sprite.unlit && sprite.is_transparent());
    assert!(sprite.model.x_axis.length() > sprite.model.y_axis.length());
}

#[test]
fn the_first_camera_object_is_the_scene_camera() {
    let mut scene = Scene::new();
    assert!(scene_camera(&scene, 1.0).is_none());
    object(&mut scene, "camera", json!({"projection": "perspective", "fov": 60, "size": 5, "near": 0.1, "far": 100}), [0.0, 5.0, 0.0], [0.0; 3]);
    let view = scene_camera(&scene, 1.0).unwrap();
    assert!(view.position.distance(Vec3::new(0.0, 5.0, 0.0)) < 1e-5);
}

/// Renders on the GPU when one is available. Set `BAYLEAF_GPU_REQUIRED=1` to fail instead of skip.
fn gpu() -> Option<HeadlessGpu> {
    match HeadlessGpu::new() {
        Ok(gpu) => Some(gpu),
        Err(RenderError::NoGpu(reason)) if std::env::var_os("BAYLEAF_GPU_REQUIRED").is_none() => {
            eprintln!("skipped: no graphics adapter ({reason})");
            None
        }
        Err(error) => panic!("graphics adapter required: {error}"),
    }
}

fn brightness(p: [u8; 4]) -> u32 {
    u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])
}

#[test]
fn a_lit_cube_is_drawn_over_the_background() {
    let Some(gpu) = gpu() else { return };
    let mut scene = Scene::new();
    mesh(&mut scene, "cube", [0.0, 0.0, 0.0]);
    // Light from straight ahead of the camera.
    object(&mut scene, "directionalLight", json!({"color": [1, 1, 1], "intensity": 3}), [0.0; 3], [0.0; 3]);
    let camera = CameraView::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, 50.0, 1.0);
    let image = gpu.render(&extract(&scene, camera, OPTIONS), 64, 64).unwrap();
    let center = image.pixel(32, 32).unwrap();
    let corner = image.pixel(1, 1).unwrap();
    assert!(brightness(center) > brightness(corner) + 60, "centre {center:?} is not brighter than background {corner:?}");
}

#[test]
fn faces_turned_toward_the_light_are_brighter() {
    let Some(gpu) = gpu() else { return };
    let mut scene = Scene::new();
    mesh(&mut scene, "sphere", [0.0, 0.0, 0.0]);
    // Light travelling toward −X, so the sphere's +X side is lit.
    object(&mut scene, "directionalLight", json!({"color": [1, 1, 1], "intensity": 3}), [0.0; 3], [0.0, 90.0, 0.0]);
    let camera = CameraView::look_at(Vec3::new(0.0, 0.0, 2.5), Vec3::ZERO, 40.0, 1.0);
    let image = gpu.render(&extract(&scene, camera, OPTIONS), 64, 64).unwrap();
    let lit = image.pixel(44, 32).unwrap();
    let dark = image.pixel(20, 32).unwrap();
    assert!(brightness(lit) > brightness(dark), "lit side {lit:?} not brighter than {dark:?}");
}
