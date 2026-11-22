//! Type-erased access to scene components by name, as JSON values.

use bayleaf_ecs::{Entity, World};
use serde_json::Value;

use crate::components::{
    AudioSource, Camera, CharacterBody, Collider, DirectionalLight, Environment, GameRules, InputMap, Interface, Joint, Mesh, Name, ParticleEmitter,
    PlayerControl, PlayerStart, PointLight, Prefab, RigidBody, SceneComponent, Scripts, SpotLight, Sprite, StreamedArea, Tags, Transform,
};

/// One component type known to the scene.
#[derive(Clone, Copy)]
pub struct ComponentKind {
    pub name: &'static str,
    pub label: &'static str,
    pub removable: bool,
    /// False for components the generic Add Component list leaves out.
    pub listed: bool,
    /// Where Add Component lists it: folders, then its name (`Light/Point Light`); empty for the top.
    pub menu: &'static str,
    has: fn(&World, Entity) -> bool,
    get: fn(&World, Entity) -> Option<Value>,
    set: fn(&mut World, Entity, Value) -> Result<(), String>,
    add_default: fn(&mut World, Entity) -> Result<(), String>,
    remove: fn(&mut World, Entity) -> bool,
    options: fn(&str) -> &'static [&'static str],
}

impl ComponentKind {
    pub fn of<T: SceneComponent>() -> Self {
        Self {
            name: T::NAME,
            label: T::LABEL,
            removable: T::REMOVABLE,
            listed: T::LISTED,
            menu: T::MENU,
            has: has::<T>,
            get: get::<T>,
            set: set::<T>,
            add_default: add_default::<T>,
            remove: remove::<T>,
            options: T::options,
        }
    }

    /// Allowed values of a text field, empty when any text is allowed.
    pub fn options(&self, field: &str) -> &'static [&'static str] {
        (self.options)(field)
    }

    pub(crate) fn has(&self, world: &World, entity: Entity) -> bool {
        (self.has)(world, entity)
    }

    pub(crate) fn get(&self, world: &World, entity: Entity) -> Option<Value> {
        (self.get)(world, entity)
    }

    pub(crate) fn set(&self, world: &mut World, entity: Entity, value: Value) -> Result<(), String> {
        (self.set)(world, entity, value)
    }

    pub(crate) fn add_default(&self, world: &mut World, entity: Entity) -> Result<(), String> {
        (self.add_default)(world, entity)
    }

    pub(crate) fn remove(&self, world: &mut World, entity: Entity) -> bool {
        (self.remove)(world, entity)
    }
}

fn has<T: SceneComponent>(world: &World, entity: Entity) -> bool {
    world.get::<T>(entity).is_some()
}

fn get<T: SceneComponent>(world: &World, entity: Entity) -> Option<Value> {
    world.get::<T>(entity).and_then(|v| serde_json::to_value(v).ok())
}

fn set<T: SceneComponent>(world: &mut World, entity: Entity, value: Value) -> Result<(), String> {
    let parsed: T = serde_json::from_value(value).map_err(|e| e.to_string())?;
    parsed.validate()?;
    world.insert(entity, parsed).map_err(|e| e.to_string())
}

fn add_default<T: SceneComponent>(world: &mut World, entity: Entity) -> Result<(), String> {
    world.insert(entity, T::default()).map_err(|e| e.to_string())
}

fn remove<T: SceneComponent>(world: &mut World, entity: Entity) -> bool {
    matches!(world.remove::<T>(entity), Ok(Some(_)))
}

/// Every component type the scene can store, in inspector order.
#[derive(Clone)]
pub struct ComponentRegistry {
    kinds: Vec<ComponentKind>,
}

impl ComponentRegistry {
    pub fn builtin() -> Self {
        Self {
            kinds: vec![
                ComponentKind::of::<Name>(),
                ComponentKind::of::<Transform>(),
                ComponentKind::of::<Mesh>(),
                ComponentKind::of::<Sprite>(),
                ComponentKind::of::<Camera>(),
                ComponentKind::of::<DirectionalLight>(),
                ComponentKind::of::<PointLight>(),
                ComponentKind::of::<SpotLight>(),
                ComponentKind::of::<Environment>(),
                ComponentKind::of::<RigidBody>(),
                ComponentKind::of::<Collider>(),
                ComponentKind::of::<CharacterBody>(),
                ComponentKind::of::<Joint>(),
                ComponentKind::of::<AudioSource>(),
                ComponentKind::of::<ParticleEmitter>(),
                ComponentKind::of::<Interface>(),
                ComponentKind::of::<StreamedArea>(),
                ComponentKind::of::<PlayerControl>(),
                ComponentKind::of::<GameRules>(),
                ComponentKind::of::<PlayerStart>(),
                ComponentKind::of::<InputMap>(),
                ComponentKind::of::<Scripts>(),
                ComponentKind::of::<Tags>(),
                ComponentKind::of::<Prefab>(),
            ],
        }
    }

    pub fn kinds(&self) -> &[ComponentKind] {
        &self.kinds
    }

    pub fn find(&self, name: &str) -> Option<&ComponentKind> {
        self.kinds.iter().find(|k| k.name == name)
    }
}
