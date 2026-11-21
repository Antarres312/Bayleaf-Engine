//! L2: object hierarchy with attached components, stored on top of the ECS.
//!
//! Every object is an ECS entity with a [`StableId`], a [`components::Name`] and a
//! [`components::Transform`]. The parent/child order lives in the scene, not in the ECS, so
//! reordering never moves component data. Prefabs come in a later milestone.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod components;
pub mod doc;
pub mod registry;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use bayleaf_core::StableId;
use bayleaf_core::math::Mat4;
use bayleaf_ecs::{Entity, World};
use serde_json::Value;

use components::{Name, Transform};
use doc::{ObjectDoc, SceneDoc};
use registry::ComponentRegistry;

/// Most objects one paste may add.
pub const MAX_IMPORT: usize = 100_000;

/// Errors from scene operations. Messages are shown to the user.
#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    #[error("no object with id {0}")]
    UnknownObject(StableId),
    #[error("unknown component \"{0}\"")]
    UnknownComponent(String),
    #[error("object {object} has no \"{component}\" component")]
    MissingComponent { object: StableId, component: String },
    #[error("object {object} already has a \"{component}\" component")]
    DuplicateComponent { object: StableId, component: String },
    #[error("the \"{0}\" component cannot be removed")]
    NotRemovable(String),
    #[error("invalid \"{component}\" value: {message}")]
    InvalidValue { component: String, message: String },
    #[error("an object cannot become a child of itself or of its own descendant")]
    Cycle,
    #[error("scene file: {0}")]
    Format(String),
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Internal(String),
}

/// The objects of one scene and their hierarchy.
pub struct Scene {
    world: World,
    registry: ComponentRegistry,
    entities: HashMap<StableId, Entity>,
    parents: HashMap<StableId, Option<StableId>>,
    children: HashMap<Option<StableId>, Vec<StableId>>,
}

impl Default for Scene {
    fn default() -> Self {
        Self { world: World::new(), registry: ComponentRegistry::builtin(), entities: HashMap::new(), parents: HashMap::new(), children: HashMap::new() }
    }
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn registry(&self) -> &ComponentRegistry {
        &self.registry
    }

    /// The ECS world, for systems that read component data directly.
    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn object_count(&self) -> usize {
        self.entities.len()
    }

    pub fn contains(&self, id: StableId) -> bool {
        self.entities.contains_key(&id)
    }

    /// Children of `parent` in order; `None` lists the root objects.
    pub fn children(&self, parent: Option<StableId>) -> &[StableId] {
        self.children.get(&parent).map_or(&[], Vec::as_slice)
    }

    /// Parent of `id`, or `None` for a root object or an unknown id.
    pub fn parent(&self, id: StableId) -> Option<StableId> {
        self.parents.get(&id).copied().flatten()
    }

