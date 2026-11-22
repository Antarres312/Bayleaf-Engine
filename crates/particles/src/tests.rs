use bayleaf_core::StableId;
use bayleaf_core::math::Vec3;
use bayleaf_render::{CameraView, NoAssets};
use bayleaf_scene::Scene;
use bayleaf_scene::components::{ColorKey, ParticleEmitter, SizeKey, Transform};
use serde_json::json;

use crate::{Particles, color_at, size_at};

fn scene_with(emitter: serde_json::Value) -> (Scene, StableId) {
    let mut scene = Scene::new();
    let id = scene.create_object(None, "Sparks").unwrap();
    let mut value = serde_json::to_value(ParticleEmitter::default()).unwrap();
    for (key, v) in emitter.as_object().unwrap() {
        value[key] = v.clone();
    }
    scene.add_component(id, "particleEmitter").unwrap();
    scene.set_component(id, "particleEmitter", value).unwrap();
    (scene, id)
}

fn run(particles: &mut Particles, scene: &Scene, seconds: f32) {
    for _ in 0..(seconds * 60.0).round() as usize {
        particles.step(scene, 1.0 / 60.0);
    }
}

fn camera() -> CameraView {
    CameraView::look_at(Vec3::new(0.0, 1.0, 6.0), Vec3::ZERO, 50.0, 1.0)
}

#[test]
fn a_cone_emits_at_its_rate_upwards_and_particles_live_out_their_lives() {
    let (scene, id) = scene_with(json!({"rate": 20.0, "lifetime": [5.0, 5.0], "speed": [2.0, 2.0]}));
    let mut particles = Particles::new();
    run(&mut particles, &scene, 1.0);
    let count = particles.count(Some(id));
    assert!((15..=25).contains(&count), "about 20 in a second: {count}");
    let items = particles.draw_items(&scene, &camera(), &mut NoAssets);
    assert_eq!(items.len(), count, "one item a particle");
    let mean_y: f32 = items.iter().map(|i| i.model.w_axis.y).sum::<f32>() / count as f32;
    assert!(mean_y > -0.5, "they rise from the emitter: {mean_y}");

    // Stopped, nothing new starts; after the longest life, none are left.
    particles.stop(id);
    run(&mut particles, &scene, 5.5);
    assert_eq!(particles.count(Some(id)), 0);
}

#[test]
fn an_emitter_keeps_within_its_most_particles_and_a_burst_starts_many_at_once() {
    let (scene, id) = scene_with(json!({"rate": 1000.0, "maxParticles": 50, "lifetime": [10.0, 10.0]}));
    let mut particles = Particles::new();
    run(&mut particles, &scene, 1.0);
    assert_eq!(particles.count(Some(id)), 50, "never more than its most");

    let (scene, id) = scene_with(json!({"rate": 0.0, "playing": false}));
    let mut particles = Particles::new();
    particles.step(&scene, 0.0);
    particles.burst(&scene, id, 30);
    assert_eq!(particles.count(Some(id)), 30, "a burst from a stopped emitter");
    particles.restart(id);
    assert_eq!(particles.count(Some(id)), 0, "restart takes them away");
}

#[test]
fn an_emitter_that_does_not_loop_stops_after_one_cycle() {
    let (scene, id) = scene_with(json!({"looping": false, "duration": 0.5, "rate": 10.0, "lifetime": [0.2, 0.2]}));
    let mut particles = Particles::new();
    run(&mut particles, &scene, 0.25);
    assert!(particles.is_playing(id));
    run(&mut particles, &scene, 1.0);
    assert!(!particles.is_playing(id), "one cycle, then it stops");
    assert_eq!(particles.count(Some(id)), 0);
}

