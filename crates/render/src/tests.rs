use bayleaf_core::StableId;
use bayleaf_core::math::{DVec3, Vec2, Vec3, Vec4};
use bayleaf_scene::Scene;
use serde_json::json;

use std::sync::Arc;

use bayleaf_graph::{Graph, Link, PinRef};

use crate::Effects;
use crate::material::{CATALOG, compile};
use crate::{CameraView, FrameOptions, HeadlessGpu, RenderError, ShadowSettings, extract, extract_with, pick, scene_camera};

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

const OPTIONS: FrameOptions = FrameOptions { show_grid: false, selected: None, time: 0.0, shadows: ShadowSettings::DEFAULT, effects: Effects::DEFAULT };

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
    let frame = extract(&scene, CameraView::framing(bayleaf_core::math::DVec3::ZERO, 1.0, 1.0), OPTIONS);
    assert_eq!(frame.lights.len(), 1);
    assert!(frame.lights[0].direction.dot(Vec3::NEG_Y) > 0.99);
}

#[test]
fn sprites_are_unlit_transparent_and_sized() {
    let mut scene = Scene::new();
    object(&mut scene, "sprite", json!({"color": [1, 0, 0, 1], "size": [3, 1]}), [0.0; 3], [0.0; 3]);
    let frame = extract(&scene, CameraView::framing(bayleaf_core::math::DVec3::ZERO, 1.0, 1.0), OPTIONS);
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
    assert!(view.eye().distance(bayleaf_core::math::DVec3::new(0.0, 5.0, 0.0)) < 1e-5);
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

/// Gives one material for every material path, or only for the one path named.
struct Materials(Option<&'static str>, Arc<crate::CompiledMaterial>);

impl crate::AssetSource for Materials {
    fn material(&mut self, path: &str) -> Option<Arc<crate::CompiledMaterial>> {
        self.0.is_none_or(|only| only == path).then(|| Arc::clone(&self.1))
    }
    fn image(&mut self, _: &str) -> Option<Arc<bayleaf_assets::Image>> {
        None
    }
    fn model(&mut self, _: &str) -> Option<Arc<bayleaf_assets::MeshData>> {
        None
    }
}

fn link(graph: &mut Graph, from: u32, from_pin: &str, to: u32, to_pin: &str) {
    graph.connect(&CATALOG, Link { from: PinRef::new(from, from_pin), to: PinRef::new(to, to_pin) }).unwrap();
}

/// A material that uses every node kind, wired into the output.
fn every_node_material() -> Graph {
    let mut g = Graph::new(&CATALOG);
    let out = g.nodes[0].id;
    // The outline option too, so its pipeline is built and checked.
    g.set_value(&CATALOG, out, "outlineWidth", json!(0.02)).unwrap();
    let mut previous: Option<u32> = None;
    for def in CATALOG.nodes.iter().filter(|d| d.kind != "output" && d.kind != "customLighting") {
        let id = g.add_node(&CATALOG, def.kind, [0.0, 0.0]).unwrap();
        if def.kind == "texture" {
            g.set_value(&CATALOG, id, "image", json!("any.png")).unwrap();
        }
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
    // Custom Lighting Output is the other output: a material has one, so it has its own test.
    for def in CATALOG.nodes.iter().filter(|d| d.kind != "output" && d.kind != "customLighting") {
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
    let frame = extract_with(&scene, camera, OPTIONS, &mut Materials(None, Arc::clone(&red)));
    // The mesh has no material path, so the resolver is not asked.
    assert!(frame.items[0].material.is_none());
    let id = scene.world_matrices()[0].0;
    scene
        .set_component(id, "mesh", json!({"shape": "cube", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.5, "material": "red.bmaterial"}))
        .unwrap();
    let frame = extract_with(&scene, camera, OPTIONS, &mut Materials(Some("red.bmaterial"), Arc::clone(&red)));
    let image = gpu.render(&frame, 64, 64).unwrap();
    let [r, g, b, _] = image.pixel(32, 32).unwrap();
    // The light adds a white highlight; the emission makes red dominate.
    assert!(r > 200 && r > g + 100 && r > b + 100, "expected red, got {:?}", [r, g, b]);

    // The material preview shows only the sphere, over a transparent background.
    let preview = gpu.render(&crate::material_preview(Arc::clone(&red), Vec::new(), 1.0, 0.0), 64, 64).unwrap();
    let [r, g, b, a] = preview.pixel(32, 32).unwrap();
    assert!(r > g + 100 && r > b + 100 && a == 255, "the sphere shows the material: {:?}", [r, g, b, a]);
    for corner in [(1, 1), (62, 1), (1, 62), (62, 62)] {
        assert_eq!(preview.pixel(corner.0, corner.1).unwrap()[3], 0, "the corner {corner:?} is transparent");
    }

    let every = Arc::new(compile(&every_node_material()).unwrap());
    let frame = extract_with(&scene, camera, OPTIONS, &mut Materials(None, Arc::clone(&every)));
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

/// A folder with `stripes.png` (left half red, right half blue) and `wall.gltf` (a 2 x 2 square
/// facing +Z, made of two triangles).
fn asset_folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bayleaf-render-assets-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
    }
    std::fs::write(dir.join("stripes.png"), png_bytes).unwrap();

    let positions: [[f32; 3]; 4] = [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [1.0, 1.0, 0.0], [-1.0, 1.0, 0.0]];
    let mut bin: Vec<u8> = positions.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
    for i in [0u16, 1, 2, 0, 2, 3] {
        bin.extend(i.to_le_bytes());
    }
    std::fs::write(dir.join("wall.bin"), &bin).unwrap();
    let gltf = json!({
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "buffers": [{"uri": "wall.bin", "byteLength": bin.len()}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 48}, {"buffer": 0, "byteOffset": 48, "byteLength": 12}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-1, -1, 0], "max": [1, 1, 0]},
            {"bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR"}
        ]
    });
    std::fs::write(dir.join("wall.gltf"), gltf.to_string()).unwrap();
    dir
}

