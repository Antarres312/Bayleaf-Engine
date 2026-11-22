use bayleaf_core::StableId;
use bayleaf_scene::Scene;
use bayleaf_scene::components::Transform;
use serde_json::json;

use super::{ContactKind, GRAVITY, Physics, Precision, STEP};

/// An object with the given components at `position`.
fn object(scene: &mut Scene, name: &str, components: serde_json::Value, position: [f32; 3], scale: [f32; 3]) -> StableId {
    let id = scene.create_object(None, name).unwrap();
    for (component, value) in components.as_object().unwrap() {
        scene.add_component(id, component).unwrap();
        scene.set_component(id, component, value.clone()).unwrap();
    }
    scene.set(id, Transform { position: position.map(f64::from), rotation: [0.0; 3], scale }).unwrap();
    id
}

fn y(scene: &Scene, id: StableId) -> f64 {
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
    for precision in [Precision::Single, Precision::Double] {
        a_box_falls_lands_on_the_floor_and_says_so_in(precision);
    }
}

fn a_box_falls_lands_on_the_floor_and_says_so_in(precision: Precision) {
    let mut scene = Scene::new();
    let floor = floor(&mut scene);
    let crate_id = crate_box(&mut scene, "Crate", 3.0);
    let mut physics = Physics::build(&scene, GRAVITY, precision);
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
    for precision in [Precision::Single, Precision::Double] {
        triggers_notice_without_blocking_and_impulses_push_in(precision);
    }
}

fn triggers_notice_without_blocking_and_impulses_push_in(precision: Precision) {
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
    let mut physics = Physics::build(&scene, GRAVITY, precision);
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
    for precision in [Precision::Single, Precision::Double] {
        gravity_comes_from_the_caller_in(precision);
    }
}

fn gravity_comes_from_the_caller_in(precision: Precision) {
    let mut scene = Scene::new();
    let up = crate_box(&mut scene, "Up", 0.0);
    let mut physics = Physics::build(&scene, [0.0, 5.0, 0.0], precision);
    for _ in 0..30 {
        physics.step(&mut scene, STEP);
    }
    assert!(y(&scene, up) > 0.5, "upward gravity lifts it: y = {}", y(&scene, up));
}

#[test]
fn double_precision_keeps_a_box_exact_far_from_the_centre() {
    // 1000 km out, a resting box stays put to the millimetre in double precision.
    let far = 1.0e6;
    let mut scene = Scene::new();
    let ground = json!({"collider": {"shape": "box", "size": [1, 1, 1], "friction": 0.5, "restitution": 0, "trigger": false}});
    let floor = scene.create_object(None, "Floor").unwrap();
    scene.add_component(floor, "collider").unwrap();
    scene.set_component(floor, "collider", ground["collider"].clone()).unwrap();
    scene.set(floor, Transform { position: [far, -0.5, far], rotation: [0.0; 3], scale: [20.0, 1.0, 20.0] }).unwrap();
    let crate_id = crate_box(&mut scene, "Crate", 0.5);
    scene.set(crate_id, Transform { position: [far + 0.25, 0.5, far - 0.25], rotation: [0.0; 3], scale: [1.0; 3] }).unwrap();
    let mut physics = Physics::build(&scene, GRAVITY, Precision::Double);
    for _ in 0..120 {
        physics.step(&mut scene, STEP);
    }
    let p = scene.get::<Transform>(crate_id).unwrap().position;
    assert!((p[0] - (far + 0.25)).abs() < 1e-3 && (p[2] - (far - 0.25)).abs() < 1e-3, "it did not drift: {p:?}");
    assert!((p[1] - 0.5).abs() < 0.01, "it rests on the floor: {p:?}");
    assert!(!physics.out_of_range(), "double precision has no range limit");
    let single = Physics::build(&scene, GRAVITY, Precision::Single);
    assert!(single.out_of_range(), "single precision warns this far out");
}

fn character(scene: &mut Scene, x: f64) -> StableId {
    let body = json!({"characterBody": {"height": 1.8, "radius": 0.4, "maxSlope": 45, "stepHeight": 0.3}});
    let id = object(scene, "Walker", body, [0.0, 1.5, 0.0], [1.0; 3]);
    scene.set(id, Transform { position: [x, 1.5, 0.0], rotation: [0.0; 3], scale: [1.0; 3] }).unwrap();
    id
}

fn x(scene: &Scene, id: StableId) -> f64 {
    scene.get::<Transform>(id).unwrap().position[0]
}