#[test]
fn local_particles_move_with_their_object_and_world_ones_stay() {
    let moved = |space: &str| {
        let (mut scene, id) = scene_with(json!({"space": space, "rate": 0.0, "speed": [0.0, 0.0], "lifetime": [10.0, 10.0], "shape": "point"}));
        let mut particles = Particles::new();
        particles.step(&scene, 0.0);
        particles.burst(&scene, id, 1);
        let before = particles.draw_items(&scene, &camera(), &mut NoAssets)[0].model.w_axis;
        scene.set(id, Transform { position: [3.0, 0.0, 0.0], ..Transform::default() }).unwrap();
        let after = particles.draw_items(&scene, &camera(), &mut NoAssets)[0].model.w_axis;
        after.x - before.x
    };
    assert!((moved("local") - 3.0).abs() < 1e-3, "a local particle follows its object");
    assert!(moved("world").abs() < 1e-3, "a world particle stays where it started");
}

#[test]
fn sprites_face_the_camera_and_flipbooks_move_through_their_frames() {
    let (scene, id) =
        scene_with(json!({"rate": 0.0, "speed": [0.0, 0.0], "lifetime": [1.0, 1.0], "flipbookColumns": 2, "flipbookRows": 2, "blend": "additive"}));
    let mut particles = Particles::new();
    particles.step(&scene, 0.0);
    particles.burst(&scene, id, 1);
    let cam = camera();
    let first = particles.draw_items(&scene, &cam, &mut NoAssets).remove(0);
    // The quad's +Z (its facing) points back towards the camera.
    let facing = first.model.z_axis.truncate().normalize();
    let to_camera = Vec3::new(cam.view.x_axis.z, cam.view.y_axis.z, cam.view.z_axis.z);
    assert!(facing.dot(to_camera) > 0.99, "faces the camera");
    assert!(first.additive && first.unlit);
    assert_eq!(first.uv_rect, [0.0, 0.0, 0.5, 0.5], "the first frame at the start of its life");
    for _ in 0..48 {
        particles.step(&scene, 1.0 / 60.0);
    }
    let later = particles.draw_items(&scene, &cam, &mut NoAssets).remove(0);
    assert_ne!(later.uv_rect, first.uv_rect, "a later frame later in its life");
}

#[test]
fn a_surface_emitter_starts_its_particles_on_its_mesh_and_curves_set_size_and_colour() {
    let mut scene = Scene::new();
    let id = scene.create_object(None, "Box").unwrap();
    scene.add_component(id, "mesh").unwrap();
    let emitter = ParticleEmitter {
        shape: "surface".to_owned(),
        rate: 0.0,
        burst: 200,
        speed: [0.0, 0.0],
        gravity: 0.0,
        size_keys: vec![SizeKey { at: 0.0, size: 2.0 }, SizeKey { at: 1.0, size: 4.0 }],
        color_keys: vec![ColorKey { at: 0.0, color: [1.0, 0.0, 0.0, 1.0] }, ColorKey { at: 1.0, color: [0.0, 0.0, 1.0, 1.0] }],
        ..ParticleEmitter::default()
    };
    scene.add_component(id, "particleEmitter").unwrap();
    scene.set_component(id, "particleEmitter", serde_json::to_value(&emitter).unwrap()).unwrap();
    let mut particles = Particles::new();
    particles.step(&scene, 0.001);
    let positions = particles.positions(id);
    assert_eq!(positions.len(), 200);
    // On the unit cube's surface: one coordinate is at its face, none beyond.
    assert!(positions.iter().all(|p| p.abs().max_element() > 0.499 && p.abs().max_element() < 0.501), "on the faces: {:?}", &positions[..3]);
    assert!(positions.iter().any(|p| p.x > 0.49) && positions.iter().any(|p| p.y < -0.49), "over every face, not one");
    // Halfway through the life, halfway along each curve.
    assert!((size_at(&emitter, 0.5) - 3.0).abs() < 1e-5);
    let middle = color_at(&emitter, 0.5);
    assert!((middle.x - 0.5).abs() < 1e-5 && (middle.z - 0.5).abs() < 1e-5);
    // Without points, start, middle and end.
    let plain = ParticleEmitter::default();
    assert!((size_at(&plain, 0.0) - plain.start_size).abs() < 1e-6);
}