fn render_with_files(gpu: &HeadlessGpu, scene: &Scene, dir: &std::path::Path, materials: &mut crate::MaterialLibrary, camera: CameraView) -> crate::Image {
    let mut cache = bayleaf_assets::AssetCache::new();
    let allow = |p: &std::path::Path| Ok(p.to_path_buf());
    let mut assets = crate::FileAssets { base: dir.to_path_buf(), materials, cache: &mut cache, allow: &allow, warnings: Vec::new() };
    let frame = extract_with(scene, camera, OPTIONS, &mut assets);
    assert!(assets.warnings.is_empty(), "{:?}", assets.warnings);
    gpu.render(&frame, 64, 64).unwrap()
}

#[test]
fn sprites_show_their_image_and_materials_read_images() {
    let Some(gpu) = gpu() else { return };
    let dir = asset_folder("images");
    let camera = CameraView::look_at(Vec3::new(0.0, 0.0, 2.0), Vec3::ZERO, 50.0, 1.0);
    let mut scene = Scene::new();
    object(&mut scene, "sprite", json!({"color": [1, 1, 1, 1], "size": [2, 2], "image": "stripes.png"}), [0.0; 3], [0.0; 3]);
    let image = render_with_files(&gpu, &scene, &dir, &mut crate::MaterialLibrary::new(), camera);
    let [lr, _, lb, _] = image.pixel(14, 32).unwrap();
    let [rr, _, rb, _] = image.pixel(50, 32).unwrap();
    assert!(lr > 200 && lb < 60, "left half red: {lr} {lb}");
    assert!(rb > 200 && rr < 60, "right half blue: {rr} {rb}");

    // A material that shows the image as emission on a quad.
    let mut graph = bayleaf_graph::Graph::new(&CATALOG);
    let out = graph.nodes[0].id;
    graph.set_value(&CATALOG, out, "baseColor", json!([0, 0, 0])).unwrap();
    let texture = graph.add_node(&CATALOG, "texture", [0.0, 0.0]).unwrap();
    graph.set_value(&CATALOG, texture, "image", json!("stripes.png")).unwrap();
    link(&mut graph, texture, "color", out, "emissive");
    std::fs::write(dir.join("striped.bmaterial"), graph.to_json()).unwrap();
    let mut scene = Scene::new();
    object(
        &mut scene,
        "mesh",
        json!({"shape": "quad", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 1, "material": "striped.bmaterial"}),
        [0.0; 3],
        [0.0; 3],
    );
    scene.set_component(scene.world_matrices()[0].0, "transform", json!({"position": [0, 0, 0], "rotation": [0, 0, 0], "scale": [2, 2, 1]})).unwrap();
    let image = render_with_files(&gpu, &scene, &dir, &mut crate::MaterialLibrary::new(), camera);
    let [lr, _, lb, _] = image.pixel(14, 32).unwrap();
    let [rr, _, rb, _] = image.pixel(50, 32).unwrap();
    assert!(lr > rr + 100 && rb > lb + 100, "the image shows through the material: left {lr},{lb} right {rr},{rb}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn meshes_draw_their_model_and_are_picked_by_its_box() {
    let Some(gpu) = gpu() else { return };
    let dir = asset_folder("model");
    let mut scene = Scene::new();
    let wall = object(
        &mut scene,
        "mesh",
        json!({"shape": "cube", "color": [0.9, 0.9, 0.9, 1], "metallic": 0, "roughness": 0.9, "model": "wall.gltf"}),
        [0.0; 3],
        [0.0; 3],
    );
    object(&mut scene, "directionalLight", json!({"color": [1, 1, 1], "intensity": 3, "shadows": false}), [0.0; 3], [0.0; 3]);
    let camera = CameraView::look_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, 50.0, 1.0);
    let image = render_with_files(&gpu, &scene, &dir, &mut crate::MaterialLibrary::new(), camera);
    // The square is 2 wide, wider than the unit cube the shape would draw.
    let edge = image.pixel(32 + 12, 32).unwrap();
    let corner = image.pixel(1, 1).unwrap();
    assert!(brightness(edge) > brightness(corner) + 100, "the model reaches past the unit cube: {edge:?}");

    let mut cache = bayleaf_assets::AssetCache::new();
    let allow = |p: &std::path::Path| Ok(p.to_path_buf());
    let mut materials = crate::MaterialLibrary::new();
    let mut assets = crate::FileAssets { base: dir.clone(), materials: &mut materials, cache: &mut cache, allow: &allow, warnings: Vec::new() };
    let frame = extract_with(&scene, camera, OPTIONS, &mut assets);
    assert_eq!(pick(&frame, Vec2::new(0.45, 0.0)), Some(wall), "picked by the model's box, past the unit cube");

    // A model's own material tints it.
    let mut gltf: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("wall.gltf")).unwrap()).unwrap();
    gltf["meshes"][0]["primitives"][0]["material"] = json!(0);
    gltf["materials"] = json!([{ "pbrMetallicRoughness": { "baseColorFactor": [0.05, 1.0, 0.05, 1.0], "metallicFactor": 0.0 } }]);
    std::fs::write(dir.join("wall.gltf"), gltf.to_string()).unwrap();
    let image = render_with_files(&gpu, &scene, &dir, &mut crate::MaterialLibrary::new(), camera);
    let [r, g, b, _] = image.pixel(32 + 12, 32).unwrap();
    assert!(g > r + 60 && g > b + 60, "the model's base colour is green: {:?}", [r, g, b]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_environment_draws_a_sky_above_the_ground_and_lights_shapes_from_it() {
    let Some(gpu) = gpu() else { return };
    let mut scene = Scene::new();
    mesh(&mut scene, "sphere", [0.0, 0.0, 0.0]);
    let env = json!({"skyColor": [0.1, 0.3, 0.9], "horizonColor": [0.7, 0.75, 0.8], "groundColor": [0.4, 0.25, 0.1], "ambient": 2.0, "showSky": true});
    let holder = object(&mut scene, "environment", env.clone(), [0.0; 3], [0.0; 3]);
    let camera = CameraView::look_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, 70.0, 1.0);
    let image = gpu.render(&extract(&scene, camera, OPTIONS), 64, 64).unwrap();
    let (top, bottom) = (image.pixel(32, 1).unwrap(), image.pixel(32, 62).unwrap());
    assert!(top[2] > top[0] + 40, "the sky above is blue: {top:?}");
    assert!(bottom[0] > bottom[2] + 40, "below the horizon is the ground colour: {bottom:?}");
    // Shapes take the sky's light from above and the ground's from below.
    let (upper, lower) = (image.pixel(32, 20).unwrap(), image.pixel(32, 44).unwrap());
    assert!(i32::from(upper[2]) - i32::from(upper[0]) > i32::from(lower[2]) - i32::from(lower[0]), "top {upper:?} is bluer than bottom {lower:?}");

    let mut hidden = env;
    hidden["showSky"] = json!(false);
    scene.set_component(holder, "environment", hidden).unwrap();
    let image = gpu.render(&extract(&scene, camera, OPTIONS), 64, 64).unwrap();
    assert_eq!(image.pixel(32, 1), image.pixel(32, 62), "without the sky the background is one colour");
}