#[test]
fn a_character_stands_walks_stops_at_a_wall_and_jumps() {
    for precision in [Precision::Single, Precision::Double] {
        let mut scene = Scene::new();
        floor(&mut scene);
        let wall = json!({"collider": {"shape": "box", "size": [1, 4, 6], "friction": 0.5, "restitution": 0, "trigger": false}});
        object(&mut scene, "Wall", wall, [3.0, 2.0, 0.0], [1.0; 3]);
        let walker = character(&mut scene, 0.0);
        let mut physics = Physics::build(&scene, GRAVITY, precision);
        for _ in 0..60 {
            physics.step(&mut scene, STEP);
        }
        assert!((y(&scene, walker) - 0.9).abs() < 0.1, "it stands on the floor: y = {}", y(&scene, walker));
        assert!(physics.grounded().contains(&walker));

        assert!(physics.walk(walker, [2.0, 0.0, 0.0]));
        for _ in 0..120 {
            physics.step(&mut scene, STEP);
        }
        let stopped = x(&scene, walker);
        assert!(stopped > 1.5 && stopped < 2.2, "it walked up to the wall and stopped there: x = {stopped}");

        assert!(physics.walk(walker, [0.0; 3]));
        assert!(physics.jump(walker, 5.0));
        for _ in 0..10 {
            physics.step(&mut scene, STEP);
        }
        assert!(y(&scene, walker) > 1.3, "the jump lifted it: y = {}", y(&scene, walker));
        assert!(!physics.jump(walker, 5.0), "no second jump in the air");
        assert!(!physics.walk(floor_id(&scene), [1.0, 0.0, 0.0]), "only characters walk");
    }
}

fn floor_id(scene: &Scene) -> StableId {
    scene.world_matrices().into_iter().map(|(id, _)| id).find(|id| scene.name(*id) == Some("Floor")).unwrap()
}

#[test]
fn rays_hit_colliders_but_not_triggers_or_the_caster() {
    for precision in [Precision::Single, Precision::Double] {
        let mut scene = Scene::new();
        let floor = floor(&mut scene);
        let zone = json!({"collider": {"shape": "box", "size": [4, 1, 4], "friction": 0.5, "restitution": 0, "trigger": true}});
        object(&mut scene, "Zone", zone, [0.0, 2.0, 0.0], [1.0; 3]);
        let ball = crate_box(&mut scene, "Ball", 4.0);
        let physics = Physics::build(&scene, GRAVITY, precision);
        let down = bayleaf_core::math::DVec3::NEG_Y;
        let hit = physics.raycast(bayleaf_core::math::DVec3::new(0.0, 10.0, 0.0), down, 100.0, Some(ball)).unwrap();
        assert_eq!(hit.object, floor, "the trigger and the caster's own box are left out");
        assert!((hit.distance - 10.0).abs() < 1e-3 && hit.normal.y > 0.99, "{hit:?}");
        let blocked = physics.raycast(bayleaf_core::math::DVec3::new(0.0, 10.0, 0.0), down, 100.0, None).unwrap();
        assert_eq!(blocked.object, ball, "without leaving it out, the box is hit first");
        assert!(physics.raycast(bayleaf_core::math::DVec3::new(0.0, 10.0, 0.0), down, 3.0, None).is_none(), "nothing within 3 units");
    }
}

fn joint(kind: &str, connected: &str, anchor: [f32; 3], axis: [f32; 3]) -> serde_json::Value {
    json!({"kind": kind, "connected": connected, "anchor": anchor, "axis": axis, "limited": false, "lower": -45, "upper": 45, "motorSpeed": 0, "motorForce": 0})
}

/// A box with a Rigid Body, a Collider and `joint`, at `position`.
fn jointed(scene: &mut Scene, name: &str, joint: serde_json::Value, position: [f32; 3]) -> StableId {
    let body = json!({
        "rigidBody": {"kind": "dynamic", "mass": 1, "gravityScale": 1, "lockRotation": false},
        "collider": {"shape": "box", "size": [1, 1, 1], "friction": 0.5, "restitution": 0, "trigger": false},
        "joint": joint,
    });
    object(scene, name, body, position, [1.0; 3])
}

fn world_point(scene: &Scene, id: StableId, local: [f64; 3]) -> bayleaf_core::math::DVec3 {
    scene.world_matrix(id).unwrap().transform_point3(local.into())
}

