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
/// `characterBody`. Older files still load.
pub const VERSION: u32 = 9;
pub const EXTENSION: &str = "bscene";

/// Whole scene as plain data. Also used for undo snapshots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneDoc {
    pub format: String,
    pub version: u32,
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
