use bayleaf_core::StableId;
use bayleaf_core::math::{Vec2, Vec3};
use bayleaf_scene::Scene;
use serde_json::json;

use std::sync::Arc;

use bayleaf_graph::{Graph, Link, PinRef};

use crate::material::{CATALOG, compile};
use crate::{CameraView, FrameOptions, HeadlessGpu, RenderError, extract, extract_with, pick, scene_camera};

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

const OPTIONS: FrameOptions = FrameOptions { show_grid: false, selected: None, time: 0.0 };

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
    let sprite = &frame.items[0];
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

fn link(graph: &mut Graph, from: u32, from_pin: &str, to: u32, to_pin: &str) {
    graph.connect(&CATALOG, Link { from: PinRef::new(from, from_pin), to: PinRef::new(to, to_pin) }).unwrap();
}

/// A material that uses every node kind, wired into the output.
fn every_node_material() -> Graph {
    let mut g = Graph::new(&CATALOG);
    let out = g.nodes[0].id;
    let mut previous: Option<u32> = None;
    for def in CATALOG.nodes.iter().filter(|d| d.kind != "output") {
        let id = g.add_node(&CATALOG, def.kind, [0.0, 0.0]).unwrap();
        // Feed the previous node into the first input that accepts it, then chain onwards.
        if let Some(prev) = previous {
            let prev_def = CATALOG.get(&g.node(prev).unwrap().kind).unwrap();
            if let Some(from) = prev_def.outputs.first() {
                // Only links that are always valid: numbers anywhere, otherwise equal types.
                let fits = |p: &&bayleaf_graph::PinDef| from.ty == bayleaf_graph::PinType::Float || (from.ty == p.ty && from.ty != bayleaf_graph::PinType::Any);
                if let Some(to) = def.inputs.iter().find(fits) {
                    let _ = g.connect(&CATALOG, Link { from: PinRef::new(prev, from.name), to: PinRef::new(id, to.name) });
                }
            }
        }
        previous = Some(id);
        // Every node also adds into the emission, so each one is compiled.
        if let Some(from) = def.outputs.first() {
            let make = g.add_node(&CATALOG, "vector", [0.0, 0.0]).unwrap();
            if from.ty == bayleaf_graph::PinType::Float {
                link(&mut g, id, from.name, make, "x");
            } else {
                // Through Split Vector, so vector and colour results fit too.
                let split = g.add_node(&CATALOG, "split", [0.0, 0.0]).unwrap();
                link(&mut g, id, from.name, split, "vector");
                link(&mut g, split, "x", make, "y");
            }
            let sum = g.add_node(&CATALOG, "add", [0.0, 0.0]).unwrap();
            link(&mut g, make, "vector", sum, "b");
            let existing = g.link_into(out, "emissive").map(|l| l.from.clone());
            if let Some(existing) = existing {
                link(&mut g, existing.node, &existing.pin, sum, "a");
            }
            link(&mut g, sum, "result", out, "emissive");
        }
    }
    g
}

#[test]
fn a_material_using_every_node_compiles() {
    let graph = every_node_material();
    let compiled = compile(&graph).unwrap_or_else(|p| panic!("{p:?}"));
    for def in CATALOG.nodes.iter().filter(|d| d.kind != "output") {
        assert!(graph.nodes.iter().any(|n| n.kind == def.kind), "{} is not in the test graph", def.kind);
    }
    assert!(compiled.animated, "the graph uses Time");
}

#[test]
fn a_vector_into_a_number_input_is_a_problem_on_that_node() {
    let mut g = Graph::new(&CATALOG);
    let out = g.nodes[0].id;
    let position = g.add_node(&CATALOG, "position", [0.0, 0.0]).unwrap();
    let add = g.add_node(&CATALOG, "add", [0.0, 0.0]).unwrap();
    link(&mut g, position, "position", add, "a");
    link(&mut g, add, "result", out, "roughness");
    let problems = compile(&g).unwrap_err();
    assert_eq!(problems[0].node, Some(out));
    assert!(problems[0].message.contains("Split Vector"), "{problems:?}");
}

