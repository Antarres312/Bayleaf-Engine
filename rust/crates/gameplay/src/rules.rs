//! Game Rules: who the player is, where it appears and starts over, and how long the game lasts.

use bayleaf_core::StableId;
use bayleaf_core::math::DMat4;
use bayleaf_scene::Scene;
use bayleaf_scene::components::{GameRules, PlayerControl, PlayerStart, Transform};

/// The scene's Game Rules: those of the first object in the hierarchy that has them.
pub fn rules(scene: &Scene) -> Option<GameRules> {
    scene.world_matrices().into_iter().find_map(|(id, _)| scene.get::<GameRules>(id).cloned())
}

/// Objects with Player Control, in hierarchy order.
pub fn players(scene: &Scene) -> Vec<StableId> {
    scene.world_matrices().into_iter().map(|(id, _)| id).filter(|id| scene.get::<PlayerControl>(*id).is_some()).collect()
}

/// The first Player Start's world matrix, else the world's centre.
pub fn start(scene: &Scene) -> DMat4 {
    scene.world_matrices().into_iter().find(|(id, _)| scene.get::<PlayerStart>(*id).is_some()).map_or(DMat4::IDENTITY, |(_, m)| m)
}

/// The local transform that puts `object` at the Player Start, keeping its own scale.
pub fn placed_at_start(scene: &Scene, object: StableId) -> Option<Transform> {
    let parent = scene.parent(object).and_then(|p| scene.world_matrix(p)).unwrap_or(DMat4::IDENTITY);
    let mut placed = Transform::from_matrix(parent.inverse() * start(scene));
    placed.scale = scene.get::<Transform>(object)?.scale;
    placed.is_finite().then_some(placed)
}

/// Players below the respawn height, when the rules respawn them.
pub fn fallen(scene: &Scene, rules: &GameRules) -> Vec<StableId> {
    if !rules.respawn {
        return Vec::new();
    }
    players(scene).into_iter().filter(|id| scene.world_matrix(*id).is_some_and(|m| m.w_axis.y < f64::from(rules.respawn_height))).collect()
}

/// Whether the time limit has run out after `seconds` of play.
pub fn time_up(rules: &GameRules, seconds: f64) -> bool {
    rules.time_limit > 0.0 && seconds >= f64::from(rules.time_limit)
}

#[cfg(test)]
mod tests {
    use super::{fallen, placed_at_start, rules, time_up};
    use bayleaf_scene::Scene;
    use bayleaf_scene::components::Transform;
    use serde_json::json;

    #[test]
    fn players_start_at_the_player_start_and_fallen_ones_are_found() {
        let mut scene = Scene::new();
        assert!(rules(&scene).is_none(), "a scene may have no rules");
        let referee = scene.create_object(None, "Rules").unwrap();
        scene.add_component(referee, "gameRules").unwrap();
        scene
            .set_component(referee, "gameRules", json!({"playerPrefab": "", "respawn": true, "respawnHeight": -10, "timeLimit": 30, "timeUp": "win"}))
            .unwrap();
        let start = scene.create_object(None, "Start").unwrap();
        scene.add_component(start, "playerStart").unwrap();
        scene.set(start, Transform { position: [5.0, 2.0, -3.0], rotation: [0.0, 90.0, 0.0], scale: [1.0; 3] }).unwrap();
        let hero = scene.create_object(None, "Hero").unwrap();
        scene.add_component(hero, "playerControl").unwrap();
        scene.set(hero, Transform { position: [0.0, -50.0, 0.0], rotation: [0.0; 3], scale: [2.0; 3] }).unwrap();

        let rules = rules(&scene).unwrap();
        assert_eq!(fallen(&scene, &rules), vec![hero]);
        let placed = placed_at_start(&scene, hero).unwrap();
        assert_eq!(placed.position, [5.0, 2.0, -3.0]);
        assert!((placed.rotation[1] - 90.0).abs() < 1e-3, "it faces the way the start faces");
        assert_eq!(placed.scale, [2.0; 3], "and keeps its own size");
        assert!(!time_up(&rules, 29.9) && time_up(&rules, 30.0));
    }
}
