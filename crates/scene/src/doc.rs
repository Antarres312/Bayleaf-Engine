//! Scene file format (`.bscene`): pretty JSON, objects listed parents first in hierarchy order.

use std::collections::BTreeMap;

use bayleaf_core::StableId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const FORMAT: &str = "bayleaf.scene";
/// Version 2 replaced the single `script` component with the `scripts` list; version 3 added
/// `mesh.material`; version 4 added point and spot lights, `directionalLight.shadows` and the
/// `prefab` link; version 5 added `mesh.model` and `sprite.image`; version 6 added the `environment`
/// component; version 7 added `environment.fogDistance`, `spotLight.shadows`, `pointLight.shadows` and prefab overrides; version 8
/// added `rigidBody` and `collider`; version 9 added
/// `characterBody`; version 10 added `playerControl`; version 11
/// added `gameRules` and `playerStart`; version 12 added `inputMap`; version 13 added `joint`; version 14 added
/// `audioSource`; version 15 added `streamedArea`; version 16 lets
/// script properties be true/false, three numbers or text; version 17 added the scene's `settings` (its lighting
/// asset), into which opening an older scene moves its Environment component; version 18 added the
/// post-processing asset a scene's settings and a camera may name; version 19 added `particleEmitter`;
/// version 20 added `interface`; version 21 interfaces in the world (`space`, `size`, `pointsPerUnit`,
/// `faceCamera`); version 22 particles from a mesh's surface and size and colour curves (`sizeKeys`,
/// `colorKeys`); version 23 the Model Animation component (`modelAnimation`); version 24 list and map script properties
/// (`{"list": [...]}`, `{"map": {...}}`); version 25 skeletal animation (`modelAnimation.states`, `rootMotion`, `rootJoint` and `events`). Older files still load.
pub const VERSION: u32 = 25;
pub const EXTENSION: &str = "bscene";

/// Whole scene as plain data. Also used for undo snapshots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneDoc {
    pub format: String,
    pub version: u32,
    /// The scene's own settings (version 17).
    #[serde(default, skip_serializing_if = "crate::lighting::SceneSettings::is_default")]
    pub settings: crate::lighting::SceneSettings,
    pub objects: Vec<ObjectDoc>,
}

/// One object. Its position among its siblings is its order in [`SceneDoc::objects`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectDoc {
    pub id: StableId,
    #[serde(default)]
    pub parent: Option<StableId>,
    pub components: BTreeMap<String, Value>,
}
