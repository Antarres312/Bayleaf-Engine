use bayleaf_core::StableId;
use bayleaf_scene::Scene;
use bayleaf_scene::components::Transform;
use serde_json::json;

use super::{ContactKind, GRAVITY, Physics, STEP};

/// An object with the given components at `position`.
fn object(scene: &mut Scene, name: &str, components: serde_json::Value, position: [f32; 3], scale: [f32; 3]) -> StableId {
    let id = scene.create_object(None, name).unwrap();
    for (component, value) in components.as_object().unwrap() {
        scene.add_component(id, component).unwrap();
        scene.set_component(id, component, value.clone()).unwrap();
    }
    scene.set(id, Transform { position, rotation: [0.0; 3], scale }).unwrap();
    id
}

fn y(scene: &Scene, id: StableId) -> f32 {
    scene.get::<Transform>(id).unwrap().position[1]
}

/// A wide static floor whose top is at y = 0.
fn floor(scene: &mut Scene) -> StableId {
    object(
        scene,
        "Floor",
        json!({"collider": {"shape": "box", "size": [1, 1, 1], "friction": 0.5, "restitution": 0, "trigger": false}}),
        [0.0, -0.5, 0.0],
        [20.0, 1.0, 20.0],
    )
}

fn crate_box(scene: &mut Scene, name: &str, height: f32) -> StableId {
    let body = json!({"rigidBody": {"kind": "dynamic", "mass": 1, "gravityScale": 1, "lockRotation": false}, "collider": {"shape": "box", "size": [1, 1, 1], "friction": 0.5, "restitution": 0, "trigger": false}});
    object(scene, name, body, [0.0, height, 0.0], [1.0; 3])
}

#[test]
fn a_box_falls_lands_on_the_floor_and_says_so() {
    let mut scene = Scene::new();
    let floor = floor(&mut scene);
    let crate_id = crate_box(&mut scene, "Crate", 3.0);
    let mut physics = Physics::build(&scene, GRAVITY);
    assert_eq!(physics.len(), 2);
    let mut contacts = Vec::new();
    for _ in 0..180 {
        contacts.extend(physics.step(&mut scene, STEP));
    }
    assert!((y(&scene, crate_id) - 0.5).abs() < 0.05, "it rests on the floor: y = {}", y(&scene, crate_id));
    assert!((y(&scene, floor) + 0.5).abs() < 1e-6, "the floor does not move");
    assert!(contacts.iter().any(|c| c.object == crate_id && c.other == floor && c.kind == ContactKind::CollisionStart));
    assert!(contacts.iter().any(|c| c.object == floor && c.other == crate_id), "both sides hear of it");
}

#[test]
fn triggers_notice_without_blocking_and_impulses_push() {
    let mut scene = Scene::new();
    floor(&mut scene);
    let zone = object(
        &mut scene,
        "Zone",
        json!({"collider": {"shape": "box", "size": [4, 1, 4], "friction": 0.5, "restitution": 0, "trigger": true}}),
        [0.0, 2.0, 0.0],
        [1.0; 3],
    );
    let ball = crate_box(&mut scene, "Ball", 4.0);
    let mut physics = Physics::build(&scene, GRAVITY);
    let mut contacts = Vec::new();
    for _ in 0..120 {
        contacts.extend(physics.step(&mut scene, STEP));
    }
    assert!(y(&scene, ball) < 1.0, "it fell through the trigger");
    assert!(contacts.iter().any(|c| c.object == zone && c.other == ball && c.kind == ContactKind::TriggerEnter));
    assert!(contacts.iter().any(|c| c.object == zone && c.kind == ContactKind::TriggerExit));

    // An upward impulse lifts it; a spawned box joins on the next step.
    assert!(physics.apply_impulse(ball, [0.0, 8.0, 0.0]));
    assert!(!physics.apply_impulse(zone, [0.0, 8.0, 0.0]), "a static collider cannot be pushed");
    physics.step(&mut scene, STEP * 10.0);
    assert!(y(&scene, ball) > 1.0, "the impulse lifted it: y = {}", y(&scene, ball));
    let late = crate_box(&mut scene, "Late", 6.0);
    physics.step(&mut scene, STEP);
    assert!(physics.velocity(late).is_some_and(|v| v[1] < 0.0), "the new box falls");
    scene.delete_object(late).unwrap();
    physics.step(&mut scene, STEP);
    assert!(physics.velocity(late).is_none(), "a deleted object's body is gone");
}

#[test]
fn gravity_comes_from_the_caller() {
    let mut scene = Scene::new();
    let up = crate_box(&mut scene, "Up", 0.0);
    let mut physics = Physics::build(&scene, [0.0, 5.0, 0.0]);
    for _ in 0..30 {
        physics.step(&mut scene, STEP);
    }
    assert!(y(&scene, up) > 0.5, "upward gravity lifts it: y = {}", y(&scene, up));
}