#[test]
fn materials_change_the_drawn_colour_and_every_node_is_valid_on_the_gpu() {
    let Some(gpu) = gpu() else { return };
    let mut scene = Scene::new();
    mesh(&mut scene, "cube", [0.0, 0.0, 0.0]);
    object(&mut scene, "directionalLight", json!({"color": [1, 1, 1], "intensity": 3}), [0.0; 3], [0.0; 3]);
    let camera = CameraView::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, 50.0, 1.0);

    let mut red = Graph::new(&CATALOG);
    let out = red.nodes[0].id;
    red.set_value(&CATALOG, out, "baseColor", json!([0.0, 0.0, 0.0])).unwrap();
    red.set_value(&CATALOG, out, "emissive", json!([1.0, 0.0, 0.0])).unwrap();
    let red = Arc::new(compile(&red).unwrap());
    let frame = extract_with(&scene, camera, OPTIONS, &mut |_| Some(Arc::clone(&red)));
    // The mesh has no material path, so the resolver is not asked.
    assert!(frame.items[0].material.is_none());
    let id = scene.world_matrices()[0].0;
    scene
        .set_component(id, "mesh", json!({"shape": "cube", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.5, "material": "red.bmaterial"}))
        .unwrap();
    let frame = extract_with(&scene, camera, OPTIONS, &mut |path| (path == "red.bmaterial").then(|| Arc::clone(&red)));
    let image = gpu.render(&frame, 64, 64).unwrap();
    let [r, g, b, _] = image.pixel(32, 32).unwrap();
    // The light adds a white highlight; the emission makes red dominate.
    assert!(r > 200 && r > g + 100 && r > b + 100, "expected red, got {:?}", [r, g, b]);

    let every = Arc::new(compile(&every_node_material()).unwrap());
    let frame = extract_with(&scene, camera, OPTIONS, &mut |_| Some(Arc::clone(&every)));
    gpu.render(&frame, 32, 32).unwrap_or_else(|e| panic!("the GPU refused the generated shader: {e}\n{}", every.surface));
}

/// A wide floor seen from above at an angle, with the given lights.
fn floor_scene(lights: &[(&str, serde_json::Value, [f32; 3], [f32; 3])]) -> Scene {
    let mut scene = Scene::new();
    let floor = object(&mut scene, "mesh", json!({"shape": "plane", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.9}), [0.0; 3], [0.0; 3]);
    scene.set_component(floor, "transform", json!({"position": [0, 0, 0], "rotation": [0, 0, 0], "scale": [20, 1, 20]})).unwrap();
    for (component, value, position, rotation) in lights {
        object(&mut scene, component, value.clone(), *position, *rotation);
    }
    scene
}

fn top_down() -> CameraView {
    CameraView::look_at(Vec3::new(0.0, 12.0, 0.01), Vec3::ZERO, 60.0, 1.0)
}

#[test]
fn point_and_spot_lights_fade_with_distance_and_outside_the_cone() {
    let Some(gpu) = gpu() else { return };
    // A point light low over the left half of the floor.
    let scene = floor_scene(&[("pointLight", json!({"color": [1, 1, 1], "intensity": 20, "range": 6}), [-3.0, 1.0, 0.0], [0.0; 3])]);
    let frame = extract(&scene, top_down(), OPTIONS);
    let (near, far) = (camera_pixel(&frame.camera, Vec3::new(-3.0, 0.0, 0.0), 64), camera_pixel(&frame.camera, Vec3::new(4.5, 0.0, 0.0), 64));
    let image = gpu.render(&frame, 64, 64).unwrap();
    let (near, far) = (image.pixel(near.0, near.1).unwrap(), image.pixel(far.0, far.1).unwrap());
    assert!(brightness(near) > brightness(far) + 60, "near {near:?} far {far:?}");

    // A spot light straight down at the centre, 30° wide.
    let scene = floor_scene(&[(
        "spotLight",
        json!({"color": [1, 1, 1], "intensity": 40, "range": 10, "angle": 30, "softness": 0.2}),
        [0.0, 3.0, 0.0],
        [-90.0, 0.0, 0.0],
    )]);
    let frame = extract(&scene, top_down(), OPTIONS);
    let outside = camera_pixel(&frame.camera, Vec3::new(3.5, 0.0, 0.0), 64);
    let image = gpu.render(&frame, 64, 64).unwrap();
    let inside = image.pixel(32, 32).unwrap();
    let outside = image.pixel(outside.0, outside.1).unwrap();
    assert!(brightness(inside) > brightness(outside) + 60, "inside {inside:?} outside {outside:?}");
}