#[test]
fn a_hinge_to_the_world_keeps_its_anchor_while_the_box_swings_down() {
    for precision in [Precision::Single, Precision::Double] {
        let mut scene = Scene::new();
        // Hinged at its left edge about Z, like a door on its side.
        let door = jointed(&mut scene, "Door", joint("hinge", "", [-0.5, 0.0, 0.0], [0.0, 0.0, 1.0]), [0.0, 2.0, 0.0]);
        let anchor = world_point(&scene, door, [-0.5, 0.0, 0.0]);
        let mut physics = Physics::build(&scene, GRAVITY, precision);
        let (mut lowest, mut drift) = (f64::MAX, 0.0f64);
        for _ in 0..60 {
            physics.step(&mut scene, STEP);
            lowest = lowest.min(y(&scene, door));
            drift = drift.max(world_point(&scene, door, [-0.5, 0.0, 0.0]).distance(anchor));
        }
        // A pendulum: it swings down through the bottom (centre 0.5 below the hinge) and back up.
        assert!(drift < 0.05, "the hinge holds: it moved {drift}");
        assert!(lowest < 1.6 && lowest > 1.3, "the box swings down about it, no further: lowest y = {lowest}");
    }
}

#[test]
fn a_fixed_joint_carries_one_box_with_another_and_a_slider_stops_at_its_limit() {
    let mut scene = Scene::new();
    // Weightless, so only the push moves them.
    let leader = object(
        &mut scene,
        "Leader",
        json!({"rigidBody": {"kind": "dynamic", "mass": 1, "gravityScale": 0, "lockRotation": false}, "collider": {"shape": "box", "size": [1, 1, 1], "friction": 0.5, "restitution": 0, "trigger": false}}),
        [0.0, 5.0, 0.0],
        [1.0; 3],
    );
    let follower = jointed(&mut scene, "Follower", joint("fixed", "Leader", [0.0; 3], [0.0, 1.0, 0.0]), [2.0, 5.0, 0.0]);
    scene.set_component(follower, "rigidBody", json!({"kind": "dynamic", "mass": 1, "gravityScale": 0, "lockRotation": false})).unwrap();
    // A slider hanging under gravity, free to slide down half a unit along Y.
    let mut slider = joint("slider", "", [0.0; 3], [0.0, 1.0, 0.0]);
    slider["limited"] = json!(true);
    slider["lower"] = json!(-0.5);
    slider["upper"] = json!(0.0);
    let drawer = jointed(&mut scene, "Drawer", slider, [10.0, 5.0, 0.0]);
    let mut physics = Physics::build(&scene, GRAVITY, Precision::Single);
    physics.apply_impulse(leader, [3.0, 0.0, 0.0]);
    for _ in 0..60 {
        physics.step(&mut scene, STEP);
    }
    let leader_x = scene.get::<Transform>(leader).unwrap().position[0];
    let follower_x = scene.get::<Transform>(follower).unwrap().position[0];
    assert!(leader_x > 0.5, "the push moved the leader: x = {leader_x}");
    assert!((follower_x - leader_x - 2.0).abs() < 0.1, "the follower kept its place beside it: {leader_x} and {follower_x}");
    let drop = 5.0 - y(&scene, drawer);
    assert!(drop > 0.4 && drop < 0.6, "the drawer slid down to its limit: {drop}");
}

#[test]
fn a_hinge_motor_turns_the_box_and_follows_a_changed_speed() {
    let mut scene = Scene::new();
    let mut motorised = joint("hinge", "", [0.0; 3], [0.0, 1.0, 0.0]);
    motorised["motorSpeed"] = json!(90);
    motorised["motorForce"] = json!(100);
    let wheel = jointed(&mut scene, "Wheel", motorised, [0.0, 5.0, 0.0]);
    scene.set_component(wheel, "rigidBody", json!({"kind": "dynamic", "mass": 1, "gravityScale": 0, "lockRotation": false})).unwrap();
    let mut physics = Physics::build(&scene, GRAVITY, Precision::Single);
    let yaw = |scene: &Scene| scene.get::<Transform>(wheel).unwrap().rotation[1];
    for _ in 0..30 {
        physics.step(&mut scene, STEP);
    }
    let turned = yaw(&scene);
    assert!(turned > 20.0, "the motor turns it the way it says: {turned}");
    // Stopping the motor, while playing, is seen by the joint.
    let mut stopped: serde_json::Value = scene.component(wheel, "joint").unwrap();
    stopped["motorSpeed"] = json!(0);
    stopped["motorForce"] = json!(1000);
    scene.set_component(wheel, "joint", stopped).unwrap();
    for _ in 0..30 {
        physics.step(&mut scene, STEP);
    }
    let before = yaw(&scene);
    for _ in 0..30 {
        physics.step(&mut scene, STEP);
    }
    assert!((yaw(&scene) - before).abs() < 10.0, "at speed 0 the motor holds it still: {before} then {}", yaw(&scene));
}