#[test]
fn a_spot_light_casts_a_shadow_when_asked() {
    let Some(gpu) = gpu() else { return };
    let render = |shadows: bool| {
        let spot = json!({"color": [1, 1, 1], "intensity": 60, "range": 12, "angle": 80, "softness": 0.1, "shadows": shadows});
        let mut scene = floor_scene(&[("spotLight", spot, [0.0, 6.0, 0.0], [-90.0, 0.0, 0.0])]);
        object(&mut scene, "mesh", json!({"shape": "cube", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.9}), [0.0, 3.0, 0.0], [0.0; 3]);
        let camera = CameraView::look_at(Vec3::new(0.0, 7.0, 7.0), Vec3::ZERO, 60.0, 1.0);
        let frame = extract(&scene, camera, OPTIONS);
        // Straight under the cube, and beside its shadow but still in the cone.
        let under = camera_pixel(&frame.camera, Vec3::new(0.0, 0.0, 0.0), 64);
        let beside = camera_pixel(&frame.camera, Vec3::new(2.2, 0.0, 0.0), 64);
        let image = gpu.render(&frame, 64, 64).unwrap();
        (image.pixel(under.0, under.1).unwrap(), image.pixel(beside.0, beside.1).unwrap())
    };
    let (under, beside) = render(true);
    assert!(brightness(beside) > brightness(under) + 60, "the floor under the cube is in shadow: under {under:?}, beside {beside:?}");
    let (under, _) = render(false);
    assert!(brightness(under) > brightness(render(true).0) + 60, "without shadows the floor under the cube is lit");
}

#[test]
fn a_sun_shadow_ends_at_the_shadow_distance() {
    let Some(gpu) = gpu() else { return };
    let sun = ("directionalLight", json!({"color": [1, 1, 1], "intensity": 1.5, "shadows": true}), [0.0, 5.0, 0.0], [-90.0, 0.0, 0.0]);
    // A cube far from the camera: about 16 units away.
    let render = |reach: f32| {
        let mut scene = floor_scene(std::slice::from_ref(&sun));
        object(&mut scene, "mesh", json!({"shape": "cube", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.9}), [0.0, 2.0, -5.0], [0.0; 3]);
        let camera = CameraView::look_at(Vec3::new(0.0, 6.0, 9.0), Vec3::new(0.0, 0.0, -3.0), 50.0, 1.0);
        let options = FrameOptions { shadows: ShadowSettings { reach, ..ShadowSettings::DEFAULT }, ..OPTIONS };
        let frame = extract(&scene, camera, options);
        let under = camera_pixel(&frame.camera, Vec3::new(0.0, 0.0, -4.7), 64);
        brightness(gpu.render(&frame, 64, 64).unwrap().pixel(under.0, under.1).unwrap())
    };
    let (whole, near) = (render(0.0), render(5.0));
    assert!(near > whole + 80, "past the distance the floor under the cube is lit: whole scene {whole}, 5 units {near}");
    assert!(render(40.0) + 80 < near, "within the distance the shadow stays");
}

