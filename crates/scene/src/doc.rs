//! Scene file format (`.bscene`): pretty JSON, objects listed parents first in hierarchy order.

use std::collections::BTreeMap;

use bayleaf_core::StableId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const FORMAT: &str = "bayleaf.scene";
pub const VERSION: u32 = 1;
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
