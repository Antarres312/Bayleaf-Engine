//! Player Control: the player steers a Character with the project's input actions.

use bayleaf_core::StableId;
use bayleaf_core::math::{DMat4, DVec3, EulerRot, Vec3};
use bayleaf_scene::Scene;
use bayleaf_scene::components::{Camera, CameraFollow, PlayerControl, Transform};

/// The actions Player Control reads.
pub const MOVE_FORWARD: &str = "MoveForward";
pub const MOVE_BACK: &str = "MoveBack";
pub const MOVE_LEFT: &str = "MoveLeft";
pub const MOVE_RIGHT: &str = "MoveRight";
pub const JUMP: &str = "Jump";

/// What Player Control asks of one object this step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Command {
    pub object: StableId,
    /// Walking velocity, world units per second.
    pub walk: [f32; 3],
    /// Jump with this upward speed.
    pub jump: Option<f32>,
    /// Turn about Y to this angle in degrees (facing where it walks).
    pub facing: Option<f32>,
}

/// Commands for every object with Player Control. `value` gives an action's value (0 to 1) and
/// `pressed` whether it went down this step. Directions are relative to where the scene's first
/// camera looks, flattened onto the ground; without a camera, forward is -Z.
pub fn drive(scene: &Scene, value: &dyn Fn(&str) -> f32, pressed: &dyn Fn(&str) -> bool) -> Vec<Command> {
    let forward = camera_forward(scene);
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    let (x, z) = (value(MOVE_RIGHT) - value(MOVE_LEFT), value(MOVE_FORWARD) - value(MOVE_BACK));
    let mut direction = right * x + forward * z;
    if direction.length() > 1.0 {
        direction = direction.normalize();
    }
    let jump = pressed(JUMP);
    scene
        .world_matrices()
        .into_iter()
        .filter_map(|(id, _)| {
            let control = scene.get::<PlayerControl>(id)?;
            let walk = (direction * control.speed).to_array();
            let facing = (control.turn_to_move && direction.length_squared() > 1e-4).then(|| (-direction.x).atan2(-direction.z).to_degrees());
            Some(Command { object: id, walk, jump: jump.then_some(control.jump_speed), facing })
        })
        .collect()
}

/// Where the scene's first camera looks, on the ground plane (unit length).
fn camera_forward(scene: &Scene) -> Vec3 {
    let world = scene.world_matrices().into_iter().find(|(id, _)| scene.get::<Camera>(*id).is_some()).map(|(_, m)| m);
    let forward = world.map_or(DVec3::NEG_Z, |m| m.transform_vector3(DVec3::NEG_Z)).as_vec3();
    Vec3::new(forward.x, 0.0, forward.z).try_normalize().unwrap_or(Vec3::NEG_Z)
}

/// The scene's first camera, moved to trail the first object with Player Control set to follow:
/// `cameraDistance` away along the ground from where it is now, `cameraHeight` up, looking at the
/// object. `None` when nothing follows or there is no camera (or the camera has a parent).
pub fn follow_camera(scene: &Scene) -> Option<(StableId, Transform)> {
    let all = scene.world_matrices();
    let (target, control) =
        all.iter().find_map(|(id, m)| scene.get::<PlayerControl>(*id).filter(|c| c.camera == CameraFollow::Follow).map(|c| (*m, c.clone())))?;
    let (camera, camera_world) = all.iter().find(|(id, _)| scene.get::<Camera>(*id).is_some()).map(|(id, m)| (*id, *m))?;
    if scene.parent(camera).is_some() {
        return None;
    }
    let target = target.w_axis.truncate();
    let away = camera_world.w_axis.truncate() - target;
    let along = DVec3::new(away.x, 0.0, away.z).try_normalize().unwrap_or(DVec3::Z);
    let eye = target + along * f64::from(control.camera_distance) + DVec3::Y * f64::from(control.camera_height);
    let look = DMat4::look_at_rh(eye, target, DVec3::Y).inverse();
    let (_, rotation, _) = look.to_scale_rotation_translation();
    let (rx, ry, rz) = rotation.to_euler(EulerRot::XYZ);
    let scale = scene.get::<Transform>(camera).map_or([1.0; 3], |t| t.scale);
    let transform = Transform { position: eye.to_array(), rotation: [rx, ry, rz].map(|a| a.to_degrees() as f32), scale };
    transform.is_finite().then_some((camera, transform))
}