#[test]
fn as_many_lights_cast_shadows_as_the_settings_allow() {
    let Some(gpu) = gpu() else { return };
    // Two spot lights far apart, each over a cube of its own.
    let render = |lights: u32| {
        let spot = json!({"color": [1, 1, 1], "intensity": 60, "range": 12, "angle": 80, "softness": 0.1, "shadows": true});
        let mut scene =
            floor_scene(&[("spotLight", spot.clone(), [-5.0, 6.0, 0.0], [-90.0, 0.0, 0.0]), ("spotLight", spot, [5.0, 6.0, 0.0], [-90.0, 0.0, 0.0])]);
        for x in [-5.0, 5.0] {
            object(&mut scene, "mesh", json!({"shape": "cube", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.9}), [x, 3.0, 0.0], [0.0; 3]);
        }
        let camera = CameraView::look_at(Vec3::new(0.0, 12.0, 12.0), Vec3::ZERO, 70.0, 1.0);
        let options = FrameOptions { shadows: ShadowSettings { lights, ..ShadowSettings::DEFAULT }, ..OPTIONS };
        let frame = extract(&scene, camera, options);
        let image = gpu.render(&frame, 96, 96).unwrap();
        let at = |x: f32| {
            let (px, py) = camera_pixel(&frame.camera, Vec3::new(x, 0.0, 0.0), 96);
            brightness(image.pixel(px, py).unwrap())
        };
        (at(-5.0), at(5.0))
    };
    let (one_left, one_right) = render(1);
    let (two_left, two_right) = render(2);
    // With one light casting shadows, one cube leaves the floor under it lit; with two, neither does.
    let lit_with_one = one_left.max(one_right);
    assert!(lit_with_one > two_left.max(two_right) + 60, "one: ({one_left}, {one_right}), two: ({two_left}, {two_right})");
    assert!(lit_with_one > one_left.min(one_right) + 60, "with one, the other cube still casts its shadow: ({one_left}, {one_right})");
}

#[test]
fn fog_hides_far_shapes_in_the_horizon_colour() {
    let Some(gpu) = gpu() else { return };
    let render = |fog: f32| {
        let mut scene = Scene::new();
        mesh(&mut scene, "cube", [0.0, 0.0, -30.0]);
        let env = json!({"skyColor": [0, 0, 0], "horizonColor": [0.9, 0.1, 0.1], "groundColor": [0, 0, 0], "ambient": 1, "showSky": false, "fogDistance": fog});
        object(&mut scene, "environment", env, [0.0; 3], [0.0; 3]);
        let camera = CameraView::look_at(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), 5.0, 1.0);
        gpu.render(&extract(&scene, camera, OPTIONS), 32, 32).unwrap().pixel(16, 16).unwrap()
    };
    let (clear, foggy) = (render(0.0), render(3.0));
    let red = |p: [u8; 4]| i32::from(p[0]) - i32::from(p[1]);
    assert!(red(foggy) > red(clear) + 80, "the far cube takes the fog colour: clear {clear:?}, foggy {foggy:?}");
}
/// A 2 x 2 square facing +Z with texture coordinates (v down, as in glTF) and a one-pixel normal
/// map of `normal` (RGB), in a folder of its own.
fn normal_mapped_square(name: &str, normal: [u8; 3]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bayleaf-render-normal-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&normal).unwrap();
    }
    std::fs::write(dir.join("normal.png"), png_bytes).unwrap();
    let positions: [[f32; 3]; 4] = [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [1.0, 1.0, 0.0], [-1.0, 1.0, 0.0]];
    let normals = [[0.0f32, 0.0, 1.0]; 4];
    let uvs: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let mut bin: Vec<u8> = positions.iter().flatten().chain(normals.iter().flatten()).chain(uvs.iter().flatten()).flat_map(|f| f.to_le_bytes()).collect();
    for i in [0u16, 1, 2, 0, 2, 3] {
        bin.extend(i.to_le_bytes());
    }
    std::fs::write(dir.join("square.bin"), &bin).unwrap();
    let gltf = json!({
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3, "material": 0}]}],
        "materials": [{"pbrMetallicRoughness": {"metallicFactor": 0.0, "roughnessFactor": 1.0}, "normalTexture": {"index": 0}}],
        "textures": [{"source": 0}], "images": [{"uri": "normal.png"}],
        "buffers": [{"uri": "square.bin", "byteLength": bin.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 48}, {"buffer": 0, "byteOffset": 48, "byteLength": 48},
            {"buffer": 0, "byteOffset": 96, "byteLength": 32}, {"buffer": 0, "byteOffset": 128, "byteLength": 12}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-1, -1, 0], "max": [1, 1, 0]},
            {"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC2"},
            {"bufferView": 3, "componentType": 5123, "count": 6, "type": "SCALAR"}
        ]
    });
    std::fs::write(dir.join("square.gltf"), gltf.to_string()).unwrap();
    dir
}