#[test]
fn a_shape_casts_a_shadow_from_the_sun_unless_shadows_are_off() {
    let Some(gpu) = gpu() else { return };
    let sun = |shadows: bool| ("directionalLight", json!({"color": [1, 1, 1], "intensity": 1.5, "shadows": shadows}), [0.0, 5.0, 0.0], [-90.0, 0.0, 0.0]);
    let render = |shadows: bool| {
        let mut scene = floor_scene(&[sun(shadows)]);
        object(&mut scene, "mesh", json!({"shape": "cube", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.9}), [3.0, 2.0, 0.0], [0.0; 3]);
        // From the side, so the floor under the cube is visible.
        let camera = CameraView::look_at(Vec3::new(3.0, 6.0, 9.0), Vec3::new(3.0, 0.0, 0.0), 50.0, 1.0);
        let frame = extract(&scene, camera, OPTIONS);
        let under = camera_pixel(&frame.camera, Vec3::new(3.0, 0.0, 0.3), 64);
        let away = camera_pixel(&frame.camera, Vec3::new(-1.5, 0.0, 0.3), 64);
        let image = gpu.render(&frame, 64, 64).unwrap();
        (image.pixel(under.0, under.1).unwrap(), image.pixel(away.0, away.1).unwrap())
    };
    let (under, away) = render(true);
    assert!(brightness(away) > brightness(under) + 80, "the floor under the cube is in shadow: under {under:?}, away {away:?}");
    let (under, away) = render(false);
    assert!(brightness(away).abs_diff(brightness(under)) < 30, "no shadow when shadows are off: under {under:?}, away {away:?}");
}

/// The pixel a world point lands on in a square image of `size`.
fn camera_pixel(camera: &CameraView, point: Vec3, size: u32) -> (u32, u32) {
    let ndc = camera.view_proj().project_point3(point);
    let x = ((ndc.x * 0.5 + 0.5) * size as f32) as u32;
    let y = ((0.5 - ndc.y * 0.5) * size as f32) as u32;
    (x.min(size - 1), y.min(size - 1))
}

#[test]
fn a_surface_on_the_ground_plane_hides_the_grid_completely() {
    let Some(gpu) = gpu() else { return };
    let scene = floor_scene(&[("directionalLight", json!({"color": [1, 1, 1], "intensity": 3, "shadows": false}), [0.0; 3], [-60.0, 20.0, 0.0])]);
    // A low, grazing view, where depth fighting shows most.
    let camera = CameraView::look_at(Vec3::new(0.0, 0.6, 8.0), Vec3::new(0.0, 0.0, -4.0), 50.0, 1.0);
    let with_grid = gpu.render(&extract(&scene, camera, FrameOptions { show_grid: true, ..OPTIONS }), 96, 96).unwrap();
    let without = gpu.render(&extract(&scene, camera, OPTIONS), 96, 96).unwrap();
    let differing =
        with_grid.pixels.chunks(4).zip(without.pixels.chunks(4)).skip(96 * 60).filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 6)).count();
    assert_eq!(differing, 0, "{differing} floor pixels show the grid");
}