    pub fn name(&self, id: StableId) -> Option<&str> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<Name>(entity).map(|n| n.0.as_str())
    }

    /// Typed read access to one component of an object.
    pub fn get<T: bayleaf_ecs::Component>(&self, id: StableId) -> Option<&T> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<T>(entity)
    }

    /// Typed write of one component, checked like a value from a file or command.
    pub fn set<T: components::SceneComponent>(&mut self, id: StableId, value: T) -> Result<(), SceneError> {
        value.validate().map_err(|message| SceneError::InvalidValue { component: T::NAME.to_owned(), message })?;
        let entity = self.entity(id)?;
        self.world.insert(entity, value).map_err(|e| SceneError::Internal(e.to_string()))
    }

    /// Local-to-world matrix of every object, parents before children in hierarchy order.
    pub fn world_matrices(&self) -> Vec<(StableId, Mat4)> {
        let mut out = Vec::with_capacity(self.entities.len());
        let mut stack: Vec<(StableId, Mat4)> = self.children(None).iter().rev().map(|id| (*id, Mat4::IDENTITY)).collect();
        while let Some((id, parent_matrix)) = stack.pop() {
            let local = self.get::<Transform>(id).map_or(Mat4::IDENTITY, Transform::matrix);
            let world = parent_matrix * local;
            out.push((id, world));
            stack.extend(self.children(Some(id)).iter().rev().map(|child| (*child, world)));
        }
        out
    }

    /// Local-to-world matrix of one object.
    pub fn world_matrix(&self, id: StableId) -> Option<Mat4> {
        if !self.contains(id) {
            return None;
        }
        let mut matrix = Mat4::IDENTITY;
        let mut current = Some(id);
        while let Some(object) = current {
            matrix = self.get::<Transform>(object).map_or(Mat4::IDENTITY, Transform::matrix) * matrix;
            current = self.parent(object);
        }
        Some(matrix)
    }

    /// Creates an object with default components, appended to `parent`'s children.
    pub fn create_object(&mut self, parent: Option<StableId>, name: &str) -> Result<StableId, SceneError> {
        let id = StableId::random().map_err(|e| SceneError::Internal(e.to_string()))?;
        let entity = self.insert_object(id, parent)?;
        for kind in self.registry.kinds().iter().filter(|k| !k.removable) {
            kind.add_default(&mut self.world, entity).map_err(SceneError::Internal)?;
        }
        self.world.insert(entity, Name(name.to_owned())).map_err(|e| SceneError::Internal(e.to_string()))?;
        Ok(id)
    }

    /// Deletes `id` and all its descendants. Returns how many objects were deleted.
    pub fn delete_object(&mut self, id: StableId) -> Result<usize, SceneError> {
        if !self.contains(id) {
            return Err(SceneError::UnknownObject(id));
        }
        let parent = self.parent(id);
        if let Some(siblings) = self.children.get_mut(&parent) {
            siblings.retain(|c| *c != id);
        }
        let subtree = self.subtree(id);
        for object in &subtree {
            if let Some(entity) = self.entities.remove(object) {
                self.world.despawn(entity).map_err(|e| SceneError::Internal(e.to_string()))?;
            }
            self.parents.remove(object);
            self.children.remove(&Some(*object));
        }
        Ok(subtree.len())
    }

    /// Moves `id` under `new_parent` at `index` among its new siblings (appended when `None` or too large).
    pub fn reparent(&mut self, id: StableId, new_parent: Option<StableId>, index: Option<usize>) -> Result<(), SceneError> {
        if !self.contains(id) {
            return Err(SceneError::UnknownObject(id));
        }
        if let Some(parent) = new_parent {
            if !self.contains(parent) {
                return Err(SceneError::UnknownObject(parent));
            }
            if self.subtree(id).contains(&parent) {
                return Err(SceneError::Cycle);
            }
        }
        let old_parent = self.parent(id);
        if let Some(siblings) = self.children.get_mut(&old_parent) {
            siblings.retain(|c| *c != id);
        }
        let siblings = self.children.entry(new_parent).or_default();
        let at = index.unwrap_or(siblings.len()).min(siblings.len());
        siblings.insert(at, id);
        self.parents.insert(id, new_parent);
        Ok(())
    }

    /// Like [`Self::reparent`], but rewrites the object's transform so it stays where it is in
    /// the world. Under a parent with rotation and uneven scale the result is the closest
    /// position, rotation and scale; a shear cannot be represented. Under a parent with zero
    /// scale the transform is left as it was.
    pub fn reparent_keeping_world(&mut self, id: StableId, new_parent: Option<StableId>, index: Option<usize>) -> Result<(), SceneError> {
        let world = self.world_matrix(id).ok_or(SceneError::UnknownObject(id))?;
        self.reparent(id, new_parent, index)?;
        let parent_world = new_parent.and_then(|p| self.world_matrix(p)).unwrap_or(Mat4::IDENTITY);
        if parent_world.determinant().abs() < 1e-12 {
            return Ok(());
        }
        let local = parent_world.inverse() * world;
        let (scale, rotation, position) = local.to_scale_rotation_translation();
        let (rx, ry, rz) = rotation.to_euler(bayleaf_core::math::EulerRot::XYZ);
        // Round away float noise such as 0.49999997 or -0.0 left by the matrix maths.
        let tidy = |v: f32| {
            let r = (v * 1e4).round() / 1e4;
            if r == 0.0 { 0.0 } else { r }
        };
        let transform =
            Transform { position: position.to_array().map(tidy), rotation: [rx, ry, rz].map(|a| tidy(a.to_degrees())), scale: scale.to_array().map(tidy) };
        let entity = self.entity(id)?;
        if transform.position.iter().chain(&transform.rotation).chain(&transform.scale).all(|v| v.is_finite()) {
            self.world.insert(entity, transform).map_err(|e| SceneError::Internal(e.to_string()))?;
        }
        Ok(())
    }

    /// Components of `id` as `(name, value)` in inspector order.
    pub fn components(&self, id: StableId) -> Result<Vec<(&'static str, Value)>, SceneError> {
        let entity = self.entity(id)?;
        Ok(self.registry.kinds().iter().filter_map(|k| k.get(&self.world, entity).map(|v| (k.name, v))).collect())
    }

    pub fn component(&self, id: StableId, component: &str) -> Result<Value, SceneError> {
        let entity = self.entity(id)?;
        let kind = self.registry.find(component).ok_or_else(|| SceneError::UnknownComponent(component.to_owned()))?;
        kind.get(&self.world, entity).ok_or_else(|| SceneError::MissingComponent { object: id, component: component.to_owned() })
    }

    /// Replaces the value of a component the object already has.
    pub fn set_component(&mut self, id: StableId, component: &str, value: Value) -> Result<(), SceneError> {
        self.component(id, component)?;
        self.write_component(id, component, value)
    }

    /// Adds a component with its default value.
    pub fn add_component(&mut self, id: StableId, component: &str) -> Result<(), SceneError> {
        let entity = self.entity(id)?;
        let kind = *self.registry.find(component).ok_or_else(|| SceneError::UnknownComponent(component.to_owned()))?;
        if kind.get(&self.world, entity).is_some() {
            return Err(SceneError::DuplicateComponent { object: id, component: component.to_owned() });
        }
        kind.add_default(&mut self.world, entity).map_err(SceneError::Internal)
    }

    pub fn remove_component(&mut self, id: StableId, component: &str) -> Result<(), SceneError> {
        let entity = self.entity(id)?;
        let kind = *self.registry.find(component).ok_or_else(|| SceneError::UnknownComponent(component.to_owned()))?;
        if !kind.removable {
            return Err(SceneError::NotRemovable(component.to_owned()));
        }
        if kind.remove(&mut self.world, entity) { Ok(()) } else { Err(SceneError::MissingComponent { object: id, component: component.to_owned() }) }
    }

    /// The scene as plain data, objects in depth-first hierarchy order.
    pub fn to_doc(&self) -> SceneDoc {
        let mut objects = Vec::with_capacity(self.entities.len());
        let mut stack: Vec<StableId> = self.children(None).iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let components: BTreeMap<String, Value> = self.components(id).unwrap_or_default().into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
            objects.push(ObjectDoc { id, parent: self.parent(id), components });
            stack.extend(self.children(Some(id)).iter().rev());
        }
        SceneDoc { format: doc::FORMAT.to_owned(), version: doc::VERSION, objects }
    }

    /// Builds a scene from plain data. Parents must come before their children.
    pub fn from_doc(scene_doc: &SceneDoc) -> Result<Self, SceneError> {
        if scene_doc.format != doc::FORMAT {
            return Err(SceneError::Format(format!("not a Bayleaf scene (format \"{}\")", scene_doc.format)));
        }
        if !(1..=doc::VERSION).contains(&scene_doc.version) {
            return Err(SceneError::Format(format!("unsupported version {}", scene_doc.version)));
        }
        let migrated;
        let scene_doc = if scene_doc.version == 1 {
            migrated = migrate_v1(scene_doc);
            &migrated
        } else {
            scene_doc
        };
        let mut scene = Self::new();
        for object in &scene_doc.objects {
            if scene.contains(object.id) {
                return Err(SceneError::Format(format!("object {} appears twice", object.id)));
            }
            if let Some(parent) = object.parent
                && !scene.contains(parent)
            {
                return Err(SceneError::Format(format!("object {} is listed before its parent {parent}", object.id)));
            }
            let entity = scene.insert_object(object.id, object.parent)?;
            for (name, value) in &object.components {
                scene.write_component(object.id, name, value.clone())?;
            }
            for kind in scene.registry.kinds().iter().filter(|k| !k.removable) {
                if kind.get(&scene.world, entity).is_none() {
                    kind.add_default(&mut scene.world, entity).map_err(SceneError::Internal)?;
                }
            }
        }
        Ok(scene)
    }

    /// `roots` and all their descendants as plain data, parents first, for copying. The roots are
    /// listed without a parent; a root inside another root is copied once, with that root.
    pub fn export(&self, roots: &[StableId]) -> Result<Vec<ObjectDoc>, SceneError> {
        let mut objects = Vec::new();
        for &root in roots {
            if !self.contains(root) {
                return Err(SceneError::UnknownObject(root));
            }
            if roots.iter().any(|&other| other != root && self.is_inside(root, other)) || objects.iter().any(|o: &ObjectDoc| o.id == root) {
                continue;
            }
            let mut stack = vec![root];
            while let Some(id) = stack.pop() {
                let components: BTreeMap<String, Value> = self.components(id)?.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
                objects.push(ObjectDoc { id, parent: if id == root { None } else { self.parent(id) }, components });
                stack.extend(self.children(Some(id)).iter().rev());
            }
        }
        Ok(objects)
    }

    /// Adds copies of `objects` (as [`Scene::export`] gives them) with new ids. Objects without a
    /// parent go under `parent`, starting at `index` among its children (appended when `None`).
    /// Returns the new ids of those top objects. Components are checked as when loading a file.
    pub fn import(&mut self, objects: &[ObjectDoc], parent: Option<StableId>, index: Option<usize>) -> Result<Vec<StableId>, SceneError> {
        if let Some(parent) = parent
            && !self.contains(parent)
        {
            return Err(SceneError::UnknownObject(parent));
        }
        if objects.len() > MAX_IMPORT {
            return Err(SceneError::Format(format!("at most {MAX_IMPORT} objects can be pasted at once")));
        }
        let mut new_ids: HashMap<StableId, StableId> = HashMap::new();
        let mut tops = Vec::new();
        for object in objects {
            if new_ids.contains_key(&object.id) {
                return Err(SceneError::Format(format!("object {} appears twice", object.id)));
            }
            let new_parent = match object.parent {
                None => parent,
                Some(old) => Some(*new_ids.get(&old).ok_or_else(|| SceneError::Format(format!("object {} is listed before its parent {old}", object.id)))?),
            };
            let id = StableId::random().map_err(|e| SceneError::Internal(e.to_string()))?;
            let entity = self.insert_object(id, new_parent)?;
            for (name, value) in &object.components {
                self.write_component(id, name, value.clone())?;
            }
            for kind in self.registry.kinds().iter().filter(|k| !k.removable) {
                if kind.get(&self.world, entity).is_none() {
                    kind.add_default(&mut self.world, entity).map_err(SceneError::Internal)?;
                }
            }
            new_ids.insert(object.id, id);
            if object.parent.is_none() {
                tops.push(id);
            }
        }
        if let Some(index) = index {
            for (offset, id) in tops.iter().enumerate() {
                self.reparent(*id, parent, Some(index + offset))?;
            }
        }
        Ok(tops)
    }

    /// True when `id` lies somewhere below `ancestor`.
    pub fn is_inside(&self, id: StableId, ancestor: StableId) -> bool {
        let mut current = self.parent(id);
        while let Some(parent) = current {
            if parent == ancestor {
                return true;
            }
            current = self.parent(parent);
        }
        false
    }

    pub fn load(path: &Path) -> Result<Self, SceneError> {
        let text = std::fs::read_to_string(path).map_err(|e| SceneError::Io(format!("cannot read {}: {e}", path.display())))?;
        let scene_doc: SceneDoc = serde_json::from_str(&text).map_err(|e| SceneError::Format(e.to_string()))?;
        Self::from_doc(&scene_doc)
    }

    /// Writes the scene to `path` through a temporary file, so a failed write keeps the old file.
    pub fn save(&self, path: &Path) -> Result<(), SceneError> {
        let mut text = serde_json::to_string_pretty(&self.to_doc()).map_err(|e| SceneError::Internal(e.to_string()))?;
        text.push('\n');
        let tmp = path.with_extension("bscene.tmp");
        std::fs::write(&tmp, text).map_err(|e| SceneError::Io(format!("cannot write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, path).map_err(|e| SceneError::Io(format!("cannot replace {}: {e}", path.display())))
    }

    fn entity(&self, id: StableId) -> Result<Entity, SceneError> {
        self.entities.get(&id).copied().ok_or(SceneError::UnknownObject(id))
    }

    fn insert_object(&mut self, id: StableId, parent: Option<StableId>) -> Result<Entity, SceneError> {
        if let Some(parent) = parent
            && !self.contains(parent)
        {
            return Err(SceneError::UnknownObject(parent));
        }
        let entity = self.world.spawn().map_err(|e| SceneError::Internal(e.to_string()))?;
        self.world.insert(entity, id).map_err(|e| SceneError::Internal(e.to_string()))?;
        self.entities.insert(id, entity);
        self.parents.insert(id, parent);
        self.children.entry(parent).or_default().push(id);
        Ok(entity)
    }

    fn write_component(&mut self, id: StableId, component: &str, value: Value) -> Result<(), SceneError> {
        let entity = self.entity(id)?;
        let kind = *self.registry.find(component).ok_or_else(|| SceneError::UnknownComponent(component.to_owned()))?;
        kind.set(&mut self.world, entity, value).map_err(|message| SceneError::InvalidValue { component: component.to_owned(), message })
    }

    /// `id` and all its descendants.
    fn subtree(&self, id: StableId) -> HashSet<StableId> {
        let mut found = HashSet::new();
        let mut stack = vec![id];
        while let Some(next) = stack.pop() {
            if found.insert(next) {
                stack.extend(self.children(Some(next)));
            }
        }
        found
    }
}

/// Version 1 had one `script` component per object; version 2 has a `scripts` list.
fn migrate_v1(old: &SceneDoc) -> SceneDoc {
    let mut new = old.clone();
    new.version = doc::VERSION;
    for object in &mut new.objects {
        if let Some(script) = object.components.remove("script") {
            object.components.insert("scripts".to_owned(), Value::Array(vec![script]));
        }
    }
    new
}

#[cfg(test)]
mod tests;