#[cfg(test)]
mod tests {
    use super::{drive, follow_camera};
    use bayleaf_core::math::DVec3;
    use bayleaf_scene::Scene;
    use bayleaf_scene::components::Transform;
    use serde_json::json;

    fn scene_with_camera(camera_rotation_y: f32) -> (Scene, bayleaf_core::StableId) {
        let mut scene = Scene::new();
        let hero = scene.create_object(None, "Hero").unwrap();
        scene.add_component(hero, "playerControl").unwrap();
        let camera = scene.create_object(None, "Camera").unwrap();
        scene.add_component(camera, "camera").unwrap();
        scene.set_component(camera, "camera", json!({"projection": "perspective", "fov": 60, "size": 5, "near": 0.1, "far": 100})).unwrap();
        scene.set(camera, Transform { position: [0.0, 3.0, 6.0], rotation: [0.0, camera_rotation_y, 0.0], scale: [1.0; 3] }).unwrap();
        (scene, hero)
    }

    #[test]
    fn forward_follows_the_camera_and_the_hero_turns_to_walk() {
        let (scene, hero) = scene_with_camera(0.0);
        let held = |name: &str| if name == "MoveForward" { 1.0 } else { 0.0 };
        let commands = drive(&scene, &held, &|_| false);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].object, hero);
        assert!(commands[0].walk[2] < -4.9, "the camera looks down -Z, so forward is -Z at 5 m/s: {:?}", commands[0].walk);
        assert!(commands[0].facing.unwrap().abs() < 1e-3, "facing -Z is no turn");
        assert!(commands[0].jump.is_none());

        // The camera turned 90° to the left looks down -X; forward goes there too.
        let (scene, _) = scene_with_camera(90.0);
        let commands = drive(&scene, &held, &|name| name == "Jump");
        assert!(commands[0].walk[0] < -4.9, "{:?}", commands[0].walk);
        assert_eq!(commands[0].jump, Some(5.0));
        let diagonal = |name: &str| if name == "MoveForward" || name == "MoveRight" { 1.0 } else { 0.0 };
        let w = drive(&scene, &diagonal, &|_| false)[0].walk;
        assert!(((w[0] * w[0] + w[2] * w[2]).sqrt() - 5.0).abs() < 1e-3, "diagonals are not faster: {w:?}");
    }

    #[test]
    fn the_camera_trails_the_hero_and_looks_at_it() {
        let (mut scene, hero) = scene_with_camera(0.0);
        scene.set(hero, Transform { position: [10.0, 0.0, -20.0], rotation: [0.0; 3], scale: [1.0; 3] }).unwrap();
        let (camera, placed) = follow_camera(&scene).unwrap();
        scene.set(camera, placed.clone()).unwrap();
        let eye = DVec3::from(placed.position);
        let hero_at = DVec3::new(10.0, 0.0, -20.0);
        let flat = DVec3::new(eye.x - hero_at.x, 0.0, eye.z - hero_at.z).length();
        assert!((flat - 6.0).abs() < 1e-6 && (eye.y - 3.0).abs() < 1e-6, "6 m away and 3 m up: {eye:?}");
        let looks = scene.world_matrix(camera).unwrap().transform_vector3(DVec3::NEG_Z).normalize();
        assert!(looks.dot((hero_at - eye).normalize()) > 0.999, "it looks at the hero");
    }
}