#[test]
fn a_normal_map_bends_the_light_toward_its_right_and_up() {
    let Some(gpu) = gpu() else { return };
    let lit = |name: &str, normal: [u8; 3], light_rotation: [f32; 3]| {
        let dir = normal_mapped_square(name, normal);
        let mut scene = Scene::new();
        object(&mut scene, "mesh", json!({"shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 1, "model": "square.gltf"}), [0.0; 3], [0.0; 3]);
        object(&mut scene, "directionalLight", json!({"color": [1, 1, 1], "intensity": 3, "shadows": false}), [0.0; 3], light_rotation);
        let camera = CameraView::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, 40.0, 1.0);
        let image = render_with_files(&gpu, &scene, &dir, &mut crate::MaterialLibrary::new(), camera);
        std::fs::remove_dir_all(&dir).ok();
        brightness(image.pixel(32, 32).unwrap())
    };
    // A light from the right (+X), grazing the square: turned toward it, the surface is brighter.
    let from_right = [0.0, 80.0, 0.0];
    let (flat, right) = (lit("flat-x", [128, 128, 255], from_right), lit("right", [255, 128, 128], from_right));
    assert!(right > flat + 40, "a normal map leaning right catches light from the right: flat {flat}, leaning {right}");
    // A light from above (+Y): the map's green leans the surface up, toward the top of the image.
    let from_above = [-80.0, 0.0, 0.0];
    let (flat, up) = (lit("flat-y", [128, 128, 255], from_above), lit("up", [128, 255, 128], from_above));
    assert!(up > flat + 40, "a normal map leaning up catches light from above: flat {flat}, leaning {up}");
}

#[test]
fn a_model_with_two_materials_draws_each_part_in_its_own() {
    let dir = std::env::temp_dir().join(format!("bayleaf-render-parts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Two squares side by side, each its own glTF primitive: left green, right red.
    let positions: [[f32; 3]; 8] =
        [[-2.0, -1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 1.0, 0.0], [-2.0, 1.0, 0.0], [0.0, -1.0, 0.0], [2.0, -1.0, 0.0], [2.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
    let mut bin: Vec<u8> = positions.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
    for i in [0u16, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7] {
        bin.extend(i.to_le_bytes());
    }
    std::fs::write(dir.join("two.bin"), &bin).unwrap();
    let gltf = json!({
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [
            {"attributes": {"POSITION": 0}, "indices": 1, "material": 0},
            {"attributes": {"POSITION": 0}, "indices": 2, "material": 1}
        ]}],
        "materials": [
            {"pbrMetallicRoughness": {"baseColorFactor": [0.05, 1.0, 0.05, 1.0], "metallicFactor": 0.0}},
            {"pbrMetallicRoughness": {"baseColorFactor": [1.0, 0.05, 0.05, 1.0], "metallicFactor": 0.0}}
        ],
        "buffers": [{"uri": "two.bin", "byteLength": bin.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 96},
            {"buffer": 0, "byteOffset": 96, "byteLength": 12},
            {"buffer": 0, "byteOffset": 108, "byteLength": 12}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 8, "type": "VEC3", "min": [-2, -1, 0], "max": [2, 1, 0]},
            {"bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR"},
            {"bufferView": 2, "componentType": 5123, "count": 6, "type": "SCALAR"}
        ]
    });
    std::fs::write(dir.join("two.gltf"), gltf.to_string()).unwrap();
    assert_eq!(bayleaf_assets::load_model(&dir.join("two.gltf")).unwrap().parts.len(), 2);

    let Some(gpu) = gpu() else { return };
    let mut scene = Scene::new();
    object(&mut scene, "mesh", json!({"shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 0.9, "model": "two.gltf"}), [0.0; 3], [0.0; 3]);
    object(&mut scene, "directionalLight", json!({"color": [1, 1, 1], "intensity": 3, "shadows": false}), [0.0; 3], [0.0; 3]);
    let camera = CameraView::look_at(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, 50.0, 1.0);
    let image = render_with_files(&gpu, &scene, &dir, &mut crate::MaterialLibrary::new(), camera);
    let (left, right) = (image.pixel(20, 32).unwrap(), image.pixel(44, 32).unwrap());
    assert!(left[1] > left[0] + 60, "the left part is green: {left:?}");
    assert!(right[0] > right[1] + 60, "the right part is red: {right:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_point_light_casts_shadows_all_around_it() {
    let Some(gpu) = gpu() else { return };
    let render = |shadows: bool| {
        let bulb = json!({"color": [1, 1, 1], "intensity": 80, "range": 14, "shadows": shadows});
        let mut scene = floor_scene(&[("pointLight", bulb, [0.0, 3.0, 0.0], [0.0; 3])]);
        // One cube below the light, one off to the side: their shadows fall on different faces.
        object(&mut scene, "mesh", json!({"shape": "cube", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.9}), [0.0, 1.5, 0.0], [0.0; 3]);
        object(&mut scene, "mesh", json!({"shape": "cube", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.9}), [2.0, 1.0, 0.0], [0.0; 3]);
        let frame = extract(&scene, top_down(), OPTIONS);
        let image = gpu.render(&frame, 96, 96).unwrap();
        let at = |p: Vec3| {
            let (x, y) = camera_pixel(&frame.camera, p, 96);
            brightness(image.pixel(x, y).unwrap())
        };
        // Below the first cube (seen past its edge), behind the second, and a lit spot in between.
        (at(Vec3::new(0.0, 0.0, 0.9)), at(Vec3::new(4.2, 0.0, 0.0)), at(Vec3::new(-2.0, 0.0, 0.0)))
    };
    let (under, behind, lit) = render(true);
    assert!(lit > under + 40, "the floor below the first cube is in shadow: under {under}, lit {lit}");
    assert!(lit > behind + 20, "the floor behind the side cube is in shadow: behind {behind}, lit {lit}");
    let (under_off, behind_off, _) = render(false);
    assert!(under_off > under + 40 && behind_off > behind + 20, "without shadows both are lit");
}

#[test]
fn smooth_surfaces_mirror_the_sky_and_rough_ones_blur_it() {
    let Some(gpu) = gpu() else { return };
    let centre = |roughness: f32| {
        let mut scene = Scene::new();
        let ball = mesh(&mut scene, "sphere", [0.0, 0.0, 0.0]);
        scene.set_component(ball, "mesh", json!({"shape": "sphere", "color": [1, 1, 1, 1], "metallic": 1, "roughness": roughness})).unwrap();
        // A red horizon between a blue top and a green ground, no lights.
        let env = json!({"skyColor": [0.0, 0.0, 1.0], "horizonColor": [1.0, 0.0, 0.0], "groundColor": [0.0, 1.0, 0.0], "ambient": 1, "showSky": true});
        object(&mut scene, "environment", env, [0.0; 3], [0.0; 3]);
        let mut frame = extract(&scene, CameraView::look_at(Vec3::new(0.0, 0.0, 2.5), Vec3::ZERO, 30.0, 1.0), OPTIONS);
        frame.lights.clear();
        gpu.render(&frame, 32, 32).unwrap().pixel(16, 16).unwrap()
    };
    // The middle of the ball mirrors the horizon behind the camera.
    let (smooth, rough) = (centre(0.05), centre(1.0));
    assert!(smooth[0] > smooth[2] + 60 && smooth[0] > smooth[1], "a mirror ball shows the red horizon: {smooth:?}");
    assert!(smooth[0] > rough[0] + 40, "a rough ball blurs it: smooth {smooth:?}, rough {rough:?}");
}
#[test]
fn a_small_shape_near_the_camera_casts_a_sharp_shadow_in_a_large_world() {
    let Some(gpu) = gpu() else { return };
    let sun = json!({"color": [1, 1, 1], "intensity": 2, "shadows": true});
    let mut scene = floor_scene(&[("directionalLight", sun, [0.0, 10.0, 0.0], [-90.0, 0.0, 0.0])]);
    let floor = scene.world_matrices()[0].0;
    scene.set_component(floor, "transform", json!({"position": [0, 0, 0], "rotation": [0, 0, 0], "scale": [400, 1, 400]})).unwrap();
    // A small box a little above the ground, close to the camera, and a far one that widens the scene.
    let small = object(&mut scene, "mesh", json!({"shape": "cube", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.9}), [0.0, 0.6, 0.0], [0.0; 3]);
    scene.set_component(small, "transform", json!({"position": [0, 0.6, 0], "rotation": [0, 0, 0], "scale": [0.4, 0.4, 0.4]})).unwrap();
    object(&mut scene, "mesh", json!({"shape": "cube", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.9}), [150.0, 0.5, -150.0], [0.0; 3]);
    let camera = CameraView::look_at(Vec3::new(0.0, 2.5, 3.0), Vec3::new(0.0, 0.0, 0.0), 50.0, 1.0);
    let frame = extract(&scene, camera, OPTIONS);
    let image = gpu.render(&frame, 128, 128).unwrap();
    let at = |p: Vec3| {
        let (x, y) = camera_pixel(&frame.camera, p, 128);
        brightness(image.pixel(x, y).unwrap())
    };
    // Straight below the box, seen past its near edge, and a little to the side of its shadow.
    let (under, beside) = (at(Vec3::new(0.0, 0.0, 0.15)), at(Vec3::new(0.6, 0.0, 0.15)));
    assert!(beside > under + 60, "the small box shadows the ground: under {under}, beside {beside}");
}

#[test]
fn a_scene_1000_km_from_the_centre_looks_as_it_does_at_the_centre() {
    let Some(gpu) = gpu() else { return };
    let render = |offset: f64| {
        let mut scene = Scene::new();
        for (shape, x) in [("cube", -0.6), ("sphere", 0.6)] {
            let id = mesh(&mut scene, shape, [0.0; 3]);
            let transform = bayleaf_scene::components::Transform { position: [offset + x, 0.0, offset], rotation: [0.0, 30.0, 0.0], scale: [0.5; 3] };
            scene.set(id, transform).unwrap();
        }
        object(&mut scene, "directionalLight", json!({"color": [1, 1, 1], "intensity": 3}), [0.0; 3], [-40.0, 20.0, 0.0]);
        let center = bayleaf_core::math::DVec3::new(offset, 0.0, offset);
        let camera = CameraView::look_at_world(center + bayleaf_core::math::DVec3::new(0.0, 0.4, 2.0), center, 50.0, 1.0);
        gpu.render(&extract(&scene, camera, OPTIONS), 48, 48).unwrap()
    };
    let (near, far) = (render(0.0), render(1.0e6));
    let differing = (0..48).flat_map(|y| (0..48).map(move |x| (x, y))).filter(|(x, y)| near.pixel(*x, *y) != far.pixel(*x, *y)).count();
    assert!(differing < 10, "{differing} pixels differ between the centre and 1000 km out");
}

#[test]
fn a_selected_shape_keeps_its_colours_and_gets_an_outline_around_it() {
    let Some(gpu) = gpu() else { return };
    let mut scene = Scene::new();
    let cube = mesh(&mut scene, "cube", [0.0, 0.0, 0.0]);
    object(&mut scene, "directionalLight", json!({"color": [1, 1, 1], "intensity": 3}), [0.0; 3], [0.0; 3]);
    let camera = CameraView::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, 50.0, 1.0);
    let plain = gpu.render(&extract(&scene, camera, OPTIONS), 64, 64).unwrap();
    let selected = gpu.render(&extract(&scene, camera, FrameOptions { selected: Some(cube), ..OPTIONS }), 64, 64).unwrap();
    assert_eq!(selected.pixel(32, 32), plain.pixel(32, 32), "the surface is not tinted");
    // The cube's left edge on the middle row, then a pixel just outside it.
    let background = plain.pixel(1, 32).unwrap();
    let edge = (0..32).find(|x| plain.pixel(*x, 32) != Some(background)).unwrap();
    let outside = selected.pixel(edge - 2, 32).unwrap();
    assert_eq!(plain.pixel(edge - 2, 32), Some(background), "outside the cube there is background");
    assert!(outside[0] > outside[2] + 80, "a selected shape has an orange band around it: {outside:?}");
}

#[test]
fn the_ground_grid_shows_when_it_is_on_near_and_far_from_the_centre() {
    let Some(gpu) = gpu() else { return };
    let scene = Scene::new();
    // Pixels that differ between two pictures of the same view.
    let changed = |a: &crate::headless::Image, b: &crate::headless::Image| {
        (0..64).flat_map(|y| (0..64).map(move |x| (x, y))).filter(|&(x, y)| a.pixel(x, y) != b.pixel(x, y)).count()
    };
    // Looking down at the ground from above, at the world's centre and far from it, as the editor's
    // view measures the frame from its eye.
    for at in [DVec3::new(0.0, 0.0, 0.0), DVec3::new(250_000.5, 0.0, -75_000.25)] {
        let camera = || CameraView::look_at_world(at + DVec3::new(0.0, 6.0, 6.0), at, 60.0, 1.0);
        let with = gpu.render(&extract(&scene, camera(), FrameOptions { show_grid: true, ..OPTIONS }), 64, 64).unwrap();
        let without = gpu.render(&extract(&scene, camera(), OPTIONS), 64, 64).unwrap();
        assert!(changed(&with, &without) > 64, "the grid draws lines on the ground around {at:?}");
    }
}

/// Gives one lighting asset for every lighting path.
struct OneLighting(Arc<bayleaf_scene::lighting::Lighting>);

impl crate::AssetSource for OneLighting {
    fn material(&mut self, _: &str) -> Option<Arc<crate::CompiledMaterial>> {
        None
    }
    fn image(&mut self, _: &str) -> Option<Arc<bayleaf_assets::Image>> {
        None
    }
    fn model(&mut self, _: &str) -> Option<Arc<bayleaf_assets::MeshData>> {
        None
    }
    fn lighting(&mut self, _: &str) -> Option<Arc<bayleaf_scene::lighting::Lighting>> {
        Some(Arc::clone(&self.0))
    }
}

#[test]
fn a_lighting_asset_gives_the_sky_and_a_sun_where_the_scene_has_none() {
    let mut scene = Scene::new();
    let mut settings = scene.settings().clone();
    settings.lighting = "day.blight".to_owned();
    scene.set_settings(settings).unwrap();
    let lighting = bayleaf_scene::lighting::Lighting::default();
    let camera = CameraView::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, 50.0, 1.0);
    let frame = extract_with(&scene, camera, OPTIONS, &mut OneLighting(Arc::new(lighting.clone())));
    assert!(frame.sky.is_some(), "the asset's sky shows");
    assert_eq!(frame.lights.len(), 1, "its sun lights a scene without a Directional Light");
    // A scene with its own sun keeps it alone.
    object(&mut scene, "directionalLight", json!({"color": [1, 1, 1], "intensity": 3}), [0.0; 3], [0.0; 3]);
    let frame = extract_with(&scene, camera, OPTIONS, &mut OneLighting(Arc::new(lighting)));
    assert_eq!(frame.lights.len(), 1, "the scene's own Directional Light, not the asset's sun as well");
}

/// A sphere lit by a sun from the front-left, with `material`, seen head on: the picture's middle row.
fn sphere_row(gpu: &HeadlessGpu, material: Arc<crate::CompiledMaterial>) -> Vec<[u8; 4]> {
    let mut scene = Scene::new();
    let id = mesh(&mut scene, "sphere", [0.0, 0.0, 0.0]);
    scene
        .set_component(id, "mesh", json!({"shape": "sphere", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.5, "material": "m.bmaterial"}))
        .unwrap();
    object(&mut scene, "directionalLight", json!({"color": [1, 1, 1], "intensity": 2}), [0.0; 3], [0.0, -60.0, 0.0]);
    let camera = CameraView::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, 50.0, 1.0);
    let frame = extract_with(&scene, camera, OPTIONS, &mut Materials(None, material));
    let image = gpu.render(&frame, 96, 96).unwrap();
    (0..96).map(|x| image.pixel(x, 48).unwrap()).collect()
}

#[test]
fn custom_lighting_bands_the_light_and_an_outline_rims_the_shape() {
    let Some(gpu) = gpu() else { return };
    // The engine's lighting: a smooth shading across the sphere.
    let plain = Arc::new(compile(&Graph::new(&CATALOG)).unwrap());
    let smooth = sphere_row(&gpu, plain);

    // Custom lighting: the main light's angle cut into three bands, times its colour and shadow.
    let mut g = Graph::empty(&CATALOG);
    let out = g.add_node(&CATALOG, "customLighting", [0.0, 0.0]).unwrap();
    let light = g.add_node(&CATALOG, "mainLight", [0.0, 0.0]).unwrap();
    let normal = g.add_node(&CATALOG, "normal", [0.0, 0.0]).unwrap();
    let dot = g.add_node(&CATALOG, "dot", [0.0, 0.0]).unwrap();
    link(&mut g, normal, "normal", dot, "a");
    link(&mut g, light, "direction", dot, "b");
    let bands = g.add_node(&CATALOG, "posterize", [0.0, 0.0]).unwrap();
    link(&mut g, dot, "result", bands, "value");
    let lit = g.add_node(&CATALOG, "multiply", [0.0, 0.0]).unwrap();
    link(&mut g, bands, "result", lit, "a");
    link(&mut g, light, "shadow", lit, "b");
    link(&mut g, lit, "result", out, "color");
    let toon = sphere_row(&gpu, Arc::new(compile(&g).unwrap_or_else(|p| panic!("{p:?}"))));
    let shades = |row: &[[u8; 4]]| row.iter().map(|p| p[0]).collect::<std::collections::BTreeSet<u8>>().len();
    assert!(shades(&toon) * 3 < shades(&smooth), "flat bands: {} shades against {}", shades(&toon), shades(&smooth));

    // An outline: a red rim just outside the sphere's edge, where the background was.
    g.set_value(&CATALOG, out, "outlineWidth", json!(0.08)).unwrap();
    g.set_value(&CATALOG, out, "outlineColor", json!([1.0, 0.0, 0.0])).unwrap();
    let compiled = compile(&g).unwrap();
    assert!(compiled.outline.is_some());
    let rimmed = sphere_row(&gpu, Arc::new(compiled));
    // Background pixels are the ones like the row's first (beside the sphere).
    let background = toon[0];
    let red = |p: [u8; 4]| p[0] > 150 && p[1] < 60 && p[2] < 60;
    let rim = (0..toon.len()).filter(|i| toon[*i] == background && red(rimmed[*i])).count();
    assert!(rim >= 2, "a red rim where the background was beside the sphere: {rim} pixels");
    assert!(!toon.iter().any(|p| red(*p)), "no rim without the outline");
    assert_eq!(rimmed[48], toon[48], "the shape itself stays as it was in front of its outline");
}

#[test]
fn the_example_materials_compile() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo/materials");
    let mut seen = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "bmaterial") {
            continue;
        }
        let graph = Graph::from_json(&std::fs::read_to_string(&path).unwrap(), &CATALOG).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let compiled = compile(&graph).unwrap_or_else(|p| panic!("{}: {p:?}", path.display()));
        seen.push((path.file_stem().unwrap().to_string_lossy().into_owned(), compiled));
    }
    let toon = seen.iter().find(|(name, _)| name == "toon").map(|(_, m)| m).expect("the toon example ships");
    assert!(toon.outline.is_some() && toon.surface.contains("CUSTOM_LIGHTING: bool = true"), "it shows the stylised nodes together");
}

#[test]
fn post_processing_effects_each_change_the_picture_as_they_say() {
    let Some(gpu) = gpu() else { return };
    use crate::PostSettings;
    // A bright orange cube in the middle on a grey floor, under a sun, against a grey background.
    let mut scene = floor_scene(&[("directionalLight", json!({"color": [1, 1, 1], "intensity": 2}), [0.0, 5.0, 0.0], [-60.0, 20.0, 0.0])]);
    object(&mut scene, "mesh", json!({"shape": "cube", "color": [1.0, 0.5, 0.1, 1], "metallic": 0, "roughness": 0.6}), [0.0, 0.5, 0.0], [0.0; 3]);
    let camera = CameraView::look_at(Vec3::new(0.0, 2.5, 4.0), Vec3::new(0.0, 0.3, 0.0), 50.0, 1.0);
    let base = extract(&scene, camera, OPTIONS);
    let render = |post: PostSettings| {
        let mut frame = base.clone();
        frame.post = post;
        gpu.render(&frame, 96, 96).unwrap()
    };
    let plain = render(PostSettings::default());
    let centre = camera_pixel(&base.camera, Vec3::new(0.0, 0.5, 0.5), 96);
    let at = |image: &crate::Image, p: (u32, u32)| image.pixel(p.0, p.1).unwrap();
    let light = |p: [u8; 4]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);

    let darker = render(PostSettings { exposure: 0.4, ..PostSettings::default() });
    assert!(light(at(&darker, centre)) + 60 < light(at(&plain, centre)), "less exposure, darker");

    let grey = render(PostSettings { saturation: 0.0, ..PostSettings::default() });
    let [r, g, b, _] = at(&grey, centre);
    assert!(r.abs_diff(g) < 6 && g.abs_diff(b) < 6, "no saturation, grey: {:?}", [r, g, b]);

    let corner = (2, 2);
    let vignetted = render(PostSettings { vignette: 0.8, ..PostSettings::default() });
    assert!(light(at(&vignetted, corner)) + 30 < light(at(&plain, corner)), "darker corners");
    assert!(light(at(&vignetted, centre)).abs_diff(light(at(&plain, centre))) < 12, "the middle as it was");

    // An overbright cube: without tone mapping its colour clips; with it, it keeps its hue.
    let mut bright = base.clone();
    // Only the cube (the orange item), not the floor.
    for item in bright.items.iter_mut().filter(|i| (i.color.y - 0.5).abs() < 1e-3) {
        item.color *= Vec4::new(6.0, 6.0, 6.0, 1.0);
    }
    let render_bright = |post: PostSettings| {
        let mut frame = bright.clone();
        frame.post = post;
        gpu.render(&frame, 96, 96).unwrap()
    };
    let clipped = at(&render_bright(PostSettings::default()), centre);
    let mapped = at(&render_bright(PostSettings { tone_mapping: true, ..PostSettings::default() }), centre);
    assert!(clipped[0] == 255 && clipped[1] == 255, "clipped: {clipped:?}");
    assert!(mapped[0] > mapped[1] && mapped[1] > mapped[2], "tone mapped keeps orange: {mapped:?}");

    // Bloom: the overbright cube's glow reaches past its edge onto the floor beside it.
    let beside = camera_pixel(&base.camera, Vec3::new(0.85, 0.05, 0.6), 96);
    let glow = render_bright(PostSettings { bloom: 1.0, bloom_threshold: 1.0, ..PostSettings::default() });
    assert!(light(at(&glow, beside)) > light(at(&render_bright(PostSettings::default()), beside)) + 20, "a glow beside the cube");

    // Depth fog: the far floor takes the fog colour, the near one less so.
    let fogged = render(PostSettings { fog: Some(([0.0, 0.0, 1.0], 2.0, 8.0)), ..PostSettings::default() });
    let far = camera_pixel(&base.camera, Vec3::new(0.0, 0.0, -6.0), 96);
    let [r, _, b, _] = at(&fogged, far);
    assert!(b > r + 60, "the far floor is blue fog: {:?}", at(&fogged, far));

    // Ambient occlusion: where the cube meets the floor goes darker.
    let crease = camera_pixel(&base.camera, Vec3::new(0.0, 0.02, 0.55), 96);
    let occluded = render(PostSettings { occlusion: 1.0, occlusion_radius: 0.4, ..PostSettings::default() });
    assert!(light(at(&occluded, crease)) + 15 < light(at(&plain, crease)), "the crease darkens");

    // Antialiasing changes only edges: the middle of the cube stays.
    let smoothed = render(PostSettings { antialiasing: true, ..PostSettings::default() });
    assert_eq!(at(&smoothed, centre), at(&plain, centre));
}
